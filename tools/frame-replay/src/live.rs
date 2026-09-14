#[path = "live_frame.rs"]
mod frame_queue;
#[path = "live_shadow.rs"]
mod shadow;
#[path = "live_target_resources.rs"]
mod target_resources;

#[cfg(target_os = "macos")]
use crate::gpu::{Renderer, validate_rows};
#[cfg(target_os = "macos")]
use anyhow::{Context, bail};
use anyhow::{Result, ensure};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    ffi::{c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicU64, Ordering},
};

struct CountingAllocator;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            count_allocation(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            count_allocation(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            count_allocation(size);
        }
        pointer
    }
}
fn count_allocation(size: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    REQUESTED_BYTES.fetch_add(size as u64, Ordering::Relaxed);
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct PresentCounters {
    acquire_ns: u64,
    acquire_block_ns: u64,
    reconfigure_count: u64,
    present_record_ns: u64,
    submit_ns: u64,
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_present_counters(
    handle: *mut c_void,
    output: *mut PresentCounters,
) {
    if !handle.is_null() && !output.is_null() {
        unsafe {
            *output = std::mem::take(&mut (*handle.cast::<Presenter>()).counters);
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct Slot {
    texture: Option<wgpu::Texture>,
    last: Option<wgpu::SubmissionIndex>,
}

/// Where a presented frame lands. Only acquisition and presentation differ between
/// the two; the palette pass and the drawing context render into a plain view either way.
#[cfg(target_os = "macos")]
enum Target {
    Swapchain {
        surface: wgpu::Surface<'static>,
        layer: *mut c_void,
        config: wgpu::SurfaceConfiguration,
        modes: Vec<wgpu::PresentMode>,
        pending: Option<wgpu::SurfaceTexture>,
        reconfigure: bool,
    },
    Offscreen {
        slots: [Slot; 2],
        next: usize,
        size: (u32, u32),
    },
}

#[cfg(target_os = "macos")]
struct Presenter {
    counters: PresentCounters,
    target: Target,
    format: wgpu::TextureFormat,
    pending_view: Option<wgpu::TextureView>,
    renderer: Renderer,
    drawing: Option<Box<crate::draw::DrawRenderer>>,
    instance: wgpu::Instance,
    adapter: String,
    failed: bool,
    verify: bool,
    verified_frames: u64,
}

#[cfg(target_os = "macos")]
impl Presenter {
    unsafe fn new(layer: *mut c_void, width: u32, height: u32, vsync: bool) -> Result<Self> {
        ensure!(!layer.is_null(), "null Metal layer");
        crate::frame::dimensions(width, height)?;
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(descriptor);
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
        }?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let info = adapter.get_info();
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| {
                matches!(
                    format,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .context("surface has no display-byte unorm format")?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&crate::draw::timing::device_descriptor(&adapter)),
        )?;
        let renderer = Renderer::with_format(device, queue, format)?;
        let present_mode = present_mode(&capabilities.present_modes, vsync)?;
        let verify = std::env::var("KFX_WGPU_VERIFY").is_ok_and(|value| value == "1");
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | if verify {
                wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::empty()
            };
        ensure!(
            capabilities.usages.contains(usage),
            "surface does not support requested validation readback"
        );
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(renderer.device(), &config);
        renderer.check_status()?;
        Ok(Self {
            counters: PresentCounters::default(),
            target: Target::Swapchain {
                surface,
                layer,
                config,
                modes: capabilities.present_modes,
                pending: None,
                reconfigure: false,
            },
            format,
            pending_view: None,
            renderer,
            drawing: None,
            instance,
            adapter: format!("{} ({:?})", info.name, info.backend),
            failed: false,
            verify,
            verified_frames: 0,
        })
    }

    /// Measurement mode: the same palette pass into a two-slot texture ring, with no
    /// surface, no drawable and therefore no dependence on an unlocked, unoccluded display.
    fn offscreen(width: u32, height: u32) -> Result<Self> {
        crate::frame::dimensions(width, height)?;
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(
            adapter.request_device(&crate::draw::timing::device_descriptor(&adapter)),
        )?;
        // What the Metal surface selects, so pipelines and the palette shader are identical.
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let renderer = Renderer::with_format(device, queue, format)?;
        renderer.check_status()?;
        Ok(Self {
            counters: PresentCounters::default(),
            target: Target::Offscreen {
                slots: Default::default(),
                next: 0,
                size: (width, height),
            },
            format,
            pending_view: None,
            renderer,
            drawing: None,
            instance,
            adapter: format!("{} ({:?})", info.name, info.backend),
            failed: false,
            verify: std::env::var("KFX_WGPU_VERIFY").is_ok_and(|value| value == "1"),
            verified_frames: 0,
        })
    }

    fn drawing(&mut self) -> Result<&mut crate::draw::DrawRenderer> {
        ensure!(!self.failed, "presenter is terminal");
        self.renderer.check_status()?;
        if self.drawing.is_none() {
            self.drawing = Some(Box::new(crate::draw::DrawRenderer::new(
                &self.renderer,
                self.format,
            )?));
        }
        Ok(self.drawing.as_mut().unwrap())
    }

    fn pending_texture(&self) -> Option<&wgpu::Texture> {
        match &self.target {
            Target::Swapchain { pending, .. } => pending.as_ref().map(|frame| &frame.texture),
            Target::Offscreen { slots, next, .. } => slots[*next].texture.as_ref(),
        }
    }

    fn acquire(&mut self, width: u32, height: u32, vsync: bool) -> Result<bool> {
        let start = std::time::Instant::now();
        let result = self.acquire_inner(width, height, vsync);
        self.counters.acquire_ns += start.elapsed().as_nanos() as u64;
        result
    }

    fn acquire_inner(&mut self, width: u32, height: u32, vsync: bool) -> Result<bool> {
        ensure!(!self.failed, "presenter is terminal");
        ensure!(
            self.pending_view.is_none(),
            "previous frame was not presented"
        );
        self.renderer.device().poll(wgpu::PollType::Poll)?;
        self.renderer.check_status()?;
        if width == 0 || height == 0 {
            return Ok(false);
        }
        crate::frame::dimensions(width, height)?;
        let Self {
            target,
            renderer,
            instance,
            pending_view,
            counters,
            format,
            ..
        } = self;
        let device = renderer.device();
        match target {
            Target::Offscreen { slots, next, size } => {
                if *size != (width, height) {
                    counters.reconfigure_count += 1;
                    *size = (width, height);
                    for slot in slots.iter_mut() {
                        slot.texture = None;
                    }
                }
                let block_start = std::time::Instant::now();
                let slot = &mut slots[*next];
                // The ring is the offscreen path's only back-pressure: `nextDrawable`
                // was what kept an uncapped host from running away from the GPU.
                if let Some(index) = slot.last.take() {
                    device.poll(wgpu::PollType::Wait {
                        submission_index: Some(index),
                        timeout: Some(std::time::Duration::from_secs(10)),
                    })?;
                }
                let texture = slot.texture.get_or_insert_with(|| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("offscreen presentation"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: *format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    })
                });
                counters.acquire_block_ns += block_start.elapsed().as_nanos() as u64;
                *pending_view = Some(texture.create_view(&Default::default()));
                renderer.check_status()?;
                Ok(true)
            }
            Target::Swapchain {
                surface,
                layer,
                config,
                modes,
                pending,
                reconfigure,
            } => {
                let mode = present_mode(modes, vsync)?;
                if *reconfigure
                    || (config.width, config.height, config.present_mode) != (width, height, mode)
                {
                    config.width = width;
                    config.height = height;
                    config.present_mode = mode;
                    counters.reconfigure_count += 1;
                    surface.configure(device, config);
                    *reconfigure = false;
                }
                for _ in 0..2 {
                    let block_start = std::time::Instant::now();
                    let acquired = surface.get_current_texture();
                    counters.acquire_block_ns += block_start.elapsed().as_nanos() as u64;
                    match acquired {
                        wgpu::CurrentSurfaceTexture::Success(frame) => {
                            *pending_view = Some(frame.texture.create_view(&Default::default()));
                            *pending = Some(frame);
                            return Ok(true);
                        }
                        wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                            *pending_view = Some(frame.texture.create_view(&Default::default()));
                            *pending = Some(frame);
                            *reconfigure = true;
                            return Ok(true);
                        }
                        wgpu::CurrentSurfaceTexture::Timeout
                        | wgpu::CurrentSurfaceTexture::Occluded => return Ok(false),
                        wgpu::CurrentSurfaceTexture::Outdated => {
                            counters.reconfigure_count += 1;
                            surface.configure(device, config);
                        }
                        wgpu::CurrentSurfaceTexture::Lost => {
                            renderer.check_status()?;
                            *surface = unsafe {
                                instance.create_surface_unsafe(
                                    wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(*layer),
                                )
                            }?;
                            counters.reconfigure_count += 1;
                            surface.configure(device, config);
                        }
                        wgpu::CurrentSurfaceTexture::Validation => {
                            bail!("surface acquisition validation failed")
                        }
                    }
                }
                bail!("surface unavailable after recovery")
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn present_mode(modes: &[wgpu::PresentMode], vsync: bool) -> Result<wgpu::PresentMode> {
    if vsync {
        return Ok(wgpu::PresentMode::Fifo);
    }
    [wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox]
        .into_iter()
        .find(|mode| modes.contains(mode))
        .context("surface cannot disable VSync")
}

unsafe fn write_text(output: *mut c_char, capacity: usize, text: &str) {
    if !output.is_null() && capacity > 0 {
        let length = text.len().min(capacity - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), output.cast(), length);
            *output.add(length) = 0;
        }
    }
}

unsafe fn boundary<T: Default>(
    error: *mut c_char,
    capacity: usize,
    action: impl FnOnce() -> Result<T>,
) -> T {
    unsafe { write_text(error, capacity, "") };
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(value)) => value,
        Ok(Err(failure)) => {
            unsafe { write_text(error, capacity, &format!("{failure:#}")) };
            T::default()
        }
        Err(_) => {
            unsafe { write_text(error, capacity, "Rust presenter panic") };
            T::default()
        }
    }
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_create(
    layer: *mut c_void,
    width: u32,
    height: u32,
    vsync: i32,
    error: *mut c_char,
    capacity: usize,
) -> *mut c_void {
    unsafe {
        boundary(error, capacity, || {
            ensure!((0..=1).contains(&vsync), "invalid VSync value");
            Ok(Box::into_raw(Box::new(Presenter::new(layer, width, height, vsync != 0)?)).cast())
        })
    }
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_create_offscreen(
    width: u32,
    height: u32,
    error: *mut c_char,
    capacity: usize,
) -> *mut c_void {
    unsafe {
        boundary(error, capacity, || {
            Ok(Box::into_raw(Box::new(Presenter::offscreen(width, height)?)).cast())
        })
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_submit(
    handle: *mut c_void,
    indices: *const u8,
    length: usize,
    width: u32,
    height: u32,
    pitch: u32,
    palette: *const u8,
    palette_length: usize,
    output_width: u32,
    output_height: u32,
    vsync: i32,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    let result: Option<i32> = unsafe {
        boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && !indices.is_null() && !palette.is_null(),
                "null presenter or input"
            );
            ensure!((0..=1).contains(&vsync), "invalid VSync value");
            let presenter = &mut *handle.cast::<Presenter>();
            validate_rows(
                width,
                height,
                length,
                pitch,
                palette_length,
                output_width.max(1),
                output_height.max(1),
                &presenter.renderer.device().limits(),
            )?;
            if !presenter.acquire(output_width, output_height, vsync != 0)? {
                return Ok(Some(0));
            }
            let view = presenter.pending_view.clone().unwrap();
            let record_start = std::time::Instant::now();
            let recorded = presenter.renderer.render_into(
                width,
                height,
                std::slice::from_raw_parts(indices, length),
                pitch,
                std::slice::from_raw_parts(palette, palette_length),
                output_width,
                output_height,
                &view,
            );
            presenter.counters.present_record_ns += record_start.elapsed().as_nanos() as u64;
            recorded?;
            if presenter.verify {
                verify_surface(
                    &presenter.renderer,
                    presenter
                        .pending_texture()
                        .context("no acquired frame to verify")?,
                    std::slice::from_raw_parts(indices, length),
                    width,
                    height,
                    pitch,
                    std::slice::from_raw_parts(palette, palette_length),
                )?;
                presenter.verified_frames += 1;
            }
            Ok(Some(1))
        })
    };
    if result.is_none() && !handle.is_null() {
        unsafe {
            (*handle.cast::<Presenter>()).failed = true;
        }
    }
    result.unwrap_or(-1)
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_present(
    handle: *mut c_void,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    let submit_start = std::time::Instant::now();
    let result: Option<i32> = unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            let presenter = &mut *handle.cast::<Presenter>();
            ensure!(!presenter.failed, "presenter is terminal");
            // The frame's one submit. It happens here and not in `prepare_present`
            // because the cursor restore is recorded between the two, and it happens on
            // the acquisition-skip path too, where the encoder holds the same work
            // minus the palette pass.
            if let Some(drawing) = presenter.drawing.as_mut() {
                drawing.frame_submit()?;
            }
            if presenter.pending_view.take().is_none() {
                presenter.renderer.check_status()?;
                return Ok(Some(0));
            }
            // The frame's own submission, taken from whichever renderer made it, so the
            // ring adds no submit of its own and `submits` stays one per frame.
            let submission = presenter
                .drawing
                .as_mut()
                .and_then(|drawing| drawing.take_submission())
                .or_else(|| presenter.renderer.take_submission());
            let Presenter {
                target, renderer, ..
            } = presenter;
            match target {
                Target::Swapchain { pending, .. } => {
                    let frame = pending.take().context("acquired frame was lost")?;
                    renderer.queue().present(frame);
                }
                Target::Offscreen { slots, next, .. } => {
                    // The ring waits on this before rendering into the slot again; an
                    // empty submit only names a place in the queue when nothing was
                    // recorded, which a frame that reached here normally did.
                    slots[*next].last = Some(submission.unwrap_or_else(|| renderer.submit_empty()));
                    *next = (*next + 1) % slots.len();
                }
            }
            presenter.renderer.check_status()?;
            Ok(Some(1))
        })
    };
    if result.is_none() && !handle.is_null() {
        unsafe {
            let presenter = &mut *handle.cast::<Presenter>();
            presenter.failed = true;
            // A terminal failure drops the recording rather than submitting half a
            // frame; the SDL fallback redraws from scratch.
            if let Some(drawing) = presenter.drawing.as_mut() {
                drawing.frame_discard();
            }
        }
    }
    if !handle.is_null() {
        unsafe {
            (*handle.cast::<Presenter>()).counters.submit_ns +=
                submit_start.elapsed().as_nanos() as u64;
        }
    }
    result.unwrap_or(-1)
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_details(
    handle: *mut c_void,
    text: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(text, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            let presenter = &*handle.cast::<Presenter>();
            // Every value here must be constant for the life of the presenter: the
            // performance capture aborts a run whose renderer identity changes.
            let mut details = serde_json::json!({
                "adapter": presenter.adapter, "backend": "Metal",
                "format": format!("{:?}", presenter.format),
                "verified_frames": presenter.verified_frames,
            });
            match &presenter.target {
                Target::Swapchain { config, .. } => {
                    details["present_mode"] = format!("{:?}", config.present_mode).into();
                    details["color_space"] = format!("{:?}", config.color_space).into();
                    details["latency"] = config.desired_maximum_frame_latency.into();
                }
                Target::Offscreen { slots, .. } => {
                    details["present_mode"] = "Offscreen".into();
                    details["latency"] = slots.len().into();
                }
            }
            let details = details.to_string();
            ensure!(details.len() < capacity, "details buffer too small");
            write_text(text, capacity, &details);
            Ok(1)
        })
    }
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_destroy(handle: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !handle.is_null() {
            unsafe {
                drop(Box::from_raw(handle.cast::<Presenter>()));
            }
        }
    }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_allocation_counts(
    allocations: *mut u64,
    requested_bytes: *mut u64,
) {
    if !allocations.is_null() {
        unsafe {
            *allocations = ALLOCATIONS.load(Ordering::Relaxed);
        }
    }
    if !requested_bytes.is_null() {
        unsafe {
            *requested_bytes = REQUESTED_BYTES.load(Ordering::Relaxed);
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(target_os = "macos")]
fn verify_surface(
    renderer: &Renderer,
    texture: &wgpu::Texture,
    indices: &[u8],
    width: u32,
    height: u32,
    pitch: u32,
    palette: &[u8],
) -> Result<()> {
    let stride = (texture.width() * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("opt-in surface verification"),
        size: u64::from(stride) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer
        .device()
        .create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    let submission = renderer.queue().submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    let result = (|| -> Result<()> {
        renderer.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })?;
        receiver.recv_timeout(std::time::Duration::from_secs(1))??;
        renderer.check_status()?;
        let pixels = readback.slice(..).get_mapped_range()?;
        for y in 0..texture.height() {
            for x in 0..texture.width() {
                let source_x = ((u64::from(x) * 2 + 1) * u64::from(width)
                    / (u64::from(texture.width()) * 2)) as usize;
                let source_y = ((u64::from(y) * 2 + 1) * u64::from(height)
                    / (u64::from(texture.height()) * 2)) as usize;
                let index = usize::from(indices[source_y * pitch as usize + source_x]);
                let expected = &palette[index * 4..index * 4 + 4];
                let offset = (y * stride + x * 4) as usize;
                let actual = &pixels[offset..offset + 4];
                let rgba = if texture.format() == wgpu::TextureFormat::Bgra8Unorm {
                    [actual[2], actual[1], actual[0], actual[3]]
                } else {
                    [actual[0], actual[1], actual[2], actual[3]]
                };
                ensure!(
                    rgba == expected,
                    "surface pixel mismatch at ({x}, {y}): {rgba:?} != {expected:?}"
                );
            }
        }
        Ok(())
    })();
    readback.unmap();
    result
}
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a native Metal adapter"]
    fn surface_format_preserves_padded_palette_frames() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut renderer =
            Renderer::with_format(device, queue, wgpu::TextureFormat::Bgra8Unorm).unwrap();
        let mut frame = crate::frame::Frame::fixture();
        let pitch = frame.width + 7;
        let mut indices = vec![255; (pitch * frame.height) as usize];
        for (source, target) in frame
            .indices
            .chunks(frame.width as usize)
            .zip(indices.chunks_mut(pitch as usize))
        {
            target[..source.len()].copy_from_slice(source);
        }
        for scale in [1, 2, 8, 1] {
            let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("surface-format test"),
                size: wgpu::Extent3d {
                    width: frame.width * scale,
                    height: frame.height * scale,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            renderer
                .render_into(
                    frame.width,
                    frame.height,
                    &indices,
                    pitch,
                    &frame.palette,
                    texture.width(),
                    texture.height(),
                    &view,
                )
                .unwrap();
            verify_surface(
                &renderer,
                &texture,
                &indices,
                frame.width,
                frame.height,
                pitch,
                &frame.palette,
            )
            .unwrap();
            frame.palette[0] ^= 137;
            frame.palette[3] ^= 255;
            assert!(
                verify_surface(
                    &renderer,
                    &texture,
                    &indices,
                    frame.width,
                    frame.height,
                    pitch,
                    &frame.palette
                )
                .is_err()
            );
            renderer
                .render_into(
                    frame.width,
                    frame.height,
                    &indices,
                    pitch,
                    &frame.palette,
                    texture.width(),
                    texture.height(),
                    &view,
                )
                .unwrap();
            verify_surface(
                &renderer,
                &texture,
                &indices,
                frame.width,
                frame.height,
                pitch,
                &frame.palette,
            )
            .unwrap();
        }
    }

    #[test]
    #[ignore = "requires a native Metal adapter"]
    fn nearest_mapping_is_exact_at_maximum_odd_widths() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut renderer =
            Renderer::with_format(device, queue, wgpu::TextureFormat::Bgra8Unorm).unwrap();
        let palette = crate::frame::Frame::fixture().palette;
        for (width, output_width) in [(8191, 8192), (8192, 8191), (4095, 4096)] {
            let indices: Vec<u8> = (0..width).map(|index| (index % 256) as u8).collect();
            let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("odd maximum-width test"),
                size: wgpu::Extent3d {
                    width: output_width,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            renderer
                .render_into(
                    width,
                    1,
                    &indices,
                    width,
                    &palette,
                    output_width,
                    1,
                    &texture.create_view(&Default::default()),
                )
                .unwrap();
            verify_surface(&renderer, &texture, &indices, width, 1, width, &palette).unwrap();
        }
    }

    #[test]
    #[ignore = "requires a native Metal adapter"]
    fn offscreen_ring_bounds_frames_in_flight() {
        let frame = crate::frame::Frame::fixture();
        let (width, height) = (frame.width, frame.height);
        let mut presenter = Presenter::offscreen(width, height).unwrap();
        let mut error = [0i8; 512];
        for index in 0..3usize {
            let Target::Offscreen { slots, next, .. } = &presenter.target else {
                unreachable!()
            };
            assert_eq!(slots[*next].last.is_some(), index >= 2);
            assert!(presenter.acquire(width, height, false).unwrap());
            let Target::Offscreen { slots, next, .. } = &presenter.target else {
                unreachable!()
            };
            assert!(
                slots[*next].last.is_none(),
                "slot {next} was reused without waiting for its submission"
            );
            let view = presenter.pending_view.clone().unwrap();
            presenter
                .renderer
                .render_into(
                    width,
                    height,
                    &frame.indices,
                    width,
                    &frame.palette,
                    width,
                    height,
                    &view,
                )
                .unwrap();
            assert_eq!(
                unsafe {
                    kfx_wgpu_present((&raw mut presenter).cast(), error.as_mut_ptr(), error.len())
                },
                1,
                "{}",
                unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) }.to_string_lossy()
            );
        }
        // One queue submission per frame and no more: the ring reuses the frame's own
        // submission, so the empty fallback must never have run.
        assert_eq!(presenter.renderer.queue_submits(), 3);
        let Target::Offscreen { slots, next, .. } = &presenter.target else {
            unreachable!()
        };
        assert_eq!(*next, 1);
        assert!(slots.iter().all(|slot| slot.last.is_some()));
    }

    #[test]
    #[ignore = "requires a native Metal adapter"]
    fn presenter_host_timers_cover_software_and_gpu_paths() {
        let mut presenter = Presenter::offscreen(2, 2).unwrap();
        let palette = [255u8; 1024];
        let mut error = [0i8; 1024];
        for gpu in [false, true] {
            let handle = (&raw mut presenter).cast();
            let result = if gpu {
                let drawing = presenter.drawing().unwrap();
                let root = drawing.create_target(2, 2).unwrap();
                drawing.frame_begin(root).unwrap();
                unsafe {
                    kfx_wgpu_draw_prepare_present(
                        handle,
                        root,
                        palette.as_ptr(),
                        1024,
                        3,
                        3,
                        0,
                        error.as_mut_ptr(),
                        error.len(),
                    )
                }
            } else {
                unsafe {
                    kfx_wgpu_submit(
                        handle,
                        [0u8; 4].as_ptr(),
                        4,
                        2,
                        2,
                        2,
                        palette.as_ptr(),
                        1024,
                        2,
                        2,
                        0,
                        error.as_mut_ptr(),
                        error.len(),
                    )
                }
            };
            assert_eq!(result, 1);
            assert_eq!(
                unsafe { kfx_wgpu_present(handle, error.as_mut_ptr(), error.len()) },
                1
            );
            let mut counters = PresentCounters::default();
            unsafe {
                kfx_wgpu_present_counters(handle, &mut counters);
            }
            assert!(counters.acquire_ns >= counters.acquire_block_ns);
            assert!(counters.acquire_block_ns > 0);
            assert!(counters.present_record_ns > 0);
            assert!(counters.submit_ns > 0);
            assert_eq!(counters.reconfigure_count, u64::from(gpu));
            unsafe {
                kfx_wgpu_present_counters(handle, &mut counters);
            }
            assert_eq!(counters.acquire_ns, 0);
            assert_eq!(counters.submit_ns, 0);
            assert_eq!(counters.present_record_ns, 0);
        }
    }

    #[test]
    fn rejects_null_handles_and_invalid_modes() {
        let mut error = [0i8; 100];
        unsafe {
            assert!(kfx_wgpu_create_offscreen(0, 0, error.as_mut_ptr(), error.len()).is_null());
            assert_ne!(error[0], 0);
            assert!(
                kfx_wgpu_create(
                    std::ptr::null_mut(),
                    640,
                    480,
                    0,
                    error.as_mut_ptr(),
                    error.len()
                )
                .is_null()
            );
            assert_ne!(error[0], 0);
            assert_eq!(
                kfx_wgpu_present(std::ptr::null_mut(), error.as_mut_ptr(), error.len()),
                -1
            );
            assert_eq!(
                kfx_wgpu_submit(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    0,
                    640,
                    480,
                    640,
                    std::ptr::null(),
                    0,
                    640,
                    480,
                    0,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            kfx_wgpu_destroy(std::ptr::null_mut());
        }
        assert!(present_mode(&[wgpu::PresentMode::Fifo], false).is_err());
        assert_eq!(
            present_mode(
                &[wgpu::PresentMode::Immediate, wgpu::PresentMode::Fifo],
                false
            )
            .unwrap(),
            wgpu::PresentMode::Immediate
        );
    }
    #[test]
    fn contains_panics_and_terminates_error_strings() {
        let mut error = [0i8; 4];
        let value: i32 = unsafe { boundary(error.as_mut_ptr(), error.len(), || panic!("test")) };
        assert_eq!(value, 0);
        assert_eq!(error[3], 0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_create(error: *mut c_char, capacity: usize) -> *mut c_void {
    unsafe {
        boundary(error, capacity, || {
            Ok(Box::into_raw(Box::new(crate::draw::DrawRenderer::headless()?)).cast())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(handle.cast::<crate::draw::DrawRenderer>()));
        }));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_context(
    handle: *mut c_void,
    error: *mut c_char,
    capacity: usize,
) -> *mut c_void {
    unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            #[cfg(target_os = "macos")]
            {
                let drawing = (&mut *handle.cast::<Presenter>()).drawing()?;
                Ok((drawing as *mut crate::draw::DrawRenderer).cast())
            }
            #[cfg(not(target_os = "macos"))]
            anyhow::bail!("live presenter requires macOS")
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_target_create(
    handle: *mut c_void,
    width: u32,
    height: u32,
    error: *mut c_char,
    capacity: usize,
) -> u64 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            let drawing = &mut *handle.cast::<crate::draw::DrawRenderer>();
            let id = drawing.create_target(width, height)?;
            drawing.check_status()?;
            Ok(id)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_target_release(
    handle: *mut c_void,
    target: u64,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            (&mut *handle.cast::<crate::draw::DrawRenderer>()).release_target(target)?;
            Ok(1)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_resource_create(
    handle: *mut c_void,
    bytes: *const u8,
    length: usize,
    width: u32,
    height: u32,
    pitch: u32,
    error: *mut c_char,
    capacity: usize,
) -> u64 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && !bytes.is_null(),
                "null presenter or asset"
            );
            crate::draw::validate_resource(length, width, height, pitch)?;
            (&mut *handle.cast::<crate::draw::DrawRenderer>()).create_resource(
                std::slice::from_raw_parts(bytes, length),
                width,
                height,
                pitch,
            )
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_resource_mark_cursor(handle: *mut c_void, resource: u64) {
    if !handle.is_null() {
        unsafe {
            (&mut *handle.cast::<crate::draw::DrawRenderer>()).mark_cursor_resource(resource);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_resource_release(
    handle: *mut c_void,
    resource: u64,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            (&mut *handle.cast::<crate::draw::DrawRenderer>()).release_resource(resource)?;
            Ok(1)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_submit(
    handle: *mut c_void,
    target: u64,
    commands: *const crate::draw::Command,
    count: usize,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && (!commands.is_null() || count == 0),
                "null presenter or commands"
            );
            ensure!(count <= 262_144, "command count exceeds limit");
            let commands = if count == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(commands, count)
            };
            let drawing = &mut *handle.cast::<crate::draw::DrawRenderer>();
            drawing.submit(target, commands)?;
            drawing.check_status()?;
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_submit_triangles(
    handle: *mut c_void,
    target: u64,
    commands: *const crate::draw::TriangleCommand,
    count: usize,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && (!commands.is_null() || count == 0),
                "null presenter or commands"
            );
            ensure!(count <= 262_144, "command count exceeds limit");
            let commands = if count == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(commands, count)
            };
            let drawing = &mut *handle.cast::<crate::draw::DrawRenderer>();
            drawing.submit_triangles(target, commands)?;
            drawing.check_status()?;
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_readback(
    handle: *mut c_void,
    target: u64,
    indices: *mut u8,
    length: usize,
    pitch: u32,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && !indices.is_null(),
                "null presenter or output"
            );
            let drawing = &mut *handle.cast::<crate::draw::DrawRenderer>();
            let (width, height) = drawing.target_dimensions(target)?;
            crate::draw::validate_resource(length, width, height, pitch)?;
            let bytes = drawing.readback(target)?;
            drawing.check_status()?;
            for (row, source) in bytes.chunks_exact(width as usize).enumerate() {
                std::ptr::copy_nonoverlapping(
                    source.as_ptr(),
                    indices.add(row * pitch as usize),
                    width as usize,
                );
            }
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn kfx_wgpu_draw_prepare_present(
    handle: *mut c_void,
    target: u64,
    palette: *const u8,
    palette_length: usize,
    output_width: u32,
    output_height: u32,
    vsync: i32,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && !palette.is_null(),
                "null presenter or palette"
            );
            ensure!(
                palette_length == 1024 && (0..=1).contains(&vsync),
                "invalid palette or VSync"
            );
            let presenter = &mut *handle.cast::<Presenter>();
            presenter.drawing()?.target_dimensions(target)?;
            // Flush what the frame has queued before the swapchain image is held, so
            // the surface is acquired for the shortest possible window.
            presenter.drawing()?.frame_flush()?;
            if !presenter.acquire(output_width, output_height, vsync != 0)? {
                // The encoder stays open and unsubmitted: `kfx_wgpu_present` still runs
                // and submits it, with the cursor restore recorded in between and the
                // palette pass omitted.
                return Ok(Some(0));
            }
            let view = presenter.pending_view.clone().unwrap();
            let record_start = std::time::Instant::now();
            let recorded = presenter.drawing()?.present_into(
                target,
                std::slice::from_raw_parts(palette, palette_length),
                output_width,
                output_height,
                &view,
            );
            presenter.counters.present_record_ns += record_start.elapsed().as_nanos() as u64;
            recorded?;
            presenter.renderer.check_status()?;
            if presenter.verify {
                let (width, height) = presenter.drawing()?.target_dimensions(target)?;
                let indices = presenter.drawing()?.readback(target)?;
                let texture = presenter
                    .pending_texture()
                    .context("no acquired frame to verify")?;
                verify_surface(
                    &presenter.renderer,
                    texture,
                    &indices,
                    width,
                    height,
                    width,
                    std::slice::from_raw_parts(palette, palette_length),
                )?;
                presenter.verified_frames += 1;
            }
            Ok(Some(1))
        });
        if result.is_none() && !handle.is_null() {
            let presenter = &mut *handle.cast::<Presenter>();
            presenter.failed = true;
            if let Some(drawing) = presenter.drawing.as_mut() {
                drawing.frame_discard();
            }
        }
        result.unwrap_or(-1)
    }
}

#[repr(C)]
pub struct DrawCounters {
    batches: u64,
    commands: u64,
    asset_upload_bytes: u64,
    command_upload_bytes: u64,
    readback_bytes: u64,
    submits: u64,
    dispatches: u64,
    waits: u64,
    wait_ns: u64,
    buffers: u64,
    buffer_bytes: u64,
    arena_evictions: u64,
    arena_overflows: u64,
    arena_bytes_uploaded: u64,
    host_staged_asset_bytes: u64,
    arena_bytes_resident: u64,
    tile_allocations: u64,
    tile_entries: u64,
    ordered_sprite_layers: u64,
    ordered_sprite_passes: u64,
    terrain_tile_entries: u64,
    tile_entries_by_kind: [u64; crate::draw::BIN_KINDS],
    prepared_row_words: u64,
    prepared_row_allocations: u64,
    pass_ns: [u64; crate::draw::timing::PASS_KINDS],
    timed_passes: u64,
    untimed_passes: u64,
    gpu_pass_union_ns: u64,
    arena_scratch_bytes_peak: u64,
    target_trig_geometry_bytes: u64,
    target_trig_table_bytes: u64,
    other_asset_upload_bytes: u64,
    target_trig_table_hits: u64,
    target_trig_table_misses: u64,
    target_trig_asset_buffers: u64,
    shadow_pairs: u64,
    preparer_buffers: u64,
    preparer_buffer_bytes: u64,
    arena_misses_new_id: u64,
    arena_misses_forget: u64,
    arena_misses_size_class: u64,
    arena_misses_generation: u64,
    arena_misses_eviction: u64,
    arena_miss_new_id_bytes: u64,
    arena_miss_forget_bytes: u64,
    arena_miss_size_class_bytes: u64,
    arena_miss_generation_bytes: u64,
    arena_miss_eviction_bytes: u64,
    arena_explicit_forgets: u64,
    arena_capacity_bytes: u64,
    arena_live_bytes: u64,
    arena_retired_bytes: u64,
    arena_growth_peak_bytes: u64,
    arena_by_kind:
        [crate::draw::arena_kinds::ArenaKindCounters; crate::draw::arena_kinds::ARENA_KINDS],
    arena_trig_texture_source_bytes: u64,
    arena_minimap_prefix_bytes: u64,
    arena_minimap_dictionary_bytes: u64,
    arena_minimap_cells_bytes: u64,
    arena_minimap_styles_bytes: u64,
    minimap_dictionary_hits: u64,
    minimap_dictionary_misses: u64,
    minimap_cells_hits: u64,
    minimap_cells_misses: u64,
    minimap_styles_hits: u64,
    minimap_styles_misses: u64,
    minimap_cache_class_bytes: u64,
    minimap_cache_cpu_bytes: u64,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_counters(
    handle: *mut c_void,
    output: *mut DrawCounters,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && !output.is_null(),
                "null drawing context or counters"
            );
            let drawing = &*handle.cast::<crate::draw::DrawRenderer>();
            let counters = drawing.counters();
            let arena = drawing.arena_counters();
            output.write(DrawCounters {
                batches: counters.batches,
                commands: counters.commands,
                asset_upload_bytes: counters.asset_upload_bytes,
                command_upload_bytes: counters.command_upload_bytes,
                readback_bytes: counters.readback_bytes,
                submits: counters.submits,
                dispatches: counters.dispatches,
                waits: counters.waits,
                wait_ns: counters.wait_ns,
                buffers: counters.buffers,
                buffer_bytes: counters.buffer_bytes,
                arena_evictions: arena.evictions,
                arena_overflows: arena.overflows,
                arena_bytes_uploaded: arena.bytes_uploaded,
                host_staged_asset_bytes: drawing.staged_asset_bytes(),
                arena_bytes_resident: arena.bytes_resident,
                tile_allocations: counters.tile_allocations,
                tile_entries: counters.tile_entries,
                ordered_sprite_layers: counters.ordered_sprite_layers,
                ordered_sprite_passes: counters.ordered_sprite_passes,
                terrain_tile_entries: counters.terrain_tile_entries,
                tile_entries_by_kind: counters.tile_entries_by_kind,
                prepared_row_words: counters.prepared_row_words,
                prepared_row_allocations: counters.prepared_row_allocations,
                pass_ns: counters.pass_ns,
                timed_passes: counters.timed_passes,
                untimed_passes: counters.untimed_passes,
                gpu_pass_union_ns: counters.gpu_pass_union_ns,
                arena_scratch_bytes_peak: arena.scratch_bytes_peak,
                target_trig_geometry_bytes: counters.target_trig_geometry_bytes,
                target_trig_table_bytes: counters.target_trig_table_bytes,
                other_asset_upload_bytes: counters.asset_upload_bytes
                    - counters.target_trig_geometry_bytes
                    - counters.target_trig_table_bytes,
                target_trig_table_hits: counters.target_trig_table_hits,
                target_trig_table_misses: counters.target_trig_table_misses,
                target_trig_asset_buffers: counters.target_trig_asset_buffers,
                shadow_pairs: counters.shadow_pairs,
                preparer_buffers: counters.preparer_buffers,
                preparer_buffer_bytes: counters.preparer_buffer_bytes,
                arena_misses_new_id: arena.misses_new_id,
                arena_misses_forget: arena.misses_forget,
                arena_misses_size_class: arena.misses_size_class,
                arena_misses_generation: arena.misses_generation,
                arena_misses_eviction: arena.misses_eviction,
                arena_miss_new_id_bytes: arena.miss_new_id_bytes,
                arena_miss_forget_bytes: arena.miss_forget_bytes,
                arena_miss_size_class_bytes: arena.miss_size_class_bytes,
                arena_miss_generation_bytes: arena.miss_generation_bytes,
                arena_miss_eviction_bytes: arena.miss_eviction_bytes,
                arena_explicit_forgets: arena.explicit_forgets,
                arena_capacity_bytes: arena.capacity_bytes,
                arena_live_bytes: arena.live_bytes,
                arena_retired_bytes: arena.retired_bytes,
                arena_growth_peak_bytes: arena.growth_peak_bytes,
                arena_by_kind: counters.arena_by_kind,
                arena_trig_texture_source_bytes: counters.arena_trig_texture_source_bytes,
                arena_minimap_prefix_bytes: counters.arena_minimap_prefix_bytes,
                arena_minimap_dictionary_bytes: counters.arena_minimap_dictionary_bytes,
                arena_minimap_cells_bytes: counters.arena_minimap_cells_bytes,
                arena_minimap_styles_bytes: counters.arena_minimap_styles_bytes,
                minimap_dictionary_hits: counters.minimap_dictionary_hits,
                minimap_dictionary_misses: counters.minimap_dictionary_misses,
                minimap_cells_hits: counters.minimap_cells_hits,
                minimap_cells_misses: counters.minimap_cells_misses,
                minimap_styles_hits: counters.minimap_styles_hits,
                minimap_styles_misses: counters.minimap_styles_misses,
                minimap_cache_class_bytes: counters.minimap_cache_class_bytes,
                minimap_cache_cpu_bytes: counters.minimap_cache_cpu_bytes,
            });
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[cfg(test)]
mod draw_abi_tests {
    use super::*;

    #[test]
    fn arena_counter_layout() {
        assert_eq!(
            std::mem::size_of::<crate::draw::arena_kinds::ArenaKindCounters>(),
            48
        );
        assert_eq!(std::mem::offset_of!(DrawCounters, arena_by_kind), 77 * 8);
        assert_eq!(
            std::mem::size_of::<DrawCounters>(),
            (77 + 19 * 6 + 1 + 12) * 8
        );
    }

    #[test]
    fn rejects_null_draw_inputs_without_gpu() {
        unsafe {
            let mut error = [0; 128];
            assert_eq!(
                kfx_wgpu_draw_target_create(
                    std::ptr::null_mut(),
                    1,
                    1,
                    error.as_mut_ptr(),
                    error.len()
                ),
                0
            );
            assert_eq!(
                kfx_wgpu_draw_submit(
                    std::ptr::null_mut(),
                    1,
                    std::ptr::null(),
                    0,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                kfx_wgpu_draw_readback(
                    std::ptr::null_mut(),
                    1,
                    std::ptr::null_mut(),
                    0,
                    0,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                kfx_wgpu_draw_counters(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                crate::live::frame_queue::kfx_wgpu_draw_frame_status(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert!(
                kfx_wgpu_draw_context(std::ptr::null_mut(), error.as_mut_ptr(), error.len())
                    .is_null()
            );
            kfx_wgpu_draw_destroy(std::ptr::null_mut());
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn headless_draw_abi_owns_and_copies_inputs() {
        unsafe {
            let mut error = [0; 512];
            let context = kfx_wgpu_draw_create(error.as_mut_ptr(), error.len());
            assert!(
                !context.is_null(),
                "{}",
                std::ffi::CStr::from_ptr(error.as_ptr()).to_string_lossy()
            );
            let target =
                kfx_wgpu_draw_target_create(context, 7, 3, error.as_mut_ptr(), error.len());
            assert_ne!(target, 0);
            let mut command = crate::draw::Command {
                kind: crate::draw::CLEAR,
                colour: 37,
                ..Default::default()
            };
            assert_eq!(
                kfx_wgpu_draw_submit(
                    context,
                    target,
                    &command,
                    1,
                    error.as_mut_ptr(),
                    error.len()
                ),
                1
            );
            command.colour = 211;
            let mut bytes = [99u8; 27];
            assert_eq!(
                kfx_wgpu_draw_readback(
                    context,
                    target,
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    9,
                    error.as_mut_ptr(),
                    error.len()
                ),
                1
            );
            for row in bytes.as_chunks::<9>().0 {
                assert_eq!(&row[..7], &[37; 7]);
                assert_eq!(&row[7..], &[99; 2]);
            }
            assert_eq!(
                kfx_wgpu_draw_submit(
                    context,
                    target,
                    &command,
                    usize::MAX,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                kfx_wgpu_draw_target_release(context, target, error.as_mut_ptr(), error.len()),
                1
            );
            kfx_wgpu_draw_destroy(context);
        }
    }
}
