use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, timing};
use std::time::Instant;

#[test]
#[ignore = "requires a GPU adapter and KFX_WGPU_GPU_TIMING=2"]
fn serialized_boundary_attributes_submission_and_gpu_drain() {
    assert!(timing::serialized());
    let mut draw = DrawRenderer::headless().unwrap();
    let root = draw.create_target(32, 32).unwrap();
    draw.frame_begin(root).unwrap();
    draw.submit(
        root,
        &[Command {
            kind: CLEAR,
            colour: 19,
            ..Default::default()
        }],
    )
    .unwrap();
    let before = draw.counters();
    let start = Instant::now();
    draw.frame_flush().unwrap();
    let elapsed = start.elapsed().as_nanos() as u64;
    let after = draw.counters();
    let wait_ns = after.wait_ns - before.wait_ns;
    let submit_wait_ns = after.replay.replay_submit_wait_ns - before.replay.replay_submit_wait_ns;
    let total_ns = after.replay.total_ns() - before.replay.total_ns();
    let pack_ns = after.replay.replay_pack_ns - before.replay.replay_pack_ns;
    assert!(after.submits > before.submits);
    assert!(
        after.waits > before.waits,
        "serialized boundary must drain the GPU queue"
    );
    assert!(wait_ns > 0);
    assert!(
        submit_wait_ns >= wait_ns,
        "{submit_wait_ns} must include the {wait_ns} ns GPU drain"
    );
    assert!(after.replay.upload_queue_writes > before.replay.upload_queue_writes);
    assert_eq!(
        after.replay.upload_ring_overflows - before.replay.upload_ring_overflows,
        0
    );
    assert!(pack_ns > 0);
    assert!(pack_ns + submit_wait_ns <= total_ns);
    assert!(total_ns <= elapsed);
    assert!(total_ns as f64 >= elapsed as f64 * 0.95);
    draw.frame_end().unwrap();
    assert_eq!(draw.readback(root).unwrap(), vec![19; 32 * 32]);
}
