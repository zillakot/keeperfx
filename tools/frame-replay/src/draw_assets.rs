use super::host::{self, Phase, Scope};

pub const PACKED: bool = cfg!(feature = "packed-arena");
pub const STRIDE: usize = if PACKED { 1 } else { 4 };
pub type ByteOffset = u32;

pub fn shader(source: &str) -> String {
    format!(
        "const PACKED_ASSETS: bool = {PACKED};\n{}\n{source}",
        include_str!("draw_asset_bytes.wgsl")
    )
}

pub(super) fn aligned(length: usize) -> usize {
    length.div_ceil(4) * 4
}

pub(super) fn store(destination: &mut [u8], source: &[u8]) {
    let _scope = Scope::new(Phase::Upload);
    let _copy = host::UploadTimer::new(
        if PACKED {
            host::UploadPart::Copy
        } else {
            host::UploadPart::Expand
        },
        destination.len(),
    );
    if PACKED {
        destination[..source.len()].copy_from_slice(source);
        destination[source.len()..].fill(0);
    } else {
        for (word, &byte) in destination.as_chunks_mut::<4>().0.iter_mut().zip(source) {
            word.copy_from_slice(&u32::from(byte).to_le_bytes());
        }
        destination[source.len() * 4..].fill(0);
    }
}

pub fn encode(source: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; aligned(source.len() * STRIDE)];
    store(&mut bytes, source);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails_and_guarded_neighbors() {
        for length in 0..17 {
            let source: Vec<_> = (0..length as u8).map(|v| v.wrapping_mul(47)).collect();
            let size = aligned(length * STRIDE);
            let mut guarded = vec![0xa5; size + 8];
            store(&mut guarded[4..4 + size], &source);
            assert_eq!(&guarded[..4], &[0xa5; 4]);
            assert_eq!(&guarded[4 + size..], &[0xa5; 4]);
            for (i, &value) in source.iter().enumerate() {
                assert_eq!(guarded[4 + i * STRIDE], value);
            }
            assert!(
                guarded[4 + length * STRIDE..4 + size]
                    .iter()
                    .all(|&b| b == 0)
            );
        }
    }
}
