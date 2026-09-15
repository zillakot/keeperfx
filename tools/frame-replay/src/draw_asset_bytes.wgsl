fn byte(i: u32) -> u32 {
    if PACKED_ASSETS {
        return (assets[i >> 2u] >> ((i & 3u) * 8u)) & 255u;
    }
    return assets[i];
}
fn le16(i: u32) -> u32 {
    return byte(i) | (byte(i + 1u) << 8u);
}
fn le32(i: u32) -> u32 {
    return byte(i) | (byte(i + 1u) << 8u) | (byte(i + 2u) << 16u) | (byte(i + 3u) << 24u);
}
