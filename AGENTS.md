# Agent Instructions

Please update this document after any relevant changes.

## Asset Pipeline

- The CLI stages assets as `download`, decoded `mesh`, and hash-keyed `model` GLTF output. Decoded meshes are cached below `roform/mesh/<blake3-of-source-payload>.bin` in the versioned `ROFMESH1` format, with dependency content hashes cached in `roform/mesh/fingerprint.json`. Model GLTF files are written below `roform/model/<hash>.gltf`, or `roform/model/NM<hash>.gltf` with `--no-materials`; their external buffers use the matching stem in `roform/bin/`, and source/model names are recorded in `roform/model/manifest.json`.
- Passing `--glb` packages each successful model GLTF, including its buffer and textures, below `roform/glb/<hash>.glb` or `roform/glb/NM<hash>.glb` after GLTF export.
- Passing `--recompile` bypasses cached model and GLB outputs while retaining cached downloads and decoded meshes.
- Materials are enabled by default. `--no-materials` exports untextured color materials: part colors and transparency are retained, while material images, samplers, and texture assignments are omitted.
- Model export embeds valid images from `assets/material/` as data URIs, using Roblox Material enum values from the official enum documentation when mapping material names. The generated model manifest records absolute source and output paths.
- GLTF `extras.roblox` is populated automatically from the Roblox DOM and reflection database: it includes non-default serialized properties, their types, referents, and recursive child instances on the document root, geometry nodes, and their meshes. Properties matching reflection defaults are omitted; unknown serialized properties are retained without an allowlist.
- The model manifest/cache format is versioned; bump the manifest version and model fingerprint discriminator when changing exported GLTF structure so stale files without metadata are regenerated. The manifest stores dependency paths and hashes once in a top-level map, while each source records only the paths that affect it. Source entries record only the downloaded assets and material fallback files that can affect that source, so adding an unrelated fixture does not invalidate existing models.
- The model manifest retains cached entries for sources outside the current input, so switching between a single file and a directory does not force unrelated models to re-export; the current model report remains scoped to the requested input.
- Cache path keys use normalized separators, and model fingerprints use stable source, option, and dependency inputs rather than serialized parsed-model ordering.
- `roform/mesh/fingerprint.json` caches content hashes for referenced dependency files; it is cache state and must not be included in its own dependency fingerprint.
- Generated flat-primitive UVs use model-space-anchored, face-normal-derived physical surface projections divided by `--studs-per-tile`; this keeps texture axes and phase consistent across translated or rotated parts and wedges. Cylinder sides follow circumference, spheres follow surface arc lengths, exported textures reference an explicit GLTF `REPEAT` sampler, and normal-mapped primitives include generated tangent space.
- `Part`, `WedgePart`, and `CornerWedgePart` instances use generated primitive geometry; legacy `Part.Shape` wedge values use the same generators.

## Module Boundaries

- `main.rs` owns CLI parsing and stage-level progress output.
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
5. Downloaded union `.rbxm` packages store geometry in `PartOperationAsset.MeshData` as a `BinaryString`; raw mesh downloads use `asset.bin`.
6. Imported `MeshPart` and `UnionOperation` geometry is scaled by `size / InitialSize`; when `InitialSize` is absent, use decoded mesh bounds as the source size.
7. Reflection-enabled Roblox XML canonicalizes the `Color3uint8` XML property to the DOM key `Color`; retain the raw key as a fallback for no-reflection input.
8. Generated primitive mesh topology and normals are part of the model export format; bump `MODEL_FORMAT_VERSION` when changing them so cached GLTF files are regenerated.
