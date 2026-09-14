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

struct Cycle {
    root: Vec<u8>,
    presented: Vec<u8>,
    submits: u64,
    checkpoints: u64,
}

/// Backup, composition, palette pass and restore over a flushed frame, either
/// recorded into one present tail or submitted a step at a time.
fn cursor_cycle(
    renderer: &keeperfx_frame_replay::gpu::Renderer,
    draw: &mut DrawRenderer,
    per_step: bool,
    present: bool,
) -> Cycle {
    let (width, height) = (16u32, 12u32);
    let (cursor_width, cursor_height) = (5u32, 4u32);
    let (x, y) = (6u32, 5u32);
    let root = draw.create_target(width, height).unwrap();
    let background = draw.create_target(cursor_width, cursor_height).unwrap();
    let (sprite_target, _) = pattern(draw, cursor_width, cursor_height);
    let sprite = draw
        .create_target_snapshot(
            sprite_target,
            0,
            0,
            cursor_width,
            cursor_height,
            cursor_width,
        )
        .unwrap();
    draw.tail_submit();
    draw.frame_begin(root).unwrap();
    let rows: Vec<_> = (0..width * height)
        .map(|i| Command {
            kind: RECT,
            colour: u32::from((i * 29 + 3) as u8),
            x: (i % width) as i32,
            y: (i / width) as i32,
            width: 1,
            height: 1,
            ..Default::default()
        })
        .collect();
    draw.submit(root, &rows).unwrap();
    draw.frame_flush().unwrap();
    let palette: Vec<u8> = (0..256)
        .flat_map(|i| [i as u8, (i * 3) as u8, (i * 7) as u8, 255])
        .collect();
    let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let submits = draw.counters().submits;
    let checkpoints = draw.frame_counters().checkpoints;
    let step = |draw: &mut DrawRenderer| {
        if per_step {
            draw.tail_submit();
        }
    };
    let part = draw
        .create_target_snapshot(root, x, y, cursor_width, cursor_height, cursor_width)
        .unwrap();
    step(draw);
    draw.submit_target_images(background, &[image(part, cursor_width, cursor_height)])
        .unwrap();
    step(draw);
    draw.release_target_snapshot(part).unwrap();
    let backup = draw
        .create_target_snapshot(background, 0, 0, cursor_width, cursor_height, cursor_width)
        .unwrap();
    step(draw);
    let composed = Command {
        x: x as i32,
        y: y as i32,
        transparent: 255,
        ..image(sprite, cursor_width, cursor_height)
    };
    draw.submit_target_images(root, &[composed]).unwrap();
    step(draw);
    if present {
        draw.present_into(
            root,
            &palette,
            width,
            height,
            &texture.create_view(&Default::default()),
        )
        .unwrap();
        step(draw);
    }
    let restored = Command {
        x: x as i32,
        y: y as i32,
        ..image(backup, cursor_width, cursor_height)
    };
    draw.submit_target_images(root, &[restored]).unwrap();
    draw.tail_submit();
    let cycle = Cycle {
        submits: draw.counters().submits - submits,
        checkpoints: draw.frame_counters().checkpoints - checkpoints,
        root: draw.readback(root).unwrap(),
        presented: read_texture(renderer, &texture, width, height),
    };
    draw.release_target_snapshot(backup).unwrap();
    draw.release_target_snapshot(sprite).unwrap();
    draw.frame_abort().unwrap();
    draw.release_target(root).unwrap();
    draw.release_target(background).unwrap();
    draw.release_target(sprite_target).unwrap();
    cycle
}

fn read_texture(
    renderer: &keeperfx_frame_replay::gpu::Renderer,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let stride = (width * 4).next_multiple_of(256);
    let staging = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * height),
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
                bytes_per_row: Some(stride),
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
    renderer
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = staging.slice(..).get_mapped_range().unwrap();
    let rows: Vec<_> = (0..height)
        .flat_map(|row| {
            let start = (row * stride) as usize;
            mapped[start..start + (width * 4) as usize].to_vec()
        })
        .collect();
    drop(mapped);
    staging.unmap();
    rows
}

#[test]
#[ignore = "requires GPU adapter"]
fn a_present_tail_matches_one_submit_per_step_and_adds_no_checkpoint() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    let mut draw = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    cursor_cycle(&renderer, &mut draw, true, true);
    let stepped = cursor_cycle(&renderer, &mut draw, true, true);
    let tailed = cursor_cycle(&renderer, &mut draw, false, true);
    assert_eq!(tailed.root, stepped.root);
    assert_eq!(tailed.presented, stepped.presented);
    assert_ne!(tailed.presented, vec![0; tailed.presented.len()]);
    assert_eq!(tailed.checkpoints, 0);
    assert_eq!(stepped.checkpoints, 0);
    assert_eq!(tailed.submits, 1);
    assert_eq!(stepped.submits, 6);
    // An acquisition the surface skipped still finishes the tail, so the cursor
    // background and the root stay where a presented frame leaves them.
    let skipped = cursor_cycle(&renderer, &mut draw, false, false);
    assert_eq!(skipped.root, stepped.root);
    assert_eq!(skipped.checkpoints, 0);
    assert_eq!(skipped.submits, 1);
    draw.check_status().unwrap();
}

#[test]
#[ignore = "requires GPU adapter"]
fn a_flush_under_an_open_tail_keeps_the_arena_region_the_tail_reads() {
    let mut draw = drawing(Default::default());
    let size = 8u32;
    let root = draw.create_target(size, size).unwrap();
    let background = draw.create_target(size, size).unwrap();
    let first: Vec<u8> = (0..size * size).map(|i| (i * 11 + 1) as u8).collect();
    let second: Vec<u8> = (0..size * size).map(|i| (i * 7 + 149) as u8).collect();
    let painted = |source| Command {
        transparent: keeperfx_frame_replay::draw::OPAQUE,
        ..image(source, size, size)
    };
    let before = draw.create_resource(&first, size, size, size).unwrap();
    let after = draw.create_resource(&second, size, size, size).unwrap();
    draw.frame_begin(root).unwrap();
    draw.submit(root, &[painted(before)]).unwrap();
    draw.frame_flush().unwrap();
    // The tail now holds a copy into an arena scratch region and a pass reading it.
    let snapshot = draw
        .create_target_snapshot(root, 0, 0, size, size, size)
        .unwrap();
    draw.submit_target_images(background, &[painted(snapshot)])
        .unwrap();
    // Replaying this needs a batch, whose arena allocation would otherwise reclaim
    // that region and stage its upload ahead of every command in the tail.
    draw.submit(root, &[painted(after)]).unwrap();
    draw.frame_flush().unwrap();
    draw.tail_submit();
    assert_eq!(draw.readback(background).unwrap(), first);
    assert_eq!(draw.readback(root).unwrap(), second);
    draw.release_target_snapshot(snapshot).unwrap();
    draw.frame_abort().unwrap();
    draw.check_status().unwrap();
}
