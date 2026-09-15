use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, IMAGE, RECT};

#[test]
#[ignore = "requires GPU adapter"]
fn rejected_allocations_preserve_target_and_device() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let limits = wgpu::Limits {
        max_buffer_size: 65_536,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    let mut drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let target = drawing.create_target(128, 128).unwrap();
    drawing
        .submit(
            target,
            &[Command {
                kind: CLEAR,
                colour: 167,
                ..Default::default()
            }],
        )
        .unwrap();
    assert!(drawing.create_target(129, 129).is_err());
    assert!(drawing.create_resource(&vec![0; 16385], 1, 1, 1).is_err());
    let resource1 = drawing
        .create_resource(&vec![71; 9000], 90, 100, 90)
        .unwrap();
    let resource2 = drawing
        .create_resource(&vec![23; 9000], 90, 100, 90)
        .unwrap();
    let image = Command {
        kind: IMAGE,
        width: 128,
        height: 128,
        source_width: 90,
        source_height: 100,
        ..Default::default()
    };
    assert!(
        drawing
            .submit(
                target,
                &[
                    Command {
                        source: resource1,
                        ..image
                    },
                    Command {
                        source: resource2,
                        ..image
                    }
                ]
            )
            .is_err()
    );
    let rectangle = Command {
        kind: RECT,
        width: 128,
        height: 128,
        colour: 9,
        ..Default::default()
    };
    assert!(drawing.submit(target, &vec![rectangle; 600]).is_err());
    assert!(drawing.submit(target, &vec![rectangle; 300]).is_err());
    assert_eq!(drawing.readback(target).unwrap(), vec![167; 128 * 128]);
    drawing
        .submit(
            target,
            &[Command {
                kind: CLEAR,
                colour: 29,
                ..Default::default()
            }],
        )
        .unwrap();
    assert_eq!(drawing.readback(target).unwrap(), vec![29; 128 * 128]);
    renderer.check_status().unwrap();
}

#[test]
#[ignore = "requires GPU adapter"]
fn rejected_dispatch_preserves_target_and_device() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let limits = wgpu::Limits {
        max_compute_workgroups_per_dimension: 1,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    let mut drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let target = drawing.create_target(9, 8).unwrap();
    let error = drawing
        .submit(
            target,
            &[Command {
                kind: CLEAR,
                colour: 167,
                ..Default::default()
            }],
        )
        .unwrap_err();
    assert!(error.to_string().contains("dispatch exceeds device limit"));
    assert_eq!(drawing.readback(target).unwrap(), vec![0; 9 * 8]);
    let small = drawing.create_target(8, 8).unwrap();
    drawing
        .submit(
            small,
            &[Command {
                kind: CLEAR,
                colour: 29,
                ..Default::default()
            }],
        )
        .unwrap();
    assert_eq!(drawing.readback(small).unwrap(), vec![29; 8 * 8]);
    renderer.check_status().unwrap();
}

fn arena_drawing(binding: u64) -> Option<DrawRenderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: binding,
            ..Default::default()
        },
        ..Default::default()
    }))
    .ok()?;
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).ok()?;
    DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).ok()
}

fn tile(colour: u32, size: u32) -> Vec<u8> {
    (0..size * size)
        .map(|i| (colour.wrapping_add(i) & 255) as u8)
        .collect()
}

fn blit(source: u64, size: u32) -> Command {
    Command {
        kind: IMAGE,
        source,
        width: size,
        height: size,
        source_width: size,
        source_height: size,
        ..Default::default()
    }
}

#[test]
#[ignore = "requires GPU adapter"]
fn arena_residency_reuse_eviction_and_generations() {
    let mut drawing = arena_drawing(32 << 20).unwrap();
    let target = drawing.create_target(64, 64).unwrap();
    let sources: Vec<_> = (0..4)
        .map(|i: u32| {
            drawing
                .create_resource(&tile(11 * i + 3, 64), 64, 64, 64)
                .unwrap()
        })
        .collect();
    let batch: Vec<_> = sources.iter().map(|&s| blit(s, 64)).collect();
    drawing.submit(target, &batch).unwrap();
    let expected = drawing.readback(target).unwrap();
    assert!(expected.iter().any(|&pixel| pixel != 0));
    let first = drawing.arena_counters();
    assert!(first.bytes_uploaded > 0 && first.bytes_resident > 0);

    drawing.submit(target, &batch).unwrap();
    assert!(drawing.readback(target).unwrap() == expected);
    assert_eq!(
        drawing.arena_counters().bytes_uploaded,
        first.bytes_uploaded
    );

    let generation = drawing.create_target(8, 8).unwrap();
    drawing.frame_begin(generation).unwrap();
    drawing.frame_abort().unwrap();
    drawing.submit(target, &batch).unwrap();
    assert!(drawing.readback(target).unwrap() == expected);
    let stale = drawing.arena_counters();
    assert_eq!(stale.bytes_uploaded, 2 * first.bytes_uploaded);
    assert_eq!(stale.bytes_resident, first.bytes_resident);

    let bulk: Vec<_> = (0..(160 / keeperfx_frame_replay::draw::assets::STRIDE as u32))
        .map(|i: u32| {
            drawing
                .create_resource(&tile(7 * i + 1, 512), 512, 512, 512)
                .unwrap()
        })
        .collect();
    for &source in &bulk {
        drawing.submit(target, &[blit(source, 64)]).unwrap();
    }
    assert!(drawing.arena_counters().evictions > 0);
    assert_eq!(drawing.arena_counters().overflows, 0);
    drawing.submit(target, &batch).unwrap();
    assert!(drawing.readback(target).unwrap() == expected);
    assert!(drawing.arena_counters().bytes_uploaded > stale.bytes_uploaded);
}

#[test]
#[ignore = "requires GPU adapter"]
fn arena_overflow_reports_the_storage_limit_and_recovers() {
    let mut drawing = arena_drawing(32 << 20).unwrap();
    let target = drawing.create_target(64, 64).unwrap();
    let odd = drawing.create_resource(&tile(5, 3), 3, 3, 3).unwrap();
    drawing.submit(target, &[blit(odd, 3)]).unwrap();
    let aligned = drawing.readback(target).unwrap();

    let batch: Vec<_> = (0..(560 / keeperfx_frame_replay::draw::assets::STRIDE as u32))
        .map(|i: u32| {
            let source = drawing
                .create_resource(&tile(3 * i + 1, 256), 256, 256, 256)
                .unwrap();
            blit(source, 64)
        })
        .collect();
    let error = drawing.submit(target, &batch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("asset batch exceeds storage limit"),
        "{error}"
    );
    assert_eq!(drawing.arena_counters().overflows, 1);
    assert!(drawing.readback(target).unwrap() == aligned);

    drawing.submit(target, &batch[..8]).unwrap();
    assert!(drawing.readback(target).unwrap() != aligned);
    drawing
        .submit(
            target,
            &[
                Command {
                    kind: CLEAR,
                    ..Default::default()
                },
                blit(odd, 3),
            ],
        )
        .unwrap();
    assert!(drawing.readback(target).unwrap() == aligned);
}
