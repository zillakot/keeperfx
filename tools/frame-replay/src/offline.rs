use anyhow::{Result, ensure};
use keeperfx_frame_replay::{frame::Frame, gpu::Renderer};
use std::time::{Duration, Instant};

pub struct Replay {
    pub renderer: Renderer,
    pub adapter: String,
    pub backend: String,
    pub setup_ms: f64,
}

impl Replay {
    pub async fn new() -> Result<Self> {
        let start = Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;
        let renderer = Renderer::new(device, queue)?;
        Ok(Self {
            renderer,
            adapter: info.name,
            backend: format!("{:?}", info.backend),
            setup_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    pub fn render(&mut self, frame: &Frame, scale: u32) -> Result<Vec<u8>> {
        let output = self.renderer.render(frame, scale)?.clone();
        readback(&self.renderer, &output)
    }
}

fn readback(renderer: &Renderer, output: &wgpu::Texture) -> Result<Vec<u8>> {
    let device = renderer.device();
    let queue = renderer.queue();
    let width = output.width();
    let height = output.height();
    let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    ensure!(
        u64::from(stride) * u64::from(height) <= device.limits().max_buffer_size,
        "readback exceeds GPU buffer limit"
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    renderer.check_status()?;
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        output.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    renderer.check_status()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    let result = (|| -> Result<Vec<u8>> {
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })?;
        receiver.recv_timeout(Duration::from_secs(30))??;
        let mapped = readback.slice(..).get_mapped_range()?;
        let pixels = mapped
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..width as usize * 4].iter().copied())
            .collect();
        drop(mapped);
        renderer.check_status()?;
        Ok(pixels)
    })();
    readback.unmap();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use keeperfx_frame_replay::frame::scale_rgba;

    #[test]
    #[ignore = "requires a GPU adapter; run explicitly in the replay workflow"]
    fn consecutive_frames_scales_and_rejected_inputs_preserve_exact_rgba() {
        let mut replay = pollster::block_on(Replay::new()).unwrap();
        let baseline = Frame::fixture();
        for scale in [1, 2, 8, 3, 1] {
            for (_, frame) in crate::sequence::fixture_frames() {
                let expected = scale_rgba(&frame.rgba(), frame.width, frame.height, scale).unwrap();
                assert_eq!(replay.render(&frame, scale).unwrap(), expected);
            }
        }
        // Different input dimensions can produce the same output dimensions.
        let mut resized = baseline.clone();
        resized.width *= 2;
        resized.height *= 2;
        resized
            .indices
            .resize((resized.width * resized.height) as usize, 128);
        replay.render(&baseline, 2).unwrap();
        assert_eq!(replay.render(&resized, 1).unwrap(), resized.rgba());
        for invalid_scale in [0, 9, u32::MAX] {
            assert!(replay.render(&baseline, invalid_scale).is_err());
            assert_eq!(replay.render(&baseline, 1).unwrap(), baseline.rgba());
        }
        for invalid in [
            Frame {
                palette: vec![0; 1023],
                ..baseline.clone()
            },
            Frame {
                indices: vec![],
                ..baseline.clone()
            },
            Frame {
                width: 8193,
                ..baseline.clone()
            },
        ] {
            assert!(replay.render(&invalid, 1).is_err());
            assert_eq!(replay.render(&baseline, 1).unwrap(), baseline.rgba());
        }
        for offset in [17 * 4, 128 * 4, 128 * 4 + 3, 129 * 4 + 3] {
            let mut changed = baseline.clone();
            changed.palette[offset] ^= 1;
            let actual = replay.render(&changed, 2).unwrap();
            let expected =
                scale_rgba(&baseline.rgba(), baseline.width, baseline.height, 2).unwrap();
            let count = baseline
                .indices
                .iter()
                .filter(|&&i| usize::from(i) == offset / 4)
                .count();
            let (different, max, _) = crate::report::compare(&expected, &actual).unwrap();
            assert_eq!((different, max), (count * 4, 1));
            assert_eq!(replay.render(&baseline, 1).unwrap(), baseline.rgba());
        }
        let mut changed = baseline.clone();
        changed.indices[0] ^= 1;
        let actual = replay.render(&changed, 1).unwrap();
        assert_eq!(
            crate::report::compare(&baseline.rgba(), &actual).unwrap().0,
            1
        );
        assert_eq!(replay.render(&baseline, 1).unwrap(), baseline.rgba());
    }
}
