use std::cell::RefCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::time::Instant;

pub const UPLOAD_LABELS: [&str; 33] = [
    "record ring",
    "index ring",
    "uniform ring",
    "immutable ordered commands",
    "ordered tile lists",
    "drawing dimensions",
    "ordered sprite commands",
    "sprite target dimensions",
    "ordered sprite layer",
    "minimap target view",
    "shadow arena region",
    "snapshot triangle commands",
    "snapshot triangle tiles",
    "snapshot image commands",
    "snapshot image tile lists",
    "immutable gpoly vertices",
    "gpoly viewport",
    "gpoly row layout",
    "arena assets",
    "snapshot tables",
    "arena flush",
    "immutable asset versions",
    "triangle immutable assets",
    "snapshot triangle fallback assets",
    "immutable lens sources and maps",
    "GPU snapshot sampling arena",
    "ordered sprite identity layer",
    "sprite artwork and run boundaries",
    "minimap semantic cells and styles",
    "immutable shadow artwork",
    "persistent asset arena",
    "effect target view",
    "compatibility",
];
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ReplayCounters {
    pub replay_pack_ns: u64,
    pub replay_upload_ns: u64,
    pub replay_bind_ns: u64,
    pub replay_encode_ns: u64,
    pub replay_tile_index_ns: u64,
    pub replay_other_ns: u64,
    pub replay_submit_wait_ns: u64,
    pub replay_bind_groups: u64,
    pub replay_buffers: u64,
    pub replay_passes: u64,
    pub replay_staged_bytes: u64,
    pub upload_queue_writes: u64,
    pub upload_queue_bytes: u64,
    pub upload_queued_bytes: u64,
    pub upload_ring_overflows: u64,
    pub upload_overflow_bytes: u64,
    pub upload_oversized_frames: u64,
    pub upload_padding_bytes: u64,
    pub upload_records_capacity: u64,
    pub upload_records_used: u64,
    pub upload_records_high_water: u64,
    pub upload_indices_capacity: u64,
    pub upload_indices_used: u64,
    pub upload_indices_high_water: u64,
    pub upload_uniforms_capacity: u64,
    pub upload_uniforms_used: u64,
    pub upload_uniforms_high_water: u64,
    pub upload_arena_dirty_bytes: u64,
    pub upload_routes: [[u64; 6]; 33],
}

impl Default for ReplayCounters {
    fn default() -> Self {
        Self {
            replay_pack_ns: 0,
            replay_upload_ns: 0,
            replay_bind_ns: 0,
            replay_encode_ns: 0,
            replay_tile_index_ns: 0,
            replay_other_ns: 0,
            replay_submit_wait_ns: 0,
            replay_bind_groups: 0,
            replay_buffers: 0,
            replay_passes: 0,
            replay_staged_bytes: 0,
            upload_queue_writes: 0,
            upload_queue_bytes: 0,
            upload_queued_bytes: 0,
            upload_ring_overflows: 0,
            upload_overflow_bytes: 0,
            upload_oversized_frames: 0,
            upload_padding_bytes: 0,
            upload_records_capacity: 0,
            upload_records_used: 0,
            upload_records_high_water: 0,
            upload_indices_capacity: 0,
            upload_indices_used: 0,
            upload_indices_high_water: 0,
            upload_uniforms_capacity: 0,
            upload_uniforms_used: 0,
            upload_uniforms_high_water: 0,
            upload_arena_dirty_bytes: 0,
            upload_routes: [[0; 6]; 33],
        }
    }
}

impl ReplayCounters {
    pub fn total_ns(&self) -> u64 {
        self.replay_pack_ns
            + self.replay_upload_ns
            + self.replay_bind_ns
            + self.replay_encode_ns
            + self.replay_tile_index_ns
            + self.replay_other_ns
            + self.replay_submit_wait_ns
    }

    pub(super) fn accumulate(&mut self, other: Self) {
        self.replay_pack_ns += other.replay_pack_ns;
        self.replay_upload_ns += other.replay_upload_ns;
        self.replay_bind_ns += other.replay_bind_ns;
        self.replay_encode_ns += other.replay_encode_ns;
        self.replay_tile_index_ns += other.replay_tile_index_ns;
        self.replay_other_ns += other.replay_other_ns;
        self.replay_submit_wait_ns += other.replay_submit_wait_ns;
        self.replay_bind_groups += other.replay_bind_groups;
        self.replay_buffers += other.replay_buffers;
        self.replay_passes += other.replay_passes;
        self.replay_staged_bytes += other.replay_staged_bytes;
        self.upload_queue_writes += other.upload_queue_writes;
        self.upload_queue_bytes += other.upload_queue_bytes;
        self.upload_queued_bytes += other.upload_queued_bytes;
        self.upload_ring_overflows += other.upload_ring_overflows;
        self.upload_overflow_bytes += other.upload_overflow_bytes;
        self.upload_oversized_frames += other.upload_oversized_frames;
        self.upload_padding_bytes += other.upload_padding_bytes;
        self.upload_records_capacity = other.upload_records_capacity;
        self.upload_records_used = other.upload_records_used;
        self.upload_records_high_water = other.upload_records_high_water;
        self.upload_indices_capacity = other.upload_indices_capacity;
        self.upload_indices_used = other.upload_indices_used;
        self.upload_indices_high_water = other.upload_indices_high_water;
        self.upload_uniforms_capacity = other.upload_uniforms_capacity;
        self.upload_uniforms_used = other.upload_uniforms_used;
        self.upload_uniforms_high_water = other.upload_uniforms_high_water;
        self.upload_arena_dirty_bytes += other.upload_arena_dirty_bytes;
        for (dst, src) in self.upload_routes.iter_mut().zip(other.upload_routes) {
            for (dst, src) in dst.iter_mut().zip(src) {
                *dst += src;
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Phase {
    Pack,
    Upload,
    Bind,
    Encode,
    TileIndex,
    Other,
    SubmitWait,
}

struct Clock {
    last: Instant,
    phase: Phase,
    counters: ReplayCounters,
}

impl Clock {
    fn switch(&mut self, next: Phase, now: Instant) -> Phase {
        let elapsed = now.duration_since(self.last).as_nanos() as u64;
        let counter = match self.phase {
            Phase::Pack => &mut self.counters.replay_pack_ns,
            Phase::Upload => &mut self.counters.replay_upload_ns,
            Phase::Bind => &mut self.counters.replay_bind_ns,
            Phase::Encode => &mut self.counters.replay_encode_ns,
            Phase::TileIndex => &mut self.counters.replay_tile_index_ns,
            Phase::Other => &mut self.counters.replay_other_ns,
            Phase::SubmitWait => &mut self.counters.replay_submit_wait_ns,
        };
        *counter += elapsed;
        self.last = now;
        std::mem::replace(&mut self.phase, next)
    }
}

thread_local! { static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) }; }

pub(super) struct Replay(PhantomData<Rc<()>>);

impl Replay {
    pub(super) fn begin() -> Self {
        CLOCK.with_borrow_mut(|clock| {
            assert!(clock.is_none(), "nested frame replay");
            *clock = Some(Clock {
                last: Instant::now(),
                phase: Phase::Other,
                counters: ReplayCounters::default(),
            });
        });
        Self(PhantomData)
    }

    pub(super) fn finish(self) -> ReplayCounters {
        CLOCK.with_borrow_mut(|clock| {
            let mut clock = clock.take().unwrap();
            clock.switch(Phase::Other, Instant::now());
            clock.counters
        })
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        CLOCK.with_borrow_mut(|clock| *clock = None);
    }
}

// Guards cannot move threads: nested phases share the replay thread's clock.
pub(crate) struct Scope(Option<Phase>, PhantomData<Rc<()>>);

impl Scope {
    pub(crate) fn new(phase: Phase) -> Self {
        Self(
            CLOCK.with_borrow_mut(|clock| {
                clock
                    .as_mut()
                    .map(|clock| clock.switch(phase, Instant::now()))
            }),
            PhantomData,
        )
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(previous) = self.0 {
            CLOCK.with_borrow_mut(|clock| {
                if let Some(clock) = clock {
                    clock.switch(previous, Instant::now());
                }
            });
        }
    }
}

pub(crate) fn created_buffer() {
    count(|c| c.replay_buffers += 1);
}
pub(crate) fn staged_bytes(bytes: usize) {
    count(|c| c.replay_staged_bytes += bytes as u64);
}
pub(crate) fn created_bind_group() {
    count(|c| c.replay_bind_groups += 1);
}
pub(crate) fn recorded_pass() {
    count(|c| c.replay_passes += 1);
}
fn count(f: impl FnOnce(&mut ReplayCounters)) {
    CLOCK.with_borrow_mut(|clock| {
        if let Some(clock) = clock {
            f(&mut clock.counters);
        }
    });
}

pub(super) struct Device(pub(super) wgpu::Device);
impl Deref for Device {
    type Target = wgpu::Device;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Device {
    pub(super) fn create_buffer(&self, descriptor: &wgpu::BufferDescriptor<'_>) -> wgpu::Buffer {
        let _scope = Scope::new(Phase::Upload);
        created_buffer();
        upload_event(descriptor.label.unwrap_or("compatibility"), 0, 1);
        upload_event(
            descriptor.label.unwrap_or("compatibility"),
            1,
            descriptor.size,
        );
        self.0.create_buffer(descriptor)
    }
    pub(super) fn create_bind_group(
        &self,
        descriptor: &wgpu::BindGroupDescriptor<'_>,
    ) -> wgpu::BindGroup {
        let _scope = Scope::new(Phase::Bind);
        created_bind_group();
        self.0.create_bind_group(descriptor)
    }
    pub(super) fn create_compute_pipeline(
        &self,
        descriptor: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> wgpu::ComputePipeline {
        let _scope = Scope::new(Phase::Bind);
        self.0.create_compute_pipeline(descriptor)
    }
    pub(super) fn create_shader_module(
        &self,
        descriptor: wgpu::ShaderModuleDescriptor<'_>,
    ) -> wgpu::ShaderModule {
        let _scope = Scope::new(Phase::Bind);
        self.0.create_shader_module(descriptor)
    }
}

pub(crate) struct Encoder(pub(super) wgpu::CommandEncoder);
impl Deref for Encoder {
    type Target = wgpu::CommandEncoder;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Encoder {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Encoder {
    pub(super) fn begin_compute_pass<'a>(
        &'a mut self,
        descriptor: &wgpu::ComputePassDescriptor<'_>,
    ) -> ComputePass<'a> {
        let scope = Scope::new(Phase::Encode);
        recorded_pass();
        ComputePass {
            pass: self.0.begin_compute_pass(descriptor),
            _scope: scope,
        }
    }
    pub(super) fn finish(self) -> wgpu::CommandBuffer {
        self.0.finish()
    }
}

pub(super) struct ComputePass<'a> {
    pass: wgpu::ComputePass<'a>,
    _scope: Scope,
}
impl<'a> Deref for ComputePass<'a> {
    type Target = wgpu::ComputePass<'a>;
    fn deref(&self) -> &Self::Target {
        &self.pass
    }
}
impl DerefMut for ComputePass<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pass
    }
}

pub(crate) fn upload_event(label: &str, metric: usize, value: u64) {
    count(|c| {
        let index = UPLOAD_LABELS
            .iter()
            .position(|&l| l == label)
            .unwrap_or(UPLOAD_LABELS.len() - 1);
        c.upload_routes[index][metric] += value;
        match metric {
            3 => c.upload_queued_bytes += value,
            4 => c.upload_queue_writes += value,
            5 => c.upload_queue_bytes += value,
            _ => (),
        }
    });
}
pub(crate) fn upload_padding(bytes: u64) {
    count(|c| c.upload_padding_bytes += bytes);
}
pub(crate) fn upload_overflow(bytes: u64, oversized: bool) {
    count(|c| {
        c.upload_ring_overflows += 1;
        c.upload_overflow_bytes += bytes;
        c.upload_oversized_frames += u64::from(oversized);
    });
}
pub(crate) fn upload_gauges(capacity: [u64; 3], used: [u64; 3], high_water: [u64; 3]) {
    count(|c| {
        c.upload_records_capacity = capacity[0];
        c.upload_records_used = used[0];
        c.upload_records_high_water = high_water[0];
        c.upload_indices_capacity = capacity[1];
        c.upload_indices_used = used[1];
        c.upload_indices_high_water = high_water[1];
        c.upload_uniforms_capacity = capacity[2];
        c.upload_uniforms_used = used[2];
        c.upload_uniforms_high_water = high_water[2];
    });
}

pub(crate) fn arena_dirty(bytes: u64) {
    count(|c| c.upload_arena_dirty_bytes += bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn exclusive_clock_partition() {
        let start = Instant::now();
        let mut clock = Clock {
            last: start,
            phase: Phase::Other,
            counters: ReplayCounters::default(),
        };
        for (i, phase) in [
            Phase::Pack,
            Phase::Upload,
            Phase::Bind,
            Phase::Encode,
            Phase::TileIndex,
            Phase::Other,
        ]
        .into_iter()
        .enumerate()
        {
            clock.switch(phase, start + Duration::from_nanos((i as u64 + 1) * 100));
        }
        assert_eq!(clock.counters.total_ns(), 600);
        assert_eq!(clock.counters.replay_pack_ns, 100);
        assert_eq!(clock.counters.replay_upload_ns, 100);
        assert_eq!(clock.counters.replay_bind_ns, 100);
        assert_eq!(clock.counters.replay_encode_ns, 100);
        assert_eq!(clock.counters.replay_tile_index_ns, 100);
        assert_eq!(clock.counters.replay_other_ns, 100);
    }

    #[test]
    fn submission_and_wait_interrupt_packing() {
        let start = Instant::now();
        let mut clock = Clock {
            last: start,
            phase: Phase::Pack,
            counters: ReplayCounters::default(),
        };
        let parent = clock.switch(Phase::SubmitWait, start + Duration::from_nanos(100));
        clock.switch(parent, start + Duration::from_nanos(1100));
        clock.switch(Phase::Other, start + Duration::from_nanos(1200));
        assert_eq!(clock.counters.replay_pack_ns, 200);
        assert_eq!(clock.counters.replay_submit_wait_ns, 1000);
        assert_eq!(clock.counters.total_ns(), 1200);
    }

    #[test]
    fn nested_error_scopes_and_disabled_counters() {
        let fail = || -> Result<(), ()> {
            let _pack = Scope::new(Phase::Pack);
            let _upload = Scope::new(Phase::Upload);
            created_buffer();
            staged_bytes(16);
            Err(())
        };
        assert!(fail().is_err());
        assert!(CLOCK.with_borrow(|clock| clock.is_none()));
        let replay = Replay::begin();
        assert!(fail().is_err());
        assert!(CLOCK.with_borrow(|clock| matches!(clock.as_ref().unwrap().phase, Phase::Other)));
        let counters = replay.finish();
        assert_eq!(counters.replay_buffers, 1);
        assert_eq!(counters.replay_staged_bytes, 16);
        assert!(counters.total_ns() > 0);
        assert_eq!(Replay::begin().finish().replay_buffers, 0);
        drop(Replay::begin());
        assert!(CLOCK.with_borrow(|clock| clock.is_none()));
    }
}
