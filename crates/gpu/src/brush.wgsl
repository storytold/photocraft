// Ordered coverage accumulation: one invocation owns one pixel; no blending atomics.
struct Dab { center: vec4<f32>, rotation: vec4<f32>, transfer: vec4<f32>, shape: vec4<f32> }
@group(0) @binding(0) var<storage, read> dabs: array<Dab>;
@group(0) @binding(1) var<storage, read> jobs: array<u32>;
@group(0) @binding(2) var<storage, read_write> coverage: array<f32>;

fn falloff(d: f32, radius: f32, hardness: f32) -> f32 {
    let inner = radius * clamp(hardness, 0.0, 1.0);
    if d <= inner || radius - inner < 0.001 { return 1.0; }
    let t = clamp((d - inner) / (radius - inner), 0.0, 1.0);
    let s = 1.0 - t * t;
    let s2 = s * s;
    return s2 * s2;
}

@compute @workgroup_size(8, 8)
fn raster(@builtin(global_invocation_id) id: vec3<u32>) {
    let header = id.z * 4u;
    let count = jobs[header + 1u];
    let x = f32(bitcast<i32>(jobs[header + 2u])) + f32(id.x) + 0.5;
    let y = f32(bitcast<i32>(jobs[header + 3u])) + f32(id.y) + 0.5;
    let index = id.z * 4096u + id.y * 64u + id.x;
    var c = coverage[index];
    for (var i = 0u; i < count; i++) {
        let d = dabs[jobs[jobs[header] + i]];
        var ux = x - d.center.x;
        var uy = -(y - d.center.y);
        let projection = (ux * d.transfer.z + uy * d.transfer.w) * d.shape.x;
        ux += projection * d.transfer.z;
        uy += projection * d.transfer.w;
        let u = (ux * d.rotation.y + uy * d.rotation.x) * d.rotation.z;
        let v = (-ux * d.rotation.x + uy * d.rotation.y) * d.rotation.w;
        let radius = d.center.z;
        let ro = d.center.w;
        let rm = max(radius * ro, 0.5);
        let ur = u * ro;
        let d2 = ur * ur + v * v;
        if d2 >= (rm + 0.5) * (rm + 0.5) { continue; }
        let distance = sqrt(d2);
        var value: f32;
        if d.shape.w > 0.5 {
            value = select(0.0, 1.0, distance <= rm);
        } else {
            value = clamp(rm + 0.5 - distance, 0.0, 1.0) * falloff(distance, rm, d.shape.y);
        }
        if value <= 0.0 { continue; }
        if d.shape.z > 0.5 {
            let t = clamp((distance / rm - 0.5) / 0.5, 0.0, 1.0);
            value *= 0.5 + 0.5 * t * t * (3.0 - 2.0 * t);
        }
        value *= d.transfer.x;
        if d.shape.z > 0.5 {
            c = max(c, value * d.transfer.y);
        } else if d.transfer.y > c {
            c = c + value * (d.transfer.y - c);
        }
    }
    coverage[index] = c;
}
