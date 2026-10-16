//! Serializes a baked mesh as binary glTF that Bevy's default glTF loader can
//! open without loader settings.

use std::{borrow::Cow, collections::BTreeMap, fmt, mem};

use bevy::{
    gltf::gltf::{
        Glb,
        binary::Header,
        json::{
            self, Accessor, Buffer, Index, Mesh as GltfMesh, Node, Root, Scene, Value,
            accessor::{ComponentType, GenericComponentType, Type},
            buffer::{Target, View},
            mesh::{Mode, Primitive, Semantic},
            validation::{Checked, USize64},
        },
    },
    mesh::{Indices, VertexAttributeValues},
    prelude::*,
};

/// Why [`mesh_to_glb`] produced no file. `NoTriangles` and
/// `NonFinitePosition` describe a mesh that cannot be exported; the other two
/// are a mesh built without the expected attributes, or a serializer failure.
#[derive(Debug)]
pub enum GlbError {
    MissingAttribute(&'static str),
    NoTriangles,
    NonFinitePosition,
    Serialize(String),
}

impl fmt::Display for GlbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAttribute(name) => write!(f, "mesh has no {name}"),
            Self::NoTriangles => write!(f, "mesh has no triangles"),
            Self::NonFinitePosition => write!(f, "mesh has a non-finite vertex position"),
            Self::Serialize(error) => write!(f, "glTF serialization failed: {error}"),
        }
    }
}

impl std::error::Error for GlbError {}

/// Encodes `mesh` as a `.glb`: one triangle-list primitive with POSITION,
/// NORMAL, COLOR_0 and u32 indices, one node, one default scene, no material.
///
/// `mesh` must carry float3 positions and normals, float4 colours and u32
/// indices; the colours are written exactly as given. Fails with
/// [`GlbError::NoTriangles`] or [`GlbError::NonFinitePosition`] when the file
/// would have no valid POSITION bounds, which Bevy's loader cannot open.
pub fn mesh_to_glb(mesh: &Mesh) -> Result<Vec<u8>, GlbError> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err(GlbError::MissingAttribute("float3 positions"));
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        return Err(GlbError::MissingAttribute("float3 normals"));
    };
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        return Err(GlbError::MissingAttribute("float4 colors"));
    };
    let Some(Indices::U32(indices)) = mesh.indices() else {
        return Err(GlbError::MissingAttribute("u32 indices"));
    };
    if indices.len() < 3 {
        return Err(GlbError::NoTriangles);
    }
    if positions.iter().flatten().any(|c| !c.is_finite()) {
        return Err(GlbError::NonFinitePosition);
    }

    let (min, max) = positions.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(min, max), p| {
            (
                std::array::from_fn(|i| min[i].min(p[i])),
                std::array::from_fn(|i| max[i].max(p[i])),
            )
        },
    );

    let byte_len = (positions.len() * 3 + normals.len() * 3 + colors.len() * 4 + indices.len()) * 4;
    let mut bin = Vec::with_capacity(byte_len);
    let mut root = Root::default();
    let buffer = root.push(Buffer {
        byte_length: USize64::from(byte_len),
        name: None,
        uri: None,
        extensions: None,
        extras: Default::default(),
    });

    let position = push_region(
        &mut root,
        &mut bin,
        buffer,
        positions.iter().flatten().map(|c| c.to_le_bytes()),
        Region {
            count: positions.len(),
            type_: Type::Vec3,
            component: ComponentType::F32,
            target: Target::ArrayBuffer,
            bounds: Some((min, max)),
        },
    );
    let normal = push_region(
        &mut root,
        &mut bin,
        buffer,
        normals.iter().flatten().map(|c| c.to_le_bytes()),
        Region {
            count: normals.len(),
            type_: Type::Vec3,
            component: ComponentType::F32,
            target: Target::ArrayBuffer,
            bounds: None,
        },
    );
    let color = push_region(
        &mut root,
        &mut bin,
        buffer,
        colors.iter().flatten().map(|c| c.to_le_bytes()),
        Region {
            count: colors.len(),
            type_: Type::Vec4,
            component: ComponentType::F32,
            target: Target::ArrayBuffer,
            bounds: None,
        },
    );
    let index = push_region(
        &mut root,
        &mut bin,
        buffer,
        indices.iter().map(|i| i.to_le_bytes()),
        Region {
            count: indices.len(),
            type_: Type::Scalar,
            component: ComponentType::U32,
            target: Target::ElementArrayBuffer,
            bounds: None,
        },
    );

    let primitive = Primitive {
        attributes: BTreeMap::from([
            (Checked::Valid(Semantic::Positions), position),
            (Checked::Valid(Semantic::Normals), normal),
            (Checked::Valid(Semantic::Colors(0)), color),
        ]),
        extensions: None,
        extras: Default::default(),
        indices: Some(index),
        material: None,
        mode: Checked::Valid(Mode::Triangles),
        targets: None,
    };
    let mesh = root.push(GltfMesh {
        extensions: None,
        extras: Default::default(),
        name: Some("sdf_surface".to_owned()),
        primitives: vec![primitive],
        weights: None,
    });
    let node = root.push(Node {
        mesh: Some(mesh),
        name: Some("sdf_surface".to_owned()),
        ..Default::default()
    });
    let scene = root.push(Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: vec![node],
    });
    root.scene = Some(scene);

    let json = json::serialize::to_vec(&root).map_err(|e| GlbError::Serialize(e.to_string()))?;
    let length = glb_length(json.len(), bin.len());
    Glb {
        header: Header {
            magic: *b"glTF",
            version: 2,
            length,
        },
        json: Cow::Owned(json),
        bin: Some(Cow::Owned(bin)),
    }
    .to_vec()
    .map_err(|e| GlbError::Serialize(e.to_string()))
}

struct Region {
    count: usize,
    type_: Type,
    component: ComponentType,
    target: Target,
    bounds: Option<([f32; 3], [f32; 3])>,
}

fn push_region(
    root: &mut Root,
    bin: &mut Vec<u8>,
    buffer: Index<Buffer>,
    bytes: impl Iterator<Item = [u8; 4]>,
    region: Region,
) -> Index<Accessor> {
    let offset = bin.len();
    bin.extend(bytes.flatten());
    let view = root.push(View {
        buffer,
        byte_length: USize64::from(bin.len() - offset),
        byte_offset: Some(USize64::from(offset)),
        byte_stride: None,
        name: None,
        target: Some(Checked::Valid(region.target)),
        extensions: None,
        extras: Default::default(),
    });
    let (min, max) = region
        .bounds
        .map(|(min, max)| (Value::from(min.to_vec()), Value::from(max.to_vec())))
        .unzip();
    root.push(Accessor {
        buffer_view: Some(view),
        byte_offset: None,
        count: USize64::from(region.count),
        component_type: Checked::Valid(GenericComponentType(region.component)),
        extensions: None,
        extras: Default::default(),
        type_: Checked::Valid(region.type_),
        min,
        max,
        name: None,
        normalized: false,
        sparse: None,
    })
}

fn glb_length(json_len: usize, bin_len: usize) -> u32 {
    let chunk_header = 2 * mem::size_of::<u32>();
    let header = 3 * mem::size_of::<u32>();
    (header
        + chunk_header
        + json_len.next_multiple_of(4)
        + chunk_header
        + bin_len.next_multiple_of(4)) as u32
}

#[cfg(test)]
mod tests {
    use bevy::{
        asset::RenderAssetUsages,
        gltf::{gltf::Gltf, vertex_attributes::convert_attribute},
        mesh::PrimitiveTopology,
        platform::collections::HashMap,
    };

    use super::*;

    fn quad() -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 2.0, 0.0],
                [-1.0, 2.0, 3.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![
                [1.0, 0.0, 0.0, 0.0],
                [0.25, 0.75, 0.0, 0.0],
                [0.0, 0.0, 1.5, -0.5],
                [0.0, 0.0, 0.0, 1.0],
            ],
        )
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
    }

    fn colors(mesh: &Mesh) -> Vec<[f32; 4]> {
        match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(c)) => c.clone(),
            other => panic!("unexpected colors {other:?}"),
        }
    }

    // Pins that the file is valid glTF and reads back every stream, the indices and the bounds unchanged.
    #[test]
    fn glb_round_trips_through_the_gltf_reader() {
        let mesh = quad();
        let bytes = mesh_to_glb(&mesh).expect("encodes");
        let gltf = Gltf::from_slice(&bytes).expect("valid glTF");
        let blob = gltf.blob.as_deref();
        let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
        let reader = primitive.reader(|_| blob);

        let positions: Vec<[f32; 3]> = reader.read_positions().unwrap().collect();
        let normals: Vec<[f32; 3]> = reader.read_normals().unwrap().collect();
        let read_colors: Vec<[f32; 4]> = reader.read_colors(0).unwrap().into_rgba_f32().collect();
        let indices: Vec<u32> = reader.read_indices().unwrap().into_u32().collect();
        let Some(VertexAttributeValues::Float32x3(expected)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!()
        };

        assert_eq!(&positions, expected);
        assert_eq!(normals, vec![[0.0, 0.0, 1.0]; 4]);
        assert_eq!(read_colors, colors(&mesh));
        assert_eq!(indices, vec![0, 1, 2, 0, 2, 3]);
        let bounds = primitive.bounding_box();
        assert_eq!(bounds.min, [-1.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [1.0, 2.0, 3.0]);
        assert!(gltf.default_scene().is_some());
    }

    // Pins that Bevy's default loader puts the weights into ATTRIBUTE_COLOR, unclamped.
    #[test]
    fn bevy_loader_maps_color_0_to_attribute_color_unchanged() {
        let mesh = quad();
        let bytes = mesh_to_glb(&mesh).expect("encodes");
        let gltf = Gltf::from_slice(&bytes).expect("valid glTF");
        let buffers = vec![gltf.blob.clone().unwrap()];
        let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
        let (semantic, accessor) = primitive
            .attributes()
            .find(|(s, _)| *s == bevy::gltf::gltf::Semantic::Colors(0))
            .unwrap();

        let (attribute, values) =
            convert_attribute(semantic, accessor, &buffers, &HashMap::default(), false)
                .expect("converts");

        assert_eq!(attribute.id, Mesh::ATTRIBUTE_COLOR.id);
        let VertexAttributeValues::Float32x4(loaded) = values else {
            panic!("unexpected format {values:?}");
        };
        assert_eq!(loaded, colors(&mesh));
    }

    // Pins the guard: a mesh without triangles has no POSITION bounds Bevy's loader can read.
    #[test]
    fn empty_mesh_is_refused() {
        let empty = quad().with_inserted_indices(Indices::U32(Vec::new()));
        assert!(matches!(mesh_to_glb(&empty), Err(GlbError::NoTriangles)));
    }

    // Pins the guard: a NaN position serializes as a null bound, which Bevy's loader panics on.
    #[test]
    fn non_finite_position_is_refused() {
        let broken = quad().with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [f32::NAN, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 2.0, 0.0],
                [-1.0, 2.0, 3.0],
            ],
        );
        assert!(matches!(
            mesh_to_glb(&broken),
            Err(GlbError::NonFinitePosition)
        ));
    }
}
