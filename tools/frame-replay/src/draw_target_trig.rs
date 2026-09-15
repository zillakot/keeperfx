use super::upload;
use super::*;

impl DrawRenderer {
    /// Geometry sources contain exactly 60 bytes; the texture is a resident 256x256 mask slot.
    pub fn submit_target_triangles(
        &mut self,
        target: u64,
        commands: &[Command],
        slot: u32,
        mask: Option<u64>,
    ) -> Result<()> {
        self.check_status()?;
        let (width, height) = self.target_dimensions(target)?;
        ensure!(
            slot < shadow::SLOTS,
            "shadow mask slot exceeds the resident ring"
        );
        self.shadow_residency()?;
        let limit = self.storage_limit() as usize;
        ensure!(
            commands.len() <= MAX_COMMANDS && commands.len() * RECORD_BYTES <= limit,
            "snapshot triangle batch exceeds limit"
        );
        let dispatch = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            width.div_ceil(8) <= dispatch && height.div_ceil(8) <= dispatch,
            "snapshot triangle dispatch exceeds limit"
        );
        let mut words = Vec::new();
        for c in commands {
            self.check_queued_resource(c.source)?;
            self.check_queued_resource(c.table)?;
        }
        let mut assets = std::collections::HashSet::new();
        let mut demand = 0;
        for id in commands
            .iter()
            .flat_map(|c| [c.source, c.table])
            .chain(mask)
        {
            if assets.insert(id) {
                let resource = self
                    .resources
                    .get(&id)
                    .context("unknown triangle or mask asset")?;
                demand += self.arena.allocation_words(id, resource.bytes.len())?;
            }
        }
        self.arena_headroom(demand.saturating_sub(self.resource_bytes as u64))?;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let mut geometry_bytes = 0;
        let mut table_bytes = 0;
        let mut table_hits = 0;
        let mut table_misses = 0;
        let packed = (|| -> Result<()> {
            for c in commands {
                ensure!(
                    c.abi_version == ABI_VERSION
                        && c.reserved == [0; 3]
                        && c.kind == TRIG
                        && c.source_y == 65536
                        && c.colour <= 255,
                    "invalid snapshot triangle command"
                );
                let geometry = self
                    .resources
                    .get(&c.source)
                    .context("unknown triangle geometry")?;
                ensure!(
                    geometry.bytes.len() == 60,
                    "snapshot triangle geometry must be 60 bytes"
                );
                let mut validation = Resource {
                    cursor: false,
                    width: 1,
                    height: 1,
                    pitch: 1,
                    bytes: geometry.bytes.clone(),
                };
                validation.bytes.resize(60 + 65536, 0);
                let box_of = trig::validate(c, &validation, width, height)?;
                let table = self
                    .resources
                    .get(&c.table)
                    .context("unknown triangle table")?;
                ensure!(
                    table.width == 256
                        && table.height == 320
                        && table.pitch == 256
                        && table.bytes.len() == 81920,
                    "invalid triangle table"
                );
                let before = packer.uploaded_bytes();
                let source_offset =
                    packer.offset(c.source, &geometry.bytes, ResourceKind::TargetTrigGeometry)?;
                geometry_bytes += packer.uploaded_bytes() - before;
                let before = packer.uploaded_bytes();
                let table_offset =
                    packer.offset(c.table, &table.bytes, ResourceKind::TargetTrigTable)?;
                let uploaded = packer.uploaded_bytes() - before;
                table_bytes += uploaded;
                table_hits += u64::from(uploaded == 0);
                table_misses += u64::from(uploaded != 0);
                words.extend([TRIG, 0, 0, c.colour]);
                let policy = self.box_policy;
                let declared = bounds(c.x, c.y, c.width, c.height)?;
                words.extend(if policy.tight {
                    policy.resolve(box_of, width, height, declared)
                } else {
                    declared
                });
                words.extend(bounds(c.clip_x, c.clip_y, c.clip_width, c.clip_height)?);
                words.extend([source_offset, table_offset, 1, slot + 1]);
                words.extend([c.source_x, 65536, 64, 0]);
                words.extend([0; 4]);
                words.extend([OPAQUE, 0, 0, 0]);
            }
            Ok(())
        })();
        let assets = packer.finish();
        if assets.is_none() {
            self.counters.target_trig_geometry_bytes += geometry_bytes;
            self.counters.target_trig_table_bytes += table_bytes;
            self.counters.target_trig_table_hits += table_hits;
            self.counters.target_trig_table_misses += table_misses;
        }
        packed?;
        if commands.is_empty() {
            return Ok(());
        }
        self.tile_index.build(
            &mut self.counters,
            &words,
            &ViewSpace::table(&[ViewSpace::whole(width, height)]),
            &[commands.len()],
            (width, height),
            limit,
        )?;
        let cb = upload::stage(
            &self.uploads,
            &self.device,
            &mut self.counters,
            "snapshot triangle commands",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let tb = upload::stage(
            &self.uploads,
            &self.device,
            &mut self.counters,
            "snapshot triangle tiles",
            self.tile_index.data(),
            wgpu::BufferUsages::STORAGE,
        );
        let assets = match assets {
            Some(assets) => {
                self.counters.target_trig_geometry_bytes += geometry_bytes;
                self.counters.target_trig_table_bytes += table_bytes;
                self.counters.target_trig_table_hits += table_hits;
                self.counters.target_trig_table_misses += table_misses;
                self.counters.asset_upload_bytes += assets.len() as u64 * 4;
                self.counters.target_trig_asset_buffers += 1;
                buffer(
                    &self.device,
                    &mut self.counters,
                    "snapshot triangle fallback assets",
                    &assets,
                    wgpu::BufferUsages::STORAGE,
                )
            }
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        self.counters.shadow_pairs += u64::from(mask.is_some());
        self.counters.command_upload_bytes +=
            (words.len() + self.tile_index.data().len()) as u64 * 4 + 20;
        let pass = self.tile_index.passes()[0];
        let target_view = self.targets[&target].clone();
        // The mask opens another packer before an encoder may exist. Keep its
        // pending triangle readers pinned across that batch and any headroom submit.
        self.arena.hold();
        let recorded = (|| -> Result<()> {
            // The mask writes the resident slot the next shadow's chain reads, so it is
            // recorded even when the triangles' own box leaves nothing to dispatch over.
            if let Some(source) = mask {
                self.record_shadow_mask(source, slot)?;
            }
            if let Some((params, span_x, span_y)) = self.pass_parameters(&target_view, &pass) {
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.compute.get_bind_group_layout(0),
                    entries: &[
                        entry(0, &self.targets[&target].indices),
                        cb.entry(1),
                        entry(2, &assets),
                        params.entry(3),
                        tb.entry(4),
                        entry(5, self.terrain_rows_binding()),
                        entry(6, self.shadow_slot_binding()),
                        entry(7, self.status_binding()),
                    ],
                });
                let stamp = self.stamp(PASS_TARGET_TRIG);
                let compute = self.compute.clone();
                {
                    let encoder = self.frame_encoder();
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("shadow-masked target triangles"),
                        timestamp_writes: stamp.compute(),
                    });
                    pass.set_pipeline(&compute);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(span_x.div_ceil(8), span_y.div_ceil(8), 1);
                }
                self.counters.dispatches += 1;
                self.pass_boundary();
            }
            Ok(())
        })();
        self.arena.release_hold();
        recorded?;
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;
        Ok(())
    }
}
