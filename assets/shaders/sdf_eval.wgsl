#import sdf_mesh::sdf_types::{SdfParams, FieldSample}
#import sdf_mesh::sdf::sdf_scene

@group(0) @binding(0) var<uniform> params: SdfParams;
@group(0) @binding(1) var<storage, read_write> field: array<FieldSample>;

@compute @workgroup_size(4, 4, 4)
fn evaluate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let res = params.grid_res;
    if gid.x >= res || gid.y >= res || gid.z >= res {
        return;
    }

    let position = params.origin + vec3<f32>(gid) * params.cell_size;
    let sample = sdf_scene(position, params.time);

    let index = gid.x + gid.y * res + gid.z * res * res;
    field[index] = FieldSample(sample.weights, sample.dist);
}
