struct MinimapView { viewport: vec4<u32>, colours: array<vec4<u32>, 8>, bounds: vec4<u32>, }
@group(0) @binding(3) var<uniform> view: MinimapView;
fn address(i: u32) -> u32 { return view.viewport.z + (i / view.viewport.x) * view.viewport.y + i % view.viewport.x; }
@group(0) @binding(0) var<storage, read_write> pixels: array<u32>;
@group(0) @binding(1) var<storage, read> assets: array<u32>;
fn data(i:u32)->u32 {return byte(view.viewport.w+i);}
@group(0) @binding(2) var<storage, read> background: array<u32>;
fn word(o:u32)->u32 {return le32(view.viewport.w+o);}
fn h(i:u32)->u32 {return word(i*4);}
fn si(i:u32)->i32 {return bitcast<i32>(h(i));}
fn root(n:i32)->i32 {var l=0;var r=2049;loop {if r-l<=1 {break;}let m=(l+r)/2;if m*m<=n {l=m;}else {r=m;}}return l;}
fn pattern(p:vec2<i32>,center:vec2<i32>,spread:i32)->bool {
 for(var i=0u;i<h(18);i++){let o=h(22)+i*8;let d=p-center-vec2<i32>(bitcast<i32>(word(o)),bitcast<i32>(word(o+4)));if all(d==vec2<i32>(0)) || (spread!=0 && ((d.y==0 && abs(d.x)==abs(spread))||(d.x==0 && abs(d.y)==abs(spread)))) {return true;}}
 return false;
}
fn octant(p:vec2<i32>,x:i32,y:i32)->bool {return (abs(p.x)==x&&abs(p.y)==y)||(abs(p.x)==y&&abs(p.y)==x);}
@compute @workgroup_size(8,8)
fn minimap(@builtin(global_invocation_id) id:vec3<u32>) {
 let d=h(5);let p=vec2<i32>(id.xy+view.bounds.xy);if u32(p.x)>=view.bounds.z||u32(p.y)>=view.bounds.w {return;}let radius=i32(d/2);let n=radius*radius-(radius-p.y-1)*(radius-p.y-1);let s=root(n);if p.x<radius-s||p.x>=radius+s{return;}
 let dst=(h(4)+u32(p.y))*h(1)+h(3)+u32(p.x);let mode=h(0);var col=h(20);var write=false;
 if mode==0u {
  let wx=si(8)+p.y*si(6)+p.x*si(7);let wy=si(9)+p.y*si(7)-p.x*si(6);
  if wx<0||wy<0||wx>=i32(h(10)<<16)||wy>=i32(h(11)<<16){return;}
  let o=h(13)+2*(u32(wx>>16)+u32(wy>>16)*(h(10)+1));let cell=le16(view.viewport.w+o);let bk=background[u32(p.y)*d+u32(p.x)];
  let colours=view.colours[bk>>5u][(bk>>3u)&3u];let colour=(colours>>((bk&7u)*4u))&15u;
  col=data(h(14)+colour*38569u+cell);write=true;
 } else if mode==1u {write=pattern(p,vec2<i32>(si(16),si(17)),si(19));}
 else if mode==2u {
  let q=p-vec2<i32>(si(16),si(17));let hi=max(abs(q.x),abs(q.y));let lo=min(abs(q.x),abs(q.y));var y=si(18);var x=0;var decision=3-2*y;
  if y>1 {loop {if x>=y {break;}if x>lo||y<hi {break;}if octant(q,x,y){write=true;break;}if decision>=0{decision+=4*(x-y)+si(21);y-=1;}else{decision+=4*(x-1)+si(21);}x+=1;}
   if x==y&&octant(q,x,y){write=true;}}
 } else if mode==3u {
  var pos=vec2<i32>(si(16),si(17));var remaining=si(21)-4;
  loop {if remaining<=0||pos.x<0||pos.y<0||pos.x>>8>=i32(d)||pos.y>>8>=i32(d){break;}pos+=vec2<i32>(si(6),si(7));if pattern(p,pos>>vec2<u32>(8),0){write=true;break;}remaining-=4;}
 } else {col=255u;write=true;}
 if write {pixels[address(dst)]=col;}
}
