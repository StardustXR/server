#import bevy_pbr::{
    forward_io::VertexOutput,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#import stardust_wboit::{WboitOutput, wboit_output}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> WboitOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var color = pbr_input.material.base_color;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        color = apply_pbr_lighting(pbr_input);
    }
    return wboit_output(in.position, main_pass_post_lighting_processing(pbr_input, color));
}
