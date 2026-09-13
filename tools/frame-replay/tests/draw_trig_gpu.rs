use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, SPRITE, TRIG};

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
    for n in 0..count {
        let mode = word(&mut data);
        let colour = word(&mut data);
        let mut source = data[..60].to_vec();
        data = &data[60..];
        let textured = matches!(mode, 2 | 3 | 7 | 8 | 10 | 11 | 12 | 13 | 18 | 19 | 22 | 23);
        if textured {
            source.extend_from_slice(&texture);
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
            ..Default::default()
        };
        commands.push(command);
        sources.push(source_id);
        last = Some(command);
        let expected = &data[..(width * height) as usize];
        data = &data[expected.len()..];
        if commands.len() == 6 || n + 1 == count {
            draw.submit(target, &commands)?;
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
    ensure!(
        draw.submit(
            target,
            &[
                Command {
                    kind: CLEAR,
                    colour: 9,
                    ..Default::default()
                },
                invalid_command
            ]
        )
        .is_err(),
        "GPU accepted an invalid late shade lookup"
    );
    ensure!(
        draw.readback(target)? == original,
        "GPU preflight failure changed target"
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
    draw.submit(sprite_target, &[sprite_command])?;
    ensure!(
        draw.readback(sprite_target)?.contains(&199),
        "valid ordered sprite did not draw"
    );
    draw.release_target(sprite_target)?;
    ensure!(
        draw.submit(
            target,
            &[
                Command {
                    kind: CLEAR,
                    colour: 9,
                    ..Default::default()
                },
                sprite_command,
                invalid_command
            ]
        )
        .is_err(),
        "mixed ordered sprite batch accepted late invalid triangle"
    );
    ensure!(
        draw.readback(target)? == original,
        "mixed sprite/triangle rejection changed target"
    );
    let mut undefined = Vec::new();
    for (x, y) in [(1_i32, 0_i32), (2, 1), (3, 3)] {
        for n in [x, y, 0, 0, 65536] {
            undefined.extend_from_slice(&n.to_le_bytes());
        }
    }
    let resource = draw.create_resource(&undefined, 1, 1, 1)?;
    ensure!(
        draw.submit(
            target,
            &[Command {
                source: resource,
                ..invalid_command
            }]
        )
        .is_err(),
        "GPU accepted an undefined native horizontal step"
    );
    ensure!(
        draw.readback(target)? == original,
        "undefined triangle changed target"
    );
    ensure!(data.is_empty(), "trailing fixture bytes");
    println!("{count} native original-vertex general triangles matched exactly");
    Ok(())
}
