# roform

`roform` converts Roblox model and place files into render-agnostic [glTF](https://en.wikipedia.org/wiki/GlTF).

## CLI

```sh
roform path/to/model.rbxmx
```

Export a directory and also create GLB files:

```sh
roform path/to/place.rbxlx \
  --out-dir build/roform \
  --materials-dir assets/material \
  --glb
```

> [!WARNING]
> Roblox does not permit redistribution of Creator Store assets outside of Roblox. Please ensure that you have the right to redistribute any assets you export with `roform`.

## Library

```rust
use std::path::Path;
use roform::{export_glbs, export_models, ModelExportOptions};

fn export() -> Result<(), String> {
    let models = export_models(
        Path::new("place.rbxmx"),
        Path::new("roform/download"),
        Path::new("roform/mesh"),
        Path::new("assets/material"),
        Path::new("roform/model"),
        ModelExportOptions::default(),
    )?;

    for failure in &models.failed {
        eprintln!("{}: {}", failure.source, failure.error);
    }

    let glbs = export_glbs(&models.models, Path::new("roform/glb"), false)?;
    assert!(glbs.failed.is_empty());
    Ok(())
}
```

## Why

Roblox [actually already has a GLTF exporter](https://create.roblox.com/docs/art/modeling/gltf-export) but there are a few reasons it is not suitable for all use cases:
1. Only available in Studio, requiring a GUI, a Windows or macOS machine, etc.
2. Can have restrictive asset exports due to account ownership politics.
3. Does not keep most Instance properties in nodes, effectively rendering the glTF output useless as an intermediate representation.

`roform` gives you your glTF with no strings attached. Shove it into any renderer like Babylon.js, Bevy, and more and you can access the `extras` field for all the properties you could ask for.

## Caveats

The glTF isn't meant to be consumed directly even thought it *can*, due to some fundamental limitations in glTF itself.

Conceptually, we expect you to run the CLI on an input, use your renderer to bring in your own features like ParticleEmitters, BillboardGuis, etc. and use that as your visual output.

### Materials

A `--materials-dir` option is provided if you need it, and you can point it at a directory that looks like this:

```text
<material>_color.png
<material>_normal.png
```

Material names are lowercase, for example `plastic_color.png` and
`plastic_normal.png`.

But you should probably implement materials yourself, as this is meant for static meshes. That is, your material textures will stretch abhorrently the moment you do any transforms on nodes.

The one thing to take in mind when rendering your material textures is that you should project UVs from world-space so texture orientation and phase are consistent, mirroring how Roblox does it.

### Unsupported Instances

We don't implement:
- surfaces like Studs, Inlet, Universal on purpose due to optimization reasons
- humanoids & clothing
- decals
- like literally everything that isnt a part, union or meshpart

You can always make a pull request if you think any of these are worth rendering.

## Output

For default `--out-dir roform`, the pipeline creates the following directory structure:

```text
roform/
├── download/                 # Downloaded Roblox assets
├── mesh/                     # Decoded mesh cache
├── model/                    # glTF documents
│   ├── M<hash>.gltf          # glTF with materials if --materials-dir is used
│   ├── <hash>.gltf           # glTF without materials when --materials-dir is omitted
│   └── manifest.json         # Source and output mapping
├── bin/                      # glTF binary buffers
└── glb/                      # Created only when --glb is used
```
