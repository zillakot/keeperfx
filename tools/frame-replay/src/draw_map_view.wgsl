fn map_i32(offset: u32) -> i32 {
    return bitcast<i32>(asset_word(offset));
}
fn map_view_sample(c: Command, pixel: vec2<u32>, destination: u32, view: vec3<u32>) -> u32 {
    let view_width = view.z;
    let base = c.assets.x;
    let local = vec2<u32>(vec2<i32>(pixel) - view_bounds(c, view).xy);
    if c.source.x == 0u {
        let cell = local.x / c.source.w;
        let style = le16(base+cell*2u);
        if style <= 256u { return style; }
        let tables = base+c.source.z*2u;
        if style == 257u { return byte(tables+destination); }
        if style == 258u { return (byte(tables+destination)+2u)&255u; }
        if style == 259u { return byte(tables+256u+destination); }
        if style == 260u { return 102u+(byte(tables+512u+destination)>>6u); }
        if style == 261u { return byte(tables+768u+destination); }
        return byte(tables+1024u+destination);
    }
    if c.source.x == 1u {
        let size = vec2<u32>(c.bounds.zw-c.bounds.xy);
        let step = vec2(2097151u)/size;
        let position = local*step;
        let u = select(position.x,2097151u-position.x,(c.source.y&16u)!=0u)>>16u;
        let v = select(position.y,2097151u-position.y,(c.source.y&32u)!=0u)>>16u;
        let offset = select(v*256u+u,u*256u+v,(c.source.y&64u)!=0u);
        return byte(base+8192u+byte(base+offset));
    }
    if c.source.x == 3u {
        let center = bitcast<vec2<i32>>(c.accumulator.xy);
        let spread = bitcast<i32>(c.accumulator.z);
        let index = i32(pixel.y*view_width+pixel.x);
        for (var i=0u;i<c.source.y;i++) {
            let offset = (center.y+map_i32(base+i*8u+4u))*i32(view_width)+center.x+map_i32(base+i*8u);
            if index == offset || (c.accumulator.w != 0u &&
                (index == offset-spread || index == offset+spread ||
                 index == offset-spread*i32(view_width) || index == offset+spread*i32(view_width))) {
                return c.operation.w;
            }
        }
        return 256u;
    }
    let split = c.accumulator.xy;
    let anchor = c.accumulator.zw;
    let delta = c.source.y;
    let left = pixel.x <= split.x;
    let top = pixel.y <= split.y;
    var sx: u32;
    var sy: u32;
    if left { sx = anchor.x-((split.x-pixel.x+1u)*delta>>8u); }
    else {
        let dx = pixel.x-split.x-1u;
        sx = anchor.x+((256u+(dx+select(0u,1u,top))*delta)>>8u);
    }
    if top { sy=anchor.y-((split.y-pixel.y)*delta>>8u); }
    else { sy=anchor.y+((256u+(pixel.y-split.y-1u)*delta)>>8u); }
    return byte(base+sy*c.source.z+sx);
}
