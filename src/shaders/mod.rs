//! The WESL package `sdf_mesh` as the editor sees it: what it is named,
//! where it is embedded, and how the editor calls into a scene that imports it.

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::shader::Shader;

use crate::sdf::pipeline::FIELD_WORKGROUP;

/// The package name a scene imports the library under, in the editor and in
/// the mesh project's `wesl.toml` alike.
pub const PACKAGE: &str = "sdf_mesh";

/// The embedded asset path of each library module, in the form bevy maps to the
/// module path `sdf_mesh::<file stem>`.
pub const MODULE_ASSETS: [&str; 2] = [
    "embedded://sdf_mesh/sdf_types.wesl",
    "embedded://sdf_mesh/sdf_lib.wesl",
];

/// The file name and source of each library module a scene may import, as
/// embedded in this build.
pub const LIBRARY_SOURCES: [(&str, &str); 2] = [
    ("sdf_types.wesl", include_str!("sdf_types.wesl")),
    ("sdf_lib.wesl", include_str!("sdf_lib.wesl")),
];

/// The embedded asset path of the meshing compute shader.
pub const MARCHING_CUBES_SHADER: &str = "embedded://sdf_mesh/marching_cubes.wesl";

/// The embedded asset path of the surface material's fragment shader.
pub const TRIPLANAR_SHADER: &str = "embedded://sdf_mesh/triplanar.wesl";

/// The asset path the entry module is registered under.
///
/// Its first component is not a WESL identifier, so no file of a mesh project
/// can map to the same module and displace it.
pub const ENTRY_ASSET: &str = "sdf-mesh-entry/evaluate.wesl";

/// Embeds the package and keeps its library modules loaded for the life of the
/// app.
pub struct ShaderPackagePlugin;

impl Plugin for ShaderPackagePlugin {
    fn build(&self, app: &mut App) {
        embed(app);
        app.add_systems(PreStartup, load_library);
    }
}

/// Strong handles to the library modules.
///
/// Held because bevy never loads a module that is only named by an import:
/// without a live handle, every shader importing `sdf_mesh::…` waits on it
/// forever.
#[derive(Resource, Debug)]
pub struct ShaderLibrary {
    handles: Vec<Handle<Shader>>,
}

impl ShaderLibrary {
    /// Whether every library module has finished loading.
    pub fn is_loaded(&self, asset_server: &AssetServer) -> bool {
        self.handles
            .iter()
            .all(|handle| asset_server.is_loaded_with_dependencies(handle))
    }
}

/// Registers every package module as an embedded asset under `sdf_mesh/`.
///
/// Must run after `AssetPlugin`, which owns the embedded registry.
pub fn embed(app: &mut App) {
    embedded_asset!(app, "src/shaders", "sdf_types.wesl");
    embedded_asset!(app, "src/shaders", "sdf_lib.wesl");
    embedded_asset!(app, "src/shaders", "marching_cubes.wesl");
    embedded_asset!(app, "src/shaders", "triplanar.wesl");
}

fn load_library(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(ShaderLibrary {
        handles: MODULE_ASSETS
            .iter()
            .map(|path| asset_server.load(*path))
            .collect(),
    });
}

/// The source of the entry module for the scene at `package::<scene_module>`.
///
/// It declares the `evaluate` compute entry point, dispatched in
/// [`FIELD_WORKGROUP`]-cubed workgroups, which samples the scene's
/// `sdf_scene(p, time)` at every grid point into the field buffer.
pub fn entry_module_source(scene_module: &[String]) -> String {
    format!(
        "import package::{scene}::sdf_scene;\n\
         import {PACKAGE}::sdf_types::{{SdfParams, FieldSample}};\n\
         \n\
         @group(0) @binding(0) var<uniform> params: SdfParams;\n\
         @group(0) @binding(1) var<storage, read_write> field: array<FieldSample>;\n\
         \n\
         @compute @workgroup_size({FIELD_WORKGROUP}, {FIELD_WORKGROUP}, {FIELD_WORKGROUP})\n\
         fn evaluate(@builtin(global_invocation_id) gid: vec3<u32>) {{\n\
         \x20   let res = params.grid_res;\n\
         \x20   if gid.x >= res || gid.y >= res || gid.z >= res {{\n\
         \x20       return;\n\
         \x20   }}\n\
         \x20   let position = params.origin + vec3<f32>(gid) * params.cell_size;\n\
         \x20   let sample = sdf_scene(position, params.time);\n\
         \x20   let index = gid.x + gid.y * res + gid.z * res * res;\n\
         \x20   field[index] = FieldSample(sample.weights, sample.dist);\n\
         }}\n",
        scene = scene_module.join("::"),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bevy::asset::AssetId;
    use bevy::asset::io::embedded::EmbeddedAssetRegistry;
    use bevy::asset::uuid::Uuid;
    use bevy::shader::{ShaderCache, ShaderCacheError, ShaderCacheSource, ValidateShader};
    use wesl::syntax::{GlobalDeclaration, TranslationUnit};

    use super::*;
    use crate::material::BLEND_SLOTS;
    use crate::project::MeshProject;
    use crate::sdf::MATERIAL_SLOTS;

    type Cache = ShaderCache<String, ()>;

    fn to_wgsl(
        _: &(),
        source: ShaderCacheSource,
        _: &ValidateShader,
    ) -> Result<String, ShaderCacheError> {
        match source {
            ShaderCacheSource::Wgsl(wgsl) => Ok(wgsl),
            _ => Err(ShaderCacheError::ProcessShaderError("not WGSL".into())),
        }
    }

    fn id(n: u128) -> AssetId<Shader> {
        AssetId::Uuid {
            uuid: Uuid::from_u128(n + 1),
        }
    }

    fn package_source(asset: &str) -> String {
        let file = asset.trim_start_matches("embedded://sdf_mesh/");
        std::fs::read_to_string(Path::new("src/shaders").join(file))
            .expect("package module should be readable")
    }

    fn compose(cache: &mut Cache, n: u128) -> naga::Module {
        let wgsl = cache.get(0, id(n), &[]).unwrap_or_else(|e| panic!("{e}"));
        naga::front::wgsl::parse_str(&wgsl)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&wgsl)))
    }

    fn library_cache() -> Cache {
        let mut cache = Cache::new((), to_wgsl);
        for (n, asset) in MODULE_ASSETS.iter().enumerate() {
            cache.set_shader(
                id(n as u128),
                Shader::from_wesl(package_source(asset), *asset),
            );
        }
        cache
    }

    fn const_initializer(source: &str, name: &str) -> String {
        let unit: TranslationUnit = source.parse().expect("module should parse");
        unit.global_declarations
            .iter()
            .find_map(|decl| match &**decl {
                GlobalDeclaration::Declaration(d) if *d.ident.name() == name => {
                    d.initializer.as_ref().map(ToString::to_string)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no const {name}"))
    }

    // Pins the embedded paths bevy turns into the module names `sdf_mesh::<file stem>`.
    #[test]
    fn package_modules_embed_at_the_paths_bevy_names_as_the_package() {
        let mut app = App::new();
        app.init_resource::<EmbeddedAssetRegistry>();
        embed(&mut app);
        let registry = app.world().resource::<EmbeddedAssetRegistry>();
        for asset in MODULE_ASSETS
            .iter()
            .chain(&[MARCHING_CUBES_SHADER, TRIPLANAR_SHADER])
        {
            let path = asset.trim_start_matches("embedded://");
            assert!(
                registry.remove_asset(Path::new(path)).is_some(),
                "nothing embedded at {path}"
            );
        }
    }

    // Pins that the generated entry module composes the example scene and the library into `evaluate`.
    #[test]
    fn the_example_scene_composes_through_the_entry_module() {
        let project = MeshProject::open(Path::new("example/shaders/scene.wesl"))
            .expect("example scene should open");
        let mut cache = library_cache();
        let scene = std::fs::read_to_string(&project.scene_file).unwrap();
        cache.set_shader(id(10), Shader::from_wesl(scene, project.scene_asset_path()));
        cache.set_shader(
            id(11),
            Shader::from_wesl(entry_module_source(&project.scene_module), ENTRY_ASSET),
        );
        let module = compose(&mut cache, 11);
        assert!(module.entry_points.iter().any(|e| e.name == "evaluate"));
    }

    // Pins that the meshing shader composes against the library it imports through `package::`.
    #[test]
    fn marching_cubes_composes_against_the_library() {
        let mut cache = library_cache();
        cache.set_shader(
            id(20),
            Shader::from_wesl(package_source(MARCHING_CUBES_SHADER), MARCHING_CUBES_SHADER),
        );
        let module = compose(&mut cache, 20);
        for entry in ["prepare", "march"] {
            assert!(
                module.entry_points.iter().any(|e| e.name == entry),
                "{entry}"
            );
        }
    }

    // Pins the WESL slot constants, kept in WESL for the language server, to their Rust values.
    #[test]
    fn wesl_slot_constants_match_rust() {
        assert_eq!(
            const_initializer(&package_source(MODULE_ASSETS[0]), "MATERIAL_SLOTS"),
            format!("{MATERIAL_SLOTS}u")
        );
        assert_eq!(
            const_initializer(&package_source(TRIPLANAR_SHADER), "BLEND_SLOTS"),
            format!("{BLEND_SLOTS}u")
        );
    }
}
