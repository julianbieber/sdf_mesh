use bevy::{
    asset::RenderAssetUsages,
    image::{
        ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor, TextureFormatPixelInfo,
    },
    pbr::{ExtendedMaterial, MaterialExtension},
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, ShaderType, TextureDataOrder, TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};

use crate::sdf::MATERIAL_SLOTS;

const TRIPLANAR_SHADER: &str = "shaders/triplanar.wgsl";
const LAYER_SIZE: u32 = 256;
const MIP_LEVELS: u32 = 9;

pub const BLEND_SLOTS: u32 = 2;

pub type SdfSurfaceMaterial = ExtendedMaterial<StandardMaterial, TriplanarExtension>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PbrChannel {
    BaseColor,
    Normal,
    Orm,
    Emissive,
}

#[derive(Clone, Debug)]
pub struct MaterialSource {
    pub name: String,
    pub dir: String,
}

#[derive(Clone, Debug)]
pub struct ChannelSpec {
    pub channel: PbrChannel,
    pub srgb: bool,
    pub packed_from: Vec<String>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
pub struct MaterialParams {
    #[shader(align(16))]
    pub uv_scale: f32,
    pub normal_strength: f32,
    pub roughness_scale: f32,
    pub metallic_scale: f32,
    #[shader(size(16))]
    pub emissive_strength: f32,
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            uv_scale: 1.0,
            normal_strength: 1.0,
            roughness_scale: 1.0,
            metallic_scale: 1.0,
            emissive_strength: 0.0,
        }
    }
}

#[derive(ShaderType, Reflect, Debug, Clone)]
pub struct MaterialParamsTable {
    pub entries: [MaterialParams; MATERIAL_SLOTS],
}

impl Default for MaterialParamsTable {
    fn default() -> Self {
        Self {
            entries: core::array::from_fn(|_| MaterialParams::default()),
        }
    }
}

#[derive(ShaderType, Reflect, Debug, Clone)]
pub struct TriplanarSettings {
    pub scale: f32,
    pub blend_sharpness: f32,
}

impl Default for TriplanarSettings {
    fn default() -> Self {
        Self {
            scale: 0.6,
            blend_sharpness: 4.0,
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct TriplanarExtension {
    #[uniform(100)]
    pub settings: TriplanarSettings,
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub layers: Option<Handle<Image>>,
    #[texture(103, dimension = "2d_array")]
    pub normal_layers: Option<Handle<Image>>,
    #[texture(104, dimension = "2d_array")]
    pub orm_layers: Option<Handle<Image>>,
    #[texture(105, dimension = "2d_array")]
    pub emissive_layers: Option<Handle<Image>>,
    #[uniform(106)]
    pub params: MaterialParamsTable,
}

impl MaterialExtension for TriplanarExtension {
    fn fragment_shader() -> ShaderRef {
        TRIPLANAR_SHADER.into()
    }
}

#[derive(Resource, Clone, Debug)]
pub struct MaterialManifest {
    pub materials: Vec<MaterialSource>,
    pub channels: Vec<ChannelSpec>,
    pub params: Vec<MaterialParams>,
    pub layer_size: u32,
    pub mip_levels: u32,
    pub blend_slots: u32,
}

impl Default for MaterialManifest {
    fn default() -> Self {
        Self {
            materials: ["stone", "sand", "moss", "lava"]
                .into_iter()
                .map(|name| MaterialSource {
                    name: name.to_owned(),
                    dir: format!("materials/{name}"),
                })
                .collect(),
            channels: vec![
                ChannelSpec {
                    channel: PbrChannel::BaseColor,
                    srgb: true,
                    packed_from: vec!["base_color.png".to_owned()],
                },
                ChannelSpec {
                    channel: PbrChannel::Normal,
                    srgb: false,
                    packed_from: vec!["normal.png".to_owned()],
                },
                ChannelSpec {
                    channel: PbrChannel::Orm,
                    srgb: false,
                    packed_from: vec!["orm.png".to_owned()],
                },
                ChannelSpec {
                    channel: PbrChannel::Emissive,
                    srgb: true,
                    packed_from: vec!["emissive.png".to_owned()],
                },
            ],
            params: vec![
                MaterialParams {
                    uv_scale: 1.0,
                    normal_strength: 1.0,
                    roughness_scale: 0.95,
                    metallic_scale: 0.0,
                    emissive_strength: 0.0,
                },
                MaterialParams {
                    uv_scale: 1.6,
                    normal_strength: 0.7,
                    roughness_scale: 1.0,
                    metallic_scale: 0.0,
                    emissive_strength: 0.0,
                },
                MaterialParams {
                    uv_scale: 1.2,
                    normal_strength: 1.2,
                    roughness_scale: 1.0,
                    metallic_scale: 0.0,
                    emissive_strength: 0.0,
                },
                MaterialParams {
                    uv_scale: 0.8,
                    normal_strength: 1.0,
                    roughness_scale: 0.6,
                    metallic_scale: 0.2,
                    emissive_strength: 6.0,
                },
            ],
            layer_size: LAYER_SIZE,
            mip_levels: MIP_LEVELS,
            blend_slots: BLEND_SLOTS,
        }
    }
}

impl MaterialManifest {
    fn params_table(&self) -> MaterialParamsTable {
        MaterialParamsTable {
            entries: core::array::from_fn(|slot| {
                self.params.get(slot).cloned().unwrap_or_default()
            }),
        }
    }
}

#[derive(Resource, Default)]
pub struct MaterialBakeSources {
    pub handles: HashMap<PbrChannel, Vec<Handle<Image>>>,
}

impl MaterialBakeSources {
    fn all(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.handles.values().flatten()
    }

    fn holds(&self, id: AssetId<Image>) -> bool {
        self.all().any(|handle| handle.id() == id)
    }
}

#[derive(Resource, Default)]
pub struct MaterialLibrary {
    pub arrays: HashMap<PbrChannel, Handle<Image>>,
    pub ready: bool,
}

pub struct TriplanarMaterialPlugin;

impl Plugin for TriplanarMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SdfSurfaceMaterial>::default())
            .init_resource::<MaterialLibrary>()
            .add_systems(Startup, load_material_manifest)
            .add_systems(
                Update,
                (
                    rebake_on_source_change.run_if(resource_exists::<MaterialBakeSources>),
                    bake_material_arrays.run_if(sources_are_packable),
                    apply_material_library.run_if(library_became_ready),
                )
                    .chain(),
            );
    }
}

fn load_material_manifest(mut commands: Commands, asset_server: Res<AssetServer>) {
    let manifest = MaterialManifest::default();
    debug_assert_eq!(
        manifest.materials.len(),
        MATERIAL_SLOTS,
        "manifest material count must match the sdf scene's material slots"
    );
    debug_assert!(
        manifest.blend_slots <= MATERIAL_SLOTS as u32,
        "blend slots must not exceed the material slot count"
    );
    debug_assert_eq!(
        manifest.blend_slots, BLEND_SLOTS,
        "manifest blend slots must match BLEND_SLOTS in {TRIPLANAR_SHADER}"
    );

    let mut handles: HashMap<PbrChannel, Vec<Handle<Image>>> = HashMap::default();

    for spec in &manifest.channels {
        let srgb = spec.srgb;
        let mut requested = Vec::with_capacity(manifest.materials.len() * spec.packed_from.len());

        for source in &manifest.materials {
            for file in &spec.packed_from {
                let path = format!("{}/{}", source.dir, file);
                requested.push(
                    asset_server
                        .load_builder()
                        .with_settings(move |settings: &mut ImageLoaderSettings| {
                            settings.is_srgb = srgb;
                            settings.asset_usage = RenderAssetUsages::MAIN_WORLD;
                        })
                        .load(path),
                );
            }
        }

        handles.insert(spec.channel, requested);
    }

    commands.insert_resource(MaterialBakeSources { handles });
    commands.insert_resource(manifest);
}

fn rebake_on_source_change(
    mut events: MessageReader<AssetEvent<Image>>,
    sources: Res<MaterialBakeSources>,
    mut library: ResMut<MaterialLibrary>,
) {
    let changed = events.read().any(|event| match event {
        AssetEvent::Modified { id } => sources.holds(*id),
        _ => false,
    });

    if changed {
        library.ready = false;
    }
}

fn sources_are_packable(
    library: Res<MaterialLibrary>,
    sources: Option<Res<MaterialBakeSources>>,
    images: Res<Assets<Image>>,
) -> bool {
    if library.ready {
        return false;
    }

    sources.is_some_and(|sources| {
        sources
            .all()
            .all(|handle| images.get(handle).is_some_and(|image| image.data.is_some()))
    })
}

fn library_became_ready(library: Res<MaterialLibrary>) -> bool {
    library.is_changed() && library.ready
}

fn bake_material_arrays(
    manifest: Res<MaterialManifest>,
    sources: Res<MaterialBakeSources>,
    mut images: ResMut<Assets<Image>>,
    mut library: ResMut<MaterialLibrary>,
) {
    let size = manifest.layer_size;
    let mips = manifest.mip_levels.min(full_mip_count(size));
    let layers = manifest.materials.len() as u32;
    let mut arrays = HashMap::default();

    for spec in &manifest.channels {
        let Some(handles) = sources.handles.get(&spec.channel) else {
            warn!("no source handles recorded for {:?}", spec.channel);
            return;
        };

        let files = spec.packed_from.len();
        let mut packed = Vec::new();

        for material in 0..manifest.materials.len() {
            let slice = &handles[material * files..(material + 1) * files];
            let Some(layer) = pack_layer(&images, slice, size) else {
                warn!(
                    "unusable source image while packing {:?} for material {} (layer {material})",
                    spec.channel, manifest.materials[material].name
                );
                return;
            };
            append_mip_chain(&mut packed, layer, size, mips, spec);
        }

        let image = array_image(packed, size, layers, mips, spec.srgb);
        arrays.insert(spec.channel, images.add(image));
    }

    library.arrays = arrays;
    library.ready = true;

    info!("baked {} material layers, {mips} mips at {size}px", layers);
}

fn apply_material_library(
    library: Res<MaterialLibrary>,
    manifest: Res<MaterialManifest>,
    surfaces: Query<&MeshMaterial3d<SdfSurfaceMaterial>>,
    mut materials: ResMut<Assets<SdfSurfaceMaterial>>,
) {
    let params = manifest.params_table();

    for surface in &surfaces {
        let Some(mut material) = materials.get_mut(&surface.0) else {
            continue;
        };

        material.extension.layers = library.arrays.get(&PbrChannel::BaseColor).cloned();
        material.extension.normal_layers = library.arrays.get(&PbrChannel::Normal).cloned();
        material.extension.orm_layers = library.arrays.get(&PbrChannel::Orm).cloned();
        material.extension.emissive_layers = library.arrays.get(&PbrChannel::Emissive).cloned();
        material.extension.params = params.clone();
    }
}

fn full_mip_count(size: u32) -> u32 {
    size.max(1).ilog2() + 1
}

fn pack_layer(images: &Assets<Image>, handles: &[Handle<Image>], size: u32) -> Option<Vec<f32>> {
    let mut sources = Vec::with_capacity(handles.len());
    for handle in handles {
        sources.push(resample(images.get(handle)?, size)?);
    }

    let (first, rest) = sources.split_first()?;
    if rest.is_empty() {
        return Some(first.clone());
    }

    let texels = (size * size) as usize;
    let mut packed = vec![0.0f32; texels * 4];
    for (channel, source) in sources.iter().enumerate().take(3) {
        for texel in 0..texels {
            packed[texel * 4 + channel] = source[texel * 4];
        }
    }
    for texel in 0..texels {
        packed[texel * 4 + 3] = 1.0;
    }

    Some(packed)
}

fn resample(image: &Image, size: u32) -> Option<Vec<f32>> {
    let data = image.data.as_ref()?;
    if image.texture_descriptor.format.pixel_size().ok()? != 4 {
        warn!(
            "material source is {:?}, expected an 8-bit rgba format",
            image.texture_descriptor.format
        );
        return None;
    }

    let srgb = image.texture_descriptor.format.is_srgb();
    let src_width = image.width().max(1) as usize;
    let src_height = image.height().max(1) as usize;
    let dst = size as usize;
    let mut out = vec![0.0f32; dst * dst * 4];

    for y in 0..dst {
        let y0 = y * src_height / dst;
        let y1 = (((y + 1) * src_height).div_ceil(dst))
            .max(y0 + 1)
            .min(src_height);
        for x in 0..dst {
            let x0 = x * src_width / dst;
            let x1 = (((x + 1) * src_width).div_ceil(dst))
                .max(x0 + 1)
                .min(src_width);

            let mut sum = [0.0f32; 4];
            let mut count = 0.0f32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let texel = (sy * src_width + sx) * 4;
                    for channel in 0..4 {
                        sum[channel] += decode(data[texel + channel], srgb && channel < 3);
                    }
                    count += 1.0;
                }
            }

            let texel = (y * dst + x) * 4;
            for channel in 0..4 {
                out[texel + channel] = sum[channel] / count;
            }
        }
    }

    Some(out)
}

fn append_mip_chain(
    packed: &mut Vec<u8>,
    layer: Vec<f32>,
    size: u32,
    mips: u32,
    spec: &ChannelSpec,
) {
    let mut level = layer;
    let mut width = size as usize;

    for mip in 0..mips as usize {
        if mip > 0 {
            level = downsample(&level, width);
            width /= 2;
            if spec.channel == PbrChannel::Normal {
                renormalise(&mut level);
            }
        }
        packed.extend(encode(&level, spec.srgb));
    }
}

fn downsample(level: &[f32], width: usize) -> Vec<f32> {
    let half = (width / 2).max(1);
    let mut out = vec![0.0f32; half * half * 4];

    for y in 0..half {
        for x in 0..half {
            let texel = (y * half + x) * 4;
            for channel in 0..4 {
                let a = level[((y * 2) * width + x * 2) * 4 + channel];
                let b = level[((y * 2) * width + x * 2 + 1) * 4 + channel];
                let c = level[((y * 2 + 1) * width + x * 2) * 4 + channel];
                let d = level[((y * 2 + 1) * width + x * 2 + 1) * 4 + channel];
                out[texel + channel] = (a + b + c + d) * 0.25;
            }
        }
    }

    out
}

fn renormalise(level: &mut [f32]) {
    for texel in level.chunks_exact_mut(4) {
        let normal = Vec3::new(
            texel[0] * 2.0 - 1.0,
            texel[1] * 2.0 - 1.0,
            texel[2] * 2.0 - 1.0,
        )
        .normalize_or(Vec3::Z);
        texel[0] = normal.x * 0.5 + 0.5;
        texel[1] = normal.y * 0.5 + 0.5;
        texel[2] = normal.z * 0.5 + 0.5;
    }
}

fn array_image(data: Vec<u8>, size: u32, layers: u32, mips: u32, srgb: bool) -> Image {
    let format = if srgb {
        TextureFormat::Rgba8UnormSrgb
    } else {
        TextureFormat::Rgba8Unorm
    };

    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );

    image.texture_descriptor.mip_level_count = mips;
    image.data_order = TextureDataOrder::LayerMajor;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..ImageSamplerDescriptor::linear()
    });

    image
}

fn encode(level: &[f32], srgb: bool) -> Vec<u8> {
    level
        .chunks_exact(4)
        .flat_map(|texel| {
            [
                quantise(texel[0], srgb),
                quantise(texel[1], srgb),
                quantise(texel[2], srgb),
                quantise(texel[3], false),
            ]
        })
        .collect()
}

fn decode(raw: u8, srgb: bool) -> f32 {
    let value = raw as f32 / 255.0;
    if srgb { srgb_to_linear(value) } else { value }
}

fn quantise(value: f32, srgb: bool) -> u8 {
    let encoded = if srgb { linear_to_srgb(value) } else { value };
    (encoded.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
