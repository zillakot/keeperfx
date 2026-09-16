use super::*;

/// Validates the triangle and returns the destination box it can write inside,
/// half-open and in view space: the vertex bounding box. `trig_sample` returns the
/// destination unchanged for a row outside `[top.y, bottom.y)`, and inside a row the
/// left and right edges are truncated interpolations between two vertex x values, so
/// the span it accepts never leaves the vertex x range either.
pub(super) fn validate(
    c: &Command,
    source: &Resource,
    texture: Option<&Resource>,
    width: u32,
    height: u32,
) -> Result<[i64; 4]> {
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
    let geometry = match texture {
        Some(page) => {
            ensure!(
                page.bytes.len() == c.source_y as usize,
                "invalid triangle texture page"
            );
            60
        }
        None => 60 + c.source_y as usize,
    };
    ensure!(
        source.bytes.len() == geometry && c.source_y <= 65536,
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
    Ok([
        i64::from(min[0]),
        i64::from(min[1]),
        i64::from(max[0]),
        i64::from(max[1]),
    ])
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
            commands,
            &self.resources,
            ViewSpace::whole(width, height),
            limit,
            self.box_policy,
        )?;
        packer.finish();
        let entries = (
            self.counters.tile_entries,
            self.counters.tile_entries_by_kind,
        );
        self.tile_index.build(
            &mut self.counters,
            &words,
            &ViewSpace::table(&[ViewSpace::whole(width, height)]),
            &[commands.len()],
            (width, height),
            limit,
        )?;
        // The preflight only proves the limits; its index is never uploaded.
        (
            self.counters.tile_entries,
            self.counters.tile_entries_by_kind,
        ) = entries;
        Ok(())
    }
}
