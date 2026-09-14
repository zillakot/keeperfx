mod families;

use families::words;
use keeperfx_frame_replay::draw::{BITMAP, CLEAR, Command, DrawRenderer, OPAQUE, SPRITE, TRIG};

const WIDTH: u32 = 67;
const HEIGHT: u32 = 51;
const CLEARED: u32 = 167;

fn scene() -> DrawRenderer {
    DrawRenderer::headless().unwrap()
}

fn blob(drawing: &mut DrawRenderer, bytes: &[u8]) -> u64 {
    let length = bytes.len() as u32;
    drawing.create_resource(bytes, length, 1, length).unwrap()
}

fn clear(drawing: &mut DrawRenderer, target: u64) {
    drawing
        .submit(
            target,
            &[Command {
                kind: CLEAR,
                colour: CLEARED,
                ..Default::default()
            }],
        )
        .unwrap();
}

/// The same records through the tight destination boxes and through the emitter's
/// whole-target bounds with binning off, which is what the raster did before the boxes
/// existed. A box that is not a superset of what the kernel writes moves a pixel here.
fn agrees_with_full_boxes(drawing: &mut DrawRenderer, commands: &[Command]) -> Vec<u8> {
    let tight = drawing.create_target(WIDTH, HEIGHT).unwrap();
    let full = drawing.create_target(WIDTH, HEIGHT).unwrap();
    clear(drawing, tight);
    drawing.submit(tight, commands).unwrap();
    drawing.tight_record_boxes(false);
    drawing.bin_records(false);
    clear(drawing, full);
    drawing.submit(full, commands).unwrap();
    drawing.bin_records(true);
    drawing.tight_record_boxes(true);
    let pixels = drawing.readback(tight).unwrap();
    assert_eq!(
        pixels,
        drawing.readback(full).unwrap(),
        "a tight bin box moved a pixel"
    );
    pixels
}

fn drew_something(pixels: &[u8]) {
    assert!(
        pixels.iter().any(|pixel| *pixel != CLEARED as u8),
        "the fixture drew nothing"
    );
}

/// Renders the same records with every derived box shrunk by one pixel on each side.
/// The parity fixture is only worth its run time if this differs.
fn with_eroded_boxes(drawing: &mut DrawRenderer, commands: &[Command]) -> Vec<u8> {
    let target = drawing.create_target(WIDTH, HEIGHT).unwrap();
    drawing.erode_record_boxes(1);
    clear(drawing, target);
    drawing.submit(target, commands).unwrap();
    drawing.erode_record_boxes(0);
    drawing.readback(target).unwrap()
}

/// `2*w*h` artwork bytes, then one contiguous `(start, length)` range per source column
/// and row, then the 256-byte remap the sampler ends in.
fn sprite_asset(
    w: usize,
    h: usize,
    xs: &[(u32, u32)],
    ys: &[(u32, u32)],
    ordered: bool,
) -> Vec<u8> {
    assert_eq!((xs.len(), ys.len()), (w, h));
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            bytes.push((y * w + x) as u8 + 3);
            bytes.push(if ordered && x + 1 == w { 2 } else { 1 });
        }
    }
    for axis in [xs, ys] {
        for (start, length) in axis {
            bytes.extend(words(&[*start, *length]));
        }
    }
    bytes.extend((0..256).map(|k: u32| ((k * 7 + 40) % 256) as u8));
    bytes
}

fn sprite(
    drawing: &mut DrawRenderer,
    xs: &[(u32, u32)],
    ys: &[(u32, u32)],
    options: u32,
) -> Command {
    let ordered = options & 8 != 0;
    let bytes = sprite_asset(xs.len(), ys.len(), xs, ys, ordered);
    Command {
        kind: SPRITE,
        source: blob(drawing, &bytes),
        width: WIDTH,
        height: HEIGHT,
        clip_width: WIDTH,
        clip_height: HEIGHT,
        source_x: options,
        source_width: xs.len() as u32,
        source_height: ys.len() as u32,
        ..Default::default()
    }
}

/// Contiguous ranges of `step` destination pixels per source column or row, starting at
/// `origin`, which is how the software scaler lays a sprite down.
fn scaled(origin: u32, count: usize, step: u32) -> Vec<(u32, u32)> {
    (0..count as u32)
        .map(|i| (origin + i * step, step))
        .collect()
}

fn triangle(
    drawing: &mut DrawRenderer,
    fades: u64,
    points: [(i32, i32); 3],
    colour: u32,
) -> Command {
    let mut bytes = Vec::new();
    for (x, y) in points {
        bytes.extend(words(&[x as u32, y as u32, 0, 0, 0]));
    }
    Command {
        kind: TRIG,
        source: drawing.create_resource(&bytes, 60, 1, 60).unwrap(),
        table: fades,
        colour,
        width: WIDTH,
        height: HEIGHT,
        clip_width: WIDTH,
        clip_height: HEIGHT,
        source_width: 64,
        transparent: OPAQUE,
        ..Default::default()
    }
}

/// One huge-bitmap pixel run: destination x, length and colour index.
type Run = (u32, u32, u32);

/// One huge-bitmap row group: `(y, rows, runs)`, laid out as the software adapter emits
/// it — a 16-byte row header per row group, then the pixel runs.
fn huge(drawing: &mut DrawRenderer, rows: &[(u32, u32, Vec<Run>)]) -> Command {
    let mut header = Vec::new();
    let mut records = Vec::new();
    let mut offset = rows.len() * 16;
    for (y, count, runs) in rows {
        header.extend(words(&[*y, *count, offset as u32, runs.len() as u32]));
        for (x, run, colour) in runs {
            records.extend(words(&[*x, *run, *colour]));
        }
        offset += runs.len() * 12;
    }
    header.extend(records);
    Command {
        kind: BITMAP,
        source: blob(drawing, &header),
        width: WIDTH,
        height: HEIGHT,
        clip_width: WIDTH,
        clip_height: HEIGHT,
        source_height: rows.len() as u32,
        transparent: OPAQUE,
        ..Default::default()
    }
}

/// A glyph: three colour words, then one bitplane bit per source pixel.
fn glyph(
    drawing: &mut DrawRenderer,
    sw: u32,
    sh: u32,
    at: (i32, i32),
    size: (u32, u32),
    shadow: bool,
) -> Command {
    let mut bytes = words(&[41, 256, 47]);
    let stride = sw.div_ceil(8) as usize;
    for row in 0..sh as usize {
        bytes.extend((0..stride).map(|byte| (row * 37 + byte * 11 + 0xa5) as u8));
    }
    Command {
        kind: BITMAP,
        source: blob(drawing, &bytes),
        width: WIDTH,
        height: HEIGHT,
        clip_width: WIDTH,
        clip_height: HEIGHT,
        source_x: if size == (sw, sh) { 1 } else { 2 },
        source_y: u32::from(shadow),
        source_width: sw,
        source_height: sh,
        start_low: at.0 as u32,
        start_high: at.1 as u32,
        step_low: size.0,
        step_high: size.1,
        transparent: OPAQUE,
        ..Default::default()
    }
}

fn fade_table(drawing: &mut DrawRenderer) -> u64 {
    let fades: Vec<u8> = (0..256 * 320).map(|i| (i % 256) as u8).collect();
    drawing.create_resource(&fades, 256, 320, 256).unwrap()
}

/// Tile edges, off-target placements, flips, scaling and empty ranges.
fn sprite_cases(drawing: &mut DrawRenderer) -> Vec<Command> {
    let mut cases = vec![
        sprite(drawing, &scaled(0, 4, 1), &scaled(0, 4, 1), 0),
        sprite(drawing, &scaled(16, 2, 1), &scaled(16, 2, 1), 0),
        sprite(drawing, &scaled(15, 2, 1), &scaled(15, 2, 1), 0),
        sprite(drawing, &scaled(14, 4, 5), &scaled(30, 3, 7), 0),
        sprite(
            drawing,
            &scaled(WIDTH - 1, 3, 1),
            &scaled(HEIGHT - 2, 3, 1),
            0,
        ),
        sprite(drawing, &scaled(WIDTH + 40, 2, 3), &scaled(4, 2, 3), 0),
        sprite(drawing, &scaled(4, 2, 3), &scaled(HEIGHT + 40, 2, 3), 0),
        sprite(drawing, &[(9, 0), (9, 0)], &[(9, 0), (9, 0)], 0),
        sprite(drawing, &[(0, 0), (0, 0)], &[(0, 0), (0, 0)], 0),
    ];
    for flip in [1, 2, 3] {
        cases.push(sprite(drawing, &scaled(12, 3, 4), &scaled(11, 3, 4), flip));
    }
    // Ordered sprites keep their own single-workgroup pass, so their box only has to
    // stay a superset; the left widening covers the `[leftmost-1, rightmost]` row copy.
    cases.push(sprite(drawing, &[(20, 3)], &[(20, 3)], 9));
    cases.push(sprite(drawing, &[(6, 0)], &[(6, 0)], 9));
    cases
}

fn triangle_cases(drawing: &mut DrawRenderer) -> Vec<Command> {
    let fades = fade_table(drawing);
    [
        ([(0, 0), (16, 0), (0, 16)], 61),
        ([(16, 16), (32, 16), (16, 32)], 62),
        ([(15, 15), (17, 15), (15, 17)], 63),
        ([(-9, -7), (24, 5), (5, 24)], 64),
        (
            [
                (WIDTH as i32 - 1, 0),
                (WIDTH as i32 + 30, 0),
                (WIDTH as i32 - 1, 20),
            ],
            65,
        ),
        (
            [
                (30, HEIGHT as i32 - 1),
                (50, HEIGHT as i32 - 1),
                (30, HEIGHT as i32 + 30),
            ],
            66,
        ),
        ([(9, 9), (9, 9), (9, 9)], 67),
        ([(3, 7), (29, 7), (17, 7)], 68),
        ([(-400, -300), (900, 4), (4, 900)], 69),
    ]
    .into_iter()
    .map(|(points, colour)| triangle(drawing, fades, points, colour))
    .collect()
}

fn bitmap_cases(drawing: &mut DrawRenderer) -> Vec<Command> {
    vec![
        huge(
            drawing,
            &[
                (1, 1, vec![(2, 3, 77), (16, 4, 88)]),
                (15, 2, vec![(15, 2, 99), (40, 5, 101)]),
                (HEIGHT - 2, 2, vec![(WIDTH - 3, 3, 103)]),
            ],
        ),
        // The whole target: the box stays the emitter's bounds and must agree with it.
        huge(drawing, &[(0, HEIGHT, vec![(0, WIDTH, 109)])]),
        huge(drawing, &[(20, 0, vec![])]),
        glyph(drawing, 9, 5, (3, 4), (9, 5), false),
        glyph(drawing, 9, 5, (16, 16), (9, 5), true),
        glyph(drawing, 9, 5, (-4, -3), (18, 15), true),
        glyph(
            drawing,
            9,
            5,
            (WIDTH as i32 - 2, HEIGHT as i32 - 2),
            (9, 5),
            true,
        ),
        glyph(drawing, 8, 4, (2, 30), (60, 40), true),
    ]
}

#[test]
#[ignore = "requires a Metal adapter"]
fn sprite_boxes_agree_with_the_whole_target_bounds() {
    let mut drawing = scene();
    let cases = sprite_cases(&mut drawing);
    for command in &cases {
        agrees_with_full_boxes(&mut drawing, std::slice::from_ref(command));
    }
    drew_something(&agrees_with_full_boxes(&mut drawing, &cases));
}

#[test]
#[ignore = "requires a Metal adapter"]
fn triangle_boxes_agree_with_the_whole_target_bounds() {
    let mut drawing = scene();
    let cases = triangle_cases(&mut drawing);
    for command in &cases {
        agrees_with_full_boxes(&mut drawing, std::slice::from_ref(command));
    }
    drew_something(&agrees_with_full_boxes(&mut drawing, &cases));
}

#[test]
#[ignore = "requires a Metal adapter"]
fn bitmap_boxes_agree_with_the_whole_target_bounds() {
    let mut drawing = scene();
    let cases = bitmap_cases(&mut drawing);
    for command in &cases {
        agrees_with_full_boxes(&mut drawing, std::slice::from_ref(command));
    }
    drew_something(&agrees_with_full_boxes(&mut drawing, &cases));
}

#[test]
#[ignore = "requires a Metal adapter"]
fn the_boxes_shrink_the_index_without_losing_a_record() {
    let mut drawing = scene();
    let tiles = u64::from(WIDTH.div_ceil(16) * HEIGHT.div_ceil(16));
    for build in [
        sprite_cases as fn(&mut DrawRenderer) -> Vec<Command>,
        triangle_cases,
        bitmap_cases,
    ] {
        let cases = build(&mut drawing);
        let target = drawing.create_target(WIDTH, HEIGHT).unwrap();
        let before = drawing.counters().tile_entries;
        clear(&mut drawing, target);
        drawing.submit(target, &cases).unwrap();
        let tight = drawing.counters().tile_entries - before;
        drawing.tight_record_boxes(false);
        let before = drawing.counters().tile_entries;
        drawing.submit(target, &cases).unwrap();
        let full = drawing.counters().tile_entries - before;
        drawing.tight_record_boxes(true);
        assert!(
            tight < full && full >= tiles,
            "tight {tight} against full {full} over {tiles} tiles"
        );
    }
}

#[test]
#[ignore = "requires a Metal adapter"]
fn an_undersized_box_is_caught() {
    let mut drawing = scene();
    for build in [
        sprite_cases as fn(&mut DrawRenderer) -> Vec<Command>,
        triangle_cases,
        bitmap_cases,
    ] {
        let cases = build(&mut drawing);
        let expected = agrees_with_full_boxes(&mut drawing, &cases);
        drew_something(&expected);
        assert!(
            with_eroded_boxes(&mut drawing, &cases) != expected,
            "shrinking every derived box by one pixel changed nothing, so the parity \
             fixture cannot see a box that is too small"
        );
    }
}

#[test]
#[ignore = "requires a Metal adapter"]
fn the_index_accounts_for_every_entry_by_kind() {
    let mut drawing = scene();
    let mut cases = sprite_cases(&mut drawing);
    cases.extend(triangle_cases(&mut drawing));
    cases.extend(bitmap_cases(&mut drawing));
    let target = drawing.create_target(WIDTH, HEIGHT).unwrap();
    let before = drawing.counters();
    clear(&mut drawing, target);
    drawing.submit(target, &cases).unwrap();
    let after = drawing.counters();
    let mut total = 0;
    let mut kinds = 0;
    for (a, b) in after
        .tile_entries_by_kind
        .iter()
        .zip(&before.tile_entries_by_kind)
    {
        total += a - b;
        kinds += u32::from(a > b);
    }
    assert_eq!(
        total,
        after.tile_entries - before.tile_entries,
        "the per-kind entries must sum to tile_entries"
    );
    assert!(kinds >= 4, "only {kinds} kinds reached the index");
}
