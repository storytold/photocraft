// Original PhotoCraft shader. The supplied colour maps already use the canvas display transform.
struct Camera { rotation:vec4<f32>, projection:vec4<f32>, options:vec4<f32>, normal:vec4<f32>, light:vec4<f32>, surface:vec4<f32>, reserved0:vec4<f32>, reserved1:vec4<f32> }
struct Material { color:vec4<f32>, flags:vec4<f32> }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<storage,read> selection:array<u32>;
@group(1) @binding(0) var color_map:texture_2d<f32>;
@group(1) @binding(1) var normal_map:texture_2d<f32>;
@group(1) @binding(2) var map_sampler:sampler;
@group(1) @binding(3) var<uniform> material:Material;
@group(1) @binding(4) var linear_color:texture_2d<f32>;
@group(1) @binding(5) var metallic_map:texture_2d<f32>;
@group(1) @binding(6) var roughness_map:texture_2d<f32>;
@group(1) @binding(7) var ao_map:texture_2d<f32>;
@group(1) @binding(8) var emissive_map:texture_2d<f32>;
@group(1) @binding(9) var mask_sampler:sampler;
@group(2) @binding(0) var sky_specular:texture_2d<f32>;
@group(2) @binding(1) var sky_diffuse:texture_2d<f32>;
@group(2) @binding(2) var sky_sampler:sampler;
@group(2) @binding(3) var brdf_map:texture_2d<f32>;
@group(2) @binding(4) var output_lut:texture_2d<f32>;
struct Vertex {
    @location(0) position:vec3<f32>, @location(1) normal:vec3<f32>,
    @location(2) face:vec3<f32>, @location(3) tangent:vec3<f32>,
    @location(4) bitangent:vec3<f32>, @location(5) uv:vec2<f32>, @location(6) triangle:u32,
}
struct Fragment {
    @builtin(position) position:vec4<f32>, @location(0) normal:vec3<f32>,
    @location(1) @interpolate(flat) face:vec3<f32>, @location(2) @interpolate(flat) tangent:vec3<f32>,
    @location(3) @interpolate(flat) bitangent:vec3<f32>, @location(4) uv:vec2<f32>,
    @location(5) @interpolate(flat) selected:u32, @location(6) world:vec3<f32>,
}
fn unit(p:vec3<f32>) -> vec3<f32> { return p * inverseSqrt(max(dot(p,p),1e-30)); }
fn rotate(p:vec3<f32>) -> vec3<f32> {
    let r=camera.rotation;
    let x=r.y*p.x+r.x*p.z;
    let z=-r.x*p.x+r.y*p.z;
    return vec3<f32>(x,r.w*p.y-r.z*z,r.z*p.y+r.w*z);
}
@vertex fn vs_main(v:Vertex) -> Fragment {
    var out:Fragment;
    let r=rotate(v.position);
    let xy=(r.xy+camera.projection.zw)*camera.projection.xy;
    if camera.normal.z>0.0 {
        let d=camera.normal.z+camera.normal.w-r.z;
        let near=max(camera.normal.z*0.001,1e-8);
        let far=camera.normal.z+2.0;
        out.position=vec4<f32>(xy,far/(far-near)*d-near*far/(far-near),d);
    } else {
        out.position=vec4<f32>(xy,0.5-r.z*0.9,1.0);
    }
    out.world=v.position;
    out.normal=v.normal*camera.options.z;
    out.face=v.face*camera.options.z;
    out.tangent=v.tangent;
    out.bitangent=v.bitangent;
    out.uv=vec2<f32>(v.uv.x,1.0-v.uv.y);
    out.selected=0u;
    if v.triangle<arrayLength(&selection) {out.selected=selection[v.triangle];}
    return out;
}
struct WireVertex { @location(0) position:vec3<f32>, @location(1) face:vec3<f32> }
struct WireFragment { @builtin(position) position:vec4<f32>, @location(0) @interpolate(flat) face:vec3<f32> }
@vertex fn vs_wireframe(v:WireVertex)->WireFragment {
    let r=rotate(v.position);
    let xy=(r.xy+camera.projection.zw)*camera.projection.xy;
    var out:WireFragment;
    if camera.normal.z>0.0 {
        let d=camera.normal.z+camera.normal.w-r.z;
        let near=max(camera.normal.z*0.001,1e-8);
        let far=camera.normal.z+2.0;
        out.position=vec4<f32>(xy,far/(far-near)*d-near*far/(far-near),d);
    } else {
        out.position=vec4<f32>(xy,0.5-r.z*0.9,1.0);
    }
    out.face=v.face;
    return out;
}
@fragment fn fs_wireframe(v:WireFragment)->@location(0) vec4<f32> {
    if camera.reserved1.x>0.5 && rotate(v.face).z*camera.options.z<=0.0 { discard; }
    return camera.reserved0;
}
struct SymmetryVertex { @location(0) position:vec3<f32> }
struct SymmetryFragment { @builtin(position) position:vec4<f32> }
@vertex fn vs_symmetry_plane(v:SymmetryVertex)->SymmetryFragment {
    let r=rotate(v.position);
    let xy=(r.xy+camera.projection.zw)*camera.projection.xy;
    var out:SymmetryFragment;
    if camera.normal.z>0.0 {
        let d=camera.normal.z+camera.normal.w-r.z;
        let near=max(camera.normal.z*0.001,1e-8);
        let far=camera.normal.z+2.0;
        out.position=vec4<f32>(xy,far/(far-near)*d-near*far/(far-near),d);
    } else {
        out.position=vec4<f32>(xy,0.5-r.z*0.9,1.0);
    }
    // Tiny clip-space bias avoids z-fighting with coplanar model geometry and works
    // for both the triangle fill and line outline pipelines.
    out.position.z-=out.position.w*1e-5;
    return out;
}
@fragment fn fs_symmetry_fill()->@location(0) vec4<f32> {
    return vec4<f32>(0.24,0.58,0.96,0.10);
}
@fragment fn fs_symmetry_outline()->@location(0) vec4<f32> {
    return vec4<f32>(0.32,0.68,1.0,0.85);
}
fn shade(v:Fragment,texel:vec4<f32>,dx:vec2<f32>,dy:vec2<f32>)->vec4<f32> {
    if material.flags.z>0.5 {
        var rgb=texel.rgb;
        if v.selected!=0u {rgb=rgb*0.82+vec3<f32>(0.12,0.55,0.9)*0.18;}
        return vec4<f32>(clamp(rgb,vec3<f32>(0.0),vec3<f32>(1.0)),1.0);
    }
    var lighting=1.0;
    var n=unit(v.normal);
    if camera.options.x>0.5 {
        n=unit(select(v.face,v.normal,camera.options.y>0.5));
        if material.flags.x>0.5 && dot(v.tangent,v.tangent)>1e-20 {
            let rgb=textureSampleGrad(normal_map,map_sampler,v.uv,dx,dy).rgb;
            let t=unit(v.tangent-n*dot(n,v.tangent));
            let b=cross(n,t)*select(-1.0,1.0,dot(cross(n,t),v.bitangent)>=0.0);
            let bump=unit((rgb*2.0-1.0)*vec3<f32>(camera.normal.x,camera.normal.x*camera.normal.y,1.0));
            if dot(t,t)>1e-20 && dot(bump,bump)>1e-20 { n=unit(t*bump.x+b*bump.y+n*bump.z); }
        }
        lighting=0.25+max(dot(rotate(n),unit(vec3<f32>(0.3,0.6,0.75))),0.0)*0.75;
    }
    var rgb=texel.rgb;
    if camera.options.w>1.5 {
        rgb=vec3<f32>(0.5,0.5,1.0);
        if material.flags.x>0.5 { rgb=textureSampleGrad(normal_map,map_sampler,v.uv,dx,dy).rgb; }
    }
    var color=rgb*select(material.color.rgb,vec3<f32>(1.0),camera.options.w>0.5)*lighting;
    if camera.options.w<0.5 {
        let flags=u32(material.flags.y);
        var emission=vec3<f32>(0.0);
        if (flags&16u)!=0u {emission=textureSampleGrad(emissive_map,map_sampler,v.uv,dx,dy).rgb*camera.surface.z;}
        if camera.light.x>0.5 && camera.options.x>0.5 {
            var base=vec3<f32>(1.0);
            if (flags&1u)!=0u {base=textureSampleGrad(linear_color,map_sampler,v.uv,dx,dy).rgb;}
            base*=material.color.rgb;
            var metal=camera.surface.y;
            var rough=camera.surface.x;
            var ao=1.0;
            if (flags&2u)!=0u {metal=textureSampleGrad(metallic_map,map_sampler,v.uv,dx,dy).r;}
            if (flags&4u)!=0u {rough=textureSampleGrad(roughness_map,map_sampler,v.uv,dx,dy).r;}
            if (flags&8u)!=0u {ao=textureSampleGrad(ao_map,map_sampler,v.uv,dx,dy).r;}
            metal=clamp(metal,0.0,1.0);rough=clamp(rough,0.045,1.0);ao=clamp(ao,0.0,1.0);
            var view=inverse_rotate(vec3<f32>(0.0,0.0,1.0));
            if camera.normal.z>0.0 {view=unit(inverse_rotate(vec3<f32>(-camera.projection.zw,camera.normal.z+camera.normal.w))-v.world);}
            let nv=max(dot(n,view),0.001);
            let f0=mix(vec3<f32>(0.04),base,vec3<f32>(metal));
            let fresnel=f0+(max(vec3<f32>(1.0-rough),f0)-f0)*pow(1.0-nv,5.0);
            let diffuse=textureSampleLevel(sky_diffuse,sky_sampler,sky_uv(n),0.0).rgb;
            let reflected=textureSampleLevel(sky_specular,sky_sampler,sky_uv(reflect(-view,n)),rough*f32(textureNumLevels(sky_specular)-1u)).rgb;
            let brdf=textureSampleLevel(brdf_map,sky_sampler,clamp(vec2<f32>(nv,rough),vec2<f32>(0.5/64.0),vec2<f32>(1.0-0.5/64.0)),0.0).rg;
            let radiance=((1.0-fresnel)*(1.0-metal)*base*diffuse+reflected*(f0*brdf.x+brdf.y))*camera.light.y*ao+emission;
            color=display(radiance);
        } else if (flags&16u)!=0u {
            color=display(srgb_decode(color)+emission);
        }
    }
    if v.selected!=0u { color=color*0.82+vec3<f32>(0.12,0.55,0.9)*0.18; }
    return vec4<f32>(clamp(color,vec3<f32>(0.0),vec3<f32>(1.0)),1.0);
}
// Opaque geometry has no discard/storage access in the fragment stage: early depth can reject it.
@fragment fn fs_opaque(v:Fragment)->@location(0) vec4<f32> {
    let dx=dpdx(v.uv); let dy=dpdy(v.uv);
    return shade(v,sample_color(v.uv,dx,dy),dx,dy);
}
@fragment fn fs_cutout(v:Fragment)->@location(0) vec4<f32> {
    let dx=dpdx(v.uv); let dy=dpdy(v.uv);
    let texel=sample_color(v.uv,dx,dy);
    if texel.a*material.color.a<0.5 {discard;}
    return shade(v,texel,dx,dy);
}
fn sample_color(uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>)->vec4<f32> {
    // A mask UV exactly on the non-tiled canvas edge belongs to its edge texel;
    // wrapping the opposite atlas edge into linear filtering creates a bright/dark seam.
    let in_canvas=all(uv>=vec2<f32>(0.0)) && all(uv<=vec2<f32>(1.0));
    if material.flags.z>0.5 && in_canvas {
        return textureSampleGrad(color_map,mask_sampler,uv,dx,dy);
    }
    return textureSampleGrad(color_map,map_sampler,uv,dx,dy);
}

fn inverse_rotate(p:vec3<f32>)->vec3<f32>{
    let r=camera.rotation;
    let y=r.w*p.y+r.z*p.z;let z=-r.z*p.y+r.w*p.z;
    return vec3<f32>(r.y*p.x-r.x*z,y,r.x*p.x+r.y*z);
}
fn sky_uv(d:vec3<f32>)->vec2<f32>{
    return vec2<f32>(atan2(d.z,d.x)/6.283185307+0.5+camera.light.z/6.283185307,acos(clamp(d.y,-1.0,1.0))/3.141592654);
}
fn srgb_decode(c:vec3<f32>)->vec3<f32>{return select(c/12.92,pow(max((c+0.055)/1.055,vec3<f32>(0.0)),vec3<f32>(2.4)),c>vec3<f32>(0.04045));}
fn display(c:vec3<f32>)->vec3<f32>{
    let x=max(c,vec3<f32>(0.0));let mapped=x/(1.0+x);
    let encoded=select(mapped*12.92,1.055*pow(mapped,vec3<f32>(1.0/2.4))-0.055,mapped>vec3<f32>(0.0031308));
    let p=clamp(encoded,vec3<f32>(0.0),vec3<f32>(1.0))*16.0;let lo=vec3<i32>(floor(p));let hi=min(lo+vec3<i32>(1),vec3<i32>(16));let f=fract(p);
    let a=mix(lut_at(vec3<i32>(lo.x,lo.y,lo.z)),lut_at(vec3<i32>(hi.x,lo.y,lo.z)),f.x);
    let b=mix(lut_at(vec3<i32>(lo.x,hi.y,lo.z)),lut_at(vec3<i32>(hi.x,hi.y,lo.z)),f.x);
    let c0=mix(lut_at(vec3<i32>(lo.x,lo.y,hi.z)),lut_at(vec3<i32>(hi.x,lo.y,hi.z)),f.x);
    let d=mix(lut_at(vec3<i32>(lo.x,hi.y,hi.z)),lut_at(vec3<i32>(hi.x,hi.y,hi.z)),f.x);
    return mix(mix(a,b,f.y),mix(c0,d,f.y),f.z);
}
struct SkyVertex{@builtin(position) position:vec4<f32>,@location(0) ndc:vec2<f32>}
@vertex fn vs_background(@builtin(vertex_index) i:u32)->SkyVertex{
    var out:SkyVertex;let x=f32((i<<1u)&2u)*2.0-1.0;let y=f32(i&2u)*2.0-1.0;
    out.position=vec4<f32>(x,y,1.0,1.0);out.ndc=vec2<f32>(x,y);return out;
}
@fragment fn fs_background(v:SkyVertex)->@location(0) vec4<f32>{
    var ray=vec3<f32>(0.0,0.0,-1.0);
    if camera.normal.z>0.0 {ray=unit(vec3<f32>(v.ndc/camera.projection.xy,-1.0));}
    let sky=textureSampleLevel(sky_specular,sky_sampler,sky_uv(inverse_rotate(ray)),0.0).rgb;
    return vec4<f32>(display(sky*camera.surface.w),1.0);
}

fn lut_at(p:vec3<i32>)->vec3<f32>{let i=p.x+17*p.y+289*p.z;return textureLoad(output_lut,vec2<i32>(i%289,i/289),0).rgb;}
