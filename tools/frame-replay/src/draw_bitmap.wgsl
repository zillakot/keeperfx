fn bitmap_sample(c: Command, pixel: vec2<u32>) -> u32 {
    let base = c.assets.x;
    if c.source.x == 0u {
        var lo = 0u;
        var hi = c.source.w;
        while lo < hi {
            let mid = lo + (hi-lo)/2u;
            let row = base+mid*16u;
            if pixel.y >= sprite_word(row)+sprite_word(row+4u) { lo=mid+1u; }
            else { hi=mid; }
        }
        if lo == c.source.w { return 256u; }
        let row = base+lo*16u;
        if pixel.y < sprite_word(row) { return 256u; }
        let records = base+sprite_word(row+8u);
        let count = sprite_word(row+12u);
        lo=0u; hi=count;
        while lo < hi {
            let mid=lo+(hi-lo)/2u;
            let record=records+mid*12u;
            if pixel.x >= sprite_word(record)+sprite_word(record+4u) { lo=mid+1u; }
            else { hi=mid; }
        }
        if lo == count { return 256u; }
        let record=records+lo*12u;
        if pixel.x < sprite_word(record) { return 256u; }
        return sprite_word(record+8u);
    }
    let origin=vec2<i32>(c.accumulator.xy);
    let size=vec2<i32>(c.accumulator.zw);
    var result=256u;
    for (var layer=0u; layer<2u; layer++) {
        if layer==0u && c.source.y==0u { continue; }
        let local=vec2<i32>(pixel)-origin-vec2<i32>(select(0,1,layer==0u));
        if any(local<vec2(0)) || any(local>=size) { continue; }
        var bit: u32;
        if c.source.x==2u {
            let scale=vec2<f32>(c.source.zw)/vec2<f32>(size);
            let sample=vec2<u32>(vec2<f32>(local)*scale);
            bit=sample.y*c.source.z+sample.x;
        } else {
            bit=u32(local.y)*((c.source.z+7u)/8u)*8u+u32(local.x);
        }
        let ink=(assets[base+12u+bit/8u] & (128u>>(bit&7u)))!=0u;
        if layer==0u {
            if ink { result=sprite_word(base+8u); }
        } else {
            let colour=sprite_word(base+select(4u,0u,ink));
            if colour!=256u { result=colour; }
        }
    }
    return result;
}
