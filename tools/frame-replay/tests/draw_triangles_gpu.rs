use anyhow::{Result, ensure};
use keeperfx_frame_replay::{
    draw::{CLEAR, Command, DrawRenderer, TriangleCommand},
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
    let mut commands = Vec::new();
    let mut expected = vec![167; (width * height) as usize];
    drawing.submit(
        target,
        &[Command {
            kind: CLEAR,
            colour: 167,
            ..Default::default()
        }],
    )?;
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
            abi_version: 1,
            reserved: 0,
            source,
            table: if triangle % 2 == 0 { table } else { table2 },
            vertices,
        });
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
        abi_version: 1,
        reserved: 0,
        source,
        table,
        vertices,
    };
    let mut invalid = valid;
    for v in &mut invalid.vertices {
        v.shade = 64 << 16;
    }
    ensure!(
        drawing.submit_triangles(target, &[valid, invalid]).is_err(),
        "late invalid shade accepted"
    );
    ensure!(
        drawing.readback(target)? == expected,
        "shade rejection changed target"
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
        drawing.readback(target)? == expected,
        "rejected batch changed target"
    );
    drawing.release_resource(source)?;
    ensure!(
        drawing.submit_triangles(target, &[valid]).is_err(),
        "released resource accepted"
    );
    ensure!(
        drawing.readback(target)? == expected,
        "released resource rejection changed target"
    );
    eprintln!(
        "PASS: {count} original native triangles through production DrawRenderer, ordered overlapping batches, immutable mutated assets, late invalid shade/resource/vertex atomic rejection"
    );
    Ok(())
}
