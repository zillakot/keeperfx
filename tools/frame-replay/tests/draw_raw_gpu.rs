use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, RAW_IMAGE};

fn word(data: &mut &[u8]) -> u32 {
    let value = u32::from_le_bytes(data[..4].try_into().unwrap());
    *data = &data[4..];
    value
}

#[test]
#[ignore = "requires GPU and KFX_RAW_IMAGE_FIXTURE"]
fn actual_native_raw_images_tiles_and_clears() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_RAW_IMAGE_FIXTURE")?)?;
    let mut data = bytes.as_slice();
    ensure!(word(&mut data) == 0x3152464b, "fixture magic");
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
        if command.kind == RAW_IMAGE {
            let mut invalid = command;
            invalid.step_low = 0;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "zero scale accepted"
            );
            invalid = command;
            invalid.source_width = sw + 1;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "source overrun accepted"
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
    ensure!(data.is_empty(), "trailing fixture bytes");
    println!("{count} actual native raw image/tile/clear cases matched exact GPU indices");
    Ok(())
}
