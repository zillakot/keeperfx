use super::upload;
use super::*;

pub(super) fn ordered(command: &Command) -> bool {
    command.kind == SPRITE && command.source_x & 8 != 0
}

/// Validates the sprite asset and returns the half-open destination box a *raster*
/// sprite can write inside, in view space: the union of the per-call axis ranges, which
/// this function proves contiguous. Outside it `sprite_axis` returns `count`,
/// `sprite_sample` returns the transparent index and `draw.wgsl` skips the pixel. An
/// ordered sprite writes through its own kernel and takes `write_rect` instead.
pub(super) fn validate(command: &Command, source: &Resource) -> Result<[i64; 4]> {
    let w = command.source_width as usize;
    let h = command.source_height as usize;
    ensure!(
        w > 0 && h > 0 && w <= 8192 && h <= 8192,
        "invalid sprite dimensions"
    );
    ensure!(
        command.source_x <= 15
            && command.source_y <= if ordered(command) { 3 } else { 0 }
            && command.transparent == 256
            && (!ordered(command) || (command.source_x & 1 != 0 && command.blend == 0)),
        "invalid sprite options"
    );
    let axis = 2 * w * h;
    ensure!(
        source.bytes.len() == axis + 8 * (w + h) + 256,
        "invalid sprite asset length"
    );
    for row in source.bytes[..axis].chunks_exact(w * 2) {
        let mut in_run = false;
        for pixel in row.as_chunks::<2>().0 {
            ensure!(
                pixel[1] <= if ordered(command) { 2 } else { 1 },
                "invalid sprite coverage"
            );
            if ordered(command) {
                ensure!(pixel[1] != 0 || !in_run, "unterminated sprite run");
                in_run = pixel[1] == 1;
            }
        }
        ensure!(!in_run, "unterminated sprite row");
    }
    let mut span = [0i64; 4];
    for (slot, (offset, count)) in [(axis, w), (axis + 8 * w, h)].into_iter().enumerate() {
        let mut previous = None;
        for i in 0..count {
            let index = offset + 8 * i;
            let start = u32::from_le_bytes(source.bytes[index..index + 4].try_into().unwrap());
            let length = u32::from_le_bytes(source.bytes[index + 4..index + 8].try_into().unwrap());
            ensure!(
                start <= 16384 && length <= 16384 && start + length <= 16384,
                "invalid sprite scaling range"
            );
            ensure!(
                previous.is_none_or(|end| end == start),
                "noncontiguous sprite scaling ranges"
            );
            previous = Some(start + length);
            if i == 0 {
                span[slot] = i64::from(start);
            }
            span[slot + 2] = i64::from(start + length);
        }
    }
    Ok(span)
}

fn range(source: &Resource, offset: usize) -> (i64, i64) {
    let start = u32::from_le_bytes(source.bytes[offset..offset + 4].try_into().unwrap());
    let count = u32::from_le_bytes(source.bytes[offset + 4..offset + 8].try_into().unwrap());
    (i64::from(start), i64::from(count))
}

fn validate_target(c: &Command, source: &Resource, width: u32, height: u32) -> Result<()> {
    ensure!(
        c.x == 0 && c.y == 0 && c.width == width && c.height == height,
        "ordered sprite requires full target bounds"
    );
    let w = c.source_width as usize;
    let h = c.source_height as usize;
    let axis = 2 * w * h;
    for (offset, count, start, length, limit) in [
        (axis, w, c.clip_x, c.clip_width, width),
        (axis + w * 8, h, c.clip_y, c.clip_height, height),
    ] {
        ensure!(
            start >= 0 && i64::from(start) + i64::from(length) <= i64::from(limit),
            "ordered sprite clip outside target"
        );
        for i in 0..count {
            let (at, n) = range(source, offset + 8 * i);
            ensure!(
                at >= i64::from(start) && at + n <= i64::from(start) + i64::from(length),
                "ordered sprite range outside clip"
            );
        }
    }
    for sy in 0..h {
        let ay = if c.source_x & 2 != 0 { h - 1 - sy } else { sy };
        let (y, n) = range(source, axis + (w + ay) * 8);
        if n <= 1 || y != 0 {
            continue;
        }
        for sx in 0..w {
            if source.bytes[2 * (sy * w + sx) + 1] == 2 {
                let (x, _) = range(source, axis + (w - 1 - sx) * 8);
                ensure!(x > 0, "ordered sprite row copy outside target");
            }
        }
    }
    Ok(())
}

/// A half-open `[x0, y0, x1, y1)` superset of every target pixel one ordered sprite
/// reads or writes, in the target's own space.
///
/// The clip rectangle is the whole drawing window, so the bound comes from the sprite's
/// own scaling ranges instead: they are validated contiguous and ascending, addressed in
/// target coordinates, and `validate_target` proves every run lies inside them. A row
/// copy spans `[leftmost - 1, rightmost]`, one pixel left of the run it replicates,
/// exactly as the native right-to-left kernel does, so the rectangle grows a column to
/// the left; where that column would cross the row start it lands on the tail of the
/// previous row instead, and the rectangle widens to the whole row band one row higher.
///
/// A sprite scrolled fully off one side has every x range clamped to zero length. It
/// still writes: with `xcount == 0` the run collapses onto `xstart - 1` and each row
/// copy carries that one pixel down, so an empty x span keeps the left column and only
/// an empty y span, which skips every row, makes the rectangle empty.
pub(super) fn write_rect(c: &Command, source: &Resource, width: u32) -> [i64; 4] {
    let (w, h) = (c.source_width as usize, c.source_height as usize);
    let axis = 2 * w * h;
    if w == 0 || h == 0 || source.bytes.len() < axis + 8 * (w + h) {
        return [0; 4];
    }
    let span = |offset: usize, count: usize| {
        let (start, _) = range(source, offset);
        let (last, length) = range(source, offset + 8 * (count - 1));
        (start, last + length)
    };
    let (x0, x1) = span(axis, w);
    let (y0, y1) = span(axis + 8 * w, h);
    let mut rect = [
        x0.max(i64::from(c.clip_x)),
        y0.max(i64::from(c.clip_y)),
        x1.min(i64::from(c.clip_x) + i64::from(c.clip_width)),
        y1.min(i64::from(c.clip_y) + i64::from(c.clip_height)),
    ];
    if rect[1] >= rect[3] {
        return [0; 4];
    }
    rect[2] = rect[2].max(rect[0]);
    if rect[0] > 0 {
        rect[0] -= 1;
    } else {
        rect[1] = (rect[1] - 1).max(0);
        rect[2] = rect[2].max(i64::from(width));
    }
    rect
}

fn disjoint(a: &[i64; 4], b: &[i64; 4]) -> bool {
    a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1]
}

/// Groups consecutive ordered sprites whose write rectangles are pairwise disjoint into
/// one layer. A sprite overlapping any member of the open layer closes it and opens the
/// next, so layers run in frame order and the members of one layer commute. The returned
/// layers are re-checked pairwise: the greedy loop already proves it, so the check is a
/// tripwire for a future membership rule, not a live rejection path.
pub(super) fn layers(rects: &[[i64; 4]]) -> Result<Vec<Vec<u32>>> {
    let mut layers: Vec<Vec<u32>> = Vec::new();
    for (index, rect) in rects.iter().enumerate() {
        let open = layers
            .last()
            .is_some_and(|layer| layer.iter().all(|i| disjoint(rect, &rects[*i as usize])));
        if !open {
            layers.push(Vec::new());
        }
        layers.last_mut().unwrap().push(index as u32);
    }
    for layer in &layers {
        for (position, i) in layer.iter().enumerate() {
            for j in &layer[position + 1..] {
                ensure!(
                    disjoint(&rects[*i as usize], &rects[*j as usize]),
                    "ordered sprite layer holds an overlapping pair"
                );
            }
        }
    }
    Ok(layers)
}

impl DrawRenderer {
    fn identity_layer(&mut self, length: usize) -> wgpu::Buffer {
        let capacity = self
            .sprite_layer_identity
            .as_ref()
            .map_or(0, |identity| identity.size() as usize / 4);
        if capacity < length {
            let values: Vec<u32> = (0..length.next_power_of_two().max(16) as u32).collect();
            self.sprite_layer_identity = Some(buffer(
                &self.device,
                &mut self.counters,
                "ordered sprite identity layer",
                &values,
                wgpu::BufferUsages::STORAGE,
            ));
        }
        self.sprite_layer_identity.clone().unwrap()
    }

    pub(super) fn submit_ordered_sprites(
        &mut self,
        target_id: u64,
        commands: &[Command],
    ) -> Result<()> {
        let (width, height) = {
            let target = self
                .targets
                .get(&target_id)
                .context("unknown sprite target")?;
            (target.width, target.height)
        };
        let limit = self.storage_limit() as usize;
        self.arena_headroom(0)?;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        pack_commands(
            &mut packer,
            commands,
            &self.resources,
            ViewSpace::whole(width, height),
            limit,
            self.box_policy,
        )?;
        packer.finish();
        for c in commands.iter().filter(|c| ordered(c)) {
            validate_target(c, &self.resources[&c.source], width, height)?;
        }
        // A raster command between two ordered sprites orders them both, so a run of
        // consecutive ordered sprites is the largest set layering may reorder within.
        let mut run = Vec::new();
        for c in commands {
            if ordered(c) {
                run.push(*c);
                continue;
            }
            self.dispatch_ordered_layers(target_id, &run)?;
            run.clear();
            self.submit(target_id, std::slice::from_ref(c))?;
        }
        self.dispatch_ordered_layers(target_id, &run)
    }

    /// One compute pass of *M* workgroups per layer of mutually disjoint sprites, in
    /// frame order, all recorded into one encoder.
    fn dispatch_ordered_layers(&mut self, target_id: u64, run: &[Command]) -> Result<()> {
        if run.is_empty() {
            return Ok(());
        }
        let target = self
            .targets
            .get(&target_id)
            .context("unknown sprite target")?
            .clone();
        let limit = self.storage_limit() as usize;
        self.arena_headroom(0)?;
        let mut packer = asset_packer(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.counters,
            self.asset_generation,
            limit,
        );
        let words = pack_commands(
            &mut packer,
            run,
            &self.resources,
            ViewSpace::whole(target.width, target.height),
            limit,
            self.box_policy,
        )?;
        let assets = packer.finish();
        let rects: Vec<_> = run
            .iter()
            .map(|c| write_rect(c, &self.resources[&c.source], target.width))
            .collect();
        let layers = layers(&rects)?;
        ensure!(
            layers.iter().map(Vec::len).max().unwrap_or(0) as u32
                <= self.device.limits().max_compute_workgroups_per_dimension,
            "ordered sprite layer exceeds device limit"
        );
        let command_buffer = upload::stage(
            &self.uploads,
            &self.device,
            &mut self.counters,
            "ordered sprite commands",
            &words,
            wgpu::BufferUsages::STORAGE,
        );
        let asset_buffer = match &assets {
            Some(assets) => buffer(
                &self.device,
                &mut self.counters,
                "sprite artwork and run boundaries",
                assets,
                wgpu::BufferUsages::STORAGE,
            ),
            None => self
                .arena
                .binding(&self.device, &self.queue, &mut self.counters),
        };
        let parameters = upload::stage(
            &self.uploads,
            &self.device,
            &mut self.counters,
            "sprite target dimensions",
            &[
                target.width,
                target.height,
                target.width,
                0,
                target.pitch,
                target.offset,
                0,
                0,
                0,
                target.height,
                0,
                0,
            ],
            wgpu::BufferUsages::UNIFORM,
        );
        let mut layer_buffers = Vec::with_capacity(layers.len());
        let ordered = self.compute_sprite_ordered.clone();
        for layer in &layers {
            let indices = if layer.iter().enumerate().all(|(i, at)| *at as usize == i) {
                Region::whole(self.identity_layer(layer.len()))
            } else {
                self.counters.command_upload_bytes += layer.len() as u64 * 4;
                upload::stage(
                    &self.uploads,
                    &self.device,
                    &mut self.counters,
                    "ordered sprite layer",
                    layer,
                    wgpu::BufferUsages::STORAGE,
                )
            };
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ordered sprite layer"),
                layout: &self.compute_sprite_ordered.get_bind_group_layout(0),
                entries: &[
                    entry(0, &target.indices),
                    command_buffer.entry(1),
                    entry(2, &asset_buffer),
                    parameters.entry(3),
                    indices.entry(5),
                ],
            });
            let stamp = self.stamp(PASS_ORDERED_SPRITES);
            {
                let encoder = self.frame_encoder();
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("native sprite write and row-copy order"),
                    timestamp_writes: stamp.compute(),
                });
                pass.set_pipeline(&ordered);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups(layer.len() as u32, 1, 1);
            }
            self.counters.dispatches += 1;
            self.counters.ordered_sprite_passes += 1;
            self.pass_boundary();
            layer_buffers.push((indices, binding));
        }
        self.counters.ordered_sprite_layers += layers.len() as u64;
        self.check_status()?;
        self.counters.batches += layers.len() as u64;
        self.counters.commands += run.len() as u64;
        if let Some(assets) = &assets {
            self.counters.asset_upload_bytes += assets.len() as u64 * 4;
        }
        self.counters.command_upload_bytes += words.len() as u64 * 4;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{DrawRenderer, IMAGE, SPRITE};
    use std::io::Read;

    fn word(bytes: &[u8], index: usize) -> u32 {
        u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
    }

    #[test]
    fn rejects_malformed_sprite_assets() {
        let command = Command {
            kind: SPRITE,
            source_width: 1,
            source_height: 1,
            ..Default::default()
        };
        let mut resource = Resource {
            cursor: false,
            width: 1,
            height: 1,
            pitch: 1,
            bytes: vec![0; 274],
        };
        validate(&command, &resource).unwrap();
        resource.bytes[1] = 2;
        assert!(validate(&command, &resource).is_err());
        resource.bytes[1] = 1;
        resource.bytes[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate(&command, &resource).is_err());
        resource.bytes.clear();
        assert!(validate(&command, &resource).is_err());
        assert!(
            validate(
                &Command {
                    source_width: u32::MAX,
                    ..command
                },
                &resource
            )
            .is_err()
        );
    }

    #[test]
    fn ordered_sprite_validates_runs_and_copy_extent() {
        let mut command = Command {
            kind: SPRITE,
            width: 8,
            height: 8,
            clip_width: 8,
            clip_height: 8,
            source_x: 9,
            source_width: 2,
            source_height: 1,
            ..Default::default()
        };
        let mut resource = Resource {
            cursor: false,
            width: 1,
            height: 1,
            pitch: 1,
            bytes: vec![0; 284],
        };
        resource.bytes[1] = 2;
        resource.bytes[3] = 2;
        for (offset, value) in [(4, 0u32), (8, 4), (12, 4), (16, 4), (20, 1), (24, 3)] {
            resource.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        validate(&command, &resource).unwrap();
        validate_target(&command, &resource, 8, 8).unwrap();
        resource.bytes[3] = 1;
        assert!(validate(&command, &resource).is_err());
        resource.bytes[3] = 0;
        resource.bytes[1] = 1;
        assert!(validate(&command, &resource).is_err());
        resource.bytes[1] = 2;
        resource.bytes[3] = 2;
        resource.bytes[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(validate_target(&command, &resource, 8, 8).is_err());
        command.source_x = 11;
        assert!(validate_target(&command, &resource, 8, 8).is_err());
        resource.bytes[24..28].copy_from_slice(&1u32.to_le_bytes());
        validate_target(&command, &resource, 8, 8).unwrap();
        command.source_y = 3;
        validate(&command, &resource).unwrap();
        command.source_y = 4;
        assert!(validate(&command, &resource).is_err());
        command.source_y = 0;
        command.source_x = 8;
        assert!(validate(&command, &resource).is_err());
        command.source_x = 9;
        command.blend = 1;
        assert!(validate(&command, &resource).is_err());
    }

    /// One ordered sprite whose single run spans the clip rectangle and replicates it
    /// down every row of the clip, so the run's row copy reaches the column left of it.
    fn layered(clip_x: i32, clip_width: u32, target: u32) -> (Command, Resource) {
        let command = Command {
            kind: SPRITE,
            width: target,
            height: target,
            clip_x: 0,
            clip_y: 0,
            clip_width: target,
            clip_height: target,
            source_x: 9,
            source_width: 2,
            source_height: 1,
            ..Default::default()
        };
        let mut resource = Resource {
            cursor: false,
            width: 1,
            height: 1,
            pitch: 1,
            bytes: vec![0; 284],
        };
        resource.bytes[1] = 1;
        resource.bytes[3] = 2;
        let split = clip_width / 2;
        for (offset, value) in [
            (4, clip_x as u32),
            (8, split),
            (12, clip_x as u32 + split),
            (16, clip_width - split),
            (20, 2),
            (24, 6),
        ] {
            resource.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        validate(&command, &resource).unwrap();
        validate_target(&command, &resource, target, target).unwrap();
        (command, resource)
    }

    /// Every x range clamped to zero length, which is what the native clipper produces
    /// for a sprite scrolled fully off one side of the drawing window.
    fn scrolled_off(origin: i32, target: u32) -> (Command, Resource) {
        let (command, mut resource) = layered(origin, 4, target);
        for (offset, value) in [(4, origin as u32), (8, 0), (12, origin as u32), (16, 0)] {
            resource.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        validate(&command, &resource).unwrap();
        validate_target(&command, &resource, target, target).unwrap();
        (command, resource)
    }

    /// Every target address `sprite_ordered` can read or write, walked exactly as the
    /// kernel walks it, so the rectangle is checked against the kernel and not itself.
    fn touched(c: &Command, source: &Resource, width: i64) -> Vec<i64> {
        let w = c.source_width as usize;
        let h = c.source_height as usize;
        let axis = 2 * w * h;
        let stride = if c.source_x & 2 != 0 { -width } else { width };
        let mut hits = Vec::new();
        for sy in 0..h {
            let ay = if stride < 0 { h - 1 - sy } else { sy };
            let (ystart, ycount) = range(source, axis + (w + ay) * 8);
            if ycount == 0 {
                continue;
            }
            let y = if stride < 0 {
                ystart + ycount - 1
            } else {
                ystart
            };
            let (mut run_right, mut in_run) = (0, false);
            for sx in 0..w {
                let coverage = source.bytes[2 * (sy * w + sx) + 1];
                if coverage == 0 {
                    continue;
                }
                let (xstart, xcount) = range(source, axis + (w - 1 - sx) * 8);
                let right = y * width + xstart + xcount - 1;
                if !in_run {
                    run_right = right;
                    in_run = true;
                }
                hits.extend((0..xcount).map(|dx| right - dx));
                if coverage == 2 {
                    let left = y * width + xstart - 1;
                    for dy in 1..ycount {
                        hits.extend(
                            (0..=run_right - left)
                                .flat_map(|at| [left + at, left + dy * stride + at]),
                        );
                    }
                    in_run = false;
                }
            }
        }
        hits
    }

    fn covers(command: &Command, resource: &Resource, width: i64) -> [i64; 4] {
        let rect = write_rect(command, resource, width as u32);
        for address in touched(command, resource, width) {
            let (x, y) = (address % width, address / width);
            assert!(
                rect[0] <= x && x < rect[2] && rect[1] <= y && y < rect[3],
                "address {address} at ({x},{y}) escapes {rect:?}"
            );
        }
        rect
    }

    #[test]
    fn write_rect_covers_every_address_the_kernel_can_touch() {
        for (flip, origin, extent) in [(0, 4, 6), (2, 4, 6), (0, 1, 3), (2, 1, 3)] {
            let (mut command, resource) = layered(origin, extent, 16);
            command.source_x |= flip;
            let rect = covers(&command, &resource, 16);
            assert!(
                rect[0] < i64::from(origin),
                "the row copy reaches the column left of the run"
            );
        }
    }

    /// A run starting at column zero copies to the tail of the previous row, so the
    /// rectangle widens to the whole row band one row higher.
    #[test]
    fn write_rect_covers_a_row_copy_that_wraps_to_the_previous_row() {
        for flip in [0, 2] {
            let (mut command, resource) = layered(0, 6, 16);
            command.source_x |= flip;
            assert_eq!(covers(&command, &resource, 16), [0, 1, 16, 8]);
        }
    }

    /// Scrolled fully off, every x range is zero length and the run collapses onto the
    /// column left of it, which the kernel still copies down every replicated row.
    #[test]
    fn write_rect_covers_a_sprite_scrolled_off_the_window() {
        for flip in [0, 2] {
            let (mut command, resource) = scrolled_off(6, 16);
            command.source_x |= flip;
            assert_eq!(covers(&command, &resource, 16), [5, 2, 6, 8]);
        }
        let (command, resource) = scrolled_off(0, 16);
        assert_eq!(covers(&command, &resource, 16), [0, 1, 16, 8]);
    }

    #[test]
    fn layers_separate_overlapping_sprites_and_share_disjoint_ones() {
        let target = 16;
        let (right, right_asset) = layered(4, 6, target);
        let (left, left_asset) = layered(1, 3, target);
        let rects = [
            write_rect(&right, &right_asset, target),
            write_rect(&left, &left_asset, target),
            write_rect(&right, &right_asset, target),
        ];
        // The clip rectangles 4..10 and 1..4 do not overlap; the write rectangles do,
        // because the left sprite's row copy reaches column 0 and the right one's
        // reaches column 3.
        assert_eq!(layers(&rects[..2]).unwrap().len(), 2);
        assert_eq!(layers(&rects).unwrap(), vec![vec![0], vec![1], vec![2]]);
        let (apart, apart_asset) = layered(5, 5, target);
        let pair = [
            write_rect(&left, &left_asset, target),
            write_rect(&apart, &apart_asset, target),
        ];
        assert!(pair[0][2] == pair[1][0], "the rectangles touch at an edge");
        assert_eq!(layers(&pair).unwrap(), vec![vec![0, 1]]);
        let (band, mut band_asset) = layered(1, 3, target);
        band_asset.bytes[20..24].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            layers(&[
                write_rect(&left, &left_asset, target),
                write_rect(&band, &band_asset, target)
            ])
            .unwrap(),
            vec![vec![0, 1]],
            "disjoint row bands share a layer"
        );
    }

    #[test]
    fn a_sprite_scrolled_off_the_window_never_joins_an_open_layer() {
        let (touching, touching_asset) = layered(2, 3, 16);
        let (gone, gone_asset) = scrolled_off(6, 16);
        let rects = [
            write_rect(&touching, &touching_asset, 16),
            write_rect(&gone, &gone_asset, 16),
        ];
        assert_eq!(
            rects,
            [[1, 2, 5, 8], [5, 2, 6, 8]],
            "edge contact, not overlap"
        );
        assert_eq!(layers(&rects).unwrap(), vec![vec![0, 1]]);
        let (wide, wide_asset) = layered(2, 4, 16);
        let overlapping = [
            write_rect(&wide, &wide_asset, 16),
            write_rect(&gone, &gone_asset, 16),
        ];
        assert_eq!(layers(&overlapping).unwrap(), vec![vec![0], vec![1]]);
    }

    #[test]
    fn empty_write_rectangles_never_hold_a_layer_open() {
        let (clipped, mut asset) = layered(4, 6, 16);
        // An empty row band is the only empty case: the kernel skips every row.
        asset.bytes[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(write_rect(&clipped, &asset, 16), [0; 4]);
        let (visible, visible_asset) = layered(4, 6, 16);
        assert_eq!(
            layers(&[
                write_rect(&clipped, &asset, 16),
                write_rect(&visible, &visible_asset, 16)
            ])
            .unwrap(),
            vec![vec![0, 1]]
        );
    }

    #[test]
    #[ignore = "requires GPU and KFX_SPRITE_FIXTURE generated by sprite_fixture"]
    fn gpu_actual_legacy_sprites() {
        gpu_sprite_fixture("KFX_SPRITE_FIXTURE");
    }

    #[test]
    #[ignore = "requires GPU and KFX_SPRITE_COPY_FIXTURE generated by sprite_copy_fixture"]
    fn gpu_actual_legacy_sprite_copies() {
        gpu_sprite_fixture("KFX_SPRITE_COPY_FIXTURE");
    }

    fn gpu_sprite_fixture(variable: &str) {
        let path = std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"));
        let mut file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        let mut header = [0u8; 20];
        file.read_exact(&mut header).unwrap();
        assert_eq!(word(&header, 0), 0x3353464b);
        assert_eq!(word(&header, 4), 112);
        let count = word(&header, 1);
        let width = word(&header, 2);
        let height = word(&header, 3);
        let size = (width * height) as usize;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        eprintln!("sprite fixtures adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let renderer = crate::gpu::Renderer::new(device, queue).unwrap();
        let mut drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let target = drawing.create_target(width, height).unwrap();
        let initial: Vec<u8> = (0..size)
            .map(|i| (i * 19 + i / width as usize * 13) as u8)
            .collect();
        let initial_source = drawing
            .create_resource(&initial, width, height, width)
            .unwrap();
        let mut expected = vec![0; size];
        for fixture in 0..count {
            let mut bytes = [0; 112];
            file.read_exact(&mut bytes).unwrap();
            let mut length = [0; 4];
            file.read_exact(&mut length).unwrap();
            let mut source = vec![0; u32::from_le_bytes(length) as usize];
            file.read_exact(&mut source).unwrap();
            let source_handle = drawing.create_resource(&source, 1, 1, 1).unwrap();
            source.fill(19);
            let mut table_handle = 0;
            if word(&bytes, 2) != 0 {
                let mut table = vec![0; 65536];
                file.read_exact(&mut table).unwrap();
                table_handle = drawing.create_resource(&table, 256, 256, 256).unwrap();
                table.fill(23);
            }
            file.read_exact(&mut expected).unwrap();
            let command = Command {
                abi_version: word(&bytes, 0),
                kind: word(&bytes, 1),
                blend: word(&bytes, 2),
                colour: word(&bytes, 3),
                x: word(&bytes, 4) as i32,
                y: word(&bytes, 5) as i32,
                width: word(&bytes, 6),
                height: word(&bytes, 7),
                clip_x: word(&bytes, 8) as i32,
                clip_y: word(&bytes, 9) as i32,
                clip_width: word(&bytes, 10),
                clip_height: word(&bytes, 11),
                source: source_handle,
                table: table_handle,
                source_x: word(&bytes, 16),
                source_y: word(&bytes, 17),
                source_width: word(&bytes, 18),
                source_height: word(&bytes, 19),
                start_low: word(&bytes, 20),
                start_high: word(&bytes, 21),
                step_low: word(&bytes, 22),
                step_high: word(&bytes, 23),
                transparent: word(&bytes, 24),
                ..Default::default()
            };
            drawing
                .submit(
                    target,
                    &[
                        Command {
                            kind: IMAGE,
                            source: initial_source,
                            width,
                            height,
                            source_width: width,
                            source_height: height,
                            ..Default::default()
                        },
                        command,
                    ],
                )
                .unwrap();
            drawing.release_resource(source_handle).unwrap();
            if table_handle != 0 {
                drawing.release_resource(table_handle).unwrap();
            }
            let actual = drawing.readback(target).unwrap();
            if let Some(pixel) = actual.iter().zip(&expected).position(|(a, b)| a != b) {
                panic!(
                    "fixture {fixture} {command:?}: pixel ({},{}) GPU={} legacy={}",
                    pixel % width as usize,
                    pixel / width as usize,
                    actual[pixel],
                    expected[pixel]
                );
            }
        }
        let mut trailing = [0];
        assert_eq!(file.read(&mut trailing).unwrap(), 0);
        eprintln!("{count} exact actual-legacy sprite GPU fixtures passed");
    }
}
