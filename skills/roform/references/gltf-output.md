# Consuming the glTF output

roform glTF documents are render-agnostic: geometry is plain glTF, and all Roblox semantics live in `extras`. Bring your own renderer features (ParticleEmitters, BillboardGuis, surfaces, humanoids) by reading `extras.roblox` and implementing them yourself.

## Document conventions

- The Roblox instance hierarchy is preserved as the glTF node hierarchy.
- `extras.roblox` appears on the document root and on every geometry node/mesh/primitive. Each entry describes its instance _and_ carries its complete recursive child-instance tree in `children`, so script-only instances (lights, emitters, GUIs) stay reachable even though they are never rendered as nodes.
- Models without renderable geometry export as a single empty root node with `extras.roform.hasGeometry: false`; their recursive instance metadata is still in `extras.roblox`.

### `extras.roblox` schema

```json
{
  "className": "Part",
  "name": "Floor",
  "referent": "0x1a2b...",
  "properties": {
    "Anchored": { "Bool": true },
    "Size": { "Vector3": [4.0, 1.0, 2.0] },
    "Color": { "Color3uint8": [153, 51, 51] },
    "Material": { "Enum": 256 },
    "PrimaryPart": { "Ref": "0x9f8e..." }
  },
  "propertyTypes": {
    "Anchored": "Bool",
    "Size": "Vector3",
    "Material": "Enum<Material>"
  },
  "children": [{ "className": "ParticleEmitter", "...": "..." }]
}
```

| Field                  | Always present | Contents                                                                    |
| ---------------------- | -------------- | --------------------------------------------------------------------------- |
| `className`            | yes            | Roblox class name (`Part`, `MeshPart`, `UnionOperation`, …)                 |
| `name`                 | yes            | Instance name                                                               |
| `referent`             | yes            | Stable instance id; use it to resolve `Ref` property values                 |
| `properties`           | yes            | Non-default serialized properties, values wrapped by value-kind tag         |
| `serializedProperties` | when non-empty | Canonical serialized keys (e.g. `Color3uint8` appears as `Color`)           |
| `propertyTypes`        | when non-empty | Property name → declared type name                                          |
| `children`             | when non-empty | All recursive child instances of this instance (geometry children included) |

Value rules:

- Every value in `properties` is a single-key object whose key is the value kind (`Bool`, `String`, `Int32`, `Float32`, `Float64`, `Vector3`, `Color3uint8`, `Ref`, `Enum`, `Content`, …). Never assume raw scalars.
- Properties whose values equal their reflection defaults are omitted entirely.
- Unknown/unrecognized serialized properties are retained rather than dropped.
- Enum properties arrive as integer values — look up their semantic meaning in the official Roblox enum documentation instead of guessing.
- `propertyTypes` values are reflection type names such as `Bool`, `Vector3`, or `Enum<Material>`; use them to disambiguate same-shaped values.

## Geometry availability

Before reading buffers, check:

```js
if (document.extras?.roform?.hasGeometry === false) {
  // meshless scene: no nodes with meshes, no .bin buffer
}
```

Meshless models still carry full `extras.roblox` metadata and export as JSON-only GLBs.

## Materials

- Part color and transparency are always emitted; material _textures_ only when materials were enabled (`--materials-dir` / `includes_materials: true`). Material-enabled outputs are named `M<hash>.gltf`.
- Textures are embedded as data URIs and reference an explicit `REPEAT` sampler.
- **Neon** materials use the glTF `KHR_materials_unlit` extension (listed in `extensionsUsed` whenever present) so they render without lighting or normal-based shading. Handle this extension, or Neon will shade like plastic.

Read the `Material` enum value from `extras.roblox.properties.Material` to pick your own textures. Values map to lowercase names, which are also the filenames used in a materials directory (`<name>_color.png` / `<name>_normal.png`). Read https://create.roblox.com/docs/reference/engine/enums/Material for the full enum table.

## UVs, buffers, and naming

- Buffers live next to models under `bin/<stem>.bin`; GLB files are self-contained.
- Generated flat-primitive UVs use model-space-anchored physical surface projections divided by `studs-per-tile`, so texture axes and phase stay consistent across translated/rotated parts. Cylinder sides follow circumference; spheres follow surface arc lengths.
- Primitives with normal maps include generated tangent space.
- Explicitly textured MeshParts keep their authored mesh UVs; untextured MeshParts get projection fallback UVs.
- Output stems are content/config hashes: `<hash>.gltf` without materials, `M<hash>.gltf` with materials. The GLB stem matches its glTF stem.

## Rendering shapes yourself

If you rebuild primitives from `extras.roblox` instead of using exported geometry, shapes follow the Roblox `PartType` enum:

| Value | Shape       |
| ----- | ----------- |
| 0     | Ball        |
| 1     | Block       |
| 2     | Cylinder    |
| 3     | Wedge       |
| 4     | CornerWedge |

Orientation details (right-handed coordinates):

- Cylinders: axis/height along local X; radius is the smaller of local Y/Z extents; the circle starts along local +Y toward +Z.
- Wedges: extruded along local X; height along local Y; slope descends along local Z.
- CornerWedge: apex at +X/+Y/-Z, rising diagonally toward +X/-Z.

Aim for **2 studs per tile** of material texture for best results.

Legacy surface decorations (Studs, Inlet, Universal, …) are intentionally not rendered. Best-effort recreations are available in [`assets/surface`](https://github.com/Unreal-Works/roform/tree/master/assets/surface) if you want them in your renderer; surface enum values arrive via `extras.roblox.properties` (0 Smooth, 3 Studs, 4 Inlet, 5 Universal, 6 Hinge, 7 Motor, …).

## Manifests

- `model/manifest.json` maps each source file to its output entries (`hash`, absolute `output`, `source`, `name`) plus dependency paths/hashes. Use it to locate outputs programmatically instead of globbing hashes.
- `mesh/manifest.json` maps sources to decoded-mesh cache entries.
- Both retain cached entries for sources outside the current input, so narrowing input scope never evicts unrelated caches.
