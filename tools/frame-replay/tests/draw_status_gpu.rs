use anyhow::{Context, Result, ensure};
use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, TriangleCommand};
use keeperfx_frame_replay::gpoly::Vertex;

const FRAME_FLAG: u32 = 1;
const TERRAIN_SHADE: u32 = 1 << 2;
const TERRAIN_SPAN: u32 = 1 << 3;

fn triangle(source: u64, table: u64, shade: i64) -> TriangleCommand {
    TriangleCommand {
        abi_version: 1,
        reserved: 0,
        source,
        table,
        vertices: [(1, 1), (13, 2), (2, 13)].map(|(x, y)| Vertex {
            x,
            y,
            u: 0,
            v: 0,
            shade,
        }),
    }
}

struct Scene {
    draw: DrawRenderer,
    root: u64,
    source: u64,
    table: u64,
    clear: Command,
}

impl Scene {
    fn new() -> Result<Self> {
        let mut draw = DrawRenderer::headless()?;
        let root = draw.create_target(16, 16)?;
        let source = draw.create_resource(&vec![37; 8192], 32, 32, 256)?;
        let fade: Vec<u8> = (0..16384).map(|i| i as u8).collect();
        let table = draw.create_resource(&fade, 256, 64, 256)?;
        let clear = Command {
            kind: CLEAR,
            colour: 5,
            ..Default::default()
        };
        Ok(Self {
            draw,
            root,
            source,
            table,
            clear,
        })
    }

    fn frame(&mut self, shade: i64) -> Result<()> {
        self.draw.frame_begin(self.root)?;
        self.draw.submit(self.root, &[self.clear])?;
        self.draw
            .submit_triangles(self.root, &[triangle(self.source, self.table, shade)])?;
        self.draw.frame_end()
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_flagged_frame_presents_then_recovers_within_two_frames() -> Result<()> {
    let mut scene = Scene::new()?;
    scene.frame(0)?;
    let good = scene.draw.readback(scene.root)?;
    ensure!(good.contains(&37), "the probe triangle must draw");
    ensure!(
        scene.draw.frame_status() == (0, 0),
        "a valid frame raised a flag"
    );
    let waits = scene.draw.counters().waits;
    scene.draw.frame_begin(scene.root)?;
    scene.draw.submit(
        scene.root,
        &[Command {
            colour: 201,
            ..scene.clear
        }],
    )?;
    scene
        .draw
        .submit_triangles(scene.root, &[triangle(scene.source, scene.table, 70 << 16)])?;
    scene.draw.frame_end()?;
    // How soon the map completes is the driver's business; the contract is two frames.
    let mut seen = None;
    for _ in 0..=2u64 {
        let (index, flags) = scene.draw.frame_status();
        if flags != 0 {
            seen = Some((index, flags));
            break;
        }
        scene.frame(0)?;
    }
    let (_, flags) = seen.context("the flag never surfaced within two frames")?;
    ensure!(
        flags == FRAME_FLAG | TERRAIN_SHADE | TERRAIN_SPAN,
        "unexpected status flags {flags:#x}"
    );
    ensure!(
        scene.draw.counters().waits == waits,
        "reading the status blocked on the queue"
    );
    ensure!(
        scene.draw.frame_counters().invalid_frames == 1,
        "the flagged frame was not counted"
    );
    ensure!(
        scene.draw.frame_status().1 == 0,
        "the flag must clear once reported"
    );
    scene.draw.frame_abort()?;
    scene.frame(0)?;
    ensure!(
        scene.draw.readback(scene.root)? == good,
        "the redrawn frame must match the reference"
    );
    ensure!(
        scene.draw.frame_status().1 == 0,
        "recovery left the flag raised"
    );
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn the_flagged_frame_is_presented_as_drawn() -> Result<()> {
    let mut scene = Scene::new()?;
    scene.frame(0)?;
    let good = scene.draw.readback(scene.root)?;
    scene.draw.frame_begin(scene.root)?;
    scene.draw.submit(
        scene.root,
        &[Command {
            colour: 201,
            ..scene.clear
        }],
    )?;
    scene
        .draw
        .submit_triangles(scene.root, &[triangle(scene.source, scene.table, 70 << 16)])?;
    scene.draw.frame_end()?;
    ensure!(
        scene.draw.readback(scene.root)? == vec![201; 16 * 16],
        "the flagged frame must present the clear it drew with the triangle skipped"
    );
    scene.frame(0)?;
    ensure!(
        scene.draw.readback(scene.root)? == good,
        "the next frame must redraw over the flagged one"
    );
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_production_frame_performs_no_blocking_wait() -> Result<()> {
    let mut scene = Scene::new()?;
    scene.frame(0)?;
    scene.draw.readback(scene.root)?;
    let before = scene.draw.counters();
    let before_frame = scene.draw.frame_counters();
    for _ in 0..8 {
        scene.frame(0)?;
    }
    let after = scene.draw.counters();
    let frame = scene.draw.frame_counters();
    ensure!(
        (after.waits, after.wait_ns) == (before.waits, before.wait_ns),
        "a production frame blocked on the queue"
    );
    ensure!(
        frame.checkpoints - before_frame.checkpoints == 8,
        "one flush per frame"
    );
    ensure!(
        frame.validation_waits == 0 && frame.validation_bytes == 0,
        "the aggregate validation wait is gone"
    );
    ensure!(
        frame.checkpoint_copy_bytes == 0,
        "the flush still copies the target"
    );
    ensure!(
        frame.status_stalls == 0,
        "the status ring stalled during a steady frame sequence"
    );
    ensure!(
        after.readback_bytes == before.readback_bytes,
        "a production frame read pixels back"
    );
    ensure!(
        after.submits - before.submits <= 8 * 3,
        "a frame submits more than its batches plus the status publish"
    );
    Ok(())
}
