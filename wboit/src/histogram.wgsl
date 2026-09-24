#import stardust_wboit::WboitParams

const BINS: u32 = #{WBOIT_BINS}u;
const THREADS: u32 = 64u;

@group(0) @binding(0) var<uniform> params: WboitParams;
@group(0) @binding(1) var<storage, read_write> hist: array<u32>;
// four bin edges per layer, edge e + 1 in channel e & 3 of layer e >> 2, edge 0 is implicitly 0
@group(0) @binding(2) var cdf: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(3) var<storage, read_write> edges: array<f32>;

var<workgroup> partial: array<array<f32, BINS>, THREADS>;

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.tiles) {
        return;
    }
    let base = (id.y * params.tiles.x + id.x) * BINS;
    var e: array<f32, BINS>;
    var acc = 0.0;
    for (var b = 0u; b < BINS; b++) {
        acc += f32(hist[base + b]);
        e[b] = acc;
        hist[base + b] = 0u;
    }
    for (var b = 0u; b < BINS; b++) {
        if acc > 0.0 {
            e[b] /= acc;
        } else {
            e[b] = f32(b + 1u) / f32(BINS);
        }
    }
    for (var l = 0u; l < BINS / 4u; l++) {
        textureStore(cdf, id.xy, l, vec4(e[l * 4u], e[l * 4u + 1u], e[l * 4u + 2u], e[l * 4u + 3u]));
    }
}

// summed in f32, the whole view's worth can overflow a u32
@compute @workgroup_size(64)
fn reduce(@builtin(local_invocation_index) i: u32) {
    var sums: array<f32, BINS>;
    for (var t = i; t < params.tiles.x * params.tiles.y; t += THREADS) {
        for (var b = 0u; b < BINS; b++) {
            sums[b] += f32(hist[t * BINS + b]);
            hist[t * BINS + b] = 0u;
        }
    }
    for (var b = 0u; b < BINS; b++) {
        partial[i][b] = sums[b];
    }
    workgroupBarrier();

    if i != 0u {
        return;
    }
    var acc = 0.0;
    for (var b = 0u; b < BINS; b++) {
        for (var j = 0u; j < THREADS; j++) {
            acc += partial[j][b];
        }
        edges[b] = acc;
    }
    for (var b = 0u; b < BINS; b++) {
        if acc > 0.0 {
            edges[b] /= acc;
        } else {
            edges[b] = f32(b + 1u) / f32(BINS);
        }
    }
}
