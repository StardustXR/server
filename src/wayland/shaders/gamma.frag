#version 320 es
precision mediump float;
precision highp int;

layout(binding = 0) uniform highp sampler2D diffuse;

layout(location = 0) in highp vec2 fs_uv;
layout(location = 0) out highp vec4 _entryPointOutput;

void main()
{
    highp vec4 _101 = texture(diffuse, fs_uv);
    highp vec3 _104 = pow(_101.xyz, vec3(2.2000000476837158203125));
    _entryPointOutput = vec4(_104.x, _104.y, _104.z, _101.w);
}

