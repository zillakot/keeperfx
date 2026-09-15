fn byte(i: u32) -> u32 {
    if PACKED_ASSETS {
        return (assets[i >> 2u] >> ((i & 3u) * 8u)) & 255u;
    }
    return assets[i];
}
fn le16(i: u32) -> u32 {
    if PACKED_ASSETS {
        let index = i >> 2u;
        let lane = i & 3u;
        let low = assets[index];
        if lane == 3u {
            return (low >> 24u) | ((assets[index + 1u] & 255u) << 8u);
        }
        return (low >> (lane * 8u)) & 65535u;
    }
    return byte(i) | (byte(i + 1u) << 8u);
}
fn le32(i: u32) -> u32 {
    if PACKED_ASSETS {
        let index = i >> 2u;
        let shift = (i & 3u) * 8u;
        let low = assets[index];
        if shift == 0u { return low; }
        return (low >> shift) | (assets[index + 1u] << (32u - shift));
    }
    return byte(i) | (byte(i + 1u) << 8u) | (byte(i + 2u) << 16u) | (byte(i + 3u) << 24u);
}
