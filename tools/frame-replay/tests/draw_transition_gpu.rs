use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{ABI_VERSION, Command, DrawRenderer, IMAGE, TRANSITION};

fn word(data: &mut &[u8]) -> u32 {
    let value = u32::from_le_bytes(data[..4].try_into().unwrap());
    *data = &data[4..];
    value
}
fn take<'a>(data: &mut &'a [u8], n: usize) -> &'a [u8] {
    let bytes = &data[..n];
    *data = &data[n..];
    bytes
}
fn seed(d: &mut DrawRenderer, target: u64, bytes: &[u8], w: u32, h: u32, p: u32) -> Result<()> {
    let source = d.create_resource(bytes, w, h, p)?;
    d.submit(
        target,
        &[Command {
            kind: IMAGE,
            width: w,
            height: h,
            source,
            source_width: w,
            source_height: h,
            ..Default::default()
        }],
    )?;
    d.release_resource(source)
}
fn snapshot(d: &mut DrawRenderer, data: &mut &[u8]) -> Result<u64> {
    let w = word(data);
    let h = word(data);
    let p = word(data);
    if w == 0 {
        return Ok(0);
    }
    let bytes = take(data, (p * h) as usize);
    let target = d.create_target(w, h)?;
    seed(d, target, bytes, w, h, p)?;
    let id = d.create_target_snapshot(target, 0, 0, w, h, p + 3)?;
    d.release_target(target)?;
    Ok(id)
}
#[test]
#[ignore = "requires GPU and KFX_TRANSITION_FIXTURE"]
fn actual_native_transitions() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_TRANSITION_FIXTURE")?)?;
    let mut data = bytes.as_slice();
    ensure!(word(&mut data) == 0x3154464b, "fixture magic");
    let count = word(&mut data);
    ensure!(count >= 270, "fixture count");
    let mut d = DrawRenderer::headless()?;
    for case in 0..count {
        let width = word(&mut data);
        let height = word(&mut data);
        let pitch = word(&mut data);
        let w: [u32; 28] = std::array::from_fn(|_| word(&mut data));
        let a = snapshot(&mut d, &mut data)?;
        let b = snapshot(&mut d, &mut data)?;
        let n = word(&mut data) as usize;
        let table = d.create_resource(take(&mut data, n), 1, 1, 1)?;
        let initial = take(&mut data, (pitch * height) as usize);
        let expected = take(&mut data, (pitch * height) as usize);
        let command = Command {
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
            source: a,
            table,
            source_x: w[16],
            source_y: w[17],
            source_width: w[18],
            source_height: w[19],
            start_low: b as u32,
            start_high: (b >> 32) as u32,
            step_low: w[22],
            step_high: w[23],
            transparent: w[24],
            reserved: [w[25], w[26], w[27]],
        };
        ensure!(command.kind == TRANSITION, "fixture operation");
        let target = d.create_target(width, height)?;
        seed(&mut d, target, initial, width, height, pitch)?;
        let before = d.counters();
        let copies = d.target_resource_counters();
        d.submit_target_images(target, &[command])?;
        let after = d.counters();
        ensure!(
            after.readback_bytes == before.readback_bytes,
            "transition readback"
        );
        ensure!(
            after.asset_upload_bytes - before.asset_upload_bytes
                == n as u64 * keeperfx_frame_replay::draw::assets::STRIDE as u64,
            "transition uploaded source pixels"
        );
        ensure!(
            d.target_resource_counters().sampling_copy_bytes > copies.sampling_copy_bytes,
            "transition did not copy GPU sources"
        );
        let actual = d.readback(target)?;
        for y in 0..height as usize {
            for x in 0..width as usize {
                ensure!(
                    actual[y * width as usize + x] == expected[y * pitch as usize + x],
                    "native transition mismatch case {case}, mode {}, x{x}, y{y}: {} vs {}",
                    command.source_x,
                    actual[y * width as usize + x],
                    expected[y * pitch as usize + x]
                );
            }
        }
        let mut bad = command;
        bad.source_x = 9;
        ensure!(
            d.submit_target_images(target, &[bad]).is_err(),
            "invalid mode accepted"
        );
        bad = command;
        bad.source = u64::MAX;
        ensure!(
            d.submit_target_images(target, &[bad]).is_err(),
            "invalid snapshot accepted"
        );
        ensure!(
            d.submit_target_images(target, &[command, command]).is_err(),
            "dependent batch accepted"
        );
        for change in 0..6 {
            let mut invalid = command;
            match change {
                0 => invalid.blend = 1,
                1 => invalid.abi_version = ABI_VERSION + 1,
                2 => invalid.table = a,
                3 => invalid.transparent = 0,
                4 => invalid.reserved[1] = 1,
                _ if command.source_x == 0 => invalid.step_low = 33,
                _ => invalid.width = width,
            }
            ensure!(
                d.submit_target_images(target, &[invalid]).is_err(),
                "malformed transition accepted"
            );
        }
        ensure!(
            d.readback(target)? == actual,
            "rejected transition changed target"
        );
        if case % 33 == 0 {
            seed(&mut d, target, initial, width, height, pitch)?;
            let mut clipped = command;
            clipped.clip_x = 7;
            clipped.clip_y = 1;
            clipped.clip_width = width - 13;
            clipped.clip_height = height - 2;
            d.submit_target_images(target, &[clipped])?;
            let actual = d.readback(target)?;
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let inside =
                        x >= 7 && x < (width - 6) as usize && y >= 1 && y < (height - 1) as usize;
                    ensure!(
                        actual[y * width as usize + x]
                            == if inside {
                                expected[y * pitch as usize + x]
                            } else {
                                initial[y * pitch as usize + x]
                            },
                        "transition clipping mismatch"
                    );
                }
            }
        }
        d.release_target_snapshot(a)?;
        if b != 0 {
            d.release_target_snapshot(b)?;
        }
        d.release_resource(table)?;
        d.release_target(target)?;
    }
    ensure!(data.is_empty(), "trailing fixture bytes");
    Ok(())
}
