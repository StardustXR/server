#version 320 es
// #ifdef GL_AMD_vertex_shader_layer
// #extension GL_AMD_vertex_shader_layer : enable
// #elif defined(GL_NV_viewport_array2)
// #extension GL_NV_viewport_array2 : enable
// #else
// #define gl_Layer int _dummy_gl_layer_var
// #endif

struct Inst
{
    mat4 world;
    vec4 color;
};

layout(binding = 1, std140) uniform StereoKitBuffer
{
    layout(row_major) mat4 sk_view[2];
    layout(row_major) mat4 sk_proj[2];
    layout(row_major) mat4 sk_proj_inv[2];
    layout(row_major) mat4 sk_viewproj[2];
    vec4 sk_lighting_sh[9];
    vec4 sk_camera_pos[2];
    vec4 sk_camera_dir[2];
    vec4 sk_fingertip[2];
    vec4 sk_cubemap_i;
    float sk_time;
    uint sk_view_count;
} _38;

layout(binding = 2, std140) uniform TransformBuffer
{
    layout(row_major) Inst sk_inst[819];
} _56;

layout(binding = 0, std140) uniform _Global
{
    vec4 diffuse_i;
    vec2 uv_scale;
    vec2 uv_offset;
    float fcFactor;
    float ripple;
    float alpha_min;
    float alpha_max;
} _91;

layout(location = 0) in vec4 input_pos;
layout(location = 1) in vec3 input_norm;
layout(location = 2) in vec2 input_uv;
layout(location = 0) out vec2 fs_uv;

mat4 spvWorkaroundRowMajor(mat4 wrap) { return wrap; }

void main()
{
    uint _155 = uint(gl_InstanceID) % _38.sk_view_count;
    gl_Position = spvWorkaroundRowMajor(_38.sk_viewproj[_155]) * vec4((spvWorkaroundRowMajor(_56.sk_inst[uint(gl_InstanceID) / _38.sk_view_count].world) * vec4(input_pos.xyz, 1.0)).xyz, 1.0);
    fs_uv = (input_uv + _91.uv_offset) * _91.uv_scale;
    // gl_Layer = int(_155);
}

