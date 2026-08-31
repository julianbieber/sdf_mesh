#import sdf_mesh::sdf_types::{SdfParams, FieldSample}

const TRI_TABLE_ROW: u32 = 16u;
const MATERIAL_SLOTS: u32 = 4u;

struct Counters {
    vertex_count: atomic<u32>,
    index_count: atomic<u32>,
    overflow: atomic<u32>,
}

@group(0) @binding(0) var<uniform> params: SdfParams;
@group(0) @binding(1) var<storage, read> field: array<FieldSample>;
@group(0) @binding(2) var<storage, read> tri_table: array<i32>;
@group(0) @binding(3) var<storage, read_write> counters: Counters;
@group(0) @binding(4) var<storage, read_write> vertices: array<f32>;
@group(0) @binding(5) var<storage, read_write> indices: array<u32>;

fn corner_offset(corner: u32) -> vec3<i32> {
    let ring = corner & 3u;
    let z = corner >> 2u;
    let x = select(0u, 1u, ring == 1u || ring == 2u);
    let y = select(0u, 1u, ring == 2u || ring == 3u);
    return vec3<i32>(i32(x), i32(y), i32(z));
}

fn edge_corners(edge: u32) -> vec2<u32> {
    if edge < 4u {
        return vec2(edge, (edge + 1u) % 4u);
    }
    if edge < 8u {
        return vec2(edge, 4u + ((edge + 1u) % 4u));
    }
    return vec2(edge - 8u, edge - 4u);
}

fn field_sample(cell: vec3<i32>) -> FieldSample {
    let limit = i32(params.grid_res) - 1;
    let clamped = clamp(cell, vec3(0), vec3(limit));
    let index = u32(clamped.x)
        + u32(clamped.y) * params.grid_res
        + u32(clamped.z) * params.grid_res * params.grid_res;
    return field[index];
}

fn field_gradient(cell: vec3<i32>) -> vec3<f32> {
    return vec3(
        field_sample(cell + vec3(1, 0, 0)).dist - field_sample(cell - vec3(1, 0, 0)).dist,
        field_sample(cell + vec3(0, 1, 0)).dist - field_sample(cell - vec3(0, 1, 0)).dist,
        field_sample(cell + vec3(0, 0, 1)).dist - field_sample(cell - vec3(0, 0, 1)).dist,
    );
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let len = length(v);
    if len < 1e-9 {
        return vec3(0.0, 1.0, 0.0);
    }
    return v / len;
}

fn grid_position(cell: vec3<i32>) -> vec3<f32> {
    return params.origin + vec3<f32>(cell) * params.cell_size;
}

@compute @workgroup_size(64)
fn prepare(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x == 0u {
        atomicStore(&counters.vertex_count, 0u);
        atomicStore(&counters.index_count, 0u);
        atomicStore(&counters.overflow, 0u);
    }
    if gid.x < params.max_indices {
        indices[params.index_base + gid.x] = 0u;
    }
}

@compute @workgroup_size(4, 4, 4)
fn march(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cells = params.grid_res - 1u;
    if gid.x >= cells || gid.y >= cells || gid.z >= cells {
        return;
    }

    let base_cell = vec3<i32>(gid);

    var distances: array<f32, 8>;
    var weights: array<vec4<f32>, 8>;
    var mask = 0u;
    for (var corner = 0u; corner < 8u; corner = corner + 1u) {
        let sample = field_sample(base_cell + corner_offset(corner));
        distances[corner] = sample.dist;
        weights[corner] = sample.weights;
        if sample.dist < params.iso {
            mask = mask | (1u << corner);
        }
    }

    let row = mask * TRI_TABLE_ROW;
    let triangle_count = u32(max(tri_table[row], 0));
    if triangle_count == 0u {
        return;
    }

    let emitted = triangle_count * 3u;
    let base = atomicAdd(&counters.vertex_count, emitted);
    if base + emitted > params.max_vertices {
        atomicStore(&counters.overflow, 1u);
        return;
    }
    atomicAdd(&counters.index_count, emitted);

    for (var slot = 0u; slot < emitted; slot = slot + 1u) {
        let edge = u32(tri_table[row + 1u + slot]);
        let ends = edge_corners(edge);
        let a = ends.x;
        let b = ends.y;

        let da = distances[a];
        let db = distances[b];
        var t = 0.5;
        let delta = db - da;
        if abs(delta) > 1e-12 {
            t = clamp((params.iso - da) / delta, 0.0, 1.0);
        }

        let cell_a = base_cell + corner_offset(a);
        let cell_b = base_cell + corner_offset(b);

        let position = mix(grid_position(cell_a), grid_position(cell_b), t);
        let normal = safe_normalize(mix(field_gradient(cell_a), field_gradient(cell_b), t));
        let blended = mix(weights[a], weights[b], t);

        let vertex = base + slot;
        let offset = params.vertex_base + vertex * params.vertex_stride;

        vertices[offset + 0u] = position.x;
        vertices[offset + 1u] = position.y;
        vertices[offset + 2u] = position.z;
        vertices[offset + 3u] = normal.x;
        vertices[offset + 4u] = normal.y;
        vertices[offset + 5u] = normal.z;
        vertices[offset + 6u] = blended.x;
        vertices[offset + 7u] = blended.y;
        vertices[offset + 8u] = blended.z;
        vertices[offset + 9u] = blended.w;

        indices[params.index_base + vertex] = vertex;
    }
}
