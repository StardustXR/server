#define_import_path stardust_wboit

struct WboitParams {
    tiles: vec2<u32>,
    tile_size: u32,
    near: f32,
    // 1 / ln(far / near)
    depth_scale: f32,
}

#ifdef WBOIT_PREPASS
#ifdef WBOIT_PIXEL
struct WboitOutput {
    @location(0) layer_a: vec4<f32>,
#if WBOIT_LAYERS >= 2
    @location(1) layer_b: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 3
    @location(2) layer_c: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 4
    @location(3) layer_d: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 5
    @location(4) layer_e: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 6
    @location(5) layer_f: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 7
    @location(6) layer_g: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 8
    @location(7) layer_h: vec4<f32>,
#endif
}
#else
struct WboitOutput {
    @location(0) tau: f32,
    @location(1) front: f32,
}
#endif
#else ifdef WBOIT_ACCUM
#ifdef WBOIT_PIXEL
struct WboitOutput {
    @location(0) accum: vec4<f32>,
    @location(1) tau: f32,
}
#else
struct WboitOutput {
    @location(0) accum: vec4<f32>,
}
#endif
#else
struct WboitOutput {
    @location(0) color: vec4<f32>,
}
#endif

#ifdef WBOIT_PASS
const BINS: u32 = #{WBOIT_BINS}u;

@group(3) @binding(0) var<uniform> params: WboitParams;

// log spaced between near and far, frag_coord.w is 1 / view depth
fn normalized_depth(frag_coord: vec4<f32>) -> f32 {
    return saturate(-log(frag_coord.w * params.near) * params.depth_scale);
}

fn optical_depth(alpha: f32) -> f32 {
    return -log(max(1.0 - alpha, 1e-6));
}

fn premultiplied(color: vec4<f32>) -> vec4<f32> {
#ifdef PREMULTIPLY_ALPHA
    return color;
#else
    return vec4(color.rgb * color.a, color.a);
#endif
}
#endif

#ifdef WBOIT_PREPASS
// histogram counters are optical depth in 1/256ths
const FIXED_POINT: f32 = 256.0;

@group(3) @binding(1) var<storage, read_write> hist: array<atomic<u32>>;
@group(3) @binding(2) var view_depth: texture_depth_2d;

fn hash(v: vec3<u32>) -> f32 {
    var h = v.x * 747796405u + v.y * 2891336453u + v.z * 3141592653u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h >> 8u) / 16777216.0;
}

// splits the optical depth between the two bins straddling z so scatter matches the
// piecewise linear cdf that gets gathered, rounding stochastically so fragments fainter
// than one step still add up right on average
fn deposit(frag_coord: vec4<f32>, z: f32, od: f32) {
    // the pipeline's depth test runs after the shader once it has side effects, reverse z
    if frag_coord.z < textureLoad(view_depth, vec2<i32>(frag_coord.xy), 0) {
        return;
    }
    let tile = vec2<u32>(frag_coord.xy) / params.tile_size;
    let base = (tile.y * params.tiles.x + tile.x) * BINS;
    let p = z * f32(BINS) - 0.5;
    let lo = i32(floor(p));
    let t = p - f32(lo);
    let a = u32(clamp(lo, 0, i32(BINS) - 1));
    let b = u32(clamp(lo + 1, 0, i32(BINS) - 1));
    let r = hash(vec3(vec2<u32>(frag_coord.xy), bitcast<u32>(frag_coord.z)));
    atomicAdd(&hist[base + a], u32(od * (1.0 - t) * FIXED_POINT + r));
    atomicAdd(&hist[base + b], u32(od * t * FIXED_POINT + fract(r + 0.5)));
}
#endif

#ifdef WBOIT_PREPASS
#ifdef WBOIT_PIXEL
// same tent as deposit, but blended into this pixel's own histogram targets
fn pixel_deposit(z: f32, od: f32) -> WboitOutput {
    var bins: array<vec4<f32>, #{WBOIT_LAYERS}>;
    let p = z * f32(BINS) - 0.5;
    let lo = i32(floor(p));
    let t = p - f32(lo);
    let a = clamp(lo, 0, i32(BINS) - 1);
    let b = clamp(lo + 1, 0, i32(BINS) - 1);
    bins[a >> 2u][a & 3] += od * (1.0 - t);
    bins[b >> 2u][b & 3] += od * t;

    var out: WboitOutput;
    out.layer_a = bins[0];
#if WBOIT_LAYERS >= 2
    out.layer_b = bins[1];
#endif
#if WBOIT_LAYERS >= 3
    out.layer_c = bins[2];
#endif
#if WBOIT_LAYERS >= 4
    out.layer_d = bins[3];
#endif
#if WBOIT_LAYERS >= 5
    out.layer_e = bins[4];
#endif
#if WBOIT_LAYERS >= 6
    out.layer_f = bins[5];
#endif
#if WBOIT_LAYERS >= 7
    out.layer_g = bins[6];
#endif
#if WBOIT_LAYERS >= 8
    out.layer_h = bins[7];
#endif
    return out;
}
#endif
#endif

#ifdef WBOIT_ACCUM
#ifdef WBOIT_PIXEL
@group(3) @binding(1) var pixel_hist: texture_2d_array<f32>;

// the pixel's histogram sums to its optical depth, so reading it up to z gives the optical
// depth in front directly. this fragment's own tent share is taken back out, left in it
// would hide a near opaque layer behind half of itself
fn optical_depth_in_front(p: vec2<i32>, z: f32, od: f32) -> f32 {
    var h: array<f32, BINS>;
    for (var l = 0u; l < BINS / 4u; l++) {
        let v = textureLoad(pixel_hist, p, l, 0);
        for (var c = 0u; c < 4u; c++) {
            h[l * 4u + c] = v[c];
        }
    }
    let e = z * f32(BINS);
    let lo = min(u32(e), BINS - 1u);
    let frac = e - f32(lo);
    var front = frac * h[lo];
    for (var k = 0u; k < lo; k++) {
        front += h[k];
    }

    let tp = z * f32(BINS) - 0.5;
    let tl = i32(floor(tp));
    let t = tp - f32(tl);
    let a = u32(clamp(tl, 0, i32(BINS) - 1));
    let b = u32(clamp(tl + 1, 0, i32(BINS) - 1));
    var own = 0.0;
    if a < lo {
        own += od * (1.0 - t);
    } else if a == lo {
        own += frac * od * (1.0 - t);
    }
    if b < lo {
        own += od * t;
    } else if b == lo {
        own += frac * od * t;
    }
    return max(front - own, 0.0);
}
#else
@group(3) @binding(1) var cdf: texture_2d_array<f32>;
@group(3) @binding(2) var cdf_sampler: sampler;
@group(3) @binding(3) var pixel_tau: texture_2d<f32>;
@group(3) @binding(4) var pixel_front: texture_2d<f32>;
@group(3) @binding(5) var<storage, read> global_edges: array<f32>;

#ifdef WBOIT_GLOBAL
fn global_edge(e: u32) -> f32 {
    if e == 0u {
        return 0.0;
    }
    return global_edges[e - 1u];
}

fn sample_cdf(frag_xy: vec2<f32>, z: f32) -> f32 {
    let e = z * f32(BINS);
    let lo = min(u32(e), BINS - 1u);
    return mix(global_edge(lo), global_edge(lo + 1u), e - f32(lo));
}
#else
// edge 0 is always 0, edge e lives in channel (e-1)&3 of layer (e-1)>>2
fn cdf_edge(e: u32, uv: vec2<f32>) -> f32 {
    if e == 0u {
        return 0.0;
    }
    let c = e - 1u;
    return textureSampleLevel(cdf, cdf_sampler, uv, c >> 2u, 0.0)[c & 3u];
}

fn cdf_at(uv: vec2<f32>, z: f32) -> f32 {
    let e = z * f32(BINS);
    let lo = min(u32(e), BINS - 1u);
    return mix(cdf_edge(lo, uv), cdf_edge(lo + 1u, uv), e - f32(lo));
}

// cubic b-spline across tiles from 4 bilinear taps so the weight field stays C1,
// exp(-tau * cdf) turns C0 kinks into visible mach bands
fn sample_cdf(frag_xy: vec2<f32>, z: f32) -> f32 {
    let tiles = vec2<f32>(params.tiles);
    let x = frag_xy / f32(params.tile_size) - 0.5;
    let i = floor(x);
    let f = x - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let s0 = w0 + w1;
    let s1 = w2 + w3;
    let p0 = (i + w1 / s0) / tiles;
    let p1 = (i + 2.0 + w3 / s1) / tiles;

    return s0.y * (s0.x * cdf_at(vec2(p0.x, p0.y), z) + s1.x * cdf_at(vec2(p1.x, p0.y), z))
         + s1.y * (s0.x * cdf_at(vec2(p0.x, p1.y), z) + s1.x * cdf_at(vec2(p1.x, p1.y), z));
}
#endif
#endif
#endif

fn wboit_output(frag_coord: vec4<f32>, color: vec4<f32>) -> WboitOutput {
    var out: WboitOutput;
#ifdef WBOIT_PREPASS
    let c = premultiplied(color);
    let z = normalized_depth(frag_coord);
    let od = optical_depth(c.a);
#ifdef WBOIT_PIXEL
    out = pixel_deposit(z, min(od, 16.0));
#else
    deposit(frag_coord, z, min(od, 16.0));
    out.tau = od;
    // stored as depth since the cdf isn't built yet, it's monotonic so the nearest depth
    // is still the smallest quantile. faint fragments like antialiased edges aren't a
    // surface, anchoring to one would push every real layer behind it down onto the
    // weight floor where they all blend evenly
    out.front = select(1.0, z, c.a >= 0.15);
#endif
#else ifdef WBOIT_ACCUM
    let c = premultiplied(color);
    let p = vec2<i32>(frag_coord.xy);
#ifdef WBOIT_PIXEL
    let od = optical_depth(c.a);
    let w = max(exp(-optical_depth_in_front(p, normalized_depth(frag_coord), min(od, 16.0))), exp2(-16.0));
    out.accum = c * w * 64.0;
    out.tau = od;
#else
    let q = sample_cdf(frag_coord.xy, normalized_depth(frag_coord));
    let front = sample_cdf(frag_coord.xy, textureLoad(pixel_front, p, 0).r);
    // relative to the pixel's front most quantile, a diluted cdf would otherwise underflow
    // every weight in the pixel to 0 in f16 once tau gets large
    let w = clamp(exp(-textureLoad(pixel_tau, p, 0).r * (q - front)), exp2(-16.0), 1.0);
    // scaled so the floor stays a normal f16
    out.accum = c * w * 64.0;
#endif
#else
    out.color = color;
#endif
    return out;
}
