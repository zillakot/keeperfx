#[path = "draw_arena.rs"]
mod arena;
pub use arena::ArenaCounters;
#[path = "draw_frame.rs"]
mod frame_queue;
pub use frame_queue::FrameCounters;
#[path = "draw_bitmap.rs"]
mod bitmap;
#[path = "draw_map_view.rs"]
mod map_view;
#[path = "draw_minimap.rs"]
mod minimap;
#[path = "draw_movie.rs"]
mod movie;
pub use minimap::MINIMAP;
#[path = "draw_shadow.rs"]
mod shadow;
pub use shadow::SHADOW;
#[path = "draw_target_resources.rs"]
mod target_resources;
#[path = "draw_target_trig.rs"]
mod target_trig;
pub use target_resources::TargetResourceCounters;
#[path = "draw_effects.rs"]
mod effects;
#[path = "draw_trig.rs"]
mod trig;
pub const LENS_EFFECT: u32 = 10;
pub const TRIG: u32 = 9;
#[path = "draw_sprites.rs"]
mod sprites;
#[path = "draw_triangles.rs"]
mod triangles;
pub use triangles::TriangleCommand;
#[path = "draw_timing.rs"]
pub mod timing;
use timing::{
    PASS_KINDS, PASS_LENS, PASS_MINIMAP, PASS_ORDERED_SPRITES, PASS_PRESENT, PASS_RASTER,
    PASS_SHADOW_MASK, PASS_TARGET_TRIG, PASS_TERRAIN_PREPARE, PASS_TERRAIN_VALIDATE, PassTimings,
};

use crate::gpoly;
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use wgpu::util::DeviceExt;

pub const ABI_VERSION: u32 = 1;
pub const CLEAR: u32 = 0;
pub const RECT: u32 = 1;
pub const IMAGE: u32 = 2;
pub const GPOLY_SPAN: u32 = 3;
pub const CIRCLE_FILLED: u32 = 4;
pub const CIRCLE_OUTLINE: u32 = 5;
pub const SPRITE: u32 = 6;
pub const RAW_IMAGE: u32 = 7;
pub const TILED_IMAGE: u32 = 8;
pub const MOVIE: u32 = 13;
pub const MAP_VIEW: u32 = 14;
pub const BITMAP: u32 = 15;
pub const TRANSITION: u32 = 16;
/// Internal only: one terrain triangle, produced from `submit_triangles`. The C ABI
/// cannot name it, so `packable` does not accept it.
pub(crate) const TERRAIN_TRI: u32 = 17;
pub const OPAQUE: u32 = 256;
const DRAW_SHADER: &str = concat!(
    include_str!("draw.wgsl"),
    "\n",
    include_str!("draw_sprites.wgsl"),
    "\n",
    include_str!("draw_raw.wgsl"),
    "\n",
    include_str!("draw_movie.wgsl"),
    "\n",
    include_str!("draw_bitmap.wgsl"),
    "\n",
    include_str!("draw_map_view.wgsl"),
    "\n",
    include_str!("draw_trig.wgsl"),
    "\n",
    include_str!("draw_transition.wgsl")
);
const MAX_COMMANDS: usize = 262_144;
pub(super) const RECORD_WORDS: usize = 28;
pub(super) const RECORD_BYTES: usize = RECORD_WORDS * 4;
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub abi_version: u32,
    pub kind: u32,
    pub blend: u32,
    pub colour: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub clip_x: i32,
    pub clip_y: i32,
    pub clip_width: u32,
    pub clip_height: u32,
    pub source: u64,
    pub table: u64,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub start_low: u32,
    pub start_high: u32,
    pub step_low: u32,
    pub step_high: u32,
    pub transparent: u32,
    pub reserved: [u32; 3],
}

impl Default for Command {
    fn default() -> Self {
        Self {
            abi_version: ABI_VERSION,
            kind: RECT,
            blend: 0,
            colour: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            clip_x: 0,
            clip_y: 0,
            clip_width: 8192,
            clip_height: 8192,
            source: 0,
            table: 0,
            source_x: 0,
            source_y: 0,
            source_width: 0,
            source_height: 0,
            start_low: 0,
            start_high: 0,
            step_low: 0,
            step_high: 0,
            transparent: OPAQUE,
            reserved: [0; 3],
        }
    }
}

/// One entry of the frame's ordered stream.
pub(super) enum Record {
    Command(Command),
    Terrain(TriangleCommand),
}

pub(super) enum Entry<'a> {
    Command(&'a Command),
    Terrain(&'a TriangleCommand),
}

impl Record {
    pub(super) fn entry(&self) -> Entry<'_> {
        match self {
            Self::Command(command) => Entry::Command(command),
            Self::Terrain(triangle) => Entry::Terrain(triangle),
        }
    }
}

struct Resource {
    width: u32,
    height: u32,
    pitch: u32,
    bytes: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct Target {
    width: u32,
    height: u32,
    indices: wgpu::Buffer,
    root: u64,
    pitch: u32,
    offset: u32,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Counters {
    pub batches: u64,
    pub commands: u64,
    pub asset_upload_bytes: u64,
    pub command_upload_bytes: u64,
    pub readback_bytes: u64,
    pub submits: u64,
    pub dispatches: u64,
    pub waits: u64,
    pub wait_ns: u64,
    pub buffers: u64,
    pub buffer_bytes: u64,
    pub tile_allocations: u64,
    pub tile_entries: u64,
    /// The terrain share of `tile_entries`; terrain inner-loop iterations are 256 times it.
    pub terrain_tile_entries: u64,
    /// Rows the compressed prepared-terrain arena carried, and how often it grew.
    pub prepared_row_words: u64,
    pub prepared_row_allocations: u64,
    /// GPU time per pass kind, in `timing::PASS_NAMES` order; zero unless timing is on.
    pub pass_ns: [u64; PASS_KINDS],
    pub timed_passes: u64,
    pub untimed_passes: u64,
}

pub struct DrawRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    compute: wgpu::ComputePipeline,
    compute_sprite_ordered: wgpu::ComputePipeline,
    effects: Option<wgpu::ComputePipeline>,
    shadow: Option<wgpu::ComputePipeline>,
    shadow_scratch: Option<wgpu::Buffer>,
    shadow_slots: Option<wgpu::Buffer>,
    shadow_placeholder: wgpu::Buffer,
    terrain_placeholder: wgpu::Buffer,
    shadow_next_slot: u32,
    minimap: Option<minimap::MinimapState>,
    triangles: Option<triangles::TrianglePipelines>,
    timings: Option<PassTimings>,
    status: wgpu::Buffer,
    status_ring: [wgpu::Buffer; frame_queue::STATUS_RING],
    status_pending: [Option<frame_queue::StatusReceiver>; frame_queue::STATUS_RING],
    status_frame: [u64; frame_queue::STATUS_RING],
    status_cursor: u64,
    frame_index: u64,
    frame_flags: u32,
    frame_flags_index: u64,
    present: wgpu::RenderPipeline,
    targets: HashMap<u64, Target>,
    resources: HashMap<u64, Resource>,
    resource_bytes: usize,
    target_snapshots: HashMap<u64, target_resources::TargetSnapshot>,
    target_resource_counters: TargetResourceCounters,
    counters: Counters,
    frame: Option<frame_queue::QueuedFrame>,
    frame_buffers: Option<frame_queue::FrameBuffers>,
    frame_counters: FrameCounters,
    replaying: bool,
    deferred_snapshot_releases: Vec<u64>,
    arena: arena::Arena,
    tile_index: TileIndex,
    stream_commands: PersistentBuffer,
    stream_tiles: PersistentBuffer,
    prepared_rows: PersistentBuffer,
    asset_generation: u64,
    failure: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl DrawRenderer {
    /// Uses the presenter's device, queue and terminal-error handling.
    pub fn new(renderer: &crate::gpu::Renderer, format: wgpu::TextureFormat) -> Result<Self> {
        ensure!(
            !format.is_srgb(),
            "indexed drawing requires an unorm attachment"
        );
        let device = renderer.device().clone();
        let queue = renderer.queue().clone();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ordered indexed drawing"),
            source: wgpu::ShaderSource::Wgsl(DRAW_SHADER.into()),
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ordered indexed drawing"),
            layout: None,
            module: &shader,
            entry_point: Some("draw"),
            compilation_options: Default::default(),
            cache: None,
        });
        let compute_sprite_ordered =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ordered sprite traversal"),
                layout: None,
                module: &shader,
                entry_point: Some("sprite_ordered"),
                compilation_options: Default::default(),
                cache: None,
            });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU-owned palette presentation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("draw_palette.wgsl").into()),
        });
        let present = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GPU-owned palette presentation"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        renderer.check_status()?;
        let shadow_placeholder = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unbound creature shadow mask slots"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let terrain_placeholder = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unbound prepared terrain rows"),
            size: 32,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let status = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame validation status"),
            size: frame_queue::STATUS_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let status_ring = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame validation status readback"),
                size: frame_queue::STATUS_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let limits = device.limits();
        let timings = PassTimings::new(&device, &queue);
        Ok(Self {
            device,
            queue,
            compute,
            compute_sprite_ordered,
            effects: None,
            shadow: None,
            shadow_scratch: None,
            shadow_slots: None,
            shadow_placeholder,
            terrain_placeholder,
            shadow_next_slot: 0,
            minimap: None,
            triangles: None,
            timings,
            status,
            status_ring,
            status_pending: [const { None }; frame_queue::STATUS_RING],
            status_frame: [0; frame_queue::STATUS_RING],
            status_cursor: 0,
            frame_index: 0,
            frame_flags: 0,
            frame_flags_index: 0,
            present,
            targets: HashMap::new(),
            resources: HashMap::new(),
            resource_bytes: 0,
            target_snapshots: HashMap::new(),
            target_resource_counters: TargetResourceCounters::default(),
            counters: Counters::default(),
            frame: None,
            frame_buffers: None,
            frame_counters: FrameCounters::default(),
            replaying: false,
            deferred_snapshot_releases: Vec::new(),
            arena: arena::Arena::new(
                limits
                    .max_storage_buffer_binding_size
                    .min(limits.max_buffer_size),
            ),
            tile_index: TileIndex::default(),
            stream_commands: PersistentBuffer::default(),
            stream_tiles: PersistentBuffer::default(),
            prepared_rows: PersistentBuffer::default(),
            asset_generation: 1,
            failure: renderer.failure.clone(),
        })
    }

    pub fn headless() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&timing::device_descriptor(&adapter)))?;
        let renderer = crate::gpu::Renderer::new(device, queue)?;
        Self::new(&renderer, wgpu::TextureFormat::Rgba8Unorm)
    }

    pub fn check_status(&self) -> Result<()> {
        if let Some(error) = &*self.failure.lock().unwrap() {
            anyhow::bail!("{error}");
        }
        Ok(())
    }

    fn storage_limit(&self) -> u64 {
        let limits = self.device.limits();
        limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size)
    }

    pub fn counters(&self) -> Counters {
        let mut counters = self.counters;
        if let Some(timings) = &self.timings {
            counters.pass_ns = timings.ns;
            counters.timed_passes = timings.passes;
            counters.untimed_passes = timings.dropped;
        }
        counters
    }

    pub fn arena_counters(&self) -> ArenaCounters {
        self.arena.counters()
    }

    /// Retires every arena resident so the next batch re-uploads it; used when
    /// a discarded frame leaves the arena's residency unproven.
    pub(super) fn invalidate_assets(&mut self) {
        self.asset_generation += 1;
    }

    /// Host-side staged asset bytes the drawing context holds; not GPU memory and
    /// not a window delta.
    pub fn staged_asset_bytes(&self) -> u64 {
        self.resource_bytes as u64
    }

    /// Opens an encoder and, when GPU timing is on, the ring slot its passes stamp into.
    pub(super) fn begin_encoder(&mut self) -> wgpu::CommandEncoder {
        if let Some(timings) = &mut self.timings {
            timings.open(&self.device);
        }
        self.device.create_command_encoder(&Default::default())
    }

    /// The prepared-row arena as the raster kernel binds it; a placeholder until a
    /// frame has carried terrain.
    pub(super) fn terrain_rows_binding(&self) -> &wgpu::Buffer {
        self.prepared_rows
            .buffer
            .as_ref()
            .unwrap_or(&self.terrain_placeholder)
    }

    /// The renderer-owned prepared-row arena, grown in powers of two and reused.
    pub(super) fn prepared_rows(&mut self, rows: u64) -> wgpu::Buffer {
        let words = rows.max(1) * 8;
        self.counters.prepared_row_words += words;
        if self.prepared_rows.words < words {
            self.counters.prepared_row_allocations += 1;
            let size = words.next_power_of_two().max(1024) * 4;
            self.counters.buffers += 1;
            self.counters.buffer_bytes += size;
            self.prepared_rows.buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU prepared gpoly rows"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
            self.prepared_rows.words = size / 4;
        }
        self.prepared_rows.buffer.clone().unwrap()
    }

    pub(super) fn stamp(&mut self, kind: usize) -> Stamp {
        Stamp(
            self.timings
                .as_mut()
                .and_then(|timings| timings.reserve(kind)),
        )
    }

    pub(super) fn submit_encoder(&mut self, mut encoder: wgpu::CommandEncoder) {
        let slot = self
            .timings
            .as_mut()
            .and_then(|timings| timings.close(&mut encoder));
        self.counters.submits += 1;
        self.queue.submit([encoder.finish()]);
        if let Some(slot) = slot {
            self.timings.as_mut().unwrap().map(slot);
        }
    }

    pub(super) fn tracked_buffer(&mut self, descriptor: &wgpu::BufferDescriptor) -> wgpu::Buffer {
        self.counters.buffers += 1;
        self.counters.buffer_bytes += descriptor.size;
        self.device.create_buffer(descriptor)
    }

    /// Blocks until the queue drains, accumulating the measured stall.
    pub(super) fn wait_for_queue(&mut self) -> Result<()> {
        let started = std::time::Instant::now();
        let status = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        self.counters.waits += 1;
        self.counters.wait_ns += started.elapsed().as_nanos() as u64;
        status?;
        Ok(())
    }

    pub fn create_target(&mut self, width: u32, height: u32) -> Result<u64> {
        self.check_status()?;
        let pixels = crate::frame::dimensions(width, height)?;
        let size = pixels as u64 * 4;
        ensure!(
            size <= self.storage_limit(),
            "target exceeds storage binding limit"
        );
        let indices = self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("authoritative indexed target"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let id = next_handle()?;
        self.targets.insert(
            id,
            Target {
                width,
                height,
                indices,
                root: id,
                pitch: width,
                offset: 0,
            },
        );
        Ok(id)
    }

    pub fn release_target(&mut self, id: u64) -> Result<()> {
        if self.defer_target_release(id)? {
            return Ok(());
        }
        self.targets.remove(&id).context("unknown target")?;
        Ok(())
    }

    pub fn create_resource(
        &mut self,
        bytes: &[u8],
        width: u32,
        height: u32,
        pitch: u32,
    ) -> Result<u64> {
        self.check_status()?;
        validate_resource(bytes.len(), width, height, pitch)?;
        ensure!(
            bytes.len() as u64 * 4 <= self.storage_limit(),
            "resource exceeds device buffer limit"
        );
        let id = next_handle()?;
        if self.frame.is_some() {
            ensure!(
                self.resource_bytes
                    .checked_add(bytes.len())
                    .is_some_and(|n| n <= frame_queue::MAX_FRAME_BYTES),
                "queued resource arena exceeds 256 MiB"
            );
        }
        self.resource_bytes += bytes.len();
        self.resources.insert(
            id,
            Resource {
                width,
                height,
                pitch,
                bytes: bytes.to_vec(),
            },
        );
        Ok(id)
    }

    pub fn release_resource(&mut self, id: u64) -> Result<()> {
        if self.defer_resource_release(id)? {
            return Ok(());
        }
        let released = self.resources.remove(&id).context("unknown resource")?;
        debug_assert!(self.resource_bytes >= released.bytes.len());
        self.resource_bytes = self.resource_bytes.saturating_sub(released.bytes.len());
        self.arena.forget(id);
        Ok(())
    }

    pub fn submit(&mut self, target: u64, commands: &[Command]) -> Result<()> {
        if self.enqueue_commands(target, commands)? {
            return Ok(());
        }
        self.check_status()?;
        if commands.len() == 1 && commands[0].kind == MINIMAP {
            return self.submit_minimap(target, &commands[0]);
        }
        if commands.len() == 1 && commands[0].kind == LENS_EFFECT {
            return self.submit_effect(target, &commands[0]);
        }
        if commands.iter().any(sprites::ordered) {
            self.preflight_ordered_commands(target, commands)?;
            return self.submit_ordered_sprites(target, commands);
        }
        let (target_width, target_height) = self.target_dimensions(target)?;
        let limit = self.storage_limit() as usize;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let words = pack_commands(
            &mut packer,
            commands,
            &self.resources,
            ViewSpace::whole(target_width, target_height),
            limit,
        )?;
        let assets = packer.finish();
        let target = self.targets.get(&target).context("unknown target")?.clone();
        if commands.is_empty() {
            return Ok(());
        }
        let dispatch_limit = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            target.width.div_ceil(8) <= dispatch_limit
                && target.height.div_ceil(8) <= dispatch_limit,
            "drawing dispatch exceeds device limit"
        );
        self.tile_index.build(
            &mut self.counters,
            &words,
            &ViewSpace::table(&[ViewSpace::whole(target.width, target.height)]),
            &[commands.len()],
            (target.width, target.height),
            limit,
        )?;
        let tile_buffer = buffer(
            &self.device,
            &mut self.counters,
            "ordered tile lists",
            self.tile_index.data(),
            wgpu::BufferUsages::STORAGE,
        );
        let command_buffer = buffer(
            &self.device,
            &mut self.counters,
            "immutable ordered commands",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let asset_buffer = match &assets {
            Some(assets) => buffer(
                &self.device,
                &mut self.counters,
                "immutable asset versions",
                assets,
                wgpu::BufferUsages::STORAGE,
            ),
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        if let Some(assets) = &assets {
            self.counters.asset_upload_bytes += assets.len() as u64 * 4;
        }
        self.counters.command_upload_bytes +=
            (words.len() + self.tile_index.data().len()) as u64 * 4;
        let pass = self.tile_index.passes()[0];
        let Some((parameters, span_x, span_y)) = self.pass_parameters(&target, &pass) else {
            return Ok(());
        };
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ordered drawing batch"),
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                entry(0, &target.indices),
                entry(1, &command_buffer),
                entry(2, &asset_buffer),
                entry(3, &parameters),
                entry(4, &tile_buffer),
                entry(5, self.terrain_rows_binding()),
                entry(6, self.shadow_slot_binding()),
                entry(7, self.status_binding()),
            ],
        });
        let mut encoder = self.begin_encoder();
        let stamp = self.stamp(PASS_RASTER);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exclusive destination pixel ownership"),
                timestamp_writes: stamp.compute(),
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(span_x.div_ceil(8), span_y.div_ceil(8), 1);
        }
        self.counters.dispatches += 1;
        self.submit_encoder(encoder);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;

        Ok(())
    }

    /// The uniform for one raster pass, with the dispatch extent its records need;
    /// `None` when the pass covers nothing.
    fn pass_parameters(
        &mut self,
        target: &Target,
        pass: &Pass,
    ) -> Option<(wgpu::Buffer, u32, u32)> {
        let boxed = pass_box(pass, target.width, target.height);
        let (span_x, span_y) = (boxed[2] - boxed[0], boxed[3] - boxed[1]);
        if span_x == 0 || span_y == 0 {
            return None;
        }
        let parameters = buffer(
            &self.device,
            &mut self.counters,
            "drawing dimensions",
            &[
                target.width,
                target.height,
                boxed[2],
                pass.columns(),
                target.pitch,
                target.offset,
                pass.header,
                boxed[0],
                boxed[1],
                boxed[3],
                0,
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        Some((parameters, span_x, span_y))
    }

    /// One raster pass over one segment of the frame's stream; the segment's header in
    /// the shared tile index bounds both what each pixel iterates and the dispatch box.
    /// The frame's terrain setup rides into the first segment's encoder, so the rows
    /// are written by an earlier pass of the same submission than the one reading them.
    fn raster_segment(
        &mut self,
        target: &Target,
        buffers: &(wgpu::Buffer, wgpu::Buffer, wgpu::Buffer),
        pass: &Pass,
        commands: usize,
        prepare: &mut Option<PendingPrepare>,
    ) -> Result<()> {
        let (command_buffer, tile_buffer, asset_buffer) = buffers;
        let mut encoder = self.begin_encoder();
        self.record_prepare(&mut encoder, prepare)?;
        let Some((parameters, span_x, span_y)) = self.pass_parameters(target, pass) else {
            self.submit_encoder(encoder);
            return Ok(());
        };
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ordered drawing segment"),
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                entry(0, &target.indices),
                entry(1, command_buffer),
                entry(2, asset_buffer),
                entry(3, &parameters),
                entry(4, tile_buffer),
                entry(5, self.terrain_rows_binding()),
                entry(6, self.shadow_slot_binding()),
                entry(7, self.status_binding()),
            ],
        });
        let stamp = self.stamp(PASS_RASTER);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exclusive destination pixel ownership"),
                timestamp_writes: stamp.compute(),
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(span_x.div_ceil(8), span_y.div_ceil(8), 1);
        }
        self.counters.dispatches += 1;
        self.submit_encoder(encoder);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands as u64;
        Ok(())
    }

    /// Records the frame's single terrain setup dispatch, once, into `encoder`.
    pub(super) fn record_prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        prepare: &mut Option<PendingPrepare>,
    ) -> Result<()> {
        let Some(pending) = prepare.take() else {
            return Ok(());
        };
        self.triangles
            .get_or_insert_with(|| triangles::TrianglePipelines::new(&self.device));
        let stamp = self.stamp(PASS_TERRAIN_PREPARE);
        let extents = self.triangles.as_ref().unwrap().prepare.encode(
            &self.device,
            encoder,
            &pending.triangles,
            &pending.layout,
            &pending.rows,
            stamp.compute(),
        )?;
        self.counters.dispatches += 1;
        let count = pending.triangles.len() as u32;
        let deepest = pending
            .layout
            .iter()
            .map(|entry| entry.rows)
            .max()
            .unwrap_or(0);
        if deepest == 0 {
            return Ok(());
        }
        let parameters = buffer(
            &self.device,
            &mut self.counters,
            "terrain batch dimensions",
            &[count, 0, 0, 0],
            wgpu::BufferUsages::UNIFORM,
        );
        let stamp = self.stamp(PASS_TERRAIN_VALIDATE);
        let pipelines = self.triangles.as_ref().unwrap();
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain span validation"),
            layout: &pipelines.validate.get_bind_group_layout(0),
            entries: &[
                entry(0, &pending.rows),
                entry(3, &parameters),
                entry(5, &extents),
                entry(6, &self.status),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("terrain span validation"),
                timestamp_writes: stamp.compute(),
            });
            pass.set_pipeline(&pipelines.validate);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(deepest.div_ceil(64), count, 1);
        }
        self.counters.dispatches += 1;
        Ok(())
    }

    pub fn target_dimensions(&self, target: u64) -> Result<(u32, u32)> {
        self.check_status()?;
        let target = self.targets.get(&target).context("unknown target")?.clone();
        Ok((target.width, target.height))
    }

    pub fn readback(&mut self, target: u64) -> Result<Vec<u8>> {
        self.checkpoint_target(target)?;
        self.check_status()?;
        let target = self.targets.get(&target).context("unknown target")?.clone();
        let size = u64::from(target.width) * u64::from(target.height) * 4;
        let staging = self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("explicit indexed readback"),
            size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let status = self.status_record(&mut encoder);
        for row in 0..target.height {
            encoder.copy_buffer_to_buffer(
                &target.indices,
                u64::from(target.offset + row * target.pitch) * 4,
                &staging,
                u64::from(row * target.width) * 4,
                u64::from(target.width) * 4,
            );
        }
        self.submit_encoder(encoder);
        if let Some(slot) = status {
            self.status_map(slot);
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.wait_for_queue()?;
        receiver.recv()??;
        let mapped = staging.slice(..).get_mapped_range()?;
        let bytes = mapped
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| word[0])
            .collect();
        drop(mapped);
        staging.unmap();
        self.counters.readback_bytes += size;
        self.check_status()?;
        Ok(bytes)
    }

    pub fn present_into(
        &mut self,
        target: u64,
        palette: &[u8],
        output_width: u32,
        output_height: u32,
        view: &wgpu::TextureView,
    ) -> Result<()> {
        self.checkpoint_target(target)?;
        self.check_status()?;
        let target = self.targets.get(&target).context("unknown target")?.clone();
        crate::frame::dimensions(output_width, output_height)?;
        ensure!(
            palette.len() == 1024,
            "palette must contain 256 RGBA entries"
        );
        ensure!(
            output_width.max(output_height) <= self.device.limits().max_texture_dimension_2d,
            "output exceeds texture limit"
        );
        let palette_words: Vec<_> = palette
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| u32::from_le_bytes(*v))
            .collect();
        let palette_buffer = buffer(
            &self.device,
            &mut self.counters,
            "presentation palette version",
            &palette_words,
            wgpu::BufferUsages::STORAGE,
        );
        let parameters = buffer(
            &self.device,
            &mut self.counters,
            "presentation dimensions",
            &[
                target.width,
                target.height,
                output_width,
                output_height,
                target.pitch,
                target.offset,
                0,
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU-owned presentation"),
            layout: &self.present.get_bind_group_layout(0),
            entries: &[
                entry(0, &target.indices),
                entry(1, &palette_buffer),
                entry(2, &parameters),
            ],
        });
        let mut encoder = self.begin_encoder();
        let stamp = self.stamp(PASS_PRESENT);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("GPU-owned presentation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: stamp.render(),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.present);
            pass.set_bind_group(0, &binding, &[]);
            pass.draw(0..3, 0..1);
        }
        self.submit_encoder(encoder);
        Ok(())
    }
}

fn next_handle() -> Result<u64> {
    NEXT_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| anyhow::anyhow!("drawing handle space exhausted"))
}

pub(crate) fn validate_resource(length: usize, width: u32, height: u32, pitch: u32) -> Result<()> {
    crate::frame::dimensions(width, height)?;
    ensure!(pitch >= width && pitch <= 16384, "invalid asset pitch");
    let required = u64::from(pitch) * u64::from(height - 1) + u64::from(width);
    ensure!(
        length as u64 >= required && length <= 16 * 1024 * 1024,
        "invalid asset length"
    );
    Ok(())
}

fn bounds(x: i32, y: i32, width: u32, height: u32) -> Result<[u32; 4]> {
    ensure!(
        width <= 16384
            && height <= 16384
            && (-16384..=16384).contains(&x)
            && (-16384..=16384).contains(&y),
        "invalid drawing bounds"
    );
    Ok([
        x as u32,
        y as u32,
        (x + width as i32) as u32,
        (y + height as i32) as u32,
    ])
}

/// Resolves an asset handle to the word offset the kernels sample from, either
/// in the persistent arena or in a batch-lifetime asset vector.
pub(super) enum AssetPacker<'a> {
    Batch {
        assets: Vec<u32>,
        offsets: HashMap<u64, u32>,
        limit: usize,
    },
    Arena {
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        arena: &'a mut arena::Arena,
        counters: &'a mut Counters,
        generation: u64,
    },
}

pub(super) fn asset_packer<'a>(
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    arena: &'a mut arena::Arena,
    counters: &'a mut Counters,
    generation: u64,
    limit: usize,
) -> AssetPacker<'a> {
    if arena.enabled() {
        arena.begin_batch();
        AssetPacker::Arena {
            device,
            queue,
            arena,
            counters,
            generation,
        }
    } else {
        AssetPacker::batch(limit)
    }
}

impl AssetPacker<'_> {
    pub(super) fn batch(limit: usize) -> Self {
        Self::Batch {
            assets: Vec::new(),
            offsets: HashMap::new(),
            limit,
        }
    }

    pub(super) fn offset(&mut self, id: u64, bytes: &[u8]) -> Result<u32> {
        self.prefix(id, bytes, bytes.len())
    }

    /// Only the batch path honours `length`; the arena keeps whole resources
    /// resident and the kernels read no further than their own bounds.
    pub(super) fn prefix(&mut self, id: u64, bytes: &[u8], length: usize) -> Result<u32> {
        match self {
            Self::Batch {
                assets,
                offsets,
                limit,
            } => {
                if let Some(offset) = offsets.get(&id) {
                    return Ok(*offset);
                }
                ensure!(
                    assets
                        .len()
                        .checked_add(length)
                        .context("asset length overflow")?
                        <= *limit / 4,
                    arena::OVERFLOW
                );
                let offset = assets.len() as u32;
                assets.extend(bytes[..length].iter().map(|byte| u32::from(*byte)));
                offsets.insert(id, offset);
                Ok(offset)
            }
            Self::Arena {
                device,
                queue,
                arena,
                counters,
                generation,
            } => arena.offset_of(device, queue, counters, id, *generation, bytes),
        }
    }

    /// `None` when the assets are resident in the arena instead.
    pub(super) fn finish(self) -> Option<Vec<u32>> {
        match self {
            Self::Batch { mut assets, .. } => {
                if assets.is_empty() {
                    assets.push(0);
                }
                Some(assets)
            }
            Self::Arena { .. } => None,
        }
    }
}

/// Kinds `pack_commands` accepts in a multi-command batch. `WgpuTerrainBridge::PacksInBatch`
/// mirrors this set; a change here needs the same change there.
pub(crate) fn packable(kind: u32) -> bool {
    kind <= TRIG || kind == MOVIE || kind == MAP_VIEW || kind == BITMAP
}

/// The rectangle a command was issued against, and its origin in the space the
/// dispatch addresses; a root-space stream carries one per view, a single-view
/// batch the view itself at the origin. Records name a view by index, so the
/// packed record stays 112 bytes and the per-pixel fetch stays coalesced.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ViewSpace {
    pub origin_x: u32,
    pub origin_y: u32,
    pub width: u32,
    pub height: u32,
}

impl ViewSpace {
    /// Four words per view: origin, then the extent, which the kernel does not read but
    /// which lets the index be compared against a view-space binning of the same frame.
    pub(super) fn table(views: &[Self]) -> Vec<u32> {
        views
            .iter()
            .flat_map(|view| [view.origin_x, view.origin_y, view.width, view.height])
            .collect()
    }

    pub(super) fn whole(width: u32, height: u32) -> Self {
        Self {
            origin_x: 0,
            origin_y: 0,
            width,
            height,
        }
    }

    fn rebase(&self, rectangle: [u32; 4]) -> [u32; 4] {
        [
            rectangle[0].wrapping_add(self.origin_x),
            rectangle[1].wrapping_add(self.origin_y),
            rectangle[2].wrapping_add(self.origin_x),
            rectangle[3].wrapping_add(self.origin_y),
        ]
    }

    /// A dispatch over the root no longer stops at the view edge, so the clip
    /// carries the view rectangle the per-view dispatch used to impose.
    fn clip(&self, rectangle: [u32; 4]) -> [u32; 4] {
        let rebased = self.rebase(rectangle);
        let far = [self.origin_x + self.width, self.origin_y + self.height];
        [
            (rebased[0] as i32).max(self.origin_x as i32) as u32,
            (rebased[1] as i32).max(self.origin_y as i32) as u32,
            (rebased[2] as i32).min(far[0] as i32) as u32,
            (rebased[3] as i32).min(far[1] as i32) as u32,
        ]
    }
}

/// A renderer-owned buffer reused across frames, grown in powers of two.
#[derive(Default)]
pub(super) struct PersistentBuffer {
    buffer: Option<wgpu::Buffer>,
    words: u64,
    staging: Vec<u8>,
}

fn persist(
    slot: &mut PersistentBuffer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    counters: &mut Counters,
    label: &str,
    words: &[u32],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let needed = words.len().max(1) as u64;
    if slot.words < needed {
        let size = needed.next_power_of_two().max(1024) * 4;
        counters.buffers += 1;
        counters.buffer_bytes += size;
        slot.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        slot.words = size / 4;
    }
    let buffer = slot.buffer.clone().unwrap();
    if !words.is_empty() {
        slot.staging.clear();
        slot.staging
            .extend(words.iter().flat_map(|word| word.to_le_bytes()));
        queue.write_buffer(&buffer, 0, &slot.staging);
    }
    buffer
}

fn pack_commands(
    packer: &mut AssetPacker,
    commands: &[Command],
    resources: &HashMap<u64, Resource>,
    view: ViewSpace,
    limit: usize,
) -> Result<Vec<u32>> {
    pack_records(
        packer,
        commands.iter().map(|c| (Entry::Command(c), view, 0)),
        commands.len(),
        resources,
        &[],
        limit,
    )
}

/// One terrain record: its conservative box is the clamped vertex range, which the
/// setup kernel provably never writes outside, so the raster's own bounds test is the
/// row guard and binning by the box is exact.
fn pack_terrain(
    words: &mut Vec<u32>,
    packer: &mut AssetPacker,
    triangle: &TriangleCommand,
    resources: &HashMap<u64, Resource>,
    layout: &gpoly::RowLayout,
    view: ViewSpace,
    index: u32,
) -> Result<()> {
    ensure!(
        triangle.abi_version == ABI_VERSION && triangle.reserved == 0,
        "invalid triangle ABI"
    );
    let mut offsets = [0; 2];
    for (slot, (handle, texture)) in [(triangle.source, true), (triangle.table, false)]
        .into_iter()
        .enumerate()
    {
        let resource = resources
            .get(&handle)
            .context("unknown triangle resource")?;
        ensure!(
            resource.pitch == 256
                && resource.width == if texture { 32 } else { 256 }
                && resource.height == if texture { 32 } else { 64 },
            "invalid triangle resource dimensions"
        );
        let length = if texture { 7968 } else { 16384 };
        ensure!(resource.bytes.len() >= length, "short triangle resource");
        offsets[slot] = packer
            .prefix(handle, &resource.bytes, length)
            .context("triangle assets exceed buffer limit")?;
    }
    for vertex in triangle.vertices {
        ensure!(
            (-32768..=32767).contains(&vertex.x) && (-32768..=32767).contains(&vertex.y),
            "vertex exceeds signed 16.16 coordinates"
        );
    }
    let axis = |extent: u32, of: fn(&crate::gpoly::Vertex) -> i32| {
        let edge = i64::from(extent);
        let values = triangle
            .vertices
            .iter()
            .map(|vertex| i64::from(of(vertex)).clamp(0, edge) as u32);
        let (mut lo, mut hi) = (u32::MAX, 0);
        for value in values {
            lo = lo.min(value);
            hi = hi.max(value);
        }
        (lo, hi)
    };
    let (x_lo, x_hi) = axis(view.width, |vertex| vertex.x);
    let (y_lo, y_hi) = axis(view.height, |vertex| vertex.y);
    words.extend([TERRAIN_TRI, 0, index, 0]);
    words.extend(view.rebase([x_lo, y_lo, x_hi, y_hi]));
    words.extend(view.clip([0, 0, view.width, view.height]));
    words.extend([offsets[0], offsets[1], 0, layout.base]);
    words.extend([0; 8]);
    words.extend([OPAQUE, 0, 0, 0]);
    Ok(())
}

fn pack_records<'a>(
    packer: &mut AssetPacker,
    records: impl Iterator<Item = (Entry<'a>, ViewSpace, u32)>,
    count: usize,
    resources: &HashMap<u64, Resource>,
    layout: &[gpoly::RowLayout],
    limit: usize,
) -> Result<Vec<u32>> {
    ensure!(
        count <= MAX_COMMANDS && count * RECORD_BYTES <= limit,
        "command batch exceeds limit"
    );
    let mut words = Vec::with_capacity(count * RECORD_WORDS);
    let mut terrain = 0;
    for (record, view, index) in records {
        let c = match record {
            Entry::Terrain(triangle) => {
                let entry = layout.get(terrain).context("missing triangle row layout")?;
                pack_terrain(&mut words, packer, triangle, resources, entry, view, index)?;
                terrain += 1;
                continue;
            }
            Entry::Command(command) => command,
        };
        let (width, height) = (view.width, view.height);
        ensure!(
            c.abi_version == ABI_VERSION && c.reserved == [0; 3],
            "invalid command ABI"
        );
        ensure!(
            packable(c.kind) && c.blend <= 2 && c.colour <= 255 && c.transparent <= OPAQUE,
            "invalid drawing operation"
        );
        let rectangle = if c.kind == CLEAR {
            [0, 0, width, height]
        } else {
            bounds(c.x, c.y, c.width, c.height)?
        };
        if c.kind == CIRCLE_FILLED || c.kind == CIRCLE_OUTLINE {
            ensure!(c.source_width <= 8191, "circle radius exceeds limit");
            ensure!(
                c.width == 2 * c.source_width + 1 && c.height == c.width,
                "invalid circle bounds"
            );
        }
        let clip = bounds(c.clip_x, c.clip_y, c.clip_width, c.clip_height)?;
        let mut source_offset = 0;
        let mut table_offset = 0;
        let mut source_pitch = 0;
        if matches!(
            c.kind,
            IMAGE
                | GPOLY_SPAN
                | SPRITE
                | RAW_IMAGE
                | TILED_IMAGE
                | TRIG
                | MOVIE
                | MAP_VIEW
                | BITMAP
        ) {
            let source = resources.get(&c.source).context("unknown source version")?;
            source_pitch = source.pitch;
            source_offset = packer.offset(c.source, &source.bytes)?;
            if matches!(c.kind, IMAGE | RAW_IMAGE | TILED_IMAGE) {
                ensure!(
                    c.width > 0 && c.height > 0 && c.source_width > 0 && c.source_height > 0,
                    "empty source or destination image"
                );
                ensure!(
                    u64::from(c.source_x) + u64::from(c.source_width) <= u64::from(source.width)
                        && u64::from(c.source_y) + u64::from(c.source_height)
                            <= u64::from(source.height),
                    "source rectangle exceeds asset"
                );
                if c.kind == RAW_IMAGE || c.kind == TILED_IMAGE {
                    ensure!(
                        c.blend == 0
                            && c.transparent == OPAQUE
                            && c.source_x == 0
                            && c.source_y == 0,
                        "invalid raw image options"
                    );
                }
                if c.kind == RAW_IMAGE {
                    ensure!(
                        c.x == 0 && c.y == 0 && c.width == width && c.height == height,
                        "raw image requires full target bounds"
                    );
                    ensure!(
                        (1..=16384).contains(&c.step_low)
                            && (1..=16384).contains(&c.step_high)
                            && (-16384..=16384).contains(&(c.start_low as i32))
                            && (-16384..=16384).contains(&(c.start_high as i32)),
                        "invalid raw image scaling"
                    );
                }
            } else if c.kind == BITMAP {
                bitmap::validate(c, source, width, height)?;
            } else if c.kind == MAP_VIEW {
                map_view::validate(c, source, width, height)?;
            } else if c.kind == MOVIE {
                movie::validate(c, source, width, height)?;
            } else if c.kind == TRIG {
                trig::validate(c, source, width, height)?;
            } else if c.kind == SPRITE {
                sprites::validate(c, source)?;
            } else {
                ensure!(
                    source.pitch == 256 && source.width >= 32 && source.height >= 32,
                    "gpoly texture requires 32 rows of 32 texels with pitch 256"
                );
                ensure!(
                    c.height == 1 && c.blend == 0 && c.transparent == OPAQUE,
                    "invalid gpoly span operation"
                );
            }
        }
        if c.blend != 0 || c.kind == GPOLY_SPAN || c.kind == TRIG {
            let table = resources.get(&c.table).context("unknown table version")?;
            ensure!(
                table.width == 256 && table.pitch == 256,
                "invalid lookup table layout"
            );
            if c.kind == TRIG {
                ensure!(
                    table.height == 320,
                    "triangle requires fade and ghost tables"
                );
            }
            if c.blend != 0 {
                ensure!(table.height >= 256, "blend table requires 256 rows");
            }
            if c.kind == GPOLY_SPAN {
                let mut low = c.start_low;
                for _ in 0..c.width {
                    ensure!((low >> 8) & 255 < table.height, "gpoly shade exceeds table");
                    low = low.wrapping_add(c.step_low);
                }
            }
            table_offset = packer.offset(c.table, &table.bytes)?;
        }
        words.extend([c.kind, c.blend, index, c.colour]);
        words.extend(view.rebase(rectangle));
        words.extend(view.clip(clip));
        words.extend([source_offset, table_offset, source_pitch, 0]);
        words.extend([c.source_x, c.source_y, c.source_width, c.source_height]);
        words.extend([c.start_low, c.start_high, c.step_low, c.step_high]);
        words.extend([c.transparent, 0, 0, 0]);
    }
    Ok(words)
}

fn buffer(
    device: &wgpu::Device,
    counters: &mut Counters,
    label: &str,
    words: &[u32],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let bytes: Vec<_> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
    counters.buffers += 1;
    counters.buffer_bytes += bytes.len() as u64;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &bytes,
        usage,
    })
}

/// The frame's terrain setup, waiting for the first raster segment's encoder.
pub(super) struct PendingPrepare {
    pub(super) triangles: Vec<gpoly::Triangle>,
    pub(super) layout: Vec<gpoly::RowLayout>,
    pub(super) rows: wgpu::Buffer,
}

/// A reserved timestamp pair, empty when the run is not timing passes.
pub(super) struct Stamp(Option<(wgpu::QuerySet, u32, u32)>);

impl Stamp {
    pub(super) fn compute(&self) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        self.0
            .as_ref()
            .map(|(set, begin, end)| wgpu::ComputePassTimestampWrites {
                query_set: set,
                beginning_of_pass_write_index: Some(*begin),
                end_of_pass_write_index: Some(*end),
            })
    }

    pub(super) fn render(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.0
            .as_ref()
            .map(|(set, begin, end)| wgpu::RenderPassTimestampWrites {
                query_set: set,
                beginning_of_pass_write_index: Some(*begin),
                end_of_pass_write_index: Some(*end),
            })
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// One ascending per-tile list of record indices for the whole frame, built by a
/// counting sort into buffers the renderer keeps. `segments` are the exclusive
/// record ends of the raster passes the frame is cut into. Each pass gets a header
/// of `(offset, length)` pairs covering only the tiles its own records touch,
/// followed by the shared entry array, so the passes together iterate each tile
/// list exactly once and a pass costs nothing for tiles it never reaches.
#[derive(Default)]
pub(super) struct TileIndex {
    counts: Vec<u32>,
    cursors: Vec<u32>,
    packed: Vec<u32>,
    passes: Vec<Pass>,
    length: usize,
    header: usize,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Pass {
    /// Tile box, half-open, in tile units.
    tiles: [u32; 4],
    header: u32,
    counts: usize,
}

impl Pass {
    fn columns(&self) -> u32 {
        self.tiles[2].saturating_sub(self.tiles[0])
    }

    fn rows(&self) -> u32 {
        self.tiles[3].saturating_sub(self.tiles[1])
    }

    fn cells(&self) -> usize {
        self.columns() as usize * self.rows() as usize
    }

    fn cell(&self, x: u32, y: u32) -> usize {
        (y - self.tiles[1]) as usize * self.columns() as usize + (x - self.tiles[0]) as usize
    }
}

/// The pixel box a pass must dispatch over, empty when it draws nothing.
pub(super) fn pass_box(pass: &Pass, width: u32, height: u32) -> [u32; 4] {
    if pass.columns() == 0 || pass.rows() == 0 {
        return [0; 4];
    }
    [
        pass.tiles[0] * 16,
        pass.tiles[1] * 16,
        (pass.tiles[2] * 16).min(width),
        (pass.tiles[3] * 16).min(height),
    ]
}

fn tile_span(
    command: &[u32; RECORD_WORDS],
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let x0 = (command[4] as i32)
        .max(command[8] as i32)
        .max(0)
        .min(width as i32) as u32;
    let y0 = (command[5] as i32)
        .max(command[9] as i32)
        .max(0)
        .min(height as i32) as u32;
    let x1 = (command[6] as i32)
        .min(command[10] as i32)
        .max(0)
        .min(width as i32) as u32;
    let y1 = (command[7] as i32)
        .min(command[11] as i32)
        .max(0)
        .min(height as i32) as u32;
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    Some((x0 / 16, y0 / 16, x1.div_ceil(16), y1.div_ceil(16)))
}

fn grow(vec: &mut Vec<u32>, length: usize, allocations: &mut u64) {
    if vec.len() >= length {
        return;
    }
    let capacity = vec.capacity();
    vec.resize(length, 0);
    if vec.capacity() != capacity {
        *allocations += 1;
    }
}

impl TileIndex {
    fn build(
        &mut self,
        counters: &mut Counters,
        words: &[u32],
        views: &[u32],
        segments: &[usize],
        extent: (u32, u32),
        limit: usize,
    ) -> Result<()> {
        let (width, height) = extent;
        let records = words.as_chunks::<RECORD_WORDS>().0;
        let count = segments.len().max(1);
        self.passes.clear();
        self.passes.resize(count, Pass::default());
        for pass in self.passes.iter_mut() {
            pass.tiles = [u32::MAX, u32::MAX, 0, 0];
        }
        let mut entries = 0usize;
        let mut terrain = 0usize;
        let mut at = 0;
        for (index, command) in records.iter().enumerate() {
            while segments.get(at).is_some_and(|end| index >= *end) {
                at += 1;
            }
            let Some((x0, y0, x1, y1)) = tile_span(command, width, height) else {
                continue;
            };
            let covered = (x1 - x0) as usize * (y1 - y0) as usize;
            if command[0] == TERRAIN_TRI {
                terrain += covered;
            }
            entries = entries
                .checked_add(covered)
                .context("tile list length overflow")?;
            let box_of = &mut self.passes[at].tiles;
            box_of[0] = box_of[0].min(x0);
            box_of[1] = box_of[1].min(y0);
            box_of[2] = box_of[2].max(x1);
            box_of[3] = box_of[3].max(y1);
        }
        let mut header = views.len();
        let mut cells = 0usize;
        for pass in self.passes.iter_mut() {
            pass.header = u32::try_from(header).context("tile header overflow")?;
            pass.counts = cells;
            header += pass.cells() * 2;
            cells += pass.cells();
        }
        let length = header
            .checked_add(entries)
            .context("tile list length overflow")?;
        ensure!(length <= limit / 4, "tile lists exceed storage limit");
        grow(
            &mut self.counts,
            cells.max(1),
            &mut counters.tile_allocations,
        );
        grow(
            &mut self.cursors,
            cells.max(1),
            &mut counters.tile_allocations,
        );
        grow(
            &mut self.packed,
            length.max(1),
            &mut counters.tile_allocations,
        );
        self.counts[..cells].fill(0);
        self.packed[..views.len()].copy_from_slice(views);
        self.length = length;
        self.header = header;
        at = 0;
        for (index, command) in records.iter().enumerate() {
            while segments.get(at).is_some_and(|end| index >= *end) {
                at += 1;
            }
            let Some((x0, y0, x1, y1)) = tile_span(command, width, height) else {
                continue;
            };
            let pass = self.passes[at];
            for y in y0..y1 {
                for x in x0..x1 {
                    self.counts[pass.counts + pass.cell(x, y)] += 1;
                }
            }
        }
        let mut offset = header as u32;
        for pass in &self.passes {
            for cell in 0..pass.cells() {
                let total = self.counts[pass.counts + cell];
                self.packed[pass.header as usize + cell * 2] = offset;
                self.packed[pass.header as usize + cell * 2 + 1] = total;
                self.cursors[pass.counts + cell] = offset;
                offset += total;
            }
        }
        at = 0;
        for (index, command) in records.iter().enumerate() {
            while segments.get(at).is_some_and(|end| index >= *end) {
                at += 1;
            }
            let Some((x0, y0, x1, y1)) = tile_span(command, width, height) else {
                continue;
            };
            let pass = self.passes[at];
            for y in y0..y1 {
                for x in x0..x1 {
                    let cursor = &mut self.cursors[pass.counts + pass.cell(x, y)];
                    self.packed[*cursor as usize] = index as u32;
                    *cursor += 1;
                }
            }
        }
        counters.tile_entries += entries as u64;
        counters.terrain_tile_entries += terrain as u64;
        Ok(())
    }

    fn data(&self) -> &[u32] {
        &self.packed[..self.length]
    }

    fn passes(&self) -> Vec<Pass> {
        self.passes.clone()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_mirrored_kinds_pack_into_a_batch() {
        // WgpuTerrainBridge::PacksInBatch mirrors this set; keep the two in step.
        for kind in [
            CLEAR,
            RECT,
            IMAGE,
            GPOLY_SPAN,
            CIRCLE_FILLED,
            CIRCLE_OUTLINE,
            SPRITE,
            RAW_IMAGE,
            TILED_IMAGE,
            TRIG,
            MOVIE,
            MAP_VIEW,
            BITMAP,
        ] {
            assert!(packable(kind), "kind {kind} must pack");
        }
        for kind in [LENS_EFFECT, SHADOW, MINIMAP, TRANSITION] {
            assert!(!packable(kind), "kind {kind} must stay solo");
        }
    }

    use super::*;

    #[test]
    fn command_abi_and_rejection_are_gpu_independent() {
        assert_eq!(std::mem::size_of::<Command>(), 112);
        assert_eq!(std::mem::offset_of!(Command, source), 48);
        assert_eq!(std::mem::offset_of!(Command, transparent), 96);
        let resources = HashMap::new();
        for command in [
            Command {
                abi_version: 2,
                ..Default::default()
            },
            Command {
                kind: 9,
                ..Default::default()
            },
            Command {
                reserved: [1, 0, 0],
                ..Default::default()
            },
            Command {
                x: i32::MAX,
                ..Default::default()
            },
            Command {
                kind: IMAGE,
                source: 999,
                ..Default::default()
            },
            Command {
                blend: 1,
                ..Default::default()
            },
            Command {
                transparent: 257,
                ..Default::default()
            },
        ] {
            let mut packer = AssetPacker::batch(1 << 20);
            assert!(
                pack_commands(
                    &mut packer,
                    &[command],
                    &resources,
                    ViewSpace::whole(32, 32),
                    1 << 20
                )
                .is_err()
            );
        }
        assert!(validate_resource(31, 32, 1, 32).is_err());
        assert!(validate_resource(256, 32, 8, 31).is_err());
        assert!(validate_resource(0, 0, 0, 0).is_err());
        assert!(validate_resource(7968, 32, 32, 256).is_ok());
    }

    #[test]
    fn tile_lists_preserve_overlap_order_and_reject_overflow() {
        let commands = [
            Command {
                width: 32,
                height: 32,
                ..Default::default()
            },
            Command {
                x: 15,
                y: 15,
                width: 2,
                height: 2,
                ..Default::default()
            },
            Command {
                x: -10,
                y: -10,
                width: 11,
                height: 11,
                ..Default::default()
            },
        ];
        let mut packer = AssetPacker::batch(1 << 20);
        let words = pack_commands(
            &mut packer,
            &commands,
            &HashMap::new(),
            ViewSpace::whole(32, 32),
            1 << 20,
        )
        .unwrap();
        let mut index = TileIndex::default();
        let mut counters = Counters::default();
        let table = ViewSpace::table(&[ViewSpace::whole(32, 32)]);
        let list = |index: &TileIndex, pass: usize, x: u32, y: u32| {
            let pass = index.passes()[pass];
            let tiles = index.data();
            let cell = pass.header as usize + pass.cell(x, y) * 2;
            let begin = tiles[cell] as usize;
            tiles[begin..begin + tiles[cell + 1] as usize].to_vec()
        };
        index
            .build(
                &mut counters,
                &words,
                &table,
                &[commands.len()],
                (32, 32),
                4096,
            )
            .unwrap();
        assert_eq!(list(&index, 0, 0, 0), [0, 1, 2]);
        for (x, y) in [(1, 0), (0, 1), (1, 1)] {
            assert_eq!(list(&index, 0, x, y), [0, 1]);
        }
        assert_eq!(counters.tile_entries, 9);
        let allocations = counters.tile_allocations;
        index
            .build(
                &mut counters,
                &words,
                &table,
                &[commands.len()],
                (32, 32),
                4096,
            )
            .unwrap();
        assert_eq!(
            counters.tile_allocations, allocations,
            "a rebuilt index of the same shape must not allocate"
        );
        // Segment ends split the same lists without reordering or duplicating entries.
        index
            .build(
                &mut counters,
                &words,
                &table,
                &[1, commands.len()],
                (32, 32),
                4096,
            )
            .unwrap();
        assert_eq!(list(&index, 0, 0, 0), [0]);
        assert_eq!(list(&index, 1, 0, 0), [1, 2]);
        assert_eq!(list(&index, 1, 1, 1), [1]);
        assert_eq!(
            pass_box(&index.passes()[1], 32, 32),
            [0, 0, 32, 32],
            "a pass covers only the tiles its own records reach"
        );
        assert!(
            index
                .build(
                    &mut counters,
                    &words,
                    &table,
                    &[commands.len()],
                    (32, 32),
                    8 * 4
                )
                .is_err()
        );
    }

    fn renderer() -> (crate::gpu::Renderer, DrawRenderer) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        eprintln!("drawing fixtures adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let renderer = crate::gpu::Renderer::new(device, queue).unwrap();
        let drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
        (renderer, drawing)
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_ordered_drawing_resource_lifetime_and_presentation() {
        let (renderer, mut drawing) = renderer();
        let target = drawing.create_target(33, 19).unwrap();
        let other = drawing.create_target(33, 19).unwrap();
        let mut bytes = vec![1, 2, 3, 255, 4, 5, 6, 255];
        let source = drawing.create_resource(&bytes, 3, 2, 4).unwrap();
        bytes.fill(200);
        let table: Vec<u8> = (0..65536)
            .map(|i| ((i >> 8) * 7 + (i & 255) * 3) as u8)
            .collect();
        let lookup = drawing.create_resource(&table, 256, 256, 256).unwrap();
        let commands = [
            Command {
                kind: CLEAR,
                colour: 17,
                ..Default::default()
            },
            Command {
                x: -2,
                y: -1,
                width: 20,
                height: 10,
                colour: 9,
                clip_x: 1,
                clip_y: 2,
                clip_width: 20,
                clip_height: 10,
                ..Default::default()
            },
            Command {
                kind: IMAGE,
                source,
                width: 9,
                height: 4,
                x: 15,
                y: 8,
                source_width: 3,
                source_height: 2,
                transparent: 2,
                ..Default::default()
            },
            Command {
                x: 16,
                y: 9,
                width: 2,
                height: 2,
                colour: 13,
                blend: 1,
                table: lookup,
                ..Default::default()
            },
            Command {
                x: 17,
                y: 10,
                width: 3,
                height: 3,
                colour: 11,
                blend: 2,
                table: lookup,
                ..Default::default()
            },
        ];
        drawing.submit(target, &commands).unwrap();
        drawing.release_resource(source).unwrap();
        drawing.release_resource(lookup).unwrap();
        let mut expected = vec![17u8; 33 * 19];
        for y in 2..9 {
            for x in 1..18 {
                expected[y * 33 + x] = 9;
            }
        }
        for y in 0..4 {
            for x in 0..9 {
                let index = [1u8, 2, 3, 4, 5, 6][y / 2 * 3 + x / 3];
                if index != 2 {
                    expected[(y + 8) * 33 + x + 15] = index;
                }
            }
        }
        for y in 9..11 {
            for x in 16..18 {
                let dst = &mut expected[y * 33 + x];
                *dst = table[(13 << 8) | *dst as usize];
            }
        }
        for y in 10..13 {
            for x in 17..20 {
                let dst = &mut expected[y * 33 + x];
                *dst = table[((*dst as usize) << 8) | 11];
            }
        }
        assert!(drawing.submit(target, &commands).is_err());
        assert_eq!(drawing.readback(target).unwrap(), expected);
        assert_eq!(drawing.readback(other).unwrap(), vec![0; 33 * 19]);
        let palette: Vec<u8> = (0..256)
            .flat_map(|i| [i as u8, (255 - i) as u8, (i * 17) as u8, 255])
            .collect();
        let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("GPU drawing presentation fixture"),
            size: wgpu::Extent3d {
                width: 67,
                height: 39,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        drawing
            .present_into(
                target,
                &palette,
                67,
                39,
                &texture.create_view(&Default::default()),
            )
            .unwrap();
        drawing.release_target(target).unwrap();
        let staging = renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 512 * 39,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = renderer
            .device()
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        renderer.queue().submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| sender.send(r).unwrap());
        renderer
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let actual = staging.slice(..).get_mapped_range().unwrap();
        for y in 0..39 {
            for x in 0..67 {
                let sx = (2 * x + 1) * 33 / (2 * 67);
                let sy = (2 * y + 1) * 19 / (2 * 39);
                let index = expected[sy * 33 + sx] as usize;
                assert_eq!(
                    &actual[y * 512 + x * 4..y * 512 + x * 4 + 4],
                    &palette[index * 4..index * 4 + 4]
                );
            }
        }
        drop(actual);
        staging.unmap();
        renderer.check_status().unwrap();
        assert!(drawing.release_target(target).is_err());
        assert_eq!(drawing.counters().batches, 1);
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_clipped_image_subregion_and_cross_context_rejection() {
        let (renderer, mut drawing) = renderer();
        let target = drawing.create_target(4, 4).unwrap();
        let mut foreign = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let foreign_target = foreign.create_target(4, 4).unwrap();
        let bytes: Vec<u8> = (0..150)
            .map(|i| {
                if i % 17 < 14 {
                    (i / 17 * 14 + i % 17) as u8
                } else {
                    255
                }
            })
            .collect();
        let source = drawing.create_resource(&bytes, 14, 9, 17).unwrap();
        let foreign_source = foreign.create_resource(&bytes, 14, 9, 17).unwrap();
        let table: Vec<u8> = (0..65536)
            .map(|i| ((i >> 8) * 7 + (i & 255) * 3) as u8)
            .collect();
        let lookup = drawing.create_resource(&table, 256, 256, 256).unwrap();
        let clear = Command {
            kind: CLEAR,
            colour: 19,
            ..Default::default()
        };
        let image = Command {
            kind: IMAGE,
            source,
            table: lookup,
            blend: 1,
            x: -2,
            y: -1,
            width: 7,
            height: 5,
            clip_x: 1,
            clip_y: 0,
            clip_width: 3,
            clip_height: 3,
            source_x: 2,
            source_y: 1,
            source_width: 11,
            source_height: 7,
            transparent: 50,
            ..Default::default()
        };
        drawing.submit(target, &[clear, image]).unwrap();
        let expected = vec![
            19, 39, 53, 60, 19, 137, 19, 158, 19, 77, 91, 98, 19, 19, 19, 19,
        ];
        assert_eq!(drawing.readback(target).unwrap(), expected);
        assert!(drawing.submit(foreign_target, &[clear]).is_err());
        assert!(drawing.readback(foreign_target).is_err());
        assert!(drawing.release_target(foreign_target).is_err());
        assert!(drawing.release_resource(foreign_source).is_err());
        assert!(
            drawing
                .submit(
                    target,
                    &[
                        clear,
                        Command {
                            source: foreign_source,
                            ..image
                        }
                    ]
                )
                .is_err()
        );
        assert_eq!(drawing.readback(target).unwrap(), expected);
        assert_eq!(foreign.readback(foreign_target).unwrap(), vec![0; 16]);
        assert_eq!(drawing.counters().batches, 1);
        assert_eq!(drawing.counters().commands, 2);
        drawing.release_resource(source).unwrap();
        let replacement = drawing.create_resource(&bytes, 14, 9, 17).unwrap();
        assert_ne!(replacement, source);
        assert!(drawing.submit(target, &[clear, image]).is_err());
        assert_eq!(drawing.readback(target).unwrap(), expected);
        renderer.check_status().unwrap();
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_gpoly_wrapping_carries_clips_and_invalid_shades() {
        let (renderer, mut drawing) = renderer();
        let target = drawing.create_target(79, 32).unwrap();
        let texture: Vec<u8> = (0..8192).map(|i| (i * 7 + i / 256 * 11) as u8).collect();
        let fade: Vec<u8> = (0..65536).map(|i| (i * 13 + i / 256 * 3) as u8).collect();
        let source = drawing.create_resource(&texture, 32, 32, 256).unwrap();
        let table = drawing.create_resource(&fade, 256, 256, 256).unwrap();
        let mut commands = Vec::new();
        let mut expected = vec![0; 79 * 32];
        for y in 0..32u32 {
            let start = 0xffff_ff00_ffff_fffeu64.wrapping_add(u64::from(y) * 0x91ab_6171_1267);
            let step = 0xfeab_9876_fedc_ba98u64.wrapping_add(u64::from(y) * 0x3213_7699);
            let c = Command {
                kind: GPOLY_SPAN,
                x: -5,
                y: y as i32,
                width: 89,
                height: 1,
                source,
                table,
                start_low: start as u32,
                start_high: (start >> 32) as u32,
                step_low: step as u32,
                step_high: (step >> 32) as u32,
                clip_x: 2,
                clip_width: 71,
                ..Default::default()
            };
            for x in 2..73usize {
                let position = start.wrapping_add(step.wrapping_mul(x as u64 + 5));
                let uv = ((position >> 32) as u32).rotate_left(8) & 0x1f1f;
                let shade = position as usize & 0xff00;
                expected[y as usize * 79 + x] = fade[shade | texture[uv as usize] as usize];
            }
            commands.push(c);
        }
        drawing.submit(target, &commands).unwrap();
        assert_eq!(drawing.readback(target).unwrap(), expected);
        let short_table = drawing
            .create_resource(&fade[..16384], 256, 64, 256)
            .unwrap();
        let mut invalid = commands[0];
        invalid.table = short_table;
        assert!(
            drawing
                .submit(
                    target,
                    &[
                        Command {
                            kind: CLEAR,
                            colour: 87,
                            ..Default::default()
                        },
                        invalid
                    ]
                )
                .is_err()
        );
        assert_eq!(drawing.readback(target).unwrap(), expected);
        renderer.check_status().unwrap();
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_device_loss_rejects_further_draws() {
        let (renderer, mut drawing) = renderer();
        let target = drawing.create_target(4, 4).unwrap();
        drawing
            .submit(
                target,
                &[Command {
                    kind: CLEAR,
                    colour: 17,
                    ..Default::default()
                }],
            )
            .unwrap();
        renderer.device().destroy();
        renderer
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        assert!(drawing.check_status().is_err());
        assert!(drawing.submit(target, &[Command::default()]).is_err());
        assert!(drawing.readback(target).is_err());
        assert!(drawing.create_target(4, 4).is_err());
    }

    #[test]
    #[ignore = "requires GPU and KFX_GPOLY_FIXTURE path generated by gpoly_capture_test"]
    fn gpu_native_gpoly_capture_matches_legacy_pixels() {
        let path = std::env::var("KFX_GPOLY_FIXTURE").expect("set KFX_GPOLY_FIXTURE");
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..8], b"KFXGSPN1");
        let mut position = 8;
        fn word(bytes: &[u8], position: &mut usize) -> u32 {
            let value = u32::from_le_bytes(bytes[*position..*position + 4].try_into().unwrap());
            *position += 4;
            value
        }
        let width = word(&bytes, &mut position);
        let height = word(&bytes, &mut position);
        let pitch = word(&bytes, &mut position);
        let count = word(&bytes, &mut position);
        let textures = word(&bytes, &mut position);
        let fades = word(&bytes, &mut position);
        let mut spans = Vec::new();
        for _ in 0..count {
            spans.push(std::array::from_fn::<_, 9, _>(|_| {
                word(&bytes, &mut position)
            }));
        }
        let (renderer, mut drawing) = renderer();
        let target = drawing.create_target(width, height).unwrap();
        let mut sources = Vec::new();
        for _ in 0..textures {
            sources.push(
                drawing
                    .create_resource(&bytes[position..position + 8192], 32, 32, 256)
                    .unwrap(),
            );
            position += 8192;
        }
        let mut tables = Vec::new();
        for _ in 0..fades {
            tables.push(
                drawing
                    .create_resource(&bytes[position..position + 16384], 256, 64, 256)
                    .unwrap(),
            );
            position += 16384;
        }
        let length = (pitch * height) as usize;
        let initial = drawing
            .create_resource(&bytes[position..position + length], width, height, pitch)
            .unwrap();
        position += length;
        let expected: Vec<_> = bytes[position..position + length]
            .chunks_exact(pitch as usize)
            .flat_map(|row| row[..width as usize].iter().copied())
            .collect();
        assert_eq!(position + length, bytes.len());
        let mut commands = vec![Command {
            kind: IMAGE,
            source: initial,
            width,
            height,
            source_width: width,
            source_height: height,
            ..Default::default()
        }];
        for s in spans {
            commands.push(Command {
                kind: GPOLY_SPAN,
                x: s[0] as i32,
                y: s[1] as i32,
                width: s[2],
                height: 1,
                start_low: s[3],
                start_high: s[4],
                step_low: s[5],
                step_high: s[6],
                source: sources[s[7] as usize],
                table: tables[s[8] as usize],
                ..Default::default()
            });
        }
        drawing.submit(target, &commands).unwrap();
        for id in sources.into_iter().chain(tables).chain([initial]) {
            drawing.release_resource(id).unwrap();
        }
        assert_eq!(drawing.readback(target).unwrap(), expected);
        renderer.check_status().unwrap();
        eprintln!(
            "native fixture exact: {} spans, {}x{}, {} texture versions, {} fade versions",
            count, width, height, textures, fades
        );
    }
    #[test]
    #[ignore = "requires GPU and KFX_PRIMITIVE_FIXTURE generated by primitive_fixture"]
    fn gpu_actual_legacy_primitives() {
        let path = std::env::var("KFX_PRIMITIVE_FIXTURE").expect("set KFX_PRIMITIVE_FIXTURE");
        let bytes = std::fs::read(path).unwrap();
        let word =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(word(0), 0x3250464b);
        let count = word(4) as usize;
        let width = word(8);
        let height = word(12);
        assert_eq!(word(16), 112);
        let size = (width * height) as usize;
        assert_eq!(bytes.len(), 20 + count * (112 + size));
        let (_, mut drawing) = renderer();
        let target = drawing.create_target(width, height).unwrap();
        let initial: Vec<u8> = (0..size)
            .map(|i| (i * 19 + i / width as usize * 13) as u8)
            .collect();
        let source = drawing
            .create_resource(&initial, width, height, width)
            .unwrap();
        let glass: Vec<u8> = (0..65536)
            .map(|i| ((i >> 8) * 7 + (i & 255) * 3 + 17) as u8)
            .collect();
        let table = drawing.create_resource(&glass, 256, 256, 256).unwrap();
        for fixture in 0..count {
            let offset = 20 + fixture * (112 + size);
            let w = |i: usize| word(offset + i * 4);
            let command = Command {
                abi_version: w(0),
                kind: w(1),
                blend: w(2),
                colour: w(3),
                x: w(4) as i32,
                y: w(5) as i32,
                width: w(6),
                height: w(7),
                clip_x: w(8) as i32,
                clip_y: w(9) as i32,
                clip_width: w(10),
                clip_height: w(11),
                table,
                source_x: w(16),
                source_y: w(17),
                source_width: w(18),
                source_height: w(19),
                start_low: w(20),
                start_high: w(21),
                step_low: w(22),
                step_high: w(23),
                transparent: w(24),
                ..Default::default()
            };
            drawing
                .submit(
                    target,
                    &[
                        Command {
                            kind: IMAGE,
                            source,
                            width,
                            height,
                            source_width: width,
                            source_height: height,
                            ..Default::default()
                        },
                        command,
                    ],
                )
                .unwrap();
            let actual = drawing.readback(target).unwrap();
            let expected = &bytes[offset + 112..offset + 112 + size];
            if let Some(pixel) = actual.iter().zip(expected).position(|(a, b)| a != b) {
                panic!(
                    "fixture {fixture} {command:?}: pixel ({},{}) GPU={} legacy={}",
                    pixel % width as usize,
                    pixel / width as usize,
                    actual[pixel],
                    expected[pixel]
                );
            }
        }
        eprintln!("{count} exact actual-legacy primitive GPU fixtures passed");
    }
}
