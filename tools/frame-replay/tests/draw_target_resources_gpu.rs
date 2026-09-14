use keeperfx_frame_replay::draw::{CLEAR, Command, DrawRenderer, IMAGE, RECT};

fn drawing(limits: wgpu::Limits) -> DrawRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let info = adapter.get_info();
    eprintln!("snapshot adapter: {} {:?}", info.name, info.backend);
    #[cfg(target_os = "macos")]
    assert_eq!(info.backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap()
}

fn image(source: u64, width: u32, height: u32) -> Command {
    Command {
        kind: IMAGE,
        source,
        width,
        height,
        source_width: width,
        source_height: height,
        ..Default::default()
    }
}

fn pattern(draw: &mut DrawRenderer, width: u32, height: u32) -> (u64, Vec<u8>) {
    let target = draw.create_target(width, height).unwrap();
    let pixels: Vec<_> = (0..width * height).map(|i| (i * 37) as u8).collect();
    let commands: Vec<_> = pixels
        .iter()
        .enumerate()
        .map(|(i, &colour)| Command {
            kind: RECT,
            colour: u32::from(colour),
            x: (i as u32 % width) as i32,
            y: (i as u32 / width) as i32,
            width: 1,
            height: 1,
            ..Default::default()
        })
        .collect();
    draw.submit(target, &commands).unwrap();
    (target, pixels)
}

fn reference(
    destination: &mut [u8],
    width: u32,
    command: &Command,
    source: &[u8],
    pitch: u32,
    table: &[u8],
) {
    for (i, pixel) in destination.iter_mut().enumerate() {
        let x = (i as u32 % width) as i32;
        let y = (i as u32 / width) as i32;
        if x < command.x
            || y < command.y
            || x >= command.x + command.width as i32
            || y >= command.y + command.height as i32
            || x < command.clip_x
            || y < command.clip_y
            || x >= command.clip_x + command.clip_width as i32
            || y >= command.clip_y + command.clip_height as i32
        {
            continue;
        }
        let sx = command.source_x + (x - command.x) as u32 * command.source_width / command.width;
        let sy = command.source_y + (y - command.y) as u32 * command.source_height / command.height;
        let colour = source[(sy * pitch + sx) as usize];
        if u32::from(colour) == command.transparent {
            continue;
        }
        *pixel = match command.blend {
            1 => table[colour as usize * 256 + *pixel as usize],
            2 => table[*pixel as usize * 256 + colour as usize],
            _ => colour,
        };
    }
}

#[test]
#[ignore = "requires GPU adapter"]
fn snapshots_preserve_order_regions_pitch_versions_and_queued_lifetimes() {
    let mut draw = drawing(Default::default());
    let (target, pixels) = pattern(&mut draw, 13, 7);
    let full = draw
        .create_target_snapshot(target, 0, 0, 13, 7, 17)
        .unwrap();
    let region = draw.create_target_snapshot(target, 3, 2, 5, 3, 9).unwrap();
    assert_eq!(draw.target_snapshot_dimensions(region).unwrap(), (5, 3, 9));
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 255,
            ..Default::default()
        }],
    )
    .unwrap();
    let later = draw
        .create_target_snapshot(target, 0, 0, 13, 7, 13)
        .unwrap();
    draw.release_target(target).unwrap();
    let output = draw.create_target(13, 7).unwrap();
    draw.submit_target_images(output, &[image(full, 13, 7)])
        .unwrap();
    draw.release_target_snapshot(full).unwrap();
    let mut expected = pixels;
    let cropped = Command {
        x: -1,
        y: 1,
        width: 11,
        height: 5,
        clip_x: 2,
        clip_y: 2,
        clip_width: 6,
        clip_height: 4,
        source_x: 1,
        source_y: 0,
        source_width: 4,
        source_height: 3,
        transparent: 151,
        ..image(region, 5, 3)
    };
    draw.submit_target_images(output, &[cropped]).unwrap();
    let mut region_pixels = vec![0; 27];
    for row in 0..3 {
        for col in 0..5 {
            region_pixels[row * 9 + col] = ((row + 2) * 13 * 37 + (col + 3) * 37) as u8;
        }
    }
    reference(&mut expected, 13, &cropped, &region_pixels, 9, &[]);
    draw.release_target_snapshot(region).unwrap();
    assert_eq!(draw.counters().asset_upload_bytes, 0);
    assert_eq!(draw.counters().readback_bytes, 0);
    assert_eq!(draw.readback(output).unwrap(), expected);
    draw.submit_target_images(output, &[image(later, 13, 7)])
        .unwrap();
    draw.release_target_snapshot(later).unwrap();
    assert_eq!(draw.readback(output).unwrap(), vec![255; 91]);
    assert_eq!(draw.target_resource_counters().snapshots, 3);
    assert_eq!(
        draw.target_resource_counters().snapshot_copy_bytes,
        (91 + 15 + 91) * 4
    );
    assert_eq!(
        draw.target_resource_counters().sampling_copy_bytes,
        (119 + 27 + 91) * 4
    );
    draw.check_status().unwrap();
}

#[test]
#[ignore = "requires GPU adapter"]
fn overlapping_images_use_immutable_sources_and_ordered_blend_destinations() {
    let mut draw = drawing(Default::default());
    let (target, original) = pattern(&mut draw, 8, 8);
    let snapshot = draw.create_target_snapshot(target, 0, 0, 8, 8, 8).unwrap();
    let table: Vec<_> = (0..65536u32)
        .map(|i| ((i / 256 * 3) ^ (i % 256 * 7)) as u8)
        .collect();
    let resource = draw.create_resource(&table, 256, 256, 256).unwrap();
    let commands = [
        Command {
            x: 1,
            y: 1,
            width: 7,
            height: 7,
            source_width: 7,
            source_height: 7,
            ..image(snapshot, 8, 8)
        },
        Command {
            x: -1,
            y: -1,
            width: 8,
            height: 8,
            blend: 1,
            table: resource,
            ..image(snapshot, 8, 8)
        },
        Command {
            x: 2,
            y: 2,
            width: 6,
            height: 6,
            blend: 2,
            table: resource,
            transparent: 0,
            ..image(snapshot, 8, 8)
        },
    ];
    let mut expected = original.clone();
    for command in &commands {
        reference(&mut expected, 8, command, &original, 8, &table);
    }
    draw.submit_target_images(target, &commands).unwrap();
    let after = draw.create_target_snapshot(target, 0, 0, 8, 8, 8).unwrap();
    draw.release_target_snapshot(snapshot).unwrap();
    draw.release_resource(resource).unwrap();
    draw.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 17,
            ..Default::default()
        }],
    )
    .unwrap();
    draw.submit_target_images(target, &[image(after, 8, 8)])
        .unwrap();
    draw.release_target_snapshot(after).unwrap();
    assert_eq!(draw.counters().readback_bytes, 0);
    assert_eq!(draw.counters().asset_upload_bytes, 65536 * 4);
    assert_eq!(
        draw.target_resource_counters().sampling_copy_bytes,
        64 * 4 * 2
    );
    assert_eq!(draw.readback(target).unwrap(), expected);
}

#[test]
#[ignore = "requires GPU adapter"]
fn invalid_snapshot_batches_are_atomic_and_context_scoped() {
    let mut draw = drawing(Default::default());
    let mut other = drawing(Default::default());
    let (target, expected) = pattern(&mut draw, 8, 8);
    let source = draw.create_target_snapshot(target, 0, 0, 8, 8, 8).unwrap();
    let foreign_target = other.create_target(8, 8).unwrap();
    let foreign_source = other
        .create_target_snapshot(foreign_target, 0, 0, 8, 8, 8)
        .unwrap();
    let cpu_source = draw.create_resource(&[42; 64], 8, 8, 8).unwrap();
    let malformed = [
        Command {
            source: foreign_source,
            ..image(source, 8, 8)
        },
        Command {
            source: cpu_source,
            ..image(source, 8, 8)
        },
        Command {
            source_x: u32::MAX,
            ..image(source, 8, 8)
        },
        Command {
            source_width: 9,
            ..image(source, 8, 8)
        },
        Command {
            width: 0,
            ..image(source, 8, 8)
        },
        Command {
            kind: CLEAR,
            ..image(source, 8, 8)
        },
        Command {
            blend: 1,
            table: foreign_source,
            ..image(source, 8, 8)
        },
        Command {
            blend: 1,
            table: cpu_source,
            ..image(source, 8, 8)
        },
        Command {
            reserved: [1, 0, 0],
            ..image(source, 8, 8)
        },
        Command {
            abi_version: 2,
            ..image(source, 8, 8)
        },
        Command {
            clip_x: i32::MIN,
            ..image(source, 8, 8)
        },
    ];
    let first = Command {
        x: 1,
        width: 7,
        source_width: 7,
        ..image(source, 8, 8)
    };
    let before = draw.counters();
    for bad in malformed {
        assert!(draw.submit_target_images(target, &[first, bad]).is_err());
    }
    assert_eq!(draw.counters().commands, before.commands);
    assert_eq!(draw.target_resource_counters().sampling_copy_bytes, 0);
    assert!(draw.submit_target_images(foreign_target, &[first]).is_err());
    assert!(
        draw.create_target_snapshot(foreign_target, 0, 0, 8, 8, 8)
            .is_err()
    );
    for (x, y, w, h, pitch) in [
        (8, 0, 1, 1, 1),
        (0, 8, 1, 1, 1),
        (0, 0, 0, 1, 1),
        (0, 0, 8, 8, 7),
        (u32::MAX, 0, 1, 1, 1),
    ] {
        assert!(
            draw.create_target_snapshot(target, x, y, w, h, pitch)
                .is_err()
        );
    }
    assert!(draw.release_target_snapshot(foreign_source).is_err());
    assert!(draw.release_target_snapshot(cpu_source).is_err());
    assert!(draw.release_resource(source).is_err());
    assert_eq!(draw.readback(target).unwrap(), expected);
    draw.release_target_snapshot(source).unwrap();
    assert!(draw.submit_target_images(target, &[first]).is_err());
    assert!(draw.release_target_snapshot(source).is_err());
    draw.check_status().unwrap();
}

#[test]
#[ignore = "requires GPU adapter"]
fn snapshot_resource_and_dispatch_limits_leave_targets_unchanged() {
    let mut draw = drawing(wgpu::Limits {
        max_buffer_size: 65_536,
        ..Default::default()
    });
    let target = draw.create_target(128, 128).unwrap();
    let one = draw
        .create_target_snapshot(target, 0, 0, 90, 100, 90)
        .unwrap();
    let two = draw
        .create_target_snapshot(target, 0, 0, 90, 100, 90)
        .unwrap();
    assert!(
        draw.create_target_snapshot(target, 0, 0, 128, 128, 129)
            .is_err()
    );
    assert!(
        draw.submit_target_images(target, &[image(one, 90, 100), image(two, 90, 100)])
            .is_err()
    );
    assert!(
        draw.submit_target_images(target, &vec![image(one, 90, 100); 600])
            .is_err()
    );
    assert!(
        draw.submit_target_images(target, &vec![image(one, 90, 100); 400])
            .is_err()
    );
    assert_eq!(draw.readback(target).unwrap(), vec![0; 128 * 128]);
    draw.check_status().unwrap();
    let mut draw = drawing(wgpu::Limits {
        max_compute_workgroups_per_dimension: 1,
        ..Default::default()
    });
    let target = draw.create_target(9, 8).unwrap();
    let snapshot = draw.create_target_snapshot(target, 0, 0, 9, 8, 9).unwrap();
    assert!(
        draw.submit_target_images(target, &[image(snapshot, 9, 8)])
            .is_err()
    );
    assert_eq!(draw.target_resource_counters().sampling_copy_bytes, 0);
    assert_eq!(draw.readback(target).unwrap(), vec![0; 9 * 8]);
    let small = draw.create_target(8, 8).unwrap();
    draw.submit_target_images(small, &[image(snapshot, 8, 8)])
        .unwrap();
    draw.check_status().unwrap();
}
