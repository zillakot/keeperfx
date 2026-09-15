use super::{Command, OPAQUE, Resource};
use anyhow::{Result, ensure};

pub(super) fn validate(c: &Command, source: &Resource, width: u32, height: u32) -> Result<()> {
    ensure!(
        c.blend == 0 && c.transparent == OPAQUE,
        "invalid map options"
    );
    match c.source_x {
        0 => {
            ensure!(
                (1..=2048).contains(&c.source_width) && (1..=2048).contains(&c.source_height),
                "invalid map row size"
            );
            ensure!(
                c.width == c.source_width * c.source_height
                    && c.height == c.source_height
                    && c.x >= 0
                    && c.y >= 0
                    && c.x as u64 + c.width as u64 <= width as u64
                    && c.y as u64 + c.height as u64 <= height as u64,
                "invalid map row bounds"
            );
            let count = c.source_width as usize;
            ensure!(source.bytes.len() == count * 2, "invalid map style count");
            ensure!(
                source.bytes[..count * 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|v| u16::from_le_bytes([v[0], v[1]]) <= 262),
                "invalid map style"
            );
        }
        1 => {
            ensure!(
                (1..=640).contains(&c.width)
                    && (1..=480).contains(&c.height)
                    && c.source_y <= 0x70
                    && c.source_y & 15 == 0
                    && source.bytes.len() == 8448,
                "invalid map texture"
            );
        }
        2 => {
            let (x, y, mx, my, d) = (
                c.start_low as u64,
                c.start_high as u64,
                c.step_low as u64,
                c.step_high as u64,
                c.source_y as u64,
            );
            ensure!(
                c.x == 0
                    && c.y == 0
                    && c.width >= 2
                    && c.width <= width
                    && c.height == height
                    && height >= 2
                    && x >= 1
                    && x < c.width as u64
                    && y >= 1
                    && y < height as u64
                    && d <= 4096,
                "invalid map zoom bounds"
            );
            ensure!(
                c.source_width > 0
                    && c.source_height > 0
                    && c.source_width <= 8192
                    && c.source_height <= 8192
                    && source.bytes.len() == c.source_width as usize * c.source_height as usize,
                "invalid map zoom asset"
            );
            ensure!(
                mx >= (((x + 1) * d) >> 8)
                    && my >= ((y * d) >> 8)
                    && mx + 1 + (((c.width as u64 - x) * d) >> 8) < c.source_width as u64
                    && my + 1 + (((height as u64 - y) * d) >> 8) < c.source_height as u64,
                "map zoom source overrun"
            );
        }
        3 => {
            ensure!(
                c.x == 0
                    && c.y == 0
                    && c.width == width
                    && c.height == height
                    && (1..=36).contains(&c.source_y)
                    && source.bytes.len() == c.source_y as usize * 8
                    && c.step_high <= 1,
                "invalid map marker"
            );
            let (x, y, spread) = (
                c.start_low as i32 as i64,
                c.start_high as i32 as i64,
                c.step_low as i32 as i64,
            );
            ensure!(
                (-16384..=16384).contains(&x)
                    && (-16384..=16384).contains(&y)
                    && (-4096..=4096).contains(&spread),
                "invalid marker coordinates"
            );
            for point in source.bytes.as_chunks::<8>().0 {
                let dx = i32::from_le_bytes(point[..4].try_into().unwrap()) as i64;
                let dy = i32::from_le_bytes(point[4..].try_into().unwrap()) as i64;
                let offset = (y + dy) * width as i64 + x + dx;
                let margin = c.step_high as i64 * spread.abs() * width as i64;
                ensure!(
                    (-32..=32).contains(&dx)
                        && (-32..=32).contains(&dy)
                        && offset - margin >= 0
                        && offset + margin < width as i64 * height as i64,
                    "marker source overrun"
                );
            }
        }
        _ => anyhow::bail!("unknown map operation"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_shader_validates_without_gpu() {
        let source = super::super::assets::shader(super::super::DRAW_SHADER);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn semantic_resource_rejection_without_gpu() {
        let mut source = Resource {
            cursor: false,
            width: 2,
            height: 1,
            pitch: 2,
            bytes: vec![0; 2],
            key: None,
        };
        let mut command = Command {
            kind: super::super::MAP_VIEW,
            width: 2,
            height: 2,
            source_width: 1,
            source_height: 2,
            ..Default::default()
        };
        assert!(validate(&command, &source, 2, 2).is_ok());
        source.bytes[1] = 2;
        assert!(validate(&command, &source, 2, 2).is_err());
        source.bytes[1] = 1;
        assert!(validate(&command, &source, 2, 2).is_ok());
        command.source_height = 0;
        assert!(validate(&command, &source, 2, 2).is_err());
        command.source_x = 9;
        assert!(validate(&command, &source, 2, 2).is_err());
    }
}
