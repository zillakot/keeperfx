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
    /// Submissions retain triangle order and reject invalid resources or shades before target writes.
    pub fn submit_triangles(&mut self, target: u64, commands: &[TriangleCommand]) -> Result<()> {
        self.check_status()?;
        let target = self.targets.get(&target).context("unknown target")?;
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
        let limit = self.device.limits().max_storage_buffer_binding_size as usize / 4;
        let mut assets = Vec::new();
        let mut offsets = HashMap::new();
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
                let offset = if let Some(offset) = offsets.get(&handle) {
                    *offset
                } else {
                    ensure!(
                        assets
                            .len()
                            .checked_add(length)
                            .is_some_and(|size| size <= limit),
                        "triangle assets exceed storage limit"
                    );
                    let offset = assets.len() as u32;
                    assets.extend(resource.bytes[..length].iter().map(|&byte| u32::from(byte)));
                    offsets.insert(handle, offset);
                    offset
                };
                metadata.push(offset);
            }
            triangles.push(Triangle {
                vertices: command.vertices,
            });
        }
        self.triangles
            .get_or_insert_with(|| TrianglePipelines::new(&self.device));
        let pipelines = self.triangles.as_ref().unwrap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let prepared = pipelines.prepare.encode(
            &self.device,
            &mut encoder,
            &triangles,
            target.width,
            target.height,
        )?;
        let params = buffer(
            &self.device,
            "triangle batch dimensions",
            &[target.width, target.height, commands.len() as u32, 0],
            wgpu::BufferUsages::UNIFORM,
        );
        let status = buffer(
            &self.device,
            "triangle resource validation",
            &[0],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("triangle validation flag readback"),
            size: 4,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        });
        let validation = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipelines.validate.get_bind_group_layout(0),
            entries: &[
                entry(0, &prepared.rows),
                entry(3, &params),
                entry(5, &status),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipelines.validate);
            pass.set_bind_group(0, &validation, &[]);
            pass.dispatch_workgroups(target.height.div_ceil(64), commands.len() as u32, 1);
        }
        encoder.copy_buffer_to_buffer(&status, 0, &staging, 0, 4);
        self.queue.submit([encoder.finish()]);
        self.counters.command_upload_bytes += commands.len() as u64 * 96 + 36;
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        receiver.recv()??;
        let mapped = staging.slice(..).get_mapped_range()?;
        let invalid = mapped.iter().any(|&byte| byte != 0);
        drop(mapped);
        staging.unmap();
        self.counters.readback_bytes += 4;
        self.check_status()?;
        ensure!(!invalid, "invalid triangle shade");
        let asset_buffer = buffer(
            &self.device,
            "triangle immutable assets",
            &assets,
            wgpu::BufferUsages::STORAGE,
        );
        let metadata_buffer = buffer(
            &self.device,
            "triangle ordered resources",
            &metadata,
            wgpu::BufferUsages::STORAGE,
        );
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipelines.render.get_bind_group_layout(0),
            entries: &[
                entry(0, &prepared.rows),
                entry(1, &asset_buffer),
                entry(2, &target.indices),
                entry(3, &params),
                entry(4, &metadata_buffer),
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipelines.render);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(target.width.div_ceil(8), target.height.div_ceil(8), 1);
        }
        self.queue.submit([encoder.finish()]);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;
        self.counters.asset_upload_bytes += assets.len() as u64 * 4;
        self.counters.command_upload_bytes += commands.len() as u64 * 8;
        Ok(())
    }
}
