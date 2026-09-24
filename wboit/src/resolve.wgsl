#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

const LAYERS: u32 = #{WBOIT_LAYERS}u;
const BINS: u32 = LAYERS * 4u;

@group(0) @binding(0) var hist: texture_2d_array<f32>;

struct CdfOutput {
    @location(0) e0: vec4<f32>,
#if WBOIT_LAYERS >= 2
    @location(1) e1: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 3
    @location(2) e2: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 4
    @location(3) e3: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 5
    @location(4) e4: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 6
    @location(5) e5: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 7
    @location(6) e6: vec4<f32>,
#endif
#if WBOIT_LAYERS >= 8
    @location(7) e7: vec4<f32>,
#endif
}

// edges[i] is the cdf at bin edge i + 1, edge 0 is implicitly 0 so a fragment never
// occludes itself or anything sharing its bin
@fragment
fn fragment(in: FullscreenVertexOutput) -> CdfOutput {
    let p = vec2<i32>(in.position.xy);
    var edges: array<f32, BINS>;
    var acc = 0.0;
    for (var l = 0u; l < LAYERS; l++) {
        let h = textureLoad(hist, p, l, 0);
        for (var c = 0u; c < 4u; c++) {
            acc += h[c];
            edges[l * 4u + c] = acc;
        }
    }
    for (var i = 0u; i < BINS; i++) {
        if acc > 0.0 {
            edges[i] /= acc;
        } else {
            edges[i] = f32(i + 1u) / f32(BINS);
        }
    }

    var out: CdfOutput;
    out.e0 = vec4(edges[0], edges[1], edges[2], edges[3]);
#if WBOIT_LAYERS >= 2
    out.e1 = vec4(edges[4], edges[5], edges[6], edges[7]);
#endif
#if WBOIT_LAYERS >= 3
    out.e2 = vec4(edges[8], edges[9], edges[10], edges[11]);
#endif
#if WBOIT_LAYERS >= 4
    out.e3 = vec4(edges[12], edges[13], edges[14], edges[15]);
#endif
#if WBOIT_LAYERS >= 5
    out.e4 = vec4(edges[16], edges[17], edges[18], edges[19]);
#endif
#if WBOIT_LAYERS >= 6
    out.e5 = vec4(edges[20], edges[21], edges[22], edges[23]);
#endif
#if WBOIT_LAYERS >= 7
    out.e6 = vec4(edges[24], edges[25], edges[26], edges[27]);
#endif
#if WBOIT_LAYERS >= 8
    out.e7 = vec4(edges[28], edges[29], edges[30], edges[31]);
#endif
    return out;
}
