use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, IMAGE, OPAQUE, SPRITE, TRIG};

#[test]
#[ignore = "requires Metal/Vulkan and KFX_TRIG_FIXTURE"]
fn native_general_triangles() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_TRIG_FIXTURE")?)?;
    ensure!(&bytes[..8] == b"KFXTRIG1", "fixture magic");
    let mut data = &bytes[8..];
    fn word(data: &mut &[u8]) -> u32 {
        let n = u32::from_le_bytes(data[..4].try_into().unwrap());
        *data = &data[4..];
        n
    }
    let width = word(&mut data);
    let height = word(&mut data);
    let count = word(&mut data);
    let mut draw = DrawRenderer::headless()?;
    let target = draw.create_target(width, height)?;
    let texture = data[..65536].to_vec();
    data = &data[65536..];
    // The page is one asset for every triangle that samples it, so the per-call source
    // is the 60 geometry bytes alone.
    let page = draw.create_resource(&texture, 1, 1, 1)?;
    let table = draw.create_resource(&data[..81920], 256, 320, 256)?;
    data = &data[81920..];
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 167,
            ..Default::default()
        }],
    )?;
    let mut commands = Vec::new();
    let mut sources = Vec::new();
    let mut last = None;
    let mut first_textured_geometry = None;
    let mut texture_uploaded = false;
    let mut warm_geometry_varied = false;
    for n in 0..count {
        let mode = word(&mut data);
        let colour = word(&mut data);
        let mut source = data[..60].to_vec();
        data = &data[60..];
        let textured = matches!(mode, 2 | 3 | 5..=13 | 18..=26);
        if textured {
            let first = first_textured_geometry.get_or_insert_with(|| source.clone());
            warm_geometry_varied |= texture_uploaded && source != *first;
        }
        let source_id = draw.create_resource(&source, 1, 1, 1)?;
        source.fill(0);
        let command = Command {
            kind: TRIG,
            source: source_id,
            table,
            source_x: mode,
            source_width: 64,
            source_y: if textured { 65536 } else { 0 },
            width,
            height,
            colour,
            start_low: if textured { page as u32 } else { 0 },
            start_high: if textured { (page >> 32) as u32 } else { 0 },
            ..Default::default()
        };
        commands.push(command);
        sources.push(source_id);
        last = Some(command);
        let expected = &data[..(width * height) as usize];
        data = &data[expected.len()..];
        if commands.len() == 6 || n + 1 == count {
            let before = draw.counters();
            draw.submit(target, &commands)?;
            let after = draw.counters();
            if texture_uploaded {
                ensure!(
                    after.arena_trig_texture_source_bytes == before.arena_trig_texture_source_bytes,
                    "triangle {n}: resident texture page was uploaded again"
                );
            } else if commands.iter().any(|command| command.source_y != 0) {
                ensure!(
                    after.arena_trig_texture_source_bytes - before.arena_trig_texture_source_bytes
                        == 65536,
                    "triangle {n}: first textured batch did not upload the full texture page"
                );
                texture_uploaded = true;
            }
            let result = draw.readback(target)?;
            if result != expected {
                let i = result
                    .iter()
                    .zip(expected)
                    .position(|(a, b)| a != b)
                    .unwrap();
                anyhow::bail!(
                    "triangle {n} mode {mode} mismatch at {},{}: got {} expected {}",
                    i % width as usize,
                    i / width as usize,
                    result[i],
                    expected[i]
                );
            }
            commands.clear();
            for source in sources.drain(..) {
                draw.release_resource(source)?;
            }
        }
    }
    ensure!(
        warm_geometry_varied,
        "fixture did not reuse the resident texture page with different geometry"
    );
    let original = draw.readback(target)?;
    let command = last.unwrap();
    ensure!(
        draw.submit(
            target,
            &[
                Command {
                    kind: CLEAR,
                    colour: 9,
                    ..Default::default()
                },
                command
            ]
        )
        .is_err(),
        "released resource accepted"
    );
    ensure!(
        draw.readback(target)? == original,
        "rejected batch changed target"
    );
    let mut invalid = Vec::new();
    for (x, y) in [(0_i32, 0_i32), (70, 0), (0, 60)] {
        for n in [x, y, 0, 0, 64 * 65536] {
            invalid.extend_from_slice(&n.to_le_bytes());
        }
    }
    let resource = draw.create_resource(&invalid, 1, 1, 1)?;
    let invalid_command = Command {
        kind: TRIG,
        source: resource,
        table,
        source_x: 4,
        source_width: 64,
        width,
        height,
        ..Default::default()
    };
    let clear9 = Command {
        kind: CLEAR,
        colour: 9,
        ..Default::default()
    };
    let restore_source = draw.create_resource(&original, width, height, width)?;
    let restore = Command {
        kind: IMAGE,
        source: restore_source,
        width,
        height,
        clip_width: width,
        clip_height: height,
        source_width: width,
        source_height: height,
        transparent: OPAQUE,
        ..Default::default()
    };
    let cleared = vec![9u8; (width * height) as usize];
    draw.submit(target, &[clear9, invalid_command])?;
    ensure!(
        draw.readback(target)? == cleared,
        "a flagged late shade lookup wrote triangle pixels"
    );
    ensure!(
        draw.frame_status().1 & 0b11 == 0b11,
        "an invalid late shade lookup raised no frame flag"
    );
    draw.submit(target, &[restore])?;
    ensure!(
        draw.readback(target)? == original,
        "the restore image did not reproduce the target"
    );
    let mut sprite = vec![199, 2];
    for n in [5_u32, 2, 5, 2] {
        sprite.extend_from_slice(&n.to_le_bytes());
    }
    sprite.extend(0_u8..=255);
    let sprite_id = draw.create_resource(&sprite, 1, 1, 1)?;
    let sprite_command = Command {
        kind: SPRITE,
        source: sprite_id,
        source_x: 9,
        source_width: 1,
        source_height: 1,
        width,
        height,
        clip_width: width,
        clip_height: height,
        ..Default::default()
    };
    let sprite_target = draw.create_target(width, height)?;
    draw.submit(sprite_target, &[clear9, sprite_command])?;
    let with_sprite = draw.readback(sprite_target)?;
    ensure!(
        with_sprite.contains(&199),
        "valid ordered sprite did not draw"
    );
    draw.release_target(sprite_target)?;
    draw.submit(target, &[clear9, sprite_command, invalid_command])?;
    ensure!(
        draw.readback(target)? == with_sprite,
        "the ordered sprite must draw beside a flagged triangle"
    );
    ensure!(
        draw.frame_status().1 & 0b11 == 0b11,
        "a mixed ordered-sprite batch raised no frame flag"
    );
    draw.submit(target, &[restore])?;
    for mode in [5, 6, 9, 20, 21, 24, 25, 26] {
        for transparent in [false, true] {
            let mut source = Vec::new();
            for (x, y) in [(0_i32, 0_i32), (70, 0), (0, 60)] {
                for n in [x, y, 0, 0, 64 * 65536] {
                    source.extend_from_slice(&n.to_le_bytes());
                }
            }
            source.push(if transparent {
                0
            } else if mode == 9 {
                64
            } else {
                1
            });
            let source = draw.create_resource(&source, 1, 1, 1)?;
            let c = Command {
                source,
                source_y: 1,
                source_x: mode,
                ..invalid_command
            };
            let skips_fade = transparent && matches!(mode, 6 | 9 | 24 | 25);
            if skips_fade {
                draw.submit(target, &[c])?;
            } else {
                draw.submit(target, &[clear9, sprite_command, c])?;
                ensure!(
                    draw.readback(target)? == with_sprite,
                    "mode {mode} wrote an invalid computed fade access"
                );
                ensure!(
                    draw.frame_status().1 & 0b11 == 0b11,
                    "mode {mode} raised no frame flag for an invalid fade access"
                );
                draw.submit(target, &[restore])?;
            }
            ensure!(
                draw.readback(target)? == original,
                "mode {mode} changed target on transparency or recovery"
            );
            draw.release_resource(source)?;
        }
    }
    for mode in [
        2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    ] {
        let mut source = Vec::new();
        for (x, y) in [(0_i32, 0_i32), (70, 0), (0, 60)] {
            for n in [x, y, 65536, 0, 0] {
                source.extend_from_slice(&n.to_le_bytes());
            }
        }
        source.push(0);
        let source = draw.create_resource(&source, 1, 1, 1)?;
        let c = Command {
            source,
            source_y: 1,
            source_x: mode,
            ..invalid_command
        };
        draw.submit(target, &[clear9, c])?;
        ensure!(
            draw.readback(target)? == cleared,
            "mode {mode} wrote a computed texture access beyond its extent"
        );
        ensure!(
            draw.frame_status().1 & 0b11 == 0b11,
            "mode {mode} raised no frame flag for a texture access beyond its extent"
        );
        draw.submit(target, &[restore])?;
        ensure!(
            draw.readback(target)? == original,
            "mode {mode} texture recovery changed target"
        );
        draw.release_resource(source)?;
    }
    let mut thin = Vec::new();
    for (x, y) in [(1_i32, 0_i32), (2, 1), (3, 3)] {
        for n in [x, y, 0, 0, 65536] {
            thin.extend_from_slice(&n.to_le_bytes());
        }
    }
    let resource = draw.create_resource(&thin, 1, 1, 1)?;
    draw.submit(
        target,
        &[Command {
            source: resource,
            source_x: 1,
            ..invalid_command
        }],
    )?;
    let mut expected = original;
    expected[width as usize + 1] = 1;
    ensure!(
        draw.readback(target)? == expected,
        "thin triangle did not use deterministic horizontal shade"
    );
    ensure!(data.is_empty(), "trailing fixture bytes");
    println!("{count} native original-vertex general triangles matched exactly");
    Ok(())
}
