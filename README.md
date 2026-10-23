# sdf_mesh

The viewer half of a mesh editor. A mesh is a signed distance function written
in WESL; the viewer evaluates it on the GPU, meshes it with marching cubes,
shades it with triplanar textures and bakes it to binary glTF. Edit the scene
and save: the mesh updates without a restart.

Scenes do not live here. They live in the project that needs the mesh, which
installs the `sdf_mesh` binary. The binary is self-contained: it embeds the
WESL package `sdf_mesh`, whose utilities a scene imports, and its textures.

## Using it in a mesh project

A mesh project is any directory with a `wesl.toml`; it need not be a Rust
crate. Pin the editor with mise (every green push to `main` is tagged
`v0.<n>`):

```bash
mise use cargo:https://github.com/julianbieber/sdf_mesh@tag:v0.1
```

mise builds it with `cargo install`, so it needs a Rust toolchain and bevy's
system libraries (on Debian/Ubuntu: `libasound2-dev libudev-dev
libwayland-dev libxkbcommon-dev`). Then, from the project:

```bash
sdf_mesh package .sdf_mesh         # the package copy the language server reads
sdf_mesh new shaders/boulder.wesl  # scaffold a scene: a unit sphere
sdf_mesh shaders/boulder.wesl      # open the viewer on it
```

with a `wesl.toml` naming that copy:

```toml
edition = "2026_pre"
root = "shaders"

[dependencies]
sdf_mesh = { path = ".sdf_mesh" }
```

`sdf_mesh package` writes the library modules of the binary that runs it, so
after changing the pinned tag it has to run again; the viewer warns when the
copy differs from what it embeds. Keep `.sdf_mesh/` out of version control,
and let a mise task rewrite it on every run:

```toml
[tools]
"cargo:https://github.com/julianbieber/sdf_mesh" = "tag:v0.1"

[tasks.mesh]
description = "Open a scene in the sdf_mesh viewer"
usage = 'arg "<scene>" help="the scene file, e.g. shaders/boulder.wesl"'
run = 'sdf_mesh package .sdf_mesh && sdf_mesh "${usage_scene}"'
```

`mise run mesh shaders/boulder.wesl` then opens the scene.

Pressing `B` bakes the mesh to `bake/<scene module path>.glb` beside the
project's `wesl.toml` (`shaders/rocks/boulder.wesl` bakes to
`bake/rocks/boulder.glb`).

### Writing a scene

A scene is a module under the package root that defines

```wesl
import sdf_mesh::sdf_lib::{Surface, surface, sd_sphere};

fn sdf_scene(p: vec3<f32>, time: f32) -> Surface {
    return surface(sd_sphere(p, 1.0), 0u);
}
```

The material id passed to `surface()` (0-3) selects a triplanar texture layer.
A scene may import its project's own modules through `package::` or `super::`;
the viewer reads the same `wesl.toml` with the same rules the language server
applies, so both resolve them alike. `example/` is such a project.

The HUD names the open scene and whether it compiled. A scene that fails to
compile leaves the last good mesh on screen and shows the error. An import that
resolves to no module shows as "waiting on an import"; the log names it. Edits
to the scene, or to any module of its project it imports, reload.

Limits that follow from this:

- `sdf_mesh` is the only dependency the viewer provides. A scene importing any
  other package does not compile, and the viewer warns about every other
  dependency `wesl.toml` lists.
- File and directory names on the way to a scene must be WESL identifiers.
- The language server support needs a wgsl-analyzer build recent enough to read
  `wesl.toml` and path dependencies (the nightly from 2026-10-15 does).

## Developing the editor

The justfile is for working on this checkout; every recipe builds it with
cargo. A scene path is relative to the directory `just` is invoked from, so
this checkout's editor opens a scene in any project:

```bash
just run                                   # opens example/shaders/scene.wesl
just run ../my_game/shaders/boulder.wesl   # a scene in another project
just all                                   # fmt-check, clippy, test, as CI runs them
```

`example/` depends on this checkout directly (`sdf_mesh = { path = ".." }`),
so the language server reads the package source being edited. A project that
depends on a `sdf_mesh package` copy instead gets the drift warning when run
against this checkout's changes.

Edits to this package's own modules (`src/shaders`) and textures (`assets/`)
reload only under `just run` / `just run-release`, which is what tells bevy
where the embedded source is. `run-ship` reloads nothing.

## The package

| Module | Role |
|---|---|
| `sdf_mesh::sdf_lib` | `Surface`, `surface()`, the `sd_*` primitives and `op_*` combinators a scene composes from |
| `sdf_mesh::sdf_types` | the uniform and field records the passes share, and `MATERIAL_SLOTS` |
| `sdf_mesh::marching_cubes` | the viewer's meshing passes |
| `sdf_mesh::triplanar` | the viewer's surface shader |

`MATERIAL_SLOTS` and `BLEND_SLOTS` are WESL constants so the language server
sees them; a test keeps them equal to their Rust counterparts.

## bevy_pbr imports in triplanar.wesl

`triplanar.wesl` imports `bevy_pbr::…` and uses `constants::MATERIAL_BIND_GROUP`.
Neither resolves in wgsl-analyzer, and the viewer is unaffected: bevy supplies
both at runtime.

The constants module is generated by bevy when it compiles a material, so no
editor setup can resolve it. Resolving `bevy_pbr` needs it as a path
dependency, because wgsl-analyzer does not yet resolve `package-manager =
"cargo"` dependencies. Untested, but it would take:

- a `wesl.toml` with `root = "src"` in a copy of the `bevy_pbr` crate matching
  the bevy version in `Cargo.lock` (bevy embeds its shaders under
  `bevy_pbr::<path below src>`);
- the same for every bevy crate that `bevy_pbr`'s shaders import in turn;
- `bevy_pbr = { path = "<that copy>" }` (and the others) under
  `[dependencies]` in this repo's `wesl.toml`.

Even then, the `@if` shader defs bevy sets per pipeline are unknown to the
language server, so both branches of each `@if` are analysed.
