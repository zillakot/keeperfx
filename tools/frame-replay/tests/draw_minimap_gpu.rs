use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, MINIMAP};
fn drawing() -> DrawRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("minimap adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
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
    let mut captures = 0;
    for case in 0..count {
        let length = word(&bytes, &mut o) as usize;
        let mut asset = bytes[o..o + length].to_vec();
        o += length;
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
        asset.fill(71);
        let before_upload = draw.counters().replay;
        draw.frame_begin(target).unwrap();
        draw.submit(target, &[c]).unwrap();
        draw.frame_end().unwrap();
        let after_upload = draw.counters().replay;
        assert_eq!(
            after_upload.upload_ring_overflows - before_upload.upload_ring_overflows,
            0
        );
        if mode != 4 {
            assert!(after_upload.upload_queue_writes > before_upload.upload_queue_writes);
        }
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
    assert_eq!(captures, 8);
    assert_eq!(draw.target_resource_counters().snapshots, captures);
    draw.release_target(target).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn dictionary_reordering_missing_colours_and_partial_workgroups() {
    let mut draw = drawing();
    let width = 14u32;
    let diameter = 10u32;
    let target = draw.create_target(width, width).unwrap();
    let palette = [
        0u8, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 239, 252, 255,
    ];
    let initial: Vec<_> = (0..width * width)
        .map(|i| {
            if i % 3 == 0 {
                254
            } else {
                palette[i as usize % palette.len()]
            }
        })
        .collect();
    let image = draw.create_resource(&initial, width, width, width).unwrap();
    let cells: Vec<u16> = (0..(diameter + 1).pow(2))
        .map(|i| ((i * 617) % 38569) as u16)
        .collect();
    let mut header = [0u32; 24];
    header[0] = 4;
    header[1] = width;
    header[2] = width;
    header[3] = 2;
    header[4] = 2;
    header[5] = diameter;
    header[22] = 96;
    let capture: Vec<_> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
    let capture = draw.create_resource(&capture, 1, 1, 1).unwrap();
    let command = Command {
        kind: MINIMAP,
        source: capture,
        width,
        height: width,
        clip_width: width,
        clip_height: width,
        ..Default::default()
    };
    for count in [1usize, 4, 16] {
        for reverse in [false, true] {
            draw.submit(
                target,
                &[Command {
                    kind: IMAGE,
                    source: image,
                    width,
                    height: width,
                    source_width: width,
                    source_height: width,
                    ..Default::default()
                }],
            )
            .unwrap();
            draw.submit(target, &[command]).unwrap();
            header[0] = 0;
            header[7] = 65536;
            header[10] = diameter;
            header[11] = diameter;
            header[12] = 96;
            header[13] = 352;
            header[14] = header[13] + cells.len() as u32 * 2;
            header[15] = count as u32 * 38569;
            let dictionary: Vec<_> = (0..count)
                .map(|i| palette[if reverse { count - 1 - i } else { i }])
                .collect();
            let mut source: Vec<_> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
            source.extend(&dictionary);
            source.resize(352, 0);
            source.extend(cells.iter().flat_map(|v| v.to_le_bytes()));
            source.extend(
                (0..count)
                    .flat_map(|colour| (0..38569).map(move |cell| (colour * 11 + cell * 7) as u8)),
            );
            let resource = draw.create_resource(&source, 1, 1, 1).unwrap();
            draw.submit(
                target,
                &[Command {
                    source: resource,
                    ..command
                }],
            )
            .unwrap();
            draw.release_resource(resource).unwrap();
            let mut expected = initial.clone();
            for y in 0..diameter {
                for x in 0..diameter {
                    let n = 25 - (5 - y as i32 - 1).pow(2);
                    let s = (0..=5).filter(|v| v * v <= n).max().unwrap();
                    if (x as i32) < 5 - s || x as i32 >= 5 + s {
                        continue;
                    }
                    let dst = ((y + 2) * width + x + 2) as usize;
                    let colour = dictionary
                        .iter()
                        .position(|&c| c == initial[dst])
                        .unwrap_or(0);
                    let cell = cells[(y * (diameter + 1) + x) as usize] as usize;
                    expected[dst] = (colour * 11 + cell * 7) as u8;
                }
            }
            assert_eq!(
                draw.readback(target).unwrap(),
                expected,
                "count={count}, reverse={reverse}"
            );
        }
    }
    draw.release_resource(capture).unwrap();
    draw.release_resource(image).unwrap();
    draw.release_target(target).unwrap();
}
