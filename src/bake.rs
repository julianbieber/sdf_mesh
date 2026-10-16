//! Bakes the live generated SDF mesh to a binary glTF file on disk when the
//! bake key is pressed.

mod gltf;

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{
        RenderApp,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        gpu_readback::{Readback, ReadbackComplete},
        mesh::allocator::MeshAllocator,
        render_asset::RenderAssets,
        renderer::{RenderContext, RenderGraph, RenderGraphSystems},
        storage::GpuShaderBuffer,
    },
};

use crate::sdf::{
    SdfBuffers, SdfCounters, SdfGrid, VERTEX_STRIDE_F32, pipeline::SdfMeshGeneration,
};

/// Pressing this key while no bake is in flight starts one.
pub const BAKE_KEY: KeyCode = KeyCode::KeyB;
const OUTPUT_PATH: &str = "bake/sdf_surface.glb";

/// Raised for exactly the one frame in which a bake starts; in that frame the
/// render world copies the generated mesh into the bake readback buffers.
#[derive(Resource, Clone, Copy, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct BakeRequest(pub bool);

/// The bake in progress. `status` is the human-readable outcome of the latest
/// bake: `None` before the first one, `"baking"` while one is in flight.
#[derive(Resource, Default)]
pub struct BakeState {
    in_flight: bool,
    vertices: Option<Vec<f32>>,
    indices: Option<Vec<u32>>,
    counters: Option<SdfCounters>,
    pub status: Option<String>,
}

impl BakeState {
    fn clear_readbacks(&mut self) {
        self.vertices = None;
        self.indices = None;
        self.counters = None;
    }
}

/// Bakes on [`BAKE_KEY`] to `bake/sdf_surface.glb`. Requires the SDF plugin:
/// it reads `SdfBuffers` and `SdfGrid`, and orders its GPU copy after
/// `SdfMeshGeneration` in the render graph.
pub struct BakePlugin;

impl Plugin for BakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BakeRequest>()
            .init_resource::<BakeState>()
            .add_plugins(ExtractResourcePlugin::<BakeRequest>::default())
            .add_systems(Update, (clear_request, start_bake, finish_bake).chain());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.add_systems(
            RenderGraph,
            copy_bake_buffers
                .in_set(RenderGraphSystems::Render)
                .after(SdfMeshGeneration),
        );
    }
}

fn clear_request(mut request: ResMut<BakeRequest>) {
    if request.0 {
        request.0 = false;
    }
}

fn start_bake(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    buffers: Option<Res<SdfBuffers>>,
    mut request: ResMut<BakeRequest>,
    mut state: ResMut<BakeState>,
) {
    let Some(buffers) = buffers else {
        return;
    };
    if state.in_flight || !keys.just_pressed(BAKE_KEY) {
        return;
    }

    state.clear_readbacks();
    state.in_flight = true;
    state.status = Some("baking".to_owned());
    request.0 = true;

    commands
        .spawn(Readback::buffer(buffers.counters.clone()))
        .observe(
            |event: On<ReadbackComplete>, mut commands: Commands, mut state: ResMut<BakeState>| {
                state.counters = Some(event.to_shader_type());
                commands.entity(event.entity).despawn();
            },
        );

    commands
        .spawn(Readback::buffer(buffers.bake_vertices.clone()))
        .observe(
            |event: On<ReadbackComplete>, mut commands: Commands, mut state: ResMut<BakeState>| {
                state.vertices = Some(event.to_shader_type());
                commands.entity(event.entity).despawn();
            },
        );

    commands
        .spawn(Readback::buffer(buffers.bake_indices.clone()))
        .observe(
            |event: On<ReadbackComplete>, mut commands: Commands, mut state: ResMut<BakeState>| {
                state.indices = Some(event.to_shader_type());
                commands.entity(event.entity).despawn();
            },
        );
}

fn finish_bake(mut state: ResMut<BakeState>, grid: Res<SdfGrid>) {
    if !state.in_flight
        || state.vertices.is_none()
        || state.indices.is_none()
        || state.counters.is_none()
    {
        return;
    }

    let vertices = state.vertices.take().expect("checked above");
    let indices = state.indices.take().expect("checked above");
    let counters = state.counters.take().expect("checked above");
    state.in_flight = false;

    if counters.overflow != 0 {
        warn!(
            "marching cubes overflowed the {} vertex budget; the bake is truncated",
            grid.max_vertices
        );
    }

    let mesh = assemble_mesh(&vertices, &indices, &counters, &grid);
    let triangles = mesh.indices().map_or(0, |i| i.len() / 3);

    let bytes = match gltf::mesh_to_glb(&mesh) {
        Ok(bytes) => bytes,
        Err(error @ (gltf::GlbError::NoTriangles | gltf::GlbError::NonFinitePosition)) => {
            warn!("bake wrote nothing: {error}");
            state.status = Some(format!("nothing written: {error}"));
            return;
        }
        Err(error) => {
            error!("bake failed: {error}");
            state.status = Some(format!("bake failed: {error}"));
            return;
        }
    };

    state.status = Some(match write_bytes(Path::new(OUTPUT_PATH), &bytes) {
        Ok(path) => format!("{triangles} tris -> {}", path.display()),
        Err(error) => {
            error!("bake failed: {error}");
            format!("bake failed: {error}")
        }
    });
}

fn assemble_mesh(
    vertices: &[f32],
    indices: &[u32],
    counters: &SdfCounters,
    grid: &SdfGrid,
) -> Mesh {
    let stride = VERTEX_STRIDE_F32 as usize;
    let available = vertices.len() / stride;
    let vertex_count = (counters.vertex_count.min(grid.max_vertices) as usize).min(available);
    let index_count = (counters.index_count.min(grid.max_indices()) as usize).min(indices.len());

    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut weights = Vec::with_capacity(vertex_count);
    for vertex in 0..vertex_count {
        let base = vertex * stride;
        positions.push([vertices[base], vertices[base + 1], vertices[base + 2]]);
        normals.push([vertices[base + 3], vertices[base + 4], vertices[base + 5]]);
        weights.push([
            vertices[base + 6],
            vertices[base + 7],
            vertices[base + 8],
            vertices[base + 9],
        ]);
    }

    let kept: Vec<u32> = indices[..index_count]
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|t| {
            t.iter().all(|i| (*i as usize) < vertex_count)
                && t[0] != t[1]
                && t[1] != t[2]
                && t[0] != t[2]
        })
        .flatten()
        .copied()
        .collect();

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, weights)
    .with_inserted_indices(Indices::U32(kept))
}

fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(path.to_path_buf())
}

fn copy_bake_buffers(
    mut render_context: RenderContext,
    request: Res<BakeRequest>,
    buffers: Res<SdfBuffers>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mesh_allocator: Res<MeshAllocator>,
) {
    if !request.0 {
        return;
    }

    let (Some(bake_vertices), Some(bake_indices)) = (
        gpu_buffers.get(&buffers.bake_vertices),
        gpu_buffers.get(&buffers.bake_indices),
    ) else {
        return;
    };

    let mesh_id = buffers.mesh.id();
    let (Some(vertex_slice), Some(index_slice)) = (
        mesh_allocator.mesh_vertex_slice(&mesh_id),
        mesh_allocator.mesh_index_slice(&mesh_id),
    ) else {
        return;
    };

    let vertex_element = VERTEX_STRIDE_F32 as u64 * 4;
    let vertex_offset = vertex_slice.range.start as u64 * vertex_element;
    let vertex_bytes = ((vertex_slice.range.end - vertex_slice.range.start) as u64
        * vertex_element)
        .min(bake_vertices.buffer.size());

    let index_offset = index_slice.range.start as u64 * 4;
    let index_bytes = ((index_slice.range.end - index_slice.range.start) as u64 * 4)
        .min(bake_indices.buffer.size());

    let encoder = render_context.command_encoder();
    encoder.copy_buffer_to_buffer(
        vertex_slice.buffer,
        vertex_offset,
        &bake_vertices.buffer,
        0,
        vertex_bytes,
    );
    encoder.copy_buffer_to_buffer(
        index_slice.buffer,
        index_offset,
        &bake_indices.buffer,
        0,
        index_bytes,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(position: [f32; 3], weights: [f32; 4]) -> [f32; 10] {
        let [x, y, z] = position;
        let [a, b, c, d] = weights;
        [x, y, z, 0.0, 1.0, 0.0, a, b, c, d]
    }

    // Pins that assembly carries each vertex's material weights and drops degenerate or out-of-range triangles.
    #[test]
    fn assembly_keeps_weights_and_drops_bad_triangles() {
        let vertices: Vec<f32> = [
            vertex([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
            vertex([1.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]),
            vertex([0.0, 1.0, 0.0], [0.0, 0.0, 0.5, 0.5]),
        ]
        .concat();
        let indices = vec![0, 1, 2, 0, 0, 1, 0, 1, 7];
        let counters = SdfCounters {
            vertex_count: 3,
            index_count: indices.len() as u32,
            overflow: 0,
        };

        let mesh = assemble_mesh(&vertices, &indices, &counters, &SdfGrid::default());

        let Some(Indices::U32(kept)) = mesh.indices() else {
            panic!("no u32 indices");
        };
        assert_eq!(kept, &vec![0, 1, 2]);
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(weights)) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("no float4 weights");
        };
        assert_eq!(
            weights,
            &vec![
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.5, 0.5]
            ]
        );
    }
}
