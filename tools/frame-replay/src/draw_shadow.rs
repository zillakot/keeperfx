use super::*;

pub const SHADOW: u32 = 11;

const HEADER: usize = 32;
const GEOMETRY: usize = 120;
const MASK: usize = 65536;
const RLE: usize = HEADER + GEOMETRY + MASK;

fn descriptor(source: &Resource) -> Result<[u32; 8]> {
    ensure!(source.bytes.len() >= RLE, "truncated shadow resource");
    let mut d = [0; 8];
    for (i, b) in source.bytes[..HEADER].as_chunks::<4>().0.iter().enumerate() {
        d[i] = u32::from_le_bytes(*b);
    }
    ensure!(
        d[0] > 0
            && d[0] <= 256
            && d[1] > 0
            && d[1] <= 256
            && d[2] > 0
            && d[2] <= 256
            && d[3] > 0
            && d[3] <= 256
            && d[6] <= 1,
        "invalid shadow dimensions/options"
    );
    ensure!(
        u64::from(d[5]) + u64::from(d[3]) <= 256 && d[4] <= 256,
        "shadow sprite exceeds mask"
    );
    ensure!(
        if d[6] == 0 {
            u64::from(d[4]) + u64::from(d[2]) <= 256
        } else {
            d[4] + 1 >= d[2]
        },
        "shadow sprite horizontal extent exceeds mask"
    );
    ensure!(
        256 * (d[5] + d[3] - 1) + d[4] + if d[6] == 0 { d[2] - 1 } else { 0 } < 65536,
        "shadow sprite allocation exceeded"
    );
    ensure!(
        source.bytes.len() == RLE + d[7] as usize,
        "invalid shadow RLE length"
    );
    let bytes = &source.bytes[RLE..];
    let mut offset = 0;
    for _ in 0..d[3] {
        let mut x = 0;
        loop {
            let run = *bytes.get(offset).context("unterminated shadow RLE row")? as i8;
            offset += 1;
            if run == 0 {
                break;
            }
            let n = i32::from(run).unsigned_abs();
            ensure!(x + n <= d[2], "shadow RLE run exceeds sprite width");
            x += n;
            if run > 0 {
                offset += n as usize;
                ensure!(offset <= bytes.len(), "truncated shadow RLE run");
            }
        }
    }
    ensure!(offset == bytes.len(), "shadow RLE has trailing data");
    Ok(d)
}

impl DrawRenderer {
    /// Source packs eight LE u32 descriptor fields, two triangle geometries, a prior scratch checkpoint, and immutable RLE artwork.
    pub fn create_shadow_mask(&mut self, source: u64) -> Result<u64> {
        self.check_status()?;
        let asset = self
            .resources
            .get(&source)
            .context("unknown shadow resource")?;
        descriptor(asset)?;
        ensure!(
            asset.bytes.len() <= self.storage_limit() as usize / 4,
            "shadow asset exceeds GPU limit"
        );
        ensure!(
            self.device.limits().max_compute_workgroups_per_dimension >= 32,
            "shadow dispatch exceeds GPU limit"
        );
        let values: Vec<_> = asset.bytes.iter().map(|&b| u32::from(b)).collect();
        if self.shadow.is_none() {
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("native creature shadow mask"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("draw_shadow.wgsl").into()),
                });
            self.shadow = Some(self.device.create_compute_pipeline(
                &wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &module,
                    entry_point: Some("shadow_mask"),
                    compilation_options: Default::default(),
                    cache: None,
                },
            ));
        }
        let target = self.create_target(256, 256)?;
        let input = buffer(
            &self.device,
            &mut self.counters,
            "immutable shadow artwork and prior scratch",
            &values,
            wgpu::BufferUsages::STORAGE,
        );
        let pipeline = self.shadow.as_ref().unwrap();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[entry(0, &self.targets[&target].indices), entry(1, &input)],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(32, 32, 1);
        }
        self.submit_encoder(encoder);
        self.counters.asset_upload_bytes += values.len() as u64 * 4;
        self.counters.commands += 1;
        self.counters.batches += 1;
        if let Err(error) = self.check_status() {
            self.release_target(target)?;
            return Err(error);
        }
        Ok(target)
    }

    pub fn submit_shadow(
        &mut self,
        target: u64,
        command: &Command,
        mirror: &mut [u8],
    ) -> Result<()> {
        self.check_status()?;
        ensure!(
            command.abi_version == ABI_VERSION
                && command.kind == SHADOW
                && command.reserved == [0; 3]
                && command.colour < 64
                && mirror.len() == MASK,
            "invalid native shadow command"
        );
        let asset = self
            .resources
            .get(&command.source)
            .context("unknown shadow resource")?;
        descriptor(asset)?;
        let geometry = [
            asset.bytes[HEADER..HEADER + 60].to_vec(),
            asset.bytes[HEADER + 60..HEADER + GEOMETRY].to_vec(),
        ];
        let mut resources = Vec::new();
        let mut mask = 0;
        let mut snapshot = 0;
        let result = (|| {
            let mut commands = Vec::new();
            for bytes in &geometry {
                let source = self.create_resource(bytes, 1, 1, 1)?;
                resources.push(source);
                commands.push(Command {
                    kind: TRIG,
                    source,
                    source_x: 10,
                    source_y: 65536,
                    source_width: 64,
                    ..*command
                });
            }
            mask = self.create_shadow_mask(command.source)?;
            snapshot = self.create_target_snapshot(mask, 0, 0, 256, 256, 256)?;
            self.submit_target_triangles(target, &commands, snapshot)?;
            mirror.copy_from_slice(&self.readback(mask)?);
            Ok(())
        })();
        for resource in resources {
            self.release_resource(resource)?;
        }
        if snapshot != 0 {
            self.release_target_snapshot(snapshot)?;
        }
        if mask != 0 {
            self.release_target(mask)?;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shadow_asset_validation() {
        let mut bytes = vec![0; RLE];
        for (i, n) in [4u32, 2, 3, 2, 3, 0, 1, 8].iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&n.to_le_bytes());
        }
        bytes.extend([1, 255, 254, 0, 253, 0, 0, 0]);
        let mut r = Resource {
            width: 1,
            height: 1,
            pitch: 1,
            bytes,
        };
        assert!(descriptor(&r).is_err());
        r.bytes.truncate(RLE + 6);
        r.bytes[28..32].copy_from_slice(&6u32.to_le_bytes());
        assert!(descriptor(&r).is_ok());
        r.bytes[RLE] = 4;
        assert!(descriptor(&r).is_err());
    }
}
