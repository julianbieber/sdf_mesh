//! Samples the opened scene's `sdf_joint` on the GPU at every keyframe of a
//! rig, into [`SdfBuffers::joint_samples`], in the frame a
//! [`JointSamplingRequest`] carries the rig.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use bevy::{
    asset::uuid_handle,
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{storage_buffer, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderGraph, RenderGraphSystems, RenderQueue},
        storage::GpuShaderBuffer,
    },
    shader::Shader,
};

use super::{JointSample, SdfBuffers};
use crate::{
    project::MeshProject,
    rig::{Rig, defines_sdf_joint},
    shaders::{JOINT_WORKGROUP, JOINTS_ASSET, joints_module_source},
};

const JOINTS_SHADER: Handle<Shader> = uuid_handle!("060889d5-b2ed-4ffc-b264-17b2fecb2593");

/// The rig whose joints are sampled in this frame; `None` in every frame
/// that samples nothing.
#[derive(Resource, Clone, Copy, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct JointSamplingRequest(pub Option<Rig>);

/// Whether the joint sampler compiled for the opened scene as of the last
/// rendered frame; a request made while it has not samples nothing. Inserted
/// into both worlds.
#[derive(Resource, Clone, Default)]
pub struct JointSamplerStatus(Arc<AtomicBool>);

impl JointSamplerStatus {
    pub fn is_ready(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Keeps the joint sampler in step with the opened scene and dispatches it
/// on request. Requires [`MeshProject`] in the main world before the plugin
/// is added, and the SDF buffers.
pub struct JointSamplerPlugin;

impl Plugin for JointSamplerPlugin {
    fn build(&self, app: &mut App) {
        let project = app
            .world()
            .get_resource::<MeshProject>()
            .expect("MeshProject must be inserted before JointSamplerPlugin")
            .clone();
        let status = JointSamplerStatus::default();
        app.init_resource::<JointSamplingRequest>()
            .insert_resource(status.clone())
            .add_plugins(ExtractResourcePlugin::<JointSamplingRequest>::default())
            .add_systems(PreUpdate, refresh_joints_module);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(project)
            .insert_resource(status)
            .add_systems(RenderStartup, init_joint_sampler)
            .add_systems(
                RenderGraph,
                sample_joints.in_set(RenderGraphSystems::Render),
            );
    }
}

fn refresh_joints_module(
    project: Res<MeshProject>,
    mut events: MessageReader<AssetEvent<Shader>>,
    mut shaders: ResMut<Assets<Shader>>,
    mut written: Local<Option<bool>>,
) {
    let changed = events
        .read()
        .any(|event| !event.is_modified(&JOINTS_SHADER) && !event.is_added(&JOINTS_SHADER));
    if written.is_some() && !changed {
        return;
    }
    let calls =
        std::fs::read_to_string(&project.scene_file).is_ok_and(|source| defines_sdf_joint(&source));
    if *written == Some(calls) {
        return;
    }
    *written = Some(calls);
    if let Err(error) = shaders.insert(
        &JOINTS_SHADER,
        Shader::from_wesl(
            joints_module_source(&project.scene_module, calls),
            JOINTS_ASSET,
        ),
    ) {
        error!("cannot register the joint sampler: {error}");
    }
}

#[derive(ShaderType, Clone, Copy, Default)]
struct JointSampling {
    joint_count: u32,
    frames: u32,
    sample_count: u32,
    seconds: f32,
}

#[derive(Resource)]
struct JointSampler {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn init_joint_sampler(mut commands: Commands, pipeline_cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new(
        "sdf_joint_sampler_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<JointSampling>(false),
                storage_buffer::<Vec<JointSample>>(false),
            ),
        ),
    );
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sdf_joint_sampler".into()),
        layout: vec![layout.clone()],
        shader: JOINTS_SHADER,
        entry_point: Some("sample_joints".into()),
        ..default()
    });
    commands.insert_resource(JointSampler { layout, pipeline });
}

#[expect(
    clippy::too_many_arguments,
    reason = "render graph systems bind many resources"
)]
fn sample_joints(
    mut render_context: RenderContext,
    mut sampling_buffer: Local<UniformBuffer<JointSampling>>,
    request: Res<JointSamplingRequest>,
    sampler: Res<JointSampler>,
    pipeline_cache: Res<PipelineCache>,
    render_queue: Res<RenderQueue>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    buffers: Res<SdfBuffers>,
    status: Res<JointSamplerStatus>,
) {
    let pipeline = pipeline_cache.get_compute_pipeline(sampler.pipeline);
    status.0.store(pipeline.is_some(), Ordering::Relaxed);
    let (Some(rig), Some(pipeline), Some(samples)) =
        (request.0, pipeline, gpu_buffers.get(&buffers.joint_samples))
    else {
        return;
    };

    let device = render_context.render_device().clone();
    sampling_buffer.set(JointSampling {
        joint_count: rig.joints,
        frames: rig.frames,
        sample_count: rig.sample_count(),
        seconds: rig.seconds,
    });
    sampling_buffer.write_buffer(&device, &render_queue);

    let bind_group = device.create_bind_group(
        Some("sdf_joint_sampler_bind_group"),
        &pipeline_cache.get_bind_group_layout(&sampler.layout),
        &BindGroupEntries::sequential((
            &*sampling_buffer,
            samples.buffer.as_entire_buffer_binding(),
        )),
    );

    let encoder = render_context.command_encoder();
    encoder.clear_buffer(&samples.buffer, 0, None);
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("sdf_sample_joints"),
        ..default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(rig.sample_count().div_ceil(JOINT_WORKGROUP), 1, 1);
}
