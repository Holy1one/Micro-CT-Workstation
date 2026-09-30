// One invocation owns one ZYX voxel. No floating atomics or cross-workgroup writes.
struct Params {
    theta: f32,
    pitch: f32,
    voxel: f32,
    sod: f32,
    sdd: f32,
    center_u: f32,
    center_v: f32,
    angle_weight: f32,
    volume_n: u32,
    detector_n: u32,
    reserved0: u32,
    reserved1: u32,
};

@group(0) @binding(0) var<storage, read> projection: array<f32>;
@group(0) @binding(1) var<storage, read_write> volume: array<f32>;
@group(0) @binding(2) var<storage, read_write> coverage: array<u32>;
@group(0) @binding(3) var<uniform> params: Params;

fn detector_sample(px: f32, py: f32) -> f32 {
    let col = u32(floor(px));
    let row = u32(floor(py));
    let tx = px - f32(col);
    let ty = py - f32(row);
    let i = row * params.detector_n + col;
    let a = projection[i] * (1.0 - tx) + projection[i + 1u] * tx;
    let b = projection[i + params.detector_n] * (1.0 - tx)
        + projection[i + params.detector_n + 1u] * tx;
    return a * (1.0 - ty) + b * ty;
}

@compute @workgroup_size(4, 4, 4)
fn backproject(@builtin(global_invocation_id) gid: vec3<u32>) {
    if any(gid >= vec3<u32>(params.volume_n)) { return; }
    let n = params.volume_n;
    let index = (gid.z * n + gid.y) * n + gid.x;
    let half = (f32(n) - 1.0) * 0.5;
    let detector_half = (f32(params.detector_n) - 1.0) * 0.5;
    let x = (f32(gid.x) - half) * params.voxel;
    let y = (f32(gid.y) - half) * params.voxel;
    let z = (f32(gid.z) - half) * params.voxel;
    let c = cos(params.theta);
    let s = sin(params.theta);
    let along = params.sod + y * c - x * s;
    if along <= 0.0 { coverage[index] = 0u; return; }
    let magnification = params.sdd / along;
    let px = (magnification * (x * c + y * s) + params.center_u) / params.pitch + detector_half;
    let py = (magnification * z + params.center_v) / params.pitch + detector_half;
    if px < 0.0 || py < 0.0 || px >= f32(params.detector_n - 1u)
        || py >= f32(params.detector_n - 1u) {
        coverage[index] = 0u;
        return;
    }
    let sampled = detector_sample(px, py);
    let weight = params.angle_weight * pow(params.sod / along, 2.0);
    volume[index] = volume[index] + sampled * weight;
    let ux = (px - detector_half) / (f32(params.detector_n) * 0.5);
    let vy = (py - detector_half) / (f32(params.detector_n) * 0.5);
    if ux * ux + vy * vy > 0.94 * 0.94 { coverage[index] = 0u; }
}
