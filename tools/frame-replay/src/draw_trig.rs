use super::*;

pub(super) fn validate(c: &Command, source: &Resource, width: u32, height: u32) -> Result<()> {
    ensure!(
        matches!(c.source_x, 0..=26),
        "unsupported general triangle mode"
    );
    ensure!(
        c.x == 0
            && c.y == 0
            && c.width == width
            && c.height == height
            && c.blend == 0
            && c.transparent == OPAQUE,
        "invalid general triangle bounds/options"
    );
    ensure!(
        source.bytes.len() == 60 + c.source_y as usize && c.source_y <= 65536,
        "invalid triangle vertex/texture resource"
    );
    ensure!(
        c.source_width == 64,
        "triangle requires native LP64 arithmetic"
    );
    let textured = matches!(
        c.source_x,
        2 | 3 | 5..=13 | 18..=26
    );
    ensure!(
        textured == (c.source_y != 0),
        "triangle texture presence mismatch"
    );
    ensure!(
        !matches!(c.source_x, 7 | 8 | 10 | 11) || c.colour < 64,
        "triangle constant shade exceeds fade table"
    );
    let mut min = [i32::MAX; 2];
    let mut max = [i32::MIN; 2];
    for vertex in source.bytes[..60].as_chunks::<20>().0 {
        for (i, bytes) in vertex.as_chunks::<4>().0.iter().enumerate() {
            let n = i32::from_le_bytes(*bytes);
            if i < 2 {
                ensure!(
                    (-32767..=32767).contains(&n),
                    "triangle coordinate exceeds native domain"
                );
                min[i] = min[i].min(n);
                max[i] = max[i].max(n);
            } else {
                ensure!(
                    (-0x0400_0000..=0x0400_0000).contains(&n),
                    "triangle attribute exceeds GPU domain"
                );
            }
        }
    }
    ensure!(
        (0..2).all(|i| max[i] - min[i] <= 32767),
        "triangle extent exceeds native domain"
    );
    Ok(())
}

impl DrawRenderer {
    pub(super) fn preflight_ordered_commands(
        &mut self,
        target: u64,
        commands: &[Command],
    ) -> Result<()> {
        let (width, height) = self.target_dimensions(target)?;
        let dispatch_limit = self.device.limits().max_compute_workgroups_per_dimension;
        ensure!(
            width.div_ceil(8) <= dispatch_limit && height.div_ceil(8) <= dispatch_limit,
            "drawing dispatch exceeds device limit"
        );
        let (words, assets) = pack_commands(
            commands,
            &self.resources,
            width,
            height,
            self.storage_limit() as usize,
        )?;
        bin_commands(&words, width, height, self.storage_limit() as usize)?;
        if commands.iter().any(|c| c.kind == TRIG) {
            self.prepare_trig();
            let command_buffer = buffer(
                &self.device,
                "mixed batch preflight commands",
                &words,
                wgpu::BufferUsages::STORAGE,
            );
            let asset_buffer = buffer(
                &self.device,
                "mixed batch preflight assets",
                &assets,
                wgpu::BufferUsages::STORAGE,
            );
            let parameters = buffer(
                &self.device,
                "mixed batch preflight parameters",
                &[
                    width,
                    height,
                    commands.len() as u32,
                    0,
                    self.targets[&target].pitch,
                    self.targets[&target].offset,
                    0,
                    0,
                ],
                wgpu::BufferUsages::UNIFORM,
            );
            self.counters.command_upload_bytes += (words.len() as u64 * 4) + 20;
            self.counters.asset_upload_bytes += assets.len() as u64 * 4;
            let valid = self.validate_trig_batch(
                &command_buffer,
                &asset_buffer,
                &parameters,
                width,
                height,
            )?;
            if self.deferred_status.is_none() {
                self.counters.readback_bytes += 4;
            }
            ensure!(valid, "triangle has an invalid lookup");
        }
        Ok(())
    }

    pub(super) fn prepare_trig(&mut self) {
        if self.trig_validate.is_some() {
            return;
        }
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("general triangle validation"),
                source: wgpu::ShaderSource::Wgsl(super::DRAW_SHADER.into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &shader,
                entry_point: Some("validate_trig"),
                compilation_options: Default::default(),
                cache: None,
            });
        self.trig_validate = Some(pipeline);
    }

    pub(super) fn validate_trig_batch(
        &mut self,
        commands: &wgpu::Buffer,
        assets: &wgpu::Buffer,
        params: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) -> Result<bool> {
        let pipeline = self.trig_validate.as_ref().unwrap();
        let status = buffer(
            &self.device,
            "triangle status",
            &[0],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = self.deferred_status.is_none().then(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 4,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            })
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(1, commands),
                entry(2, assets),
                entry(3, params),
                entry(5, &status),
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        if let Some(staging) = &readback {
            encoder.copy_buffer_to_buffer(&status, 0, staging, 0, 4);
        }
        self.queue.submit([encoder.finish()]);
        if let Some(statuses) = &mut self.deferred_status {
            statuses.push(status);
            return Ok(true);
        }
        let readback = readback.unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        receiver.recv()??;
        let mapped = readback.slice(..).get_mapped_range()?;
        let valid = mapped.iter().all(|&b| b == 0);
        drop(mapped);
        readback.unmap();
        self.check_status()?;
        Ok(valid)
    }
}
