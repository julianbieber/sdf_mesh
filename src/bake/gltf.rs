//! Serializes a baked mesh, and the skin and animation of a rigged one, as
//! binary glTF that Bevy's default glTF loader can open without loader
//! settings.

use std::{borrow::Cow, collections::BTreeMap, fmt, mem};

use bevy::{
    gltf::gltf::{
        Glb,
        binary::Header,
        json::{
            self, Accessor, Animation, Buffer, Index, Mesh as GltfMesh, Node, Root, Scene, Skin,
            Value,
            accessor::{ComponentType, GenericComponentType, Type},
            animation::{Channel, Interpolation, Property, Sampler, Target as ChannelTarget},
            buffer::{Target, View},
            mesh::{Mode, Primitive, Semantic},
            scene::UnitQuaternion,
            validation::{Checked, USize64},
        },
    },
    mesh::{Indices, VertexAttributeValues},
    prelude::*,
};

use super::skin::SkinAnimation;

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
/// With `skin`, the primitive also carries JOINTS_0 and WEIGHTS_0, the mesh
/// node is skinned to one node per joint — joint 0 a root node beside it, the
/// others its children, each named `joint_<i>` and placed at keyframe 0 — and
/// the file holds one animation, `sdf_animation`, with a linear translation
/// and rotation channel per joint.
///
/// `mesh` must carry float3 positions and normals, float4 colours and u32
/// indices, and with `skin` u16x4 joint indices and float4 joint weights; the
/// colours are written exactly as given. Fails with
/// [`GlbError::NoTriangles`] or [`GlbError::NonFinitePosition`] when the file
/// would have no valid POSITION bounds, which Bevy's loader cannot open.
pub fn mesh_to_glb(mesh: &Mesh, skin: Option<&SkinAnimation>) -> Result<Vec<u8>, GlbError> {
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

    let skin_attributes = match skin {
        None => None,
        Some(_) => {
            let Some(VertexAttributeValues::Uint16x4(joints)) =
                mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
            else {
                return Err(GlbError::MissingAttribute("u16x4 joint indices"));
            };
            let Some(VertexAttributeValues::Float32x4(weights)) =
                mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
            else {
                return Err(GlbError::MissingAttribute("float4 joint weights"));
            };
            Some((joints, weights))
        }
    };

    let mut bin = Vec::new();
    let mut root = Root::default();
    let buffer = root.push(Buffer {
        byte_length: USize64::from(0usize),
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
            target: Some(Target::ArrayBuffer),
            bounds: Some((min.to_vec(), max.to_vec())),
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
            target: Some(Target::ArrayBuffer),
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
            target: Some(Target::ArrayBuffer),
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
            target: Some(Target::ElementArrayBuffer),
            bounds: None,
        },
    );

    let mut attributes = BTreeMap::from([
        (Checked::Valid(Semantic::Positions), position),
        (Checked::Valid(Semantic::Normals), normal),
        (Checked::Valid(Semantic::Colors(0)), color),
    ]);
    if let Some((joints, weights)) = skin_attributes {
        let joint_index = push_region(
            &mut root,
            &mut bin,
            buffer,
            joints
                .iter()
                .flat_map(|j| [pack_u16(j[0], j[1]), pack_u16(j[2], j[3])]),
            Region {
                count: joints.len(),
                type_: Type::Vec4,
                component: ComponentType::U16,
                target: Some(Target::ArrayBuffer),
                bounds: None,
            },
        );
        let joint_weight = push_region(
            &mut root,
            &mut bin,
            buffer,
            weights.iter().flatten().map(|c| c.to_le_bytes()),
            Region {
                count: weights.len(),
                type_: Type::Vec4,
                component: ComponentType::F32,
                target: Some(Target::ArrayBuffer),
                bounds: None,
            },
        );
        attributes.insert(Checked::Valid(Semantic::Joints(0)), joint_index);
        attributes.insert(Checked::Valid(Semantic::Weights(0)), joint_weight);
    }

    let primitive = Primitive {
        attributes,
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
    let mut scene_nodes = vec![node];
    if let Some(skin) = skin {
        let (skin_index, root_joint) = push_skin(&mut root, &mut bin, buffer, skin);
        root.nodes[node.value()].skin = Some(skin_index);
        scene_nodes.push(root_joint);
    }
    let scene = root.push(Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: scene_nodes,
    });
    root.scene = Some(scene);
    root.buffers[buffer.value()].byte_length = USize64::from(bin.len());

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

fn push_skin(
    root: &mut Root,
    bin: &mut Vec<u8>,
    buffer: Index<Buffer>,
    skin: &SkinAnimation,
) -> (Index<Skin>, Index<Node>) {
    let first_joint = root.nodes.len() as u32;
    let joints: Vec<Index<Node>> = (0..skin.joint_count())
        .map(|i| Index::new(first_joint + i as u32))
        .collect();
    for (i, track) in skin.tracks.iter().enumerate() {
        let rotation = track.rotations[0];
        root.push(Node {
            name: Some(format!("joint_{i}")),
            translation: Some(track.translations[0].to_array()),
            rotation: Some(UnitQuaternion(rotation.to_array())),
            children: (i == 0 && joints.len() > 1).then(|| joints[1..].to_vec()),
            ..Default::default()
        });
    }

    let inverse_binds = push_region(
        root,
        bin,
        buffer,
        skin.inverse_binds
            .iter()
            .flat_map(|m| m.to_cols_array())
            .map(f32::to_le_bytes),
        Region {
            count: skin.inverse_binds.len(),
            type_: Type::Mat4,
            component: ComponentType::F32,
            target: None,
            bounds: None,
        },
    );
    let skin_index = root.push(Skin {
        extensions: None,
        extras: Default::default(),
        inverse_bind_matrices: Some(inverse_binds),
        joints: joints.clone(),
        name: Some("sdf_skin".to_owned()),
        skeleton: Some(joints[0]),
    });

    let (first, last) = (skin.times[0], skin.times[skin.times.len() - 1]);
    let times = push_region(
        root,
        bin,
        buffer,
        skin.times.iter().map(|t| t.to_le_bytes()),
        Region {
            count: skin.times.len(),
            type_: Type::Scalar,
            component: ComponentType::F32,
            target: None,
            bounds: Some((vec![first], vec![last])),
        },
    );
    let mut samplers = Vec::new();
    let mut channels = Vec::new();
    for (joint, track) in joints.iter().zip(&skin.tracks) {
        let translations = push_region(
            root,
            bin,
            buffer,
            track
                .translations
                .iter()
                .flat_map(|v| v.to_array())
                .map(f32::to_le_bytes),
            Region {
                count: track.translations.len(),
                type_: Type::Vec3,
                component: ComponentType::F32,
                target: None,
                bounds: None,
            },
        );
        let rotations = push_region(
            root,
            bin,
            buffer,
            track
                .rotations
                .iter()
                .flat_map(|q| q.to_array())
                .map(f32::to_le_bytes),
            Region {
                count: track.rotations.len(),
                type_: Type::Vec4,
                component: ComponentType::F32,
                target: None,
                bounds: None,
            },
        );
        for (output, path) in [
            (translations, Property::Translation),
            (rotations, Property::Rotation),
        ] {
            channels.push(Channel {
                sampler: Index::new(samplers.len() as u32),
                target: ChannelTarget {
                    extensions: None,
                    extras: Default::default(),
                    node: *joint,
                    path: Checked::Valid(path),
                },
                extensions: None,
                extras: Default::default(),
            });
            samplers.push(Sampler {
                extensions: None,
                extras: Default::default(),
                input: times,
                interpolation: Checked::Valid(Interpolation::Linear),
                output,
            });
        }
    }
    root.push(Animation {
        extensions: None,
        extras: Default::default(),
        channels,
        name: Some("sdf_animation".to_owned()),
        samplers,
    });

    (skin_index, joints[0])
}

fn pack_u16(low: u16, high: u16) -> [u8; 4] {
    let [a, b] = low.to_le_bytes();
    let [c, d] = high.to_le_bytes();
    [a, b, c, d]
}

struct Region {
    count: usize,
    type_: Type,
    component: ComponentType,
    target: Option<Target>,
    bounds: Option<(Vec<f32>, Vec<f32>)>,
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
        target: region.target.map(Checked::Valid),
        extensions: None,
        extras: Default::default(),
    });
    let (min, max) = region
        .bounds
        .map(|(min, max)| (Value::from(min), Value::from(max)))
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
        let bytes = mesh_to_glb(&mesh, None).expect("encodes");
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
        let bytes = mesh_to_glb(&mesh, None).expect("encodes");
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

    fn rigged_glb(rig: &crate::rig::Rig) -> Vec<u8> {
        use crate::sdf::JointSample;
        let samples: Vec<JointSample> = (0..rig.keyframes())
            .flat_map(|frame| {
                (0..rig.joints).map(move |joint| JointSample {
                    translation: Vec3::new(joint as f32, frame as f32 * 0.1, 0.0),
                    rotation: Vec4::from(Quat::from_rotation_z(frame as f32 * 0.05)),
                })
            })
            .collect();
        let skin = super::super::skin::SkinAnimation::from_samples(rig, &samples).unwrap();
        let mesh = quad()
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_JOINT_INDEX,
                VertexAttributeValues::Uint16x4(vec![[0, 1, 0, 0]; 4]),
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_JOINT_WEIGHT,
                vec![[0.5f32, 0.5, 0.0, 0.0]; 4],
            );
        mesh_to_glb(&mesh, Some(&skin)).expect("encodes")
    }

    #[test]
    fn a_rigged_bake_loads_in_bevy_with_a_skin_of_the_scene_joints_and_one_animation() {
        use bevy::{
            animation::AnimationClip,
            app::TaskPoolPlugin,
            asset::{
                AssetPlugin, LoadState,
                io::{
                    AssetSourceBuilder, AssetSourceId,
                    memory::{Dir, MemoryAssetReader},
                },
            },
            gltf::{GltfPlugin, GltfSkin},
            mesh::{MeshPlugin, skinning::SkinnedMeshInverseBindposes},
            world_serialization::WorldSerializationPlugin,
        };

        let source = std::fs::read_to_string("example/shaders/animated_skinned.wesl").unwrap();
        let rig = crate::rig::read_rig(&source)
            .unwrap()
            .expect("the scene declares a rig");

        let dir = Dir::default();
        dir.insert_asset(std::path::Path::new("rigged.glb"), rigged_glb(&rig));
        let mut app = App::new();
        let reader = MemoryAssetReader { root: dir };
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(move || Box::new(reader.clone())),
        )
        .add_plugins((
            TaskPoolPlugin::default(),
            AssetPlugin::default(),
            WorldSerializationPlugin,
            MeshPlugin,
            GltfPlugin::default(),
        ))
        .init_asset::<AnimationClip>();
        app.finish();
        app.cleanup();

        let handle: Handle<bevy::gltf::Gltf> =
            app.world().resource::<AssetServer>().load("rigged.glb");
        for _ in 0..10_000 {
            app.update();
            match app.world().resource::<AssetServer>().load_state(&handle) {
                LoadState::Loaded => break,
                LoadState::Failed(error) => panic!("{error}"),
                _ => {}
            }
        }

        let world = app.world();
        let gltf = world
            .resource::<Assets<bevy::gltf::Gltf>>()
            .get(&handle)
            .expect("loaded");
        assert_eq!(gltf.skins.len(), 1);
        let skin = world
            .resource::<Assets<GltfSkin>>()
            .get(&gltf.skins[0])
            .unwrap();
        assert_eq!(skin.joints.len(), rig.joints as usize);
        let binds = world
            .resource::<Assets<SkinnedMeshInverseBindposes>>()
            .get(&skin.inverse_bind_matrices)
            .unwrap();
        assert_eq!(binds.len(), rig.joints as usize);
        assert_eq!(gltf.animations.len(), 1);
        let clip = world
            .resource::<Assets<AnimationClip>>()
            .get(&gltf.animations[0])
            .unwrap();
        assert_eq!(clip.duration(), rig.seconds);
    }

    // Pins the guard: a mesh without triangles has no POSITION bounds Bevy's loader can read.
    #[test]
    fn empty_mesh_is_refused() {
        let empty = quad().with_inserted_indices(Indices::U32(Vec::new()));
        assert!(matches!(
            mesh_to_glb(&empty, None),
            Err(GlbError::NoTriangles)
        ));
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
            mesh_to_glb(&broken, None),
            Err(GlbError::NonFinitePosition)
        ));
    }
}
