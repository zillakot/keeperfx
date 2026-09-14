use super::*;
use crate::gpoly::{GpolyPreparer, Triangle, Vertex};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TriangleCommand {
    pub abi_version: u32,
    pub reserved: u32,
    pub source: u64,
    pub table: u64,
    pub vertices: [Vertex; 3],
}

const _: () = assert!(std::mem::size_of::<TriangleCommand>() == 120);
const _: () = assert!(std::mem::size_of::<Vertex>() == 32);

pub(super) struct TrianglePipelines {
    prepare: GpolyPreparer,
    validate: wgpu::ComputePipeline,
    render: wgpu::ComputePipeline,
}

impl TrianglePipelines {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ordered original-vertex terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("draw_triangles.wgsl").into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            prepare: GpolyPreparer::new(device),
            validate: pipeline("validate"),
            render: pipeline("render"),
        }
    }
}

impl DrawRenderer {
    /// Submissions retain triangle order. Invalid resources are rejected before any target
    /// write; an out-of-range shade skips its own pixels and raises the frame status flag.
    pub fn submit_triangles(&mut self, target: u64, commands: &[TriangleCommand]) -> Result<()> {
        if self.enqueue_triangles(target, commands)? {
            return Ok(());
        }
        self.check_status()?;
        let target = self.targets.get(&target).context("unknown target")?.clone();
        ensure!(
            commands.len() <= MAX_COMMANDS,
            "triangle count exceeds limit"
        );
        if commands.is_empty() {
            return Ok(());
        }
        ensure!(
            commands.len() <= self.device.limits().max_compute_workgroups_per_dimension as usize,
            "triangle validation dispatch exceeds device limits"
        );
        ensure!(
            target.width.div_ceil(8) <= self.device.limits().max_compute_workgroups_per_dimension
                && target.height.div_ceil(8)
                    <= self.device.limits().max_compute_workgroups_per_dimension,
            "triangle pixel dispatch exceeds device limits"
        );
        let limit = self.storage_limit() as usize;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let mut metadata = Vec::new();
        let mut triangles = Vec::new();
        for command in commands {
            ensure!(
                command.abi_version == ABI_VERSION && command.reserved == 0,
                "invalid triangle ABI"
            );
            for (handle, texture) in [(command.source, true), (command.table, false)] {
                let resource = self
                    .resources
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
                metadata.push(
                    packer
                        .prefix(handle, &resource.bytes, length)
                        .context("triangle assets exceed buffer limit")?,
                );
            }
            triangles.push(Triangle {
                vertices: command.vertices,
            });
        }
        let assets = packer.finish();
        self.triangles
            .get_or_insert_with(|| TrianglePipelines::new(&self.device));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let prepared = self.triangles.as_ref().unwrap().prepare.encode(
            &self.device,
            &mut encoder,
            &triangles,
            target.width,
            target.height,
        )?;
        let params = buffer(
            &self.device,
            &mut self.counters,
            "triangle batch dimensions",
            &[
                target.width,
                target.height,
                commands.len() as u32,
                0,
                target.pitch,
                target.offset,
                0,
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        {
            let pipelines = self.triangles.as_ref().unwrap();
            let validation = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipelines.validate.get_bind_group_layout(0),
                entries: &[
                    entry(0, &prepared.rows),
                    entry(3, &params),
                    entry(6, &self.status),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipelines.validate);
            pass.set_bind_group(0, &validation, &[]);
            pass.dispatch_workgroups(target.height.div_ceil(64), commands.len() as u32, 1);
        }
        self.counters.dispatches += 1;
        self.counters.command_upload_bytes += commands.len() as u64 * 96 + 36;
        let asset_buffer = match &assets {
            Some(assets) => buffer(
                &self.device,
                &mut self.counters,
                "triangle immutable assets",
                assets,
                wgpu::BufferUsages::STORAGE,
            ),
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        let metadata_buffer = buffer(
            &self.device,
            &mut self.counters,
            "triangle ordered resources",
            &metadata,
            wgpu::BufferUsages::STORAGE,
        );
        {
            let pipelines = self.triangles.as_ref().unwrap();
            let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipelines.render.get_bind_group_layout(0),
                entries: &[
                    entry(0, &prepared.rows),
                    entry(1, &asset_buffer),
                    entry(2, &target.indices),
                    entry(3, &params),
                    entry(4, &metadata_buffer),
                    entry(6, self.status_binding()),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipelines.render);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(target.width.div_ceil(8), target.height.div_ceil(8), 1);
        }
        self.counters.dispatches += 1;
        self.submit_encoder(encoder);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;
        if let Some(assets) = &assets {
            self.counters.asset_upload_bytes += assets.len() as u64 * 4;
        }
        self.counters.command_upload_bytes += commands.len() as u64 * 8;
        Ok(())
    }
}
