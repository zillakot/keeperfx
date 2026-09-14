use keeperfx_frame_replay::draw::{Command, DrawRenderer, SPRITE};

const SIZE: u32 = 64;

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// An ordered sprite covering `[x, x + width) x [y, y + height)` of the target. The
/// clip rectangle is the whole target, as the emitter always sets it, so only the
/// scaling ranges bound the sprite. The artwork is a single run per source row, so the
/// kernel writes the run right to left and replicates it down every row of the sprite.
/// `gaps` leaves source columns uncovered, which makes the written pixels a strict
/// subset of the write rectangle.
fn sprite(
    drawing: &mut DrawRenderer,
    x: i32,
    width: u32,
    y: i32,
    height: u32,
    tint: u8,
    gaps: bool,
) -> Command {
    let (w, h) = (4usize, 1usize);
    let mut bytes = Vec::new();
    for column in 0..w {
        bytes.push(tint.wrapping_add(column as u8));
        // A gap has to precede the run: a zero inside one is an unterminated run.
        bytes.push(match column {
            0 if gaps => 0,
            column if column + 1 == w => 2,
            _ => 1,
        });
    }
    let base = width / w as u32;
    let mut start = x as u32;
    for column in 0..w {
        let span = if column + 1 == w {
            width - base * (w as u32 - 1)
        } else {
            base
        };
        bytes.extend(words(&[start, span]));
        start += span;
    }
    bytes.extend(words(&[y as u32, height]));
    bytes.extend(0..=255u8);
    let source = drawing.create_resource(&bytes, 1, 1, 1).unwrap();
    Command {
        kind: SPRITE,
        source,
        width: SIZE,
        height: SIZE,
        clip_x: 0,
        clip_y: 0,
        clip_width: SIZE,
        clip_height: SIZE,
        source_x: 9,
        source_y: u32::from(tint) % 4,
        source_width: w as u32,
        source_height: h as u32,
        ..Default::default()
    }
}

fn seeded(drawing: &mut DrawRenderer, width: u32, height: u32) -> u64 {
    let pixels: Vec<u8> = (0..(width * height) as usize)
        .map(|i| (i * 31 + i / width as usize * 7) as u8)
        .collect();
    let target = drawing.create_target(width, height).unwrap();
    let source = drawing
        .create_resource(&pixels, width, height, width)
        .unwrap();
    drawing
        .submit(
            target,
            &[Command {
                kind: keeperfx_frame_replay::draw::IMAGE,
                source,
                width,
                height,
                source_width: width,
                source_height: height,
                ..Default::default()
            }],
        )
        .unwrap();
    drawing.release_resource(source).unwrap();
    target
}

/// Replays the sprites one per call, which is one layer of one workgroup each, and then
/// as a single run that layers. Returns the layers the run formed.
fn parity(drawing: &mut DrawRenderer, commands: &[Command]) -> u64 {
    let serial = seeded(drawing, SIZE, SIZE);
    for command in commands {
        drawing
            .submit(serial, std::slice::from_ref(command))
            .unwrap();
    }
    let expected = drawing.readback(serial).unwrap();

    let layered = seeded(drawing, SIZE, SIZE);
    let before = drawing.counters();
    drawing.submit(layered, commands).unwrap();
    let after = drawing.counters();
    let actual = drawing.readback(layered).unwrap();

    assert_eq!(
        after.ordered_sprite_passes - before.ordered_sprite_passes,
        after.ordered_sprite_layers - before.ordered_sprite_layers,
        "one compute pass per layer"
    );
    assert_eq!(
        after.dispatches - before.dispatches,
        after.ordered_sprite_layers - before.ordered_sprite_layers,
        "one dispatch per layer"
    );
    if let Some(pixel) = expected.iter().zip(&actual).position(|(a, b)| a != b) {
        panic!(
            "pixel ({},{}) serial={} layered={}",
            pixel % SIZE as usize,
            pixel / SIZE as usize,
            expected[pixel],
            actual[pixel]
        );
    }
    after.ordered_sprite_layers - before.ordered_sprite_layers
}

/// The emitter gives every ordered sprite the whole drawing window as its clip, so the
/// scaling ranges are what separate these three.
#[test]
#[ignore = "requires a Metal adapter"]
fn disjoint_ordered_sprites_share_one_layer() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let commands: Vec<_> = (0..3)
        .map(|i| sprite(&mut drawing, 4 + 20 * i, 12, 4, 12, 40 + 9 * i as u8, false))
        .collect();
    assert_eq!(parity(&mut drawing, &commands), 1);
}

#[test]
#[ignore = "requires a Metal adapter"]
fn overlapping_ordered_sprites_keep_a_layer_each() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let commands: Vec<_> = (0..3)
        .map(|i| sprite(&mut drawing, 8, 16, 8, 16, 60 + 11 * i as u8, false))
        .collect();
    assert_eq!(parity(&mut drawing, &commands), 3);
}

/// Rectangles that share only an edge are disjoint and layer together. The sprite that
/// straddles both opens the next layer, which the one below it joins, and the pixels
/// still match the serial replay.
#[test]
#[ignore = "requires a Metal adapter"]
fn edge_contact_layers_and_real_overlap_does_not() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let left = sprite(&mut drawing, 9, 12, 4, 10, 33, false);
    let touching = sprite(&mut drawing, 22, 12, 4, 10, 71, false);
    let straddling = sprite(&mut drawing, 14, 12, 4, 10, 105, false);
    let below = sprite(&mut drawing, 9, 12, 30, 10, 143, false);
    assert_eq!(
        parity(&mut drawing, &[left, touching, straddling, below]),
        2,
        "edge contact keeps one layer; the straddling sprite opens the second"
    );
}

/// Uncovered source columns leave the written pixels a strict subset of the rectangle,
/// and per-sprite `source_y` alignments differ inside one layer.
#[test]
#[ignore = "requires a Metal adapter"]
fn sparse_sprites_and_mixed_alignments_layer_together() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let commands: Vec<_> = (0..4)
        .map(|i| sprite(&mut drawing, 2 + 15 * i, 13, 6, 20, 17 + i as u8, true))
        .collect();
    let alignments: Vec<_> = commands.iter().map(|c| c.source_y).collect();
    assert_eq!(alignments, vec![1, 2, 3, 0]);
    assert_eq!(parity(&mut drawing, &commands), 1);
}

/// A view is an offset alias of the root, so a sprite issued against it writes inside
/// the view only and layers in the view's own space.
#[test]
#[ignore = "requires a Metal adapter"]
fn sprites_clipped_by_a_view_layer_in_view_space() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let root = seeded(&mut drawing, SIZE, SIZE);
    let view = drawing.create_target_view(root, 10, 8, 40, 32).unwrap();
    let commands: Vec<_> = (0..2)
        .map(|i| {
            let mut command = sprite(
                &mut drawing,
                2 + 20 * i,
                14,
                3,
                20,
                90 + 13 * i as u8,
                false,
            );
            command.width = 40;
            command.height = 32;
            command.clip_width = 40;
            command.clip_height = 32;
            command
        })
        .collect();

    let reference = drawing.readback(root).unwrap();
    let before = drawing.counters();
    drawing.submit(view, &commands).unwrap();
    let layers = drawing.counters().ordered_sprite_layers - before.ordered_sprite_layers;
    let actual = drawing.readback(root).unwrap();
    assert_eq!(layers, 1);

    for index in 0..(SIZE * SIZE) as usize {
        let (x, y) = (index as u32 % SIZE, index as u32 / SIZE);
        if !(10..50).contains(&x) || !(8..40).contains(&y) {
            assert_eq!(
                reference[index], actual[index],
                "pixel ({x},{y}) outside the view changed"
            );
        }
    }
    assert!(
        reference != actual,
        "the sprites must have written inside the view"
    );
}

/// Queued ordered sprites with no record between them reach the layering path as one
/// run, so the frame stream forms the same layers the direct call does.
#[test]
#[ignore = "requires a Metal adapter"]
fn queued_ordered_sprites_coalesce_into_the_same_layers() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let commands: Vec<_> = (0..3)
        .map(|i| sprite(&mut drawing, 4 + 20 * i, 12, 4, 12, 40 + 9 * i as u8, false))
        .collect();
    let direct = seeded(&mut drawing, SIZE, SIZE);
    drawing.submit(direct, &commands).unwrap();
    let expected = drawing.readback(direct).unwrap();

    let root = seeded(&mut drawing, SIZE, SIZE);
    drawing.frame_begin(root).unwrap();
    let before = drawing.counters();
    for command in &commands {
        drawing.submit(root, std::slice::from_ref(command)).unwrap();
    }
    drawing.frame_end().unwrap();
    let layers = drawing.counters().ordered_sprite_layers - before.ordered_sprite_layers;
    assert_eq!(layers, 1, "the frame's serial routes merged into one run");
    assert_eq!(expected, drawing.readback(root).unwrap());
}

/// A row copy spans the run plus the pixel left of it, so two sprites whose extents are
/// merely adjacent still overlap. A rectangle that stopped at the run would put these
/// two in one layer and race on the shared column.
#[test]
#[ignore = "requires a Metal adapter"]
fn adjacent_sprite_extents_do_not_share_a_layer() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let left = sprite(&mut drawing, 8, 12, 4, 10, 51, false);
    let right = sprite(&mut drawing, 20, 12, 4, 10, 97, false);
    assert_eq!(parity(&mut drawing, &[left, right]), 2);
}
