# Agent Instructions

Please update this document after any relevant changes.

## Asset Pipeline

- The CLI stages assets as `download`, decoded `mesh`, and per-source-model `model` GLTF output. Model GLTF files are written below `roform/model/<source>/`.
- Model export embeds valid images from `assets/material/` as data URIs, using Roblox Material enum values from the official enum documentation when mapping material names.

## Gotchas

1. Roblox uses a right-handed coordinate system.
2. If you see something like <token name="RightSurface">3</token> in a Roblox XML file, this is the enum value. Search its actual semantic meaning in `https://create.roblox.com/docs/reference/engine/enums/*`. Do not guess the meaning of enum values.
