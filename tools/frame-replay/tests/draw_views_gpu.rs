mod families;

use families::{assets, family};
use keeperfx_frame_replay::draw::DrawRenderer;

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
