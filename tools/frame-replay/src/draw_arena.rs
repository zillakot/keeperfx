use super::ResourceKind;
use super::host::{self, Phase, Scope};
use anyhow::{Result, ensure};
use std::collections::{BTreeSet, HashMap, HashSet};

const ALIGN_WORDS: u32 = 4;
const MIN_CLASS_WORDS: u32 = 256;
const INITIAL_WORDS: u32 = 1 << 20;
/// Below this the per-batch packing path stays in use.
const MIN_LIMIT_BYTES: u64 = 32 << 20;
pub(super) const OVERFLOW: &str = "asset batch exceeds storage limit";

#[derive(Default, Debug, Clone, Copy)]
pub struct ArenaCounters {
    pub misses_new_id: u64,
    pub misses_forget: u64,
    pub misses_size_class: u64,
    pub misses_generation: u64,
    pub misses_eviction: u64,
    pub miss_new_id_bytes: u64,
    pub miss_forget_bytes: u64,
    pub miss_size_class_bytes: u64,
    pub miss_generation_bytes: u64,
    pub miss_eviction_bytes: u64,
    pub explicit_forgets: u64,
    pub capacity_bytes: u64,
    pub live_bytes: u64,
    pub retired_bytes: u64,
    pub growth_peak_bytes: u64,
    pub evictions: u64,
    pub overflows: u64,
    pub bytes_resident: u64,
    pub bytes_uploaded: u64,
    /// Widest extent transient regions reached inside one pinning scope. Frame-scoped
    /// pinning stops recycling them mid-frame, so this is what that costs.
    pub scratch_bytes_peak: u64,
}

#[derive(Clone, Copy)]
enum MissReason {
    NewId,
    Forget,
    SizeClass,
    Generation,
    Eviction,
}

struct Residency {
    offset: u32,
    class: usize,
    generation: u64,
    last_used: u64,
}

/// One persistent storage buffer holding every packed asset, suballocated in
/// power-of-two word classes and reclaimed by LRU over whole frames.
pub(crate) struct Arena {
    buffer: Option<wgpu::Buffer>,
    capacity: u32,
    limit: u32,
    high_water: u32,
    free: Vec<Vec<u32>>,
    residency: HashMap<u64, Residency>,
    missing: HashMap<u64, MissReason>,
    lru: BTreeSet<(u64, u64)>,
    pinned: HashSet<u64>,
    scratch: Vec<(usize, u32)>,
    scratch_words: u32,
    /// Regions released while a hold is open. A recorded pass may still read them, and
    /// a reuse would stage its upload at the head of that same submission.
    retired: Vec<(usize, u32)>,
    image: HostImage,
    generation: u64,
    pub(super) coalesced: bool,
    clock: u64,
    holds: u32,
    /// Set while an encoder is open: growth would change the buffer identity under the
    /// bind groups it already holds, and its forward copy would be overtaken by every
    /// staged write of the submission.
    locked: bool,
    wanted: u32,
    enabled: bool,
    counters: ArenaCounters,
    pub(super) lengths: super::arena_kinds::SourceLengths,
}

fn class_words(class: usize) -> u32 {
    MIN_CLASS_WORDS << class
}

fn size_class(words: u32) -> usize {
    let mut class = 0;
    while class_words(class) < words {
        class += 1;
    }
    class
}

impl Arena {
    pub(super) fn new(limit_bytes: u64) -> Self {
        let limit = (limit_bytes / 4).min(u64::from(u32::MAX)) as u32;
        let classes = size_class(limit.max(MIN_CLASS_WORDS)) + 1;
        Self {
            buffer: None,
            capacity: 0,
            limit,
            high_water: ALIGN_WORDS,
            free: (0..classes).map(|_| Vec::new()).collect(),
            residency: HashMap::new(),
            missing: HashMap::new(),
            lru: BTreeSet::new(),
            pinned: HashSet::new(),
            scratch: Vec::new(),
            scratch_words: 0,
            retired: Vec::new(),
            image: HostImage::default(),
            generation: 0,
            coalesced: true,
            clock: 0,
            holds: 0,
            locked: false,
            wanted: 0,
            enabled: limit_bytes >= MIN_LIMIT_BYTES,
            counters: ArenaCounters::default(),
            lengths: Default::default(),
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    /// bytes_resident is the suballocated extent, free-listed slots included,
    /// because the bump allocator never returns them.
    pub(super) fn counters(&self) -> ArenaCounters {
        ArenaCounters {
            bytes_resident: u64::from(self.high_water) * 4,
            capacity_bytes: u64::from(self.capacity) * 4,
            ..self.counters
        }
    }

    /// Ends the pinning scope unless a hold is open: everything referenced by the
    /// batch just built becomes evictable and transient regions return to their free
    /// lists. A recycled region is rewritten at the head of the next submission, so
    /// this is safe only once the encoder that reads it has been submitted.
    pub(super) fn begin_batch(&mut self) {
        self.clock += 1;
        if self.holds > 0 {
            return;
        }
        self.pinned.clear();
        self.recycle_scratch();
    }

    /// Extends the pinning scope to the open encoder, which is the submission every
    /// staged write of this frame lands at the head of.
    pub(super) fn hold(&mut self) {
        self.holds += 1;
    }

    pub(super) fn release_hold(&mut self) {
        self.holds = self.holds.saturating_sub(1);
        if self.holds > 0 {
            return;
        }
        self.pinned.clear();
        self.recycle_scratch();
    }

    fn recycle_scratch(&mut self) {
        for (class, offset) in self.scratch.drain(..).chain(self.retired.drain(..)) {
            self.free[class].push(offset);
        }
        self.scratch_words = 0;
        self.counters.retired_bytes = 0;
    }

    /// Locks growth for the life of an encoder, and grows ahead of it to the demand
    /// the last frames showed so the cold path stays off the frame.
    pub(super) fn lock(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// Whether `words` more can be suballocated without growing. Conservative: it
    /// ignores the free lists, so a true answer is a guarantee and a false one only
    /// means growth is possible.
    pub(super) fn fits(&self, words: u64) -> bool {
        !self.enabled || u64::from(self.high_water) + words <= u64::from(self.capacity)
    }

    pub(super) fn allocation_words(&self, id: u64, length: usize) -> Result<u64> {
        if !self.enabled {
            return Ok(0);
        }
        let words = u32::try_from(length).map_err(|_| anyhow::anyhow!(OVERFLOW))?;
        let class = size_class(words.max(ALIGN_WORDS));
        if self
            .residency
            .get(&id)
            .is_some_and(|entry| entry.class == class)
        {
            return Ok(0);
        }
        Ok(u64::from(class_words(class)))
    }

    /// Grows to hold `words` more past the high-water mark. The caller must have no
    /// encoder open: growth replaces the buffer bind groups name and copies it
    /// forward through its own submission.
    pub(super) fn grow_to(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
        words: u64,
    ) {
        if !self.enabled {
            return;
        }
        let need = words.min(u64::from(u32::MAX)) as u32;
        let need = need
            .max(self.wanted.saturating_sub(self.high_water))
            .max(ALIGN_WORDS);
        self.reserve(device, queue, counters, need);
    }

    pub(super) fn release(&mut self, id: u64) {
        self.forget(id, true);
    }

    fn forget(&mut self, id: u64, released: bool) {
        if self.remove(id) {
            if !released {
                self.missing.insert(id, MissReason::Forget);
            }
            self.counters.explicit_forgets += 1;
        }
        if released {
            self.missing.remove(&id);
        }
    }

    fn remove(&mut self, id: u64) -> bool {
        if let Some(entry) = self.residency.remove(&id) {
            let bytes = u64::from(class_words(entry.class)) * 4;
            self.counters.live_bytes -= bytes;
            self.lru.remove(&(entry.last_used, id));
            self.pinned.remove(&id);
            if self.holds > 0 {
                self.counters.retired_bytes += bytes;
                self.retired.push((entry.class, entry.offset));
            } else {
                self.free[entry.class].push(entry.offset);
            }
            return true;
        }
        false
    }

    pub(super) fn binding(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
    ) -> wgpu::Buffer {
        self.reserve(device, queue, counters, ALIGN_WORDS);
        self.buffer.clone().unwrap()
    }

    /// Resolves an asset to its arena word offset, uploading it when absent or
    /// stale. The caller must treat the offset as valid only for this batch.
    pub(super) fn offset_of(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
        identity: (u64, u64),
        bytes: &[u8],
        kind: ResourceKind,
    ) -> Result<u32> {
        let (id, generation) = identity;
        let words = u32::try_from(bytes.len()).map_err(|_| anyhow::anyhow!(OVERFLOW))?;
        let class = size_class(words.max(ALIGN_WORDS));
        if let Some(entry) = self.residency.get(&id)
            && entry.class == class
        {
            let offset = entry.offset;
            let stale = entry.generation != generation;
            self.touch(id, generation);
            if !stale {
                self.lengths.record(counters, kind, bytes.len(), true);
                return Ok(offset);
            }
            self.lengths.record(counters, kind, bytes.len(), false);
            self.record_miss(MissReason::Generation, bytes.len());
            self.upload(queue, counters, offset, bytes);
            return Ok(offset);
        }
        let reason = if self.remove(id) {
            MissReason::SizeClass
        } else {
            self.missing.get(&id).copied().unwrap_or(MissReason::NewId)
        };
        let offset = self.allocate(device, queue, counters, class)?;
        self.missing.remove(&id);
        self.lengths.record(counters, kind, bytes.len(), false);
        self.record_miss(reason, bytes.len());
        self.upload(queue, counters, offset, bytes);
        self.counters.live_bytes += u64::from(class_words(class)) * 4;
        self.residency.insert(
            id,
            Residency {
                offset,
                class,
                generation,
                last_used: self.clock,
            },
        );
        self.lru.insert((self.clock, id));
        self.pinned.insert(id);
        Ok(offset)
    }

    fn record_miss(&mut self, reason: MissReason, length: usize) {
        let c = &mut self.counters;
        let (count, bytes) = match reason {
            MissReason::NewId => (&mut c.misses_new_id, &mut c.miss_new_id_bytes),
            MissReason::Forget => (&mut c.misses_forget, &mut c.miss_forget_bytes),
            MissReason::SizeClass => (&mut c.misses_size_class, &mut c.miss_size_class_bytes),
            MissReason::Generation => (&mut c.misses_generation, &mut c.miss_generation_bytes),
            MissReason::Eviction => (&mut c.misses_eviction, &mut c.miss_eviction_bytes),
        };
        *count += 1;
        *bytes += length as u64 * 4;
    }

    /// Reserves a region for GPU-to-GPU copies, released at the next batch.
    pub(super) fn reserve_scratch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
        words: u32,
    ) -> Result<u32> {
        let class = size_class(words.max(ALIGN_WORDS));
        let offset = self.allocate(device, queue, counters, class)?;
        self.image
            .invalidate(offset as usize * 4, class_words(class) as usize * 4);
        self.scratch.push((class, offset));
        self.scratch_words = self.scratch_words.saturating_add(class_words(class));
        self.counters.scratch_bytes_peak = self
            .counters
            .scratch_bytes_peak
            .max(u64::from(self.scratch_words) * 4);
        Ok(offset)
    }

    fn touch(&mut self, id: u64, generation: u64) {
        let clock = self.clock;
        if let Some(entry) = self.residency.get_mut(&id) {
            self.lru.remove(&(entry.last_used, id));
            entry.last_used = clock;
            entry.generation = generation;
            self.lru.insert((clock, id));
        }
        self.pinned.insert(id);
    }

    fn upload(
        &mut self,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
        offset: u32,
        bytes: &[u8],
    ) {
        let _scope = Scope::new(Phase::Upload);
        if bytes.is_empty() {
            return;
        }
        self.stage_expanded(queue, offset, bytes, "arena assets");
        let uploaded = bytes.len() as u64 * 4;
        self.counters.bytes_uploaded += uploaded;
        counters.asset_upload_bytes += uploaded;
    }

    pub(super) fn stage_expanded(
        &mut self,
        queue: &wgpu::Queue,
        offset: u32,
        bytes: &[u8],
        label: &str,
    ) {
        let _scope = Scope::new(Phase::Upload);
        if self.capacity as u64 * 4 > 32 << 20 {
            self.flush(queue);
            let expanded: Vec<_> = bytes
                .iter()
                .flat_map(|&b| u32::from(b).to_le_bytes())
                .collect();
            if !expanded.is_empty() {
                queue.write_buffer(
                    self.buffer.as_ref().unwrap(),
                    u64::from(offset) * 4,
                    &expanded,
                );
                host::staged_bytes(expanded.len());
                host::upload_event(label, 2, 1);
                host::upload_event(label, 3, expanded.len() as u64);
                host::upload_event(label, 4, 1);
                host::upload_event(label, 5, expanded.len() as u64);
            }
            return;
        }
        if self.image.dirty.len() == super::MAX_COMMANDS {
            self.flush(queue);
        }
        let start = offset as usize * 4;
        for (dst, &byte) in self.image.bytes[start..start + bytes.len() * 4]
            .chunks_exact_mut(4)
            .zip(bytes)
        {
            dst.copy_from_slice(&u32::from(byte).to_le_bytes());
        }
        self.image.mark(start, bytes.len() * 4, self.generation);
        host::upload_event(label, 2, 1);
        host::upload_event(label, 3, bytes.len() as u64 * 4);
        if !self.coalesced {
            self.flush(queue);
        }
    }

    pub(super) fn flush(&mut self, queue: &wgpu::Queue) {
        let _scope = Scope::new(Phase::Upload);
        host::arena_dirty(
            self.image
                .dirty
                .iter()
                .map(|r| (r.end - r.start) as u64)
                .sum(),
        );
        self.image.merge(self.generation);
        for dirty in self.image.dirty.drain(..) {
            let bytes = &self.image.bytes[dirty.start..dirty.end];
            queue.write_buffer(self.buffer.as_ref().unwrap(), dirty.start as u64, bytes);
            host::upload_event("arena flush", 4, 1);
            host::upload_event("arena flush", 5, bytes.len() as u64);
            host::staged_bytes(bytes.len());
        }
    }

    pub(super) fn discard(&mut self) {
        let ids: Vec<_> = self
            .residency
            .iter()
            .filter_map(|(&id, entry)| {
                let start = entry.offset as usize * 4;
                let end = start + class_words(entry.class) as usize * 4;
                self.image
                    .dirty
                    .iter()
                    .any(|r| r.start < end && r.end > start)
                    .then_some(id)
            })
            .collect();
        for id in ids {
            self.forget(id, false);
        }
        for dirty in std::mem::take(&mut self.image.dirty) {
            self.image.invalidate(dirty.start, dirty.end - dirty.start);
        }
    }

    fn allocate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
        class: usize,
    ) -> Result<u32> {
        ensure!(class < self.free.len(), OVERFLOW);
        let need = class_words(class);
        loop {
            if let Some(offset) = self.free[class].pop() {
                return Ok(offset);
            }
            if let Some(offset) = self.bump(need) {
                return Ok(offset);
            }
            if self.reserve(device, queue, counters, need) {
                continue;
            }
            // Inside a pinning scope a reclaimed region cannot return to the free lists,
            // so neither eviction nor defragmentation can make room: growth is the only
            // way forward and the open encoder forbids it. The overflow is a host
            // rejection, which `FullRedraw` already recovers from.
            if self.holds == 0 && (self.evict() || self.reclaim()) {
                continue;
            }
            self.counters.overflows += 1;
            anyhow::bail!(OVERFLOW);
        }
    }

    fn bump(&mut self, need: u32) -> Option<u32> {
        let offset = self.high_water;
        if offset.checked_add(need)? > self.capacity {
            return None;
        }
        self.high_water = offset + need;
        Some(offset)
    }

    /// Grows to at least `need` free words past the high water mark, doubling
    /// from the initial size and copying the live contents forward.
    fn reserve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
        need: u32,
    ) -> bool {
        let wanted = match self.high_water.checked_add(need) {
            Some(wanted) if wanted <= self.limit => wanted,
            _ => return false,
        };
        if wanted <= self.capacity {
            return false;
        }
        if self.locked && self.buffer.is_some() {
            // An open encoder names this buffer; `pregrow` takes the growth next frame.
            self.wanted = self.wanted.max(wanted);
            return false;
        }
        self.wanted = 0;
        let mut capacity = self.capacity.max(INITIAL_WORDS.min(self.limit));
        while capacity < wanted {
            capacity = capacity.saturating_mul(2).min(self.limit);
        }
        self.counters.growth_peak_bytes = self
            .counters
            .growth_peak_bytes
            .max((u64::from(self.capacity) + u64::from(capacity)) * 4);
        let _scope = Scope::new(Phase::Upload);
        host::created_buffer();
        host::upload_event("persistent asset arena", 0, 1);
        host::upload_event("persistent asset arena", 1, u64::from(capacity) * 4);
        let grown = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("persistent asset arena"),
            size: u64::from(capacity) * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        counters.buffers += 1;
        counters.buffer_bytes += u64::from(capacity) * 4;
        self.flush(queue);
        if let Some(previous) = self.buffer.take() {
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&previous, 0, &grown, 0, u64::from(self.capacity) * 4);
            counters.submits += 1;
            let _submit = Scope::new(Phase::SubmitWait);
            queue.submit([encoder.finish()]);
        }
        self.buffer = Some(grown);
        self.generation += 1;
        self.image.resize((capacity as usize * 4).min(32 << 20));
        self.capacity = capacity;
        true
    }

    fn evict(&mut self) -> bool {
        let victim = self
            .lru
            .iter()
            .find(|(_, id)| !self.pinned.contains(id))
            .copied();
        let Some((_, id)) = victim else {
            return false;
        };
        self.remove(id);
        self.missing.insert(id, MissReason::Eviction);
        self.counters.evictions += 1;
        true
    }

    /// Defragments by starting over when nothing is live, which the class free
    /// lists cannot do on their own.
    fn reclaim(&mut self) -> bool {
        if !self.residency.is_empty()
            || !self.scratch.is_empty()
            || !self.retired.is_empty()
            || self.high_water == ALIGN_WORDS
        {
            return false;
        }
        for class in &mut self.free {
            class.clear();
        }
        self.high_water = ALIGN_WORDS;
        true
    }
}

#[derive(Clone, Copy)]
struct Dirty {
    start: usize,
    end: usize,
    generation: u64,
}

#[derive(Default)]
struct HostImage {
    bytes: Vec<u8>,
    valid: Vec<u64>,
    dirty: Vec<Dirty>,
}

impl HostImage {
    fn resize(&mut self, size: usize) {
        let old = self.bytes.len();
        self.bytes.resize(size, 0);
        self.valid.resize((size / 4).div_ceil(64), 0);
        self.set_valid(old, size - old, true);
    }

    fn set_valid(&mut self, start: usize, size: usize, valid: bool) {
        let mut word = start / 4;
        let end = (start + size) / 4;
        while word < end {
            let bits = (end - word).min(64 - word % 64);
            let mask = (u64::MAX >> (64 - bits)) << (word % 64);
            if valid {
                self.valid[word / 64] |= mask;
            } else {
                self.valid[word / 64] &= !mask;
            }
            word += bits;
        }
    }

    fn invalidate(&mut self, start: usize, size: usize) {
        let end = start.saturating_add(size).min(self.bytes.len());
        let start = start.min(end);
        self.set_valid(start, end - start, false);
    }

    fn mark(&mut self, start: usize, size: usize, generation: u64) {
        if size == 0 {
            return;
        }
        self.set_valid(start, size, true);
        self.dirty.push(Dirty {
            start,
            end: start + size,
            generation,
        });
    }

    fn merge(&mut self, generation: u64) {
        self.dirty.retain(|r| r.generation == generation);
        self.dirty.sort_unstable_by_key(|r| r.start);
        let mut budget = self.dirty.iter().map(|r| r.end - r.start).sum::<usize>() / 10;
        let mut used = 0;
        for read in 0..self.dirty.len() {
            let next = self.dirty[read];
            if used > 0 {
                let prior = &mut self.dirty[used - 1];
                let gap = next.start.saturating_sub(prior.end);
                let valid = gap <= budget
                    && (prior.end / 4..next.start / 4)
                        .all(|w| self.valid[w / 64] & (1 << (w % 64)) != 0);
                if next.start <= prior.end || valid {
                    prior.end = prior.end.max(next.end);
                    budget -= gap;
                    continue;
                }
            }
            self.dirty[used] = next;
            used += 1;
        }
        self.dirty.truncate(used);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_preserves_gpu_holes_and_generation() {
        let mut image = HostImage::default();
        image.resize(4096);
        image.mark(0, 400, 1);
        image.mark(404, 400, 1);
        image.mark(808, 400, 1);
        image.invalidate(804, 4);
        image.mark(2048, 4, 0);
        image.merge(1);
        assert_eq!(
            image
                .dirty
                .iter()
                .map(|r| (r.start, r.end))
                .collect::<Vec<_>>(),
            [(0, 804), (808, 1208)]
        );
        image.resize(8192);
        assert_eq!(image.valid[804 / 4 / 64] & (1 << (804 / 4 % 64)), 0);
        assert!(image.bytes[4096..].iter().all(|&b| b == 0));
    }

    #[test]
    fn adjacent_intervals_merge_without_unbounded_gap_amplification() {
        let mut image = HostImage::default();
        image.resize(4096);
        for (start, size) in [(256, 4), (0, 8), (4, 8), (12, 4)] {
            image.mark(start, size, 7);
        }
        image.merge(7);
        assert_eq!(
            image
                .dirty
                .iter()
                .map(|r| (r.start, r.end))
                .collect::<Vec<_>>(),
            [(0, 16), (256, 260)]
        );
    }

    #[test]
    fn discarded_promises_are_not_resident() {
        let mut arena = Arena::new(32 << 20);
        arena.image.resize(4096);
        arena.residency.insert(
            7,
            Residency {
                offset: 4,
                class: 0,
                generation: 1,
                last_used: 0,
            },
        );
        arena.counters.live_bytes = 1024;
        arena.lru.insert((0, 7));
        arena.pinned.insert(7);
        arena.hold();
        arena.image.mark(16, 240, 0);
        arena.discard();
        assert!(arena.residency.is_empty());
        assert!(arena.image.dirty.is_empty());
        assert_eq!(arena.counters.retired_bytes, 1024);
        arena.release_hold();
        assert_eq!(arena.free[0], [4]);
    }

    #[test]
    fn allocation_demand_uses_classes_and_existing_residency() {
        let mut arena = Arena::new(32 << 20);
        assert_eq!(arena.allocation_words(1, 60).unwrap(), 256);
        assert_eq!(arena.allocation_words(2, 81920).unwrap(), 131072);
        assert_eq!(arena.allocation_words(3, 176).unwrap(), 256);
        arena.residency.insert(
            2,
            Residency {
                offset: 4,
                class: size_class(81920),
                generation: 1,
                last_used: 0,
            },
        );
        assert_eq!(arena.allocation_words(2, 81920).unwrap(), 0);
        assert_eq!(arena.allocation_words(2, 65536).unwrap(), 65536);
        assert_eq!(Arena::new(16 << 20).allocation_words(1, 81920).unwrap(), 0);
        assert!(arena.allocation_words(1, usize::MAX).is_err());
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn shadow_batch_grows_before_packing_rounded_slots_in_an_open_encoder() {
        use super::super::{CLEAR, Command, DrawRenderer, TRIG};

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let renderer = crate::gpu::Renderer::new(device, queue).unwrap();
        let mut draw = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let target = draw.create_target(8, 8).unwrap();
        draw.submit(
            target,
            &[Command {
                kind: CLEAR,
                colour: 71,
                ..Default::default()
            }],
        )
        .unwrap();
        let vertices: [i32; 15] = [1, 1, 0, 0, 0, 7, 1, 0, 0, 0, 1, 7, 0, 0, 0];
        let geometry: Vec<_> = vertices.into_iter().flat_map(i32::to_le_bytes).collect();
        let table = draw
            .create_resource(&vec![93; 81920], 256, 320, 256)
            .unwrap();
        let sources: Vec<_> = (0..2)
            .map(|_| draw.create_resource(&geometry, 1, 1, 1).unwrap())
            .collect();
        let mut artwork: Vec<_> = [256u32, 256, 4, 4, 0, 0, 0, 24]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        artwork.extend(&geometry);
        artwork.extend(&geometry);
        artwork.extend((0..4).flat_map(|_| [4, 1, 1, 1, 1, 0]));
        let mask = draw.create_resource(&artwork, 1, 1, 1).unwrap();
        let commands: Vec<_> = sources
            .iter()
            .map(|&source| Command {
                kind: TRIG,
                source,
                table,
                source_x: 10,
                source_y: 65536,
                source_width: 64,
                colour: 1,
                width: 8,
                height: 8,
                clip_width: 8,
                clip_height: 8,
                ..Default::default()
            })
            .collect();
        draw.submit_target_triangles(target, &commands, 0, Some(mask))
            .unwrap();
        let expected = draw.readback(target).unwrap();
        assert!(expected.contains(&93));
        assert!(expected.iter().all(|&pixel| pixel == 71 || pixel == 93));
        draw.submit(
            target,
            &[Command {
                kind: CLEAR,
                colour: 71,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.arena = Arena::new(draw.storage_limit());
        let count = (INITIAL_WORDS - 100000 - ALIGN_WORDS) / MIN_CLASS_WORDS;
        for id in 1_000_000..1_000_000 + u64::from(count) {
            draw.arena
                .offset_of(
                    &draw.device,
                    &draw.queue,
                    &mut draw.counters,
                    (id, 1),
                    &[0],
                    ResourceKind::Other,
                )
                .unwrap();
        }
        for id in 1_000_000..1_000_000 + u64::from(count) {
            draw.arena.release(id);
        }
        assert_eq!(draw.arena.capacity, INITIAL_WORDS);
        assert_eq!(draw.arena.free[0].len(), count as usize);
        assert!(draw.arena.free[size_class(81920)].is_empty());
        assert!(draw.arena.fits(draw.resource_bytes as u64));
        assert!(!draw.arena.fits(131072 + 3 * 256));
        draw.frame_begin(target).unwrap();
        draw.frame_encoder();
        assert!(draw.arena.locked);
        let before = draw.counters();
        draw.submit_target_triangles(target, &commands, 0, Some(mask))
            .unwrap();
        assert_eq!(draw.arena.capacity, INITIAL_WORDS * 2);
        assert_eq!(draw.arena.counters().overflows, 0);
        assert_eq!(draw.arena.counters().evictions, 0);
        assert_eq!(
            draw.counters().target_trig_table_bytes - before.target_trig_table_bytes,
            327680
        );
        assert_eq!(
            draw.counters().target_trig_geometry_bytes - before.target_trig_geometry_bytes,
            480
        );
        assert_eq!(
            draw.counters().target_trig_table_hits - before.target_trig_table_hits,
            1
        );
        draw.frame_end().unwrap();
        assert_eq!(draw.counters().submits - before.submits, 3);
        assert_eq!(draw.frame_status().1, 0);
        assert_eq!(draw.readback(target).unwrap(), expected);
    }

    #[test]
    fn releases_retire_pinned_regions_and_drop_miss_history() {
        let mut arena = Arena::new(32 << 20);
        arena.hold();
        arena.residency.insert(
            7,
            Residency {
                offset: 4,
                class: 0,
                generation: 1,
                last_used: 0,
            },
        );
        arena.lru.insert((0, 7));
        arena.counters.live_bytes = 1024;
        arena.pinned.insert(7);
        arena.begin_batch();
        assert!(arena.pinned.contains(&7));
        assert!(!arena.evict());
        arena.release(7);
        assert!(arena.residency.is_empty());
        assert!(arena.missing.is_empty());
        assert!(arena.free[0].is_empty());
        assert_eq!(arena.counters().retired_bytes, 1024);
        arena.release_hold();
        assert_eq!(arena.free[0], [4]);
        assert_eq!(arena.counters().retired_bytes, 0);
        assert_eq!(arena.counters().explicit_forgets, 1);
        arena.missing.insert(9, MissReason::Eviction);
        arena.release(9);
        assert!(arena.missing.is_empty());
        assert_eq!(arena.counters().explicit_forgets, 1);
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn offsets_miss_causes_and_cold_growth_preserve_uploaded_bytes() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut counters = super::super::Counters::default();
        let mut arena = Arena::new(32 << 20);
        let mut resolve = |arena: &mut Arena, id, generation, bytes: &[u8]| {
            arena
                .offset_of(
                    &device,
                    &queue,
                    &mut counters,
                    (id, generation),
                    bytes,
                    ResourceKind::Other,
                )
                .unwrap()
        };
        let offset = resolve(&mut arena, 1, 1, &[71; 60]);
        assert_eq!(resolve(&mut arena, 1, 1, &[71; 60]), offset);
        assert_eq!(arena.counters().bytes_uploaded, 240);
        arena.forget(1, false);
        assert_eq!(resolve(&mut arena, 1, 1, &[71; 60]), offset);
        let offset = resolve(&mut arena, 1, 1, &[93; 300]);
        assert_eq!(resolve(&mut arena, 1, 2, &[117; 300]), offset);
        let c = arena.counters();
        assert_eq!(
            (
                c.misses_new_id,
                c.misses_forget,
                c.misses_size_class,
                c.misses_generation
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(
            (
                c.miss_new_id_bytes,
                c.miss_forget_bytes,
                c.miss_size_class_bytes,
                c.miss_generation_bytes
            ),
            (240, 240, 1200, 1200)
        );
        arena.grow_to(&device, &queue, &mut counters, u64::from(INITIAL_WORDS));
        assert_eq!(arena.counters().capacity_bytes, 8 << 20);
        assert_eq!(arena.counters().growth_peak_bytes, 12 << 20);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1200,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            arena.buffer.as_ref().unwrap(),
            u64::from(offset) * 4,
            &output,
            0,
            1200,
        );
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        output
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = output.slice(..).get_mapped_range().unwrap();
        for word in mapped.as_chunks::<4>().0 {
            assert_eq!(*word, 117u32.to_le_bytes());
        }
        drop(mapped);
        output.unmap();
        let mut small = Arena::new(32 << 20);
        small.limit = 1024;
        small
            .offset_of(
                &device,
                &queue,
                &mut counters,
                (2, 1),
                &[7; 300],
                ResourceKind::Other,
            )
            .unwrap();
        small.begin_batch();
        small
            .offset_of(
                &device,
                &queue,
                &mut counters,
                (3, 1),
                &[9; 300],
                ResourceKind::Other,
            )
            .unwrap();
        small.begin_batch();
        small
            .offset_of(
                &device,
                &queue,
                &mut counters,
                (2, 1),
                &[7; 300],
                ResourceKind::Other,
            )
            .unwrap();
        assert_eq!(small.counters().evictions, 2);
        assert_eq!(small.counters().misses_eviction, 1);
        assert_eq!(small.counters().miss_eviction_bytes, 1200);
        let c = small.counters();
        assert_eq!(
            c.bytes_uploaded,
            c.miss_new_id_bytes
                + c.miss_forget_bytes
                + c.miss_size_class_bytes
                + c.miss_generation_bytes
                + c.miss_eviction_bytes
        );
    }
}
