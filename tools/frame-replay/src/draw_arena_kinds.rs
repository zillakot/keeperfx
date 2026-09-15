use super::*;

pub const ARENA_KINDS: usize = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum ResourceKind {
    Sprite,
    OrderedSprite,
    Cursor,
    Trig,
    TerrainTile,
    TerrainFade,
    NativeTable,
    Minimap,
    Shadow,
    TargetTrigGeometry,
    TargetTrigTable,
    Image,
    RawImage,
    TiledImage,
    Movie,
    MapView,
    Bitmap,
    Lens,
    Other,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaKindCounters {
    pub bytes: u64,
    pub misses: u64,
    pub hits: u64,
    pub source_bytes: u64,
    pub distinct_lengths: u64,
    pub length_overflows: u64,
}

pub(super) fn source_kind(command: &Command, cursor: bool) -> ResourceKind {
    match command.kind {
        SPRITE if cursor => ResourceKind::Cursor,
        SPRITE if sprites::ordered(command) => ResourceKind::OrderedSprite,
        SPRITE => ResourceKind::Sprite,
        TRIG => ResourceKind::Trig,
        IMAGE => ResourceKind::Image,
        RAW_IMAGE => ResourceKind::RawImage,
        TILED_IMAGE => ResourceKind::TiledImage,
        MOVIE => ResourceKind::Movie,
        MAP_VIEW => ResourceKind::MapView,
        BITMAP => ResourceKind::Bitmap,
        LENS_EFFECT => ResourceKind::Lens,
        SHADOW => ResourceKind::Shadow,
        _ => ResourceKind::Other,
    }
}

pub(super) struct SourceLengths {
    values: [[usize; 64]; ARENA_KINDS],
    counts: [usize; ARENA_KINDS],
}

impl Default for SourceLengths {
    fn default() -> Self {
        Self {
            values: [[0; 64]; ARENA_KINDS],
            counts: [0; ARENA_KINDS],
        }
    }
}

impl SourceLengths {
    pub(super) fn begin_frame(&mut self) {
        self.counts.fill(0);
    }

    pub(super) fn record(
        &mut self,
        counters: &mut Counters,
        kind: ResourceKind,
        length: usize,
        hit: bool,
    ) {
        let index = kind as usize;
        let c = &mut counters.arena_by_kind[index];
        if hit {
            c.hits += 1;
        } else {
            c.misses += 1;
            c.source_bytes += length as u64;
            c.bytes += length as u64 * super::assets::STRIDE as u64;
            if kind == ResourceKind::Trig {
                counters.arena_trig_texture_source_bytes += length.saturating_sub(60) as u64;
            }
        }
        let count = &mut self.counts[index];
        let values = &mut self.values[index];
        if !values[..*count].contains(&length) {
            if *count == values.len() {
                c.length_overflows += 1;
            } else {
                values[*count] = length;
                *count += 1;
                c.distinct_lengths += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [ResourceKind; ARENA_KINDS] = [
        ResourceKind::Sprite,
        ResourceKind::OrderedSprite,
        ResourceKind::Cursor,
        ResourceKind::Trig,
        ResourceKind::TerrainTile,
        ResourceKind::TerrainFade,
        ResourceKind::NativeTable,
        ResourceKind::Minimap,
        ResourceKind::Shadow,
        ResourceKind::TargetTrigGeometry,
        ResourceKind::TargetTrigTable,
        ResourceKind::Image,
        ResourceKind::RawImage,
        ResourceKind::TiledImage,
        ResourceKind::Movie,
        ResourceKind::MapView,
        ResourceKind::Bitmap,
        ResourceKind::Lens,
        ResourceKind::Other,
    ];

    #[test]
    fn lengths_are_bounded_and_reset_without_resetting_totals() {
        let mut lengths = SourceLengths::default();
        let mut counters = Counters::default();
        for length in 0..65 {
            lengths.record(&mut counters, ResourceKind::Sprite, length, false);
        }
        lengths.record(&mut counters, ResourceKind::Sprite, 0, true);
        let c = counters.arena_by_kind[ResourceKind::Sprite as usize];
        assert_eq!(
            (c.misses, c.hits, c.distinct_lengths, c.length_overflows),
            (65, 1, 64, 1)
        );
        assert_eq!(c.source_bytes, (0..65).sum::<u64>());
        assert_eq!(
            c.bytes,
            c.source_bytes * super::super::assets::STRIDE as u64
        );
        lengths.begin_frame();
        lengths.record(&mut counters, ResourceKind::Sprite, 64, true);
        let next = counters.arena_by_kind[ResourceKind::Sprite as usize];
        assert_eq!(
            (
                next.misses,
                next.hits,
                next.distinct_lengths,
                next.length_overflows
            ),
            (65, 2, 65, 1)
        );
    }

    #[test]
    fn command_classification_is_explicit() {
        let mut c = Command {
            kind: SPRITE,
            ..Default::default()
        };
        assert_eq!(source_kind(&c, false), ResourceKind::Sprite);
        c.source_x = 8;
        assert_eq!(source_kind(&c, false), ResourceKind::OrderedSprite);
        assert_eq!(source_kind(&c, true), ResourceKind::Cursor);
        for (kind, expected) in [
            (TRIG, ResourceKind::Trig),
            (IMAGE, ResourceKind::Image),
            (RAW_IMAGE, ResourceKind::RawImage),
            (TILED_IMAGE, ResourceKind::TiledImage),
            (MOVIE, ResourceKind::Movie),
            (MAP_VIEW, ResourceKind::MapView),
            (BITMAP, ResourceKind::Bitmap),
            (LENS_EFFECT, ResourceKind::Lens),
            (SHADOW, ResourceKind::Shadow),
            (GPOLY_SPAN, ResourceKind::Other),
        ] {
            c.kind = kind;
            assert_eq!(source_kind(&c, false), expected);
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn pack_every_kind_and_conserve_arena_uploads() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut arena = arena::Arena::new(32 << 20);
        let mut counters = Counters::default();
        let mut packer = asset_packer(&device, &queue, &mut arena, &mut counters, 1, 32 << 20);
        for (index, kind) in KINDS.into_iter().enumerate() {
            let bytes = vec![index as u8; 60 + index];
            let id = index as u64 + 1;
            let offset = packer.offset(id, &bytes, kind).unwrap();
            assert_eq!(packer.offset(id, &bytes, kind).unwrap(), offset);
        }
        assert!(packer.finish().is_none());
        for (index, kind) in KINDS.into_iter().enumerate() {
            assert_eq!(kind as usize, index);
            let c = counters.arena_by_kind[index];
            assert_eq!(
                c,
                ArenaKindCounters {
                    bytes: (60 + index as u64) * 4,
                    source_bytes: 60 + index as u64,
                    hits: 1,
                    misses: 1,
                    distinct_lengths: 1,
                    length_overflows: 0,
                }
            );
        }
        assert_eq!(
            counters.arena_by_kind.iter().map(|c| c.bytes).sum::<u64>(),
            arena.counters().bytes_uploaded
        );
        assert_eq!(counters.asset_upload_bytes, arena.counters().bytes_uploaded);
        assert_eq!(counters.arena_trig_texture_source_bytes, 3);
        let before = counters;
        arena.lengths.begin_frame();
        let mut packer = asset_packer(&device, &queue, &mut arena, &mut counters, 2, 32 << 20);
        packer.offset(4, &[0; 63], ResourceKind::Trig).unwrap();
        packer
            .offset(4, &[0; 63], ResourceKind::NativeTable)
            .unwrap();
        assert!(packer.finish().is_none());
        assert_eq!(
            counters.arena_by_kind[3].bytes - before.arena_by_kind[3].bytes,
            63 * super::super::assets::STRIDE as u64
        );
        assert_eq!(
            counters.arena_by_kind[3].distinct_lengths - before.arena_by_kind[3].distinct_lengths,
            1
        );
        assert_eq!(
            counters.arena_by_kind[6].hits - before.arena_by_kind[6].hits,
            1
        );
        assert_eq!(
            counters.arena_by_kind.iter().map(|c| c.bytes).sum::<u64>(),
            arena.counters().bytes_uploaded
        );
        let mut small = arena::Arena::new(1024);
        let mut fallback = asset_packer(&device, &queue, &mut small, &mut counters, 2, 1024);
        fallback.offset(1, &[0; 5], ResourceKind::Other).unwrap();
        assert_eq!(fallback.finish().unwrap(), vec![0; 5]);
    }
}
