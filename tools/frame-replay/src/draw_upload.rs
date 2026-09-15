use super::Counters;
use super::host::{self, Phase, Scope};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const MIB: u64 = 1 << 20;
const OVERFLOW_LIMIT: u64 = 32 * MIB;

#[derive(Clone)]
pub(crate) struct Region {
    buffer: wgpu::Buffer,
    offset: u64,
    size: u64,
    generation: Option<GenerationStamp>,
}

impl Region {
    pub(crate) fn whole(buffer: wgpu::Buffer) -> Self {
        let size = buffer.size();
        Self {
            buffer,
            offset: 0,
            size,
            generation: None,
        }
    }

    pub(crate) fn entry(&self, binding: u32) -> wgpu::BindGroupEntry<'_> {
        if let Some(generation) = &self.generation {
            generation.record();
        }
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.buffer,
                offset: self.offset,
                size: std::num::NonZeroU64::new(self.size),
            }),
        }
    }
}

struct EncoderGenerations {
    recording: Cell<u64>,
    retired: Cell<u64>,
}

impl EncoderGenerations {
    fn new() -> Self {
        Self {
            recording: Cell::new(1),
            retired: Cell::new(0),
        }
    }

    fn retire(&self) {
        let generation = self.recording.get();
        self.retired.set(generation);
        self.recording.set(
            generation
                .checked_add(1)
                .expect("encoder generation exhausted"),
        );
    }
}

struct RingGeneration {
    encoders: Rc<EncoderGenerations>,
    last_use: Cell<u64>,
}

impl RingGeneration {
    fn can_rewind(self: &Rc<Self>) -> bool {
        // Live reservations may be bound again after a nested submission.
        Rc::strong_count(self) == 1 && self.last_use.get() <= self.encoders.retired.get()
    }
}

#[derive(Clone)]
struct GenerationStamp {
    generation: Cell<u64>,
    ring: Rc<RingGeneration>,
}

impl GenerationStamp {
    fn new(ring: &Rc<RingGeneration>) -> Self {
        let stamp = Self {
            generation: Cell::new(0),
            ring: ring.clone(),
        };
        stamp.record();
        stamp
    }

    fn record(&self) {
        let generation = self.ring.encoders.recording.get();
        self.generation.set(generation);
        self.ring
            .last_use
            .set(self.ring.last_use.get().max(self.generation.get()));
    }
}

#[derive(Default)]
struct Reservation {
    cursor: u64,
    demand: u64,
}

impl Reservation {
    fn reserve(&mut self, size: u64, alignment: u64, capacity: u64) -> Option<u64> {
        let start = self.cursor.checked_add(alignment - 1)? / alignment * alignment;
        let end = start.checked_add(size)?;
        self.cursor = end;
        self.demand = self.demand.max(end);
        (end <= capacity).then_some(start)
    }
}

struct Ring {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    high_water: u64,
    initial: u64,
    cap: u64,
    alignment: u64,
    reservations: Reservation,
    image: Vec<u8>,
    dirty: Option<u64>,
    generation: Rc<RingGeneration>,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl Ring {
    fn new(
        limits: &wgpu::Limits,
        initial: u64,
        cap: u64,
        label: &'static str,
        uniform: bool,
        encoders: Rc<EncoderGenerations>,
    ) -> Self {
        let (alignment, binding_limit, usage) = if uniform {
            (
                limits.min_uniform_buffer_offset_alignment,
                limits.max_uniform_buffer_binding_size,
                wgpu::BufferUsages::UNIFORM,
            )
        } else {
            (
                limits.min_storage_buffer_offset_alignment,
                limits.max_storage_buffer_binding_size,
                wgpu::BufferUsages::STORAGE,
            )
        };
        // Each Region, rather than the whole ring, is constrained by the binding limit.
        let cap = cap.min(limits.max_buffer_size);
        assert!(binding_limit >= 4);
        Self {
            buffer: None,
            capacity: 0,
            high_water: 0,
            initial: initial.min(cap),
            cap,
            alignment: u64::from(alignment).max(wgpu::COPY_BUFFER_ALIGNMENT),
            reservations: Reservation::default(),
            image: Vec::new(),
            dirty: None,
            generation: Rc::new(RingGeneration {
                encoders,
                last_use: Cell::new(0),
            }),
            label,
            usage,
        }
    }

    fn rewind(&mut self) {
        if self.generation.can_rewind() {
            self.reservations.cursor = 0;
            self.image.clear();
            self.dirty = None;
        }
    }

    fn reserve(
        &mut self,
        device: &wgpu::Device,
        counters: &mut Counters,
        words: &[u32],
    ) -> Option<Region> {
        self.rewind();
        let size = (words.len().max(1) as u64).checked_mul(4)?;
        if self.reservations.cursor == 0 && self.generation.can_rewind() {
            let wanted = self
                .initial
                .max(self.reservations.demand)
                .max(size)
                .checked_next_power_of_two()
                .unwrap_or(self.cap)
                .min(self.cap);
            if wanted > self.capacity {
                counters.buffers += 1;
                counters.buffer_bytes += wanted;
                host::created_buffer();
                host::upload_event(self.label, 0, 1);
                host::upload_event(self.label, 1, wanted);
                self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(self.label),
                    size: wanted,
                    usage: self.usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.capacity = wanted;
                self.image.reserve_exact(wanted as usize - self.image.len());
            }
        }
        let prior = self.reservations.cursor;
        let start = self
            .reservations
            .reserve(size, self.alignment, self.capacity)?;
        self.high_water = self.high_water.max(start + size);
        host::upload_padding(start - prior);
        self.image.resize((start + size) as usize, 0);
        for (dst, word) in self.image[start as usize..]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(words)
        {
            dst.copy_from_slice(&word.to_le_bytes());
        }
        self.dirty.get_or_insert(start);
        Some(Region {
            buffer: self.buffer.as_ref()?.clone(),
            offset: start,
            size,
            generation: Some(GenerationStamp::new(&self.generation)),
        })
    }

    fn flush(&mut self, queue: &wgpu::Queue) {
        if let Some(start) = self.dirty.take() {
            let bytes = &self.image[start as usize..];
            queue.write_buffer(self.buffer.as_ref().unwrap(), start, bytes);
            host::upload_event(self.label, 4, 1);
            host::upload_event(self.label, 5, bytes.len() as u64);
            host::staged_bytes(bytes.len());
        }
    }
}

pub(crate) struct Uploads {
    rings: [Ring; 3],
    encoders: Rc<EncoderGenerations>,
    overflow_bytes: u64,
    compatibility: bool,
}

impl Uploads {
    pub(crate) fn new(limits: &wgpu::Limits) -> Self {
        let encoders = Rc::new(EncoderGenerations::new());
        Self {
            rings: [
                Ring::new(
                    limits,
                    2 * MIB,
                    8 * MIB,
                    "record ring",
                    false,
                    encoders.clone(),
                ),
                Ring::new(
                    limits,
                    8 * MIB,
                    16 * MIB,
                    "index ring",
                    false,
                    encoders.clone(),
                ),
                Ring::new(limits, MIB, 2 * MIB, "uniform ring", true, encoders.clone()),
            ],
            encoders,
            overflow_bytes: 0,
            compatibility: false,
        }
    }

    pub(crate) fn configure(&mut self, capacities: [u64; 3], alignment: u64) -> anyhow::Result<()> {
        for (ring, capacity) in self.rings.iter_mut().zip(capacities) {
            anyhow::ensure!(
                capacity <= ring.cap && capacity % 4 == 0,
                "invalid ring capacity"
            );
            anyhow::ensure!(
                alignment >= ring.alignment && alignment.is_multiple_of(ring.alignment),
                "invalid ring alignment"
            );
            anyhow::ensure!(ring.generation.can_rewind(), "live upload reservation");
            ring.buffer = None;
            ring.capacity = 0;
            ring.initial = capacity;
            ring.cap = capacity;
            ring.alignment = alignment;
            ring.reservations = Reservation::default();
            ring.image.clear();
            ring.dirty = None;
        }
        Ok(())
    }

    pub(crate) fn stage(
        &mut self,
        device: &wgpu::Device,
        counters: &mut Counters,
        label: &str,
        words: &[u32],
        usage: wgpu::BufferUsages,
    ) -> Region {
        let _scope = Scope::new(Phase::Upload);
        host::upload_event(label, 2, 1);
        host::upload_event(label, 3, words.len() as u64 * 4);
        let kind = if usage.contains(wgpu::BufferUsages::UNIFORM) {
            2
        } else if label.contains("tile") || label == "ordered sprite layer" {
            1
        } else {
            0
        };
        if !self.compatibility {
            if let Some(region) = self.rings[kind].reserve(device, counters, words) {
                return region;
            }
            let size = (words.len().max(1) as u64).saturating_mul(4);
            self.charge_overflow(size);
        }
        let contents = if words.is_empty() { &[0][..] } else { words };
        Region::whole(super::buffer(device, counters, label, contents, usage))
    }

    fn charge_overflow(&mut self, size: u64) {
        if let Some(total) = self
            .overflow_bytes
            .checked_add(size)
            .filter(|&n| n <= OVERFLOW_LIMIT)
        {
            self.overflow_bytes = total;
            host::upload_overflow(size, false);
        } else {
            self.compatibility = true;
            host::upload_overflow(size, true);
        }
    }

    pub(crate) fn flush(&mut self, queue: &wgpu::Queue) {
        for ring in &mut self.rings {
            ring.flush(queue);
        }
        host::upload_gauges(
            self.rings.each_ref().map(|r| r.capacity),
            self.rings.each_ref().map(|r| r.image.len() as u64),
            self.rings.each_ref().map(|r| r.high_water),
        );
    }

    pub(crate) fn retire(&mut self) {
        self.encoders.retire();
    }

    pub(crate) fn begin_frame(&mut self) {
        self.overflow_bytes = 0;
        self.compatibility = false;
    }

    pub(crate) fn drop_pending(&mut self) {
        for ring in &mut self.rings {
            ring.dirty = None;
        }
    }

    pub(crate) fn discard(&mut self) {
        self.drop_pending();
        self.retire();
    }
}

pub(super) fn stage(
    uploads: &RefCell<Uploads>,
    device: &wgpu::Device,
    counters: &mut Counters,
    label: &str,
    words: &[u32],
    usage: wgpu::BufferUsages,
) -> Region {
    uploads
        .borrow_mut()
        .stage(device, counters, label, words, usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_allowance_falls_back_without_rejecting_valid_work() {
        let mut uploads = Uploads::new(&wgpu::Limits::default());
        uploads.charge_overflow(OVERFLOW_LIMIT);
        assert!(!uploads.compatibility);
        assert_eq!(uploads.overflow_bytes, OVERFLOW_LIMIT);
        uploads.retire();
        uploads.charge_overflow(4);
        assert!(uploads.compatibility);
        uploads.begin_frame();
        assert!(!uploads.compatibility);
        assert_eq!(uploads.overflow_bytes, 0);
    }

    #[test]
    fn rebound_generation_prevents_rewind_after_cpu_reservations_drop() {
        let mut uploads = Uploads::new(&wgpu::Limits::default());
        let ring = &mut uploads.rings[0];
        ring.reservations.cursor = 64;
        ring.reservations.demand = 64;
        ring.image.resize(64, 17);
        let earlier = GenerationStamp::new(&ring.generation);
        assert_eq!(earlier.generation.get(), 1);
        uploads.retire();
        uploads.rings[0].rewind();
        assert_eq!(uploads.rings[0].reservations.cursor, 64);
        earlier.record();
        assert_eq!(earlier.generation.get(), 2);
        drop(earlier);
        uploads.rings[0].rewind();
        assert_eq!(uploads.rings[0].reservations.cursor, 64);
        assert_eq!(uploads.rings[0].image, [17; 64]);
        uploads.retire();
        uploads.rings[0].rewind();
        assert_eq!(uploads.rings[0].reservations.cursor, 0);
        assert!(uploads.rings[0].image.is_empty());
        assert_eq!(uploads.rings[0].reservations.demand, 64);
    }

    #[test]
    fn discard_retires_the_generation_and_drops_pending_writes() {
        let mut uploads = Uploads::new(&wgpu::Limits::default());
        let ring = &mut uploads.rings[0];
        ring.reservations.cursor = 64;
        ring.image.resize(64, 17);
        ring.dirty = Some(0);
        drop(GenerationStamp::new(&ring.generation));
        ring.rewind();
        assert_eq!(ring.reservations.cursor, 64);
        uploads.drop_pending();
        uploads.rings[0].rewind();
        assert_eq!(uploads.rings[0].reservations.cursor, 64);
        assert_eq!(uploads.encoders.retired.get(), 0);
        uploads.discard();
        let ring = &mut uploads.rings[0];
        assert!(ring.dirty.is_none());
        ring.rewind();
        assert_eq!(ring.reservations.cursor, 0);
    }

    #[test]
    fn budgets_and_device_alignments_are_bounded() {
        let limits = wgpu::Limits {
            min_storage_buffer_offset_alignment: 512,
            min_uniform_buffer_offset_alignment: 1024,
            ..Default::default()
        };
        let mut uploads = Uploads::new(&limits);
        assert_eq!(
            uploads.rings.each_ref().map(|r| r.initial),
            [2 * MIB, 8 * MIB, MIB]
        );
        assert_eq!(
            uploads.rings.each_ref().map(|r| r.cap),
            [8 * MIB, 16 * MIB, 2 * MIB]
        );
        assert_eq!(
            uploads.rings.each_ref().map(|r| r.alignment),
            [512, 512, 1024]
        );
        assert!(uploads.configure([0; 3], 256).is_err());
        uploads.configure([0; 3], 1024).unwrap();
        assert!(
            uploads
                .rings
                .iter_mut()
                .all(|r| r.reservations.reserve(4, r.alignment, r.cap).is_none())
        );
    }

    #[test]
    fn reservations_align_and_keep_overflow_demand() {
        for alignment in [4, 16, 64, 256, 512] {
            let mut r = Reservation::default();
            assert_eq!(r.reserve(4, alignment, alignment * 2), Some(0));
            assert_eq!(
                r.reserve(alignment, alignment, alignment * 2),
                Some(alignment)
            );
            assert_eq!(r.reserve(1, alignment, alignment * 2), None);
            assert_eq!(r.demand, alignment * 2 + 1);
        }
        let mut r = Reservation {
            cursor: u64::MAX,
            demand: 0,
        };
        assert_eq!(r.reserve(4, 256, u64::MAX), None);
    }
}
