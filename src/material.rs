use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat},
    shader::ShaderRef,
};

use crate::sdf::MATERIAL_SLOTS;

const TRIPLANAR_SHADER: &str = "shaders/triplanar.wgsl";
const LAYER_SIZE: u32 = 128;

pub type SdfSurfaceMaterial = ExtendedMaterial<StandardMaterial, TriplanarExtension>;

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
}

impl MaterialExtension for TriplanarExtension {
    fn fragment_shader() -> ShaderRef {
        TRIPLANAR_SHADER.into()
    }
}

pub struct TriplanarMaterialPlugin;

impl Plugin for TriplanarMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SdfSurfaceMaterial>::default());
    }
}

pub fn build_material_layers(images: &mut Assets<Image>) -> Handle<Image> {
    let size = LAYER_SIZE as usize;
    let mut data = Vec::with_capacity(size * size * MATERIAL_SLOTS * 4);

    for layer in 0..MATERIAL_SLOTS {
        for y in 0..size {
            for x in 0..size {
                let [r, g, b] = layer_texel(layer, x, y, size);
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: LAYER_SIZE,
            height: LAYER_SIZE * MATERIAL_SLOTS as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );

    image
        .reinterpret_stacked_2d_as_array(MATERIAL_SLOTS as u32)
        .expect("stacked layers divide evenly into the image height");

    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });

    images.add(image)
}

fn layer_texel(layer: usize, x: usize, y: usize, size: usize) -> [u8; 3] {
    let fx = x as f32 / size as f32;
    let fy = y as f32 / size as f32;

    match layer {
        0 => {
            let checker = ((x / 16) + (y / 16)) % 2;
            let shade = if checker == 0 { 205 } else { 165 };
            [shade, shade, (shade as f32 * 1.05).min(255.0) as u8]
        }
        1 => {
            let bands = (fy * 14.0).fract();
            let value = 120.0 + 70.0 * (bands * core::f32::consts::TAU).sin();
            [value as u8, (value * 0.72) as u8, (value * 0.42) as u8]
        }
        2 => {
            let n = value_noise(fx * 9.0, fy * 9.0);
            let value = 90.0 + 110.0 * n;
            [(value * 0.55) as u8, value as u8, (value * 0.62) as u8]
        }
        _ => {
            let cx = (fx * 8.0).fract() - 0.5;
            let cy = (fy * 8.0).fract() - 0.5;
            let disc = (cx * cx + cy * cy).sqrt();
            let value = if disc < 0.28 { 235.0 } else { 110.0 };
            [value as u8, (value * 0.85) as u8, (value * 0.5) as u8]
        }
    }
}

fn value_noise(x: f32, y: f32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let xf = x - xi;
    let yf = y - yi;

    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (smooth(xf), smooth(yf));

    let a = hash2(xi, yi);
    let b = hash2(xi + 1.0, yi);
    let c = hash2(xi, yi + 1.0);
    let d = hash2(xi + 1.0, yi + 1.0);

    (a * (1.0 - u) + b * u) * (1.0 - v) + (c * (1.0 - u) + d * u) * v
}

fn hash2(x: f32, y: f32) -> f32 {
    let n = (x * 127.1 + y * 311.7).sin() * 43758.547;
    n - n.floor()
}
