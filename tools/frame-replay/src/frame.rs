use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
};

const HEADER: usize = 16 + 256 * 4;
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub palette: Vec<u8>,
    pub indices: Vec<u8>,
}

pub fn dimensions(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    ensure!(
        width > 0 && height > 0 && width <= 8192 && height <= 8192 && pixels <= MAX_PIXELS,
        "dimensions must be nonzero, at most 8192 per axis and 16 megapixels"
    );
    Ok(pixels as usize)
}

impl Frame {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= HEADER && &bytes[..8] == b"KFXFRM01",
            "invalid or truncated frame header"
        );
        let width = u32::from_le_bytes(bytes[8..12].try_into()?);
        let height = u32::from_le_bytes(bytes[12..16].try_into()?);
        let pixels = dimensions(width, height)?;
        ensure!(
            bytes.len() == HEADER + pixels,
            "frame payload length does not match dimensions"
        );
        Ok(Self {
            width,
            height,
            palette: bytes[16..HEADER].to_vec(),
            indices: bytes[HEADER..].to_vec(),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            path.metadata()?.len() <= HEADER as u64 + MAX_PIXELS,
            "frame file exceeds size limit"
        );
        Self::parse(&std::fs::read(path)?).with_context(|| format!("reading {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut bytes = b"KFXFRM01".to_vec();
        bytes.extend(self.width.to_le_bytes());
        bytes.extend(self.height.to_le_bytes());
        bytes.extend(&self.palette);
        bytes.extend(&self.indices);
        std::fs::write(path, bytes)?;
        Ok(())
    }

    pub fn rgba(&self) -> Vec<u8> {
        self.indices
            .iter()
            .flat_map(|&i| {
                self.palette[usize::from(i) * 4..usize::from(i) * 4 + 4]
                    .iter()
                    .copied()
            })
            .collect()
    }

    pub fn fixture() -> Self {
        let palette = (0..256u32)
            .flat_map(|i| {
                let alpha = match i {
                    128 => 0,
                    129 => 127,
                    _ => 255,
                };
                [i as u8, (255 - i) as u8, (i * 73) as u8, alpha]
            })
            .collect();
        let indices = (0..19)
            .flat_map(|y| (0..257).map(move |x| ((x + y * 37) % 256) as u8))
            .collect();
        Self {
            width: 257,
            height: 19,
            palette,
            indices,
        }
    }
}

pub fn scale_rgba(source: &[u8], width: u32, height: u32, scale: u32) -> Result<Vec<u8>> {
    ensure!((1..=8).contains(&scale), "scale must be between 1 and 8");
    let pixels = dimensions(
        width.checked_mul(scale).context("width overflow")?,
        height.checked_mul(scale).context("height overflow")?,
    )?;
    ensure!(
        source.len() == width as usize * height as usize * 4,
        "invalid RGBA buffer"
    );
    let mut output = Vec::with_capacity(pixels * 4);
    for y in 0..height * scale {
        for x in 0..width * scale {
            let offset = ((y / scale) * width + x / scale) as usize * 4;
            output.extend_from_slice(&source[offset..offset + 4]);
        }
    }
    Ok(output)
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    decoder.set_limits(png::Limits {
        bytes: MAX_PIXELS as usize * 4,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    dimensions(reader.info().width, reader.info().height)?;
    let size = reader
        .output_buffer_size()
        .context("PNG buffer size overflow")?;
    ensure!(size <= MAX_PIXELS as usize * 4, "PNG exceeds size limit");
    let mut bytes = vec![0; size];
    let info = reader.next_frame(&mut bytes)?;
    bytes.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes,
        png::ColorType::Rgb => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => bail!("reference must decode to RGB or RGBA"),
    };
    Ok((info.width, info.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded() -> Vec<u8> {
        let f = Frame::fixture();
        let mut b = b"KFXFRM01".to_vec();
        b.extend(f.width.to_le_bytes());
        b.extend(f.height.to_le_bytes());
        b.extend(f.palette);
        b.extend(f.indices);
        b
    }
    #[test]
    fn rejects_bad_magic_and_truncation() {
        let mut b = encoded();
        assert!(Frame::parse(&b[..15]).is_err());
        assert!(Frame::parse(&b[..b.len() - 1]).is_err());
        b[0] = 0;
        assert!(Frame::parse(&b).is_err());
    }
    #[test]
    fn rejects_invalid_dimensions_and_trailing_data() {
        let mut b = encoded();
        b[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(Frame::parse(&b).is_err());
        b[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Frame::parse(&b).is_err());
        let mut b = encoded();
        b.push(0);
        assert!(Frame::parse(&b).is_err());
    }
    #[test]
    fn decodes_palette_and_rows() {
        let f = Frame::parse(&encoded()).unwrap();
        let p = f.rgba();
        assert_eq!((f.width, f.height), (257, 19));
        assert_eq!(&p[..8], &[0, 255, 0, 255, 1, 254, 73, 255]);
        assert_eq!(&p[257 * 4..257 * 4 + 4], &[37, 218, 141, 255]);
        assert_eq!(p[128 * 4 + 3], 0);
        assert_eq!(p[129 * 4 + 3], 127);
    }
    #[test]
    fn scales_without_blending_or_flipping() {
        let input = [1, 2, 3, 255, 4, 5, 6, 255];
        let p = scale_rgba(&input, 1, 2, 2).unwrap();
        assert_eq!(
            &p[..16],
            &[1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255]
        );
        assert_eq!(
            &p[16..],
            &[4, 5, 6, 255, 4, 5, 6, 255, 4, 5, 6, 255, 4, 5, 6, 255]
        );
        assert!(scale_rgba(&input, 1, 2, 0).is_err());
        assert!(scale_rgba(&input, 1, 2, 9).is_err());
    }
}
