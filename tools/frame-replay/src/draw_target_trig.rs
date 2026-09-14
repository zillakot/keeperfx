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
            commands.len() <= MAX_COMMANDS && commands.len() * 112 <= limit,
            "snapshot triangle batch exceeds limit"
        );
        let dispatch = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            width.div_ceil(8) <= dispatch && height.div_ceil(8) <= dispatch,
            "snapshot triangle dispatch exceeds limit"
        );
        let mut words = Vec::new();
        let mut uploads: Vec<(usize, Vec<u32>)> = Vec::new();
        let mut length = 0usize;
        for c in commands {
            self.check_queued_resource(c.source)?;
            self.check_queued_resource(c.table)?;
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
                width: 1,
                height: 1,
                pitch: 1,
                bytes: geometry.bytes.clone(),
            };
            validation.bytes.resize(60 + 65536, 0);
            trig::validate(c, &validation, width, height)?;
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
            let source_offset = length;
            uploads.push((
                length,
                geometry.bytes.iter().map(|&v| u32::from(v)).collect(),
            ));
            length = length.checked_add(60).context("triangle arena overflow")?;
            let table_offset = length;
            uploads.push((length, table.bytes.iter().map(|&v| u32::from(v)).collect()));
            length = length
                .checked_add(table.bytes.len())
                .context("triangle arena overflow")?;
            ensure!(
                length <= limit / 4 && length <= u32::MAX as usize,
                "triangle arena exceeds limit"
            );
            words.extend([TRIG, 0, 0, c.colour]);
            words.extend(bounds(c.x, c.y, c.width, c.height)?);
            words.extend(bounds(c.clip_x, c.clip_y, c.clip_width, c.clip_height)?);
            words.extend([source_offset as u32, table_offset as u32, 1, slot + 1]);
            words.extend([c.source_x, 65536, 64, 0]);
            words.extend([0; 4]);
            words.extend([OPAQUE, 0, 0, 0]);
        }
        if commands.is_empty() {
            return Ok(());
        }
        let tiles = bin_commands(&words, width, height, limit)?;
        self.prepare_trig();
        let cb = buffer(
            &self.device,
            &mut self.counters,
            "snapshot triangle commands",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let tb = buffer(
            &self.device,
            &mut self.counters,
            "snapshot triangle tiles",
            &tiles,
            wgpu::BufferUsages::STORAGE,
        );
        let params = buffer(
            &self.device,
            &mut self.counters,
            "snapshot triangle parameters",
            &[
                width,
                height,
                commands.len() as u32,
                width.div_ceil(16),
                self.targets[&target].pitch,
                self.targets[&target].offset,
                0,
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        let assets = self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU texture and geometry arena"),
            size: length as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut uploaded = 0;
        for (offset, values) in uploads {
            let bytes: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.queue.write_buffer(&assets, offset as u64 * 4, &bytes);
            uploaded += bytes.len() as u64;
        }
        self.counters.asset_upload_bytes += uploaded;
        self.counters.command_upload_bytes += (words.len() + tiles.len()) as u64 * 4 + 20;
        let valid = self.validate_trig_batch(&cb, &assets, &params, width, height)?;
        if self.deferred_status.is_none() {
            self.counters.readback_bytes += 4;
        }
        ensure!(valid, "snapshot triangle invalid lookup");
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                entry(0, &self.targets[&target].indices),
                entry(1, &cb),
                entry(2, &assets),
                entry(3, &params),
                entry(4, &tb),
                entry(6, self.shadow_slot_binding()),
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Some(source) = mask {
            self.record_shadow_mask(&mut encoder, source, slot)?;
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        self.counters.dispatches += 1;
        self.submit_encoder(encoder);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;
        Ok(())
    }
}
