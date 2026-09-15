use super::upload;
use super::*;

fn header(bytes: &[u8]) -> Result<[u32; 16]> {
    ensure!(bytes.len() >= 64, "short lens header");
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }))
}

fn validate(command: &Command, bytes: &[u8], width: u32, height: u32) -> Result<[u32; 16]> {
    let h = header(bytes)?;
    ensure!(
        command.abi_version == ABI_VERSION
            && command.kind == LENS_EFFECT
            && command.reserved == [0; 3],
        "invalid lens command"
    );
    ensure!(
        command.x == 0
            && command.y == 0
            && command.width == width
            && command.height == height
            && command.clip_x == 0
            && command.clip_y == 0
            && command.clip_width == width
            && command.clip_height == height
            && command.blend == 0
            && command.table == 0,
        "lens must cover target"
    );
    ensure!(
        h[0] <= 2
            && h[1] == width
            && h[2] == height
            && h[3] >= width
            && h[4] >= width
            && h[3] <= 1048576
            && h[4] <= 1048576
            && h[6] <= 1
            && h[9] <= 256
            && h[11] == 64,
        "invalid lens parameters"
    );
    let source_length = (height as u64 - 1) * h[3] as u64 + width as u64;
    let destination_length = (height as u64 - 1) * h[4] as u64 + width as u64;
    ensure!(
        source_length <= 32 * 1024 * 1024
            && destination_length <= 32 * 1024 * 1024
            && h[12] as u64 == 64 + source_length,
        "invalid lens source extent"
    );
    ensure!(
        h[6] == 0 || (h[5] as i32 as i64).unsigned_abs() <= 32 * 1024 * 1024,
        "invalid alias offset"
    );
    let asset_length = match h[0] {
        0 => width as u64 * height as u64 * 4,
        1 => 65536,
        _ => {
            ensure!(
                h[14] > 0 && h[14] <= 8192 && h[15] > 0 && h[15] <= 8192,
                "invalid overlay extent"
            );
            h[14] as u64 * h[15] as u64
        }
    };
    ensure!(
        h[13] as u64 == h[12] as u64 + asset_length
            && bytes.len() as u64 == h[13] as u64 + if h[0] == 1 { 33 * 256 } else { 0 },
        "invalid lens asset extent"
    );
    let sw = if h[0] == 2 { h[14] } else { 640 };
    let sh = if h[0] == 2 { h[15] } else { 480 };
    ensure!(
        h[7] == (sw << 16) / width && h[8] == (sh << 16) / height,
        "invalid lens scaling"
    );
    if h[0] == 0 {
        for pair in bytes[h[12] as usize..].as_chunks::<4>().0 {
            let x = i16::from_le_bytes(pair[..2].try_into().unwrap());
            let y = i16::from_le_bytes(pair[2..].try_into().unwrap());
            ensure!(
                x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height,
                "invalid signed lens lookup"
            );
        }
    }
    Ok(h)
}

impl DrawRenderer {
    pub(super) fn submit_effect(&mut self, target_id: u64, command: &Command) -> Result<()> {
        self.check_status()?;
        let target = self
            .targets
            .get(&target_id)
            .context("unknown lens target")?;
        let source = self
            .resources
            .get(&command.source)
            .context("unknown lens source")?;
        let h = validate(command, &source.bytes, target.width, target.height)?;
        let dispatch = if h[6] != 0 {
            [1, 1]
        } else {
            [target.width.div_ceil(8), target.height.div_ceil(8)]
        };
        ensure!(
            dispatch
                .into_iter()
                .all(|n| n <= self.device.limits().max_compute_workgroups_per_dimension),
            "lens dispatch exceeds device limit"
        );
        if self.effects.is_none() {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("ordered native lens"),
                    source: wgpu::ShaderSource::Wgsl(assets::shader(include_str!("draw_effects.wgsl")).into()),
                });
            let pipeline = self
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("ordered native lens"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("effect"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            self.effects = Some(pipeline);
        }
        let limit = self.storage_limit() as usize;
        self.arena_headroom(0)?;
        let bytes = &self.resources[&command.source].bytes;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let base = packer.offset(command.source, bytes, ResourceKind::Lens)?;
        let words = packer.finish();
        let assets = match &words {
            Some(words) => byte_buffer(
                &self.device,
                &mut self.counters,
                "immutable lens sources and maps",
                words,
                wgpu::BufferUsages::STORAGE,
            ),
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        let target = &self.targets[&target_id];
        let view = upload::stage(
            &self.uploads,
            &self.device,
            &mut self.counters,
            "effect target view",
            &[target.width, target.pitch, target.offset, base],
            wgpu::BufferUsages::UNIFORM,
        );
        let pipeline = self.effects.clone().unwrap();
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("native lens"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[entry(0, &target.indices), entry(1, &assets), view.entry(2)],
        });
        let stamp = self.stamp(PASS_LENS);
        {
            let encoder = self.frame_encoder();
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("native lens pixel production"),
                timestamp_writes: stamp.compute(),
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(dispatch[0], dispatch[1], 1);
        }
        self.counters.dispatches += 1;
        self.pass_boundary();
        self.check_status()?;
        self.counters.batches += 1;
        self.counters.commands += 1;
        if let Some(words) = &words {
            self.counters.asset_upload_bytes += words.len() as u64 * assets::STRIDE as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_lens_resources_reject_before_submission() {
        let h: [u32; 16] = [
            0,
            2,
            2,
            2,
            2,
            0,
            0,
            640 << 15,
            480 << 15,
            0,
            0,
            64,
            68,
            84,
            0,
            0,
        ];
        let mut bytes: Vec<u8> = h.into_iter().flat_map(u32::to_le_bytes).collect();
        bytes.resize(84, 0);
        let command = Command {
            kind: LENS_EFFECT,
            width: 2,
            height: 2,
            clip_width: 2,
            clip_height: 2,
            ..Command::default()
        };
        validate(&command, &bytes, 2, 2).unwrap();
        bytes[68] = 255;
        bytes[69] = 255;
        assert!(validate(&command, &bytes, 2, 2).is_err());
        bytes[68] = 2;
        bytes[69] = 0;
        assert!(validate(&command, &bytes, 2, 2).is_err());
        bytes[68] = 0;
        assert!(validate(&command, &bytes[..83], 2, 2).is_err());
        assert!(
            validate(
                &Command {
                    clip_x: -1,
                    ..command
                },
                &bytes,
                2,
                2
            )
            .is_err()
        );
        bytes[24] = 2;
        assert!(validate(&command, &bytes, 2, 2).is_err());
    }
}
