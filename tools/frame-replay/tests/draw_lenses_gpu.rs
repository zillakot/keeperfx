use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, IMAGE, LENS_EFFECT, arena_kinds};

fn word(bytes: &[u8], offset: &mut usize) -> u32 {
    let value = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    value
}

#[test]
#[ignore = "requires a GPU and the native lens fixture"]
fn native_lens_indices_match() {
    let path = std::env::var("KFX_LENS_FIXTURE").expect("KFX_LENS_FIXTURE is required");
    let bytes = std::fs::read(path).unwrap();
    let mut offset = 0;
    assert_eq!(word(&bytes, &mut offset), 0x534e454c);
    let count = word(&bytes, &mut offset);
    assert_eq!(count, 54);
    let mut drawing = DrawRenderer::headless().unwrap();
    for case in 0..count {
        let width = word(&bytes, &mut offset);
        let height = word(&bytes, &mut offset);
        let source_length = word(&bytes, &mut offset) as usize;
        let map_length = word(&bytes, &mut offset) as usize;
        let fade_length = word(&bytes, &mut offset) as usize;
        let length = (width * height) as usize;
        let initial = &bytes[offset..offset + length];
        offset += length;
        let mut source = bytes[offset..offset + source_length].to_vec();
        offset += source_length;
        let mut tables = [0u64; 2];
        for (slot, table_length) in [map_length, fade_length].into_iter().enumerate() {
            if table_length > 0 {
                let mut table = bytes[offset..offset + table_length].to_vec();
                tables[slot] = drawing.create_resource(&table, 1, 1, 1).unwrap();
                table.fill(41);
                offset += table_length;
            }
        }
        let expected = &bytes[offset..offset + length];
        offset += length;
        let target = drawing.create_target(width, height).unwrap();
        let initial = drawing
            .create_resource(initial, width, height, width)
            .unwrap();
        let source_handle = drawing.create_resource(&source, 1, 1, 1).unwrap();
        source.fill(93);
        drawing
            .submit(
                target,
                &[Command {
                    kind: IMAGE,
                    width,
                    height,
                    source: initial,
                    source_width: width,
                    source_height: height,
                    ..Command::default()
                }],
            )
            .unwrap();
        let command = Command {
            kind: LENS_EFFECT,
            width,
            height,
            source: source_handle,
            clip_width: width,
            clip_height: height,
            start_low: tables[0] as u32,
            start_high: (tables[0] >> 32) as u32,
            step_low: tables[1] as u32,
            step_high: (tables[1] >> 32) as u32,
            ..Command::default()
        };
        if case == 0 {
            let before = drawing.readback(target).unwrap();
            assert!(
                drawing
                    .submit(
                        target,
                        &[
                            Command {
                                kind: CLEAR,
                                colour: 91,
                                width,
                                height,
                                ..Command::default()
                            },
                            command
                        ]
                    )
                    .is_err()
            );
            assert_eq!(drawing.readback(target).unwrap(), before);
        }
        drawing.submit(target, &[command]).unwrap();
        drawing.release_resource(source_handle).unwrap();
        assert_eq!(
            drawing.readback(target).unwrap(),
            expected,
            "native lens case {case}"
        );
        drawing.release_target(target).unwrap();
        drawing.release_resource(initial).unwrap();
        for table in tables {
            if table != 0 {
                drawing.release_resource(table).unwrap();
            }
        }
    }
    assert_eq!(offset, bytes.len());
}

#[test]
#[ignore = "requires Metal/Vulkan"]
fn lens_device_limit_rejection_preserves_target_and_device() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_compute_workgroups_per_dimension: 1,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    let mut drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    for (width, height) in [(16, 8), (8, 16)] {
        let target = drawing.create_target(width, height).unwrap();
        let before = drawing.readback(target).unwrap();
        let header = [
            2u32,
            width,
            height,
            width,
            width,
            0,
            0,
            65536 / width,
            65536 / height,
            256,
            0,
            64,
            64 + width * height,
            65 + width * height,
            1,
            1,
        ];
        let mut bytes: Vec<u8> = header.into_iter().flat_map(u32::to_le_bytes).collect();
        bytes.resize((65 + width * height) as usize, 67);
        let source = drawing.create_resource(&bytes, 1, 1, 1).unwrap();
        let mut command = Command {
            kind: LENS_EFFECT,
            width,
            height,
            clip_width: width,
            clip_height: height,
            source,
            ..Default::default()
        };
        let error = drawing.submit(target, &[command]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("lens dispatch exceeds device limit"),
            "{error}"
        );
        assert_eq!(drawing.readback(target).unwrap(), before);
        drawing.release_resource(source).unwrap();
        bytes[24] = 1;
        command.source = drawing.create_resource(&bytes, 1, 1, 1).unwrap();
        drawing.submit(target, &[command]).unwrap();
        drawing.release_resource(command.source).unwrap();
        assert_eq!(
            drawing.readback(target).unwrap(),
            vec![67; (width * height) as usize]
        );
        drawing.release_target(target).unwrap();
    }
}

/// A named lens map is resident: the second frame reads it instead of re-uploading it,
/// and the pixels are the ones the unsplit packing produced.
#[test]
#[ignore = "requires a GPU"]
fn a_named_lens_map_stays_resident() {
    let (width, height) = (2u32, 2u32);
    let header: [u32; 16] = [
        2,
        width,
        height,
        width,
        width,
        0,
        0,
        (width << 16) / width,
        (height << 16) / height,
        256,
        0,
        64,
        0,
        0,
        width,
        height,
    ];
    let mut source: Vec<u8> = header.into_iter().flat_map(u32::to_le_bytes).collect();
    source.extend([61, 62, 63, 64]);
    let overlay = vec![7u8, 8, 9, 10];
    let mut drawing = DrawRenderer::headless().unwrap();
    let map = drawing.create_resource(&overlay, 1, 1, 1).unwrap();
    let target = drawing.create_target(width, height).unwrap();
    let mut hits = 0;
    for _ in 0..2 {
        let handle = drawing.create_resource(&source, 1, 1, 1).unwrap();
        drawing
            .submit(
                target,
                &[Command {
                    kind: CLEAR,
                    colour: 3,
                    width,
                    height,
                    ..Command::default()
                }],
            )
            .unwrap();
        drawing
            .submit(
                target,
                &[Command {
                    kind: LENS_EFFECT,
                    width,
                    height,
                    clip_width: width,
                    clip_height: height,
                    source: handle,
                    start_low: map as u32,
                    start_high: (map >> 32) as u32,
                    ..Command::default()
                }],
            )
            .unwrap();
        assert_eq!(drawing.readback(target).unwrap(), overlay);
        drawing.release_resource(handle).unwrap();
        hits = drawing.counters().arena_by_kind[arena_kinds::LENS_KIND].hits;
    }
    assert!(hits > 0, "the lens map was uploaded on every frame");
    drawing.release_target(target).unwrap();
    drawing.release_resource(map).unwrap();
}
