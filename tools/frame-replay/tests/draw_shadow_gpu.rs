use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, RECT, TRIG};
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

struct Case {
    asset: Vec<u8>,
    colour: u32,
    mask: Vec<u8>,
    pixels: Vec<u8>,
}

fn fixture() -> (Vec<u8>, Vec<Case>) {
    let path = std::env::var("KFX_SHADOW_FIXTURE").expect("KFX_SHADOW_FIXTURE is required");
    let bytes = std::fs::read(path).unwrap();
    let mut offset = 0;
    let count = word(&bytes, &mut offset);
    let table = bytes[offset..offset + 81920].to_vec();
    offset += 81920;
    let mut cases = Vec::new();
    for _ in 0..count {
        let length = word(&bytes, &mut offset) as usize;
        let colour = word(&bytes, &mut offset);
        let asset = bytes[offset..offset + length].to_vec();
        offset += length;
        let mask = bytes[offset..offset + 65536].to_vec();
        offset += 65536;
        let pixels = bytes[offset..offset + 79 * 61].to_vec();
        offset += 79 * 61;
        cases.push(Case {
            asset,
            colour,
            mask,
            pixels,
        });
    }
    assert_eq!(offset, bytes.len());
    (table, cases)
}

fn shadow(source: u64, table: u64, colour: u32) -> Command {
    Command {
        kind: 11,
        source,
        table,
        colour,
        width: 79,
        height: 61,
        clip_width: 79,
        clip_height: 61,
        ..Default::default()
    }
}

fn clear(draw: &mut DrawRenderer, target: u64) {
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 167,
            ..Default::default()
        }],
    )
    .unwrap();
}

#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn actual_native_shadow_masks_and_triangles() {
    let (table_bytes, cases) = fixture();
    let mut draw = drawing();
    let target = draw.create_target(79, 61).unwrap();
    let table = draw.create_resource(&table_bytes, 256, 320, 256).unwrap();
    clear(&mut draw, target);
    for (case, c) in cases.iter().enumerate() {
        if case == 0 {
            let mut malformed = c.asset.clone();
            malformed[..4].fill(0);
            let invalid = draw.create_resource(&malformed, 1, 1, 1).unwrap();
            let before = draw.readback(target).unwrap();
            let scratch = draw.shadow_scratch_read().unwrap();
            assert!(
                draw.submit_shadow(target, &shadow(invalid, table, c.colour))
                    .is_err()
            );
            assert_eq!(draw.readback(target).unwrap(), before);
            assert_eq!(draw.shadow_scratch_read().unwrap(), scratch);
            draw.release_resource(invalid).unwrap();
        }
        let mut asset = c.asset.clone();
        let source = draw.create_resource(&asset, 1, 1, 1).unwrap();
        asset.fill(123);
        draw.submit_shadow(target, &shadow(source, table, c.colour))
            .unwrap();
        draw.release_resource(source).unwrap();
        let mask = draw.shadow_scratch_read().unwrap();
        if mask != c.mask {
            let i = mask.iter().zip(&c.mask).position(|(a, b)| a != b).unwrap();
            panic!(
                "mask case {case}, index {i}, actual {}, expected {}",
                mask[i], c.mask[i]
            );
        }
        assert_eq!(
            draw.readback(target).unwrap(),
            c.pixels,
            "shadow triangles case {case}"
        );
    }
    assert_eq!(draw.target_resource_counters().snapshots, 0);
    assert_eq!(draw.target_resource_counters().sampling_copy_bytes, 0);
    eprintln!(
        "{} actual native shadow masks and 2 triangles each match",
        cases.len()
    );
}

/// The chain runs inside queued frames with unrelated commands between the masks and their
/// triangles, so slot reuse or a hoisted mask pass would show up as a pixel or scratch mismatch.
#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn a_shadow_that_bins_to_nothing_still_records_its_mask() {
    let (table_bytes, cases) = fixture();
    let mut draw = drawing();
    let target = draw.create_target(79, 61).unwrap();
    let table = draw.create_resource(&table_bytes, 256, 320, 256).unwrap();
    clear(&mut draw, target);
    let empty = draw.readback(target).unwrap();
    draw.shadow_scratch_reset().unwrap();
    // Collapse both vertex triples onto one point: the triangles' tight box is empty, so
    // they have nothing to dispatch over. The mask is built from the descriptor and the
    // RLE, which sit either side of the vertex block, so the resident chain must still
    // advance to exactly this case's mask.
    let mut asset = cases[0].asset.clone();
    asset[32..152].fill(0);
    let source = draw.create_resource(&asset, 1, 1, 1).unwrap();
    draw.submit_shadow(target, &shadow(source, table, cases[0].colour))
        .unwrap();
    draw.release_resource(source).unwrap();
    assert!(
        cases[0].mask.iter().any(|&v| v != 0),
        "the fixture mask is empty"
    );
    assert_eq!(
        draw.shadow_scratch_read().unwrap(),
        cases[0].mask,
        "a shadow whose triangles bin to nothing must still record its mask"
    );
    assert_eq!(
        draw.readback(target).unwrap(),
        empty,
        "degenerate shadow triangles must not write a pixel"
    );
    assert_eq!(
        draw.frame_status().1,
        0,
        "a degenerate shadow raised a flag"
    );
}

#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn interleaved_frame_shadow_chain() {
    let (table_bytes, cases) = fixture();
    let mut draw = drawing();
    let target = draw.create_target(79, 61).unwrap();
    let table = draw.create_resource(&table_bytes, 256, 320, 256).unwrap();
    clear(&mut draw, target);
    let mut seed = 0x971413u32;
    let mut random = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed >> 24
    };
    let mut checkpoints = 0;
    let mut frames = 0;
    let mut prior = 167;
    for chunk in cases.chunks(7) {
        draw.frame_begin(target).unwrap();
        frames += 1;
        let before = draw.counters();
        for c in chunk {
            let noise = random();
            draw.submit(
                target,
                &[Command {
                    kind: RECT,
                    colour: prior,
                    x: i32::from(noise as u8 % 79),
                    y: 0,
                    width: 1,
                    height: 1,
                    ..Default::default()
                }],
            )
            .unwrap();
            let source = draw.create_resource(&c.asset, 1, 1, 1).unwrap();
            draw.submit_shadow(target, &shadow(source, table, c.colour))
                .unwrap();
            draw.release_resource(source).unwrap();
            prior = u32::from(c.pixels[0]);
        }
        assert_eq!(
            draw.counters().waits,
            before.waits,
            "a queued shadow frame must not block"
        );
        assert_eq!(draw.frame_counters().checkpoints, checkpoints);
        draw.frame_end().unwrap();
        checkpoints = draw.frame_counters().checkpoints;
        assert_eq!(checkpoints, 0, "a flush is no longer a submission boundary");
        assert!(frames > 0);
        assert_eq!(draw.frame_counters().validation_waits, 0);
        let last = chunk.last().unwrap();
        assert_eq!(draw.shadow_scratch_read().unwrap(), last.mask);
        assert_eq!(draw.readback(target).unwrap(), last.pixels);
    }
    let carried = draw.shadow_scratch_read().unwrap();
    draw.shadow_scratch_reset().unwrap();
    let cleared = draw.shadow_scratch_read().unwrap();
    assert!(cleared.iter().all(|&v| v == 0));
    assert_ne!(carried, cleared, "the scratch must carry state to reset");
    draw.frame_begin(target).unwrap();
    let source = draw.create_resource(&cases[0].asset, 1, 1, 1).unwrap();
    draw.submit_shadow(target, &shadow(source, table, cases[0].colour))
        .unwrap();
    draw.release_resource(source).unwrap();
    draw.frame_end().unwrap();
    assert_eq!(
        draw.shadow_scratch_read().unwrap(),
        cases[0].mask,
        "a reset restarts the chain at the first fixture case"
    );
    assert_eq!(draw.target_resource_counters().snapshots, 0);
    eprintln!("{} shadows chained across {frames} frames", cases.len());
}

#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn verify_mode_scratch_read_is_blocking() {
    let (table_bytes, cases) = fixture();
    let mut draw = drawing();
    let target = draw.create_target(79, 61).unwrap();
    let table = draw.create_resource(&table_bytes, 256, 320, 256).unwrap();
    clear(&mut draw, target);
    let source = draw.create_resource(&cases[0].asset, 1, 1, 1).unwrap();
    draw.submit_shadow(target, &shadow(source, table, cases[0].colour))
        .unwrap();
    let before = draw.counters();
    assert_eq!(draw.shadow_scratch_read().unwrap(), cases[0].mask);
    let after = draw.counters();
    assert_eq!(after.waits, before.waits + 1);
    assert_eq!(after.readback_bytes, before.readback_bytes + 65536 * 4);
    assert!(after.wait_ns > before.wait_ns);
}

#[test]
#[ignore = "requires GPU"]
fn target_triangle_validation_and_resident_slots() {
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
    let mut table = vec![0; 81920];
    for (i, b) in table.iter_mut().enumerate() {
        *b = (i + 19) as u8;
    }
    let table = draw.create_resource(&table, 256, 320, 256).unwrap();
    let vertices: [i32; 15] = [1, 1, 0, 0, 0, 7, 1, 0, 0, 0, 1, 7, 0, 0, 0];
    let geometry: Vec<_> = vertices.into_iter().flat_map(i32::to_le_bytes).collect();
    let source = draw.create_resource(&geometry, 1, 1, 1).unwrap();
    let mut asset = Vec::new();
    let rle: Vec<u8> = (0..4).flat_map(|_| [4u8, 1, 1, 1, 1, 0]).collect();
    for n in [256u32, 256, 4, 4, 0, 0, 0, rle.len() as u32] {
        asset.extend(n.to_le_bytes());
    }
    asset.extend(&geometry);
    asset.extend(&geometry);
    asset.extend(&rle);
    let mask_source = draw.create_resource(&asset, 1, 1, 1).unwrap();
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
    draw.submit_shadow(
        target,
        &Command {
            kind: 11,
            source: mask_source,
            table,
            colour: 1,
            width: 8,
            height: 8,
            clip_width: 8,
            clip_height: 8,
            ..Default::default()
        },
    )
    .unwrap();
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 71,
            ..Default::default()
        }],
    )
    .unwrap();
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
            0,
            None
        )
        .is_err()
    );
    assert_eq!(draw.readback(target).unwrap(), initial);
    assert!(draw.submit_target_triangles(target, &[c], 9, None).is_err());
    assert_eq!(draw.readback(target).unwrap(), initial);
    let before = draw.counters();
    draw.submit_target_triangles(target, &[c], 0, None).unwrap();
    let after = draw.counters();
    assert_eq!(
        after.asset_upload_bytes - before.asset_upload_bytes,
        (60 + 81920) * 4
    );
    assert_eq!(after.readback_bytes, before.readback_bytes);
    draw.release_resource(mask_source).unwrap();
    draw.release_resource(source).unwrap();
    draw.release_resource(table).unwrap();
    let result = draw.readback(target).unwrap();
    assert!(result.iter().any(|&p| p != 71));
    assert!(result.iter().all(|&p| p == 71 || p == 90));
}

/// The whole chain in one frame and one encoder against the same chain cut into one
/// submission per shadow. Slot reuse rests on pass order inside the encoder, so mask
/// *i+2* overwriting the slot `TRIG` *i* read must still be a barrier, not a race.
#[test]
#[ignore = "requires GPU and native shadow fixture"]
fn the_whole_chain_in_one_encoder_matches_the_per_submit_replay() {
    let (table_bytes, cases) = fixture();
    let mut draw = drawing();
    let table = draw.create_resource(&table_bytes, 256, 320, 256).unwrap();
    let reset_at = cases.len() / 2;
    let mut run = |per_submit: bool| -> (Vec<u8>, Vec<u8>, u64) {
        let target = draw.create_target(79, 61).unwrap();
        draw.shadow_scratch_reset().unwrap();
        draw.frame_submit().unwrap();
        let before = draw.counters().submits;
        draw.frame_begin(target).unwrap();
        clear(&mut draw, target);
        for (at, c) in cases.iter().enumerate() {
            if at == reset_at {
                // A reset records straight into the encoder while shadows are queued,
                // so the queued half has to be replayed before it or the clear would
                // land ahead of the masks it is meant to follow.
                draw.frame_flush().unwrap();
                draw.shadow_scratch_reset().unwrap();
            }
            let source = draw.create_resource(&c.asset, 1, 1, 1).unwrap();
            draw.submit_shadow(target, &shadow(source, table, c.colour))
                .unwrap();
            draw.release_resource(source).unwrap();
            if per_submit {
                draw.frame_flush().unwrap();
                draw.frame_submit().unwrap();
            }
        }
        draw.frame_end().unwrap();
        let submits = draw.counters().submits - before;
        let scratch = draw.shadow_scratch_read().unwrap();
        let pixels = draw.readback(target).unwrap();
        draw.frame_abort().unwrap();
        draw.release_target(target).unwrap();
        (scratch, pixels, submits)
    };
    let (stepped_scratch, stepped_pixels, stepped_submits) = run(true);
    let (single_scratch, single_pixels, single_submits) = run(false);
    assert_eq!(single_scratch, stepped_scratch, "the chain diverged");
    assert_eq!(single_pixels, stepped_pixels, "the triangles diverged");
    assert!(
        stepped_submits > single_submits,
        "the per-submit replay must cut the frame more than once"
    );
    assert_eq!(
        single_submits, 1,
        "the whole chain belongs to one command buffer"
    );
    assert_eq!(draw.frame_status().1, 0, "the chain raised a flag");
}
