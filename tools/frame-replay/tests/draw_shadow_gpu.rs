use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, TRIG};
fn drawing() -> DrawRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("shadow adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
fn word(bytes: &[u8], offset: &mut usize) -> u32 {
    let v = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    v
}
#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn actual_native_shadow_masks_and_triangles() {
    native_shadow_cases(false);
}

#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn queued_world_keeps_shadow_mask_and_cpu_checkpoint_order() {
    native_shadow_cases(true);
}

fn native_shadow_cases(queued: bool) {
    let path = std::env::var("KFX_SHADOW_FIXTURE").expect("KFX_SHADOW_FIXTURE is required");
    let bytes = std::fs::read(path).unwrap();
    let mut offset = 0;
    let count = word(&bytes, &mut offset);
    let mut draw = drawing();
    let target = draw.create_target(79, 61).unwrap();
    if queued {
        draw.frame_begin(target).unwrap();
    }
    let mut prior_pixel = 167;
    let table = draw
        .create_resource(&bytes[offset..offset + 81920], 256, 320, 256)
        .unwrap();
    offset += 81920;
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 167,
            ..Default::default()
        }],
    )
    .unwrap();
    for case in 0..count {
        let length = word(&bytes, &mut offset) as usize;
        let colour = word(&bytes, &mut offset);
        let mut asset = bytes[offset..offset + length].to_vec();
        offset += length;
        if case == 0 {
            let mut malformed = asset.clone();
            malformed[..4].fill(0);
            let invalid = draw.create_resource(&malformed, 1, 1, 1).unwrap();
            let before = draw.readback(target).unwrap();
            let mut untouched = vec![77; 65536];
            assert!(
                draw.submit_shadow(
                    target,
                    &Command {
                        kind: 11,
                        source: invalid,
                        table,
                        colour,
                        width: 79,
                        height: 61,
                        clip_width: 79,
                        clip_height: 61,
                        ..Default::default()
                    },
                    &mut untouched
                )
                .is_err()
            );
            assert_eq!(draw.readback(target).unwrap(), before);
            assert!(untouched.iter().all(|&v| v == 77));
            draw.release_resource(invalid).unwrap();
        }
        let source = draw.create_resource(&asset, 1, 1, 1).unwrap();
        asset.fill(123);
        let command = Command {
            kind: 11,
            source,
            table,
            colour,
            width: 79,
            height: 61,
            clip_width: 79,
            clip_height: 61,
            ..Default::default()
        };
        let before = draw.frame_counters().checkpoints;
        if queued {
            draw.submit(
                target,
                &[Command {
                    kind: keeperfx_frame_replay::draw::RECT,
                    colour: prior_pixel,
                    width: 1,
                    height: 1,
                    ..Default::default()
                }],
            )
            .unwrap();
        }
        let mut mirror = vec![77; 65536];
        draw.submit_shadow(target, &command, &mut mirror).unwrap();
        draw.release_resource(source).unwrap();
        if queued {
            assert_eq!(draw.frame_counters().checkpoints, before + 1);
        }
        if mirror != bytes[offset..offset + 65536] {
            let i = mirror
                .iter()
                .zip(&bytes[offset..])
                .position(|(a, b)| a != b)
                .unwrap();
            panic!(
                "mask case {case}, index {i}, actual {}, expected {}",
                mirror[i],
                bytes[offset + i]
            );
        }
        offset += 65536;
        let pixels = draw.readback(target).unwrap();
        assert_eq!(
            pixels,
            &bytes[offset..offset + 79 * 61],
            "shadow triangles case {case}"
        );
        prior_pixel = u32::from(bytes[offset]);
        offset += 79 * 61;
    }
    if queued {
        draw.frame_end().unwrap();
    }
    assert_eq!(offset, bytes.len());
    assert_eq!(draw.target_resource_counters().snapshots, u64::from(count));
    assert_eq!(
        draw.target_resource_counters().sampling_copy_bytes,
        u64::from(count) * 2 * 65536 * 4
    );
    eprintln!("{count} actual native shadow masks and 2 triangles each match");
}
#[test]
#[ignore = "requires GPU"]
fn target_triangle_validation_and_snapshot_lifetime() {
    let mut draw = drawing();
    let target = draw.create_target(8, 8).unwrap();
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 71,
            ..Default::default()
        }],
    )
    .unwrap();
    let mask = draw.create_target(256, 256).unwrap();
    draw.submit(
        mask,
        &[Command {
            kind: CLEAR,
            colour: 255,
            ..Default::default()
        }],
    )
    .unwrap();
    let snapshot = draw
        .create_target_snapshot(mask, 0, 0, 256, 256, 256)
        .unwrap();
    draw.release_target(mask).unwrap();
    let mut table = vec![0; 81920];
    for (i, b) in table.iter_mut().enumerate() {
        *b = (i + 19) as u8;
    }
    let table = draw.create_resource(&table, 256, 320, 256).unwrap();
    let vertices: [i32; 15] = [1, 1, 0, 0, 0, 7, 1, 0, 0, 0, 1, 7, 0, 0, 0];
    let geometry: Vec<_> = vertices.into_iter().flat_map(i32::to_le_bytes).collect();
    let source = draw.create_resource(&geometry, 1, 1, 1).unwrap();
    let c = Command {
        kind: TRIG,
        source,
        table,
        source_x: 10,
        source_y: 65536,
        source_width: 64,
        colour: 1,
        width: 8,
        height: 8,
        ..Default::default()
    };
    let initial = draw.readback(target).unwrap();
    assert!(
        draw.submit_target_triangles(
            target,
            &[
                c,
                Command {
                    source: u64::MAX,
                    ..c
                }
            ],
            snapshot
        )
        .is_err()
    );
    assert_eq!(draw.readback(target).unwrap(), initial);
    assert!(draw.submit_target_triangles(target, &[c], source).is_err());
    assert_eq!(draw.readback(target).unwrap(), initial);
    let before = draw.counters();
    draw.submit_target_triangles(target, &[c], snapshot)
        .unwrap();
    let after = draw.counters();
    assert_eq!(
        after.asset_upload_bytes - before.asset_upload_bytes,
        (60 + 81920) * 4
    );
    assert_eq!(after.readback_bytes - before.readback_bytes, 4);
    draw.release_target_snapshot(snapshot).unwrap();
    draw.release_resource(source).unwrap();
    draw.release_resource(table).unwrap();
    let result = draw.readback(target).unwrap();
    assert!(result.iter().any(|&p| p != 71));
    assert!(result.iter().all(|&p| p == 71 || p == 90));
}
