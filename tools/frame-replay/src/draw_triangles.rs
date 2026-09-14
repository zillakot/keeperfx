use super::*;
use crate::gpoly::{GpolyPreparer, Triangle, Vertex, row_layout};

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
    pub(super) prepare: GpolyPreparer,
}

impl TrianglePipelines {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            prepare: GpolyPreparer::new(device),
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
        let dispatch_limit = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            commands.len() <= dispatch_limit as usize,
            "triangle setup dispatch exceeds device limits"
        );
        ensure!(
            target.width.div_ceil(8) <= dispatch_limit
                && target.height.div_ceil(8) <= dispatch_limit,
            "triangle pixel dispatch exceeds device limits"
        );
        let limit = self.storage_limit() as usize;
        let view = ViewSpace::whole(target.width, target.height);
        let geometry: Vec<_> = commands
            .iter()
            .map(|command| Triangle {
                vertices: command.vertices,
            })
            .collect();
        let extents = vec![(view.width, view.height); geometry.len()];
        let (layout, rows) = row_layout(&geometry, &extents);
        let records: Vec<_> = commands.iter().copied().map(Record::Terrain).collect();
        self.arena_headroom(0)?;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let words = pack_records(
            &mut packer,
            records.iter().map(|record| (record.entry(), view, 0)),
            records.len(),
            &self.resources,
            &layout,
            limit,
            self.box_policy,
        )?;
        let assets = packer.finish();
        self.tile_index.build(
            &mut self.counters,
            &words,
            &ViewSpace::table(&[view]),
            &[records.len()],
            (target.width, target.height),
            limit,
        )?;
        let tile_buffer = buffer(
            &self.device,
            &mut self.counters,
            "ordered tile lists",
            self.tile_index.data(),
            wgpu::BufferUsages::STORAGE,
        );
        let command_buffer = buffer(
            &self.device,
            &mut self.counters,
            "immutable ordered commands",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let asset_buffer = match &assets {
            Some(assets) => {
                self.counters.asset_upload_bytes += assets.len() as u64 * 4;
                buffer(
                    &self.device,
                    &mut self.counters,
                    "triangle immutable assets",
                    assets,
                    wgpu::BufferUsages::STORAGE,
                )
            }
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        self.counters.command_upload_bytes +=
            (words.len() + self.tile_index.data().len()) as u64 * 4;
        let rows_buffer = self.prepared_rows(u64::from(rows));
        let mut prepare = Some(PendingPrepare {
            triangles: geometry,
            layout,
            rows: rows_buffer,
        });
        let pass = self.tile_index.passes()[0];
        self.raster_segment(
            &target,
            &(
                Region::whole(command_buffer),
                Region::whole(tile_buffer),
                asset_buffer,
            ),
            &pass,
            commands.len(),
            &mut prepare,
        )
    }
}
