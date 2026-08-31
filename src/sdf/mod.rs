pub mod pipeline;
pub mod tables;

use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::Aabb,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::{BufferUsages, ShaderType},
        storage::ShaderBuffer,
    },
};

pub const VERTEX_STRIDE_F32: u32 = 10;
pub const MATERIAL_SLOTS: usize = 4;

#[derive(Resource, Clone, Copy, ExtractResource)]
pub struct SdfGrid {
    pub resolution: u32,
    pub half_extent: f32,
    pub max_vertices: u32,
    pub iso: f32,
}

impl Default for SdfGrid {
    fn default() -> Self {
        Self {
            resolution: 64,
            half_extent: 1.6,
            max_vertices: 300_000,
            iso: 0.0,
        }
    }
}

impl SdfGrid {
    pub fn cell_size(&self) -> f32 {
        2.0 * self.half_extent / (self.resolution - 1) as f32
    }

    pub fn origin(&self) -> Vec3 {
        Vec3::splat(-self.half_extent)
    }

    pub fn sample_count(&self) -> u64 {
        let res = self.resolution as u64;
        res * res * res
    }

    pub fn max_indices(&self) -> u32 {
        self.max_vertices
    }

    pub fn vertex_bytes(&self) -> u64 {
        self.max_vertices as u64 * VERTEX_STRIDE_F32 as u64 * 4
    }

    pub fn index_bytes(&self) -> u64 {
        self.max_indices() as u64 * 4
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct SdfBuffers {
    pub field: Handle<ShaderBuffer>,
    pub tri_table: Handle<ShaderBuffer>,
    pub counters: Handle<ShaderBuffer>,
    pub bake_vertices: Handle<ShaderBuffer>,
    pub bake_indices: Handle<ShaderBuffer>,
    pub mesh: Handle<Mesh>,
}

#[derive(Resource, Clone, Copy, Default, ExtractResource)]
pub struct SdfClock(pub f32);

#[derive(ShaderType, Clone, Copy, Default)]
pub struct SdfParams {
    pub origin: Vec3,
    pub cell_size: f32,
    pub grid_res: u32,
    pub max_vertices: u32,
    pub max_indices: u32,
    pub vertex_stride: u32,
    pub vertex_base: u32,
    pub index_base: u32,
    pub iso: f32,
    pub time: f32,
}

#[derive(ShaderType, Clone, Copy, Default, Debug)]
pub struct FieldSample {
    pub weights: Vec4,
    pub dist: f32,
}

#[derive(ShaderType, Clone, Copy, Default, Debug)]
pub struct SdfCounters {
    pub vertex_count: u32,
    pub index_count: u32,
    pub overflow: u32,
}

pub struct SdfPlugin;

impl Plugin for SdfPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SdfGrid>()
            .init_resource::<SdfClock>()
            .add_plugins((
                ExtractResourcePlugin::<SdfGrid>::default(),
                ExtractResourcePlugin::<SdfBuffers>::default(),
                ExtractResourcePlugin::<SdfClock>::default(),
                pipeline::SdfComputePlugin,
            ))
            .add_systems(PreStartup, create_sdf_buffers)
            .add_systems(Update, advance_clock);
    }
}

fn advance_clock(time: Res<Time>, mut clock: ResMut<SdfClock>) {
    clock.0 = time.elapsed_secs();
}

fn create_sdf_buffers(
    mut commands: Commands,
    grid: Res<SdfGrid>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let field = buffers.add(sized_buffer(
        "sdf_field",
        grid.sample_count() * FieldSample::min_size().get(),
        BufferUsages::STORAGE,
    ));

    let mut tri_table = ShaderBuffer::from(tables::build_tri_table());
    tri_table.buffer_description.label = Some("sdf_tri_table");
    tri_table.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let tri_table = buffers.add(tri_table);

    let counters = buffers.add(sized_buffer(
        "sdf_counters",
        SdfCounters::min_size().get(),
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    ));

    let bake_vertices = buffers.add(sized_buffer(
        "sdf_bake_vertices",
        grid.vertex_bytes(),
        BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    ));

    let bake_indices = buffers.add(sized_buffer(
        "sdf_bake_indices",
        grid.index_bytes(),
        BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    ));

    let mesh = meshes.add(placeholder_mesh(&grid));

    commands.insert_resource(SdfBuffers {
        field,
        tri_table,
        counters,
        bake_vertices,
        bake_indices,
        mesh,
    });
}

fn sized_buffer(label: &'static str, size: u64, usage: BufferUsages) -> ShaderBuffer {
    let mut buffer = ShaderBuffer::with_size(size as usize, RenderAssetUsages::RENDER_WORLD);
    buffer.buffer_description.label = Some(label);
    buffer.buffer_description.usage = usage;
    buffer
}

fn placeholder_mesh(grid: &SdfGrid) -> Mesh {
    let vertices = grid.max_vertices as usize;
    let indices = grid.max_indices() as usize;

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; vertices])
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; vertices])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[1.0f32, 0.0, 0.0, 0.0]; vertices],
    )
    .with_inserted_indices(Indices::U32(vec![0; indices]));

    mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
    mesh
}

pub fn domain_aabb(grid: &SdfGrid) -> Aabb {
    Aabb::from_min_max(
        Vec3::splat(-grid.half_extent),
        Vec3::splat(grid.half_extent),
    )
}
