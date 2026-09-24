#define_import_path stardust_wboit

struct WboitParams {
    tiles: vec2<u32>,
    tile_size: u32,
    near: f32,
    // 1 / ln(far / near)
    depth_scale: f32,
}

#ifdef WBOIT_ACCUM
struct WboitOutput {
    @location(0) accum: vec4<f32>,
    @location(1) tau: f32,
    @location(2) front: f32,
}
#else ifdef WBOIT_BIN
struct WboitOutput {
    @location(0) h0: vec4<f32>,
#if WBOIT_LAYERS >= 2
    @location(1) h1: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 3
    @location(2) h2: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 4
    @location(3) h3: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 5
    @location(4) h4: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 6
    @location(5) h5: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 7
    @location(6) h6: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 8
    @location(7) h7: vec4<f32>,
#endif
}
#else
struct WboitOutput {
    @location(0) color: vec4<f32>,
}
#endif

#ifdef WBOIT_PASS
// four bins per rgba layer
const BINS: u32 = #{WBOIT_LAYERS}u * 4u;

@group(3) @binding(0) var<uniform> params: WboitParams;
@group(3) @binding(1) var cdf: texture_2d_array<f32>;
@group(3) @binding(2) var cdf_sampler: sampler;
@group(3) @binding(3) var prev_tau: texture_2d<f32>;
@group(3) @binding(4) var prev_front: texture_2d<f32>;

// log spaced between near and far, frag_coord.w is 1 / view depth
fn normalized_depth(frag_coord: vec4<f32>) -> f32 {
    return saturate(-log(frag_coord.w * params.near) * params.depth_scale);
}

fn optical_depth(alpha: f32) -> f32 {
    return -log(max(1.0 - alpha, 1e-6));
}
#endif

#ifdef WBOIT_ACCUM
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

#ifdef WBOIT_BIN
// splits the optical depth between the two bins straddling z so scatter matches the
// piecewise linear cdf the accum pass gathers
fn tent_deposit(z: f32, od: f32) -> WboitOutput {
    var bins: array<vec4<f32>, #{WBOIT_LAYERS}>;
    let p = z * f32(BINS) - 0.5;
    let lo = i32(floor(p));
    let t = p - f32(lo);
    let a = clamp(lo, 0, i32(BINS) - 1);
    let b = clamp(lo + 1, 0, i32(BINS) - 1);
    bins[a >> 2u][a & 3] += od * (1.0 - t);
    bins[b >> 2u][b & 3] += od * t;

    var out: WboitOutput;
    out.h0 = bins[0];
#if WBOIT_LAYERS >= 2
    out.h1 = bins[1];
#endif
#if WBOIT_LAYERS >= 3
    out.h2 = bins[2];
#endif
#if WBOIT_LAYERS >= 4
    out.h3 = bins[3];
#endif
#if WBOIT_LAYERS >= 5
    out.h4 = bins[4];
#endif
#if WBOIT_LAYERS >= 6
    out.h5 = bins[5];
#endif
#if WBOIT_LAYERS >= 7
    out.h6 = bins[6];
#endif
#if WBOIT_LAYERS >= 8
    out.h7 = bins[7];
#endif
    return out;
}
#endif

fn wboit_output(frag_coord: vec4<f32>, color: vec4<f32>) -> WboitOutput {
#ifdef PREMULTIPLY_ALPHA
    let c = color;
#else
    let c = vec4(color.rgb * color.a, color.a);
#endif

#ifdef WBOIT_ACCUM
    let p = vec2<i32>(frag_coord.xy);
    let q = sample_cdf(frag_coord.xy, normalized_depth(frag_coord));
    // relative to the pixel's front most quantile last frame, a diluted tile cdf would
    // otherwise underflow every weight in the pixel to 0 in f16 once tau gets large
    let w = clamp(exp(-textureLoad(prev_tau, p, 0).r * (q - textureLoad(prev_front, p, 0).r)), exp2(-16.0), 1.0);
    var out: WboitOutput;
    // scaled so the floor stays a normal f16
    out.accum = c * w * 64.0;
    out.tau = optical_depth(c.a);
    out.front = q;
    return out;
#else ifdef WBOIT_BIN
    return tent_deposit(normalized_depth(frag_coord), min(optical_depth(c.a), 16.0));
#else
    var out: WboitOutput;
    out.color = color;
    return out;
#endif
}
