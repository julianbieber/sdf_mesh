//! Compiles and dispatches the GPU passes that turn the opened scene into the
//! displayed mesh, and reports whether the scene compiles.

use std::sync::{Arc, Mutex};

use bevy::{
    core_pipeline::schedule::camera_driver,
    ecs::schedule::IntoScheduleConfigs,
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        mesh::allocator::{MeshAllocator, MeshAllocatorSettings},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{storage_buffer, storage_buffer_read_only, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderGraph, RenderGraphSystems, RenderQueue},
        storage::GpuShaderBuffer,
    },
    shader::{Shader, ShaderCacheError},
};

use super::{
    FieldSample, SdfBuffers, SdfClock, SdfCounters, SdfGrid, SdfParams, VERTEX_STRIDE_F32,
};
use crate::{
    project::MeshProject,
    shaders::{ENTRY_ASSET, MARCHING_CUBES_SHADER, entry_module_source},
};

/// Edge length, in invocations, of the cubic workgroups the field and the cells
/// are dispatched in.
pub const FIELD_WORKGROUP: u32 = 4;
const CLEAR_WORKGROUP: u32 = 64;

/// The set the mesh-generating passes run in; order after it to read this
/// frame's mesh.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SdfMeshGeneration;

/// Whether the opened scene currently compiles, as the HUD reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SceneState {
    #[default]
    Compiling,
    /// Composition found an import with no module behind it: one still
    /// loading, or one that names no module at all. Holds until the pipeline
    /// compiles or fails, or a shader asset changes.
    WaitingOnImport,
    Compiled,
    /// The compile error. The last good mesh stays on screen.
    Failed(String),
}

impl SceneState {
    fn next(self, pipeline: &CachedPipelineState) -> Self {
        match pipeline {
            CachedPipelineState::Ok(_) => Self::Compiled,
            CachedPipelineState::Err(
                ShaderCacheError::ShaderImportNotYetAvailable
                | ShaderCacheError::ShaderNotLoaded(_),
            ) => Self::WaitingOnImport,
            CachedPipelineState::Err(error) => Self::Failed(error.to_string()),
            CachedPipelineState::Queued | CachedPipelineState::Creating(_) => match self {
                Self::WaitingOnImport => Self::WaitingOnImport,
                _ => Self::Compiling,
            },
        }
    }
}

/// The [`SceneState`], written in the render world and read in the main world;
/// inserted into both.
#[derive(Resource, Clone, Default)]
pub struct SceneStatus(Arc<Mutex<SceneState>>);

impl SceneStatus {
    /// The current state, or [`SceneState::Compiling`] if the mutex is poisoned.
    pub fn get(&self) -> SceneState {
        self.0.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn advance(&self, pipeline: &CachedPipelineState) {
        if let Ok(mut current) = self.0.lock() {
            *current = std::mem::take(&mut *current).next(pipeline);
        }
    }
}

fn forget_import_wait_on_shader_change(
    mut events: MessageReader<AssetEvent<Shader>>,
    status: Res<SceneStatus>,
) {
    if events.read().count() == 0 {
        return;
    }
    if let Ok(mut current) = status.0.lock()
        && *current == SceneState::WaitingOnImport
    {
        *current = SceneState::Compiling;
    }
}

/// Queues and dispatches the field and meshing passes for the opened scene.
///
/// Requires [`MeshProject`] in the main world before the plugin is added.
pub struct SdfComputePlugin;

impl Plugin for SdfComputePlugin {
    fn build(&self, app: &mut App) {
        let project = app
            .world()
            .get_resource::<MeshProject>()
            .expect("MeshProject must be inserted before SdfComputePlugin")
            .clone();
        let status = SceneStatus::default();
        app.insert_resource(status.clone())
            .add_systems(PostUpdate, forget_import_wait_on_shader_change);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(project)
            .insert_resource(status)
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                RenderGraph,
                run_sdf_passes
                    .in_set(SdfMeshGeneration)
                    .in_set(RenderGraphSystems::Render)
                    .before(camera_driver),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .world_mut()
            .resource_mut::<MeshAllocatorSettings>()
            .extra_buffer_usages |= BufferUsages::STORAGE | BufferUsages::COPY_SRC;
    }
}

/// The queued pipelines and bind group layouts of the three passes, in the
/// render world once `RenderStartup` has run.
#[derive(Resource)]
pub struct SdfPipelines {
    #[expect(
        dead_code,
        reason = "strong handles keep the scene, the entry module and the meshing shader loaded"
    )]
    modules: Vec<Handle<Shader>>,
    field_layout: BindGroupLayoutDescriptor,
    mesh_layout: BindGroupLayoutDescriptor,
    evaluate: CachedComputePipelineId,
    prepare: CachedComputePipelineId,
    march: CachedComputePipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
    project: Res<MeshProject>,
) {
    let field_layout = BindGroupLayoutDescriptor::new(
        "sdf_field_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<SdfParams>(false),
                storage_buffer::<Vec<FieldSample>>(false),
            ),
        ),
    );

    let mesh_layout = BindGroupLayoutDescriptor::new(
        "sdf_mesh_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<SdfParams>(false),
                storage_buffer_read_only::<Vec<FieldSample>>(false),
                storage_buffer_read_only::<Vec<i32>>(false),
                storage_buffer::<SdfCounters>(false),
                storage_buffer::<Vec<f32>>(false),
                storage_buffer::<Vec<u32>>(false),
            ),
        ),
    );

    let scene = asset_server.load::<Shader>(project.scene_asset_path());
    let sdf_shader = asset_server.add(Shader::from_wesl(
        entry_module_source(&project.scene_module),
        ENTRY_ASSET,
    ));
    let meshing_shader = asset_server.load(MARCHING_CUBES_SHADER);

    let evaluate = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sdf_evaluate".into()),
        layout: vec![field_layout.clone()],
        shader: sdf_shader.clone(),
        entry_point: Some("evaluate".into()),
        ..default()
    });

    let prepare = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sdf_meshing_prepare".into()),
        layout: vec![mesh_layout.clone()],
        shader: meshing_shader.clone(),
        entry_point: Some("prepare".into()),
        ..default()
    });

    let march = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sdf_marching_cubes".into()),
        layout: vec![mesh_layout.clone()],
        shader: meshing_shader.clone(),
        entry_point: Some("march".into()),
        ..default()
    });

    commands.insert_resource(SdfPipelines {
        modules: vec![scene, sdf_shader, meshing_shader],
        field_layout,
        mesh_layout,
        evaluate,
        prepare,
        march,
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "render graph systems bind many resources"
)]
fn run_sdf_passes(
    mut render_context: RenderContext,
    mut params_buffer: Local<UniformBuffer<SdfParams>>,
    pipelines: Res<SdfPipelines>,
    pipeline_cache: Res<PipelineCache>,
    render_queue: Res<RenderQueue>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mesh_allocator: Res<MeshAllocator>,
    grid: Res<SdfGrid>,
    buffers: Res<SdfBuffers>,
    clock: Res<SdfClock>,
    status: Res<SceneStatus>,
) {
    status.advance(pipeline_cache.get_compute_pipeline_state(pipelines.evaluate));

    let (Some(evaluate), Some(prepare), Some(march)) = (
        pipeline_cache.get_compute_pipeline(pipelines.evaluate),
        pipeline_cache.get_compute_pipeline(pipelines.prepare),
        pipeline_cache.get_compute_pipeline(pipelines.march),
    ) else {
        return;
    };

    let (Some(field), Some(tri_table), Some(counters)) = (
        gpu_buffers.get(&buffers.field),
        gpu_buffers.get(&buffers.tri_table),
        gpu_buffers.get(&buffers.counters),
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

    let slab_vertices = vertex_slice.range.end - vertex_slice.range.start;
    let slab_indices = index_slice.range.end - index_slice.range.start;

    let params = SdfParams {
        origin: grid.origin(),
        cell_size: grid.cell_size(),
        grid_res: grid.resolution,
        max_vertices: grid.max_vertices.min(slab_vertices),
        max_indices: grid.max_indices().min(slab_indices),
        vertex_stride: VERTEX_STRIDE_F32,
        vertex_base: vertex_slice.range.start * VERTEX_STRIDE_F32,
        index_base: index_slice.range.start,
        iso: grid.iso,
        time: clock.0,
    };

    let device = render_context.render_device().clone();
    params_buffer.set(params);
    params_buffer.write_buffer(&device, &render_queue);

    let field_bind_group = device.create_bind_group(
        Some("sdf_field_bind_group"),
        &pipeline_cache.get_bind_group_layout(&pipelines.field_layout),
        &BindGroupEntries::sequential((&*params_buffer, field.buffer.as_entire_buffer_binding())),
    );

    let mesh_bind_group = device.create_bind_group(
        Some("sdf_mesh_bind_group"),
        &pipeline_cache.get_bind_group_layout(&pipelines.mesh_layout),
        &BindGroupEntries::sequential((
            &*params_buffer,
            field.buffer.as_entire_buffer_binding(),
            tri_table.buffer.as_entire_buffer_binding(),
            counters.buffer.as_entire_buffer_binding(),
            vertex_slice.buffer.as_entire_buffer_binding(),
            index_slice.buffer.as_entire_buffer_binding(),
        )),
    );

    let mut pass = render_context
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("sdf_generate_mesh"),
            ..default()
        });

    let field_groups = grid.resolution.div_ceil(FIELD_WORKGROUP);
    pass.set_pipeline(evaluate);
    pass.set_bind_group(0, &field_bind_group, &[]);
    pass.dispatch_workgroups(field_groups, field_groups, field_groups);

    pass.set_bind_group(0, &mesh_bind_group, &[]);
    pass.set_pipeline(prepare);
    pass.dispatch_workgroups(params.max_indices.div_ceil(CLEAR_WORKGROUP), 1, 1);

    let cell_groups = (grid.resolution - 1).div_ceil(FIELD_WORKGROUP);
    pass.set_pipeline(march);
    pass.dispatch_workgroups(cell_groups, cell_groups, cell_groups);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pins that an import wait holds through bevy's queued/creating retry frames instead of flickering to compiling.
    #[test]
    fn an_import_wait_survives_the_retry_cycle_until_the_pipeline_settles() {
        let waiting = SceneState::Compiling.next(&CachedPipelineState::Err(
            ShaderCacheError::ShaderImportNotYetAvailable,
        ));
        assert_eq!(waiting, SceneState::WaitingOnImport);
        let waiting = waiting.next(&CachedPipelineState::Queued);
        assert_eq!(waiting, SceneState::WaitingOnImport);
        assert_eq!(
            SceneState::Compiled.next(&CachedPipelineState::Queued),
            SceneState::Compiling
        );
    }
}
