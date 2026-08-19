use std::{
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
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

pub const BAKE_KEY: KeyCode = KeyCode::KeyB;
const OUTPUT_PATH: &str = "bake/sdf_surface.obj";

#[derive(Resource, Clone, Copy, Default, ExtractResource)]
pub struct BakeRequest(pub bool);

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

    state.status = Some(match write_obj(Path::new(OUTPUT_PATH), &mesh) {
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
    for vertex in 0..vertex_count {
        let base = vertex * stride;
        positions.push([vertices[base], vertices[base + 1], vertices[base + 2]]);
        normals.push([vertices[base + 3], vertices[base + 4], vertices[base + 5]]);
    }

    let kept: Vec<u32> = indices[..index_count - index_count % 3]
        .chunks_exact(3)
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
    .with_inserted_indices(Indices::U32(kept))
}

fn write_obj(path: &Path, mesh: &Mesh) -> io::Result<PathBuf> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err(io::Error::other("mesh has no float3 positions"));
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        return Err(io::Error::other("mesh has no float3 normals"));
    };
    let Some(Indices::U32(indices)) = mesh.indices() else {
        return Err(io::Error::other("mesh has no u32 indices"));
    };

    let mut out = String::with_capacity(positions.len() * 48 + indices.len() * 12);
    for p in positions {
        let _ = writeln!(out, "v {} {} {}", p[0], p[1], p[2]);
    }
    for n in normals {
        let _ = writeln!(out, "vn {} {} {}", n[0], n[1], n[2]);
    }
    for triangle in indices.chunks_exact(3) {
        let (a, b, c) = (triangle[0] + 1, triangle[1] + 1, triangle[2] + 1);
        let _ = writeln!(out, "f {a}//{a} {b}//{b} {c}//{c}");
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, out)?;
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
        .min(bake_vertices.buffer_descriptor.size);

    let index_offset = index_slice.range.start as u64 * 4;
    let index_bytes = ((index_slice.range.end - index_slice.range.start) as u64 * 4)
        .min(bake_indices.buffer_descriptor.size);

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
