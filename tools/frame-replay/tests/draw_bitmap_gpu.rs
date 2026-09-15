use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{BITMAP, Command, DrawRenderer, IMAGE, OPAQUE, arena_kinds};

fn word(data: &mut &[u8]) -> u32 {
    let value = u32::from_le_bytes(data[..4].try_into().unwrap());
    *data = &data[4..];
    value
}

#[test]
#[ignore = "requires GPU and KFX_BITMAP_FIXTURE"]
fn actual_native_huge_sprites_and_glyphs() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_BITMAP_FIXTURE")?)?;
    let mut data = bytes.as_slice();
    ensure!(word(&mut data) == 0x3142464b, "fixture magic");
    let count = word(&mut data);
    let pitch = word(&mut data);
    let rows = word(&mut data);
    ensure!(word(&mut data) == 112 && count >= 100, "fixture contract");
    let initial: Vec<_> = (0..pitch * rows)
        .map(|i| ((i * 19 + i / pitch * 13) & 255) as u8)
        .collect();
    let mut drawing = DrawRenderer::headless()?;
    for case in 0..count {
        let width = word(&mut data);
        let height = word(&mut data);
        ensure!(word(&mut data) == pitch, "fixture pitch");
        let sw = word(&mut data);
        let sh = word(&mut data);
        let sp = word(&mut data);
        let length = word(&mut data) as usize;
        let artwork_length = word(&mut data) as usize;
        let w: [u32; 28] = std::array::from_fn(|_| word(&mut data));
        let mut command = Command {
            abi_version: w[0],
            kind: w[1],
            blend: w[2],
            colour: w[3],
            x: w[4] as i32,
            y: w[5] as i32,
            width: w[6],
            height: w[7],
            clip_x: w[8] as i32,
            clip_y: w[9] as i32,
            clip_width: w[10],
            clip_height: w[11],
            source: 0,
            table: 0,
            source_x: w[16],
            source_y: w[17],
            source_width: w[18],
            source_height: w[19],
            start_low: w[20],
            start_high: w[21],
            step_low: w[22],
            step_high: w[23],
            transparent: w[24],
            reserved: [w[25], w[26], w[27]],
        };
        if length > 0 {
            let mut source = data[..length].to_vec();
            command.source = drawing.create_resource(&source, sw, sh, sp)?;
            source.fill(0);
        }
        data = &data[length..];
        let mut artwork = 0;
        if artwork_length > 0 {
            let mut bytes = data[..artwork_length].to_vec();
            artwork = drawing.create_resource(&bytes, 1, 1, 1)?;
            bytes.fill(0);
            command.start_low = artwork as u32;
            command.start_high = (artwork >> 32) as u32;
        }
        data = &data[artwork_length..];
        let expected = &data[..(pitch * rows) as usize];
        data = &data[(pitch * rows) as usize..];
        let target = drawing.create_target(width, height)?;
        let seed = drawing.create_resource(&initial, width, height, pitch)?;
        drawing.submit(
            target,
            &[
                Command {
                    kind: IMAGE,
                    width,
                    height,
                    source: seed,
                    source_width: width,
                    source_height: height,
                    ..Default::default()
                },
                command,
            ],
        )?;
        let actual = drawing.readback(target)?;
        for y in 0..height as usize {
            ensure!(
                actual[y * width as usize..(y + 1) * width as usize]
                    == expected[y * pitch as usize..y * pitch as usize + width as usize],
                "native image mismatch case {case}, kind {}, row {y}",
                command.kind
            );
        }
        if command.kind == keeperfx_frame_replay::draw::BITMAP {
            let mut invalid = command;
            invalid.source_x = 3;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "invalid bitmap accepted"
            );
            ensure!(
                drawing.readback(target)? == actual,
                "rejected batch changed target"
            );
        }
        drawing.release_resource(seed)?;
        if command.source != 0 {
            drawing.release_resource(command.source)?;
        }
        if artwork != 0 {
            drawing.release_resource(artwork)?;
        }
        drawing.release_target(target)?;
    }
    ensure!(data.is_empty(), "trailing fixture bytes");
    println!("{count} actual native huge sprite/glyph cases matched exact GPU indices");
    Ok(())
}

/// One artwork, two scroll positions, one frame: the second command reads the resident
/// asset and every position draws its own geometry.
#[test]
#[ignore = "requires a GPU"]
fn one_huge_artwork_serves_two_positions() -> Result<()> {
    let mut artwork = Vec::new();
    for (offset, count) in [(16u32, 2u32), (24, 1)] {
        artwork.extend(offset.to_le_bytes());
        artwork.extend(count.to_le_bytes());
    }
    for record in [10u32 << 16, 2 | 20 << 16, 1 | 30 << 16] {
        artwork.extend(record.to_le_bytes());
    }
    let mut drawing = DrawRenderer::headless()?;
    let handle = drawing.create_resource(&artwork, 1, 1, 1)?;
    let (width, height) = (12u32, 4u32);
    let target = drawing.create_target(width, height)?;
    let mut commands = vec![Command {
        kind: keeperfx_frame_replay::draw::CLEAR,
        colour: 0,
        width,
        height,
        ..Default::default()
    }];
    let mut sources = Vec::new();
    for base in [0u32, 6] {
        let mut geometry = Vec::new();
        for (y, copies) in [(0u32, 1u32), (1, 1)] {
            geometry.extend(y.to_le_bytes());
            geometry.extend(copies.to_le_bytes());
        }
        for column in 0..3u32 {
            geometry.extend((base + column).to_le_bytes());
            geometry.extend(1u32.to_le_bytes());
        }
        let source = drawing.create_resource(&geometry, 1, 1, 1)?;
        sources.push(source);
        commands.push(Command {
            kind: BITMAP,
            source,
            width,
            height,
            clip_width: width,
            clip_height: height,
            source_height: 2,
            transparent: OPAQUE,
            start_low: handle as u32,
            start_high: (handle >> 32) as u32,
            ..Default::default()
        });
    }
    let before = drawing.counters().arena_by_kind[arena_kinds::BITMAP_KIND];
    drawing.submit(target, &commands)?;
    let after = drawing.counters().arena_by_kind[arena_kinds::BITMAP_KIND];
    ensure!(
        after.hits > before.hits,
        "the second position re-uploaded the artwork"
    );
    let pixels = drawing.readback(target)?;
    for base in [0usize, 6] {
        ensure!(
            pixels[base] == 10 && pixels[base + 2] == 20,
            "row 0 at {base}: {:?}",
            &pixels[base..base + 3]
        );
        ensure!(pixels[width as usize + base + 1] == 30, "row 1 at {base}");
    }
    drawing.release_target(target)?;
    drawing.release_resource(handle)?;
    for source in sources {
        drawing.release_resource(source)?;
    }
    Ok(())
}
