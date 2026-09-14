mod families;

use families::{
    alias_lens, assets, family, lens_command, minimap, minimap_command, ordered_sprite,
    terrain_triangle,
};
use keeperfx_frame_replay::draw::DrawRenderer;

struct Seed(u32);

impl Seed {
    fn next(&mut self, bound: u32) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 16) % bound
    }
}

enum Step {
    Raster(usize, usize),
    Ordered(usize),
    Lens(usize),
    Minimap(usize),
    Terrain(usize, i64),
}

fn plan(steps: usize) -> Vec<Step> {
    let mut seed = Seed(0x5eed_1234);
    let mut plan = Vec::new();
    for _ in 0..steps {
        let view = seed.next(3) as usize;
        let kind = seed.next(10);
        let raster = seed.next(14) as usize;
        plan.push(match kind {
            0 => Step::Ordered(view),
            1 => Step::Lens(view),
            2 => Step::Minimap(view),
            3 | 4 => Step::Terrain(view, i64::from(seed.next(48))),
            _ => Step::Raster(view, raster),
        });
    }
    plan
}

/// Raster passes the stream must cut: one per non-empty run of raster steps.
fn boundaries(plan: &[Step]) -> u64 {
    let mut passes = 0;
    let mut open = false;
    for step in plan {
        match step {
            Step::Raster(..) => open = true,
            _ => {
                passes += u64::from(open);
                open = false;
            }
        }
    }
    passes + u64::from(open)
}

/// One frame of every family interleaved across three views in a seeded order,
/// replayed from the same commands through the per-batch API and through the
/// single stream: the roots must hold identical pixels.
#[test]
#[ignore = "requires a Metal adapter"]
fn interleaved_families_keep_their_order() {
    let mut drawing = DrawRenderer::headless().unwrap();
    let a = assets(&mut drawing);
    let sizes = [(71u32, 53u32), (47, 35), (23, 17)];
    let lenses: Vec<_> = sizes
        .iter()
        .map(|&(w, h)| alias_lens(&mut drawing, w, h))
        .collect();
    let maps: Vec<_> = sizes
        .iter()
        .map(|&(w, h)| minimap(&mut drawing, w, h))
        .collect();
    let mut targets = Vec::new();
    for _ in 0..2 {
        let root = drawing.create_target(71, 53).unwrap();
        let view = drawing.create_target_view(root, 7, 5, 47, 35).unwrap();
        let nested = drawing.create_target_view(view, 3, 2, 23, 17).unwrap();
        targets.push([root, view, nested]);
    }
    let plan = plan(48);

    let run = |drawing: &mut DrawRenderer, into: &[u64; 3]| {
        for (index, step) in plan.iter().enumerate() {
            let tint = 7 * index as u32 + 1;
            match *step {
                Step::Raster(view, kind) => {
                    let (w, h) = sizes[view];
                    let commands = family(&a, w, h, tint % 200);
                    drawing.submit(into[view], &[commands[kind]]).unwrap();
                }
                Step::Ordered(view) => {
                    let (w, h) = sizes[view];
                    drawing
                        .submit(into[view], &[ordered_sprite(&a, w, h)])
                        .unwrap();
                }
                Step::Lens(view) => {
                    let (w, h) = sizes[view];
                    drawing
                        .submit(into[view], &[lens_command(lenses[view], w, h)])
                        .unwrap();
                }
                Step::Minimap(view) => {
                    let (w, h) = sizes[view];
                    drawing
                        .submit(into[view], &[minimap_command(maps[view], w, h)])
                        .unwrap();
                }
                Step::Terrain(view, shade) => {
                    drawing
                        .submit_triangles(into[view], &[terrain_triangle(&a, shade)])
                        .unwrap();
                }
            }
        }
    };

    let start = drawing.counters().batches;
    run(&mut drawing, &targets[0].clone());
    let separate = drawing.counters().batches - start;
    let before = drawing.counters();
    drawing.frame_begin(targets[1][0]).unwrap();
    run(&mut drawing, &targets[1].clone());
    drawing.frame_end().unwrap();
    let streamed = drawing.counters().batches - before.batches;

    let expected = drawing.readback(targets[0][0]).unwrap();
    assert_eq!(
        drawing.readback(targets[1][0]).unwrap(),
        expected,
        "the single stream must reproduce the per-batch pixel order exactly"
    );
    let mut seen = expected.clone();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 32, "the fixture must actually draw");

    let serials = plan
        .iter()
        .filter(|step| !matches!(step, Step::Raster(..)))
        .count() as u64;
    assert!(
        streamed <= 2 * serials + 1,
        "one raster pass per serial segment, not one per command: {streamed} for {serials} serials"
    );
    assert!(
        streamed < separate,
        "the stream must collapse the per-command batches: {streamed} against {separate}"
    );
    assert_eq!(
        streamed - serials,
        boundaries(&plan),
        "one raster pass per serial segment"
    );
    assert!(drawing.counters().tile_entries > 0);
}
