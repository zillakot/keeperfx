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
    pub evictions: u64,
    pub overflows: u64,
    pub bytes_resident: u64,
    pub bytes_uploaded: u64,
    /// Widest extent transient regions reached inside one pinning scope. Frame-scoped
    /// pinning stops recycling them mid-frame, so this is what that costs.
    pub scratch_bytes_peak: u64,
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
    lru: BTreeSet<(u64, u64)>,
    pinned: HashSet<u64>,
    scratch: Vec<(usize, u32)>,
    scratch_words: u32,
    /// Regions released while a hold is open. A recorded pass may still read them, and
    /// a reuse would stage its upload at the head of that same submission.
    retired: Vec<(usize, u32)>,
    staging: Vec<u8>,
    clock: u64,
    holds: u32,
    /// Set while an encoder is open: growth would change the buffer identity under the
    /// bind groups it already holds, and its forward copy would be overtaken by every
    /// staged write of the submission.
    locked: bool,
    wanted: u32,
    enabled: bool,
    counters: ArenaCounters,
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
            lru: BTreeSet::new(),
            pinned: HashSet::new(),
            scratch: Vec::new(),
            scratch_words: 0,
            retired: Vec::new(),
            staging: Vec::new(),
            clock: 0,
            holds: 0,
            locked: false,
            wanted: 0,
            enabled: limit_bytes >= MIN_LIMIT_BYTES,
            counters: ArenaCounters::default(),
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
    }

    /// Locks growth for the life of an encoder, and grows ahead of it to the demand
    /// the last frames showed so the cold path stays off the frame.
    pub(super) fn lock(&mut self, locked: bool) {
        self.locked = locked;
    }

    pub(super) fn pregrow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        counters: &mut super::Counters,
    ) {
        if !self.enabled || self.locked {
            return;
        }
        // The growth a locked frame had to refuse, plus a margin that takes the next
        // doubling a frame early rather than inside one.
        let need = self
            .wanted
            .saturating_sub(self.high_water)
            .max(self.high_water / 16)
            .max(ALIGN_WORDS);
        self.reserve(device, queue, counters, need);
    }

    pub(super) fn forget(&mut self, id: u64) {
        if let Some(entry) = self.residency.remove(&id) {
            self.lru.remove(&(entry.last_used, id));
            self.pinned.remove(&id);
            if self.holds > 0 {
                self.retired.push((entry.class, entry.offset));
            } else {
                self.free[entry.class].push(entry.offset);
            }
        }
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
        id: u64,
        generation: u64,
        bytes: &[u8],
    ) -> Result<u32> {
        let words = u32::try_from(bytes.len()).map_err(|_| anyhow::anyhow!(OVERFLOW))?;
        let class = size_class(words.max(ALIGN_WORDS));
        if let Some(entry) = self.residency.get(&id)
            && entry.class == class
        {
            let offset = entry.offset;
            let stale = entry.generation != generation;
            self.touch(id, generation);
            if !stale {
                return Ok(offset);
            }
            self.upload(queue, counters, offset, bytes);
            return Ok(offset);
        }
        self.forget(id);
        let offset = self.allocate(device, queue, counters, class)?;
        self.upload(queue, counters, offset, bytes);
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
        self.staging.clear();
        self.staging
            .extend(bytes.iter().flat_map(|&b| u32::from(b).to_le_bytes()));
        if self.staging.is_empty() {
            return;
        }
        queue.write_buffer(
            self.buffer.as_ref().unwrap(),
            u64::from(offset) * 4,
            &self.staging,
        );
        let uploaded = self.staging.len() as u64;
        self.counters.bytes_uploaded += uploaded;
        counters.asset_upload_bytes += uploaded;
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
        if let Some(previous) = self.buffer.take() {
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&previous, 0, &grown, 0, u64::from(self.capacity) * 4);
            counters.submits += 1;
            queue.submit([encoder.finish()]);
        }
        self.buffer = Some(grown);
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
        self.forget(id);
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
