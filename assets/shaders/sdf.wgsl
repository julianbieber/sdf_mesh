#define_import_path sdf_mesh::sdf

#import sdf_mesh::sdf_lib::{
    Surface,
    surface,
    sd_sphere,
    sd_box,
    sd_round_box,
    sd_cylinder,
    sd_torus,
    sd_plane,
    sd_capsule,
    op_union,
    op_intersect,
    op_subtract,
    op_smooth_union,
    op_smooth_intersect,
    op_smooth_subtract,
    op_translate,
    op_repeat,
    op_rotate_x,
    op_rotate_y,
    op_rotate_z,
    op_mirror_x,
    op_twist_y,
}

// TODO(jb-comment): explain that this function is the whole editable surface of the tool,
// that material ids 0-3 select triplanar texture layers, and that saving re-dispatches both
// compute passes without a restart

fn sdf_scene(p: vec3<f32>, time: f32) -> Surface {
    let body = surface(sd_round_box(p, vec3(0.9, 0.65, 0.55), 0.18), 3u);

    let dome = surface(sd_sphere(op_translate(p, vec3(0.0, 0.55, 0.0)), 0.62), 1u);
    var shape = op_smooth_union(body, dome, 0.35);

    let ring = surface(sd_torus(op_rotate_x(p, 1.5707964), 1.05, 0.11), 2u);
    shape = op_union(shape, ring);

    let bore = surface(sd_cylinder(op_translate(p, vec3(0.0, 0.0, 0.0)), 3.0, 0.38), 3u);
    shape = op_smooth_subtract(shape, bore, 0.12);

    let studs = surface(
        sd_sphere(op_translate(op_mirror_x(p), vec3(0.72, -0.45, 0.0)), 0.16),
        3u,
    );
    shape = op_smooth_union(shape, studs, 0.18);

    return shape;
}
