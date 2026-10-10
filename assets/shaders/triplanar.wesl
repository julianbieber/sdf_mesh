import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
};

@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

const MATERIAL_SLOTS: u32 = 4u;
const BLEND_SLOTS: u32 = 2u;

struct TriplanarSettings {
    scale: f32,
    blend_sharpness: f32,
}

struct MaterialParams {
    @align(16) uv_scale: f32,
    normal_strength: f32,
    roughness_scale: f32,
    metallic_scale: f32,
    @size(16) emissive_strength: f32,
}

struct MaterialParamsTable {
    entries: array<MaterialParams, MATERIAL_SLOTS>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> triplanar: TriplanarSettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var base_color_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var layer_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var normal_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var orm_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var emissive_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var<uniform> material_params: MaterialParamsTable;

struct PlaneGradients {
    zy_dx: vec2<f32>,
    zy_dy: vec2<f32>,
    xz_dx: vec2<f32>,
    xz_dy: vec2<f32>,
    xy_dx: vec2<f32>,
    xy_dy: vec2<f32>,
}

struct Picked {
    indices: array<u32, BLEND_SLOTS>,
    weights: array<f32, BLEND_SLOTS>,
}

struct SurfaceSample {
    base_color: vec3<f32>,
    normal: vec3<f32>,
    occlusion: f32,
    roughness: f32,
    metallic: f32,
    emissive: vec3<f32>,
}

fn plane_gradients(world_position: vec3<f32>) -> PlaneGradients {
    var gradients: PlaneGradients;
    gradients.zy_dx = dpdx(world_position.zy);
    gradients.zy_dy = dpdy(world_position.zy);
    gradients.xz_dx = dpdx(world_position.xz);
    gradients.xz_dy = dpdy(world_position.xz);
    gradients.xy_dx = dpdx(world_position.xy);
    gradients.xy_dy = dpdy(world_position.xy);
    return gradients;
}

fn pick_slots(weights: vec4<f32>) -> Picked {
    var pool = array<f32, MATERIAL_SLOTS>(weights.x, weights.y, weights.z, weights.w);
    var picked: Picked;
    var total = 0.0;

    for (var slot = 0u; slot < BLEND_SLOTS; slot += 1u) {
        var best = 0u;
        var best_weight = -1.0;
        for (var candidate = 0u; candidate < MATERIAL_SLOTS; candidate += 1u) {
            if pool[candidate] > best_weight {
                best_weight = pool[candidate];
                best = candidate;
            }
        }
        pool[best] = -1.0;
        picked.indices[slot] = best;
        picked.weights[slot] = max(best_weight, 0.0);
        total += picked.weights[slot];
    }

    let inverse_total = 1.0 / max(total, 1e-5);
    for (var slot = 0u; slot < BLEND_SLOTS; slot += 1u) {
        picked.weights[slot] *= inverse_total;
    }

    return picked;
}

fn plane_blend(world_normal: vec3<f32>) -> vec3<f32> {
    let weights = pow(abs(world_normal), vec3(triplanar.blend_sharpness));
    return weights / max(weights.x + weights.y + weights.z, 1e-5);
}

fn tangent_normal(encoded: vec3<f32>, strength: f32) -> vec3<f32> {
    var normal = encoded * 2.0 - 1.0;
    normal = vec3(normal.xy * strength, normal.z);
    return normalize(normal);
}

fn sample_layer(
    layer: u32,
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    gradients: PlaneGradients,
    params: MaterialParams,
) -> SurfaceSample {
    let scale = triplanar.scale * params.uv_scale;
    let index = i32(layer);
    let blend = plane_blend(world_normal);

    let uv_x = world_position.zy * scale;
    let uv_y = world_position.xz * scale;
    let uv_z = world_position.xy * scale;

    let dx_x = gradients.zy_dx * scale;
    let dy_x = gradients.zy_dy * scale;
    let dx_y = gradients.xz_dx * scale;
    let dy_y = gradients.xz_dy * scale;
    let dx_z = gradients.xy_dx * scale;
    let dy_z = gradients.xy_dy * scale;

    let base_x = textureSampleGrad(base_color_layers, layer_sampler, uv_x, index, dx_x, dy_x).rgb;
    let base_y = textureSampleGrad(base_color_layers, layer_sampler, uv_y, index, dx_y, dy_y).rgb;
    let base_z = textureSampleGrad(base_color_layers, layer_sampler, uv_z, index, dx_z, dy_z).rgb;

    let orm_x = textureSampleGrad(orm_layers, layer_sampler, uv_x, index, dx_x, dy_x).rgb;
    let orm_y = textureSampleGrad(orm_layers, layer_sampler, uv_y, index, dx_y, dy_y).rgb;
    let orm_z = textureSampleGrad(orm_layers, layer_sampler, uv_z, index, dx_z, dy_z).rgb;

    let emissive_x = textureSampleGrad(emissive_layers, layer_sampler, uv_x, index, dx_x, dy_x).rgb;
    let emissive_y = textureSampleGrad(emissive_layers, layer_sampler, uv_y, index, dx_y, dy_y).rgb;
    let emissive_z = textureSampleGrad(emissive_layers, layer_sampler, uv_z, index, dx_z, dy_z).rgb;

    let raw_x = textureSampleGrad(normal_layers, layer_sampler, uv_x, index, dx_x, dy_x).rgb;
    let raw_y = textureSampleGrad(normal_layers, layer_sampler, uv_y, index, dx_y, dy_y).rgb;
    let raw_z = textureSampleGrad(normal_layers, layer_sampler, uv_z, index, dx_z, dy_z).rgb;

    let normal_x = tangent_normal(raw_x, params.normal_strength);
    let normal_y = tangent_normal(raw_y, params.normal_strength);
    let normal_z = tangent_normal(raw_z, params.normal_strength);

    let reoriented_x = vec3(normal_x.xy + world_normal.zy, abs(normal_x.z) * world_normal.x);
    let reoriented_y = vec3(normal_y.xy + world_normal.xz, abs(normal_y.z) * world_normal.y);
    let reoriented_z = vec3(normal_z.xy + world_normal.xy, abs(normal_z.z) * world_normal.z);

    let orm = orm_x * blend.x + orm_y * blend.y + orm_z * blend.z;

    var sample: SurfaceSample;
    sample.base_color = base_x * blend.x + base_y * blend.y + base_z * blend.z;
    sample.normal = normalize(
        reoriented_x.zyx * blend.x + reoriented_y.xzy * blend.y + reoriented_z.xyz * blend.z,
    );
    sample.occlusion = orm.r;
    sample.roughness = orm.g * params.roughness_scale;
    sample.metallic = orm.b * params.metallic_scale;
    sample.emissive = (emissive_x * blend.x + emissive_y * blend.y + emissive_z * blend.z)
        * params.emissive_strength;
    return sample;
}

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let in = vertex_output;

    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let world_position = in.world_position.xyz;
    let world_normal = normalize(in.world_normal);
    let gradients = plane_gradients(world_position);

@if(VERTEX_COLORS)
    let weights = in.color;
@if(!VERTEX_COLORS)
    let weights = vec4(1.0, 0.0, 0.0, 0.0);

    let picked = pick_slots(weights);

    var base_color = vec3(0.0);
    var normal = vec3(0.0);
    var occlusion = 0.0;
    var roughness = 0.0;
    var metallic = 0.0;
    var emissive = vec3(0.0);

    for (var slot = 0u; slot < BLEND_SLOTS; slot += 1u) {
        let index = picked.indices[slot];
        let weight = picked.weights[slot];
        let sample = sample_layer(
            index,
            world_position,
            world_normal,
            gradients,
            material_params.entries[index],
        );

        base_color += sample.base_color * weight;
        normal += sample.normal * weight;
        occlusion += sample.occlusion * weight;
        roughness += sample.roughness * weight;
        metallic += sample.metallic * weight;
        emissive += sample.emissive * weight;
    }

    pbr_input.material.base_color = vec4(base_color, 1.0);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
    pbr_input.material.perceptual_roughness = clamp(roughness, 0.045, 1.0);
    pbr_input.material.metallic = clamp(metallic, 0.0, 1.0);
    pbr_input.material.emissive = vec4(emissive, 1.0);
    pbr_input.diffuse_occlusion = vec3(clamp(occlusion, 0.0, 1.0));
    pbr_input.N = normalize(normal);

@if(PREPASS_PIPELINE)
    let out = deferred_output(in, pbr_input);
@if(!PREPASS_PIPELINE)
    var out: FragmentOutput;
@if(!PREPASS_PIPELINE)
    out.color = apply_pbr_lighting(pbr_input);
@if(!PREPASS_PIPELINE)
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

    return out;
}
