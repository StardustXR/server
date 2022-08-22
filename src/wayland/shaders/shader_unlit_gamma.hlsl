#include "stereokit.hlsli"

//--name = sk/unlit
//--diffuse     = white
//--uv_offset   = 0.0, 0.0
//--uv_scale    = 1.0, 1.0
Texture2D    diffuse   : register(t0);
SamplerState diffuse_s : register(s0);
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
float4 ps(psIn input) : SV_TARGET {
	float4 col = diffuse.Sample(diffuse_s, input.uv);
	col.rgb = pow(col.rgb, float3(2.2));

	return col; 
}
