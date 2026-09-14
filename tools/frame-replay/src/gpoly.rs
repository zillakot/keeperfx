use anyhow::{Result, ensure};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub x: i32,
    pub y: i32,
    pub u: i64,
    pub v: i64,
    pub shade: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub vertices: [Vertex; 3],
}

/// Eight u32 words per row: x, y, count, zero, start low/high, step low/high.
/// Rows are indexed by triangle * height + y; count zero means no coverage.
pub struct PreparedTriangles {
    pub rows: wgpu::Buffer,
    pub triangle_count: u32,
    pub width: u32,
    pub height: u32,
}

pub struct GpolyPreparer {
    pipeline: wgpu::ComputePipeline,
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

impl GpolyPreparer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpoly triangle preparation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpoly_prepare.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpoly triangle preparation"),
            layout: None,
            module: &shader,
            entry_point: Some("prepare"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline }
    }

    /// Records GPU setup from immutable original 16.16 attributes and integer coordinates.
    /// Geometry outside the native signed 16.16 edge domain is rejected before encoding.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        triangles: &[Triangle],
        width: u32,
        height: u32,
        stamp: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> Result<PreparedTriangles> {
        ensure!(
            width > 0 && height > 0 && width <= 32767 && height <= 32767,
            "gpoly viewport exceeds signed 16.16 coordinates"
        );
        ensure!(!triangles.is_empty(), "empty triangle batch");
        let count = u32::try_from(triangles.len())?;
        ensure!(
            count.div_ceil(64) <= device.limits().max_compute_workgroups_per_dimension,
            "triangle dispatch exceeds device limits"
        );
        let size = u64::from(count) * u64::from(height) * 32;
        ensure!(
            size <= device.limits().max_buffer_size
                && u64::from(count) * 96 <= device.limits().max_buffer_size,
            "triangle buffers exceed device allocation limit"
        );
        ensure!(
            size <= device.limits().max_storage_buffer_binding_size,
            "triangle rows exceed device storage limit"
        );
        ensure!(
            u64::from(count) * 96 <= device.limits().max_storage_buffer_binding_size,
            "triangle inputs exceed device storage limit"
        );
        let mut words = Vec::with_capacity(triangles.len() * 24);
        for triangle in triangles {
            for vertex in triangle.vertices {
                ensure!(
                    (-32768..=32767).contains(&vertex.x) && (-32768..=32767).contains(&vertex.y),
                    "vertex exceeds signed 16.16 coordinates"
                );
                words.extend([vertex.x as u32, vertex.y as u32]);
                for value in [vertex.u, vertex.v, vertex.shade] {
                    words.extend([value as u32, (value as u64 >> 32) as u32]);
                }
            }
        }
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable gpoly vertices"),
            contents: &bytes(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpoly viewport"),
            contents: &bytes(&[width, height, count, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let rows = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU prepared gpoly rows"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpoly preparation"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: rows.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpoly triangle preparation"),
                timestamp_writes: stamp,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        Ok(PreparedTriangles {
            rows,
            triangle_count: count,
            width,
            height,
        })
    }
}
