use super::upload;
use super::*;
pub const MINIMAP: u32 = 12;
const HEADER: usize = 96;
/// One complete style table: `PnC_End` palette indices for one background colour.
const TABLE: usize = 38569;
const STYLES: usize = 16;

pub(super) struct MinimapState {
    pipeline: wgpu::ComputePipeline,
    resident: Resident,
    pub(super) background: Option<(u64, u32)>,
}

/// Live cached size classes, counted in arena source bytes so the bound holds in both
/// asset formats; the GPU cost is this times `assets::STRIDE`. It holds the advertised
/// retention at the validated maximum of 16 background colours: 16 x `STYLE_VERSIONS`
/// tables, one cell and one dictionary version and `PREFIX_VERSIONS` prefixes.
const CACHE_CLASS_BYTES: u64 = 5 << 20;
const PREFIX_VERSIONS: usize = 16;
const STYLE_VERSIONS: usize = 4;
const ROLE_PREFIX: u32 = 0;
const ROLE_DICTIONARY: u32 = 1;
const ROLE_CELLS: u32 = 2;
/// One role per style table, so a table that did not change is not re-uploaded
/// when its neighbours did.
const ROLE_STYLE: u32 = 3;
/// Bytes sampled at each end of a segment to reject a changed one without reading it
/// all. `update_panel_colors` rewrites entries 3, 4 and 10 of every style table, so the
/// leading sample alone rejects the common style change.
const PROBE: usize = 16;

/// Versions one role retains. The dictionary and the cells describe the current
/// world, so a change replaces them; the style tables recur over the blink and
/// highlight phases and are worth keeping.
fn versions(role: u32) -> usize {
    match role {
        ROLE_PREFIX => PREFIX_VERSIONS,
        ROLE_DICTIONARY | ROLE_CELLS => 1,
        _ => STYLE_VERSIONS,
    }
}

/// Whether the byte budget may retire this role. The single-version roles are the base
/// the next world command needs, so table pressure must not evict them.
fn evictable(role: u32) -> bool {
    versions(role) > 1
}

/// The arena's rounded class for a segment, in arena source bytes.
fn class_bytes(length: usize) -> u64 {
    (length.max(256) as u64).next_power_of_two()
}

type Probe = (usize, [u8; PROBE], [u8; PROBE]);

fn probe(bytes: &[u8]) -> Probe {
    let mut head = [0; PROBE];
    let mut tail = [0; PROBE];
    let taken = bytes.len().min(PROBE);
    head[..taken].copy_from_slice(&bytes[..taken]);
    tail[..taken].copy_from_slice(&bytes[bytes.len() - taken..]);
    (bytes.len(), head, tail)
}

struct Segment {
    id: u64,
    role: u32,
    layout: [u32; 2],
    probe: Probe,
    bytes: Vec<u8>,
    used: u64,
}

/// Immutable minimap segments kept by exact content. Its identities are private to
/// the renderer and outlive the transient source each command carries; a hit here
/// is an identity, not a promise of arena residency.
#[derive(Default)]
pub(super) struct Resident {
    entries: Vec<Segment>,
    clock: u64,
    evictions: u64,
}

impl Resident {
    /// Live arena source bytes; multiply by `assets::STRIDE` for the GPU figure.
    fn class_bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| class_bytes(e.bytes.len()))
            .sum()
    }

    /// The owning host copies this cache holds, separate from the C, bridge and
    /// renderer copies of the whole source that already exist per command.
    fn cpu_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.bytes.len() as u64).sum()
    }

    fn oldest(&self, mut wanted: impl FnMut(&Segment) -> bool) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| wanted(e))
            .min_by_key(|(_, e)| e.used)
            .map(|(index, _)| index)
    }

    fn retire(&mut self, arena: &mut arena::Arena, index: usize) {
        self.evictions += 1;
        arena.release(self.entries.remove(index).id);
    }

    /// Exact role, layout and byte identity. Only the retained versions of this role
    /// and layout are candidates, and the length and end samples reject a changed
    /// segment before the full comparison reads it.
    fn resolve(
        &mut self,
        arena: &mut arena::Arena,
        role: u32,
        layout: [u32; 2],
        bytes: &[u8],
    ) -> Result<(u64, bool)> {
        self.clock += 1;
        let probe = probe(bytes);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.role == role && e.layout == layout && e.probe == probe && e.bytes == bytes)
        {
            entry.used = self.clock;
            return Ok((entry.id, true));
        }
        while self.entries.iter().filter(|e| e.role == role).count() >= versions(role) {
            let index = self
                .oldest(|e| e.role == role)
                .context("empty minimap role")?;
            self.retire(arena, index);
        }
        while self.class_bytes() + class_bytes(bytes.len()) > CACHE_CLASS_BYTES {
            let Some(index) = self.oldest(|e| evictable(e.role)) else {
                break;
            };
            self.retire(arena, index);
        }
        let id = next_handle()?;
        self.entries.push(Segment {
            id,
            role,
            layout,
            probe,
            bytes: bytes.to_vec(),
            used: self.clock,
        });
        Ok((id, false))
    }
}

/// Role, identity layout and source range of every segment one command carries.
/// The prefix holds the header and the pattern the kernel still addresses through
/// `h()`; the dictionary is resident for identity and accounting, since the kernel
/// resolves background colours through the uniform nibble table instead.
fn segment_plan(h: &[u32; 24], prefix: std::ops::Range<usize>) -> Vec<(u32, [u32; 2], Range)> {
    let mut plan = vec![(ROLE_PREFIX, [h[0], h[5]], prefix)];
    if h[0] == 0 {
        let tables = h[15] as usize / TABLE;
        plan.push((ROLE_DICTIONARY, [tables as u32, 0], range(h[12], h[13])));
        plan.push((ROLE_CELLS, [h[10], h[11]], range(h[13], h[14])));
        for table in 0..tables {
            let start = h[14] as usize + table * TABLE;
            plan.push((
                ROLE_STYLE + table as u32,
                [tables as u32, table as u32],
                start..start + TABLE,
            ));
        }
    }
    plan
}

/// Only an arena overflow is worth retrying on the contiguous path; a device or
/// validation failure must reach the host as itself.
fn overflowed(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string() == arena::OVERFLOW)
}

type Range = std::ops::Range<usize>;

fn range(start: u32, end: u32) -> Range {
    start as usize..end as usize
}

fn background_colours(header: &[u32; 24], bytes: &[u8]) -> [u32; 32] {
    let mut colours = [0; 32];
    if header[0] == 0 {
        let start = header[12] as usize;
        for (index, &colour) in bytes[start..start + (header[15] / TABLE as u32) as usize]
            .iter()
            .enumerate()
            .rev()
        {
            let shift = (u32::from(colour) & 7) * 4;
            let word = &mut colours[usize::from(colour) / 8];
            *word = (*word & !(15 << shift)) | ((index as u32 & 15) << shift);
        }
    }
    colours
}

const PATTERN_LOW: i64 = 2;
const PATTERN_HIGH: i64 = 3;
const SWEEP_LIMIT: i64 = 1 << 16;

fn pattern_within_bound(h: &[u32; 24], b: &[u8]) -> bool {
    (0..h[18] as usize).all(|i| {
        let o = h[22] as usize + i * 8;
        o + 8 <= b.len()
            && [o, o + 4].into_iter().all(|s| {
                let delta = i64::from(i32::from_le_bytes(b[s..s + 4].try_into().unwrap()));
                (-PATTERN_LOW..=PATTERN_HIGH).contains(&delta)
            })
    })
}

fn grow(centre: [i64; 2], low: i64, high: i64) -> [i64; 4] {
    [
        centre[0] - low,
        centre[1] - low,
        centre[0] + high + 1,
        centre[1] + high + 1,
    ]
}

fn merge(a: [i64; 4], b: [i64; 4]) -> [i64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Half-open box, clamped to `[0, h[5])`, containing every pixel the minimap kernel
/// can write. Supersets are safe; an empty box means the command writes nothing.
fn written_box(h: &[u32; 24], b: &[u8]) -> [u32; 4] {
    let d = i64::from(h[5]);
    let full = [0, 0, h[5], h[5]];
    let si = |i: usize| i64::from(h[i] as i32);
    let centre = [si(16), si(17)];
    let raw = match h[0] {
        1 => {
            if !pattern_within_bound(h, b) {
                return full;
            }
            let spread = si(19).abs();
            grow(centre, PATTERN_LOW + spread, PATTERN_HIGH + spread)
        }
        2 => {
            let radius = i64::from(h[18]) + 1;
            if radius >= d / 2 {
                return full;
            }
            grow(centre, radius, radius)
        }
        3 => {
            if !pattern_within_bound(h, b) || si(21) - 4 > SWEEP_LIMIT {
                return full;
            }
            let step = [h[6] as i32, h[7] as i32];
            let mut pos = [h[16] as i32, h[17] as i32];
            let mut remaining = si(21) - 4;
            let mut swept: Option<[i64; 4]> = None;
            while remaining > 0 {
                if pos[0] < 0
                    || pos[1] < 0
                    || (pos[0] >> 8) >= h[5] as i32
                    || (pos[1] >> 8) >= h[5] as i32
                {
                    break;
                }
                let (Some(x), Some(y)) = (pos[0].checked_add(step[0]), pos[1].checked_add(step[1]))
                else {
                    return full;
                };
                pos = [x, y];
                let next = grow(
                    [i64::from(x >> 8), i64::from(y >> 8)],
                    PATTERN_LOW,
                    PATTERN_HIGH,
                );
                swept = Some(swept.map_or(next, |seen| merge(seen, next)));
                remaining -= 4;
            }
            match swept {
                Some(swept) => swept,
                None => return [0; 4],
            }
        }
        _ => return full,
    };
    let clamp = |v: i64| v.clamp(0, d) as u32;
    let bounds = [clamp(raw[0]), clamp(raw[1]), clamp(raw[2]), clamp(raw[3])];
    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return [0; 4];
    }
    bounds
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
                && h[15] as usize <= STYLES * TABLE,
            "invalid minimap world descriptor"
        );
        let cell_end = u64::from(h[13]) + (u64::from(h[10]) + 1) * (u64::from(h[11]) + 1) * 2;
        ensure!(
            u64::from(h[14]) == cell_end && b.len() as u64 == cell_end + u64::from(h[15]),
            "invalid minimap world extent"
        );
        ensure!(
            h[15].is_multiple_of(TABLE as u32),
            "invalid minimap palette size"
        );
        for cell in b[h[13] as usize..h[14] as usize].as_chunks::<2>().0 {
            ensure!(
                usize::from(u16::from_le_bytes(*cell)) < TABLE,
                "invalid minimap cell"
            );
        }
        let n = h[15] / TABLE as u32;
        ensure!(n as usize <= STYLES, "too many minimap backgrounds");
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
        let colours = background_colours(&h, &source.bytes);
        let bounds = written_box(&h, &source.bytes);
        let (span_x, span_y) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
        ensure!(
            source.bytes.len() as u64 * 4 <= self.storage_limit(),
            "minimap source exceeds GPU storage"
        );
        ensure!(
            span_x.div_ceil(8).max(span_y.div_ceil(8))
                <= self.device.limits().max_compute_workgroups_per_dimension,
            "minimap dispatch exceeds GPU limit"
        );
        if self.minimap.is_none() {
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("native minimap"),
                    source: wgpu::ShaderSource::Wgsl(
                        assets::shader(include_str!("draw_minimap.wgsl")).into(),
                    ),
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
                resident: Resident::default(),
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
        let limit = self.storage_limit() as usize;
        let plan = segment_plan(&h, 0..HEADER + h[23] as usize * 8);
        let mut split = self.arena.enabled()
            && plan
                .iter()
                .map(|(_, _, r)| class_bytes(r.len()))
                .sum::<u64>()
                <= CACHE_CLASS_BYTES;
        let mut segments = Vec::new();
        if split {
            let source = &self.resources[&c.source].bytes;
            let resident = &mut self.minimap.as_mut().unwrap().resident;
            for (role, layout, range) in &plan {
                let (id, _) =
                    resident.resolve(&mut self.arena, *role, *layout, &source[range.clone()])?;
                segments.push(id);
            }
            self.counters.minimap_cache_class_bytes =
                resident.class_bytes() * assets::STRIDE as u64;
            self.counters.minimap_cache_cpu_bytes = resident.cpu_bytes();
            self.counters.minimap_cache_evictions = resident.evictions;
        }
        let (bases, words) = loop {
            let demand = if split {
                segments
                    .iter()
                    .zip(&plan)
                    .try_fold(0, |sum, (id, (_, _, range))| {
                        self.arena
                            .allocation_bytes(*id, range.len())
                            .map(|n| sum + n)
                    })?
            } else {
                self.arena
                    .allocation_bytes(c.source, self.resources[&c.source].bytes.len())?
            };
            self.arena_headroom(demand.saturating_sub(self.resource_bytes as u64))?;
            let bytes = &self.resources[&c.source].bytes;
            let mut packer = asset_packer(
                &self.device,
                &self.queue,
                &mut self.arena,
                &mut self.counters,
                self.asset_generation,
                limit,
            );
            let mut bases = [0; 2 + STYLES];
            let packed = (|| -> Result<()> {
                if split {
                    for (index, (id, (role, _, range))) in segments.iter().zip(&plan).enumerate() {
                        let offset =
                            packer.offset(*id, &bytes[range.clone()], ResourceKind::Minimap)?;
                        let slot = match *role {
                            ROLE_PREFIX => 0,
                            ROLE_DICTIONARY => continue,
                            ROLE_CELLS => 1,
                            _ => 2 + (index - 3),
                        };
                        bases[slot] = offset;
                    }
                } else {
                    let base = packer.offset(c.source, bytes, ResourceKind::Minimap)?;
                    bases[0] = base;
                    if h[0] == 0 {
                        bases[1] = base + h[13];
                        for table in 0..h[15] as usize / TABLE {
                            bases[2 + table] = base + h[14] + (table * TABLE) as u32;
                        }
                    }
                }
                Ok(())
            })();
            let words = packer.finish();
            match packed {
                Ok(()) => break (bases, words),
                // A segment plan can fragment where one contiguous class still fits.
                // Earlier readers of this encoder still own the released regions
                // until retirement, and the counters already carry what was uploaded.
                Err(error) if split && overflowed(&error) => {
                    for id in &segments {
                        self.arena.release(*id);
                    }
                    split = false;
                }
                Err(error) => return Err(error),
            }
        };
        let base = bases[0];
        let styles: [u32; STYLES] = std::array::from_fn(|table| {
            if table < (h[15] as usize / TABLE).max(1) {
                bases[2 + table]
            } else {
                bases[2]
            }
        });
        if span_x == 0 || span_y == 0 {
            if let Some(words) = &words {
                self.counters.asset_upload_bytes += words.len() as u64 * assets::STRIDE as u64;
            }
            self.counters.batches += 1;
            self.counters.commands += 1;
            return self.check_status();
        }
        let assets = match &words {
            Some(words) => byte_buffer(
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
        let dummy = self.shadow_placeholder.clone();
        let state = self.minimap.as_ref().unwrap();
        let pipeline = state.pipeline.clone();
        let background = if h[0] == 0 {
            &self.target_snapshots[&state.background.unwrap().0].indices
        } else {
            &dummy
        };
        let target = &self.targets[&target_id];
        let mut view_words = [0; 60];
        view_words[..4].copy_from_slice(&[target.width, target.pitch, target.offset, base]);
        view_words[4..36].copy_from_slice(&colours);
        view_words[36..40].copy_from_slice(&bounds);
        view_words[40] = bases[1];
        view_words[44..].copy_from_slice(&styles);
        let view = upload::stage(
            &self.uploads,
            &self.device,
            &mut self.counters,
            "minimap target view",
            &view_words,
            wgpu::BufferUsages::UNIFORM,
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, &self.targets[&target_id].indices),
                entry(1, &assets),
                entry(2, background),
                view.entry(3),
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
            pass.dispatch_workgroups(span_x.div_ceil(8), span_y.div_ceil(8), 1);
        }
        self.counters.dispatches += 1;
        self.pass_boundary();
        self.counters.batches += 1;
        self.counters.commands += 1;
        if let Some(words) = &words {
            self.counters.asset_upload_bytes += words.len() as u64 * assets::STRIDE as u64;
        }
        self.check_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn styles(header: &[u32; 24]) -> usize {
        header[15] as usize / TABLE
    }

    fn world_header() -> [u32; 24] {
        let mut h = [0u32; 24];
        h[1] = 64;
        h[2] = 64;
        h[5] = 32;
        h[10] = 3;
        h[11] = 5;
        h[22] = 96;
        h[23] = 36;
        h[12] = 384;
        h[13] = h[12] + 256;
        h[14] = h[13] + (h[10] + 1) * (h[11] + 1) * 2;
        h[15] = 3 * TABLE as u32;
        h
    }

    #[test]
    fn the_segment_plan_partitions_the_validated_payload() {
        let h = world_header();
        let plan = segment_plan(&h, 0..384);
        assert_eq!(plan.len(), 3 + styles(&h));
        let mut covered = 0;
        for (_, _, range) in &plan {
            assert_eq!(range.start, covered);
            covered = range.end;
        }
        assert_eq!(covered as u32, h[14] + h[15]);
        let roles: Vec<_> = plan.iter().map(|(role, _, _)| *role).collect();
        assert_eq!(roles, [ROLE_PREFIX, ROLE_DICTIONARY, ROLE_CELLS, 3, 4, 5]);
        let mut overlay = h;
        overlay[0] = 1;
        assert_eq!(segment_plan(&overlay, 0..384).len(), 1);
    }

    #[test]
    fn style_tables_version_independently_of_their_neighbours() {
        let mut arena = arena::Arena::new(32 << 20);
        let mut resident = Resident::default();
        let table = |value: u8| vec![value; TABLE];
        let mut resolve = |resident: &mut Resident, index: u32, value: u8| {
            resident
                .resolve(&mut arena, ROLE_STYLE + index, [2, index], &table(value))
                .unwrap()
        };
        let first = resolve(&mut resident, 0, 1);
        let second = resolve(&mut resident, 1, 9);
        assert!(!first.1 && !second.1);
        assert_ne!(first.0, second.0);

        let changed = resolve(&mut resident, 0, 2);
        assert!(!changed.1);
        assert_eq!(resolve(&mut resident, 1, 9), (second.0, true));
        assert_eq!(resolve(&mut resident, 0, 1), (first.0, true));

        for value in 3..3 + STYLE_VERSIONS as u8 {
            resolve(&mut resident, 0, value);
        }
        assert_eq!(
            resident
                .entries
                .iter()
                .filter(|e| e.role == ROLE_STYLE)
                .count(),
            STYLE_VERSIONS
        );
        assert_ne!(resolve(&mut resident, 0, 1).0, first.0);
        assert_eq!(resolve(&mut resident, 1, 9), (second.0, true));
    }

    #[test]
    fn the_cache_budget_retires_the_least_recently_used_segment() {
        let mut arena = arena::Arena::new(32 << 20);
        let mut resident = Resident::default();
        let slots = (CACHE_CLASS_BYTES / class_bytes(TABLE)) as usize;
        let mut resolve = |resident: &mut Resident, index: usize, value: usize| {
            resident
                .resolve(
                    &mut arena,
                    ROLE_STYLE + index as u32,
                    [16, 0],
                    &vec![value as u8; TABLE],
                )
                .unwrap()
        };
        let first = resolve(&mut resident, 0, 0);
        for step in 1..=slots {
            resolve(&mut resident, step / STYLE_VERSIONS, step);
            assert!(resident.class_bytes() <= CACHE_CLASS_BYTES);
        }
        assert_eq!(
            resident.cpu_bytes(),
            resident.entries.len() as u64 * TABLE as u64
        );
        assert_ne!(resolve(&mut resident, 0, 0).0, first.0);
    }

    #[test]
    fn the_budget_holds_the_advertised_retention_at_sixteen_backgrounds() {
        let base = class_bytes(384) * PREFIX_VERSIONS as u64
            + class_bytes(256)
            + class_bytes(256 * 256 * 2);
        let styles = STYLES as u64 * STYLE_VERSIONS as u64 * class_bytes(TABLE);
        assert!(base + styles <= CACHE_CLASS_BYTES, "{base} + {styles}");
    }

    #[test]
    fn table_pressure_never_retires_the_cells_or_the_dictionary() {
        let mut arena = arena::Arena::new(32 << 20);
        let mut resident = Resident::default();
        let cells = vec![3u8; 131072];
        let dictionary = [7u8; 256];
        let cell_id = resident
            .resolve(&mut arena, ROLE_CELLS, [255, 255], &cells)
            .unwrap()
            .0;
        let dictionary_id = resident
            .resolve(&mut arena, ROLE_DICTIONARY, [16, 0], &dictionary)
            .unwrap()
            .0;
        let slots = (CACHE_CLASS_BYTES / class_bytes(TABLE)) as usize;
        for step in 0..slots + STYLE_VERSIONS {
            resident
                .resolve(
                    &mut arena,
                    ROLE_STYLE + (step / STYLE_VERSIONS) as u32,
                    [16, 0],
                    &vec![step as u8; TABLE],
                )
                .unwrap();
        }
        assert!(resident.evictions > 0);
        assert_eq!(
            resident
                .resolve(&mut arena, ROLE_CELLS, [255, 255], &cells)
                .unwrap(),
            (cell_id, true)
        );
        assert_eq!(
            resident
                .resolve(&mut arena, ROLE_DICTIONARY, [16, 0], &dictionary)
                .unwrap(),
            (dictionary_id, true)
        );
    }

    #[test]
    fn the_end_samples_narrow_candidates_without_deciding_identity() {
        let mut arena = arena::Arena::new(32 << 20);
        let mut resident = Resident::default();
        let mut first = vec![1u8; TABLE];
        let mut second = first.clone();
        second[TABLE / 2] = 2;
        assert_eq!(probe(&first), probe(&second));
        let a = resident
            .resolve(&mut arena, ROLE_STYLE, [1, 0], &first)
            .unwrap();
        let b = resident
            .resolve(&mut arena, ROLE_STYLE, [1, 0], &second)
            .unwrap();
        assert_ne!(a.0, b.0);
        assert!(!b.1);
        assert_eq!(
            resident
                .resolve(&mut arena, ROLE_STYLE, [1, 0], &first)
                .unwrap(),
            (a.0, true)
        );
        first[3] = 9;
        assert_ne!(probe(&first), probe(&second));
        assert!(
            !resident
                .resolve(&mut arena, ROLE_STYLE, [1, 0], &first)
                .unwrap()
                .1
        );
    }

    #[test]
    fn only_an_arena_overflow_is_retried_on_the_contiguous_path() {
        assert!(overflowed(&anyhow::anyhow!(arena::OVERFLOW)));
        assert!(overflowed(
            &anyhow::anyhow!(arena::OVERFLOW).context("minimap segment")
        ));
        assert!(!overflowed(&anyhow::anyhow!("GPU error: device lost")));
        assert!(!overflowed(&anyhow::anyhow!("asset length overflow")));
    }

    #[test]
    fn a_changed_layout_replaces_the_single_cell_and_dictionary_version() {
        let mut arena = arena::Arena::new(32 << 20);
        let mut resident = Resident::default();
        let cells = vec![0u8; 131072];
        let first = resident
            .resolve(&mut arena, ROLE_CELLS, [255, 255], &cells)
            .unwrap();
        assert_eq!(
            resident
                .resolve(&mut arena, ROLE_CELLS, [255, 255], &cells)
                .unwrap(),
            (first.0, true)
        );
        let resized = resident
            .resolve(&mut arena, ROLE_CELLS, [511, 127], &cells)
            .unwrap();
        assert_ne!(resized.0, first.0);
        assert!(!resized.1);
        assert_eq!(
            resident
                .entries
                .iter()
                .filter(|e| e.role == ROLE_CELLS)
                .count(),
            1
        );
        let dictionary = resident
            .resolve(&mut arena, ROLE_DICTIONARY, [11, 0], &[7; 256])
            .unwrap();
        assert!(!dictionary.1);
        assert!(
            !resident
                .resolve(&mut arena, ROLE_DICTIONARY, [11, 0], &[8; 256])
                .unwrap()
                .1
        );
        assert_eq!(
            resident
                .entries
                .iter()
                .filter(|e| e.role == ROLE_DICTIONARY)
                .count(),
            1
        );
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

    fn table(dictionary: &[u8]) -> [u32; 32] {
        let mut header = [0; 24];
        header[12] = 4;
        header[15] = dictionary.len() as u32 * TABLE as u32;
        let mut bytes = vec![0; 4];
        bytes.extend_from_slice(dictionary);
        background_colours(&header, &bytes)
    }

    fn index(colours: &[u32; 32], colour: u8) -> u32 {
        (colours[colour as usize / 8] >> ((u32::from(colour) & 7) * 4)) & 15
    }

    #[test]
    fn duplicate_and_overflowing_dictionary_entries_keep_their_own_nibble() {
        let colours = table(&[9, 17, 9, 16]);
        assert_eq!(index(&colours, 9), 0);
        assert_eq!(index(&colours, 17), 1);
        assert_eq!(index(&colours, 16), 3);
        assert_eq!(index(&colours, 8), 0);

        let dictionary: Vec<u8> = (8..28).collect();
        let colours = table(&dictionary);
        for (position, &colour) in dictionary.iter().enumerate() {
            assert_eq!(index(&colours, colour), position as u32 & 15);
        }
    }

    const DRAW_SQUARE: [(i32, i32); 36] = [
        (0, 0),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
        (2, -1),
        (2, 0),
        (2, 1),
        (2, 2),
        (1, 2),
        (0, 2),
        (-1, 2),
        (-2, 2),
        (-2, 1),
        (-2, 0),
        (-2, -1),
        (-2, -2),
        (-1, -2),
        (0, -2),
        (1, -2),
        (2, -2),
        (3, -2),
        (3, -1),
        (3, 0),
        (3, 1),
        (3, 2),
        (3, 3),
        (2, 3),
        (1, 3),
        (0, 3),
        (-1, 3),
        (-2, 3),
    ];

    fn payload(h: &[u32; 24], pattern: &[(i32, i32)]) -> Vec<u8> {
        let mut b: Vec<u8> = h.iter().flat_map(|v| v.to_le_bytes()).collect();
        for (dx, dy) in pattern {
            b.extend(dx.to_le_bytes());
            b.extend(dy.to_le_bytes());
        }
        b
    }

    fn shipped_box(h: &[u32; 24]) -> [u32; 4] {
        written_box(h, &payload(h, &DRAW_SQUARE))
    }

    fn noise(state: &mut u32) -> u32 {
        *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *state >> 8
    }

    fn pattern_hit(h: &[u32; 24], b: &[u8], p: [i32; 2], centre: [i32; 2], spread: i32) -> bool {
        (0..h[18] as usize).any(|i| {
            let o = h[22] as usize + i * 8;
            let dx = p[0] - centre[0] - i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            let dy = p[1] - centre[1] - i32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap());
            (dx == 0 && dy == 0)
                || (spread != 0
                    && ((dy == 0 && dx.abs() == spread.abs())
                        || (dx == 0 && dy.abs() == spread.abs())))
        })
    }

    fn octant(q: [i32; 2], x: i32, y: i32) -> bool {
        (q[0].abs() == x && q[1].abs() == y) || (q[0].abs() == y && q[1].abs() == x)
    }

    fn circle_hit(h: &[u32; 24], q: [i32; 2], early: bool) -> bool {
        let (high, low) = (q[0].abs().max(q[1].abs()), q[0].abs().min(q[1].abs()));
        let increment = h[21] as i32;
        let mut y = h[18] as i32;
        let mut x = 0;
        let mut decision = 3 - 2 * y;
        if y <= 1 {
            return false;
        }
        while x < y {
            if early && (x > low || y < high) {
                break;
            }
            if octant(q, x, y) {
                return true;
            }
            if decision >= 0 {
                decision += 4 * (x - y) + increment;
                y -= 1;
            } else {
                decision += 4 * (x - 1) + increment;
            }
            x += 1;
        }
        x == y && octant(q, x, y)
    }

    fn kernel_writes(h: &[u32; 24], b: &[u8], p: [i32; 2]) -> bool {
        let si = |i: usize| h[i] as i32;
        match h[0] {
            1 => pattern_hit(h, b, p, [si(16), si(17)], si(19)),
            2 => circle_hit(h, [p[0] - si(16), p[1] - si(17)], false),
            3 => {
                let mut pos = [si(16), si(17)];
                let mut remaining = si(21) - 4;
                while remaining > 0 {
                    if pos[0] < 0 || pos[1] < 0 || (pos[0] >> 8) >= si(5) || (pos[1] >> 8) >= si(5)
                    {
                        break;
                    }
                    pos = [pos[0] + si(6), pos[1] + si(7)];
                    if pattern_hit(h, b, p, [pos[0] >> 8, pos[1] >> 8], 0) {
                        return true;
                    }
                    remaining -= 4;
                }
                false
            }
            _ => true,
        }
    }

    #[test]
    fn written_box_is_a_superset_of_every_written_pixel() {
        let d = 32u32;
        let mut seed = 0x5eed_u32;
        let mut boxed = 0;
        for mode in [1u32, 2, 3] {
            for _ in 0..96 {
                let mut h = [0u32; 24];
                h[0] = mode;
                h[1] = 64;
                h[2] = 64;
                h[5] = d;
                h[22] = 96;
                h[23] = 36;
                h[16] = (noise(&mut seed) % (d + 16)).wrapping_sub(8);
                h[17] = (noise(&mut seed) % (d + 16)).wrapping_sub(8);
                h[18] = noise(&mut seed) % 37;
                h[19] = (noise(&mut seed) % 17).wrapping_sub(8);
                h[20] = noise(&mut seed) % 256;
                h[21] = noise(&mut seed) % 200;
                if mode == 2 {
                    h[18] = noise(&mut seed) % 24;
                }
                if mode == 3 {
                    h[16] = (h[16] as i32).wrapping_mul(256) as u32;
                    h[17] = (h[17] as i32).wrapping_mul(256) as u32;
                    h[6] = (noise(&mut seed) % 1024).wrapping_sub(512);
                    h[7] = (noise(&mut seed) % 1024).wrapping_sub(512);
                    h[21] = noise(&mut seed) % 400;
                }
                let b = payload(&h, &DRAW_SQUARE);
                let bounds = written_box(&h, &b);
                if bounds != [0, 0, d, d] {
                    boxed += 1;
                }
                for y in 0..d as i32 {
                    for x in 0..d as i32 {
                        if kernel_writes(&h, &b, [x, y]) {
                            assert!(
                                x >= bounds[0] as i32
                                    && x < bounds[2] as i32
                                    && y >= bounds[1] as i32
                                    && y < bounds[3] as i32,
                                "mode {mode} pixel ({x},{y}) outside {bounds:?} of {h:?}"
                            );
                        }
                    }
                }
            }
        }
        assert!(boxed > 200, "the randomized headers never narrowed the box");
    }

    #[test]
    fn written_box_clamps_to_the_square_and_reports_empty_spans() {
        let mut h = [0u32; 24];
        h[0] = 1;
        h[1] = 64;
        h[2] = 64;
        h[5] = 32;
        h[18] = 36;
        h[22] = 96;
        h[23] = 36;
        assert_eq!(shipped_box(&h), [0, 0, 4, 4]);
        h[16] = 31;
        h[17] = 31;
        assert_eq!(shipped_box(&h), [29, 29, 32, 32]);
        h[19] = (-4i32) as u32;
        assert_eq!(shipped_box(&h), [25, 25, 32, 32]);
        h[19] = 0;
        h[16] = (-40i32) as u32;
        assert_eq!(shipped_box(&h), [0; 4]);
        h[0] = 3;
        h[16] = 0;
        h[21] = 4;
        assert_eq!(shipped_box(&h), [0; 4]);
        h[0] = 0;
        assert_eq!(shipped_box(&h), [0, 0, 32, 32]);
        h[0] = 4;
        assert_eq!(shipped_box(&h), [0, 0, 32, 32]);
        h[0] = 2;
        h[16] = 16;
        h[17] = 16;
        h[18] = 4;
        assert_eq!(shipped_box(&h), [11, 11, 22, 22]);
        h[18] = 15;
        assert_eq!(shipped_box(&h), [0, 0, 32, 32]);
    }

    #[test]
    fn shipped_pattern_stays_inside_the_assumed_bound() {
        let bound = -(PATTERN_LOW as i32)..=PATTERN_HIGH as i32;
        assert!(
            DRAW_SQUARE
                .iter()
                .all(|(x, y)| bound.contains(x) && bound.contains(y))
        );
        let mut h = [0u32; 24];
        h[0] = 1;
        h[1] = 64;
        h[2] = 64;
        h[5] = 32;
        h[16] = 16;
        h[17] = 16;
        h[18] = 36;
        h[22] = 96;
        h[23] = 36;
        assert_eq!(shipped_box(&h), [14, 14, 20, 20]);
        let mut wide = DRAW_SQUARE;
        wide[35] = (4, 0);
        assert_eq!(written_box(&h, &payload(&h, &wide)), [0, 0, 32, 32]);
    }

    #[test]
    fn circle_early_break_matches_the_full_bresenham() {
        for radius in 0..52u32 {
            for increment in [0u32, 1, 6] {
                let mut h = [0u32; 24];
                h[0] = 2;
                h[18] = radius;
                h[21] = increment;
                let span = radius as i32 + 2;
                for y in -span..=span {
                    for x in -span..=span {
                        assert_eq!(
                            circle_hit(&h, [x, y], false),
                            circle_hit(&h, [x, y], true),
                            "radius {radius} increment {increment} at ({x},{y})"
                        );
                    }
                }
            }
        }
    }
}
