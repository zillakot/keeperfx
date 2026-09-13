use crate::gpu::{Renderer, validate_rows};
use anyhow::{Context, Result, bail, ensure};
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

struct Presenter {
    pending: Option<wgpu::SurfaceTexture>,
    surface: wgpu::Surface<'static>,
    renderer: Renderer,
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
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
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
#[cfg(test)]
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
