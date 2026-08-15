# Agent Instructions

Please update this document after any relevant changes.

## Asset Pipeline

- The CLI stages assets as `download`, decoded `mesh`, and hash-keyed `model` GLTF output. Decoded meshes are cached below `roform/mesh/<blake3-of-source-payload>.bin` in the versioned `ROFMESH1` format, with dependency content hashes cached in `roform/mesh/fingerprint.json`. Model GLTF files are written below `roform/model/<blake3-hash>.gltf`, with their external buffers in `roform/bin/<blake3-hash>.bin` and source/model names recorded in `roform/model/manifest.json`.
- Passing `--glb` packages each successful model GLTF, including its buffer and textures, below `roform/glb/<blake3-hash>.glb` after GLTF export.
- Model export embeds valid images from `assets/material/` as data URIs, using Roblox Material enum values from the official enum documentation when mapping material names. The generated model manifest records absolute source and output paths.

## Gotchas

1. Roblox uses a right-handed coordinate system.
2. If you see something like <token name="RightSurface">3</token> in a Roblox XML file, this is the enum value. Search its actual semantic meaning in `https://create.roblox.com/docs/reference/engine/enums/*`. Do not guess the meaning of enum values.
3. `rbx_dom_weak::Instance::properties` is keyed by `Ustr`; convert string property names before calling `get`.
4. Roblox XML may serialize `MeshId` as `<Content name="MeshId">`, while `rbx_xml` exposes the canonical property as `MeshContent`.
5. Downloaded union `.rbxm` packages store geometry in `PartOperationAsset.MeshData` as a `BinaryString`; raw mesh downloads use `asset.bin`.
6. Imported `MeshPart` and `UnionOperation` geometry is scaled by `size / InitialSize`; when `InitialSize` is absent, use decoded mesh bounds as the source size.
7. Reflection-enabled Roblox XML canonicalizes the `Color3uint8` XML property to the DOM key `Color`; retain the raw key as a fallback for no-reflection input.
