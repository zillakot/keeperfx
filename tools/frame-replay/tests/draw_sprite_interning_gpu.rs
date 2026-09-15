//! Sprite artwork held resident by the name its emitter gave it: the same expanded
//! pixels reach the kernel whether they arrive once under a key or afresh with every
//! command, and the arena stops paying for them.

use keeperfx_frame_replay::draw::arena_kinds::SPRITE_KIND;
use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, SPRITE};

const ARTWORK_KEY: u32 = 3;
const TARGET: u32 = 64;
const W: usize = 9;
const H: usize = 7;

fn drawing(binding: u64) -> Option<DrawRenderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: binding,
            ..Default::default()
        },
        ..Default::default()
    }))
    .ok()?;
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).ok()?;
    DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).ok()
}

/// Index/coverage pairs for one opaque run per row, `2` marking the run's last pixel
/// exactly as the emitter's RLE walk does.
fn artwork(seed: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; 2 * W * H];
    for y in 0..H {
        for x in 0..W {
            bytes[2 * (y * W + x)] = seed.wrapping_add((y * W + x) as u8).wrapping_mul(7);
            bytes[2 * (y * W + x) + 1] = if x + 1 == W { 2 } else { 1 };
        }
    }
    bytes
}

/// Contiguous ascending start/count pairs, `scale` target columns and rows per source
/// pixel, which is the only part of a sprite asset that is genuinely per call.
fn ranges(x: u32, y: u32, scale: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 * (W + H));
    for (origin, count) in [(x, W), (y, H)] {
        for i in 0..count as u32 {
            bytes.extend((origin + i * scale).to_le_bytes());
            bytes.extend(scale.to_le_bytes());
        }
    }
    bytes
}

fn remap() -> Vec<u8> {
    (0..=255u8)
        .map(|i| i.wrapping_mul(13).wrapping_add(5))
        .collect()
}

fn sprite(source: u64, ranges: u64, remap: u64) -> Command {
    Command {
        kind: SPRITE,
        source,
        width: TARGET,
        height: TARGET,
        clip_width: TARGET,
        clip_height: TARGET,
        source_width: W as u32,
        source_height: H as u32,
        start_low: ranges as u32,
        start_high: (ranges >> 32) as u32,
        step_low: remap as u32,
        step_high: (remap >> 32) as u32,
        transparent: 256,
        ..Default::default()
    }
}

fn clear() -> Command {
    Command {
        kind: CLEAR,
        colour: 37,
        ..Default::default()
    }
}

/// The eight positions and four scales one sprite is drawn at, across three frames.
fn scene() -> Vec<(u32, u32, u32)> {
    let mut placements = Vec::new();
    for frame in 0..3u32 {
        for slot in 0..8u32 {
            let scale = 1 + (slot + frame) % 4;
            placements.push((
                (slot * 5 + frame) % (TARGET - W as u32 * scale),
                (slot * 3 + frame) % (TARGET - H as u32 * scale),
                scale,
            ));
        }
    }
    placements
}

#[test]
#[ignore = "requires GPU adapter"]
fn interned_artwork_draws_the_same_pixels_and_stops_being_uploaded() {
    let Some(mut drawing) = drawing(32 << 20) else {
        return;
    };
    let target = drawing.create_target(TARGET, TARGET).unwrap();
    let bytes = artwork(41);
    let table = remap();

    // A per-call resource for every command, which is what a sprite costs today.
    let mut expected = Vec::new();
    for &(x, y, scale) in &scene() {
        let mut combined = bytes.clone();
        combined.extend(ranges(x, y, scale));
        combined.extend(&table);
        let source = drawing.create_resource(&combined, 1, 1, 1).unwrap();
        drawing
            .submit(target, &[clear(), sprite(source, 0, 0)])
            .unwrap();
        expected.push(drawing.readback(target).unwrap());
        drawing.release_resource(source).unwrap();
    }
    assert!(
        expected[0].iter().any(|&pixel| pixel != 37),
        "the sprite must actually write"
    );

    // The same scene with the artwork and the remap named once.
    let (artwork_handle, previous) = drawing
        .create_resource_keyed((ARTWORK_KEY, 0, 0x5170), 1, &bytes, 1, 1, 1)
        .unwrap();
    assert_eq!(previous, 0);
    let remap_handle = drawing.create_resource(&table, 1, 1, 1).unwrap();
    let mut warm = None;
    for (index, &(x, y, scale)) in scene().iter().enumerate() {
        let range_handle = drawing
            .create_resource(&ranges(x, y, scale), 1, 1, 1)
            .unwrap();
        drawing
            .submit(
                target,
                &[clear(), sprite(artwork_handle, range_handle, remap_handle)],
            )
            .unwrap();
        assert_eq!(
            drawing.readback(target).unwrap(),
            expected[index],
            "interning changed a pixel at placement {index}"
        );
        drawing.release_resource(range_handle).unwrap();
        // Everything after the first command is steady state.
        if index == 0 {
            warm = Some(drawing.counters().arena_by_kind[SPRITE_KIND]);
        }
    }
    let warm = warm.unwrap();
    let steady = drawing.counters().arena_by_kind[SPRITE_KIND];
    let commands = (scene().len() - 1) as u64;
    assert_eq!(
        steady.hits - warm.hits,
        2 * commands,
        "artwork and remap must hit once per command"
    );
    assert_eq!(
        steady.misses - warm.misses,
        commands,
        "only the per-call ranges may miss"
    );
    assert_eq!(
        steady.source_bytes - warm.source_bytes,
        commands * 8 * (W + H) as u64,
        "the arena must pay for the ranges and nothing else"
    );
}

#[test]
#[ignore = "requires GPU adapter"]
fn evicted_artwork_comes_back_with_the_same_pixels() {
    let Some(mut drawing) = drawing(32 << 20) else {
        return;
    };
    let target = drawing.create_target(TARGET, TARGET).unwrap();
    let bytes = artwork(97);
    let table = remap();
    let (artwork_handle, _) = drawing
        .create_resource_keyed((ARTWORK_KEY, 0, 0x6a11), 1, &bytes, 1, 1, 1)
        .unwrap();
    let remap_handle = drawing.create_resource(&table, 1, 1, 1).unwrap();
    let range_handle = drawing.create_resource(&ranges(3, 5, 2), 1, 1, 1).unwrap();
    let batch = [clear(), sprite(artwork_handle, range_handle, remap_handle)];
    drawing.submit(target, &batch).unwrap();
    let expected = drawing.readback(target).unwrap();

    let stride = keeperfx_frame_replay::draw::assets::STRIDE as u32;
    let before = drawing.arena_counters();
    for i in 0..(160 / stride) {
        let bulk = drawing
            .create_resource(
                &(0..512 * 512u32).map(|v| (v + i) as u8).collect::<Vec<_>>(),
                512,
                512,
                512,
            )
            .unwrap();
        drawing
            .submit(
                target,
                &[Command {
                    kind: keeperfx_frame_replay::draw::IMAGE,
                    source: bulk,
                    width: 8,
                    height: 8,
                    source_width: 8,
                    source_height: 8,
                    ..Default::default()
                }],
            )
            .unwrap();
    }
    let flooded = drawing.arena_counters();
    assert!(flooded.evictions > before.evictions, "nothing was evicted");
    assert_eq!(flooded.overflows, 0);

    drawing.submit(target, &batch).unwrap();
    assert_eq!(
        drawing.readback(target).unwrap(),
        expected,
        "artwork re-uploaded after eviction must be the same artwork"
    );
    assert_eq!(drawing.arena_counters().overflows, 0);
}

/// The hazard the split exists for: the scaling ranges carry the sprite's position, so
/// two commands for one artwork in the same frame must keep their own ranges. Keyed on
/// the artwork alone this holds; keyed over the whole asset the second command would
/// have drawn at the first one's position.
#[test]
#[ignore = "requires GPU adapter"]
fn one_identity_at_two_positions_in_one_batch_keeps_both_positions() {
    let Some(mut drawing) = drawing(32 << 20) else {
        return;
    };
    let target = drawing.create_target(TARGET, TARGET).unwrap();
    let bytes = artwork(13);
    let table = remap();
    let places = [(2u32, 3u32, 1u32), (33, 41, 2)];

    let combined: Vec<_> = places
        .iter()
        .map(|&(x, y, scale)| {
            let mut asset = bytes.clone();
            asset.extend(ranges(x, y, scale));
            asset.extend(&table);
            drawing.create_resource(&asset, 1, 1, 1).unwrap()
        })
        .collect();
    let batch = [
        clear(),
        sprite(combined[0], 0, 0),
        sprite(combined[1], 0, 0),
    ];
    drawing.submit(target, &batch).unwrap();
    let expected = drawing.readback(target).unwrap();
    let painted = expected.iter().filter(|&&pixel| pixel != 37).count();
    assert!(
        painted >= 2 * W * H,
        "both placements must paint their own pixels, not one on top of the other"
    );

    let (art, _) = drawing
        .create_resource_keyed((ARTWORK_KEY, 0, 0x1d05), 1, &bytes, 1, 1, 1)
        .unwrap();
    let map = drawing.create_resource(&table, 1, 1, 1).unwrap();
    let split: Vec<_> = places
        .iter()
        .map(|&(x, y, scale)| {
            let handle = drawing
                .create_resource(&ranges(x, y, scale), 1, 1, 1)
                .unwrap();
            sprite(art, handle, map)
        })
        .collect();
    drawing
        .submit(target, &[clear(), split[0], split[1]])
        .unwrap();
    assert_eq!(drawing.readback(target).unwrap(), expected);
    // And in the other order, so neither command's ranges can be the other's.
    drawing
        .submit(target, &[clear(), split[1], split[0]])
        .unwrap();
    assert_eq!(drawing.readback(target).unwrap(), expected);
}
