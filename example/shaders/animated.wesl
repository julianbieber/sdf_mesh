import sdf_mesh::sdf_lib::{
    Surface,
    surface,
    sd_sphere,
    sd_capsule,
    op_smooth_union,
    op_translate,
    op_rotate_z,
};

const ANIMATION_SECONDS: f32 = 2.0;
const ANIMATION_FRAMES: u32 = 30;

fn sdf_scene(p: vec3<f32>, time: f32) -> Surface {
    let phase = sin(6.2831855 * time / ANIMATION_SECONDS);

    let body = surface(sd_sphere(p, 0.6), 0u);

    let arm_p = op_rotate_z(op_translate(p, vec3(0.45, 0.2, 0.0)), 0.7 * phase);
    let arm = surface(sd_capsule(arm_p, vec3(0.0), vec3(0.7, 0.0, 0.0), 0.14), 1u);
    var shape = op_smooth_union(body, arm, 0.12);

    let blob_x = -0.95 - 0.35 * phase;
    let blob = surface(sd_sphere(op_translate(p, vec3(blob_x, -0.1, 0.0)), 0.25), 2u);
    shape = op_smooth_union(shape, blob, 0.2);

    return shape;
}
