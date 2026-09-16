use super::*;

pub(super) const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// Word 0 is the frame flag; the remaining words name which kernel raised it.
pub(super) const STATUS_WORDS: usize = 8;
pub(super) const STATUS_BYTES: u64 = STATUS_WORDS as u64 * 4;
pub(super) const STATUS_RING: usize = 8;
pub(super) type StatusReceiver = std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct FrameCounters {
    pub queued_commands: u64,
    /// Frame flushes that had to cut the frame's single submission. Structurally zero
    /// since the flush became a replay into the open encoder.
    pub checkpoints: u64,
    pub validation_waits: u64,
    pub validation_bytes: u64,
    pub checkpoint_copy_bytes: u64,
    pub rejected_checkpoints: u64,
    pub invalid_frames: u64,
    pub status_reads: u64,
    pub status_stalls: u64,
}

/// Work the raster stream cannot absorb: it keeps its own pass and closes the
/// raster range at the stream position it was recorded at.
pub(super) enum Serial {
    Commands(u64, Vec<Command>),
    Shadow(u64, u64, u32, Vec<Command>),
}

/// The record buffers a retired frame leaves behind for the next one.
pub(super) type FrameBuffers = (Vec<(u32, Record)>, Vec<ViewSpace>, Vec<(usize, Serial)>);

pub(super) struct QueuedFrame {
    root: u64,
    /// Every rasterizable command of the frame in order, each naming the view it was
    /// issued against, so a target change is not a boundary.
    stream: Vec<(u32, Record)>,
    views: Vec<ViewSpace>,
    serials: Vec<(usize, Serial)>,
    count: usize,
    released_resources: std::collections::HashSet<u64>,
    released_targets: Vec<u64>,
    invalid: bool,
}

impl DrawRenderer {
    pub fn frame_counters(&self) -> FrameCounters {
        self.frame_counters
    }

    pub(super) fn status_binding(&self) -> &wgpu::Buffer {
        &self.status
    }

    /// Records the status copy and reset into an encoder the caller submits; the
    /// returned slot must then be mapped with `status_map`.
    pub(super) fn status_record(&mut self, encoder: &mut wgpu::CommandEncoder) -> Option<usize> {
        let slot = (self.status_cursor % STATUS_RING as u64) as usize;
        if self.status_pending[slot].is_some() {
            self.status_drain();
        }
        if self.status_pending[slot].is_some() {
            self.frame_counters.status_stalls += 1;
            return None;
        }
        encoder.copy_buffer_to_buffer(&self.status, 0, &self.status_ring[slot], 0, STATUS_BYTES);
        encoder.clear_buffer(&self.status, 0, None);
        self.status_cursor += 1;
        Some(slot)
    }

    pub(super) fn status_map(&mut self, slot: usize) {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.status_ring[slot]
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.status_frame[slot] = self.frame_index;
        self.status_pending[slot] = Some(receiver);
    }

    /// Collects completed maps. The poll is the non-blocking kind wgpu needs to run
    /// map callbacks, so a status read never waits on the queue.
    pub(super) fn status_drain(&mut self) {
        if self.status_pending.iter().all(Option::is_none) {
            return;
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
        for slot in 0..STATUS_RING {
            let received = match &self.status_pending[slot] {
                Some(receiver) => receiver.try_recv(),
                None => continue,
            };
            match received {
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Ok(Ok(())) => {
                    let mut flags = 0u32;
                    if let Ok(mapped) = self.status_ring[slot].slice(..).get_mapped_range() {
                        for (word, bytes) in mapped.as_chunks::<4>().0.iter().enumerate() {
                            if u32::from_le_bytes(*bytes) != 0 {
                                flags |= 1 << word;
                            }
                        }
                    }
                    self.status_ring[slot].unmap();
                    self.frame_counters.status_reads += 1;
                    if flags != 0 {
                        self.frame_counters.invalid_frames += 1;
                        self.frame_flags |= flags;
                        self.frame_flags_index = self.status_frame[slot];
                    }
                }
                _ => {}
            }
            self.status_pending[slot] = None;
        }
    }

    /// Flags raised by the most recent frame whose staging read has completed,
    /// with the frame index they belong to. Never blocks; clears what it reports.
    pub fn frame_status(&mut self) -> (u64, u32) {
        self.status_drain();
        let flags = self.frame_flags;
        self.frame_flags = 0;
        (self.frame_flags_index, flags)
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
        self.uploads.borrow_mut().begin_frame();
        self.check_status()?;
        // A presenter that never reached its present call still owes the queue what it
        // recorded; nothing may straddle two frames' worth of staged writes.
        self.frame_submit()?;
        self.status_drain();
        // One GPU-occupancy union per frame: passes that were in flight together are
        // counted once, which the per-drain close could not do because a drain usually
        // harvests a single submission.
        if let Some(timings) = &mut self.timings {
            timings.settle();
        }
        self.arena.lengths.begin_frame();
        self.frame_index += 1;
        ensure!(self.frame.is_none(), "frame already active");
        // Growth is forbidden once the encoder is open, so it happens here, sized to
        // the demand the previous frames showed.
        self.arena.grow_to(
            &self.device,
            &self.queue,
            &mut self.counters,
            self.resource_bytes as u64,
        );
        let target = self.targets.get(&root).context("unknown frame target")?;
        ensure!(target.root == root, "frame target must be canonical");
        let (mut stream, mut views, mut serials) = self.frame_buffers.take().unwrap_or_default();
        stream.clear();
        views.clear();
        serials.clear();
        self.frame = Some(QueuedFrame {
            root,
            stream,
            views,
            serials,
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
        let index = self.view_index(target)?;
        let frame = self.frame.as_mut().unwrap();
        frame.count += commands.len();
        self.frame_counters.queued_commands += commands.len() as u64;
        for command in commands {
            if !matches!(command.kind, LENS_EFFECT | MINIMAP) && !sprites::ordered(command) {
                frame.stream.push((index, Record::Command(*command)));
                continue;
            }
            let at = frame.stream.len();
            // Ordered sprites with no record between them layer against each other, so
            // they reach `submit` as one run rather than one route apiece.
            if sprites::ordered(command)
                && let Some((position, Serial::Commands(prior_target, prior))) =
                    frame.serials.last_mut()
                && *prior_target == target
                && *position == at
                && prior.last().is_some_and(sprites::ordered)
            {
                prior.push(*command);
                continue;
            }
            frame
                .serials
                .push((at, Serial::Commands(target, vec![*command])));
        }
        Ok(true)
    }

    /// The frame's index for a target's view space, interning it on first use.
    fn view_index(&mut self, target: u64) -> Result<u32> {
        let view = self.view_space(target)?;
        let frame = self.frame.as_mut().unwrap();
        Ok(match frame.views.iter().position(|known| *known == view) {
            Some(index) => index as u32,
            None => {
                frame.views.push(view);
                frame.views.len() as u32 - 1
            }
        })
    }

    /// The origin and extent of a frame target inside the frame root. Every view
    /// is an offset alias of the root at the root's pitch, which is what lets one
    /// dispatch in root space serve all of them.
    fn view_space(&self, target: u64) -> Result<ViewSpace> {
        let frame = self.frame.as_ref().context("no active frame")?;
        let root = self
            .targets
            .get(&frame.root)
            .context("missing frame root")?;
        let view = self.targets.get(&target).context("unknown frame target")?;
        ensure!(
            view.pitch == root.pitch && root.offset == 0,
            "frame view is not an offset alias of the root"
        );
        let origin_x = view.offset % root.pitch;
        let origin_y = view.offset / root.pitch;
        ensure!(
            origin_x + view.width <= root.width && origin_y + view.height <= root.height,
            "frame view exceeds the root"
        );
        Ok(ViewSpace {
            origin_x,
            origin_y,
            width: view.width,
            height: view.height,
        })
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
        let index = self.view_index(target)?;
        let frame = self.frame.as_mut().unwrap();
        frame.count += commands.len();
        self.frame_counters.queued_commands += commands.len() as u64;
        for command in commands {
            frame.stream.push((index, Record::Terrain(*command)));
        }
        Ok(true)
    }

    /// A mask and the triangles reading its slot are adjacent submissions on one queue,
    /// which is what keeps a later mask from overwriting a slot an earlier one still reads.
    pub(super) fn enqueue_shadow(
        &mut self,
        target: u64,
        source: u64,
        slot: u32,
        commands: &[Command],
    ) -> Result<bool> {
        if self.frame.is_none() {
            return Ok(false);
        }
        self.check_status()?;
        let aliases = self.frame_target_aliases(target)?;
        self.check_queued_resource(source)?;
        for command in commands {
            self.check_queued_resource(command.source)?;
            self.check_queued_resource(command.table)?;
        }
        if !aliases {
            return Ok(false);
        }
        self.check_queued_target(commands.len())?;
        let frame = self.frame.as_mut().unwrap();
        frame.count += commands.len();
        self.frame_counters.queued_commands += commands.len() as u64;
        let at = frame.stream.len();
        frame
            .serials
            .push((at, Serial::Shadow(target, source, slot, commands.to_vec())));
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
                debug_assert!(self.resource_bytes >= released.bytes.len());
                self.resource_bytes = self.resource_bytes.saturating_sub(released.bytes.len());
            }
            self.arena.release(id);
        }
        for id in frame.released_targets.drain(..) {
            self.targets.remove(&id);
        }
    }

    /// Replays whatever the frame has queued into the open encoder. It is no longer a
    /// submission boundary, so a checkpoint costs a replay and nothing else.
    pub fn frame_flush(&mut self) -> Result<()> {
        let replay = host::Replay::begin();
        let result = self.frame_flush_inner();
        if result.is_ok() {
            self.flush_uploads();
        } else {
            self.uploads.borrow_mut().drop_pending();
            self.arena.discard();
        }
        self.counters.replay.accumulate(replay.finish());
        result
    }

    fn frame_flush_inner(&mut self) -> Result<()> {
        let Some(mut frame) = self.frame.take() else {
            return Ok(());
        };
        if frame.invalid {
            self.frame = Some(frame);
            anyhow::bail!("queued frame is invalid; abort required");
        }
        if frame.stream.is_empty() && frame.serials.is_empty() {
            self.drain_releases(&mut frame);
            self.frame = Some(frame);
            return self.check_status();
        }
        self.replaying = true;
        let root = frame.root;
        let mut stream = std::mem::take(&mut frame.stream);
        let mut views = std::mem::take(&mut frame.views);
        let mut serials = std::mem::take(&mut frame.serials);
        // The whole stream is packed before the serial routes between its raster passes
        // open batches of their own, so its residents stay pinned until the last dispatch.
        // The encoder holds the same pin for the whole frame; this covers a replay that
        // records nothing and so never opens one.
        self.arena.hold();
        let mut result = self.replay_stream(root, &stream, &views, &mut serials);
        self.arena.release_hold();
        self.replaying = false;
        // The buffers go back to the frame emptied, so a frame's records grow their capacity
        // once rather than reallocating through it again on the next one.
        stream.clear();
        views.clear();
        serials.clear();
        frame.stream = stream;
        frame.views = views;
        frame.serials = serials;
        if result.is_ok() {
            result = self.check_status();
        }
        if result.is_err() {
            self.frame_counters.rejected_checkpoints += 1;
            self.deferred_snapshot_releases.clear();
        } else {
            for id in self.deferred_snapshot_releases.drain(..) {
                self.target_snapshots.remove(&id);
            }
        }
        frame.invalid = result.is_err();
        frame.count = 0;
        self.drain_releases(&mut frame);
        self.frame = Some(frame);
        result
    }

    /// Replays the frame as one command stream in root space: a raster dispatch per
    /// serial segment over a single tile index, with the serial work in place.
    fn replay_stream(
        &mut self,
        root: u64,
        stream: &[(u32, Record)],
        views: &[ViewSpace],
        serials: &mut Vec<(usize, Serial)>,
    ) -> Result<()> {
        let _scope = Scope::new(Phase::Pack);
        self.check_status()?;
        let limit = self.storage_limit() as usize;
        let target = self
            .targets
            .get(&root)
            .context("missing frame root")?
            .clone();
        let dispatch_limit = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            target.width.div_ceil(8) <= dispatch_limit
                && target.height.div_ceil(8) <= dispatch_limit,
            "drawing dispatch exceeds device limit"
        );
        let mut boundaries = Vec::new();
        let mut prior = 0;
        for (at, _) in serials.iter() {
            if *at > prior {
                boundaries.push(*at);
                prior = *at;
            }
        }
        if stream.len() > prior {
            boundaries.push(stream.len());
        }
        let mut raster = None;
        let mut prepare = None;
        if !boundaries.is_empty() {
            self.arena_headroom(0)?;
            let mut packer = asset_packer(
                &self.device,
                &self.queue,
                &mut self.arena,
                &mut self.counters,
                self.asset_generation,
                limit,
            );
            let mut geometry = Vec::new();
            let mut extents = Vec::new();
            for (view, record) in stream {
                if let Record::Terrain(triangle) = record {
                    let view = views[*view as usize];
                    geometry.push(crate::gpoly::Triangle {
                        vertices: triangle.vertices,
                    });
                    extents.push((view.width, view.height));
                }
            }
            let (layout, rows) = crate::gpoly::row_layout(&geometry, &extents);
            let words = pack_records(
                &mut packer,
                stream
                    .iter()
                    .map(|(view, record)| (record.entry(), views[*view as usize], *view)),
                stream.len(),
                &self.resources,
                &layout,
                limit,
                self.box_policy,
            )?;
            let assets = packer.finish();
            if !geometry.is_empty() {
                let buffer = self.prepared_rows(u64::from(rows));
                prepare = Some(PendingPrepare {
                    triangles: geometry,
                    layout,
                    rows: buffer,
                });
            }
            self.tile_index.build(
                &mut self.counters,
                &words,
                &ViewSpace::table(views),
                &boundaries,
                (target.width, target.height),
                limit,
            )?;
            let commands = upload::stage(
                &self.uploads,
                &self.device,
                &mut self.counters,
                "immutable ordered commands",
                &words,
                wgpu::BufferUsages::STORAGE,
            );
            let tiles = upload::stage(
                &self.uploads,
                &self.device,
                &mut self.counters,
                "ordered tile lists",
                self.tile_index.data(),
                wgpu::BufferUsages::STORAGE,
            );
            let arena = match &assets {
                Some(assets) => {
                    self.counters.asset_upload_bytes += assets.len() as u64 * assets::STRIDE as u64;
                    byte_buffer(
                        &self.device,
                        &mut self.counters,
                        "immutable asset versions",
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
            raster = Some(((commands, tiles, arena), self.tile_index.passes()));
        }
        let mut segment = 0;
        let mut prior = 0;
        for (at, serial) in serials.drain(..) {
            if at > prior {
                let (buffers, passes) = raster.as_ref().unwrap();
                let (buffers, pass) = (buffers.clone(), passes[segment]);
                self.raster_segment(&target, &buffers, &pass, at - prior, &mut prepare)?;
                segment += 1;
                prior = at;
            }
            match serial {
                Serial::Commands(target, commands) => self.submit(target, &commands)?,
                Serial::Shadow(target, source, slot, commands) => {
                    self.submit_shadow_batch(target, source, slot, &commands)?
                }
            }
        }
        if stream.len() > prior {
            let (buffers, passes) = raster.as_ref().unwrap();
            let (buffers, pass) = (buffers.clone(), passes[segment]);
            self.raster_segment(&target, &buffers, &pass, stream.len() - prior, &mut prepare)?;
        }
        Ok(())
    }

    /// Parks a retired frame's record buffers so the next frame reuses their capacity.
    fn retire(&mut self, frame: &mut QueuedFrame) {
        self.frame_buffers = Some((
            std::mem::take(&mut frame.stream),
            std::mem::take(&mut frame.views),
            std::mem::take(&mut frame.serials),
        ));
    }

    /// Never reached on the wgpu presenter, which submits inside the present call and
    /// ends the frame from the next `BeginFrame`; every other caller ends it here.
    pub fn frame_end(&mut self) -> Result<()> {
        ensure!(self.frame.is_some(), "no active frame");
        self.frame_flush()?;
        if let Some(mut frame) = self.frame.take() {
            self.retire(&mut frame);
        }
        self.frame_submit()
    }

    pub fn frame_abort(&mut self) -> Result<()> {
        self.frame_discard();
        if let Some(mut frame) = self.frame.take() {
            self.drain_releases(&mut frame);
            self.retire(&mut frame);
        }
        self.frame_flags = 0;
        self.invalidate_assets();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpoly::Vertex;

    /// Backend-agnostic entry for the GPU tests below: the same bodies cover Metal on a
    /// host, Vulkan (lavapipe) on the Linux job and DX12 (WARP) on the Windows job, and
    /// each one prints the backend and adapter it ran on. Only a host with no adapter at
    /// all stands down, printing why; an adapter that is short of a limit the drawing
    /// context needs is a finding on a backend we claim to cover, so it panics naming the
    /// limit rather than passing quietly.
    fn headless_or_skip(test: &str) -> Option<DrawRenderer> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                // Own line: libtest leaves "test … ... " unterminated, and CI
                // anchors the stand-down at the start of a line.
                eprintln!("\nskipping {test}: no wgpu adapter on this host: {error}");
                return None;
            }
        };
        let mut missing = None;
        wgpu::Limits::default().check_limits_with_fail_fn(
            &adapter.limits(),
            true,
            |limit, required, available| {
                missing = Some(format!(
                    "{limit} needs {required}, adapter offers {available}"
                ));
            },
        );
        let info = adapter.get_info();
        if let Some(missing) = missing {
            panic!(
                "{test} on {:?} adapter {:?}: {missing}",
                info.backend, info.name
            );
        }
        eprintln!("{test} on {:?} adapter {:?}", info.backend, info.name);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&super::timing::device_descriptor(&adapter)))
                .expect("an adapter within the drawing limits must yield a device");
        let renderer = crate::gpu::Renderer::new(device, queue).unwrap();
        Some(DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap())
    }

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
    #[ignore = "requires a GPU adapter"]
    fn gpu_frame_counters_account_for_submits_waits_buffers_and_dispatches() {
        let Some(mut draw) =
            headless_or_skip("gpu_frame_counters_account_for_submits_waits_buffers_and_dispatches")
        else {
            return;
        };
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
        assert_eq!(after.submits, 1, "one command buffer for the whole frame");
        assert_eq!(after.dispatches, 1);
        assert_eq!(draw.staged_asset_bytes(), 64);
        assert_eq!(
            (after.waits, after.wait_ns),
            (0, 0),
            "a production frame must not block"
        );
        assert!(after.buffers > baseline.buffers);
        let frame = draw.frame_counters();
        assert_eq!(frame.checkpoints, 0);
        assert_eq!(
            (
                frame.checkpoint_copy_bytes,
                frame.validation_waits,
                frame.validation_bytes,
                frame.status_stalls,
                frame.rejected_checkpoints
            ),
            (0, 0, 0, 0, 0)
        );
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
        assert_eq!(draw.staged_asset_bytes(), 0);
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_deferred_resource_releases_are_a_set_and_track_arena_bytes() {
        let Some(mut draw) =
            headless_or_skip("gpu_deferred_resource_releases_are_a_set_and_track_arena_bytes")
        else {
            return;
        };
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
    #[ignore = "requires a GPU adapter"]
    fn gpu_queued_views_copy_inputs_batch_and_checkpoint() {
        let Some(mut draw) = headless_or_skip("gpu_queued_views_copy_inputs_batch_and_checkpoint")
        else {
            return;
        };
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
        assert_eq!(
            draw.counters().batches,
            1,
            "three views and the root are one raster pass over the root"
        );
        assert_eq!(draw.counters().readback_bytes, 0);
        assert_eq!(
            draw.counters().asset_upload_bytes,
            6 * assets::STRIDE as u64
        );
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
    #[ignore = "requires a GPU adapter"]
    fn gpu_queued_replay_host_partition_and_counts() {
        const TIMER_SLACK_NS: u64 = 64_000;
        let Some(mut draw) = headless_or_skip("gpu_queued_replay_host_partition_and_counts") else {
            return;
        };
        let root = draw.create_target(16, 16).unwrap();
        let mut tightest_unaccounted = u64::MAX;
        let mut tightest_elapsed = 0;
        for colour in [17, 29, 43, 61, 83, 101, 127, 149, 173, 199, 211, 229] {
            draw.frame_begin(root).unwrap();
            draw.submit(
                root,
                &[Command {
                    kind: CLEAR,
                    colour,
                    ..Default::default()
                }],
            )
            .unwrap();
            let before = draw.counters();
            let start = std::time::Instant::now();
            draw.frame_flush().unwrap();
            let elapsed = start.elapsed().as_nanos() as u64;
            let after = draw.counters();
            let accounted = after.replay.total_ns() - before.replay.total_ns();
            assert!(accounted <= elapsed);
            let unaccounted = elapsed - accounted;
            if unaccounted < tightest_unaccounted {
                tightest_unaccounted = unaccounted;
                tightest_elapsed = elapsed;
            }
            assert!(after.replay.replay_pack_ns > before.replay.replay_pack_ns);
            assert!(after.replay.replay_upload_ns > before.replay.replay_upload_ns);
            assert!(after.replay.replay_bind_ns > before.replay.replay_bind_ns);
            assert!(after.replay.replay_encode_ns > before.replay.replay_encode_ns);
            assert!(after.replay.replay_tile_index_ns > before.replay.replay_tile_index_ns);
            assert!(after.replay.replay_other_ns > before.replay.replay_other_ns);
            assert_eq!(
                after.replay.replay_bind_groups - before.replay.replay_bind_groups,
                1
            );
            assert_eq!(
                after.replay.replay_passes - before.replay.replay_passes,
                after.dispatches - before.dispatches
            );
            assert_eq!(
                after.replay.replay_buffers - before.replay.replay_buffers,
                after.buffers - before.buffers
            );
            assert!(
                after.replay.replay_staged_bytes - before.replay.replay_staged_bytes
                    >= after.command_upload_bytes - before.command_upload_bytes
            );
            draw.frame_end().unwrap();
            assert_eq!(draw.readback(root).unwrap(), vec![colour as u8; 256]);
        }
        assert!(
            tightest_unaccounted <= TIMER_SLACK_NS
                || tightest_unaccounted as f64 <= tightest_elapsed as f64 * 0.05,
            "{tightest_unaccounted} unaccounted of {tightest_elapsed}"
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_queued_mixed_triangles_flag_and_present_invalid_frames() {
        let Some(mut draw) =
            headless_or_skip("gpu_queued_mixed_triangles_flag_and_present_invalid_frames")
        else {
            return;
        };
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
        assert_eq!(draw.counters().readback_bytes, before.readback_bytes);
        assert_eq!(draw.frame_counters().validation_waits, 0);
        assert_eq!(draw.readback(root).unwrap(), expected);
        assert_eq!(draw.frame_status(), (0, 0));
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
        draw.frame_flush().unwrap();
        draw.frame_end().unwrap();
        assert_eq!(draw.readback(root).unwrap(), vec![201; 12 * 11]);
        let (index, flags) = draw.frame_status();
        assert_eq!((index, flags), (2, 1 | 1 << 2));
        assert_eq!(draw.frame_counters().rejected_checkpoints, 0);
        assert_eq!(draw.frame_counters().invalid_frames, 1);
        draw.frame_abort().unwrap();
        draw.frame_begin(root).unwrap();
        draw.submit(root, &[clear]).unwrap();
        draw.submit_triangles(view, &[valid]).unwrap();
        draw.submit(view, &[overlay]).unwrap();
        draw.submit_triangles(root, &[valid]).unwrap();
        draw.frame_end().unwrap();
        assert_eq!(draw.readback(root).unwrap(), expected);
        assert_eq!(draw.frame_status().1, 0);
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
        assert_eq!(
            draw.readback(root).unwrap(),
            expected,
            "the frame's stream is validated as a unit, before any target write"
        );
        draw.frame_begin(root).unwrap();
        draw.submit(root, &[clear]).unwrap();
        draw.submit_triangles(view, &[valid]).unwrap();
        draw.submit(view, &[overlay]).unwrap();
        draw.submit_triangles(root, &[valid]).unwrap();
        draw.frame_end().unwrap();
        assert_eq!(draw.readback(root).unwrap(), expected);
    }
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_queued_ordered_sprite_and_alias_lens_keep_view_pitch() {
        let Some(mut draw) =
            headless_or_skip("gpu_queued_ordered_sprite_and_alias_lens_keep_view_pitch")
        else {
            return;
        };
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
