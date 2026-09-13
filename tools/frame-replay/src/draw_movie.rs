use super::{Command, OPAQUE, Resource};
use anyhow::{Result, ensure};

pub(super) fn validate(c: &Command, source: &Resource, width: u32, height: u32) -> Result<()> {
    ensure!(
        c.x == 0 && c.y == 0 && c.width == width && c.height == height,
        "movie requires full target bounds"
    );
    ensure!(
        c.blend == 0 && c.transparent == OPAQUE && c.source_x <= 7 && c.source_y == 0,
        "invalid movie options"
    );
    ensure!(
        c.source_width > 0
            && c.source_height > 0
            && c.source_width <= source.width
            && c.source_height <= source.height,
        "invalid movie source"
    );
    ensure!(
        c.source_x & 3 == 0 || c.source_width >= 4,
        "packed movie requires at least four pixels"
    );
    ensure!(
        (-16384..=16384).contains(&(c.start_low as i32))
            && (-16384..=16384).contains(&(c.start_high as i32)),
        "invalid movie origin"
    );
    let copied_width = if c.source_x & 3 != 0 {
        c.source_width / 4 * 4
    } else {
        c.source_width
    } * if c.source_x & 1 != 0 { 2 } else { 1 };
    ensure!(copied_width <= width, "movie row exceeds target pitch");
    Ok(())
}
