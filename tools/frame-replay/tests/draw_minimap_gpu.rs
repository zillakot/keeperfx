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
        draw.submit(target, &[c]).unwrap();
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
