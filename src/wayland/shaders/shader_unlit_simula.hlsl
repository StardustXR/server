#include "stereokit.hlsli"

// Port of https://github.com/SimulaVR/Simula/blob/master/addons/godot-haskell-plugin/TextShader.tres to StereoKit and HLSL.

//--name = stardust/text_shader
//--diffuse     = white
//--fcFactor    = 1.0
//--ripple      = 4.0
//--size        = 256, 256
//--uv_offset   = 0.0, 0.0
//--uv_scale    = 1.0, 1.0
Texture2D    diffuse   : register(t0);
SamplerState diffuse_s : register(s0);
int2         size;
float        fcFactor;
float        ripple;
float2       uv_scale;
float2       uv_offset;

struct vsIn {
	float4 pos  : SV_Position;
	float3 norm : NORMAL0;
	float2 uv   : TEXCOORD0;
};
struct psIn {
	float4 pos   : SV_POSITION;
	float2 uv    : TEXCOORD0;
	uint view_id : SV_RenderTargetArrayIndex;
};

psIn vs(vsIn input, uint id : SV_InstanceID) {
	psIn o;
	o.view_id = id % sk_view_count;
	id        = id / sk_view_count;

	float3 world = mul(float4(input.pos.xyz, 1), sk_inst[id].world).xyz;
	o.pos        = mul(float4(world,         1), sk_viewproj[o.view_id]);

	o.uv    = (input.uv) + uv_offset * uv_scale;
	return o;
}

// float gaussian(float x, float t) {
// 	float PI = 3.14159265358;
// 	return exp(-x*x/(2.0 * t*t))/(sqrt(2.0*PI)*t);
// }

float besselI0(float x) {
	return 1.0 + pow(x, 2.0) * (0.25 +  pow(x, 2.0) * (0.015625 +  pow(x, 2.0) * (0.000434028 +  pow(x, 2.0) * (6.78168e-6 +  pow(x, 2.0) * (6.78168e-8 +  pow(x, 2.0) * (4.7095e-10 +  pow(x, 2.0) * (2.40281e-12 + pow(x, 2.0) * (9.38597e-15 + pow(x, 2.0) * (2.8969e-17 + 7.24226e-20 * pow(x, 2.0))))))))));
}

float kaiser(float x, float alpha) {
	if (x > 1.0) { 
		return 0.0;
	}
	return besselI0(alpha * sqrt(1.0-x*x));
}

float4 lowpassFilter(Texture2D tex, sampler2D texSampler, float2 uv, float alpha) {
	float PI = 3.14159265358;
	
	float4 q = float4(0.0);
	
	float2 dx_uv = ddx(uv);
	float2 dy_uv = ddy(uv);
	//float width = sqrt(max(dot(dx_uv, dx_uv), dot(dy_uv, dy_uv)));
	float2 width = abs(float2(dx_uv.x, dy_uv.y));
	
	float2 pixelWidth = floor(width * float2(size));
	float2 aspectRatio = normalize(pixelWidth);
	
	float2 xyf = uv * float2(size);
	int2 xy = int2(xyf);
	
	pixelWidth = clamp(pixelWidth, float2(1.0), float2(2.0));

	int2 start = xy - int2(pixelWidth);
	int2 end = xy + int2(pixelWidth);
	
	float4 outColor = float4(0.0);
	
	float qSum = 0.0;
	
	for (int v = start.y; v <= end.y; v++) {
		for (int u = start.x; u <= end.x; u++) {
			float kx = fcFactor * (xyf.x - float(u))/pixelWidth.x;
			float ky = fcFactor * (xyf.y - float(v))/pixelWidth.y;
			 
			//float lanczosValue = gaussian(kx, fcx);
			float lanczosValue = kaiser(sqrt(kx*kx + ky*ky), alpha);
			
			q += tex.Sample(texSampler, (float2(u, v)+float2(0.5))/float2(size)) * lanczosValue;
			// q += tex.Load(int3(u, v, 0)) * lanczosValue;
			qSum += lanczosValue;
		}
	}
	
	return q/qSum;
}

float4 ps(psIn input) : SV_TARGET {
	float gamma = 2.2;
	// float4 col = diffuse.Sample(diffuse_s, input.uv);

	// float4 col = lowpassFilter(diffuse, diffuse_s, diffuse_i.xy, float2(1.0 - input.uv.x, input.uv.y), ripple);
	float4 col = lowpassFilter(diffuse, diffuse_s, input.uv, ripple);
	// float4 col = diffuse.Sample(diffuse_s, input.uv);
	col.rgb = pow(col.rgb, float3(gamma));

	return col; 
}
