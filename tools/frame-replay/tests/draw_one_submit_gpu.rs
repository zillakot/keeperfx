mod families;

use families::{
    Assets, alias_lens, assets, family, lens_command, minimap, minimap_command, ordered_sprite,
    shadow_command, terrain_triangle,
};
use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, OPAQUE};

const WIDTH: u32 = 71;
const HEIGHT: u32 = 53;
const CURSOR: u32 = 5;

struct Scene {
    /// Kept alive: it owns the device error callback the drawing context reports through.
    _renderer: keeperfx_frame_replay::gpu::Renderer,
    draw: DrawRenderer,
    a: Assets,
    lens: u64,
    map: u64,
    root: u64,
    background: u64,
    pointer: u64,
    texture: wgpu::Texture,
}

impl Scene {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
        let mut draw = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let a = assets(&mut draw);
        let lens = alias_lens(&mut draw, WIDTH, HEIGHT);
        let map = minimap(&mut draw, WIDTH, HEIGHT);
        let root = draw.create_target(WIDTH, HEIGHT).unwrap();
        let background = draw.create_target(CURSOR, CURSOR).unwrap();
        let art = draw.create_target(CURSOR, CURSOR).unwrap();
        draw.submit(
            art,
            &[Command {
                kind: keeperfx_frame_replay::draw::CLEAR,
                colour: 211,
                ..Default::default()
            }],
        )
        .unwrap();
        let pointer = draw
            .create_target_snapshot(art, 0, 0, CURSOR, CURSOR, CURSOR)
            .unwrap();
        draw.frame_submit().unwrap();
        let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            _renderer: renderer,
            draw,
            a,
            lens,
            map,
            root,
            background,
            pointer,
            texture,
        }
    }

    /// Everything a presented frame records: the command stream with its asset deltas,
    /// the terrain prepare, a shadow mask and its triangles, an ordered sprite, the
    /// alias lens and the minimap, then the cursor backup, composition and restore
    /// around the palette pass. `present` omits the palette pass, as a skipped
    /// acquisition does.
    fn record(&mut self, present: bool) {
        let (root, background) = (self.root, self.background);
        self.draw.frame_begin(root).unwrap();
        for command in family(&self.a, WIDTH, HEIGHT, 11) {
            self.draw.submit(root, &[command]).unwrap();
        }
        self.draw
            .submit_triangles(root, &[terrain_triangle(&self.a, 17)])
            .unwrap();
        self.draw
            .submit(root, &[ordered_sprite(&self.a, WIDTH, HEIGHT)])
            .unwrap();
        self.draw
            .submit_shadow(root, &shadow_command(&self.a, WIDTH, HEIGHT))
            .unwrap();
        self.draw
            .submit(root, &[lens_command(self.lens, WIDTH, HEIGHT)])
            .unwrap();
        self.draw
            .submit(root, &[minimap_command(self.map, WIDTH, HEIGHT)])
            .unwrap();
        let part = self
            .draw
            .create_target_snapshot(root, 6, 5, CURSOR, CURSOR, CURSOR)
            .unwrap();
        self.draw
            .submit_target_images(background, &[sampled(part, CURSOR, CURSOR)])
            .unwrap();
        self.draw.release_target_snapshot(part).unwrap();
        let backup = self
            .draw
            .create_target_snapshot(background, 0, 0, CURSOR, CURSOR, CURSOR)
            .unwrap();
        self.draw
            .submit_target_images(
                root,
                &[Command {
                    x: 6,
                    y: 5,
                    transparent: 255,
                    ..sampled(self.pointer, CURSOR, CURSOR)
                }],
            )
            .unwrap();
        if present {
            let view = self.texture.create_view(&Default::default());
            self.draw
                .present_into(root, &palette(), WIDTH, HEIGHT, &view)
                .unwrap();
        }
        self.draw
            .submit_target_images(
                root,
                &[Command {
                    x: 6,
                    y: 5,
                    ..sampled(backup, CURSOR, CURSOR)
                }],
            )
            .unwrap();
        self.draw.release_target_snapshot(backup).unwrap();
    }

    /// One presented frame, ended the way `kfx_wgpu_present` ends it.
    fn present(&mut self) {
        self.record(true);
        self.draw.frame_submit().unwrap();
        self.draw.frame_abort().unwrap();
    }
}

fn sampled(source: u64, width: u32, height: u32) -> Command {
    Command {
        kind: IMAGE,
        source,
        width,
        height,
        source_width: width,
        source_height: height,
        clip_width: width,
        clip_height: height,
        transparent: OPAQUE,
        ..Default::default()
    }
}

fn palette() -> Vec<u8> {
    (0..256)
        .flat_map(|i| [i as u8, (i * 3) as u8, (i * 7) as u8, 255])
        .collect()
}

/// Design fixture 8. The structure, not the pixels: a presented frame is one command
/// buffer, it never blocks on the queue, and no flush inside it cuts the submission.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_production_frame_is_one_command_buffer() {
    let mut scene = Scene::new();
    // The first frames warm the arena, the stream ring and every pipeline.
    scene.record(true);
    scene.draw.frame_end().unwrap();
    scene.record(true);
    scene.draw.frame_end().unwrap();

    let before = scene.draw.counters();
    let before_frame = scene.draw.frame_counters();
    scene.record(true);
    assert_eq!(
        scene.draw.counters().submits,
        before.submits,
        "recording a frame must not reach the queue"
    );
    scene.draw.frame_submit().unwrap();
    let after = scene.draw.counters();
    let frame = scene.draw.frame_counters();

    assert_eq!(
        after.replay.upload_ring_overflows - before.replay.upload_ring_overflows,
        0
    );
    assert!(after.replay.upload_queue_writes > before.replay.upload_queue_writes);
    assert_eq!(
        after.replay.replay_buffers - before.replay.replay_buffers,
        0
    );
    assert_eq!(after.submits - before.submits, 1, "submits");
    assert_eq!(
        after.target_trig_table_bytes - before.target_trig_table_bytes,
        0
    );
    assert_eq!(
        after.target_trig_geometry_bytes - before.target_trig_geometry_bytes,
        120 * keeperfx_frame_replay::draw::assets::STRIDE as u64
    );
    assert_eq!(
        after.target_trig_table_hits - before.target_trig_table_hits,
        2
    );
    assert_eq!(after.target_trig_asset_buffers, 0);
    assert_eq!(after.preparer_buffers - before.preparer_buffers, 0);
    assert_eq!(
        after.preparer_buffer_bytes - before.preparer_buffer_bytes,
        0
    );
    assert_eq!(after.waits - before.waits, 0, "wait_count");
    assert_eq!(after.wait_ns - before.wait_ns, 0, "wait_ns");
    assert_eq!(
        frame.checkpoints - before_frame.checkpoints,
        0,
        "checkpoints"
    );
    assert_eq!(
        frame.status_stalls - before_frame.status_stalls,
        0,
        "status_stalls"
    );
    assert_eq!(
        after.readback_bytes, before.readback_bytes,
        "a production frame read pixels back"
    );
    assert!(
        after.dispatches > before.dispatches,
        "the frame drew nothing"
    );
    assert_eq!(
        scene.draw.arena_counters().overflows,
        0,
        "the arena overflowed inside a frame"
    );
    // Not the design's eventual bound of 8: the per-call command, tile and parameter
    // buffers of the serial routes need the byte-packed arena and persistent uniforms.
    assert!(
        after.buffers - before.buffers <= 64,
        "buffers {}",
        after.buffers - before.buffers
    );
    assert_eq!(scene.draw.frame_status().1, 0, "the frame raised a flag");
    scene.draw.frame_abort().unwrap();
}

/// `queue.write_buffer` applies before every pass of the submission it lands in, so an
/// asset uploaded mid-frame must not land in a region an already-recorded pass reads.
/// The snapshot forces a mid-frame replay, whose release then retires the region the
/// second half would otherwise be handed straight back.
#[test]
#[ignore = "requires a GPU adapter"]
fn an_asset_uploaded_mid_frame_is_read_by_the_later_pass_only() {
    let mut scene = Scene::new();
    let size = 8u32;
    let draw = &mut scene.draw;
    let root = draw.create_target(size * 2, size).unwrap();
    let left = draw.create_target_view(root, 0, 0, size, size).unwrap();
    let right = draw.create_target_view(root, size, 0, size, size).unwrap();
    let first: Vec<u8> = (0..size * size).map(|i| (i * 11 + 1) as u8).collect();
    let second: Vec<u8> = (0..size * size).map(|i| (i * 7 + 149) as u8).collect();

    let run = |draw: &mut DrawRenderer, per_batch: bool| -> Vec<u8> {
        let before = draw.create_resource(&first, size, size, size).unwrap();
        draw.frame_begin(root).unwrap();
        draw.submit(left, &[sampled(before, size, size)]).unwrap();
        // A release plus a create of the same size is the generation bump the bridge
        // makes for an asset mutated in place; the arena is free to reuse the region
        // as soon as the release drains, which the snapshot's flush makes happen here.
        draw.release_resource(before).unwrap();
        let taken = draw
            .create_target_snapshot(root, 0, 0, size, size, size)
            .unwrap();
        if per_batch {
            draw.frame_submit().unwrap();
        }
        let after = draw.create_resource(&second, size, size, size).unwrap();
        draw.submit(right, &[sampled(after, size, size)]).unwrap();
        draw.frame_end().unwrap();
        draw.release_target_snapshot(taken).unwrap();
        draw.release_resource(after).unwrap();
        draw.readback(root).unwrap()
    };
    let stepped = run(draw, true);
    let single = run(draw, false);
    assert_eq!(single, stepped, "a staged upload reached an earlier pass");
    let row = &single[..size as usize * 2];
    assert_eq!(row[..size as usize], first[..size as usize]);
    assert_eq!(row[size as usize..], second[..size as usize]);
    assert_eq!(draw.frame_status().1, 0);
    draw.frame_abort().unwrap();
}

/// A frame the stream ring cannot hold in one region still has to read the right
/// bytes: the second replay of a frame bumps past the first rather than over it.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_frame_replayed_twice_keeps_both_halves_of_the_stream() {
    let mut scene = Scene::new();
    let size = 8u32;
    let draw = &mut scene.draw;
    let root = draw.create_target(size * 2, size).unwrap();
    let left = draw.create_target_view(root, 0, 0, size, size).unwrap();
    let right = draw.create_target_view(root, size, 0, size, size).unwrap();
    let paint = |colour: u32| Command {
        kind: keeperfx_frame_replay::draw::CLEAR,
        colour,
        ..Default::default()
    };
    let run = |draw: &mut DrawRenderer, per_batch: bool| -> Vec<u8> {
        draw.frame_begin(root).unwrap();
        draw.submit(left, &[paint(31)]).unwrap();
        // A snapshot of the root aliases the frame, which replays the stream so far
        // into the open encoder; the remainder then persists a second region.
        let taken = draw
            .create_target_snapshot(root, 0, 0, size, size, size)
            .unwrap();
        if per_batch {
            draw.frame_submit().unwrap();
        }
        draw.submit(right, &[paint(97)]).unwrap();
        draw.frame_end().unwrap();
        draw.release_target_snapshot(taken).unwrap();
        draw.readback(root).unwrap()
    };
    let stepped = run(draw, true);
    let single = run(draw, false);
    assert_eq!(single, stepped, "the second region overwrote the first");
    let row = &single[..size as usize * 2];
    assert!(row[..size as usize].iter().all(|&v| v == 31), "left half");
    assert!(row[size as usize..].iter().all(|&v| v == 97), "right half");
    draw.frame_abort().unwrap();
}

/// Acquisition skipped and a discarded frame both have to leave the encoder in a
/// defined state: submitted exactly once, or dropped without reaching the queue.
#[test]
#[ignore = "requires a GPU adapter"]
fn an_unpresented_frame_is_neither_leaked_nor_submitted_twice() {
    let mut scene = Scene::new();
    scene.present();

    let before = scene.draw.counters().submits;
    scene.record(false);
    scene.draw.frame_submit().unwrap();
    assert_eq!(
        scene.draw.counters().submits - before,
        1,
        "a skipped acquisition must still submit the frame exactly once"
    );
    scene.draw.frame_submit().unwrap();
    assert_eq!(
        scene.draw.counters().submits - before,
        1,
        "an empty encoder must not be submitted again"
    );
    scene.draw.frame_abort().unwrap();
    let presented = scene.draw.readback(scene.root).unwrap();

    let before = scene.draw.counters().submits;
    scene.record(false);
    scene.draw.frame_discard();
    assert_eq!(
        scene.draw.counters().submits - before,
        0,
        "a discarded encoder must not reach the queue"
    );
    scene.draw.frame_abort().unwrap();
    scene.draw.frame_submit().unwrap();
    assert_eq!(
        scene.draw.counters().submits - before,
        0,
        "a discarded encoder was submitted after the fact"
    );
    assert_eq!(
        scene.draw.readback(scene.root).unwrap(),
        presented,
        "a discarded frame must leave the target as the last presented one did"
    );
    scene.draw.frame_abort().unwrap();
    scene.draw.check_status().unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shadow_table_versions_survive_serial_release_and_recovery() {
    let mut scene = Scene::new();
    let source = scene.a.shadow;
    let draw = &mut scene.draw;
    let root = draw.create_target(48, 16).unwrap();
    let views: Vec<_> = (0..3)
        .map(|i| draw.create_target_view(root, i * 16, 0, 16, 16).unwrap())
        .collect();
    let run = |draw: &mut DrawRenderer, stepped: bool| {
        let before = draw.counters();
        let mut tables: Vec<_> = [71, 93]
            .iter()
            .map(|&value| {
                draw.create_resource(&vec![value; 81920], 256, 320, 256)
                    .unwrap()
            })
            .collect();
        draw.shadow_scratch_reset().unwrap();
        draw.frame_submit().unwrap();
        draw.frame_begin(root).unwrap();
        draw.submit(
            root,
            &[Command {
                kind: keeperfx_frame_replay::draw::CLEAR,
                colour: 167,
                ..Default::default()
            }],
        )
        .unwrap();
        for i in 0..3 {
            if i == 2 {
                tables.push(
                    draw.create_resource(&vec![117; 81920], 256, 320, 256)
                        .unwrap(),
                );
            }
            draw.submit_shadow(
                views[i],
                &Command {
                    kind: keeperfx_frame_replay::draw::SHADOW,
                    source,
                    table: tables[i],
                    colour: 17,
                    width: 16,
                    height: 16,
                    clip_width: 16,
                    clip_height: 16,
                    ..Default::default()
                },
            )
            .unwrap();
            draw.release_resource(tables[i]).unwrap();
            if i == 1 {
                let snapshot = draw.create_target_snapshot(root, 0, 0, 16, 16, 16).unwrap();
                draw.release_target_snapshot(snapshot).unwrap();
            }
            if stepped {
                draw.frame_flush().unwrap();
                draw.frame_submit().unwrap();
            }
        }
        draw.frame_end().unwrap();
        let after = draw.counters();
        assert_eq!(
            after.target_trig_table_bytes - before.target_trig_table_bytes,
            3 * 81920 * keeperfx_frame_replay::draw::assets::STRIDE as u64
        );
        assert_eq!(
            after.target_trig_geometry_bytes - before.target_trig_geometry_bytes,
            3 * 120 * keeperfx_frame_replay::draw::assets::STRIDE as u64
        );
        assert_eq!(
            after.target_trig_table_hits - before.target_trig_table_hits,
            3
        );
        assert_eq!(
            after.target_trig_table_misses - before.target_trig_table_misses,
            3
        );
        assert_eq!(after.target_trig_asset_buffers, 0);
        draw.readback(root).unwrap()
    };
    let expected = run(draw, true);
    let actual = run(draw, false);
    assert_eq!(actual, expected);
    for (view, colour) in [71, 93, 117].into_iter().enumerate() {
        let pixels: Vec<_> = actual
            .as_chunks::<48>()
            .0
            .iter()
            .flat_map(|row| row[view * 16..view * 16 + 16].iter().copied())
            .collect();
        assert!(pixels.contains(&colour));
        assert!(pixels.iter().all(|&pixel| pixel == colour || pixel == 167));
    }
    let table = draw
        .create_resource(&vec![71; 81920], 256, 320, 256)
        .unwrap();
    let command = Command {
        kind: keeperfx_frame_replay::draw::SHADOW,
        source,
        table,
        colour: 17,
        width: 16,
        height: 16,
        clip_width: 16,
        clip_height: 16,
        ..Default::default()
    };
    draw.frame_begin(root).unwrap();
    draw.submit_shadow(views[0], &command).unwrap();
    draw.frame_flush().unwrap();
    let before = draw.arena_counters();
    draw.frame_abort().unwrap();
    draw.frame_begin(root).unwrap();
    draw.submit_shadow(views[0], &command).unwrap();
    draw.frame_end().unwrap();
    let after = draw.arena_counters();
    assert_eq!(after.misses_generation - before.misses_generation, 2);
    assert_eq!(
        after.miss_generation_bytes - before.miss_generation_bytes,
        (81920 + 164) * keeperfx_frame_replay::draw::assets::STRIDE as u64
    );
    assert_eq!(draw.readback(root).unwrap(), expected);
    assert_eq!(draw.frame_status().1, 0);
    draw.release_resource(table).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn all_serial_routes_match_immutable_inputs_with_tiny_and_shared_rings() {
    let mut expected = None;
    for budgets in [[0; 3], [512; 3], [2 << 20, 8 << 20, 1 << 20]] {
        let mut scene = Scene::new();
        scene.draw.configure_upload_rings(budgets, 512).unwrap();
        for _ in 0..3 {
            scene.present();
        }
        let pixels = scene.draw.readback(scene.root).unwrap();
        if let Some(expected) = &expected {
            assert_eq!(&pixels, expected);
        } else {
            expected = Some(pixels);
        }
        let c = scene.draw.counters().replay;
        if budgets[0] == 0 {
            assert!(c.upload_ring_overflows > 0);
        }
        if budgets[0] > 512 {
            assert_eq!(c.upload_ring_overflows, 0);
        }
    }
}
