# roform

`roform` converts Roblox model and place files into render-agnostic GLTF.

## CLI

```sh
roform path/to/model.rbxmx
```

Export a directory and also create GLB files:

```sh
roform path/to/place.rbxlx \
  --out-dir build/roform \
  --assets-dir assets \
  --glb
```

## Output

For default `--out-dir roform`, the pipeline creates the following directory structure:

```text
roform/
├── download/                 # Downloaded Roblox assets
├── mesh/                     # Decoded mesh cache
├── model/                    # GLTF documents
│   ├── <hash>.gltf           # GLTF with materials
│   ├── NM<hash>.gltf         # GLTF without materials if --no-materials is used
│   └── manifest.json         # Source and output mapping
├── bin/                      # GLTF binary buffers
└── glb/                      # Created only when --glb is used
```

## Library

```rust
use std::path::Path;
use roform::{export_glbs, export_models, ModelExportOptions};

fn export() -> Result<(), String> {
    let models = export_models(
        Path::new("place.rbxmx"),
        Path::new("roform/download"),
        Path::new("roform/mesh"),
        Path::new("assets"),
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
