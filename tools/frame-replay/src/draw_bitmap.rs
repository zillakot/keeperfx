use super::{Command, OPAQUE, Resource};
use anyhow::{Result, ensure};

pub(super) fn validate(c: &Command, source: &Resource, width: u32, height: u32) -> Result<()> {
    ensure!(
        c.blend == 0
            && c.transparent == OPAQUE
            && c.x == 0
            && c.y == 0
            && c.width <= width
            && c.height <= height,
        "invalid bitmap target"
    );
    let bytes = &source.bytes;
    let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    if c.source_x == 0 {
        let rows = c.source_height as usize;
        ensure!(
            rows > 0 && rows <= 8192 && bytes.len() >= rows * 16,
            "invalid huge rows"
        );
        let mut end_y = 0;
        let mut offset = rows * 16;
        for i in 0..rows {
            let y = word(i * 16) as u64;
            let n = word(i * 16 + 4) as u64;
            ensure!(
                y >= end_y && y + n <= height as u64,
                "invalid huge row coverage"
            );
            end_y = y + n;
            let start = word(i * 16 + 8) as usize;
            let count = word(i * 16 + 12) as usize;
            ensure!(
                start == offset && count <= 8192 && count * 12 <= bytes.len() - offset,
                "invalid huge records"
            );
            let mut end_x = 0;
            for j in 0..count {
                let x = word(start + j * 12) as u64;
                let n = word(start + j * 12 + 4) as u64;
                ensure!(
                    x >= end_x && n > 0 && x + n <= width as u64 && word(start + j * 12 + 8) <= 255,
                    "invalid huge pixel coverage"
                );
                end_x = x + n;
            }
            offset += count * 12;
        }
        ensure!(offset == bytes.len(), "trailing huge records");
    } else {
        ensure!(
            matches!(c.source_x, 1 | 2)
                && c.source_y <= 1
                && (1..=256).contains(&c.source_width)
                && (1..=256).contains(&c.source_height)
                && c.step_low > 0
                && c.step_high > 0
                && c.step_low as u64 * c.step_high as u64 <= 8192
                && (-16384..=24576).contains(&(c.start_low as i32))
                && (-16384..=24576).contains(&(c.start_high as i32)),
            "invalid glyph geometry"
        );
        ensure!(
            bytes.len() == 12 + c.source_width.div_ceil(8) as usize * c.source_height as usize,
            "invalid glyph bitplane"
        );
        ensure!(
            word(0) <= 256 && word(4) <= 256 && word(8) <= 256,
            "invalid glyph colours"
        );
        if c.source_x == 1 {
            ensure!(
                c.step_low == c.source_width && c.step_high == c.source_height,
                "unscaled glyph mismatch"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bitmap_payload_before_submission() {
        let mut bytes = Vec::new();
        for value in [0u32, 1, 16, 1, 0, 4, 255] {
            bytes.extend(value.to_le_bytes());
        }
        let mut resource = Resource {
            width: 28,
            height: 1,
            pitch: 28,
            bytes,
        };
        let mut command = Command {
            kind: super::super::BITMAP,
            width: 4,
            height: 4,
            source_height: 1,
            ..Default::default()
        };
        assert!(validate(&command, &resource, 4, 4).is_ok());
        resource.bytes[20] = 5;
        assert!(validate(&command, &resource, 4, 4).is_err());
        resource.bytes[20] = 4;
        resource.bytes[8] = 255;
        assert!(validate(&command, &resource, 4, 4).is_err());
        command.source_x = 1;
        command.source_width = 9;
        command.source_height = 2;
        command.step_low = 9;
        command.step_high = 2;
        resource.bytes = vec![0; 16];
        assert!(validate(&command, &resource, 4, 4).is_ok());
        resource.bytes.pop();
        assert!(validate(&command, &resource, 4, 4).is_err());
    }
}
