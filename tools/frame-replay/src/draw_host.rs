use std::cell::RefCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::time::Instant;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayCounters {
    pub replay_pack_ns: u64,
    pub replay_upload_ns: u64,
    pub replay_bind_ns: u64,
    pub replay_encode_ns: u64,
    pub replay_tile_index_ns: u64,
    pub replay_other_ns: u64,
    pub replay_bind_groups: u64,
    pub replay_buffers: u64,
    pub replay_passes: u64,
    pub replay_staged_bytes: u64,
}

impl ReplayCounters {
    pub fn total_ns(&self) -> u64 {
        self.replay_pack_ns
            + self.replay_upload_ns
            + self.replay_bind_ns
            + self.replay_encode_ns
            + self.replay_tile_index_ns
            + self.replay_other_ns
    }

    pub(super) fn accumulate(&mut self, other: Self) {
        self.replay_pack_ns += other.replay_pack_ns;
        self.replay_upload_ns += other.replay_upload_ns;
        self.replay_bind_ns += other.replay_bind_ns;
        self.replay_encode_ns += other.replay_encode_ns;
        self.replay_tile_index_ns += other.replay_tile_index_ns;
        self.replay_other_ns += other.replay_other_ns;
        self.replay_bind_groups += other.replay_bind_groups;
        self.replay_buffers += other.replay_buffers;
        self.replay_passes += other.replay_passes;
        self.replay_staged_bytes += other.replay_staged_bytes;
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
