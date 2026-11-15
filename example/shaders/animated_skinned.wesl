import sdf_mesh::sdf_lib::{
    Surface,
    Joint,
    surface,
    on_joint,
    joint,
    joint_identity,
    quat_axis_angle,
    op_joint,
    sd_sphere,
    sd_capsule,
    op_smooth_union,
};

const ANIMATION_SECONDS: f32 = 2.0;
const ANIMATION_FRAMES: u32 = 30;
const JOINT_COUNT: u32 = 3;

const BODY: u32 = 0u;
const ARM: u32 = 1u;
const BLOB: u32 = 2u;

fn sdf_joint(i: u32, time: f32) -> Joint {
    let phase = sin(6.2831855 * time / ANIMATION_SECONDS);
    if i == ARM {
        return joint(vec3(0.45, 0.2, 0.0), quat_axis_angle(vec3(0.0, 0.0, 1.0), -0.7 * phase));
    }
    if i == BLOB {
        return joint(vec3(-0.95 - 0.35 * phase, -0.1, 0.0), vec4(0.0, 0.0, 0.0, 1.0));
    }
    return joint_identity();
}

fn sdf_scene(p: vec3<f32>, time: f32) -> Surface {
    let body = on_joint(surface(sd_sphere(op_joint(p, sdf_joint(BODY, time)), 0.6), 0u), BODY);

    let arm_p = op_joint(p, sdf_joint(ARM, time));
    let arm = on_joint(surface(sd_capsule(arm_p, vec3(0.0), vec3(0.7, 0.0, 0.0), 0.14), 1u), ARM);
    var shape = op_smooth_union(body, arm, 0.12);

    let blob = on_joint(surface(sd_sphere(op_joint(p, sdf_joint(BLOB, time)), 0.25), 2u), BLOB);
    shape = op_smooth_union(shape, blob, 0.2);

    return shape;
}
