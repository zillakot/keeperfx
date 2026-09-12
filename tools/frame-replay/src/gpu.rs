use crate::frame::{Frame, dimensions};
use anyhow::{Context, Result, ensure};
use std::sync::{Arc, Mutex};

struct Inputs {
    indices: wgpu::Texture,
    binding: wgpu::BindGroup,
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Retained indexed-frame renderer. The caller supplies a matched device and queue.
/// GPU errors are terminal: discard the renderer and create a new device to recover.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    palette: wgpu::Texture,
    parameters: wgpu::Buffer,
    inputs: Option<Inputs>,
    target: Option<Target>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Renderer {
    /// Takes ownership of error and device-loss callbacks on this device.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        let failure = Arc::new(Mutex::new(None));
        let errors = failure.clone();
        device.on_uncaptured_error(Arc::new(move |error| {
            errors
                .lock()
                .unwrap()
                .get_or_insert_with(|| format!("GPU error: {error}"));
        }));
        let errors = failure.clone();
        device.set_device_lost_callback(move |reason, message| {
            errors
                .lock()
                .unwrap()
                .get_or_insert_with(|| format!("GPU device lost ({reason:?}): {message}"));
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("palette replay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("palette.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("palette replay"),
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
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let palette = texture(
            &device,
            "palette",
            256,
            1,
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let parameters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("parameters"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let renderer = Self {
            device,
            queue,
            pipeline,
            palette,
            parameters,
            inputs: None,
            target: None,
            failure,
        };
        renderer.check_status()?;
        Ok(renderer)
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Reports errors delivered so far; callers must also handle polling/submission failures.
    pub fn check_status(&self) -> Result<()> {
        if let Some(error) = &*self.failure.lock().unwrap() {
            anyhow::bail!("{error}");
        }
        Ok(())
    }

    /// Copies input and submits rendering without waiting or reading back pixels.
    /// The output is Rgba8Unorm and is overwritten on reuse; submit consumers before
    /// the next render. Invalid input leaves retained resources usable.
    pub fn render(&mut self, frame: &Frame, scale: u32) -> Result<&wgpu::Texture> {
        let (width, height) = validate(frame, scale, &self.device.limits())?;
        self.check_status()?;
        if self.inputs.as_ref().is_none_or(|inputs| {
            (inputs.indices.width(), inputs.indices.height()) != (frame.width, frame.height)
        }) {
            let indices = texture(
                &self.device,
                "indices",
                frame.width,
                frame.height,
                wgpu::TextureFormat::R8Uint,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            self.check_status()?;
            let indices_view = indices.create_view(&Default::default());
            let palette_view = self.palette.create_view(&Default::default());
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frame"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&indices_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&palette_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.parameters.as_entire_binding(),
                    },
                ],
            });
            self.check_status()?;
            self.inputs = Some(Inputs { indices, binding });
        }
        if self.target.as_ref().is_none_or(|target| {
            (target.texture.width(), target.texture.height()) != (width, height)
        }) {
            // Palette bytes are display-encoded; an sRGB attachment would encode them twice.
            let texture = texture(
                &self.device,
                "output",
                width,
                height,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            self.check_status()?;
            let view = texture.create_view(&Default::default());
            self.check_status()?;
            self.target = Some(Target { texture, view });
        }
        let inputs = self.inputs.as_ref().unwrap();
        let target = self.target.as_ref().unwrap();
        for (tex, bytes, pitch) in [
            (&inputs.indices, &frame.indices, frame.width),
            (&self.palette, &frame.palette, 1024),
        ] {
            self.queue.write_texture(
                tex.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: None,
                },
                tex.size(),
            );
        }
        let mut parameters = [0; 16];
        for (bytes, value) in
            parameters
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip([frame.width, frame.height, scale, 0])
        {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        self.queue.write_buffer(&self.parameters, 0, &parameters);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame replay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &inputs.binding, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.check_status()?;
        Ok(&self.target.as_ref().unwrap().texture)
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn validate(frame: &Frame, scale: u32, limits: &wgpu::Limits) -> Result<(u32, u32)> {
    let pixels = dimensions(frame.width, frame.height)?;
    ensure!(
        frame.indices.len() == pixels,
        "index buffer length does not match dimensions"
    );
    ensure!(
        frame.palette.len() == 1024,
        "palette must contain 256 RGBA entries"
    );
    ensure!((1..=8).contains(&scale), "scale must be between 1 and 8");
    let width = frame.width.checked_mul(scale).context("width overflow")?;
    let height = frame.height.checked_mul(scale).context("height overflow")?;
    dimensions(width, height)?;
    ensure!(
        width.max(height).max(256) <= limits.max_texture_dimension_2d,
        "frame exceeds GPU texture limits"
    );
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_inputs_and_device_limits() {
        let baseline = Frame::fixture();
        let limits = wgpu::Limits::default();
        for scale in [0, 9, u32::MAX] {
            assert!(validate(&baseline, scale, &limits).is_err());
        }
        for (width, height) in [(0, 19), (8193, 1), (8192, 8192), (u32::MAX, 2)] {
            let mut frame = baseline.clone();
            frame.width = width;
            frame.height = height;
            assert!(validate(&frame, 1, &limits).is_err());
        }
        for length in [0, 1023, 1025] {
            let mut frame = baseline.clone();
            frame.palette.resize(length, 0);
            assert!(validate(&frame, 1, &limits).is_err());
        }
        let mut frame = baseline.clone();
        frame.indices.pop();
        assert!(validate(&frame, 1, &limits).is_err());
        frame.indices.extend([0, 0]);
        assert!(validate(&frame, 1, &limits).is_err());
        let limits = wgpu::Limits {
            max_texture_dimension_2d: 512,
            ..limits
        };
        assert_eq!(validate(&baseline, 1, &limits).unwrap(), (257, 19));
        assert!(validate(&baseline, 2, &limits).is_err());
        let frame = Frame {
            width: 2048,
            height: 2048,
            palette: baseline.palette,
            indices: vec![0; 2048 * 2048],
        };
        assert!(validate(&frame, 3, &wgpu::Limits::default()).is_err());
    }

    #[test]
    #[ignore = "requires a GPU adapter; run explicitly in the replay workflow"]
    fn retains_resources_and_reports_gpu_failures() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let make_renderer = || {
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default())).unwrap();
            Renderer::new(device, queue).unwrap()
        };
        let mut renderer = make_renderer();
        let mut frame = Frame::fixture();
        let target = renderer.render(&frame, 1).unwrap().clone();
        let indices = renderer.inputs.as_ref().unwrap().indices.clone();
        let binding = renderer.inputs.as_ref().unwrap().binding.clone();
        let palette = renderer.palette.clone();
        let parameters = renderer.parameters.clone();
        let pipeline = renderer.pipeline.clone();
        frame.palette[0] ^= 1;
        assert_eq!(&target, renderer.render(&frame, 1).unwrap());
        assert_eq!(binding, renderer.inputs.as_ref().unwrap().binding);
        frame.indices[0] ^= 1;
        assert_eq!(&target, renderer.render(&frame, 1).unwrap());
        assert_eq!(indices, renderer.inputs.as_ref().unwrap().indices);
        assert_eq!(binding, renderer.inputs.as_ref().unwrap().binding);
        let scaled = renderer.render(&frame, 2).unwrap().clone();
        assert_ne!(target, scaled);
        assert_eq!(binding, renderer.inputs.as_ref().unwrap().binding);
        frame.width *= 2;
        frame.height *= 2;
        frame
            .indices
            .resize((frame.width * frame.height) as usize, 17);
        assert_eq!(&scaled, renderer.render(&frame, 1).unwrap());
        assert_ne!(indices, renderer.inputs.as_ref().unwrap().indices);
        assert_ne!(binding, renderer.inputs.as_ref().unwrap().binding);
        assert_eq!(palette, renderer.palette);
        assert_eq!(parameters, renderer.parameters);
        assert_eq!(pipeline, renderer.pipeline);
        // Resize with earlier submissions still in flight, retaining no old handles.
        drop((target, indices, binding, scaled));
        for scale in [3, 1, 8, 2, 1] {
            renderer.render(&Frame::fixture(), scale).unwrap();
        }
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        renderer.check_status().unwrap();
        texture(
            &renderer.device,
            "invalid",
            0,
            1,
            wgpu::TextureFormat::R8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        assert!(renderer.check_status().is_err());
        assert!(renderer.render(&Frame::fixture(), 1).is_err());
        let mut renderer = make_renderer();
        renderer.render(&Frame::fixture(), 1).unwrap();
        renderer.device.destroy();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        assert!(renderer.check_status().is_err());
        assert!(renderer.render(&Frame::fixture(), 1).is_err());
    }
}
