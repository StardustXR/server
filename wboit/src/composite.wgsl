#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var accum: texture_2d<f32>;
@group(0) @binding(1) var tau: texture_2d<f32>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let a = textureLoad(accum, p, 0);
    let alpha = 1.0 - exp(-textureLoad(tau, p, 0).r);
    return vec4(a.rgb / max(a.a, 1e-5) * alpha, alpha);
}
