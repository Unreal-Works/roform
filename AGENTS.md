# Agent Instructions

Please update this document after any relevant changes.

## Asset Pipeline

- The CLI stages assets as `download`, decoded `mesh`, and hash-keyed `model` GLTF output. Decoded meshes are cached below `roform/mesh/<blake3-of-source-payload>.bin` in the versioned `ROFMESH1` format, with source-to-payload reuse recorded in `roform/mesh/manifest.json` and dependency content hashes cached in `roform/mesh/fingerprint.json`. Without `--materials-dir`, model GLTF files are written below `roform/model/<hash>.gltf`; when `--materials-dir` is provided, they use `roform/model/M<hash>.gltf`. Models with geometry use external buffers with the matching stem in `roform/bin/`; meshless models omit buffers, and source/model names are recorded in `roform/model/manifest.json`.
- Compilation stages default to `mesh,model`; `--compile` accepts comma-separated `mesh`, `model`, and `glb` stages, with model and GLB stages including their required earlier stages. `--no-compile` skips mesh, model, and GLB compilation while downloads still run.
- `--jobs` controls both mhif download concurrency and the maximum number of OS threads used by mesh, model, and GLB compilation; it defaults to mhif's `DEFAULT_DOWNLOAD_JOBS` value, and values below one are treated as one by the workers.
- The public `export_meshes_with_jobs`, `export_models_with_jobs`, and `export_glbs_with_jobs` APIs expose explicit compile worker limits; the original export APIs use the host's available parallelism. Mesh asset IDs are deduplicated before threaded cache writes, and model/GLB output paths are locked independently.
- Including `glb` in `--compile` packages each successful model GLTF, including its buffer and textures when present, below `roform/glb/<hash>.glb` or `roform/glb/M<hash>.glb` after GLTF export. Meshless models produce JSON-only GLBs without a BIN chunk.
- Passing `--recompile` bypasses cached model and GLB outputs while retaining cached downloads and decoded meshes.
- Models with no renderable geometry are exported as meshless scenes with one empty root node and `extras.roform.hasGeometry` set to `false`; their recursive Roblox instance metadata remains in `extras.roblox`.
- Materials are enabled only when `--materials-dir` is provided. Without it, exports retain part colors and transparency but omit material images, samplers, and texture assignments.
- Imported `MeshPart`, `UnionOperation`, and `IntersectOperation` instances recolor decoded geometry from `Color` only when `UsePartColor` is enabled; enabled part color clears authored RGB vertex tint while preserving vertex alpha, and disabled part color leaves decoded mesh colors intact. CSG operations default to disabled, while mesh parts retain their tint when the legacy flag is absent.
- MeshParts without an explicit `TextureID` or `SurfaceAppearance.ColorMap` use model-space physical UV projection for fallback material textures; explicitly textured MeshParts preserve their authored mesh UVs.
- Roblox Neon materials use the GLTF `KHR_materials_unlit` extension so they render without lighting or normal-based shading; the extension is advertised only when a model contains Neon.
- Model export embeds valid images directly from `<materials-dir>/` as data URIs, using Roblox Material enum values from the official enum documentation when mapping material names. Downloaded MeshPart and SurfaceAppearance images are resolved from their staged asset files, including `asset.png`, and referenced by the generated GLTF/GLB. The generated model manifest records absolute source and output paths.
- GLTF `extras.roblox` is populated automatically from the Roblox DOM and reflection database: it includes non-default serialized properties, their types, referents, and recursive child instances on the document root, geometry nodes, and their meshes. Properties matching reflection defaults are omitted; unknown serialized properties are retained without an allowlist. Meshless model documents also include `extras.roform.hasGeometry: false`.
- The model manifest/cache format is versioned; bump the manifest version and model fingerprint discriminator when changing exported GLTF structure so stale files without metadata are regenerated. The manifest stores dependency paths and hashes once in a top-level map, while each source records only the paths that affect it. Source entries record only the downloaded assets and material fallback files that can affect that source, so adding an unrelated fixture does not invalidate existing models.
- The model manifest retains cached entries for sources outside the current input, so switching between a single file and a directory does not force unrelated models to re-export; the current model report remains scoped to the requested input.
- Cache path keys use normalized separators, and model fingerprints use stable source, option, and dependency inputs rather than serialized parsed-model ordering.
- `roform/mesh/fingerprint.json` caches content hashes for referenced dependency files; it is cache state and must not be included in its own dependency fingerprint.
- Generated flat-primitive UVs use model-space-anchored, face-normal-derived physical surface projections divided by `--studs-per-tile`; this keeps texture axes and phase consistent across translated or rotated parts and wedges. Cylinder sides follow circumference, spheres follow surface arc lengths, exported textures reference an explicit GLTF `REPEAT` sampler, and normal-mapped primitives include generated tangent space.
- `Part`, `WedgePart`, and `CornerWedgePart` instances use generated primitive geometry; cylinders use 64 radial segments, while spheres use 128 longitude segments and 64 latitude bands with a shared seam-aware indexed grid so their diagonal facets are no larger than cylinder facets. GLTF uses 16-bit indices when a primitive fits within the unsigned-short range. Curved primitive triangle winding matches outward normals, and ball normals use the ellipsoid surface normal for non-uniform sizes. Legacy `Part.Shape` wedge values use the same generators.

## Module Boundaries

- `main.rs` owns CLI parsing and stage-level progress output.
- `lib.rs` is the documented library entry point; it exposes the pipeline API
  while keeping format and conversion internals private.
- `pipeline.rs` owns model/GLB export orchestration, manifests, fingerprints, and cache reuse.
- `model.rs` converts Roblox DOM instances into renderable model assets and handles downloaded mesh loading.
- `geometry.rs` owns generated primitive topology, normals, and UV projection.
- `metadata.rs` owns Roblox reflection, property conversion, material mapping, and `extras.roblox` generation.
- `csg.rs` owns decoded mesh types, payload decoders, and the versioned decoded-mesh cache format.
- `gltf.rs` owns GLTF/GLB serialization and texture staging.

## Gotchas

1. Roblox uses a right-handed coordinate system.
2. If you see something like <token name="RightSurface">3</token> in a Roblox XML file, this is the enum value. Search its actual semantic meaning in `https://create.roblox.com/docs/reference/engine/enums/*`. Do not guess the meaning of enum values.
3. `rbx_dom_weak::Instance::properties` is keyed by `Ustr`; convert string property names before calling `get`.
4. Roblox XML may serialize `MeshId` as `<Content name="MeshId">`, while `rbx_xml` exposes the canonical property as `MeshContent`.
5. Downloaded CSG operation `.rbxm` packages store geometry in `PartOperationAsset.MeshData` as a `BinaryString`; raw mesh downloads use `asset.bin`.
6. Imported `MeshPart`, `UnionOperation`, and `IntersectOperation` geometry is scaled by `size / InitialSize`; when `InitialSize` is absent, use decoded mesh bounds as the source size.
7. Reflection-enabled Roblox XML canonicalizes the `Color3uint8` XML property to the DOM key `Color`, `TextureID` to `TextureContent`, and SurfaceAppearance `ColorMap`/`NormalMap` to `ColorMapContent`/`NormalMapContent`; retain raw keys as fallbacks for no-reflection input.
8. Generated primitive mesh topology and normals are part of the model export format; bump `MODEL_MANIFEST_VERSION` when changing them so cached GLTF files are regenerated.
