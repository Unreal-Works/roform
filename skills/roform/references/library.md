# Rust library reference

The `roform` crate's root re-exports the whole public API:

```rust
pub use roform::{
    GlbReport, MeshReport, ModelExportOptions, ModelFailure, ModelManifestEntry, ModelReport,
    export_glbs, export_glbs_with_jobs, export_meshes, export_meshes_with_jobs, export_models,
    export_models_with_jobs,
};
```

All functions return `Result<T, String>`. Relative paths resolve against the current working directory; all paths returned in reports and manifests are absolute with forward slashes.

## Prerequisite: downloading assets

The library **never downloads** Roblox assets — the CLI does that for you. When calling the library directly, stage downloads first with the [`mhif`](https://crates.io/crates/mhif) crate (`roform` depends on it):

```rust
use mhif::{DownloadOptions, download_assets, extract_asset_ids_cached};
use std::path::Path;

fn stage(input: &Path, download_dir: &Path) -> Result<(), String> {
    let extraction = extract_asset_ids_cached(input, download_dir).map_err(|e| e.to_string())?;
    let report = download_assets(
        &extraction.asset_ids,
        &DownloadOptions::new(download_dir).with_jobs(4),
    )
    .map_err(|e| e.to_string())?;

    for failure in &report.failed {
        eprintln!("failed {}: {}", failure.asset_id, failure.error);
    }
    Ok(())
}
```

Pass the same `download_dir` to `export_models`/`export_meshes` afterwards.

## Exporting meshes

```rust
pub fn export_meshes(
    input: &Path,
    download_dir: &Path,
    mesh_dir: &Path,
) -> Result<MeshReport, String>;

pub fn export_meshes_with_jobs(
    input: &Path,
    download_dir: &Path,
    mesh_dir: &Path,
    jobs: usize,
) -> Result<MeshReport, String>;
```

Compiles content-addressed decoded-mesh cache entries for every mesh payload referenced by `input`. Useful for pre-warming the cache or splitting stages across processes. Downloads must already exist in `download_dir`.

## Exporting models

```rust
pub fn export_models(
    input: &Path,
    download_dir: &Path,
    mesh_dir: &Path,
    materials_dir: &Path,
    output_dir: &Path,
    options: ModelExportOptions,
) -> Result<ModelReport, String>;

pub fn export_models_with_jobs(
    /* same, plus */
    jobs: usize,
) -> Result<ModelReport, String>;
```

`input` is a single `.rbxm`/`.rbxmx`/`.rbxl`/`.rbxlx` file or a directory searched recursively. `materials_dir` may be any path (even empty) when `options.includes_materials` is `false`; when materials are enabled it should contain `<material>_color.png` / `<material>_normal.png` files.

### `ModelExportOptions`

```rust
let options = ModelExportOptions {
    studs_per_tile: f32,       // default 1.0 in the library (CLI defaults to 2.0!)
    includes_materials: bool,  // default true
    includes_geometry: bool,   // default true
    recompile: bool,           // default false
};
ModelExportOptions::default() // studs_per_tile: 1.0, includes_materials: true, includes_geometry: true, recompile: false
```

- `studs_per_tile`: physical studs per texture tile; must be finite and `> 0`, otherwise the call returns `Err`. Changing it changes every model hash.
- `includes_materials`: emits material definitions/textures and switches geometry-enabled output naming from `<hash>.gltf` to `M<hash>.gltf`.
- `includes_geometry`: emits generated mesh geometry when `true`. When `false`, the GLTF contains only recursive Roblox metadata in an empty root scene, with no meshes or external buffer; the corresponding GLB is JSON-only and both output filenames use an `NG` prefix.
- `recompile`: ignore cached model outputs. Cached downloads and decoded meshes stay reusable.

## Packaging GLBs

```rust
pub fn export_glbs(
    models: &[ModelManifestEntry],
    output_dir: &Path,
    recompile: bool,
) -> Result<GlbReport, String>;

pub fn export_glbs_with_jobs(
    models: &[ModelManifestEntry],
    output_dir: &Path,
    recompile: bool,
    jobs: usize,
) -> Result<GlbReport, String>;
```

Pass the `models` slice from a previous `ModelReport` — its entries carry absolute glTF paths that are read directly. Each GLB embeds the external buffer and any images referenced by its source glTF. Existing GLBs are reused unless `recompile` is `true`.

## Reports

### `MeshReport`

| Field              | Type                | Meaning                                        |
| ------------------ | ------------------- | ---------------------------------------------- |
| `decoded`          | `usize`             | New decoded mesh cache files written this call |
| `cached`           | `usize`             | Decoded mesh cache files reused this call      |
| `failed`           | `Vec<ModelFailure>` | Sources whose meshes could not be decoded      |
| `output_directory` | `PathBuf`           | Absolute mesh cache directory                  |

### `ModelReport`

| Field              | Type                      | Meaning                                        |
| ------------------ | ------------------------- | ---------------------------------------------- |
| `exported`         | `usize`                   | Models written as new glTF files this call     |
| `cached`           | `usize`                   | Models reused from cache this call             |
| `failed`           | `Vec<ModelFailure>`       | Models that could not be exported              |
| `models`           | `Vec<ModelManifestEntry>` | Successful models; feed these to `export_glbs` |
| `output_directory` | `PathBuf`                 | Absolute model output directory                |

### `GlbReport`

| Field              | Type                | Meaning                           |
| ------------------ | ------------------- | --------------------------------- |
| `exported`         | `usize`             | GLB files written this call       |
| `cached`           | `usize`             | GLB files reused this call        |
| `failed`           | `Vec<ModelFailure>` | Models whose GLB packaging failed |
| `output_directory` | `PathBuf`           | Absolute GLB output directory     |

### `ModelManifestEntry`

| Field    | Type     | Meaning                                          |
| -------- | -------- | ------------------------------------------------ |
| `hash`   | `String` | Content/config hash used as the output stem      |
| `output` | `String` | Absolute path to the model glTF file             |
| `source` | `String` | Absolute path to the Roblox source file          |
| `name`   | `String` | Name of the Roblox model represented by the file |

### `ModelFailure`

| Field    | Type     | Meaning                                       |
| -------- | -------- | --------------------------------------------- |
| `source` | `String` | Source file or model name tied to the failure |
| `error`  | `String` | Human-readable failure description            |

## Error handling

Two distinct channels:

1. **Fatal errors** return `Err(String)`: directory creation failures, manifest serialization failures, model write failures, invalid options. Nothing else should be assumed about the run.
2. **Per-source failures** accumulate in `report.failed` while processing continues — a successful result can still list failures. Always iterate them.

Dependency fingerprint-cache write failures are downgraded to warnings because the cache rebuilds on the next call.

## Threading

- The plain `export_*` functions use the host's available parallelism.
- The `*_with_jobs` variants take an explicit limit applied to compile work; values below 1 are treated as 1.
- The same limit does not automatically apply to `mhif` downloads — pass `.with_jobs(n)` there yourself.

## End-to-end example

```rust
use std::path::Path;
use roform::{export_glbs_with_jobs, export_models_with_jobs, ModelExportOptions};

fn main() -> Result<(), String> {
    let input = Path::new("place.rbxmx");
    let download_dir = Path::new("roform/download");
    let mesh_dir = Path::new("roform/mesh");

    // 1. Stage downloads with mhif (see above), then:
    let models = export_models_with_jobs(
        input,
        download_dir,
        mesh_dir,
        Path::new("assets/material"),
        Path::new("roform/model"),
        ModelExportOptions { studs_per_tile: 2.0, ..ModelExportOptions::default() },
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
