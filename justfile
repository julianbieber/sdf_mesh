set shell := ["bash", "-uc"]

EXAMPLE := justfile_directory() / "example/shaders/scene.wesl"
GLB := "example/bake/scene.glb"

default:
    @just --list

# Open this checkout's viewer on a scene (default: the example), relative to where just is invoked; edits hot reload, including to src/shaders
run scene=EXAMPLE:
    cargo run -- {{ quote(join(invocation_directory(), scene)) }}

# Run optimised, hot reload still on
run-release scene=EXAMPLE:
    cargo run --release -- {{ quote(join(invocation_directory(), scene)) }}

# Run without the asset file watcher
run-ship scene=EXAMPLE:
    cargo run --release --no-default-features -- {{ quote(join(invocation_directory(), scene)) }}

# Write the starter material source PNGs the bake pipeline reads
gen-textures:
    cargo run --bin gen_material_textures

build:
    cargo build

build-release:
    cargo build --release

check:
    cargo check --all-targets

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets -- -D warnings

test:
    cargo test

all: fmt-check clippy test

clean:
    cargo clean
    rm -rf example/bake

# Prose the AI policy leaves to a human. Not part of `all`.
check-placeholders:
    @rg -n 'TODO\(jb-(doc|comment)\):' src/ example/ assets/ || echo "none outstanding"

# Run under a throwaway X server with software Vulkan, capture a PNG
screenshot OUT="shot.png" SECONDS="20" scene=EXAMPLE:
    #!/usr/bin/env bash
    set -uo pipefail
    command -v Xvfb >/dev/null || { echo "Xvfb not installed"; exit 1; }
    disp=":97"
    Xvfb "$disp" -screen 0 1280x960x24 >/dev/null 2>&1 &
    xvfb=$!
    trap 'kill $xvfb 2>/dev/null' EXIT
    timeout 2 tail -f /dev/null
    DISPLAY="$disp" VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
        cargo run -- {{ quote(join(invocation_directory(), scene)) }} > /tmp/sdf_mesh_screenshot.log 2>&1 &
    app=$!
    timeout "{{ SECONDS }}" tail -f /dev/null
    win=$(DISPLAY="$disp" xdotool search --name '^sdf_mesh$' 2>/dev/null | head -1)
    if [ -z "$win" ]; then
        echo "  window never appeared; see /tmp/sdf_mesh_screenshot.log"
        kill $app 2>/dev/null
        exit 1
    fi
    DISPLAY="$disp" xwd -id "$win" -silent | convert xwd:- "{{ OUT }}"
    pkill -P $app 2>/dev/null; kill $app 2>/dev/null
    echo "  wrote {{ OUT }}"

# Check the baked GLB is a closed, consistently wound manifold
verify-glb FILE=GLB:
    #!/usr/bin/env python3
    import json, math, struct, sys
    from collections import Counter
    path = "{{ FILE }}"
    try:
        data = open(path, "rb").read()
    except FileNotFoundError:
        sys.exit(f"  {path} not found -- press B in the editor to bake one")
    magic, version, length = struct.unpack_from("<4sII", data, 0)
    if magic != b"glTF" or version != 2 or length != len(data):
        sys.exit(f"  FAIL not a glTF 2 binary (magic {magic!r}, version {version}, length {length}/{len(data)})")
    chunks, offset = {}, 12
    while offset < len(data):
        size, kind = struct.unpack_from("<I4s", data, offset)
        chunks[kind] = data[offset + 8 : offset + 8 + size]
        offset += 8 + size
    doc, bin_chunk = json.loads(chunks[b"JSON"]), chunks.get(b"BIN\0", b"")
    def read(index, width, fmt):
        acc = doc["accessors"][index]
        view = doc["bufferViews"][acc["bufferView"]]
        start = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
        flat = struct.unpack_from(f"<{acc['count'] * width}{fmt}", bin_chunk, start)
        return [tuple(flat[i : i + width]) for i in range(0, len(flat), width)], acc
    prim = doc["meshes"][0]["primitives"][0]
    vs, pos_acc = read(prim["attributes"]["POSITION"], 3, "f")
    ns, _ = read(prim["attributes"]["NORMAL"], 3, "f")
    ws, _ = read(prim["attributes"]["COLOR_0"], 4, "f")
    flat_idx, _ = read(prim["indices"], 1, "I")
    idx = [i[0] for i in flat_idx]
    fs = [tuple(idx[i : i + 3]) for i in range(0, len(idx), 3)]
    print(f"  {path}: {len(vs)} vertices, {len(fs)} faces")
    if not fs:
        sys.exit("  FAIL no faces")
    axes = list(zip(*vs))
    print("  bbox " + " ".join(f"{n}[{min(a):.3f},{max(a):.3f}]" for n, a in zip("xyz", axes)))
    problems = []
    if any(abs(pos_acc["min"][i] - min(axes[i])) > 1e-6 or abs(pos_acc["max"][i] - max(axes[i])) > 1e-6 for i in range(3)):
        problems.append("POSITION min/max disagree with the data")
    if any(i >= len(vs) for f in fs for i in f):
        problems.append("index out of range")
    if any(not math.isfinite(c) for v in vs for c in v):
        problems.append("non-finite vertex")
    if any(not math.isfinite(c) for w in ws for c in w):
        problems.append("non-finite material weight")
    if any(len(set(f)) < 3 for f in fs):
        problems.append("degenerate face")
    lengths = [math.sqrt(sum(c * c for c in n)) for n in ns]
    if lengths and (min(lengths) < 0.99 or max(lengths) > 1.01):
        problems.append(f"normal length {min(lengths):.4f}..{max(lengths):.4f}")
    weld = lambda i: tuple(round(c, 4) for c in vs[i])
    edges = Counter()
    for f in fs:
        k = [weld(i) for i in f]
        for a in range(3):
            edges[(k[a], k[(a + 1) % 3])] += 1
    unmatched = sum(1 for e, c in edges.items() if edges.get((e[1], e[0]), 0) != c)
    if unmatched:
        problems.append(f"{unmatched} unmatched directed edges")
    for problem in problems:
        print(f"  FAIL {problem}")
    if problems:
        sys.exit(1)
    print(f"  OK closed manifold, {len(edges)} directed edges all paired")
