---
name: roform
description: "Use this skill when converting Roblox .rbxm, .rbxmx, .rbxl, or .rbxlx files into glTF or GLB with the roform CLI or Rust library. Covers installing via npm, CLI flags (--out-dir, --compile, --no-compile, --materials-dir, --recompile, --jobs, --studs-per-tile), the output directory layout, cache reuse, the Rust export API (export_meshes, export_models, export_glbs and their *_with_jobs variants), ModelExportOptions, report types, staging asset downloads with mhif, reading extras.roblox metadata, material texture directories, and consuming the exported models in a renderer. Triggers on: roform, rbxm, rbxmx, rbxl, rbxlx, Roblox to glTF, glTF export, GLB export, export_models, export_glbs, ModelExportOptions, extras.roblox, KHR_materials_unlit."
license: MIT
---

`roform` converts Roblox model and place files into render-agnostic [glTF](https://en.wikipedia.org/wiki/GlTF). Use the CLI for one-off exports or the Rust library inside a larger pipeline. Every Roblox Instance property survives in glTF `extras`, so your renderer can rebuild features like ParticleEmitters and BillboardGuis itself.

> [!WARNING]
> Roblox does not permit redistribution of Creator Store assets outside of Roblox. Make sure you have the right to redistribute any assets you export.

## Quick Start

```sh
npm install -D @unrealworks/roform

# Convert a single model file (output lands in ./roform/)
npx roform path/to/model.rbxmx

# Convert a whole place with materials and GLB packaging
roform path/to/place.rbxlx \
  --out-dir build/roform \
  --materials-dir assets/material \
  --compile mesh,model,glb \
  --jobs 4
```

Output layout for `--out-dir build/roform`:

```text
build/roform/
├── download/                 # staged Roblox asset downloads
├── mesh/                     # decoded mesh cache (<hash>.bin + manifest.json)
├── model/                    # glTF documents (+ manifest.json)
│   └── M<hash>.gltf          # M prefix when --materials-dir is set
├── bin/                      # external glTF binary buffers
└── glb/                      # only created when glb is in --compile
```

Rust library:

```rust
use std::path::Path;
use roform::{export_glbs_with_jobs, export_models_with_jobs, ModelExportOptions};

fn export() -> Result<(), String> {
    let models = export_models_with_jobs(
        Path::new("place.rbxmx"),
        Path::new("roform/download"), // must already contain downloaded assets
        Path::new("roform/mesh"),
        Path::new("assets/material"),
        Path::new("roform/model"),
        ModelExportOptions::default(),
        4,
    )?;

    for failure in &models.failed {
        eprintln!("{}: {}", failure.source, failure.error);
    }

    let glbs = export_glbs_with_jobs(&models.models, Path::new("roform/glb"), false, 4)?;
    assert!(glbs.failed.is_empty());
    Ok(())
}
```

The library never downloads assets; stage them yourself with the `mhif` crate first. See [references/library.md](references/library.md).

## Supported inputs

| Input              | Behavior                                                                  |
| ------------------ | ------------------------------------------------------------------------- |
| `.rbxm` / `.rbxmx` | Converts that one model file                                              |
| `.rbxl` / `.rbxlx` | Converts that one place file                                              |
| A directory        | Recursively converts every `.rbxm` / `.rbxmx` / `.rbxl` / `.rbxlx` inside |

## CLI flags

| Flag                        | Default            | Effect                                                                                    |
| --------------------------- | ------------------ | ----------------------------------------------------------------------------------------- |
| `INPUT` (positional)        | required           | File or directory to scan for Roblox sources                                              |
| `--out-dir DIRECTORY`       | `./roform`         | Root for all stages' output and caches                                                    |
| `--compile STAGE[,STAGE…]`  | `mesh,model`       | Stages to run after download: `mesh`, `model`, `glb`. Later stages imply earlier ones     |
| `--no-compile`              | off                | Skip all compilation; downloads still run. Conflicts with `--compile`                     |
| `--materials-dir DIRECTORY` | off                | Enables material textures; without it exports keep part colors/transparency but no images |
| `--recompile`               | off                | Ignore cached model and GLB outputs; cached downloads and decoded meshes are still reused |
| `--jobs N`                  | downloader default | Max concurrent downloads **and** max compile threads; values below 1 are treated as 1     |
| `--studs-per-tile STUDS`    | `2.0`              | Physical studs represented by one texture tile; must be finite and greater than zero      |

Compile-stage implications:

| Invocation        | mesh | model | glb |
| ----------------- | ---- | ----- | --- |
| (default)         | yes  | yes   | no  |
| `--compile mesh`  | yes  | no    | no  |
| `--compile model` | yes  | yes   | no  |
| `--compile glb`   | yes  | yes   | yes |
| `--no-compile`    | no   | no    | no  |

Each stage prints one summary line; per-source failures go to stderr as `failed <stage> <source>: <error>` without aborting the run:

```text
download: fetched 12, reused 3, failed 0 -> Z:\proj\roform\download in 1.23s
mesh: decoded 5, reused 8, failed 0 -> Z:\proj\roform\mesh in 0.45s
model: exported 4, reused 9, failed 0 -> Z:\proj\roform\model in 2.10s
glb: exported 4, reused 0, failed 0 -> Z:\proj\roform\glb in 0.30s
```

## What is inside the glTF

- `extras.roblox` on the document root, every geometry node, and its meshes carries the full instance tree: `className`, `name`, `referent`, typed `properties`, `propertyTypes`, recursive `children`.
- Models with no renderable geometry export as scenes with `extras.roform.hasGeometry: false` and no buffers.
- Neon materials use the glTF `KHR_materials_unlit` extension so they render without lighting.
- Textures are embedded as data URIs; buffers are external `<stem>.bin` files under `bin/` (GLBs are fully self-contained).

See [references/gltf-output.md](references/gltf-output.md) for the exact schema.

## Topics

Every workflow has a reference file:

| Topic                     | Reference                                              | When                                                      |
| ------------------------- | ------------------------------------------------------ | --------------------------------------------------------- |
| CLI deep dive             | [references/cli.md](references/cli.md)                 | Flag details, caching rules, recipes, CI usage            |
| Rust library              | [references/library.md](references/library.md)         | Full API, downloading assets with `mhif`, reports, errors |
| Consuming the glTF output | [references/gltf-output.md](references/gltf-output.md) | `extras.roblox` schema, materials, UVs, shapes, manifests |

## Decision guide

- **One-off conversion?** `npx roform input.rbxmx`. Done.
- **Need single-file binaries for an engine import pipeline?** Add `glb` to `--compile`.
- **Your renderer implements materials itself?** Omit `--materials-dir`; read part color from `extras.roblox`.
- **Only want downloads staged for later processing?** Pass `--no-compile`.
- **Edited a source file?** Just rerun the same command; caches re-export only what changed.
- **Suspect stale output after upgrading roform?** Rerun with `--recompile` (cache formats are versioned and self-invalidate).
- **Embedding in Rust?** Use the library API; see [references/library.md](references/library.md).

## Common Mistakes

### [CRITICAL] Treating exit code 0 as "everything converted"

Wrong:

```sh
roform place.rbxlx && echo "all good"
```

Correct:

```sh
roform place.rbxlx 2>stderr.log || exit 1
grep '^failed' stderr.log && echo "some sources failed" >&2
```

Per-source failures (a bad MeshPart, a failed download) print `failed ...` lines to stderr while the rest of the input continues; only fatal errors (bad input path, unwritable output) produce a nonzero exit code. In the library, always iterate `report.failed`.

### [CRITICAL] Expecting GLB files without asking for them

Wrong:

```sh
roform place.rbxlx
ls roform/glb # empty — glb stage did not run
```

Correct:

```sh
roform place.rbxlx --compile mesh,model,glb
```

### [HIGH] Expecting material textures without `--materials-dir`

Without the flag, exports keep part colors and transparency but omit material images, samplers, and texture assignments. In the library, that means `ModelExportOptions { includes_materials: true, .. }` plus a directory that actually contains `<material>_color.png` files.

### [HIGH] Reading `extras.roblox.properties` values as raw JSON scalars

Property values are typed wrappers keyed by value kind, not plain JSON:

```json
{ "Anchored": { "Bool": true }, "PrimaryPart": { "Ref": "0x..." } }
```

Use `propertyTypes` to learn each property's declared type, and remember properties at their reflection-default values are omitted entirely.

### [HIGH] Assuming every glTF document has geometry

Meshless models (e.g. folders, script-only models) have no nodes with meshes and no buffer files. Check `extras.roform.hasGeometry === false` before touching buffers.

### [MEDIUM] Deleting the output directory between runs

The `download/`, `mesh/`, and `model/` directories _are_ the incremental cache. Wiping them forces full re-downloads and re-decodes. To refresh only derived output, use `--recompile`.

### [MEDIUM] Being surprised by option-driven cache misses

Changing `--studs-per-tile`, adding/removing `--materials-dir`, or editing any referenced file changes the model hash, so affected models re-export even though nothing else changed. This is intentional fingerprinting, not a bug.

## API Reference

- [github.com/Unreal-Works/roform](https://github.com/Unreal-Works/roform) — source, README, sample surface assets (`assets/surface`) and material textures (`assets/material`)
