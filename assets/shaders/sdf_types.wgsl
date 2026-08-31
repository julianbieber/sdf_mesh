#define_import_path sdf_mesh::sdf_types

struct FieldSample {
    weights: vec4<f32>,
    dist: f32,
}

struct SdfParams {
    origin: vec3<f32>,
    cell_size: f32,
    grid_res: u32,
    max_vertices: u32,
    max_indices: u32,
    vertex_stride: u32,
    vertex_base: u32,
    index_base: u32,
    iso: f32,
    time: f32,
}
