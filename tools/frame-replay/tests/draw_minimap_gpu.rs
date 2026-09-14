use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, MINIMAP};
fn drawing() -> DrawRenderer {
    drawing_limit(128 << 20)
}
fn drawing_limit(limit: u64) -> DrawRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("minimap adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: limit,
            max_buffer_size: limit,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
fn word(bytes: &[u8], offset: &mut usize) -> u32 {
    let n = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    n
}
#[test]
#[ignore = "requires GPU and actual native minimap fixture"]
fn actual_native_minimap_world_background_and_markers() {
    let bytes = std::fs::read(
        std::env::var("KFX_MINIMAP_FIXTURE").expect("KFX_MINIMAP_FIXTURE is required"),
    )
    .unwrap();
    let mut o = 0;
    let count = word(&bytes, &mut o);
    let width = word(&bytes, &mut o);
    let height = word(&bytes, &mut o);
    let size = (width * height) as usize;
    let mut draw = drawing();
    let target = draw.create_target(width, height).unwrap();
    let mut unsplit = drawing_limit(16 << 20);
    let reference = unsplit.create_target(width, height).unwrap();
    let mut captures = 0;
    for case in 0..count {
        let length = word(&bytes, &mut o) as usize;
        let mut asset = bytes[o..o + length].to_vec();
        o += length;
        let background = &bytes[o..o + size];
        let reference_initial = unsplit
            .create_resource(background, width, height, width)
            .unwrap();
        unsplit
            .submit(
                reference,
                &[Command {
                    kind: IMAGE,
                    source: reference_initial,
                    width,
                    height,
                    source_width: width,
                    source_height: height,
                    clip_width: width,
                    clip_height: height,
                    ..Default::default()
                }],
            )
            .unwrap();
        unsplit.release_resource(reference_initial).unwrap();
        let initial = draw
            .create_resource(&bytes[o..o + size], width, height, width)
            .unwrap();
        o += size;
        draw.submit(
            target,
            &[Command {
                kind: IMAGE,
                source: initial,
                width,
                height,
                source_width: width,
                source_height: height,
                clip_width: width,
                clip_height: height,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.release_resource(initial).unwrap();
        let source = draw.create_resource(&asset, 1, 1, 1).unwrap();
        let c = Command {
            kind: MINIMAP,
            source,
            width,
            height,
            clip_width: width,
            clip_height: height,
            ..Default::default()
        };
        let mode = asset[0];
        if mode == 4 {
            captures += 1;
        }
        if case < 5 {
            let before = draw.readback(target).unwrap();
            let mut malformed = asset.clone();
            malformed[20..24].copy_from_slice(&9999u32.to_le_bytes());
            let bad = draw.create_resource(&malformed, 1, 1, 1).unwrap();
            assert!(
                draw.submit(target, &[Command { source: bad, ..c }])
                    .is_err()
            );
            assert_eq!(draw.readback(target).unwrap(), before);
            draw.release_resource(bad).unwrap();
            assert!(
                draw.submit(
                    target,
                    &[
                        c,
                        Command {
                            kind: u32::MAX,
                            ..Default::default()
                        }
                    ]
                )
                .is_err()
            );
            assert_eq!(draw.readback(target).unwrap(), before);
        }
        let reference_source = unsplit.create_resource(&asset, 1, 1, 1).unwrap();
        unsplit
            .submit(
                reference,
                &[Command {
                    source: reference_source,
                    ..c
                }],
            )
            .unwrap();
        unsplit.release_resource(reference_source).unwrap();
        assert_eq!(
            unsplit.readback(reference).unwrap(),
            bytes[o..o + size],
            "unsplit case {case}"
        );
        asset.fill(71);
        draw.submit(target, &[c]).unwrap();
        draw.release_resource(source).unwrap();
        let actual = draw.readback(target).unwrap();
        if let Some(i) = actual
            .iter()
            .zip(&bytes[o..o + size])
            .position(|(a, b)| a != b)
        {
            panic!(
                "case {case} mode {mode} pixel ({},{}) actual {} expected {}",
                i % width as usize,
                i / width as usize,
                actual[i],
                bytes[o + i]
            );
        }
        o += size;
    }
    assert_eq!(o, bytes.len());
    assert_eq!(captures, 12);
    assert_eq!(draw.target_resource_counters().snapshots, captures);
    draw.release_target(target).unwrap();
}

fn payload(mode: u32, phase: u8, grid: u32) -> Vec<u8> {
    let mut h = [0u32; 24];
    h[0] = mode;
    h[1] = 64;
    h[2] = 64;
    h[3] = 4;
    h[4] = 4;
    h[5] = 32;
    h[7] = 32768;
    h[10] = grid;
    h[11] = grid;
    h[12] = 104;
    h[13] = 360;
    h[14] = 360 + 2 * (grid + 1).pow(2);
    h[15] = 11 * 38569;
    h[22] = 96;
    h[23] = 1;
    let mut bytes: Vec<u8> = h.into_iter().flat_map(u32::to_le_bytes).collect();
    bytes.resize(104, 0);
    if mode == 0 {
        bytes.extend(0..=255);
        bytes.resize(h[14] as usize, 0);
        bytes.resize((h[14] + h[15]) as usize, phase);
    }
    bytes
}

fn issue(draw: &mut DrawRenderer, target: u64, bytes: &[u8]) {
    let source = draw.create_resource(bytes, 1, 1, 1).unwrap();
    draw.submit(
        target,
        &[Command {
            kind: MINIMAP,
            source,
            ..Default::default()
        }],
    )
    .unwrap();
    draw.release_resource(source).unwrap();
}

fn segments(draw: &DrawRenderer) -> [u64; 4] {
    let c = draw.counters();
    [
        c.arena_minimap_prefix_bytes,
        c.arena_minimap_dictionary_bytes,
        c.arena_minimap_cells_bytes,
        c.arena_minimap_styles_bytes,
    ]
}

#[test]
#[ignore = "requires GPU adapter"]
fn recurring_segments_queued_replacement_recovery_and_nested_views() {
    let mut draw = drawing();
    let root = draw.create_target(80, 80).unwrap();
    let parent = draw.create_target_view(root, 3, 4, 72, 72).unwrap();
    let target = draw.create_target_view(parent, 2, 3, 64, 64).unwrap();
    let mut unsplit = drawing_limit(16 << 20);
    let reference_root = unsplit.create_target(80, 80).unwrap();
    let parent = unsplit
        .create_target_view(reference_root, 3, 4, 72, 72)
        .unwrap();
    let reference = unsplit.create_target_view(parent, 2, 3, 64, 64).unwrap();
    for frame in 0..24 {
        let before = segments(&draw);
        let counters = draw.counters();
        let mut bytes = payload(0, (frame % 4 + 11) as u8, 255);
        if frame >= 8 {
            bytes[32..36].copy_from_slice(&65536u32.to_le_bytes());
        }
        if frame >= 12 {
            bytes[360..362].copy_from_slice(&1u16.to_le_bytes());
        }
        if frame >= 16 {
            bytes[104] = 200;
        }
        if frame >= 20 {
            bytes[131432] = 71;
        }
        for (renderer, view, root_id) in [
            (&mut draw, target, root),
            (&mut unsplit, reference, reference_root),
        ] {
            renderer.frame_begin(root_id).unwrap();
            if frame == 0 || frame == 10 || frame == 11 {
                renderer
                    .submit(
                        view,
                        &[Command {
                            kind: keeperfx_frame_replay::draw::CLEAR,
                            colour: frame,
                            ..Default::default()
                        }],
                    )
                    .unwrap();
                issue(renderer, view, &payload(4, 0, 255));
            }
            issue(renderer, view, &bytes);
            renderer.frame_flush().unwrap();
            // Release of the public source must not release the private cached readers.
            issue(renderer, view, &bytes);
            renderer.frame_end().unwrap();
        }
        let after = segments(&draw);
        let delta: [u64; 4] = std::array::from_fn(|i| after[i] - before[i]);
        let capture = frame == 0 || frame == 10 || frame == 11;
        assert_eq!(
            delta,
            [
                416 * (2 + u64::from(capture)),
                if frame == 0 || frame == 16 { 1024 } else { 0 },
                if frame == 0 || frame == 12 { 524288 } else { 0 },
                if !(4..20).contains(&frame) {
                    1697036
                } else {
                    0
                }
            ],
            "frame {frame}"
        );
        let current = draw.counters();
        assert_eq!(
            current.dispatches - counters.dispatches,
            2 + u64::from(capture) * 2
        );
        if frame >= 4 {
            assert_eq!(current.submits - counters.submits, 1);
        }
        assert_eq!(after.iter().sum::<u64>(), current.arena_by_kind[7].bytes);
        assert_eq!(
            current.arena_by_kind.iter().map(|k| k.bytes).sum::<u64>(),
            draw.arena_counters().bytes_uploaded
        );
        assert!(current.minimap_cache_class_bytes <= 9 << 20);
        assert_eq!(
            draw.readback(root).unwrap(),
            unsplit.readback(reference_root).unwrap(),
            "frame {frame}"
        );
    }
    let before = segments(&draw);
    draw.frame_begin(root).unwrap();
    issue(&mut draw, target, &payload(0, 11, 255));
    draw.frame_flush().unwrap();
    draw.frame_abort().unwrap();
    let recovery = draw.arena_counters().misses_generation;
    let before_recovery = segments(&draw);
    draw.frame_begin(root).unwrap();
    issue(&mut draw, target, &payload(0, 11, 255));
    draw.frame_end().unwrap();
    assert_eq!(draw.arena_counters().misses_generation, recovery + 3);
    let after_recovery = segments(&draw);
    assert_eq!(
        std::array::from_fn::<_, 4, _>(|i| after_recovery[i] - before_recovery[i]),
        [416, 1024, 524288, 1697036]
    );
    assert!(segments(&draw)[1] > before[1]);
}

#[test]
#[ignore = "requires GPU adapter"]
fn oversized_cache_falls_back_and_open_encoder_pressure_preserves_readers() {
    let mut draw = drawing();
    let target = draw.create_target(64, 64).unwrap();
    issue(&mut draw, target, &payload(4, 0, 255));
    let oversized = payload(0, 17, 1024);
    let before = segments(&draw);
    issue(&mut draw, target, &oversized);
    assert_eq!(draw.counters().minimap_cache_class_bytes, 0);
    let after = segments(&draw);
    assert_eq!(
        after.iter().sum::<u64>() - before.iter().sum::<u64>(),
        oversized.len() as u64 * 4
    );
    assert_eq!(draw.readback(target).unwrap()[16 * 64 + 16], 17);
    let root = draw.create_target(256, 192).unwrap();
    let views: Vec<_> = (0..12)
        .map(|i| {
            draw.create_target_view(root, i % 4 * 64, i / 4 * 64, 64, 64)
                .unwrap()
        })
        .collect();
    draw.frame_begin(root).unwrap();
    for (version, view) in views.iter().enumerate() {
        let mut bytes = payload(0, version as u8, 255);
        bytes[360..362].copy_from_slice(&(version as u16).to_le_bytes());
        issue(&mut draw, *view, &bytes);
        draw.frame_flush().unwrap();
    }
    draw.frame_end().unwrap();
    assert_eq!(draw.arena_counters().overflows, 0);
    for (version, view) in views.iter().enumerate() {
        assert_eq!(draw.readback(*view).unwrap()[16 * 64 + 16], version as u8);
    }
}

#[test]
#[ignore = "requires GPU adapter"]
fn content_hits_still_upload_after_arena_eviction() {
    let mut draw = drawing_limit(32 << 20);
    let target = draw.create_target(64, 64).unwrap();
    issue(&mut draw, target, &payload(4, 0, 255));
    let terrain = payload(0, 19, 255);
    issue(&mut draw, target, &terrain);
    let mut resources = Vec::new();
    for _ in 0..8 {
        let source = draw.create_resource(&vec![9; 2 << 20], 1, 1, 1).unwrap();
        resources.push(source);
        draw.submit(
            target,
            &[Command {
                kind: IMAGE,
                source,
                width: 1,
                height: 1,
                source_width: 1,
                source_height: 1,
                clip_width: 64,
                clip_height: 64,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    assert!(draw.arena_counters().evictions > 0);
    for source in resources {
        draw.release_resource(source).unwrap();
    }
    let before = segments(&draw);
    let counters = draw.counters();
    let evicted = draw.arena_counters().misses_eviction;
    issue(&mut draw, target, &terrain);
    let after = segments(&draw);
    assert_eq!(
        std::array::from_fn::<_, 4, _>(|i| after[i] - before[i]),
        [416, 1024, 524288, 1697036]
    );
    assert_eq!(
        draw.counters().minimap_cells_hits,
        counters.minimap_cells_hits + 1
    );
    assert_eq!(
        draw.counters().minimap_styles_hits,
        counters.minimap_styles_hits + 1
    );
    assert_eq!(
        draw.counters().minimap_dictionary_hits,
        counters.minimap_dictionary_hits + 1
    );
    assert_eq!(draw.arena_counters().misses_eviction, evicted + 3);
    assert_eq!(draw.readback(target).unwrap()[16 * 64 + 16], 19);
}
