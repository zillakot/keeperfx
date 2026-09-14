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

struct Resource {
    width: u32,
    height: u32,
    pitch: u32,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct Target {
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
}

pub struct DrawRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    compute: wgpu::ComputePipeline,
    compute_sprite_ordered: wgpu::ComputePipeline,
    effects: Option<wgpu::ComputePipeline>,
    trig_validate: Option<wgpu::ComputePipeline>,
    shadow: Option<wgpu::ComputePipeline>,
    minimap: Option<minimap::MinimapState>,
    triangles: Option<triangles::TrianglePipelines>,
    present: wgpu::RenderPipeline,
    targets: HashMap<u64, Target>,
    resources: HashMap<u64, Resource>,
    resource_bytes: usize,
    target_snapshots: HashMap<u64, target_resources::TargetSnapshot>,
    target_resource_counters: TargetResourceCounters,
    counters: Counters,
    frame: Option<frame_queue::QueuedFrame>,
    frame_counters: FrameCounters,
    deferred_status: Option<Vec<wgpu::Buffer>>,
    deferred_snapshot_releases: Vec<u64>,
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
        Ok(Self {
            device,
            queue,
            compute,
            compute_sprite_ordered,
            effects: None,
            trig_validate: None,
            shadow: None,
            minimap: None,
            triangles: None,
            present,
            targets: HashMap::new(),
            resources: HashMap::new(),
            resource_bytes: 0,
            target_snapshots: HashMap::new(),
            target_resource_counters: TargetResourceCounters::default(),
            counters: Counters::default(),
            frame: None,
            frame_counters: FrameCounters::default(),
            deferred_status: None,
            deferred_snapshot_releases: Vec::new(),
            failure: renderer.failure.clone(),
        })
    }

    pub fn headless() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
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
        self.counters
    }

    /// Host-side staged asset bytes the drawing context holds; not GPU memory and
    /// not a window delta.
    pub fn staged_asset_bytes(&self) -> u64 {
        self.resource_bytes as u64
    }

    pub(super) fn submit_encoder(&mut self, encoder: wgpu::CommandEncoder) {
        self.counters.submits += 1;
        self.queue.submit([encoder.finish()]);
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
        if commands.iter().any(|c| c.kind == TRIG) {
            self.prepare_trig();
        }
        let (target_width, target_height) = self.target_dimensions(target)?;
        let (words, assets) = pack_commands(
            commands,
            &self.resources,
            target_width,
            target_height,
            self.storage_limit() as usize,
        )?;
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
        let tiles = bin_commands(
            &words,
            target.width,
            target.height,
            self.storage_limit() as usize,
        )?;
        let tile_buffer = buffer(
            &self.device,
            &mut self.counters,
            "ordered tile lists",
            &tiles,
            wgpu::BufferUsages::STORAGE,
        );
        let command_buffer = buffer(
            &self.device,
            &mut self.counters,
            "immutable ordered commands",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let asset_buffer = buffer(
            &self.device,
            &mut self.counters,
            "immutable asset versions",
            &assets,
            wgpu::BufferUsages::STORAGE,
        );
        let parameters = buffer(
            &self.device,
            &mut self.counters,
            "drawing dimensions",
            &[
                target.width,
                target.height,
                commands.len() as u32,
                target.width.div_ceil(16),
                target.pitch,
                target.offset,
                0,
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        self.counters.asset_upload_bytes += assets.len() as u64 * 4;
        self.counters.command_upload_bytes += (words.len() + tiles.len()) as u64 * 4;
        if commands.iter().any(|c| c.kind == TRIG) {
            let valid = self.validate_trig_batch(
                &command_buffer,
                &asset_buffer,
                &parameters,
                target.width,
                target.height,
            )?;
            if self.deferred_status.is_none() {
                self.counters.readback_bytes += 4;
            }
            self.counters.command_upload_bytes += 4;
            ensure!(valid, "triangle has an invalid lookup");
        }
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ordered drawing batch"),
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                entry(0, &target.indices),
                entry(1, &command_buffer),
                entry(2, &asset_buffer),
                entry(3, &parameters),
                entry(4, &tile_buffer),
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exclusive destination pixel ownership"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(target.width.div_ceil(8), target.height.div_ceil(8), 1);
        }
        self.counters.dispatches += 1;
        self.submit_encoder(encoder);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;

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
        let mut encoder = self.device.create_command_encoder(&Default::default());
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
                timestamp_writes: None,
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

/// Kinds `pack_commands` accepts in a multi-command batch. `WgpuTerrainBridge::PacksInBatch`
/// mirrors this set; a change here needs the same change there.
pub(crate) fn packable(kind: u32) -> bool {
    kind <= TRIG || kind == MOVIE || kind == MAP_VIEW || kind == BITMAP
}

fn pack_commands(
    commands: &[Command],
    resources: &HashMap<u64, Resource>,
    width: u32,
    height: u32,
    limit: usize,
) -> Result<(Vec<u32>, Vec<u32>)> {
    ensure!(
        commands.len() <= MAX_COMMANDS && commands.len() * 112 <= limit,
        "command batch exceeds limit"
    );
    let mut words = Vec::with_capacity(commands.len() * 28);
    let mut assets = Vec::new();
    let mut offsets = HashMap::new();
    for c in commands {
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
            source_offset = pack_resource(c.source, source, &mut offsets, &mut assets, limit)?;
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
            table_offset = pack_resource(c.table, table, &mut offsets, &mut assets, limit)?;
        }
        words.extend([c.kind, c.blend, 0, c.colour]);
        words.extend(rectangle);
        words.extend(clip);
        words.extend([source_offset, table_offset, source_pitch, 0]);
        words.extend([c.source_x, c.source_y, c.source_width, c.source_height]);
        words.extend([c.start_low, c.start_high, c.step_low, c.step_high]);
        words.extend([c.transparent, 0, 0, 0]);
    }
    if assets.is_empty() {
        assets.push(0);
    }
    Ok((words, assets))
}

fn pack_resource(
    id: u64,
    resource: &Resource,
    offsets: &mut HashMap<u64, u32>,
    assets: &mut Vec<u32>,
    limit: usize,
) -> Result<u32> {
    if let Some(offset) = offsets.get(&id) {
        return Ok(*offset);
    }
    ensure!(
        assets
            .len()
            .checked_add(resource.bytes.len())
            .context("asset length overflow")?
            <= limit / 4,
        "asset batch exceeds storage limit"
    );
    let offset = assets.len() as u32;
    assets.extend(resource.bytes.iter().map(|byte| u32::from(*byte)));
    offsets.insert(id, offset);
    Ok(offset)
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

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn bin_commands(words: &[u32], width: u32, height: u32, limit: usize) -> Result<Vec<u32>> {
    let columns = width.div_ceil(16);
    let rows = height.div_ceil(16);
    let mut lists = vec![Vec::new(); (columns * rows) as usize];
    let mut length = lists.len() * 2;
    for (index, command) in words.as_chunks::<28>().0.iter().enumerate() {
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
            continue;
        }
        let count = (x1.div_ceil(16) - x0 / 16) as usize * (y1.div_ceil(16) - y0 / 16) as usize;
        length = length
            .checked_add(count)
            .context("tile list length overflow")?;
        ensure!(length <= limit / 4, "tile lists exceed storage limit");
        for y in y0 / 16..y1.div_ceil(16) {
            for x in x0 / 16..x1.div_ceil(16) {
                lists[(y * columns + x) as usize].push(index as u32);
            }
        }
    }
    let mut packed = vec![0; lists.len() * 2];
    for (tile, list) in lists.iter().enumerate() {
        packed[tile * 2] = packed.len() as u32;
        packed[tile * 2 + 1] = list.len() as u32;
        packed.extend(list);
    }
    Ok(packed)
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
            assert!(pack_commands(&[command], &resources, 32, 32, 1 << 20).is_err());
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
        let (words, _) = pack_commands(&commands, &HashMap::new(), 32, 32, 1 << 20).unwrap();
        let tiles = bin_commands(&words, 32, 32, 1024).unwrap();
        for (tile, expected) in [&[0, 1, 2][..], &[0, 1], &[0, 1], &[0, 1]]
            .iter()
            .enumerate()
        {
            let offset = tiles[tile * 2] as usize;
            let count = tiles[tile * 2 + 1] as usize;
            assert_eq!(&tiles[offset..offset + count], *expected);
        }
        assert!(bin_commands(&words, 32, 32, 8 * 4).is_err());
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
