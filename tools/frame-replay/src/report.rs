use crate::frame::write_png;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub fn compare(reference: &[u8], rendered: &[u8]) -> Result<(usize, u8, Vec<u8>)> {
    ensure!(
        reference.len() == rendered.len() && reference.len().is_multiple_of(4),
        "comparison buffers differ in size"
    );
    let mut changed = 0;
    let mut maximum = 0;
    let mut difference = Vec::with_capacity(reference.len());
    for (a, b) in reference
        .as_chunks::<4>()
        .0
        .iter()
        .zip(rendered.as_chunks::<4>().0.iter())
    {
        let error = a
            .iter()
            .zip(b)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        changed += usize::from(error != 0);
        maximum = maximum.max(error);
        difference.extend([error.saturating_mul(8), 0, 0, 255]);
    }
    Ok((changed, maximum, difference))
}

pub fn write(
    directory: &Path,
    width: u32,
    height: u32,
    reference: &[u8],
    rendered: &[u8],
    mut metadata: Value,
) -> Result<usize> {
    let (changed, maximum, difference) = compare(reference, rendered)?;
    fs::create_dir_all(directory)?;
    write_png(&directory.join("reference.png"), width, height, reference)?;
    write_png(&directory.join("gpu.png"), width, height, rendered)?;
    write_png(
        &directory.join("difference.png"),
        width,
        height,
        &difference,
    )?;
    let mut pair = Vec::with_capacity(reference.len() * 2);
    for (left, right) in reference
        .chunks_exact(width as usize * 4)
        .zip(rendered.chunks_exact(width as usize * 4))
    {
        pair.extend_from_slice(left);
        pair.extend_from_slice(right);
    }
    write_png(&directory.join("comparison.png"), width * 2, height, &pair)?;
    metadata["width"] = json!(width);
    metadata["height"] = json!(height);
    metadata["different_pixels"] = json!(changed);
    metadata["total_pixels"] = json!(u64::from(width) * u64::from(height));
    metadata["max_channel_error"] = json!(maximum);
    metadata["passed"] = json!(changed == 0 && metadata["capture_reference_different_pixels"] == 0);
    fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;
    let data = serde_json::to_string(&metadata)?.replace('<', "\\u003c");
    let html = include_str!("report.html").replace("REPORT_DATA", &data);
    fs::write(directory.join("report.html"), html)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_rgb_and_alpha_errors() {
        let reference = [12, 13, 14, 255, 20, 21, 22, 255];
        let actual = [12, 15, 14, 255, 20, 21, 22, 254];
        let (count, max, diff) = compare(&reference, &actual).unwrap();
        assert_eq!((count, max), (2, 2));
        assert_eq!(diff, [16, 0, 0, 255, 8, 0, 0, 255]);
        assert!(compare(&reference, &actual[..4]).is_err());
    }
    #[test]
    fn exact_matches_produce_black_difference() {
        let pixels = [0, 3, 25, 127];
        let (count, max, diff) = compare(&pixels, &pixels).unwrap();
        assert_eq!((count, max), (0, 0));
        assert_eq!(diff, [0, 0, 0, 255]);
    }
}
