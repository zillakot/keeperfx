use super::*;
pub const MINIMAP: u32 = 12;
const HEADER: usize = 96;
pub(super) struct MinimapState {
    pipeline: wgpu::ComputePipeline,
    cache: SegmentCache,
    pub(super) background: Option<(u64, u32)>,
}

const CACHE_BYTES: u64 = 9 << 20;

#[derive(Default)]
struct SegmentCache {
    entries: Vec<Segment>,
    clock: u64,
}

struct Segment {
    id: u64,
    role: usize,
    layout: [u32; 2],
    hash: u64,
    bytes: Vec<u8>,
    used: u64,
}

fn class_bytes(length: usize) -> u64 {
    (length.max(256) as u64).next_power_of_two() * 4
}

fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    hash.write(bytes);
    hash.finish()
}

impl SegmentCache {
    fn class_bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| class_bytes(e.bytes.len()))
            .sum()
    }

    fn cpu_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.bytes.len() as u64).sum()
    }

    fn resolve(
        &mut self,
        arena: &mut arena::Arena,
        role: usize,
        layout: [u32; 2],
        bytes: &[u8],
        hash: u64,
    ) -> Result<(u64, bool)> {
        self.clock += 1;
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.role == role && e.layout == layout && e.hash == hash && e.bytes == bytes)
        {
            entry.used = self.clock;
            return Ok((entry.id, true));
        }
        let id = next_handle()?;
        while let Some(index) = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.role == role)
            .min_by_key(|(_, e)| e.used)
            .map(|(i, _)| i)
            .filter(|_| role != 2 || self.entries.iter().filter(|e| e.role == 2).count() >= 4)
        {
            arena.release(self.entries.remove(index).id);
        }
        while self.class_bytes() + class_bytes(bytes.len()) > CACHE_BYTES {
            let index = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.role == 2)
                .min_by_key(|(_, e)| e.used)
                .map(|(i, _)| i)
                .context("minimap cache admission exceeds budget")?;
            arena.release(self.entries.remove(index).id);
        }
        self.entries.push(Segment {
            id,
            role,
            layout,
            hash,
            bytes: bytes.to_vec(),
            used: self.clock,
        });
        Ok((id, false))
    }
}

struct Validated {
    header: [u32; 24],
    ranges: [std::ops::Range<usize>; 4],
}

fn validate(c: &Command, b: &[u8], width: u32, height: u32) -> Result<Validated> {
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
    let ranges = if h[0] == 0 {
        [
            0..pattern_end,
            h[12] as usize..h[13] as usize,
            h[13] as usize..h[14] as usize,
            h[14] as usize..b.len(),
        ]
    } else {
        [0..pattern_end, 0..0, 0..0, 0..0]
    };
    Ok(Validated { header: h, ranges })
}
impl DrawRenderer {
    pub(super) fn submit_minimap(&mut self, target_id: u64, c: &Command) -> Result<()> {
        self.check_status()?;
        let (width, height) = self.target_dimensions(target_id)?;
        let source = self
            .resources
            .get(&c.source)
            .context("unknown minimap source")?;
        let validated = validate(c, &source.bytes, width, height)?;
        let h = validated.header;
        ensure!(
            source.bytes.len() as u64 * 4 <= self.storage_limit(),
            "minimap source exceeds GPU storage"
        );
        ensure!(
            h[5].div_ceil(8) <= self.device.limits().max_compute_workgroups_per_dimension,
            "minimap dispatch exceeds GPU limit"
        );
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
                cache: SegmentCache::default(),
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
        let limit = self.storage_limit() as usize;
        let ranges = validated.ranges;
        let split = self.arena.enabled()
            && (h[0] != 0
                || ranges[1..]
                    .iter()
                    .map(|r| class_bytes(r.len()))
                    .sum::<u64>()
                    <= CACHE_BYTES);
        let mut ids = [0; 4];
        if split {
            ids[0] = next_handle()?;
            if h[0] == 0 {
                let cache = &mut self.minimap.as_mut().unwrap().cache;
                let bytes = &self.resources[&c.source].bytes;
                for role in 0..2 {
                    let layout = if role == 1 {
                        [h[10], h[11]]
                    } else {
                        [h[15] / 38569, 0]
                    };
                    let segment = &bytes[ranges[role + 1].clone()];
                    if let Some(index) = cache
                        .entries
                        .iter()
                        .position(|e| e.role == role && (e.layout != layout || e.bytes != segment))
                    {
                        self.arena.release(cache.entries.remove(index).id);
                    }
                }
                for role in 0..3 {
                    let layout = if role == 1 {
                        [h[10], h[11]]
                    } else {
                        [h[15] / 38569, 0]
                    };
                    let segment = &bytes[ranges[role + 1].clone()];
                    let (id, hit) = cache.resolve(
                        &mut self.arena,
                        role,
                        layout,
                        segment,
                        content_hash(segment),
                    )?;
                    ids[role + 1] = id;
                    let counter = match (role, hit) {
                        (0, true) => &mut self.counters.minimap_dictionary_hits,
                        (0, false) => &mut self.counters.minimap_dictionary_misses,
                        (1, true) => &mut self.counters.minimap_cells_hits,
                        (1, false) => &mut self.counters.minimap_cells_misses,
                        (2, true) => &mut self.counters.minimap_styles_hits,
                        _ => &mut self.counters.minimap_styles_misses,
                    };
                    *counter += 1;
                }
                self.counters.minimap_cache_class_bytes = cache.class_bytes();
                self.counters.minimap_cache_cpu_bytes = cache.cpu_bytes();
            }
        }
        let demand = if split {
            ids.iter()
                .zip(&ranges)
                .filter(|(id, _)| **id != 0)
                .try_fold(0, |sum, (id, range)| {
                    self.arena
                        .allocation_words(*id, range.len())
                        .map(|n| sum + n)
                })?
        } else {
            self.arena
                .allocation_words(c.source, self.resources[&c.source].bytes.len())?
        };
        self.arena_headroom(demand)?;
        let bytes = &self.resources[&c.source].bytes;
        let enabled = self.arena.enabled();
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let mut bases = [0; 4];
        let mut uploaded = [0; 4];
        if split {
            for i in 0..4 {
                if ids[i] == 0 {
                    continue;
                }
                let segment = if i == 0 {
                    &bytes[ranges[0].clone()]
                } else {
                    &self
                        .minimap
                        .as_ref()
                        .unwrap()
                        .cache
                        .entries
                        .iter()
                        .find(|e| e.id == ids[i])
                        .unwrap()
                        .bytes
                };
                let before = packer.uploaded_bytes();
                bases[i] = packer.offset(ids[i], segment, ResourceKind::Minimap)?;
                uploaded[i] = packer.uploaded_bytes() - before;
            }
        } else {
            let before = packer.uploaded_bytes();
            bases[0] = packer.offset(c.source, bytes, ResourceKind::Minimap)?;
            for i in 1..4 {
                bases[i] = bases[0] + h[11 + i];
            }
            if enabled && packer.uploaded_bytes() != before {
                uploaded = std::array::from_fn(|i| ranges[i].len() as u64 * 4);
            }
        }
        let words = packer.finish();
        self.counters.arena_minimap_prefix_bytes += uploaded[0];
        self.counters.arena_minimap_dictionary_bytes += uploaded[1];
        self.counters.arena_minimap_cells_bytes += uploaded[2];
        self.counters.arena_minimap_styles_bytes += uploaded[3];
        let assets = match &words {
            Some(words) => buffer(
                &self.device,
                &mut self.counters,
                "minimap semantic cells and styles",
                words,
                wgpu::BufferUsages::STORAGE,
            ),
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        let dummy = buffer(
            &self.device,
            &mut self.counters,
            "unused minimap background",
            &[0],
            wgpu::BufferUsages::STORAGE,
        );
        let state = self.minimap.as_ref().unwrap();
        let pipeline = state.pipeline.clone();
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
            &[
                target.width,
                target.pitch,
                target.offset,
                bases[0],
                bases[1],
                bases[2],
                bases[3],
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, &self.targets[&target_id].indices),
                entry(1, &assets),
                entry(2, background),
                entry(3, &view),
            ],
        });
        let stamp = self.stamp(PASS_MINIMAP);
        {
            let encoder = self.frame_encoder();
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("minimap production"),
                timestamp_writes: stamp.compute(),
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(h[5].div_ceil(8), h[5].div_ceil(8), 1);
        }
        if split {
            self.arena.release(ids[0]);
        }
        self.counters.dispatches += 1;
        self.pass_boundary();
        self.counters.batches += 1;
        self.counters.commands += 1;
        if let Some(words) = &words {
            self.counters.asset_upload_bytes += words.len() as u64 * 4;
        }
        self.check_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_identity_layout_and_style_lru() {
        let mut arena = arena::Arena::new(32 << 20);
        let mut cache = SegmentCache::default();
        let first = cache
            .resolve(&mut arena, 2, [11, 0], &vec![1; 424259], 7)
            .unwrap();
        assert!(!first.1);
        let second = cache
            .resolve(&mut arena, 2, [11, 0], &vec![2; 424259], 7)
            .unwrap();
        assert_ne!(first.0, second.0);
        assert_eq!(
            cache
                .resolve(&mut arena, 2, [11, 0], &vec![1; 424259], 7)
                .unwrap(),
            (first.0, true)
        );
        for phase in [3, 4] {
            cache
                .resolve(&mut arena, 2, [11, 0], &vec![phase; 424259], 7)
                .unwrap();
        }
        cache
            .resolve(&mut arena, 1, [255, 255], &vec![0; 131072], 7)
            .unwrap();
        cache.resolve(&mut arena, 0, [11, 0], &[0; 256], 7).unwrap();
        assert_eq!(cache.class_bytes(), 8_913_920);
        assert_eq!(cache.cpu_bytes(), 4 * 424259 + 131072 + 256);
        cache
            .resolve(&mut arena, 2, [11, 0], &vec![5; 424259], 7)
            .unwrap();
        assert!(!cache.entries.iter().any(|e| e.id == second.0));
        assert_eq!(cache.entries.len(), 6);
        let old = cache
            .resolve(&mut arena, 1, [255, 255], &vec![0; 131072], 7)
            .unwrap()
            .0;
        let changed = cache
            .resolve(&mut arena, 1, [511, 127], &vec![0; 131072], 7)
            .unwrap();
        assert_ne!(old, changed.0);
        assert!(!changed.1);
        assert!(cache.class_bytes() <= CACHE_BYTES);
    }

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
