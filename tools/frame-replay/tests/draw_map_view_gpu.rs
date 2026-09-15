use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, MAP_VIEW, TRIG};

fn word(data: &mut &[u8]) -> u32 {
    let value = u32::from_le_bytes(data[..4].try_into().unwrap());
    *data = &data[4..];
    value
}

#[test]
#[ignore = "requires GPU and KFX_MAP_VIEW_FIXTURE"]
fn actual_native_map_views() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_MAP_VIEW_FIXTURE")?)?;
    let mut data = bytes.as_slice();
    ensure!(word(&mut data) == 0x3156464b, "fixture magic");
    let count = word(&mut data);
    let pitch = word(&mut data);
    let rows = word(&mut data);
    ensure!(word(&mut data) == 112 && count >= 1000, "fixture contract");
    let initial: Vec<_> = (0..pitch * rows)
        .map(|i| ((i * 19 + i / pitch * 13) & 255) as u8)
        .collect();
    let mut drawing = DrawRenderer::headless()?;
    // One resource for every row, so the suite exercises the row table's residency.
    let mut row_table = 0;
    for case in 0..count {
        let width = word(&mut data);
        let height = word(&mut data);
        ensure!(word(&mut data) == pitch, "fixture pitch");
        let sw = word(&mut data);
        let sh = word(&mut data);
        let sp = word(&mut data);
        let length = word(&mut data) as usize;
        let tw = word(&mut data);
        let th = word(&mut data);
        let tp = word(&mut data);
        let table_length = word(&mut data) as usize;
        let table_new = word(&mut data);
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
        if table_new != 0 {
            row_table = drawing.create_resource(&data[..table_length], tw, th, tp)?;
            data = &data[table_length..];
        }
        if table_length > 0 {
            command.table = row_table;
        }
        let expected = &data[..(pitch * rows) as usize];
        data = &data[(pitch * rows) as usize..];
        let target = drawing.create_target(width, height)?;
        let seed = drawing.create_resource(&initial, width, height, pitch)?;
        drawing.submit(
            target,
            &[Command {
                kind: IMAGE,
                width,
                height,
                source: seed,
                source_width: width,
                source_height: height,
                ..Default::default()
            }],
        )?;
        let before = drawing.counters();
        drawing.submit(target, &[command])?;
        let after = drawing.counters();
        ensure!(
            after.readback_bytes == before.readback_bytes,
            "map command read back pixels"
        );
        let stride = keeperfx_frame_replay::draw::assets::STRIDE as u64;
        let uploaded = after.asset_upload_bytes - before.asset_upload_bytes;
        let table_upload = if table_new != 0 {
            table_length as u64
        } else {
            0
        };
        ensure!(
            uploaded == (length as u64 + table_upload) * stride,
            "map case {case} uploaded {uploaded} bytes against {} for its source and {} for its              table: a row whose table uploads again has lost arena residency, by eviction or by              a changed key, and one that uploads less has lost its source",
            length as u64 * stride,
            table_upload * stride
        );
        let actual = drawing.readback(target)?;
        for y in 0..height as usize {
            for x in 0..width as usize {
                ensure!(
                    actual[y * width as usize + x] == expected[y * pitch as usize + x],
                    "native map mismatch case {case}, mode {}, x{x}, y{y}: got {}, expected {}; command {command:?}",
                    command.source_x,
                    actual[y * width as usize + x],
                    expected[y * pitch as usize + x]
                );
            }
        }
        if case == 0 {
            let vertices = drawing.create_resource(&[0; 60], 1, 1, 1)?;
            let table = drawing.create_resource(&vec![0; 256 * 320], 256, 320, 256)?;
            let triangle = Command {
                kind: TRIG,
                source: vertices,
                table,
                source_width: 64,
                width,
                height,
                ..Default::default()
            };
            drawing.submit(target, &[command, triangle])?;
            let mut only_map = command;
            only_map.source_x = 9;
            ensure!(
                drawing.submit(target, &[triangle, only_map]).is_err(),
                "mixed triangle/map rejected command accepted"
            );
            drawing.release_resource(vertices)?;
            drawing.release_resource(table)?;
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
        }
        if command.kind == MAP_VIEW {
            let mut invalid = command;
            invalid.source_x = 9;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "unknown map mode accepted"
            );
            invalid = command;
            invalid.blend = 1;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "map blend accepted"
            );
            if command.source_x == 0 {
                invalid = command;
                invalid.table = 0;
                ensure!(
                    drawing.submit(target, &[command, invalid]).is_err(),
                    "map row without a table accepted"
                );
            }
            invalid = command;
            match command.source_x {
                0 => invalid.source_width += 1,
                1 => invalid.source_y = 1,
                2 => invalid.step_low = u32::MAX,
                3 => invalid.source_y = 37,
                _ => unreachable!(),
            }
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "malformed map source accepted"
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
        drawing.release_target(target)?;
    }
    if row_table != 0 {
        drawing.release_resource(row_table)?;
    }
    ensure!(data.is_empty(), "trailing fixture bytes");
    println!("{count} actual native map-view cases matched exact GPU indices");
    Ok(())
}
