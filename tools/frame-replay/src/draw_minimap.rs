use super::upload;
use super::*;
pub const MINIMAP: u32 = 12;
const HEADER: usize = 96;
pub(super) struct MinimapState {
    pipeline: wgpu::ComputePipeline,
    pub(super) background: Option<(u64, u32)>,
}

fn background_colours(header: &[u32; 24], bytes: &[u8]) -> [u32; 32] {
    let mut colours = [0; 32];
    if header[0] == 0 {
        let start = header[12] as usize;
        for (index, &colour) in bytes[start..start + (header[15] / 38569) as usize]
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
        self.arena_headroom(0)?;
        let bytes = &self.resources[&c.source].bytes;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let base = packer.offset(c.source, bytes, ResourceKind::Minimap)?;
        let words = packer.finish();
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
        let mut view_words = [0; 40];
        view_words[..4].copy_from_slice(&[target.width, target.pitch, target.offset, base]);
        view_words[4..36].copy_from_slice(&colours);
        view_words[36..].copy_from_slice(&bounds);
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
        header[15] = dictionary.len() as u32 * 38569;
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
