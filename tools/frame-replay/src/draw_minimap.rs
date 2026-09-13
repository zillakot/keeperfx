use super::*;
pub const MINIMAP: u32 = 12;
const HEADER: usize = 96;
pub(super) struct MinimapState {
    pipeline: wgpu::ComputePipeline,
    pub(super) background: Option<(u64, u32)>,
}

fn validate(c: &Command, b: &[u8], width: u32, height: u32) -> Result<[u32; 24]> {
    ensure!(b.len() >= HEADER, "short minimap header");
    let h: [u32; 24] =
        std::array::from_fn(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()));
    ensure!(
        c.abi_version == ABI_VERSION
            && c.kind == MINIMAP
            && c.reserved == [0; 3]
            && c.table == 0
            && c.blend == 0,
        "invalid minimap command"
    );
    ensure!(
        h[0] <= 4
            && h[1] == width
            && h[2] == height
            && h[5] > 0
            && h[5] <= 2048
            && h[5].is_multiple_of(2)
            && u64::from(h[3]) + u64::from(h[5]) <= u64::from(width)
            && u64::from(h[4]) + u64::from(h[5]) <= u64::from(height),
        "invalid minimap extent"
    );
    ensure!(
        h[20] <= 255 && h[23] <= 36 && h[22] == HEADER as u32,
        "invalid minimap pattern"
    );
    let pattern_end = HEADER + h[23] as usize * 8;
    ensure!(
        b.len() >= pattern_end && h[18] <= h[23] || h[0] == 2 || h[0] == 0,
        "invalid minimap pattern length"
    );
    if h[0] == 0 {
        ensure!(
            h[10] > 0
                && h[10] <= 2048
                && h[11] > 0
                && h[11] <= 2048
                && h[12] as usize == pattern_end
                && h[13] == h[12] + 256
                && h[15] > 0
                && h[15] <= 16 * 38569,
            "invalid minimap world descriptor"
        );
        let cell_end = u64::from(h[13]) + (u64::from(h[10]) + 1) * (u64::from(h[11]) + 1) * 2;
        ensure!(
            u64::from(h[14]) == cell_end && b.len() as u64 == cell_end + u64::from(h[15]),
            "invalid minimap world extent"
        );
        ensure!(h[15].is_multiple_of(38569), "invalid minimap palette size");
        for cell in b[h[13] as usize..h[14] as usize].as_chunks::<2>().0 {
            ensure!(u16::from_le_bytes(*cell) < 38569, "invalid minimap cell");
        }
        let n = h[15] / 38569;
        ensure!(n <= 16, "too many minimap backgrounds");
        for j in 0..n {
            ensure!(
                !b[h[12] as usize..h[12] as usize + j as usize]
                    .contains(&b[h[12] as usize + j as usize]),
                "duplicate minimap background"
            );
        }
    } else {
        ensure!(b.len() == pattern_end, "unexpected minimap data");
    }
    if h[0] == 1 {
        ensure!(
            (h[19] as i32).unsigned_abs() <= 8192,
            "invalid minimap spread"
        );
    }
    if h[0] == 2 {
        ensure!(h[18] <= 8192 && h[21] <= 4096, "invalid minimap circle");
    }
    if h[0] == 3 {
        ensure!(h[21] <= 65536, "invalid minimap heart distance");
    }
    Ok(h)
}
impl DrawRenderer {
    pub(super) fn submit_minimap(&mut self, target_id: u64, c: &Command) -> Result<()> {
        self.check_status()?;
        let (width, height) = self.target_dimensions(target_id)?;
        let source = self
            .resources
            .get(&c.source)
            .context("unknown minimap source")?;
        let h = validate(c, &source.bytes, width, height)?;
        ensure!(
            source.bytes.len() as u64 * 4 <= self.storage_limit(),
            "minimap source exceeds GPU storage"
        );
        ensure!(
            h[5].div_ceil(8) <= self.device.limits().max_compute_workgroups_per_dimension,
            "minimap dispatch exceeds GPU limit"
        );
        let words: Vec<u32> = source.bytes.iter().map(|&b| u32::from(b)).collect();
        if self.minimap.is_none() {
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("native minimap"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("draw_minimap.wgsl").into()),
                });
            self.minimap = Some(MinimapState {
                pipeline: self
                    .device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: None,
                        layout: None,
                        module: &module,
                        entry_point: Some("minimap"),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                background: None,
            });
        }
        if h[0] == 0 {
            ensure!(
                self.minimap
                    .as_ref()
                    .unwrap()
                    .background
                    .is_some_and(|(_, d)| d == h[5]),
                "minimap background snapshot missing"
            );
        }
        if h[0] == 4 {
            let snapshot = self.create_target_snapshot(target_id, h[3], h[4], h[5], h[5], h[5])?;
            if let Some((old, _)) = self
                .minimap
                .as_mut()
                .unwrap()
                .background
                .replace((snapshot, h[5]))
            {
                self.release_target_snapshot(old)?;
            }
        }
        let assets = buffer(
            &self.device,
            &mut self.counters,
            "minimap semantic cells and styles",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let dummy = buffer(
            &self.device,
            &mut self.counters,
            "unused minimap background",
            &[0],
            wgpu::BufferUsages::STORAGE,
        );
        let state = self.minimap.as_ref().unwrap();
        let background = if h[0] == 0 {
            &self.target_snapshots[&state.background.unwrap().0].indices
        } else {
            &dummy
        };
        let target = &self.targets[&target_id];
        let view = buffer(
            &self.device,
            &mut self.counters,
            "target view",
            &[target.width, target.pitch, target.offset, 0],
            wgpu::BufferUsages::UNIFORM,
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &state.pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, &self.targets[&target_id].indices),
                entry(1, &assets),
                entry(2, background),
                entry(3, &view),
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(h[5].div_ceil(8), h[5].div_ceil(8), 1);
        }
        self.submit_encoder(encoder);
        self.counters.batches += 1;
        self.counters.commands += 1;
        self.counters.asset_upload_bytes += words.len() as u64 * 4;
        self.check_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_semantic_resources() {
        let mut h = [0u32; 24];
        h[0] = 1;
        h[1] = 64;
        h[2] = 64;
        h[5] = 32;
        h[18] = 1;
        h[22] = 96;
        h[23] = 1;
        let command = Command {
            kind: MINIMAP,
            width: 64,
            height: 64,
            clip_width: 64,
            clip_height: 64,
            ..Default::default()
        };
        let encode = |h: [u32; 24]| {
            let mut b: Vec<u8> = h.into_iter().flat_map(u32::to_le_bytes).collect();
            b.resize(104, 0);
            b
        };
        assert!(validate(&command, &encode(h), 64, 64).is_ok());
        for (field, value) in [
            (0, 5),
            (5, 33),
            (3, u32::MAX),
            (18, 2),
            (20, 256),
            (22, 0),
            (23, 37),
        ] {
            let mut invalid = h;
            invalid[field] = value;
            assert!(
                validate(&command, &encode(invalid), 64, 64).is_err(),
                "field {field}"
            );
        }
        h[0] = 2;
        h[18] = 9000;
        assert!(validate(&command, &encode(h), 64, 64).is_err());
        assert!(validate(&command, &[0; 12], 64, 64).is_err());
    }
}
