use super::*;

pub const SHADOW: u32 = 11;

const HEADER: usize = 32;
const GEOMETRY: usize = 120;
const RLE: usize = HEADER + GEOMETRY;
pub(super) const MASK_WORDS: usize = 65536;
pub(super) const SLOTS: u32 = 2;

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
    pub(super) fn shadow_slot_binding(&self) -> &wgpu::Buffer {
        self.shadow_slots
            .as_ref()
            .unwrap_or(&self.shadow_placeholder)
    }

    /// Allocates the cross-frame scratch and the mask slot ring on first shadow use.
    pub(super) fn shadow_residency(&mut self) -> Result<()> {
        if self.shadow_slots.is_some() {
            return Ok(());
        }
        let scratch = MASK_WORDS as u64 * 4;
        ensure!(
            scratch * u64::from(SLOTS) <= self.storage_limit(),
            "resident shadow slots exceed the device buffer limit"
        );
        self.shadow_scratch = Some(self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("resident creature shadow scratch"),
            size: scratch,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.shadow_slots = Some(self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("resident creature shadow mask slots"),
            size: scratch * u64::from(SLOTS),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        }));
        Ok(())
    }

    pub(super) fn shadow_pipeline(&mut self) -> Result<&wgpu::ComputePipeline> {
        ensure!(
            self.device.limits().max_compute_workgroups_per_dimension >= 32,
            "shadow dispatch exceeds GPU limit"
        );
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
        Ok(self.shadow.as_ref().unwrap())
    }

    /// Records the mask into the resident scratch and the given slot; the caller submits.
    pub(super) fn record_shadow_mask(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: u64,
        slot: u32,
    ) -> Result<()> {
        let asset = self
            .resources
            .get(&source)
            .context("unknown shadow resource")?;
        descriptor(asset)?;
        ensure!(
            asset.bytes.len() <= self.storage_limit() as usize / 4,
            "shadow asset exceeds GPU limit"
        );
        ensure!(slot < SLOTS, "shadow mask slot exceeds the resident ring");
        self.shadow_residency()?;
        self.shadow_pipeline()?;
        let limit = self.storage_limit() as usize;
        let bytes = &self.resources[&source].bytes;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let base = packer.offset(source, bytes)?;
        let values = packer.finish();
        let input = match &values {
            Some(values) => buffer(
                &self.device,
                &mut self.counters,
                "immutable shadow artwork",
                values,
                wgpu::BufferUsages::STORAGE,
            ),
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        let region = buffer(
            &self.device,
            &mut self.counters,
            "shadow arena region",
            &[base, 0, 0, 0],
            wgpu::BufferUsages::UNIFORM,
        );
        let pipeline = self.shadow.as_ref().unwrap();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, self.shadow_scratch.as_ref().unwrap()),
                entry(1, &input),
                entry(2, &region),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: self.shadow_slots.as_ref().unwrap(),
                        offset: u64::from(slot) * MASK_WORDS as u64 * 4,
                        size: std::num::NonZeroU64::new(MASK_WORDS as u64 * 4),
                    }),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(32, 32, 1);
        }
        self.counters.dispatches += 1;
        if let Some(values) = &values {
            self.counters.asset_upload_bytes += values.len() as u64 * 4;
        }
        self.counters.commands += 1;
        Ok(())
    }

    /// Clears the cross-frame shadow scratch; required after a full CPU redraw or device loss.
    pub fn shadow_scratch_reset(&mut self) -> Result<()> {
        self.check_status()?;
        let Some(scratch) = self.shadow_scratch.clone() else {
            return Ok(());
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&scratch, 0, None);
        self.submit_encoder(encoder);
        self.check_status()
    }

    /// Blocking read of the resident scratch; verification and recovery only.
    pub fn shadow_scratch_read(&mut self) -> Result<Vec<u8>> {
        self.check_status()?;
        let Some(scratch) = self.shadow_scratch.clone() else {
            return Ok(vec![0; MASK_WORDS]);
        };
        let size = MASK_WORDS as u64 * 4;
        let staging = self.tracked_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow scratch verification"),
            size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&scratch, 0, &staging, 0, size);
        self.submit_encoder(encoder);
        let (sender, receiver) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        self.wait_for_queue()?;
        receiver.recv()??;
        let mapped = staging.slice(..).get_mapped_range()?;
        let values: Vec<_> = mapped.as_chunks::<4>().0.iter().map(|w| w[0]).collect();
        drop(mapped);
        staging.unmap();
        self.counters.readback_bytes += size;
        self.check_status()?;
        Ok(values)
    }

    pub fn submit_shadow(&mut self, target: u64, command: &Command) -> Result<()> {
        self.check_status()?;
        ensure!(
            command.abi_version == ABI_VERSION
                && command.kind == SHADOW
                && command.reserved == [0; 3]
                && command.colour < 64,
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
        let slot = self.shadow_next_slot;
        let mut resources = Vec::new();
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
            if self.enqueue_shadow(target, command.source, slot, &commands)? {
                return Ok(());
            }
            self.submit_shadow_batch(target, command.source, slot, &commands)
        })();
        if result.is_ok() {
            self.shadow_next_slot = (slot + 1) % SLOTS;
        }
        for resource in resources {
            self.release_resource(resource)?;
        }
        result
    }

    /// Submits the mask immediately ahead of the triangles that sample its slot.
    pub(super) fn submit_shadow_batch(
        &mut self,
        target: u64,
        source: u64,
        slot: u32,
        commands: &[Command],
    ) -> Result<()> {
        self.submit_target_triangles(target, commands, slot, Some(source))
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
