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
struct Presenter {
    pending: Option<wgpu::SurfaceTexture>,
    surface: wgpu::Surface<'static>,
    renderer: Renderer,
    drawing: Option<Box<crate::draw::DrawRenderer>>,
    instance: wgpu::Instance,
    layer: *mut c_void,
    config: wgpu::SurfaceConfiguration,
    modes: Vec<wgpu::PresentMode>,
    adapter: String,
    reconfigure: bool,
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
            pending: None,
            surface,
            renderer,
            drawing: None,
            instance,
            layer,
            config,
            modes: capabilities.present_modes,
            adapter: format!("{} ({:?})", info.name, info.backend),
            reconfigure: false,
            failed: false,
            verify,
            verified_frames: 0,
        })
    }

    fn drawing(&mut self) -> Result<&mut crate::draw::DrawRenderer> {
        ensure!(!self.failed, "presenter is terminal");
        self.renderer.check_status()?;
        if self.drawing.is_none() {
            self.drawing = Some(Box::new(crate::draw::DrawRenderer::new(
                &self.renderer,
                self.config.format,
            )?));
        }
        Ok(self.drawing.as_mut().unwrap())
    }

    fn acquire(&mut self, width: u32, height: u32, vsync: bool) -> Result<bool> {
        ensure!(!self.failed, "presenter is terminal");
        ensure!(self.pending.is_none(), "previous frame was not presented");
        self.renderer.device().poll(wgpu::PollType::Poll)?;
        self.renderer.check_status()?;
        if width == 0 || height == 0 {
            return Ok(false);
        }
        crate::frame::dimensions(width, height)?;
        let mode = present_mode(&self.modes, vsync)?;
        if self.reconfigure
            || (
                self.config.width,
                self.config.height,
                self.config.present_mode,
            ) != (width, height, mode)
        {
            self.config.width = width;
            self.config.height = height;
            self.config.present_mode = mode;
            self.surface.configure(self.renderer.device(), &self.config);
            self.reconfigure = false;
        }
        for _ in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => {
                    self.pending = Some(frame);
                    return Ok(true);
                }
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    self.pending = Some(frame);
                    self.reconfigure = true;
                    return Ok(true);
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(false);
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    self.surface.configure(self.renderer.device(), &self.config)
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    self.renderer.check_status()?;
                    self.surface = unsafe {
                        self.instance.create_surface_unsafe(
                            wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(self.layer),
                        )
                    }?;
                    self.surface.configure(self.renderer.device(), &self.config);
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    bail!("surface acquisition validation failed")
                }
            }
        }
        bail!("surface unavailable after recovery")
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
            let view = presenter
                .pending
                .as_ref()
                .unwrap()
                .texture
                .create_view(&Default::default());
            presenter.renderer.render_into(
                width,
                height,
                std::slice::from_raw_parts(indices, length),
                pitch,
                std::slice::from_raw_parts(palette, palette_length),
                output_width,
                output_height,
                &view,
            )?;
            if presenter.verify {
                verify_surface(
                    &presenter.renderer,
                    &presenter.pending.as_ref().unwrap().texture,
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
    let result: Option<i32> = unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            let presenter = &mut *handle.cast::<Presenter>();
            ensure!(!presenter.failed, "presenter is terminal");
            let frame = presenter.pending.take().context("no submitted frame")?;
            presenter.renderer.queue().present(frame);
            presenter.renderer.check_status()?;
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
pub unsafe extern "C" fn kfx_wgpu_details(
    handle: *mut c_void,
    text: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(text, capacity, || {
            ensure!(!handle.is_null(), "null presenter");
            let presenter = &*handle.cast::<Presenter>();
            let details = serde_json::json!({
                "adapter": presenter.adapter, "backend": "Metal",
                "format": format!("{:?}", presenter.config.format),
                "present_mode": format!("{:?}", presenter.config.present_mode),
                "color_space": format!("{:?}", presenter.config.color_space),
                "latency": presenter.config.desired_maximum_frame_latency,
            "verified_frames": presenter.verified_frames,
            })
            .to_string();
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
    fn rejects_null_handles_and_invalid_modes() {
        let mut error = [0i8; 100];
        unsafe {
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
            if !presenter.acquire(output_width, output_height, vsync != 0)? {
                return Ok(Some(0));
            }
            let view = presenter
                .pending
                .as_ref()
                .unwrap()
                .texture
                .create_view(&Default::default());
            presenter.drawing()?.present_into(
                target,
                std::slice::from_raw_parts(palette, palette_length),
                output_width,
                output_height,
                &view,
            )?;
            presenter.renderer.check_status()?;
            if presenter.verify {
                let (width, height) = presenter.drawing()?.target_dimensions(target)?;
                let indices = presenter.drawing()?.readback(target)?;
                verify_surface(
                    &presenter.renderer,
                    &presenter.pending.as_ref().unwrap().texture,
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
            (*handle.cast::<Presenter>()).failed = true;
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
    prepared_row_words: u64,
    prepared_row_allocations: u64,
    pass_ns: [u64; crate::draw::timing::PASS_KINDS],
    timed_passes: u64,
    untimed_passes: u64,
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
                prepared_row_words: counters.prepared_row_words,
                prepared_row_allocations: counters.prepared_row_allocations,
                pass_ns: counters.pass_ns,
                timed_passes: counters.timed_passes,
                untimed_passes: counters.untimed_passes,
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
