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
    shader::Shader,
};

use super::{
    FieldSample, SdfBuffers, SdfClock, SdfCounters, SdfGrid, SdfParams, VERTEX_STRIDE_F32,
};

const SHADER_LIBRARIES: [&str; 3] = [
    "shaders/sdf_types.wesl",
    "shaders/sdf_lib.wesl",
    "shaders/sdf.wesl",
];

const SDF_EVAL_SHADER: &str = "shaders/sdf_eval.wesl";
const MARCHING_CUBES_SHADER: &str = "shaders/marching_cubes.wesl";

const FIELD_WORKGROUP: u32 = 4;
const CLEAR_WORKGROUP: u32 = 64;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SdfMeshGeneration;

pub struct SdfComputePlugin;

impl Plugin for SdfComputePlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
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

#[derive(Resource)]
pub struct SdfPipelines {
    #[expect(
        dead_code,
        reason = "strong handles keep imported shader libraries loaded"
    )]
    libraries: Vec<Handle<Shader>>,
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

    let libraries = SHADER_LIBRARIES
        .iter()
        .map(|path| asset_server.load::<Shader>(*path))
        .collect();

    let sdf_shader = asset_server.load(SDF_EVAL_SHADER);
    let meshing_shader = asset_server.load(MARCHING_CUBES_SHADER);

    let evaluate = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sdf_evaluate".into()),
        layout: vec![field_layout.clone()],
        shader: sdf_shader,
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
        shader: meshing_shader,
        entry_point: Some("march".into()),
        ..default()
    });

    commands.insert_resource(SdfPipelines {
        libraries,
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
) {
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
