use crate::draw::host::{self, Phase, Scope};
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
/// A count of zero means no coverage.
///
/// Where one triangle's rows live in the shared arena, and the view it was set up
/// against. Row `y` of triangle `t` is at `layout[t].base + (y - layout[t].y_lo)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RowLayout {
    pub base: u32,
    pub y_lo: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
}

/// A prefix sum over each triangle's covered row extent, with the total row count.
/// The extent is the clamped vertex y range, which the setup kernel provably never
/// writes outside: it sorts by y, starts at the lowest vertex, breaks at the highest
/// or at the view edge, and writes no row below zero.
pub fn row_layout(triangles: &[Triangle], extents: &[(u32, u32)]) -> (Vec<RowLayout>, u32) {
    let mut layout = Vec::with_capacity(triangles.len());
    let mut base = 0;
    for (triangle, &(width, height)) in triangles.iter().zip(extents) {
        let edge = i64::from(height);
        let ys = triangle
            .vertices
            .map(|vertex| i64::from(vertex.y).clamp(0, edge) as u32);
        let y_lo = ys.iter().copied().min().unwrap_or(0);
        let y_hi = ys.iter().copied().max().unwrap_or(0);
        layout.push(RowLayout {
            base,
            y_lo,
            rows: y_hi - y_lo,
            width,
            height,
        });
        base += y_hi - y_lo;
    }
    (layout, base)
}

pub struct GpolyPreparer {
    pipeline: wgpu::ComputePipeline,
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

impl GpolyPreparer {
    pub fn new(device: &wgpu::Device) -> Self {
        let _scope = Scope::new(Phase::Bind);
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
    /// `rows` must hold `layout`'s total row count; the caller owns and reuses it.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        triangles: &[Triangle],
        layout: &[RowLayout],
        rows: &wgpu::Buffer,
        stamp: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> Result<()> {
        ensure!(
            layout.len() == triangles.len(),
            "triangle row layout does not match the batch"
        );
        for entry in layout {
            ensure!(
                entry.width > 0
                    && entry.height > 0
                    && entry.width <= 32767
                    && entry.height <= 32767,
                "gpoly viewport exceeds signed 16.16 coordinates"
            );
        }
        ensure!(!triangles.is_empty(), "empty triangle batch");
        let count = u32::try_from(triangles.len())?;
        ensure!(
            count.div_ceil(64) <= device.limits().max_compute_workgroups_per_dimension,
            "triangle dispatch exceeds device limits"
        );
        let size = layout.iter().map(|e| u64::from(e.rows) * 32).sum::<u64>();
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
        ensure!(rows.size() >= size, "prepared row arena is too small");
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
        let upload = Scope::new(Phase::Upload);
        host::created_buffer();
        host::staged_bytes(words.len() * 4);
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable gpoly vertices"),
            contents: &bytes(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        host::created_buffer();
        host::staged_bytes(16);
        let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpoly viewport"),
            contents: &bytes(&[count, 0, 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        drop(upload);
        let mut layout_words = Vec::with_capacity(layout.len() * 5);
        for entry in layout {
            layout_words.extend([
                entry.base,
                entry.y_lo,
                entry.rows,
                entry.width,
                entry.height,
            ]);
        }
        let upload = Scope::new(Phase::Upload);
        host::created_buffer();
        host::staged_bytes(layout_words.len() * 4);
        let layout_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpoly row layout"),
            contents: &bytes(&layout_words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        drop(upload);
        let bind = Scope::new(Phase::Bind);
        host::created_bind_group();
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
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: layout_buffer.as_entire_binding(),
                },
            ],
        });
        drop(bind);
        {
            let _scope = Scope::new(Phase::Encode);
            host::recorded_pass();
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpoly triangle preparation"),
                timestamp_writes: stamp,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        Ok(())
    }
}
