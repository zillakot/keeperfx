use super::*;

pub(super) const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct FrameCounters {
    pub queued_commands: u64,
    pub checkpoints: u64,
    pub validation_waits: u64,
    pub validation_bytes: u64,
    pub checkpoint_copy_bytes: u64,
    pub rejected_checkpoints: u64,
}

pub(super) enum Batch {
    Commands(u64, Vec<Command>),
    Triangles(u64, Vec<TriangleCommand>),
}

pub(super) struct QueuedFrame {
    root: u64,
    batches: Vec<Batch>,
    count: usize,
    released_resources: std::collections::HashSet<u64>,
    released_targets: Vec<u64>,
    invalid: bool,
}

impl DrawRenderer {
    pub fn frame_counters(&self) -> FrameCounters {
        self.frame_counters
    }

    pub fn create_target_view(
        &mut self,
        parent: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<u64> {
        self.check_status()?;
        crate::frame::dimensions(width, height)?;
        let parent = self.targets.get(&parent).context("unknown view parent")?;
        ensure!(
            u64::from(x) + u64::from(width) <= u64::from(parent.width)
                && u64::from(y) + u64::from(height) <= u64::from(parent.height),
            "view exceeds parent"
        );
        let id = next_handle()?;
        self.targets.insert(
            id,
            Target {
                width,
                height,
                indices: parent.indices.clone(),
                root: parent.root,
                pitch: parent.pitch,
                offset: parent.offset + y * parent.pitch + x,
            },
        );
        Ok(id)
    }

    pub fn frame_begin(&mut self, root: u64) -> Result<()> {
        self.check_status()?;
        ensure!(self.frame.is_none(), "frame already active");
        let target = self.targets.get(&root).context("unknown frame target")?;
        ensure!(target.root == root, "frame target must be canonical");
        self.frame = Some(QueuedFrame {
            root,
            batches: Vec::new(),
            count: 0,
            released_resources: std::collections::HashSet::new(),
            released_targets: Vec::new(),
            invalid: false,
        });
        Ok(())
    }

    fn frame_target_aliases(&self, target: u64) -> Result<bool> {
        let Some(frame) = &self.frame else {
            return Ok(false);
        };
        ensure!(!frame.invalid, "queued frame is invalid; abort required");
        ensure!(
            !frame.released_targets.contains(&target),
            "released queued target"
        );
        let target = self.targets.get(&target).context("unknown frame target")?;
        let root = self
            .targets
            .get(&frame.root)
            .context("missing frame root")?;
        Ok(target.indices == root.indices)
    }

    pub(super) fn checkpoint_target(&mut self, target: u64) -> Result<()> {
        self.check_status()?;
        if self.frame_target_aliases(target)? {
            self.frame_flush()?;
        }
        Ok(())
    }

    fn check_queued_target(&self, count: usize) -> Result<()> {
        let frame = self.frame.as_ref().unwrap();
        ensure!(!frame.invalid, "queued frame is invalid; abort required");
        ensure!(
            frame
                .count
                .checked_add(count)
                .is_some_and(|n| n <= MAX_COMMANDS),
            "queued frame command limit exceeded"
        );
        ensure!(
            self.resource_bytes <= MAX_FRAME_BYTES,
            "queued resource arena exceeds 256 MiB"
        );
        Ok(())
    }

    pub(super) fn check_queued_resource(&self, handle: u64) -> Result<()> {
        ensure!(
            handle == 0
                || (self.resources.contains_key(&handle)
                    && !self
                        .frame
                        .as_ref()
                        .is_some_and(|frame| frame.released_resources.contains(&handle))),
            "unknown or released queued resource"
        );
        Ok(())
    }

    pub(super) fn enqueue_commands(&mut self, target: u64, commands: &[Command]) -> Result<bool> {
        if self.frame.is_none() {
            return Ok(false);
        }
        self.check_status()?;
        let aliases = self.frame_target_aliases(target)?;
        for command in commands {
            ensure!(
                command.abi_version == ABI_VERSION && command.reserved == [0; 3],
                "invalid queued command ABI"
            );
            self.check_queued_resource(command.source)?;
            self.check_queued_resource(command.table)?;
        }
        if !aliases {
            return Ok(false);
        }
        self.check_queued_target(commands.len())?;
        if commands.is_empty() {
            return Ok(true);
        }
        let compatible = |commands: &[Command]| {
            commands
                .iter()
                .all(|c| !matches!(c.kind, LENS_EFFECT | MINIMAP) && !sprites::ordered(c))
        };
        let frame = self.frame.as_mut().unwrap();
        frame.count += commands.len();
        self.frame_counters.queued_commands += commands.len() as u64;
        if compatible(commands)
            && let Some(Batch::Commands(prior_target, prior)) = frame.batches.last_mut()
            && *prior_target == target
            && compatible(prior)
        {
            prior.extend_from_slice(commands);
            return Ok(true);
        }
        frame
            .batches
            .push(Batch::Commands(target, commands.to_vec()));
        Ok(true)
    }

    pub(super) fn enqueue_triangles(
        &mut self,
        target: u64,
        commands: &[TriangleCommand],
    ) -> Result<bool> {
        if self.frame.is_none() {
            return Ok(false);
        }
        self.check_status()?;
        let aliases = self.frame_target_aliases(target)?;
        for command in commands {
            ensure!(
                command.abi_version == ABI_VERSION && command.reserved == 0,
                "invalid queued triangle ABI"
            );
            self.check_queued_resource(command.source)?;
            self.check_queued_resource(command.table)?;
        }
        if !aliases {
            return Ok(false);
        }
        self.check_queued_target(commands.len())?;
        if commands.is_empty() {
            return Ok(true);
        }
        let frame = self.frame.as_mut().unwrap();
        frame.count += commands.len();
        self.frame_counters.queued_commands += commands.len() as u64;
        if let Some(Batch::Triangles(prior_target, prior)) = frame.batches.last_mut()
            && *prior_target == target
        {
            prior.extend_from_slice(commands);
            return Ok(true);
        }
        frame
            .batches
            .push(Batch::Triangles(target, commands.to_vec()));
        Ok(true)
    }

    pub(super) fn defer_resource_release(&mut self, resource: u64) -> Result<bool> {
        let Some(frame) = &mut self.frame else {
            return Ok(false);
        };
        ensure!(
            self.resources.contains_key(&resource) && !frame.released_resources.contains(&resource),
            "unknown resource"
        );
        frame.released_resources.insert(resource);
        Ok(true)
    }

    pub(super) fn defer_target_release(&mut self, target: u64) -> Result<bool> {
        let Some(frame) = &mut self.frame else {
            return Ok(false);
        };
        ensure!(
            self.targets.contains_key(&target) && !frame.released_targets.contains(&target),
            "unknown target"
        );
        ensure!(target != frame.root, "cannot release active frame root");
        frame.released_targets.push(target);
        Ok(true)
    }

    fn drain_releases(&mut self, frame: &mut QueuedFrame) {
        for id in frame.released_resources.drain() {
            if let Some(released) = self.resources.remove(&id) {
                self.resource_bytes -= released.bytes.len();
            }
        }
        for id in frame.released_targets.drain(..) {
            self.targets.remove(&id);
        }
    }

    fn validate_frame_status(&mut self, statuses: &[wgpu::Buffer]) -> Result<()> {
        if statuses.is_empty() {
            return self.check_status();
        }
        let size = statuses.len() as u64 * 4;
        let staging = self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("aggregated frame validation"),
            size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (i, status) in statuses.iter().enumerate() {
            encoder.copy_buffer_to_buffer(status, 0, &staging, i as u64 * 4, 4);
        }
        self.submit_encoder(encoder);
        let (sender, receiver) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        self.wait_for_queue()?;
        receiver.recv()??;
        let mapped = staging.slice(..).get_mapped_range()?;
        let valid = mapped.iter().all(|&b| b == 0);
        drop(mapped);
        staging.unmap();
        self.counters.readback_bytes += size;
        self.frame_counters.validation_waits += 1;
        self.frame_counters.validation_bytes += size;
        self.check_status()?;
        ensure!(valid, "queued frame has an invalid GPU lookup");
        Ok(())
    }

    pub fn frame_flush(&mut self) -> Result<()> {
        let Some(mut frame) = self.frame.take() else {
            return Ok(());
        };
        if frame.invalid {
            self.frame = Some(frame);
            anyhow::bail!("queued frame is invalid; abort required");
        }
        if frame.batches.is_empty() {
            self.drain_releases(&mut frame);
            self.frame = Some(frame);
            return self.check_status();
        }
        let target = self
            .targets
            .get(&frame.root)
            .context("missing frame root")?
            .clone();
        let size = u64::from(target.width) * u64::from(target.height) * 4;
        let scratch = self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("transactional queued frame"),
            size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&target.indices, 0, &scratch, 0, size);
        self.submit_encoder(encoder);
        for view in self.targets.values_mut().filter(|t| t.root == frame.root) {
            view.indices = scratch.clone();
        }
        let prior_background = self.minimap.as_ref().and_then(|m| m.background);
        let prior_snapshots: std::collections::HashSet<_> =
            self.target_snapshots.keys().copied().collect();
        self.deferred_status = Some(Vec::new());
        let mut result = (|| {
            for batch in frame.batches.drain(..) {
                match batch {
                    Batch::Commands(target, commands) => self.submit(target, &commands)?,
                    Batch::Triangles(target, commands) => {
                        self.submit_triangles(target, &commands)?
                    }
                }
            }
            Ok(())
        })();
        let statuses = self.deferred_status.take().unwrap();
        if result.is_ok() {
            result = self.validate_frame_status(&statuses);
        }
        for view in self.targets.values_mut().filter(|t| t.root == frame.root) {
            view.indices = target.indices.clone();
        }
        if result.is_ok() {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&scratch, 0, &target.indices, 0, size);
            self.submit_encoder(encoder);
            result = self.check_status();
        }
        self.frame_counters.checkpoints += 1;
        self.frame_counters.checkpoint_copy_bytes += size * if result.is_ok() { 2 } else { 1 };
        if result.is_err() {
            self.frame_counters.rejected_checkpoints += 1;
            if let Some(minimap) = &mut self.minimap {
                minimap.background = prior_background;
            }
            self.target_snapshots
                .retain(|id, _| prior_snapshots.contains(id));
        }
        if result.is_ok() {
            for id in self.deferred_snapshot_releases.drain(..) {
                self.target_snapshots.remove(&id);
            }
        } else {
            self.deferred_snapshot_releases.clear();
        }
        frame.invalid = result.is_err();
        frame.batches.clear();
        frame.count = 0;
        self.drain_releases(&mut frame);
        self.frame = Some(frame);
        result
    }

    pub fn frame_end(&mut self) -> Result<()> {
        ensure!(self.frame.is_some(), "no active frame");
        self.frame_flush()?;
        self.frame = None;
        Ok(())
    }

    pub fn frame_abort(&mut self) -> Result<()> {
        if let Some(mut frame) = self.frame.take() {
            self.drain_releases(&mut frame);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpoly::Vertex;

    fn rectangle(colour: u32, x: i32, y: i32, width: u32, height: u32) -> Command {
        Command {
            colour,
            x,
            y,
            width,
            height,
            ..Default::default()
        }
    }

    #[test]
    #[ignore = "requires a Metal adapter"]
    fn gpu_frame_counters_account_for_submits_waits_buffers_and_dispatches() {
        let mut draw = DrawRenderer::headless().unwrap();
        let root = draw.create_target(16, 16).unwrap();
        let baseline = draw.counters();
        assert_eq!(baseline.buffers, 1);
        assert_eq!(baseline.buffer_bytes, 16 * 16 * 4);
        assert_eq!(
            (baseline.submits, baseline.waits, baseline.dispatches),
            (0, 0, 0)
        );
        draw.frame_begin(root).unwrap();
        let bytes = vec![9u8; 64];
        let source = draw.create_resource(&bytes, 8, 8, 8).unwrap();
        assert_eq!(draw.resource_bytes, 64);
        draw.submit(root, &[rectangle(3, 0, 0, 4, 4)]).unwrap();
        assert_eq!(draw.counters().submits, 0);
        draw.frame_end().unwrap();
        let after = draw.counters();
        assert!(
            after.submits >= 3,
            "checkpoint copies and the batch must submit"
        );
        assert_eq!(after.dispatches, 1);
        assert_eq!(after.waits, 0);
        assert_eq!(after.wait_ns, 0);
        assert!(after.buffers > baseline.buffers);
        assert!(after.buffer_bytes >= baseline.buffer_bytes + 16 * 16 * 4);
        assert_eq!(draw.frame_counters().checkpoints, 1);
        assert_eq!(draw.frame_counters().checkpoint_copy_bytes, 2 * 16 * 16 * 4);
        let before_readback = draw.counters().submits;
        draw.readback(root).unwrap();
        let read = draw.counters();
        assert_eq!(read.submits, before_readback + 1);
        assert_eq!(read.waits, 1);
        assert!(
            read.wait_ns > 0,
            "a blocking poll must record measured time"
        );
        assert_eq!(read.readback_bytes, 16 * 16 * 4);
        draw.release_resource(source).unwrap();
        assert_eq!(draw.resource_bytes, 0);
        assert!(draw.counters().gpu_spans == 0 && draw.counters().gpu_span_ns == 0);
    }

    #[test]
    #[ignore = "requires a Metal adapter"]
    fn gpu_deferred_resource_releases_are_a_set_and_track_arena_bytes() {
        let mut draw = DrawRenderer::headless().unwrap();
        let root = draw.create_target(8, 8).unwrap();
        let first = draw.create_resource(&[1u8; 32], 8, 4, 8).unwrap();
        let second = draw.create_resource(&[2u8; 16], 8, 2, 8).unwrap();
        assert_eq!(draw.resource_bytes, 48);
        draw.frame_begin(root).unwrap();
        draw.release_resource(first).unwrap();
        assert!(draw.release_resource(first).is_err());
        assert!(draw.check_queued_resource(first).is_err());
        assert!(draw.check_queued_resource(second).is_ok());
        assert_eq!(
            draw.resource_bytes, 48,
            "deferred releases keep the queued version"
        );
        draw.frame_end().unwrap();
        assert_eq!(draw.resource_bytes, 16);
        draw.release_resource(second).unwrap();
        assert_eq!(draw.resource_bytes, 0);
    }

    #[test]
    #[ignore = "requires a Metal adapter"]
    fn gpu_queued_views_copy_inputs_batch_and_checkpoint() {
        let mut draw = DrawRenderer::headless().unwrap();
        let root = draw.create_target(13, 9).unwrap();
        let view = draw.create_target_view(root, 3, 2, 6, 4).unwrap();
        let nested = draw.create_target_view(view, 2, 1, 3, 2).unwrap();
        assert!(draw.create_target_view(root, 12, 0, 2, 1).is_err());
        draw.frame_begin(root).unwrap();
        draw.submit(
            root,
            &[Command {
                kind: CLEAR,
                colour: 7,
                ..Default::default()
            }],
        )
        .unwrap();
        let mut bytes = vec![31, 32, 33, 34, 35, 36];
        let source = draw.create_resource(&bytes, 3, 2, 3).unwrap();
        let mut image = Command {
            kind: IMAGE,
            source,
            width: 6,
            height: 4,
            source_width: 3,
            source_height: 2,
            ..Default::default()
        };
        draw.submit(view, &[image]).unwrap();
        bytes.fill(255);
        image.colour = 255;
        draw.release_resource(source).unwrap();
        assert!(draw.submit(view, &[image]).is_err());
        draw.submit(nested, &[rectangle(99, 0, 0, 3, 2)]).unwrap();
        for x in 0..13 {
            draw.submit(root, &[rectangle(120 + x as u32, x, 8, 1, 1)])
                .unwrap();
        }
        draw.release_target(view).unwrap();
        assert_eq!(draw.counters().batches, 0);
        assert_eq!(draw.counters().asset_upload_bytes, 0);
        assert_eq!(draw.counters().readback_bytes, 0);
        draw.frame_end().unwrap();
        assert_eq!(draw.counters().batches, 4);
        assert_eq!(draw.counters().readback_bytes, 0);
        assert_eq!(draw.counters().asset_upload_bytes, 36);
        assert_eq!(draw.frame_counters().validation_waits, 0);
        let mut expected = vec![7; 13 * 9];
        for y in 0..4 {
            for x in 0..6 {
                expected[(y + 2) * 13 + x + 3] = 31 + (y / 2 * 3 + x / 2) as u8;
            }
        }
        for y in 3..5 {
            for x in 5..8 {
                expected[y * 13 + x] = 99;
            }
        }
        for x in 0..13 {
            expected[8 * 13 + x] = 120 + x as u8;
        }
        assert_eq!(draw.readback(root).unwrap(), expected);
        assert_eq!(draw.readback(nested).unwrap(), vec![99; 6]);
        assert!(!draw.resources.contains_key(&source));
        assert!(!draw.targets.contains_key(&view));
        draw.frame_begin(root).unwrap();
        draw.submit(nested, &[rectangle(42, 0, 0, 3, 2)]).unwrap();
        let snapshot = draw.create_target_snapshot(nested, 0, 0, 3, 2, 3).unwrap();
        draw.submit_target_images(
            root,
            &[Command {
                kind: IMAGE,
                source: snapshot,
                width: 3,
                height: 2,
                source_width: 3,
                source_height: 2,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.release_target_snapshot(snapshot).unwrap();
        draw.frame_end().unwrap();
        for y in 0..2 {
            for x in 0..3 {
                expected[y * 13 + x] = 42;
            }
        }
        for y in 3..5 {
            for x in 5..8 {
                expected[y * 13 + x] = 42;
            }
        }
        assert_eq!(draw.readback(root).unwrap(), expected);
    }

    fn triangle(source: u64, table: u64, shade: i64) -> TriangleCommand {
        TriangleCommand {
            abi_version: ABI_VERSION,
            reserved: 0,
            source,
            table,
            vertices: [(1, 1), (7, 2), (2, 7)].map(|(x, y)| Vertex {
                x,
                y,
                u: 0,
                v: 0,
                shade,
            }),
        }
    }

    #[test]
    #[ignore = "requires a Metal adapter"]
    fn gpu_queued_mixed_triangles_aggregate_validation_and_rollback() {
        let mut draw = DrawRenderer::headless().unwrap();
        let root = draw.create_target(12, 11).unwrap();
        let view = draw.create_target_view(root, 2, 2, 8, 8).unwrap();
        let reference = draw.create_target(12, 11).unwrap();
        let reference_view = draw.create_target_view(reference, 2, 2, 8, 8).unwrap();
        let source = draw.create_resource(&vec![37; 8192], 32, 32, 256).unwrap();
        let fade: Vec<_> = (0..16384).map(|i| i as u8).collect();
        let table = draw.create_resource(&fade, 256, 64, 256).unwrap();
        let valid = triangle(source, table, 0);
        let clear = Command {
            kind: CLEAR,
            colour: 5,
            ..Default::default()
        };
        let overlay = rectangle(81, 2, 2, 2, 2);
        draw.submit(reference, &[clear]).unwrap();
        draw.submit_triangles(reference_view, &[valid]).unwrap();
        draw.submit(reference_view, &[overlay]).unwrap();
        draw.submit_triangles(reference, &[valid]).unwrap();
        let expected = draw.readback(reference).unwrap();
        assert!(expected.contains(&37));
        let before = draw.counters();
        draw.frame_begin(root).unwrap();
        draw.submit(root, &[clear]).unwrap();
        draw.submit_triangles(view, &[valid]).unwrap();
        draw.submit(view, &[overlay]).unwrap();
        draw.submit_triangles(root, &[valid]).unwrap();
        assert_eq!(draw.counters().readback_bytes, before.readback_bytes);
        draw.frame_end().unwrap();
        assert_eq!(draw.counters().readback_bytes - before.readback_bytes, 8);
        assert_eq!(draw.frame_counters().validation_waits, 1);
        assert_eq!(draw.readback(root).unwrap(), expected);
        draw.frame_begin(root).unwrap();
        draw.submit(
            root,
            &[Command {
                colour: 201,
                ..clear
            }],
        )
        .unwrap();
        draw.submit_triangles(view, &[triangle(source, table, 70 << 16)])
            .unwrap();
        assert!(draw.frame_flush().is_err());
        assert!(draw.frame_end().is_err());
        assert!(draw.readback(root).is_err());
        draw.frame_abort().unwrap();
        assert_eq!(draw.readback(root).unwrap(), expected);
        assert_eq!(draw.frame_counters().rejected_checkpoints, 1);
        draw.frame_begin(root).unwrap();
        draw.submit(root, &[clear]).unwrap();
        draw.submit(
            view,
            &[Command {
                kind: 0xffff,
                ..overlay
            }],
        )
        .unwrap();
        assert!(draw.frame_end().is_err());
        draw.frame_abort().unwrap();
        assert_eq!(draw.readback(root).unwrap(), expected);
    }
    #[test]
    #[ignore = "requires a Metal adapter"]
    fn gpu_queued_ordered_sprite_and_alias_lens_keep_view_pitch() {
        let mut draw = DrawRenderer::headless().unwrap();
        let root = draw.create_target(11, 9).unwrap();
        let view = draw.create_target_view(root, 3, 2, 6, 6).unwrap();
        let reference = draw.create_target(6, 6).unwrap();
        let mut sprite = vec![77, 2];
        for n in [2u32, 2, 2, 2] {
            sprite.extend(n.to_le_bytes());
        }
        sprite.extend(0..=255);
        let source = draw.create_resource(&sprite, 1, 1, 1).unwrap();
        let sprite_command = Command {
            kind: SPRITE,
            source,
            source_x: 9,
            source_width: 1,
            source_height: 1,
            width: 6,
            height: 6,
            clip_width: 6,
            clip_height: 6,
            ..Default::default()
        };
        let clear = Command {
            kind: CLEAR,
            colour: 9,
            ..Default::default()
        };
        draw.submit(reference, &[clear, sprite_command]).unwrap();
        let mut expected_view = draw.readback(reference).unwrap();
        assert!(expected_view.contains(&77));
        let header = [
            2u32,
            6,
            6,
            6,
            6,
            0,
            1,
            65536 / 6,
            65536 / 6,
            128,
            0,
            64,
            100,
            101,
            1,
            1,
        ];
        let mut lens: Vec<u8> = header.into_iter().flat_map(u32::to_le_bytes).collect();
        lens.extend([0; 36]);
        lens.push(100);
        let lens_source = draw.create_resource(&lens, 1, 1, 1).unwrap();
        let before = draw.counters().readback_bytes;
        draw.frame_begin(root).unwrap();
        draw.submit(root, &[clear]).unwrap();
        draw.submit(view, &[sprite_command]).unwrap();
        draw.submit(
            view,
            &[Command {
                kind: LENS_EFFECT,
                source: lens_source,
                width: 6,
                height: 6,
                clip_width: 6,
                clip_height: 6,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.submit(view, &[rectangle(201, 5, 5, 1, 1)]).unwrap();
        draw.release_resource(source).unwrap();
        draw.release_resource(lens_source).unwrap();
        draw.frame_end().unwrap();
        assert_eq!(draw.counters().readback_bytes, before);
        for pixel in &mut expected_view {
            *pixel = ((100u32 + u32::from(*pixel)) / 2) as u8;
        }
        expected_view[35] = 201;
        let mut expected = vec![9; 11 * 9];
        for y in 0..6 {
            expected[(y + 2) * 11 + 3..(y + 2) * 11 + 9]
                .copy_from_slice(&expected_view[y * 6..y * 6 + 6]);
        }
        assert_eq!(draw.readback(root).unwrap(), expected);
    }
}
