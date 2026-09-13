use anyhow::{Result, ensure};
use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, MOVIE, RAW_IMAGE};

fn word(data: &mut &[u8]) -> u32 {
    let value = u32::from_le_bytes(data[..4].try_into().unwrap());
    *data = &data[4..];
    value
}

#[test]
#[ignore = "requires GPU and KFX_MOVIE_FRAME_FIXTURE"]
fn actual_native_movie_frames() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_MOVIE_FRAME_FIXTURE")?)?;
    let mut data = bytes.as_slice();
    ensure!(word(&mut data) == 0x314d464b, "fixture magic");
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
        if command.kind == MOVIE {
            let mut invalid = command;
            invalid.source_x = 8;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "invalid mode accepted"
            );
            invalid = command;
            invalid.source_width = sw + 1;
            ensure!(
                drawing.submit(target, &[command, invalid]).is_err(),
                "source overrun accepted"
            );
            ensure!(
                drawing.readback(target)? == actual,
                "rejected movie batch changed target"
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
    println!("{count} actual native movie cases matched exact GPU indices");
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn ordered_movie_sources_and_palettes() -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    eprintln!("movie palette adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue)?;
    let mut drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm)?;
    let target = drawing.create_target(8, 6)?;
    let mut expected = vec![7u8; 48];
    drawing.submit(
        target,
        &[Command {
            kind: keeperfx_frame_replay::draw::CLEAR,
            colour: 7,
            ..Default::default()
        }],
    )?;
    let mut outputs = Vec::new();
    for version in 0..2u32 {
        let mut source: Vec<_> = (0..12).map(|i| (i * 17 + version * 31) as u8).collect();
        let asset = drawing.create_resource(&source, 4, 2, 6)?;
        let original = source.clone();
        source.fill(253);
        let mut palette: Vec<_> = (0..256u32)
            .flat_map(|i| {
                [
                    (i + version * 11) as u8,
                    (255 - i) as u8,
                    (i * 17 + version * 23) as u8,
                    255,
                ]
            })
            .collect();
        drawing.submit(
            target,
            &[Command {
                kind: MOVIE,
                width: 8,
                height: 6,
                source: asset,
                source_width: 4,
                source_height: 2,
                source_x: if version == 0 { 4 } else { 0 },
                start_low: version + 1,
                start_high: 1,
                ..Default::default()
            }],
        )?;
        drawing.release_resource(asset)?;
        for sy in 0..2 {
            for sx in 0..4 {
                let y = 1 + sy * if version == 0 { 2 } else { 1 };
                expected[y * 8 + sx + version as usize + 1] = original[sy * 6 + sx];
            }
        }
        let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("movie palette version"),
            size: wgpu::Extent3d {
                width: 8,
                height: 6,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        drawing.present_into(
            target,
            &palette,
            8,
            6,
            &texture.create_view(&Default::default()),
        )?;
        let expected_rgba: Vec<u8> = expected
            .iter()
            .flat_map(|&i| palette[i as usize * 4..i as usize * 4 + 4].iter().copied())
            .collect();
        palette.fill(0);
        outputs.push((texture, expected_rgba));
    }
    ensure!(
        drawing.readback(target)? == expected,
        "ordered movie indices differ"
    );
    drawing.release_target(target)?;
    for (texture, expected_rgba) in outputs {
        let staging = renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 6,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = renderer
            .device()
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        renderer.queue().submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| sender.send(r).unwrap());
        renderer.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        receiver.recv()??;
        let rgba = staging.slice(..).get_mapped_range()?;
        for y in 0..6 {
            ensure!(
                rgba[y * 256..y * 256 + 32] == expected_rgba[y * 32..y * 32 + 32],
                "movie palette version mismatch"
            );
        }
        drop(rgba);
        staging.unmap();
    }
    renderer.check_status()?;
    Ok(())
}
