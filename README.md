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
  --compile mesh,model,glb \
  --jobs 4
```

> [!WARNING]
> Roblox does not permit redistribution of Creator Store assets outside of Roblox. Please ensure that you have the right to redistribute any assets you export with `roform`.

## Library

```rust
use std::path::Path;
use roform::{export_glbs_with_jobs, export_models_with_jobs, ModelExportOptions};

fn export() -> Result<(), String> {
    let models = export_models_with_jobs(
        Path::new("place.rbxmx"),
        Path::new("roform/download"),
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

## Why

Roblox [actually already has a GLTF exporter](https://create.roblox.com/docs/art/modeling/gltf-export) but there are a few reasons it is not suitable for all use cases:

1. Only available in Studio, requiring a GUI, a Windows or macOS machine, etc.
2. Can have restrictive asset exports due to account ownership politics.
3. Does not keep most Instance properties in nodes, effectively rendering the glTF output useless as an intermediate representation.

`roform` gives you your glTF with **no strings attached.** Shove it into any renderer like Babylon.js, Bevy, and more and you can access the `extras` field for all the properties you could ask for.

## Strings Attached

The glTF isn't meant to be consumed directly even thought it _can_, due to some fundamental limitations in glTF itself.

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
├── mesh/                     # Decoded mesh cache and source manifest
│   ├── <hash>.bin            # ROFMESH1 decoded payload
│   └── manifest.json         # Source-to-mesh cache mapping
├── model/                    # glTF documents when model compilation is selected
│   ├── M<hash>.gltf          # glTF with materials if --materials-dir is used
│   ├── <hash>.gltf           # glTF without materials when --materials-dir is omitted
│   └── manifest.json         # Source and output mapping
├── bin/                      # glTF binary buffers
└── glb/                      # Created only when glb compilation is selected
```

## Hand-writing Your Rendering Implementation

### Blocks

1. Shapes are determined by a PartType enum:
   - 0: Ball
   - 1: Block
   - 2: Cylinder
   - 3: Wedge
   - 4: CornerWedge

### Cylinders, Wedges & CornerWedges

1. Cylinders are defined as:
   - Axis / height: local X.
   - Radius: local Y-Z plane, whichever is smaller.
   - Radial orientation: circle starts along local +Y and proceeds toward +Z.
2. Wedges are extruded along the local X axis. Its slope lies in the Y-Z plane:
   - height: local Y.
   - Slope direction: local Z.
   - X has no slope, it is the wedge's constant-width/extrusion axis.
3. For CornerWedge, the apex is +X, +Y, -Z, so it rises diagonally toward +X/-Z.
4. If the coloring looks off compared to blocks, try disabling material textures first to see if UVs are affecting it. If the issue persists, you implemented the normals wrong.

### Materials

1. Project UVs from world-space so texture orientation and phase are consistent.
2. Enums for each Material:
   - 256: plastic
   - 272: smoothplastic
   - 288: neon
   - 512: wood
   - 528: woodplanks
   - 784: marble
   - 788: basalt
   - 800: slate
   - 804: crackedlava
   - 816: concrete
   - 820: limestone
   - 832: granite
   - 836: pavement
   - 848: brick
   - 864: pebble
   - 880: cobblestone
   - 896: rock
   - 912: sandstone
   - 1040: corrodedmetal
   - 1056: diamondplate
   - 1072: foil
   - 1088: metal
   - 1280: grass
   - 1284: leafygrass
   - 1296: sand
   - 1312: fabric
   - 1328: snow
   - 1344: mud
   - 1360: ground
   - 1376: asphalt
   - 1392: salt
   - 1536: ice
   - 1552: glacier
   - 1568: glass
   - 1584: forcefield
   - 1792: air
   - 2048: water
   - 2304: cardboard
   - 2305: carpet
   - 2306: ceramictiles
   - 2307: clayrooftiles
   - 2308: roofshingles
   - 2309: leather
   - 2310: plaster
   - 2311: rubber
3. You should aim for 2 studs per tile of material texture for best results.

### Surfaces

1. Surfaces are defined as:
   - 0: Smooth. Adds no details on the surface.
   - 1: Glue. Adds an "X" pattern across the surface.
   - 2: Weld. Adds an "X" pattern across the surface.
   - 3: Studs. Adds square studs across the surface.
   - 4: Inlet. Adds square holes across the surface where studs would be.
   - 5: Universal. Adds a checker pattern to the surface using studs and inlets.
   - 6: Hinge. Adds a yellow 0.2 radius, 0.5 length cylinder hinge to the surface.
   - 7: Motor. Functionally identical to a hinge with the addition of a grey ring.
   - 8: SteppingMotor. Functionally identical to a motor.
   - 10: SmoothNoOutlines. Functionally identical to Smooth.
2. You can [take these best-effort surface asset recreations](https://github.com/Unreal-Works/roform/tree/master/assets/surface) and use them in your renderer to approximate the surface effects.
