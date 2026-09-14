use keeperfx_frame_replay::draw::{
    BITMAP, CIRCLE_FILLED, CIRCLE_OUTLINE, CLEAR, Command, DrawRenderer, GPOLY_SPAN, IMAGE,
    MAP_VIEW, MOVIE, OPAQUE, RAW_IMAGE, RECT, SPRITE, TILED_IMAGE, TRIG,
};

struct Assets {
    image: u64,
    sprite: u64,
    texture: u64,
    shades: u64,
    tiles: u64,
    raw: u64,
    movie: u64,
    styles: u64,
    markers: u64,
    huge: u64,
    geometry: u64,
    fades: u64,
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn sprite_bytes(w: usize, h: usize, origin: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            bytes.push((y * w + x) as u8 + 3);
            bytes.push(1);
        }
    }
    for axis in [w, h] {
        for i in 0..axis {
            bytes.extend(words(&[origin + 2 * i as u32, 2]));
        }
    }
    bytes.extend((0..256).map(|k: u32| ((k * 7 + 40) % 256) as u8));
    bytes
}

fn huge_bytes() -> Vec<u8> {
    let mut bytes = words(&[1, 1, 32, 1]);
    bytes.extend(words(&[2, 1, 44, 1]));
    bytes.extend(words(&[1, 3, 77]));
    bytes.extend(words(&[2, 3, 88]));
    bytes
}

fn assets(drawing: &mut DrawRenderer) -> Assets {
    let image: Vec<u8> = (0..12).map(|i| 100 + i as u8).collect();
    let sprite = sprite_bytes(2, 2, 4);
    let texture: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
    let shades: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    let raw: Vec<u8> = (0..16).map(|i| 150 + i as u8).collect();
    let movie: Vec<u8> = (0..32).map(|i| 60 + i as u8).collect();
    let mut styles = words(&[10, 20]);
    styles.extend(words(&[257, 258]));
    styles.truncate(8);
    styles.extend((0..1280).map(|k: u32| (k % 199) as u8));
    let markers = words(&[0, 0, 1, 1]);
    let mut geometry = words(&[2, 2, 0, 0, 0]);
    geometry.extend(words(&[12, 3, 0, 0, 0]));
    geometry.extend(words(&[4, 12, 0, 0, 0]));
    let fades: Vec<u8> = (0..256 * 320).map(|i| (i % 256) as u8).collect();
    Assets {
        image: drawing.create_resource(&image, 4, 3, 4).unwrap(),
        sprite: drawing
            .create_resource(&sprite, sprite.len() as u32, 1, sprite.len() as u32)
            .unwrap(),
        texture: drawing.create_resource(&texture, 32, 32, 256).unwrap(),
        shades: drawing.create_resource(&shades, 256, 256, 256).unwrap(),
        tiles: drawing.create_resource(&raw, 4, 4, 4).unwrap(),
        raw: drawing.create_resource(&raw, 4, 4, 4).unwrap(),
        movie: drawing.create_resource(&movie, 8, 4, 8).unwrap(),
        styles: drawing
            .create_resource(&styles, styles.len() as u32, 1, styles.len() as u32)
            .unwrap(),
        markers: drawing.create_resource(&markers, 16, 1, 16).unwrap(),
        huge: {
            let bytes = huge_bytes();
            drawing
                .create_resource(&bytes, bytes.len() as u32, 1, bytes.len() as u32)
                .unwrap()
        },
        geometry: drawing.create_resource(&geometry, 60, 1, 60).unwrap(),
        fades: drawing.create_resource(&fades, 256, 320, 256).unwrap(),
    }
}

/// One command of every kind the raster stream carries, sized to the view it is
/// issued against, so each sampler is exercised at a nonzero view origin.
fn family(a: &Assets, width: u32, height: u32, tint: u32) -> Vec<Command> {
    vec![
        Command {
            kind: CLEAR,
            colour: tint,
            ..Default::default()
        },
        Command {
            kind: RECT,
            colour: tint + 1,
            x: 1,
            y: 1,
            width: 5,
            height: 4,
            ..Default::default()
        },
        Command {
            kind: IMAGE,
            source: a.image,
            x: 2,
            y: 3,
            width: 8,
            height: 6,
            source_width: 4,
            source_height: 3,
            ..Default::default()
        },
        Command {
            kind: GPOLY_SPAN,
            source: a.texture,
            table: a.shades,
            x: 3,
            y: 6,
            width: 9,
            height: 1,
            step_high: 1,
            transparent: OPAQUE,
            ..Default::default()
        },
        Command {
            kind: CIRCLE_FILLED,
            colour: tint + 2,
            x: 8,
            y: 8,
            width: 7,
            height: 7,
            source_width: 3,
            ..Default::default()
        },
        Command {
            kind: CIRCLE_OUTLINE,
            colour: tint + 3,
            x: 1,
            y: 8,
            width: 9,
            height: 9,
            source_width: 4,
            ..Default::default()
        },
        Command {
            kind: SPRITE,
            source: a.sprite,
            width,
            height,
            source_width: 2,
            source_height: 2,
            ..Default::default()
        },
        Command {
            kind: TILED_IMAGE,
            source: a.tiles,
            x: 5,
            y: 10,
            width: 11,
            height: 5,
            source_width: 4,
            source_height: 4,
            ..Default::default()
        },
        Command {
            kind: RAW_IMAGE,
            source: a.raw,
            width,
            height,
            source_width: 4,
            source_height: 4,
            step_low: width,
            step_high: height,
            ..Default::default()
        },
        Command {
            kind: MOVIE,
            source: a.movie,
            width,
            height,
            source_width: 8,
            source_height: 4,
            ..Default::default()
        },
        Command {
            kind: MAP_VIEW,
            source: a.styles,
            x: 1,
            y: 12,
            width: 8,
            height: 2,
            source_width: 4,
            source_height: 2,
            ..Default::default()
        },
        Command {
            kind: MAP_VIEW,
            source: a.markers,
            colour: tint + 4,
            width,
            height,
            source_x: 3,
            source_y: 2,
            start_low: 5,
            start_high: 5,
            step_low: 1,
            step_high: 1,
            ..Default::default()
        },
        Command {
            kind: BITMAP,
            source: a.huge,
            width,
            height,
            source_height: 2,
            ..Default::default()
        },
        Command {
            kind: TRIG,
            source: a.geometry,
            table: a.fades,
            colour: tint + 5,
            width,
            height,
            source_width: 64,
            transparent: OPAQUE,
            ..Default::default()
        },
    ]
}

/// Nested views inside a wide root, every sampler per view, with deliberate
/// cross-view overlap: the per-view dispatches and the one root-space stream
/// must produce identical pixels.
#[test]
#[ignore = "requires a Metal adapter"]
fn nested_views_rebase_onto_root_space() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let a = assets(&mut drawing);
    let mut roots = Vec::new();
    for _ in 0..2 {
        let root = drawing.create_target(71, 53).unwrap();
        let view = drawing.create_target_view(root, 7, 5, 47, 35).unwrap();
        let nested = drawing.create_target_view(view, 3, 2, 23, 17).unwrap();
        roots.push([root, view, nested]);
    }
    let sizes = [(71, 53), (47, 35), (23, 17)];
    let order = [0usize, 1, 2, 1, 0, 2];
    let batches = drawing.counters().batches;

    for (step, &which) in order.iter().enumerate() {
        let (width, height) = sizes[which];
        for command in family(&a, width, height, 11 * step as u32 + 1) {
            drawing.submit(roots[0][which], &[command]).unwrap();
        }
    }
    let separate = drawing.counters().batches - batches;

    drawing.frame_begin(roots[1][0]).unwrap();
    for (step, &which) in order.iter().enumerate() {
        let (width, height) = sizes[which];
        for command in family(&a, width, height, 11 * step as u32 + 1) {
            drawing.submit(roots[1][which], &[command]).unwrap();
        }
    }
    drawing.frame_end().unwrap();
    let streamed = drawing.counters().batches - batches - separate;

    let pixels = drawing.readback(roots[0][0]).unwrap();
    assert_eq!(
        pixels,
        drawing.readback(roots[1][0]).unwrap(),
        "root-space rebasing must not move a pixel"
    );
    let mut seen = pixels.clone();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 32, "the fixture must actually draw: {seen:?}");
    assert_eq!(streamed, 1, "the whole frame is one raster pass");
    assert!(separate >= order.len() as u64);
}

/// A second identical frame must reuse the binning scratch rather than grow it.
#[test]
#[ignore = "requires a Metal adapter"]
fn repeated_frames_stop_allocating_tile_lists() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let a = assets(&mut drawing);
    let root = drawing.create_target(71, 53).unwrap();
    let view = drawing.create_target_view(root, 7, 5, 47, 35).unwrap();
    let mut warm = 0;
    for frame in 0..4 {
        drawing.frame_begin(root).unwrap();
        for command in family(&a, 71, 53, 1) {
            drawing.submit(root, &[command]).unwrap();
        }
        for command in family(&a, 47, 35, 9) {
            drawing.submit(view, &[command]).unwrap();
        }
        drawing.frame_end().unwrap();
        if frame == 1 {
            warm = drawing.counters().tile_allocations;
        }
    }
    assert_eq!(
        drawing.counters().tile_allocations,
        warm,
        "a warmed tile index must not allocate again"
    );
    assert!(warm > 0);
}
