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
