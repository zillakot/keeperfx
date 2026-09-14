mod families;

use families::{Assets, assets, family, terrain_at};
use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, TriangleCommand};

const WIDTH: u32 = 67;
const HEIGHT: u32 = 51;
const CLEARED: u32 = 167;

fn scene() -> (DrawRenderer, Assets) {
    let mut drawing = DrawRenderer::headless().unwrap();
    let a = assets(&mut drawing);
    (drawing, a)
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

/// The same triangles through the counting sort and through an index that reaches every
/// tile: binning must change addressing only.
fn agrees_unbinned(drawing: &mut DrawRenderer, triangles: &[TriangleCommand]) -> Vec<u8> {
    let binned = drawing.create_target(WIDTH, HEIGHT).unwrap();
    let plain = drawing.create_target(WIDTH, HEIGHT).unwrap();
    clear(drawing, binned);
    drawing.submit_triangles(binned, triangles).unwrap();
    drawing.bin_records(false);
    clear(drawing, plain);
    drawing.submit_triangles(plain, triangles).unwrap();
    drawing.bin_records(true);
    let pixels = drawing.readback(binned).unwrap();
    assert_eq!(
        pixels,
        drawing.readback(plain).unwrap(),
        "binning moved a pixel"
    );
    pixels
}

fn tiles(triangles: &[TriangleCommand]) -> u64 {
    triangles
        .iter()
        .map(|triangle| {
            let axis = |extent: u32, of: fn(&keeperfx_frame_replay::gpoly::Vertex) -> i32| {
                let values: Vec<_> = triangle
                    .vertices
                    .iter()
                    .map(|vertex| i64::from(of(vertex)).clamp(0, i64::from(extent)) as u32)
                    .collect();
                (*values.iter().min().unwrap(), *values.iter().max().unwrap())
            };
            let (x0, x1) = axis(WIDTH, |vertex| vertex.x);
            let (y0, y1) = axis(HEIGHT, |vertex| vertex.y);
            if x0 == x1 || y0 == y1 {
                return 0;
            }
            u64::from((x1.div_ceil(16) - x0 / 16) * (y1.div_ceil(16) - y0 / 16))
        })
        .sum()
}

#[test]
#[ignore = "requires a Metal adapter"]
fn tile_boundaries_and_edges_agree_with_the_unbinned_raster() {
    let (mut drawing, a) = scene();
    let mut triangles = Vec::new();
    // Boxes that land exactly on tile multiples, on the view edges, and one-pixel
    // slivers straddling a boundary.
    for points in [
        [(0, 0), (16, 0), (0, 16)],
        [(16, 16), (32, 16), (16, 32)],
        [(15, 15), (17, 15), (15, 17)],
        [(0, 0), (WIDTH as i32, 0), (0, HEIGHT as i32)],
        [
            (WIDTH as i32 - 1, 0),
            (WIDTH as i32, 0),
            (WIDTH as i32 - 1, 16),
        ],
        [
            (0, HEIGHT as i32 - 1),
            (32, HEIGHT as i32 - 1),
            (0, HEIGHT as i32),
        ],
        [(15, 0), (16, 0), (15, HEIGHT as i32)],
        [(0, 31), (WIDTH as i32, 31), (0, 33)],
        [(-9, -7), (24, 5), (5, 24)],
        [(48, 32), (WIDTH as i32 + 20, 40), (50, HEIGHT as i32 + 20)],
    ] {
        triangles.push(terrain_at(&a, points, 3 << 16));
    }
    let pixels = agrees_unbinned(&mut drawing, &triangles);
    let mut seen = pixels.clone();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 1, "the boundary fixture drew nothing");
}

#[test]
#[ignore = "requires a Metal adapter"]
fn triangles_spanning_many_tiles_keep_the_closed_form_entry_count() {
    let (mut drawing, a) = scene();
    let triangles = [
        // Whole target, one full tile row, one full tile column.
        [(0, 0), (WIDTH as i32, 0), (0, HEIGHT as i32)],
        [(0, 20), (WIDTH as i32, 20), (0, 30)],
        [(20, 0), (30, 0), (20, HEIGHT as i32)],
    ]
    .map(|points| terrain_at(&a, points, 5 << 16));
    let before = drawing.counters().terrain_tile_entries;
    let binned = drawing.create_target(WIDTH, HEIGHT).unwrap();
    clear(&mut drawing, binned);
    drawing.submit_triangles(binned, &triangles).unwrap();
    assert_eq!(
        drawing.counters().terrain_tile_entries - before,
        tiles(&triangles),
        "terrain tile entries must match the closed form"
    );
    agrees_unbinned(&mut drawing, &triangles);
}

#[test]
#[ignore = "requires a Metal adapter"]
fn degenerate_and_off_view_triangles_touch_neither_arena_nor_index() {
    let (mut drawing, a) = scene();
    let target = drawing.create_target(WIDTH, HEIGHT).unwrap();
    clear(&mut drawing, target);
    let empty = drawing.readback(target).unwrap();
    // Zero height, repeated and collinear vertices, an edge delta outside the setup
    // kernel's domain, and geometry entirely outside the view in each axis. The arena
    // is sized by the clamped y extent alone, so an x-empty box still owns its rows.
    let shapes = [
        ([(3, 7), (29, 7), (17, 7)], 0, 0),
        ([(9, 9), (9, 9), (9, 9)], 0, 0),
        ([(4, 4), (12, 12), (20, 20)], 4, 16),
        ([(0, 0), (30000, 1), (1, 2)], 5, 2),
        ([(-80, 4), (-40, 9), (-60, 20)], 0, 16),
        ([(4, -80), (9, -40), (20, -60)], 0, 0),
        ([(4, -30), (9, -20), (20, -10)], 0, 0),
    ];
    for (points, entries, rows) in shapes {
        let triangles = [terrain_at(&a, points, 2 << 16)];
        let before = drawing.counters();
        drawing.submit_triangles(target, &triangles).unwrap();
        let after = drawing.counters();
        assert_eq!(
            drawing.readback(target).unwrap(),
            empty,
            "a degenerate triangle wrote pixels: {points:?}"
        );
        assert_eq!(
            after.terrain_tile_entries - before.terrain_tile_entries,
            entries,
            "wrong tile entries: {points:?}"
        );
        assert_eq!(
            after.prepared_row_words - before.prepared_row_words,
            8 * rows.max(1),
            "wrong arena rows: {points:?}"
        );
    }
    assert_eq!(drawing.frame_status().1, 0, "a degenerate raised a flag");
}

#[test]
#[ignore = "requires a Metal adapter"]
fn terrain_keeps_its_place_among_the_other_families() {
    let (mut drawing, a) = scene();
    let mut roots = Vec::new();
    for _ in 0..2 {
        let root = drawing.create_target(WIDTH, HEIGHT).unwrap();
        let view = drawing.create_target_view(root, 5, 3, 41, 33).unwrap();
        roots.push([root, view]);
    }
    let commands = family(&a, WIDTH, HEIGHT, 21);
    let inner = family(&a, 41, 33, 61);
    // A chain of overlapping terrain triangles spanning several tiles, interleaved with
    // raster records in both views, so the stream must keep the issue order across tiles.
    let steps: Vec<_> = (0..24)
        .map(|step| {
            let base = step * 2;
            (
                step % 3,
                terrain_at(
                    &a,
                    [
                        (base, base / 2),
                        (base + 29, base / 2 + 4),
                        (base + 6, base / 2 + 27),
                    ],
                    i64::from(step % 7 + 1) << 16,
                ),
                commands[(step as usize * 5 + 1) % commands.len()],
                inner[(step as usize * 3 + 2) % inner.len()],
            )
        })
        .collect();

    let run = |drawing: &mut DrawRenderer, into: &[u64; 2]| {
        clear(drawing, into[0]);
        for (which, triangle, outer, nested) in &steps {
            match which {
                0 => drawing.submit(into[0], &[*outer]).unwrap(),
                1 => drawing.submit(into[1], &[*nested]).unwrap(),
                _ => {}
            }
            drawing.submit_triangles(into[0], &[*triangle]).unwrap();
            drawing.submit_triangles(into[1], &[*triangle]).unwrap();
        }
    };

    run(&mut drawing, &roots[0].clone());
    let expected = drawing.readback(roots[0][0]).unwrap();
    drawing.frame_begin(roots[1][0]).unwrap();
    run(&mut drawing, &roots[1].clone());
    drawing.frame_end().unwrap();
    assert_eq!(
        drawing.readback(roots[1][0]).unwrap(),
        expected,
        "the stream must reproduce the per-batch terrain order exactly"
    );
    let mut seen = expected.clone();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 8, "the ordering fixture drew too little");
}

#[test]
#[ignore = "requires a Metal adapter"]
fn the_prepared_arena_holds_only_covered_rows() {
    let (mut drawing, a) = scene();
    let root = drawing.create_target(WIDTH, HEIGHT).unwrap();
    let tall: Vec<_> = (0..16)
        .map(|step| {
            terrain_at(
                &a,
                [(step, 0), (step + 20, 0), (step, HEIGHT as i32)],
                2 << 16,
            )
        })
        .collect();
    let low: Vec<_> = (0..16)
        .map(|step| {
            let top = 3 * HEIGHT as i32 / 4;
            terrain_at(
                &a,
                [(step, top), (step + 20, top), (step, HEIGHT as i32)],
                2 << 16,
            )
        })
        .collect();

    let rows = |drawing: &mut DrawRenderer, triangles: &[TriangleCommand]| {
        let before = drawing.counters().prepared_row_words;
        drawing.frame_begin(root).unwrap();
        clear(drawing, root);
        drawing.submit_triangles(root, triangles).unwrap();
        drawing.frame_end().unwrap();
        drawing.counters().prepared_row_words - before
    };

    let full = rows(&mut drawing, &tall);
    assert_eq!(
        full,
        8 * u64::from(HEIGHT) * tall.len() as u64,
        "a full-height population must occupy exactly its rows"
    );
    let quarter = rows(&mut drawing, &low);
    assert_eq!(
        quarter,
        8 * u64::from(HEIGHT - 3 * HEIGHT / 4) * low.len() as u64,
        "a bottom-quarter population must occupy a quarter of the rows"
    );
    assert!(
        quarter * 3 < full,
        "compression did not shrink the arena: {quarter} against {full}"
    );

    let allocations = drawing.counters().prepared_row_allocations;
    for _ in 0..10 {
        rows(&mut drawing, &tall);
    }
    assert_eq!(
        drawing.counters().prepared_row_allocations,
        allocations,
        "the prepared row arena must stop growing after warm-up"
    );
}
