# CLI reference

## Synopsis

```text
roform INPUT [--out-dir DIRECTORY] [--compile STAGE[,STAGE...]] [--no-compile]
             [--no-geometry] [--materials-dir DIRECTORY]
             [--recompile] [--jobs N] [--studs-per-tile STUDS]
```

## Installation

```sh
# npm (recommended; ships prebuilt binaries)
npm install -D @unrealworks/roform

# then run via the local bin
npx roform --help
```

## Pipeline stages

Stages always run in this order; each later stage depends on the earlier one:

```text
download -> mesh -> model -> glb
```

- **download**: fetches every asset id referenced by the input (MeshPart meshes, textures, CSG operation packages) into `<out-dir>/download/`, keyed by asset id. Always runs, even with `--no-compile`.
- **mesh**: decodes downloaded mesh payloads into the versioned `ROFMESH1` cache under `<out-dir>/mesh/<blake3-of-payload>.bin`.
- **model**: writes one glTF document per Roblox source file under `<out-dir>/model/`, with external buffers under `<out-dir>/bin/`. The filename stem is the content/config hash; a `M` prefix marks material-enabled output and an `NG` prefix marks no mesh or buffer output.
- **glb**: packages each model glTF (plus its buffer and textures) into `<out-dir>/glb/<stem>.glb`. Meshless models become JSON-only GLBs without a BIN chunk.

## Console output

One summary line per stage:

```text
download: fetched 12, reused 3, failed 0 -> Z:\proj\roform\download in 1.23s
mesh: decoded 5, reused 8, failed 0 -> Z:\proj\roform\mesh in 0.45s
model: exported 4, reused 9, failed 0 -> Z:\proj\roform\model in 2.10s
glb: exported 4, reused 0, failed 0 -> Z:\proj\roform\glb in 0.30s
```

- `fetched`/`decoded`/`exported`: items written during this run.
- `reused`/`cached`: items served from cache.
- `failed`: items that failed; each also prints a detail line to stderr:
  - downloads: `failed <assetId>: <error>`
  - mesh/model stages: `failed mesh <source>: <error>` / `failed model <source>: <error>`
  - GLB stage: `failed GLB <source>: <error>` (uppercase `GLB`)

Exit codes: `0` when the pipeline completed (even with per-source failures), `1` on a fatal error printed as `error: <message>` (missing input, unwritable output, invalid flag values). In scripts, check both the exit code and stderr for `^failed` lines.

## Caching rules

| Layer       | Cache key                                      | Invalidated by                                                                     |
| ----------- | ---------------------------------------------- | ---------------------------------------------------------------------------------- |
| `download/` | asset id                                       | nothing (reused forever; new ids are fetched on demand)                            |
| `mesh/`     | blake3 hash of the downloaded mesh payload     | only new/changed payloads; unchanged sources skip DOM parsing entirely             |
| `model/`    | fingerprint of source + dependencies + options | edits, changed `--studs-per-tile`/`--materials-dir`/geometry mode, roform upgrades |
| `glb/`      | existence of the output file                   | `--recompile` or deleting the file                                                 |

- `--recompile` regenerates model glTFs and GLBs but still reuses cached downloads and decoded meshes.
- Cache formats are versioned; upgrading roform to a version that changed output structure regenerates affected files automatically.
- Switching between a single file and a directory input does not evict other cached models; the run report only covers the requested input.
- `mesh/manifest.json` and `model/manifest.json` are human-inspectable source↔output maps. `mesh/fingerprint.json` is internal cache state; do not edit or template it.

## Materials directory

Point `--materials-dir` at a directory of lowercase material textures:

```text
assets/material/
├── plastic_color.png
├── plastic_normal.png
├── grass_color.png
└── ...
```

- Each material may provide `<material>_color.png` and `<material>_normal.png`; missing or invalid images are simply skipped.
- Valid names are the lowercase Roblox material names (`plastic`, `neon`, `wood`, `grass`, …). The full enum↔name table is in [gltf-output.md](gltf-output.md).
- MeshPart and SurfaceAppearance textures that were downloaded from Roblox resolve from the staged download files (including `asset.png`) and do not come from this directory.
- Explicitly textured MeshParts keep their authored UVs; untextured MeshParts and flat primitives get physical surface UV projections divided by `--studs-per-tile` (2 studs per tile is the recommended value).

## Recipes

### First full export of a place

```sh
roform maps/lobby.rbxlx \
  --out-dir build/roform \
  --materials-dir assets/material \
  --compile mesh,model,glb \
  --jobs 8
```

### Iterate on one model after editing it

```sh
roform maps/lobby.rbxlx # same command; only changed sources re-export
```

### Refresh models after a roform upgrade or option change

```sh
roform maps/lobby.rbxlx --recompile --compile mesh,model,glb
```

### Stage downloads only (custom downstream processing)

```sh
roform maps/lobby.rbxlx --no-compile
# consume roform/download/ yourself, or call the library API
```

### CI

- Pin the version so cache fingerprints stay stable.
- Pass `--jobs` explicitly for predictable parallelism.
- Treat a nonzero exit code as an infra failure; grep stderr for `^failed` to detect per-source failures.
- Cache the whole `--out-dir` between builds; it is designed for incremental reuse.
