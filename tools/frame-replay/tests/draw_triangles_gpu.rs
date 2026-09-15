use anyhow::{Context, Result, ensure};
use keeperfx_frame_replay::{
    draw::{ABI_VERSION, CLEAR, Command, DrawRenderer, TriangleCommand},
    gpoly::Vertex,
};

#[test]
#[ignore = "requires Metal/Vulkan and KFX_GPOLY_TRIANGLE_FIXTURE"]
fn original_vertex_production_order_resources_and_rejection() -> Result<()> {
    let bytes = std::fs::read(std::env::var("KFX_GPOLY_TRIANGLE_FIXTURE")?)?;
    let mut data = &bytes[8..];
    ensure!(&bytes[..8] == b"KFXGTRI1", "fixture magic");
    fn word(data: &mut &[u8]) -> u32 {
        let result = u32::from_le_bytes(data[..4].try_into().unwrap());
        *data = &data[4..];
        result
    }
    let width = word(&mut data);
    let height = word(&mut data);
    let pitch = word(&mut data);
    let count = word(&mut data);
    let mut drawing = DrawRenderer::headless()?;
    let target = drawing.create_target(width, height)?;
    let mut texture = data[..7968].to_vec();
    let source = drawing.create_resource(&texture, 32, 32, 256)?;
    texture.fill(0);
    let mut fade = data[7968..7968 + 16384].to_vec();
    let table = drawing.create_resource(&fade, 256, 64, 256)?;
    for byte in &mut fade {
        *byte = byte.wrapping_add(1);
    }
    let table2 = drawing.create_resource(&fade, 256, 64, 256)?;
    fade.fill(0);
    data = &data[7968 + 16384..];
    let unbinned = drawing.create_target(width, height)?;
    let mut commands = Vec::new();
    let mut every = Vec::new();
    let mut expected = vec![167; (width * height) as usize];
    for into in [target, unbinned] {
        drawing.submit(
            into,
            &[Command {
                kind: CLEAR,
                colour: 167,
                ..Default::default()
            }],
        )?;
    }
    for triangle in 0..count {
        let vertices = std::array::from_fn(|_| {
            let x = word(&mut data) as i32;
            let y = word(&mut data) as i32;
            let mut attr = || {
                let lo = word(&mut data);
                let hi = word(&mut data);
                (u64::from(lo) | (u64::from(hi) << 32)) as i64
            };
            Vertex {
                x,
                y,
                u: attr(),
                v: attr(),
                shade: attr(),
            }
        });
        let mut bounds = Vec::new();
        for _ in 0..height {
            let row: [u32; 8] = std::array::from_fn(|_| word(&mut data));
            bounds.push(row);
        }
        let pixels = &data[..(pitch * height) as usize];
        data = &data[(pitch * height) as usize..];
        for row in bounds {
            for x in row[0]..row[0] + row[2] {
                expected[(row[1] * width + x) as usize] =
                    pixels[(row[1] * pitch + x) as usize].wrapping_add((triangle % 2) as u8);
            }
        }
        commands.push(TriangleCommand {
            abi_version: ABI_VERSION,
            reserved: 0,
            source,
            table: if triangle % 2 == 0 { table } else { table2 },
            vertices,
        });
        every.push(*commands.last().unwrap());
        if commands.len() == 64 || triangle + 1 == count {
            drawing.submit_triangles(target, &commands)?;
            ensure!(
                drawing.readback(target)? == expected,
                "native ordered original-vertex mismatch at triangle {triangle}"
            );
            commands.clear();
        }
    }
    ensure!(data.is_empty(), "trailing fixture bytes");
    // The same geometry through an index that reaches every tile: binning changes
    // addressing, not arithmetic, so the two targets must be byte-equal.
    drawing.bin_records(false);
    for batch in every.chunks(64) {
        drawing.submit_triangles(unbinned, batch)?;
    }
    drawing.bin_records(true);
    ensure!(
        drawing.readback(unbinned)? == expected,
        "the unbinned raster disagrees with the binned one"
    );
    let vertices = [
        Vertex {
            x: 2,
            y: 2,
            u: 0,
            v: 0,
            shade: 31 << 16,
        },
        Vertex {
            x: 50,
            y: 2,
            u: 0,
            v: 0,
            shade: 31 << 16,
        },
        Vertex {
            x: 2,
            y: 40,
            u: 0,
            v: 0,
            shade: 31 << 16,
        },
    ];
    let valid = TriangleCommand {
        abi_version: ABI_VERSION,
        reserved: 0,
        source,
        table,
        vertices,
    };
    let mut invalid = valid;
    for v in &mut invalid.vertices {
        v.shade = 64 << 16;
    }
    drawing.submit_triangles(target, &[valid])?;
    let drawn = drawing.readback(target)?;
    ensure!(drawn != expected, "the probe triangle must write pixels");
    ensure!(
        drawing.frame_status().1 == 0,
        "a valid triangle raised a frame flag"
    );
    drawing.submit_triangles(target, &[valid, invalid])?;
    ensure!(
        drawing.readback(target)? == drawn,
        "a flagged shade must leave the target as the valid triangle drew it"
    );
    let (_, flags) = drawing.frame_status();
    ensure!(flags & 1 != 0, "an invalid shade raised no frame flag");
    ensure!(
        flags & (1 << 2) != 0,
        "the per-pixel shade check must report"
    );
    invalid = valid;
    invalid.source = u64::MAX;
    ensure!(
        drawing.submit_triangles(target, &[valid, invalid]).is_err(),
        "invalid resource accepted"
    );
    invalid = valid;
    invalid.vertices[0].x = 32768;
    ensure!(
        drawing.submit_triangles(target, &[valid, invalid]).is_err(),
        "invalid vertex accepted"
    );
    ensure!(
        drawing.readback(target)? == drawn,
        "rejected batch changed target"
    );
    drawing.release_resource(source)?;
    ensure!(
        drawing.submit_triangles(target, &[valid]).is_err(),
        "released resource accepted"
    );
    ensure!(
        drawing.readback(target)? == drawn,
        "released resource rejection changed target"
    );
    ensure!(
        drawing.frame_status().1 == 0,
        "host rejections must not raise the frame flag"
    );
    eprintln!(
        "PASS: {count} original native triangles binned and unbinned through production DrawRenderer, ordered overlapping batches, immutable mutated assets, flagged late shade, and host-rejected resource/vertex"
    );
    Ok(())
}

#[test]
#[ignore = "requires Metal/Vulkan"]
fn triangle_device_limit_rejection_preserves_target_and_device() -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    for (width, height, limits, message) in [
        (
            8,
            8,
            wgpu::Limits {
                max_buffer_size: 65_536,
                ..Default::default()
            },
            "triangle assets exceed buffer limit",
        ),
        (
            16,
            8,
            wgpu::Limits {
                max_compute_workgroups_per_dimension: 1,
                ..Default::default()
            },
            "triangle pixel dispatch",
        ),
        (
            8,
            16,
            wgpu::Limits {
                max_compute_workgroups_per_dimension: 1,
                ..Default::default()
            },
            "triangle pixel dispatch",
        ),
    ] {
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            }))?;
        let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue)?;
        let mut drawing = DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm)?;
        let target = drawing.create_target(width, height)?;
        let initial = drawing.readback(target)?;
        let source = drawing.create_resource(&[0; 7968], 32, 32, 256)?;
        let table = drawing.create_resource(&[0; 16384], 256, 64, 256)?;
        let command = TriangleCommand {
            abi_version: ABI_VERSION,
            reserved: 0,
            source,
            table,
            vertices: [
                Vertex {
                    x: 0,
                    y: 0,
                    u: 0,
                    v: 0,
                    shade: 0,
                },
                Vertex {
                    x: 8,
                    y: 0,
                    u: 0,
                    v: 0,
                    shade: 0,
                },
                Vertex {
                    x: 0,
                    y: 8,
                    u: 0,
                    v: 0,
                    shade: 0,
                },
            ],
        };
        let error = drawing
            .submit_triangles(target, &[command])
            .err()
            .context("triangle submission exceeded device limit without rejection")?;
        ensure!(error.to_string().contains(message), "{error}");
        ensure!(
            drawing.readback(target)? == initial,
            "rejected batch changed target"
        );
        let small_target = drawing.create_target(8, 8)?;
        drawing.submit(
            small_target,
            &[Command {
                kind: CLEAR,
                colour: 23,
                ..Default::default()
            }],
        )?;
        ensure!(
            drawing.readback(small_target)? == [23; 64],
            "triangle rejection poisoned device"
        );
    }
    Ok(())
}
