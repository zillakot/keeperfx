pub const PASS_KINDS: usize = 10;
pub(super) const PASS_RASTER: usize = 0;
pub(super) const PASS_TERRAIN_PREPARE: usize = 1;
pub(super) const PASS_SHADOW_MASK: usize = 2;
pub(super) const PASS_TARGET_TRIG: usize = 3;
pub(super) const PASS_ORDERED_SPRITES: usize = 4;
pub(super) const PASS_MINIMAP: usize = 5;
pub(super) const PASS_LENS: usize = 6;
pub(super) const PASS_PRESENT: usize = 7;
pub(super) const PASS_SNAPSHOT_RASTER: usize = 8;
#[allow(dead_code)]
pub(super) const PASS_SNAPSHOT_PACK: usize = 9;

/// Reporting order of `Counters::pass_ns`; mirrored by `KfxWgpuDrawCounters`.
pub const PASS_NAMES: [&str; PASS_KINDS] = [
    "raster",
    "terrain_prepare",
    "shadow_mask",
    "target_trig",
    "ordered_sprites",
    "minimap",
    "lens",
    "present",
    "snapshot_raster",
    "snapshot_pack",
];

/// One shared query set; a submission takes a contiguous run of pairs from it and
/// resolves into its own ring slot, so the ring bounds how many submissions can be
/// in flight and the pair cursor can never overtake a slot that still holds pairs.
/// A slot must hold a whole frame of passes, because a frame is one submission; a
/// busy 1080p frame records about 114.
const SLOT_PAIRS: u32 = 256;
const SLOTS: usize = 8;
const PAIRS: u32 = SLOT_PAIRS * SLOTS as u32;

/// `KFX_WGPU_GPU_TIMING`: 1 resolves per-pass windows, 2 additionally drains the queue
/// after every timed submission so the windows cannot overlap.
pub fn level() -> u32 {
    match std::env::var("KFX_WGPU_GPU_TIMING").as_deref() {
        Ok("1") => 1,
        Ok("2") => 2,
        _ => 0,
    }
}

pub fn requested() -> bool {
    level() >= 1
}

pub fn serialized() -> bool {
    level() == 2
}

/// Adds `TIMESTAMP_QUERY` when the run asked for GPU timing and the adapter has it.
/// Pass-level `timestamp_writes` need only that feature, not the encoder-scoped one.
pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let mut descriptor = wgpu::DeviceDescriptor::default();
    if requested() && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        descriptor.required_features |= wgpu::Features::TIMESTAMP_QUERY;
    }
    descriptor
}

type PassTrace = (std::path::PathBuf, Vec<(u8, u64)>, u64);

struct Slot {
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    base: u32,
    kinds: Vec<u8>,
    reserved: bool,
    pending: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

/// Per-pass GPU durations, resolved into a ring and read back without blocking.
/// A submission whose ring is saturated goes untimed rather than stalling.
pub(super) struct PassTimings {
    period: f64,
    queries: wgpu::QuerySet,
    slots: Vec<Slot>,
    cursor: usize,
    pairs: u32,
    pub(super) ns: [u64; PASS_KINDS],
    pub(super) passes: u64,
    pub(super) dropped: u64,
    /// Scratch for the union below, kept to avoid a per-drain allocation.
    spans: Vec<(u64, u64)>,
    pub(super) union_ns: u64,
    trace: Option<PassTrace>,
}

impl PassTimings {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !requested() || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = u64::from(SLOT_PAIRS) * 16;
        let slots = (0..SLOTS)
            .map(|_| Slot {
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("per-pass GPU timestamp resolve"),
                    size: bytes,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                staging: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("per-pass GPU timestamp readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                base: 0,
                kinds: Vec::new(),
                reserved: false,
                pending: None,
            })
            .collect();
        Some(Self {
            period: f64::from(queue.get_timestamp_period()),
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("per-pass GPU timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: PAIRS * 2,
            }),
            slots,
            cursor: 0,
            pairs: 0,
            ns: [0; PASS_KINDS],
            passes: 0,
            dropped: 0,
            spans: Vec::new(),
            union_ns: 0,
            trace: std::env::var_os("KFX_WGPU_PASS_TRACE").map(|path| (path.into(), Vec::new(), 0)),
        })
    }

    /// Reserves a ring slot for one encoder. An encoder holds its slot from the first
    /// pass it stamps until its submission's readback completes.
    pub(super) fn open(&mut self, device: &wgpu::Device) -> Option<usize> {
        self.drain(device);
        for step in 0..self.slots.len() {
            let index = (self.cursor + step) % self.slots.len();
            if !self.slots[index].reserved && self.slots[index].pending.is_none() {
                self.slots[index].kinds.clear();
                self.slots[index].reserved = true;
                self.slots[index].base = self.pairs;
                self.pairs = (self.pairs + SLOT_PAIRS) % PAIRS;
                self.cursor = (index + 1) % self.slots.len();
                return Some(index);
            }
        }
        self.dropped += 1;
        None
    }

    pub(super) fn release(&mut self, index: usize) {
        self.slots[index].reserved = false;
    }

    pub(super) fn reserve(
        &mut self,
        index: usize,
        kind: usize,
    ) -> Option<(wgpu::QuerySet, u32, u32)> {
        let slot = &mut self.slots[index];
        let pair = slot.kinds.len() as u32;
        if pair >= SLOT_PAIRS {
            self.dropped += 1;
            return None;
        }
        slot.kinds.push(kind as u8);
        let query = (slot.base + pair) * 2;
        Some((self.queries.clone(), query, query + 1))
    }

    /// Records the resolve and the staging copy into the encoder about to be submitted.
    pub(super) fn close(
        &mut self,
        index: usize,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<usize> {
        let slot = &self.slots[index];
        let used = slot.kinds.len() as u32;
        if used == 0 {
            self.slots[index].reserved = false;
            return None;
        }
        let first = slot.base * 2;
        encoder.resolve_query_set(&self.queries, first..first + used * 2, &slot.resolve, 0);
        encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.staging, 0, u64::from(used) * 16);
        Some(index)
    }

    pub(super) fn map(&mut self, index: usize) {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.slots[index]
            .staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.slots[index].pending = Some(receiver);
    }

    /// Adds the length of the union of the pass intervals collected since the last call
    /// to `union_ns`, so passes that were in flight together are counted once. The caller
    /// closes one accumulation per frame, and a readback lag of a frame or two only moves
    /// a pass between adjacent frames. Two passes that overlap but land either side of a
    /// close are still counted twice, so this is an upper bound on the frame's GPU
    /// occupancy. On an adapter whose pass windows never overlap it is exactly the sum of
    /// them, which is what a Metal adapter measures here; the overlap the per-pass windows
    /// hide is stall inside each window, and only `serialized()` removes that.
    pub(super) fn settle(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        self.spans.sort_unstable();
        let mut union = 0;
        let mut open = self.spans[0];
        for &(begin, end) in &self.spans[1..] {
            if begin <= open.1 {
                open.1 = open.1.max(end);
                continue;
            }
            union += open.1 - open.0;
            open = (begin, end);
        }
        union += open.1 - open.0;
        self.spans.clear();
        self.union_ns += (union as f64 * self.period) as u64;
    }

    /// Spans held between two closes. A caller that never closes still bounds its memory
    /// and keeps reporting, at the cost of merging across more of the timeline.
    const SPAN_LIMIT: usize = 4096;

    pub(super) fn drain(&mut self, device: &wgpu::Device) {
        if self.slots.iter().all(|slot| slot.pending.is_none()) {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        for index in 0..self.slots.len() {
            let received = match &self.slots[index].pending {
                Some(receiver) => receiver.try_recv(),
                None => continue,
            };
            match received {
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Ok(Ok(())) => {
                    if let Ok(mapped) = self.slots[index].staging.slice(..).get_mapped_range() {
                        let stamps = mapped.as_chunks::<8>().0;
                        for (pair, kind) in self.slots[index].kinds.iter().enumerate() {
                            let begin = u64::from_le_bytes(stamps[pair * 2]);
                            let end = u64::from_le_bytes(stamps[pair * 2 + 1]);
                            if end > begin {
                                self.ns[usize::from(*kind)] +=
                                    ((end - begin) as f64 * self.period) as u64;
                                if let Some((_, records, dropped)) = &mut self.trace {
                                    if records.len() < 262144 {
                                        records.push((
                                            *kind,
                                            ((end - begin) as f64 * self.period) as u64,
                                        ));
                                    } else {
                                        *dropped += 1;
                                    }
                                }
                                self.passes += 1;
                                self.spans.push((begin, end));
                            }
                        }
                    }
                    self.slots[index].staging.unmap();
                }
                _ => {}
            }
            self.slots[index].reserved = false;
            self.slots[index].pending = None;
        }
        if self.spans.len() >= Self::SPAN_LIMIT {
            self.settle();
        }
    }
}

impl Drop for PassTimings {
    fn drop(&mut self) {
        if let Some((path, records, dropped)) = &self.trace {
            let report = serde_json::json!({"pass_names": PASS_NAMES, "instances": records, "dropped": dropped, "untimed": self.dropped, "serialized": serialized()});
            if let Err(error) = std::fs::write(path, report.to_string()) {
                eprintln!("GPU pass trace: {error}");
            }
        }
    }
}
