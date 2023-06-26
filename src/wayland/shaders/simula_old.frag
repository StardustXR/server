#version 320 es
#extension GL_OES_EGL_image_external : require
precision mediump float;
precision highp int;

layout(binding = 0, std140) uniform _Global
{
    highp vec4 diffuse_i;
    highp vec2 uv_scale;
    highp vec2 uv_offset;
    highp float fcFactor;
    highp float ripple;
    highp float alpha_min;
    highp float alpha_max;
} uniforms;

layout(binding = 0) uniform highp samplerExternalOES diffuse;

layout(location = 0) in highp vec2 fs_uv;
layout(location = 0) out highp vec4 fragColor;

void main()
{
    highp vec2 dx_uv = dFdx(fs_uv);
    highp vec2 dy_uv = dFdy(fs_uv);
    highp vec2 width = fs_uv * uniforms.diffuse_i.xy;
    ivec2 _475 = ivec2(width);
    highp vec2 _477 = clamp(floor(abs(vec2(dx_uv.x, dy_uv.y)) * uniforms.diffuse_i.xy), vec2(1.0), vec2(2.0));
    ivec2 _480 = ivec2(_477);
    ivec2 _481 = _475 - _480;
    ivec2 _485 = _475 + _480;
    int _487 = _481.y;
    highp vec4 _671;
    highp float _672;
    _672 = 0.0;
    _671 = vec4(0.0);
    highp vec4 _679;
    highp float _681;
    for (int _670 = _487; _670 <= _485.y; _672 = _681, _671 = _679, _670++)
    {
        int _496 = _481.x;
        _681 = _672;
        _679 = _671;
        highp vec4 _553;
        highp float _556;
        for (int _673 = _496; _673 <= _485.x; _681 = _556, _679 = _553, _673++)
        {
            highp float _509 = float(_673);
            highp float _514 = (uniforms.fcFactor * (width.x - _509)) / _477.x;
            highp float _520 = float(_670);
            highp float _525 = (uniforms.fcFactor * (width.y - _520)) / _477.y;
            highp float _533 = sqrt((_514 * _514) + (_525 * _525));
            highp float _675;
            do
            {
                if (_533 > 1.0)
                {
                    _675 = 0.0;
                    break;
                }
                highp float _592 = pow(uniforms.ripple * sqrt(1.0 - (_533 * _533)), 2.0);
                _675 = 1.0 + (_592 * (0.25 + (_592 * (0.015625 + (_592 * (0.00043402801384218037128448486328125 + (_592 * (6.7816799855791032314300537109375e-06 + (_592 * (6.7816799287356843706220388412476e-08 + (_592 * (4.709500012189948847662890329957e-10 + (_592 * (2.4028099388645474121517509047408e-12 + (_592 * (9.3859703944590075486154034933861e-15 + (_592 * (2.8968999943407451927966655969016e-17 + (7.242260299760125752555485045131e-20 * _592)))))))))))))))))));
                break;
            } while(false);
            _553 = _679 + (texture2D(diffuse, (vec2(_509, _520) + vec2(0.5)) / uniforms.diffuse_i.xy) * _675);
            _556 = _681 + _675;
        }
    }
    highp vec4 _568 = _671 / vec4(_672);
    highp vec3 _417 = pow(_568.xyz, vec3(2.2000000476837158203125));
    highp vec4 _669 = vec4(_417.x, _417.y, _417.z, _568.w);
    _669.w = uniforms.alpha_min + (_568.w * (uniforms.alpha_max - uniforms.alpha_min));
    fragColor = _669;
}

