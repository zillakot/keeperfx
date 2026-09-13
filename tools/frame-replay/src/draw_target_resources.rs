use super::*;

pub(super) struct TargetSnapshot {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pitch: u32,
    pub(super) indices: wgpu::Buffer,
}

#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct TargetResourceCounters {
    pub snapshots: u64,
    pub snapshot_copy_bytes: u64,
    pub sampling_copy_bytes: u64,
}

struct ImageBatch {
    words: Vec<u32>,
    snapshots: HashMap<u64, u32>,
    tables: HashMap<u64, u32>,
    asset_words: usize,
}

fn snapshot_size(width: u32, height: u32, pitch: u32, limit: u64) -> Result<u64> {
    crate::frame::dimensions(width, height)?;
    ensure!(pitch >= width && pitch <= 16384, "invalid snapshot pitch");
    let size = u64::from(pitch) * u64::from(height) * 4;
    ensure!(
        size <= limit && size / 4 <= crate::frame::MAX_PIXELS,
        "snapshot exceeds storage limit"
    );
    Ok(size)
}

impl DrawRenderer {
    /// Captures prior queued draws. Padding is zero; later target writes cannot change this version.
    pub fn create_target_snapshot(
        &mut self,
        target: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        pitch: u32,
    ) -> Result<u64> {
        self.check_status()?;
        let source_target = self
            .targets
            .get(&target)
            .context("unknown snapshot target")?;
        let size = snapshot_size(width, height, pitch, self.storage_limit())?;
        ensure!(
            u64::from(x) + u64::from(width) <= u64::from(source_target.width)
                && u64::from(y) + u64::from(height) <= u64::from(source_target.height),
            "snapshot rectangle exceeds target"
        );
        self.checkpoint_target(target)?;
        let target = self
            .targets
            .get(&target)
            .context("unknown snapshot target")?;
        let id = next_handle()?;
        let indices = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("immutable GPU target snapshot"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for row in 0..height {
            encoder.copy_buffer_to_buffer(
                &target.indices,
                (u64::from(target.offset)
                    + u64::from(y + row) * u64::from(target.pitch)
                    + u64::from(x))
                    * 4,
                &indices,
                u64::from(row) * u64::from(pitch) * 4,
                u64::from(width) * 4,
            );
        }
        self.queue.submit([encoder.finish()]);
        self.check_status()?;
        self.target_snapshots.insert(
            id,
            TargetSnapshot {
                width,
                height,
                pitch,
                indices,
            },
        );
        self.target_resource_counters.snapshots += 1;
        self.target_resource_counters.snapshot_copy_bytes +=
            u64::from(width) * u64::from(height) * 4;
        Ok(id)
    }

    pub fn release_target_snapshot(&mut self, snapshot: u64) -> Result<()> {
        if self.deferred_status.is_some() {
            ensure!(
                self.target_snapshots.contains_key(&snapshot)
                    && !self.deferred_snapshot_releases.contains(&snapshot),
                "unknown snapshot"
            );
            self.deferred_snapshot_releases.push(snapshot);
            return Ok(());
        }
        self.target_snapshots
            .remove(&snapshot)
            .context("unknown target snapshot")?;
        Ok(())
    }

    pub fn target_snapshot_dimensions(&self, snapshot: u64) -> Result<(u32, u32, u32)> {
        self.check_status()?;
        let snapshot = self
            .target_snapshots
            .get(&snapshot)
            .context("unknown target snapshot")?;
        Ok((snapshot.width, snapshot.height, snapshot.pitch))
    }

    pub fn target_resource_counters(&self) -> TargetResourceCounters {
        self.target_resource_counters
    }

    /// IMAGE and single TRANSITION sources are GPU snapshots; tables are CPU resources.
    pub fn submit_target_images(&mut self, target: u64, commands: &[Command]) -> Result<()> {
        self.check_status()?;
        let (width, height) = self.target_dimensions(target)?;
        for command in commands {
            self.check_queued_resource(command.table)?;
        }
        let batch = self.pack_target_images(commands)?;
        if commands.is_empty() {
            return Ok(());
        }
        let dispatch_limit = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            width.div_ceil(8) <= dispatch_limit && height.div_ceil(8) <= dispatch_limit,
            "snapshot drawing dispatch exceeds device limit"
        );
        let tiles = bin_commands(&batch.words, width, height, self.storage_limit() as usize)?;
        self.checkpoint_target(target)?;
        let command_buffer = buffer(
            &self.device,
            "snapshot image commands",
            &batch.words,
            wgpu::BufferUsages::STORAGE,
        );
        let tile_buffer = buffer(
            &self.device,
            "snapshot image tile lists",
            &tiles,
            wgpu::BufferUsages::STORAGE,
        );
        let parameters = buffer(
            &self.device,
            "snapshot image dimensions",
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
        let assets = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU snapshot sampling arena"),
            size: batch.asset_words as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut copied = 0;
        for (&id, &offset) in &batch.snapshots {
            let source = &self.target_snapshots[&id];
            let size = u64::from(source.pitch) * u64::from(source.height) * 4;
            encoder.copy_buffer_to_buffer(&source.indices, 0, &assets, u64::from(offset) * 4, size);
            copied += size;
        }
        let mut uploaded = 0;
        for (&id, &offset) in &batch.tables {
            let bytes: Vec<_> = self.resources[&id]
                .bytes
                .iter()
                .flat_map(|&b| u32::from(b).to_le_bytes())
                .collect();
            self.queue
                .write_buffer(&assets, u64::from(offset) * 4, &bytes);
            uploaded += bytes.len() as u64;
        }
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ordered GPU snapshot sampling"),
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                entry(0, &self.targets[&target].indices),
                entry(1, &command_buffer),
                entry(2, &assets),
                entry(3, &parameters),
                entry(4, &tile_buffer),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("immutable source overlapping destination images"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        self.queue.submit([encoder.finish()]);
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += commands.len() as u64;
        self.counters.asset_upload_bytes += uploaded;
        self.counters.command_upload_bytes += (batch.words.len() + tiles.len()) as u64 * 4;
        self.target_resource_counters.sampling_copy_bytes += copied;
        Ok(())
    }

    fn pack_target_images(&self, commands: &[Command]) -> Result<ImageBatch> {
        let limit = self.storage_limit() as usize;
        ensure!(
            commands.len() <= MAX_COMMANDS && commands.len() * 112 <= limit,
            "snapshot command batch exceeds limit"
        );
        let mut batch = ImageBatch {
            words: Vec::with_capacity(commands.len() * 28),
            snapshots: HashMap::new(),
            tables: HashMap::new(),
            asset_words: 0,
        };
        for c in commands {
            ensure!(
                c.abi_version == ABI_VERSION && c.reserved == [0; 3],
                "invalid snapshot command ABI"
            );
            ensure!(
                (c.kind == IMAGE || c.kind == TRANSITION)
                    && c.blend <= 2
                    && c.colour <= 255
                    && c.transparent <= OPAQUE,
                "invalid snapshot image operation"
            );
            let rectangle = bounds(c.x, c.y, c.width, c.height)?;
            let clip = bounds(c.clip_x, c.clip_y, c.clip_width, c.clip_height)?;
            let source = self
                .target_snapshots
                .get(&c.source)
                .context("unknown source snapshot")?;
            if c.kind == TRANSITION {
                ensure!(
                    commands.len() == 1 && c.blend == 0 && c.transparent == OPAQUE,
                    "transition requires one opaque command"
                );
                ensure!(
                    c.x >= 0 && c.y >= 0 && c.width > 0 && c.height > 0,
                    "invalid transition rectangle"
                );
                let table = self
                    .resources
                    .get(&c.table)
                    .context("unknown transition table")?;
                let source_offset = reserve(
                    c.source,
                    source.pitch as usize * source.height as usize,
                    &mut batch.snapshots,
                    &mut batch.asset_words,
                    limit,
                )?;
                let table_offset = reserve(
                    c.table,
                    table.bytes.len(),
                    &mut batch.tables,
                    &mut batch.asset_words,
                    limit,
                )?;
                let mut second_offset = 0;
                let mut second_pitch = 0;
                match c.source_x {
                    0 => {
                        ensure!(
                            (256..=640).contains(&c.width)
                                && (1..=480).contains(&c.height)
                                && c.source_width == c.width
                                && c.source_height == c.height
                                && c.step_low <= 32
                                && table.bytes.len() == 33 * 256 + 65536,
                            "invalid map transition dimensions or tables"
                        );
                        let second_id = u64::from(c.start_low) | (u64::from(c.start_high) << 32);
                        let second = self
                            .target_snapshots
                            .get(&second_id)
                            .context("unknown second transition snapshot")?;
                        ensure!(
                            source.width >= c.width
                                && source.height >= c.height
                                && second.width >= c.width
                                && second.height >= c.height,
                            "transition source too small"
                        );
                        second_offset = reserve(
                            second_id,
                            second.pitch as usize * second.height as usize,
                            &mut batch.snapshots,
                            &mut batch.asset_words,
                            limit,
                        )?;
                        second_pitch = second.pitch;
                    }
                    1 => {
                        ensure!(
                            table.bytes.len() == 65536
                                && (c.x as u64 + c.width as u64) < source.width as u64
                                && (c.y as u64 + c.height as u64) < source.height as u64,
                            "invalid smoothing source or tables"
                        );
                    }
                    _ => anyhow::bail!("unknown transition operation"),
                }
                batch.words.extend([TRANSITION, 0, 0, 0]);
                batch.words.extend(rectangle);
                batch.words.extend(clip);
                batch
                    .words
                    .extend([source_offset, table_offset, source.pitch, 0]);
                batch
                    .words
                    .extend([c.source_x, 0, c.source_width, c.source_height]);
                batch
                    .words
                    .extend([second_offset, second_pitch, c.step_low, 0]);
                batch.words.extend([OPAQUE, 0, 0, 0]);
                continue;
            }
            ensure!(
                c.width > 0 && c.height > 0 && c.source_width > 0 && c.source_height > 0,
                "empty snapshot image"
            );
            ensure!(
                u64::from(c.source_x) + u64::from(c.source_width) <= u64::from(source.width)
                    && u64::from(c.source_y) + u64::from(c.source_height)
                        <= u64::from(source.height),
                "source rectangle exceeds snapshot"
            );
            let source_offset = reserve(
                c.source,
                source.pitch as usize * source.height as usize,
                &mut batch.snapshots,
                &mut batch.asset_words,
                limit,
            )?;
            let mut table_offset = 0;
            if c.blend != 0 {
                let table = self
                    .resources
                    .get(&c.table)
                    .context("unknown snapshot blend table")?;
                ensure!(
                    table.width == 256 && table.pitch == 256 && table.height >= 256,
                    "invalid snapshot blend table layout"
                );
                table_offset = reserve(
                    c.table,
                    table.bytes.len(),
                    &mut batch.tables,
                    &mut batch.asset_words,
                    limit,
                )?;
            }
            batch.words.extend([IMAGE, c.blend, 0, c.colour]);
            batch.words.extend(rectangle);
            batch.words.extend(clip);
            batch
                .words
                .extend([source_offset, table_offset, source.pitch, 0]);
            batch
                .words
                .extend([c.source_x, c.source_y, c.source_width, c.source_height]);
            batch.words.extend([0; 4]);
            batch.words.extend([c.transparent, 0, 0, 0]);
        }
        Ok(batch)
    }
}

fn reserve(
    id: u64,
    length: usize,
    offsets: &mut HashMap<u64, u32>,
    total: &mut usize,
    limit: usize,
) -> Result<u32> {
    if let Some(&offset) = offsets.get(&id) {
        return Ok(offset);
    }
    let next = total
        .checked_add(length)
        .context("snapshot arena length overflow")?;
    ensure!(
        next <= limit / 4 && next <= u32::MAX as usize,
        "snapshot arena exceeds storage limit"
    );
    let offset = *total as u32;
    offsets.insert(id, offset);
    *total = next;
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_storage_bounds() {
        assert_eq!(snapshot_size(3, 2, 5, 40).unwrap(), 40);
        for (w, h, p, limit) in [
            (0, 1, 1, 40),
            (3, 2, 2, 40),
            (3, 2, 5, 39),
            (1, 1, 16385, u64::MAX),
            (8192, 4096, 8192, u64::MAX),
        ] {
            assert!(snapshot_size(w, h, p, limit).is_err());
        }
        let mut offsets = HashMap::new();
        let mut total = 0;
        assert_eq!(reserve(1, 4, &mut offsets, &mut total, 16).unwrap(), 0);
        assert_eq!(reserve(1, 4, &mut offsets, &mut total, 16).unwrap(), 0);
        assert!(reserve(2, 1, &mut offsets, &mut total, 16).is_err());
        assert_eq!(total, 4);
        assert_eq!(offsets.len(), 1);
        let mut offsets = HashMap::new();
        let mut total = u32::MAX as usize;
        assert!(reserve(1, 1, &mut offsets, &mut total, usize::MAX).is_err());
        assert!(offsets.is_empty());
    }

    #[test]
    #[ignore = "requires GPU adapter"]
    fn gpu_snapshot_device_loss_rejects_unrecoverable_versions() {
        let mut draw = DrawRenderer::headless().unwrap();
        let target = draw.create_target(4, 4).unwrap();
        let snapshot = draw.create_target_snapshot(target, 0, 0, 4, 4, 4).unwrap();
        draw.device.destroy();
        draw.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        assert!(draw.check_status().is_err());
        assert!(draw.create_target_snapshot(target, 0, 0, 4, 4, 4).is_err());
        assert!(draw.target_snapshot_dimensions(snapshot).is_err());
        assert!(
            draw.submit_target_images(
                target,
                &[Command {
                    kind: IMAGE,
                    source: snapshot,
                    width: 4,
                    height: 4,
                    source_width: 4,
                    source_height: 4,
                    ..Default::default()
                }]
            )
            .is_err()
        );
        draw.release_target_snapshot(snapshot).unwrap();
    }
}
