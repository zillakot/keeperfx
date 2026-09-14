@group(0) @binding(0) var<storage,read_write> scratch:array<u32>;
@group(0) @binding(1) var<storage,read> source:array<u32>;
@group(0) @binding(2) var<uniform> arena:vec4<u32>;
@group(0) @binding(3) var<storage,read_write> slot:array<u32>;
fn asset(at:u32)->u32 {return source[arena.x+at];}
fn word(at:u32)->u32 {return asset(at)|(asset(at+1u)<<8u)|(asset(at+2u)<<16u)|(asset(at+3u)<<24u);}
// One invocation owns one address, so reading and writing the resident scratch needs no second buffer.
@compute @workgroup_size(8,8)
fn shadow_mask(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=256u || id.y>=256u {return;}
    let address=id.y*256u+id.x;
    var value=scratch[address];
    if id.x<word(0u) && id.y<word(4u) {value=0u;}
    let offx=word(16u);let offy=word(20u);let flip=word(24u);
    var first=256u*offy+offx;
    if flip!=0u {first=first+1u-word(8u);}
    let source_row=(address-first)/256u;
    if address>=first && source_row<word(12u) {
        var cursor=152u;
        for(var row=0u;row<source_row;row++) {
            loop {let run=asset(cursor);cursor++;if run==0u {break;} if run<128u {cursor+=run;}}
        }
        var x=0u;
        loop {
            let run=asset(cursor);cursor++;
            if run==0u {break;}
            if run>=128u {x+=256u-run;continue;}
            var artwork_x=i32(address)-i32(256u*(offy+source_row)+offx);
            if flip!=0u {artwork_x=i32(256u*(offy+source_row)+offx)-i32(address);}
            if artwork_x>=i32(x) && artwork_x<i32(x+run) {value=255u;}
            x+=run;cursor+=run;
        }
    }
    scratch[address]=value;
    slot[address]=value;
}
