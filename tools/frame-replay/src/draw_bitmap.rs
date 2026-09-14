use super::{Command, OPAQUE, Resource};
use anyhow::{Result, ensure};

/// Validates the bitmap payload and returns the destination box it can write inside,
/// half-open and in view space. A huge bitmap writes only where a row record covers the
/// pixel, so the union of the row extents and of their pixel runs bounds it; outside
/// that, both binary searches fall off their table and `bitmap_sample` returns the
/// transparent index. A glyph writes only inside the scaled destination rectangle, plus
/// one pixel right and down when the shadow layer is enabled.
pub(super) fn validate(
    c: &Command,
    source: &Resource,
    width: u32,
    height: u32,
) -> Result<[i64; 4]> {
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
        let mut span = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
        for i in 0..rows {
            let y = word(i * 16) as u64;
            let n_y = word(i * 16 + 4) as u64;
            ensure!(
                y >= end_y && y + n_y <= height as u64,
                "invalid huge row coverage"
            );
            end_y = y + n_y;
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
                span[0] = span[0].min(x as i64);
                span[2] = span[2].max(end_x as i64);
                span[1] = span[1].min(y as i64);
                span[3] = span[3].max((y + n_y) as i64);
            }
            offset += count * 12;
        }
        ensure!(offset == bytes.len(), "trailing huge records");
        if span[0] > span[2] {
            span = [0; 4];
        }
        return Ok(span);
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
    let origin = [
        i64::from(c.start_low as i32),
        i64::from(c.start_high as i32),
    ];
    let shadow = i64::from(c.source_y != 0);
    Ok([
        origin[0],
        origin[1],
        origin[0] + i64::from(c.step_low) + shadow,
        origin[1] + i64::from(c.step_high) + shadow,
    ])
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
            cursor: false,
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
