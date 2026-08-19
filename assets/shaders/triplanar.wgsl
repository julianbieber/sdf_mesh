#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#import bevy_pbr::pbr_functions::alpha_discard

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{VertexOutput, FragmentOutput}
#import bevy_pbr::pbr_deferred_functions::deferred_output
#else
#import bevy_pbr::forward_io::{VertexOutput, FragmentOutput}
#import bevy_pbr::pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing}
#endif

struct TriplanarSettings {
    scale: f32,
    blend_sharpness: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> triplanar: TriplanarSettings;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var layer_texture: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var layer_sampler: sampler;

fn triplanar_layer(world_position: vec3<f32>, world_normal: vec3<f32>, layer: i32) -> vec3<f32> {
    let uv = world_position * triplanar.scale;

    let x_plane = textureSample(layer_texture, layer_sampler, uv.zy, layer).rgb;
    let y_plane = textureSample(layer_texture, layer_sampler, uv.xz, layer).rgb;
    let z_plane = textureSample(layer_texture, layer_sampler, uv.xy, layer).rgb;

    var blend = pow(abs(world_normal), vec3(triplanar.blend_sharpness));
    blend = blend / max(blend.x + blend.y + blend.z, 1e-5);

    return x_plane * blend.x + y_plane * blend.y + z_plane * blend.z;
}

fn triplanar_color(world_position: vec3<f32>, world_normal: vec3<f32>, weights: vec4<f32>) -> vec3<f32> {
    let total = max(weights.x + weights.y + weights.z + weights.w, 1e-5);
    let w = weights / total;

    var color = vec3(0.0);
    if w.x > 0.001 {
        color += triplanar_layer(world_position, world_normal, 0) * w.x;
    }
    if w.y > 0.001 {
        color += triplanar_layer(world_position, world_normal, 1) * w.y;
    }
    if w.z > 0.001 {
        color += triplanar_layer(world_position, world_normal, 2) * w.z;
    }
    if w.w > 0.001 {
        color += triplanar_layer(world_position, world_normal, 3) * w.w;
    }
    return color;
}

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let in = vertex_output;

    var pbr_input = pbr_input_from_standard_material(in, is_front);

#ifdef VERTEX_COLORS
    let weights = in.color;
#else
    let weights = vec4(1.0, 0.0, 0.0, 0.0);
#endif

    let tinted = triplanar_color(in.world_position.xyz, normalize(in.world_normal), weights);
    pbr_input.material.base_color = vec4(tinted, 1.0);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    return out;
}
