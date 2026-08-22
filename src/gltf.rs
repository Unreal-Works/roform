use crate::{
    csg::UnionMesh,
    geometry,
    model::{ModelAsset, ModelMaterial},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) const NO_RENDERABLE_GEOMETRY: &str = "model contains no renderable geometry";

pub(crate) fn model_to_gltf(
    model: &ModelAsset,
    download_dir: &Path,
    materials_dir: &Path,
    gltf_output_dir: &Path,
    asset_output_dir: &Path,
    buffer_output_path: &Path,
    include_textures: bool,
) -> Result<Vec<u8>, String> {
    if model.primitives.is_empty() {
        return Err(NO_RENDERABLE_GEOMETRY.to_owned());
    }

    let vertex_stride = 36usize;
    let mut binary = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut gltf_materials = Vec::new();
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let samplers = vec![json!({ "wrapS": 10497, "wrapT": 10497 })];
    let mut material_indices = HashMap::<String, usize>::new();
    let mut image_indices = HashMap::<PathBuf, usize>::new();
    let mut material_export = MaterialExport {
        download_dir,
        materials_dir,
        gltf_output_dir,
        asset_output_dir,
        material_indices: &mut material_indices,
        materials: &mut gltf_materials,
        include_textures,
        image_indices: &mut image_indices,
        images: &mut images,
        textures: &mut textures,
    };

    for primitive in &model.primitives {
        let material_index = material_export.material_index(&primitive.material)?;
        let uses_normal_texture = material_export.materials[material_index]
            .get("normalTexture")
            .is_some();
        let vertex_offset = binary.len();
        for vertex in &primitive.mesh.vertices {
            for value in vertex.position {
                binary.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.normal {
                binary.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.tex_coord {
                binary.extend_from_slice(&value.to_le_bytes());
            }
            binary.extend_from_slice(&vertex.color);
        }
        let vertex_length = binary.len() - vertex_offset;
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": vertex_offset,
            "byteLength": vertex_length,
            "byteStride": vertex_stride,
            "target": 34962
        }));
        let vertex_view = buffer_views.len() - 1;

        let tangent_view = if uses_normal_texture {
            pad_to_four(&mut binary, 0);
            let tangent_offset = binary.len();
            for tangent in geometry::mesh_tangents(&primitive.mesh) {
                for value in tangent {
                    binary.extend_from_slice(&value.to_le_bytes());
                }
            }
            let tangent_length = binary.len() - tangent_offset;
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": tangent_offset,
                "byteLength": tangent_length,
                "target": 34962
            }));
            Some(buffer_views.len() - 1)
        } else {
            None
        };

        pad_to_four(&mut binary, 0);
        let index_offset = binary.len();
        let use_unsigned_short_indices = primitive
            .mesh
            .indices
            .iter()
            .all(|index| *index <= u16::MAX as u32);
        if use_unsigned_short_indices {
            for index in &primitive.mesh.indices {
                binary.extend_from_slice(
                    &u16::try_from(*index)
                        .map_err(|_| "mesh index does not fit in an unsigned short".to_owned())?
                        .to_le_bytes(),
                );
            }
        } else {
            for index in &primitive.mesh.indices {
                binary.extend_from_slice(&index.to_le_bytes());
            }
        }
        let index_length = binary.len() - index_offset;
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": index_offset,
            "byteLength": index_length,
            "target": 34963
        }));
        let index_view = buffer_views.len() - 1;
        pad_to_four(&mut binary, 0);
        let (min_position, max_position) = position_bounds(&primitive.mesh);

        let position_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": vertex_view,
            "byteOffset": 0,
            "componentType": 5126,
            "count": primitive.mesh.vertices.len(),
            "type": "VEC3",
            "min": min_position.0,
            "max": max_position.0
        }));
        let normal_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": vertex_view,
            "byteOffset": 12,
            "componentType": 5126,
            "count": primitive.mesh.vertices.len(),
            "type": "VEC3"
        }));
        let tex_coord_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": vertex_view,
            "byteOffset": 24,
            "componentType": 5126,
            "count": primitive.mesh.vertices.len(),
            "type": "VEC2"
        }));
        let color_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": vertex_view,
            "byteOffset": 32,
            "componentType": 5121,
            "normalized": true,
            "count": primitive.mesh.vertices.len(),
            "type": "VEC4"
        }));
        let tangent_accessor = tangent_view.map(|tangent_view| {
            let tangent_accessor = accessors.len();
            accessors.push(json!({
                "bufferView": tangent_view,
                "componentType": 5126,
                "count": primitive.mesh.vertices.len(),
                "type": "VEC4"
            }));
            tangent_accessor
        });
        let index_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": index_view,
            "componentType": if use_unsigned_short_indices { 5123 } else { 5125 },
            "count": primitive.mesh.indices.len(),
            "type": "SCALAR"
        }));

        let mut attributes = json!({
            "POSITION": position_accessor,
            "NORMAL": normal_accessor,
            "TEXCOORD_0": tex_coord_accessor,
            "COLOR_0": color_accessor
        });
        if let Some(tangent_accessor) = tangent_accessor {
            attributes["TANGENT"] = json!(tangent_accessor);
        }
        let mut primitive_json = json!({
            "attributes": attributes,
            "indices": index_accessor,
            "mode": 4
        });
        primitive_json["material"] = json!(material_index);
        let mesh_index = meshes.len();
        meshes.push(json!({
            "name": primitive.name.as_str(),
            "extras": { "roblox": primitive.extras },
            "primitives": [primitive_json]
        }));
        nodes.push(json!({
            "name": primitive.name.as_str(),
            "mesh": mesh_index,
            "matrix": primitive.matrix,
            "extras": { "roblox": primitive.extras }
        }));
    }

    let binary_uri = relative_uri(gltf_output_dir, buffer_output_path)?;
    let mut json_value = json!({
        "asset": { "version": "2.0", "generator": "roform" },
        "extras": { "roblox": model.extras },
        "buffers": [{ "byteLength": binary.len(), "uri": binary_uri }],
        "bufferViews": buffer_views,
        "accessors": accessors,
        "meshes": meshes,
        "nodes": nodes,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "scene": 0
    });
    if gltf_materials.iter().any(|material| {
        material
            .get("extensions")
            .and_then(|extensions| extensions.get("KHR_materials_unlit"))
            .is_some()
    }) {
        json_value["extensionsUsed"] = json!(["KHR_materials_unlit"]);
    }
    if include_textures {
        json_value["images"] = json!(images);
        json_value["samplers"] = json!(samplers);
        json_value["textures"] = json!(textures);
    }
    json_value["materials"] = json!(gltf_materials);
    let gltf = serde_json::to_vec_pretty(&json_value)
        .map_err(|error| format!("failed to serialize glTF: {error}"))?;
    write_binary_buffer(buffer_output_path, &binary)?;
    Ok(gltf)
}

pub(crate) fn gltf_to_glb(gltf: &[u8], gltf_path: &Path) -> Result<Vec<u8>, String> {
    let mut document: Value = serde_json::from_slice(gltf)
        .map_err(|error| format!("failed to parse GLTF {}: {error}", gltf_path.display()))?;
    let gltf_dir = gltf_path.parent().ok_or_else(|| {
        format!(
            "cannot resolve GLTF resources without a parent directory: {}",
            gltf_path.display()
        )
    })?;

    let buffer_uri = document
        .get("buffers")
        .and_then(Value::as_array)
        .and_then(|buffers| buffers.first())
        .and_then(|buffer| buffer.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| "GLTF does not contain an external buffer URI".to_owned())?
        .to_owned();
    let mut binary = read_external_resource(&buffer_uri, gltf_dir)?;

    let buffer = document
        .get_mut("buffers")
        .and_then(Value::as_array_mut)
        .and_then(|buffers| buffers.first_mut())
        .ok_or_else(|| "GLTF does not contain a buffer".to_owned())?;
    let buffer_object = buffer
        .as_object_mut()
        .ok_or_else(|| "GLTF buffer is not an object".to_owned())?;
    buffer_object.remove("uri");

    let image_count = document
        .get("images")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    for image_index in 0..image_count {
        let image_uri = document["images"][image_index]
            .get("uri")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(image_uri) = image_uri else {
            continue;
        };
        let image_bytes = read_external_resource(&image_uri, gltf_dir)?;
        pad_to_four(&mut binary, 0);
        let byte_offset = binary.len();
        let byte_length = image_bytes.len();
        binary.extend_from_slice(&image_bytes);

        if document.get("bufferViews").is_none() {
            document["bufferViews"] = json!([]);
        }
        let buffer_view_index = document["bufferViews"].as_array().map_or(0, Vec::len);
        document["bufferViews"]
            .as_array_mut()
            .ok_or_else(|| "GLTF bufferViews is not an array".to_owned())?
            .push(json!({
                "buffer": 0,
                "byteOffset": byte_offset,
                "byteLength": byte_length
            }));
        let image = document["images"][image_index]
            .as_object_mut()
            .ok_or_else(|| "GLTF image is not an object".to_owned())?;
        image.remove("uri");
        image.insert("bufferView".to_owned(), json!(buffer_view_index));
    }

    document["buffers"][0]["byteLength"] = json!(binary.len());

    let mut json_chunk = serde_json::to_vec(&document)
        .map_err(|error| format!("failed to serialize GLB JSON: {error}"))?;
    pad_to_four(&mut json_chunk, b' ');
    pad_to_four(&mut binary, 0);

    let total_length = 12usize
        .checked_add(8)
        .and_then(|length| length.checked_add(json_chunk.len()))
        .and_then(|length| length.checked_add(8))
        .and_then(|length| length.checked_add(binary.len()))
        .ok_or_else(|| "GLB is too large".to_owned())?;
    let total_length = u32::try_from(total_length).map_err(|_| "GLB is too large".to_owned())?;
    let json_length =
        u32::try_from(json_chunk.len()).map_err(|_| "GLB JSON is too large".to_owned())?;
    let binary_length =
        u32::try_from(binary.len()).map_err(|_| "GLB binary chunk is too large".to_owned())?;

    let mut glb = Vec::with_capacity(total_length as usize);
    glb.extend_from_slice(&0x46546c67u32.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&total_length.to_le_bytes());
    glb.extend_from_slice(&json_length.to_le_bytes());
    glb.extend_from_slice(&0x4e4f534au32.to_le_bytes());
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&binary_length.to_le_bytes());
    glb.extend_from_slice(&0x004e4942u32.to_le_bytes());
    glb.extend_from_slice(&binary);
    Ok(glb)
}

fn read_external_resource(uri: &str, base_dir: &Path) -> Result<Vec<u8>, String> {
    if uri.starts_with("data:") || uri.contains("://") {
        return Err(format!(
            "GLB conversion only supports local external resources, got {uri}"
        ));
    }
    let path = base_dir.join(uri);
    fs::read(&path)
        .map_err(|error| format!("failed to read GLTF resource {}: {error}", path.display()))
}

fn write_binary_buffer(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create GLTF buffer directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, bytes)
        .map_err(|error| format!("failed to write GLTF buffer {}: {error}", path.display()))
}

struct MaterialExport<'a> {
    download_dir: &'a Path,
    materials_dir: &'a Path,
    gltf_output_dir: &'a Path,
    asset_output_dir: &'a Path,
    material_indices: &'a mut HashMap<String, usize>,
    materials: &'a mut Vec<serde_json::Value>,
    include_textures: bool,
    image_indices: &'a mut HashMap<PathBuf, usize>,
    images: &'a mut Vec<serde_json::Value>,
    textures: &'a mut Vec<serde_json::Value>,
}

impl MaterialExport<'_> {
    fn material_index(&mut self, material: &ModelMaterial) -> Result<usize, String> {
        let key = format!(
            "{}:{:?}:{:?}:{:?}",
            material.name, material.color, material.base_color_asset, material.normal_asset
        );
        if let Some(index) = self.material_indices.get(&key) {
            return Ok(*index);
        }

        let (base_color, normal) = if self.include_textures {
            (
                self.material_image(material, true)?,
                self.material_image(material, false)?,
            )
        } else {
            (None, None)
        };
        let mut pbr = json!({
            "baseColorFactor": material.color,
            "metallicFactor": 0.0,
            "roughnessFactor": 0.8
        });
        if let Some(image_index) = base_color {
            pbr["baseColorTexture"] = json!({ "index": image_index });
        }
        let mut material_json = json!({
            "name": material.name.as_str(),
            "pbrMetallicRoughness": pbr,
            "doubleSided": true
        });
        if material.name == "neon" {
            material_json["extensions"] = json!({ "KHR_materials_unlit": {} });
        }
        if let Some(image_index) = normal {
            material_json["normalTexture"] = json!({ "index": image_index });
        }
        if material.color[3] < 1.0 {
            material_json["alphaMode"] = json!("BLEND");
        }

        let index = self.materials.len();
        self.materials.push(material_json);
        self.material_indices.insert(key, index);
        Ok(index)
    }

    fn material_image(
        &mut self,
        material: &ModelMaterial,
        base_color: bool,
    ) -> Result<Option<usize>, String> {
        let (asset_id, fallback_name) = if base_color {
            (
                material.base_color_asset.as_deref(),
                format!("{}_color.png", material.name),
            )
        } else {
            (
                material.normal_asset.as_deref(),
                format!("{}_normal.png", material.name),
            )
        };
        let asset_path = if let Some(asset_id) = asset_id {
            downloaded_image_path(self.download_dir, asset_id)?
        } else {
            None
        };
        let fallback_path = self.materials_dir.join(fallback_name);
        let source_path = asset_path.or_else(|| fallback_path.is_file().then_some(fallback_path));
        let Some(source_path) = source_path else {
            return Ok(None);
        };

        if let Some(index) = self.image_indices.get(&source_path) {
            return Ok(Some(*index));
        }
        let bytes = fs::read(&source_path).map_err(|error| {
            format!("failed to read texture {}: {error}", source_path.display())
        })?;
        let Some(mime_type) = image_mime_type(&bytes) else {
            return Ok(None);
        };
        let output_path = stage_asset(&source_path, self.materials_dir, self.asset_output_dir)?;
        let uri = relative_uri(self.gltf_output_dir, &output_path)?;
        let image_index = self.images.len();
        self.images.push(json!({
            "name": output_path.file_name().and_then(|name| name.to_str()),
            "mimeType": mime_type,
            "uri": uri
        }));
        let texture_index = self.textures.len();
        self.textures
            .push(json!({ "sampler": 0, "source": image_index }));
        self.image_indices.insert(source_path, texture_index);
        Ok(Some(texture_index))
    }
}

fn downloaded_image_path(download_dir: &Path, asset_id: &str) -> Result<Option<PathBuf>, String> {
    let asset_dir = download_dir.join(asset_id);
    let entries = match fs::read_dir(&asset_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read downloaded texture directory {}: {error}",
                asset_dir.display()
            ));
        }
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to inspect downloaded texture directory {}: {error}",
                asset_dir.display()
            )
        })?;
    paths.retain(|path| path.is_file());
    paths.sort();
    for path in paths {
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read texture {}: {error}", path.display()))?;
        if image_mime_type(&bytes).is_some() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn stage_asset(
    source_path: &Path,
    materials_dir: &Path,
    output_dir: &Path,
) -> Result<PathBuf, String> {
    let Ok(relative_path) = source_path.strip_prefix(materials_dir) else {
        return Ok(source_path.to_owned());
    };
    let output_path = output_dir.join(relative_path);
    if output_path != source_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create staged asset directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::copy(source_path, &output_path).map_err(|error| {
            format!(
                "failed to copy asset {} to {}: {error}",
                source_path.display(),
                output_path.display()
            )
        })?;
    }
    Ok(output_path)
}

fn relative_uri(from_dir: &Path, to: &Path) -> Result<String, String> {
    let from_components: Vec<_> = from_dir.components().collect();
    let to_components: Vec<_> = to.components().collect();
    let common = from_components
        .iter()
        .zip(&to_components)
        .take_while(|(from, to)| from == to)
        .count();
    if common == 0 {
        return Err(format!(
            "cannot create a relative asset URI from {} to {}",
            from_dir.display(),
            to.display()
        ));
    }

    let mut relative = PathBuf::new();
    for _ in common..from_components.len() {
        relative.push("..");
    }
    for component in &to_components[common..] {
        relative.push(component.as_os_str());
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn position_bounds(mesh: &UnionMesh) -> (Bounds, Bounds) {
    let first = mesh.vertices[0].position;
    mesh.vertices.iter().skip(1).fold(
        (Bounds(first), Bounds(first)),
        |(mut min, mut max), vertex| {
            for axis in 0..3 {
                min.0[axis] = min.0[axis].min(vertex.position[axis]);
                max.0[axis] = max.0[axis].max(vertex.position[axis]);
            }
            (min, max)
        },
    )
}

struct Bounds([f32; 3]);

fn pad_to_four(bytes: &mut Vec<u8>, value: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{csg::UnionVertex, model::ModelPrimitive};
    use serde_json::{Value, json};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn uses_downloaded_image_assets_for_material_textures() {
        let root = std::env::temp_dir().join(format!(
            "roform-downloaded-texture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let model_dir = root.join("model");
        let download_dir = root.join("download");
        let buffer_path = root.join("bin").join("triangle.bin");
        fs::create_dir_all(&model_dir).unwrap();
        fs::create_dir_all(buffer_path.parent().unwrap()).unwrap();
        let texture_dir = download_dir.join("123");
        fs::create_dir_all(&texture_dir).unwrap();
        fs::write(texture_dir.join("asset.png"), b"\x89PNG\r\n\x1a\n").unwrap();

        let model = ModelAsset {
            name: "Triangle".to_owned(),
            primitives: vec![ModelPrimitive {
                name: "TrianglePart".to_owned(),
                mesh: UnionMesh {
                    vertices: vec![
                        UnionVertex {
                            position: [0.0, 0.0, 0.0],
                            normal: [0.0, 1.0, 0.0],
                            tex_coord: [0.0, 0.0],
                            color: [255; 4],
                        },
                        UnionVertex {
                            position: [1.0, 0.0, 0.0],
                            normal: [0.0, 1.0, 0.0],
                            tex_coord: [1.0, 0.0],
                            color: [255; 4],
                        },
                        UnionVertex {
                            position: [0.0, 1.0, 0.0],
                            normal: [0.0, 1.0, 0.0],
                            tex_coord: [0.0, 1.0],
                            color: [255; 4],
                        },
                    ],
                    indices: vec![0, 1, 2],
                },
                matrix: [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                material: ModelMaterial {
                    name: "plastic".to_owned(),
                    color: [1.0; 4],
                    base_color_asset: Some("123".to_owned()),
                    normal_asset: None,
                },
                extras: Value::Null,
            }],
            extras: Value::Null,
            warnings: Vec::new(),
            asset_ids: vec!["123".to_owned()],
        };

        let gltf = model_to_gltf(
            &model,
            &download_dir,
            &root.join("materials"),
            &model_dir,
            &root,
            &buffer_path,
            true,
        )
        .unwrap();
        let document: Value = serde_json::from_slice(&gltf).unwrap();
        assert_eq!(document["images"][0]["uri"], "../download/123/asset.png");
        assert_eq!(
            document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn packs_external_buffer_and_image_into_glb() {
        let root = std::env::temp_dir().join(format!(
            "roform-glb-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let model_dir = root.join("model");
        let buffer_path = root.join("bin").join("triangle.bin");
        let image_path = root.join("material").join("triangle.png");
        fs::create_dir_all(&model_dir).unwrap();
        fs::create_dir_all(buffer_path.parent().unwrap()).unwrap();
        fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        fs::write(&buffer_path, [1, 2, 3, 4]).unwrap();
        fs::write(&image_path, [5, 6, 7]).unwrap();

        let gltf = serde_json::to_vec(&json!({
            "asset": { "version": "2.0" },
            "buffers": [{ "byteLength": 4, "uri": "../bin/triangle.bin" }],
            "bufferViews": [{ "buffer": 0, "byteLength": 4 }],
            "images": [{ "mimeType": "image/png", "uri": "../material/triangle.png" }]
        }))
        .unwrap();
        let glb = gltf_to_glb(&gltf, &model_dir.join("triangle.gltf")).unwrap();

        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
        let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        assert_eq!(&glb[16..20], b"JSON");
        let json_start = 20;
        let json_end = json_start + json_length;
        let document: Value = serde_json::from_slice(&glb[json_start..json_end]).unwrap();
        assert!(document["buffers"][0].get("uri").is_none());
        assert_eq!(document["buffers"][0]["byteLength"], 7);
        assert_eq!(document["images"][0]["bufferView"], 1);
        assert_eq!(document["bufferViews"][1]["byteOffset"], 4);
        assert_eq!(document["bufferViews"][1]["byteLength"], 3);

        let binary_header_start = json_end;
        let binary_length = u32::from_le_bytes(
            glb[binary_header_start..binary_header_start + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(
            &glb[binary_header_start + 4..binary_header_start + 8],
            b"BIN\0"
        );
        let binary_start = binary_header_start + 8;
        assert_eq!(binary_length, 8);
        assert_eq!(&glb[binary_start..binary_start + 7], [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(glb[binary_start + 7], 0);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_external_buffer_with_relative_uri() {
        let root = std::env::temp_dir().join(format!(
            "roform-gltf-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let model_dir = root.join("model");
        let materials_dir = root.join("materials");
        let buffer_path = root.join("bin").join("triangle.bin");
        fs::create_dir_all(&model_dir).unwrap();
        let image_path = materials_dir.join("Plastic_color.png");
        fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        fs::write(&image_path, b"\x89PNG\r\n\x1a\n").unwrap();
        fs::write(
            materials_dir.join("Plastic_normal.png"),
            b"\x89PNG\r\n\x1a\n",
        )
        .unwrap();

        let mut model = ModelAsset {
            name: "Triangle".to_owned(),
            primitives: vec![ModelPrimitive {
                name: "TrianglePart".to_owned(),
                mesh: UnionMesh {
                    vertices: vec![
                        UnionVertex {
                            position: [0.0, 0.0, 0.0],
                            normal: [0.0, 1.0, 0.0],
                            tex_coord: [0.0, 0.0],
                            color: [255; 4],
                        },
                        UnionVertex {
                            position: [1.0, 0.0, 0.0],
                            normal: [0.0, 1.0, 0.0],
                            tex_coord: [1.0, 0.0],
                            color: [255; 4],
                        },
                        UnionVertex {
                            position: [0.0, 1.0, 0.0],
                            normal: [0.0, 1.0, 0.0],
                            tex_coord: [0.0, 1.0],
                            color: [255; 4],
                        },
                    ],
                    indices: vec![0, 1, 2],
                },
                matrix: [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                material: ModelMaterial {
                    name: "Plastic".to_owned(),
                    color: [0.25, 0.5, 0.75, 0.5],
                    base_color_asset: None,
                    normal_asset: None,
                },
                extras: json!({
                    "className": "Part",
                    "properties": { "Anchored": true }
                }),
            }],
            extras: json!({
                "className": "Model",
                "properties": { "PrimaryPart": { "Ref": "none" } }
            }),
            warnings: Vec::new(),
            asset_ids: Vec::new(),
        };

        let gltf = model_to_gltf(
            &model,
            &root.join("download"),
            &materials_dir,
            &model_dir,
            &root,
            &buffer_path,
            true,
        )
        .unwrap();
        let document: Value = serde_json::from_slice(&gltf).unwrap();
        assert_eq!(document["buffers"][0]["uri"], "../bin/triangle.bin");
        assert_eq!(document["samplers"][0]["wrapS"], 10497);
        assert_eq!(document["samplers"][0]["wrapT"], 10497);
        assert_eq!(document["textures"][0]["sampler"], 0);
        assert!(
            document["meshes"][0]["primitives"][0]["attributes"]
                .get("TANGENT")
                .is_some()
        );
        assert!(
            !document["buffers"][0]["uri"]
                .as_str()
                .unwrap()
                .starts_with("data:")
        );
        assert_eq!(
            document["buffers"][0]["byteLength"].as_u64(),
            Some(fs::metadata(&buffer_path).unwrap().len())
        );
        let index_accessor = document["meshes"][0]["primitives"][0]["indices"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(document["accessors"][index_accessor]["componentType"], 5123);
        assert_eq!(document["extras"]["roblox"]["className"], "Model");
        assert_eq!(
            document["extras"]["roblox"]["properties"]["PrimaryPart"]["Ref"],
            "none"
        );
        assert_eq!(
            document["nodes"][0]["extras"]["roblox"]["className"],
            "Part"
        );
        assert_eq!(
            document["meshes"][0]["extras"]["roblox"]["properties"]["Anchored"],
            true
        );
        assert_eq!(fs::read(&buffer_path).unwrap().len(), 164);

        let no_material_buffer_path = root.join("bin").join("triangle-nm.bin");
        let no_material_gltf = model_to_gltf(
            &model,
            &root.join("download"),
            &materials_dir,
            &model_dir,
            &root,
            &no_material_buffer_path,
            false,
        )
        .unwrap();
        let no_material_document: Value = serde_json::from_slice(&no_material_gltf).unwrap();
        assert_eq!(
            no_material_document["materials"].as_array().unwrap().len(),
            1
        );
        assert!(no_material_document.get("images").is_none());
        assert!(no_material_document.get("textures").is_none());
        assert!(no_material_document.get("samplers").is_none());
        assert_eq!(
            no_material_document["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
            json!([0.25, 0.5, 0.75, 0.5])
        );
        assert!(
            no_material_document["materials"][0]["pbrMetallicRoughness"]
                .get("baseColorTexture")
                .is_none()
        );
        assert!(
            no_material_document["materials"][0]
                .get("normalTexture")
                .is_none()
        );
        assert_eq!(
            no_material_document["meshes"][0]["primitives"][0]["material"],
            0
        );
        assert!(
            no_material_document["meshes"][0]["primitives"][0]["attributes"]
                .get("TANGENT")
                .is_none()
        );

        model.primitives[0].material.name = "neon".to_owned();
        let neon_buffer_path = root.join("bin").join("neon.bin");
        let neon_gltf = model_to_gltf(
            &model,
            &root.join("download"),
            &materials_dir,
            &model_dir,
            &root,
            &neon_buffer_path,
            false,
        )
        .unwrap();
        let neon_document: Value = serde_json::from_slice(&neon_gltf).unwrap();
        assert_eq!(
            neon_document["extensionsUsed"],
            json!(["KHR_materials_unlit"])
        );
        assert_eq!(
            neon_document["materials"][0]["extensions"]["KHR_materials_unlit"],
            json!({})
        );

        fs::remove_dir_all(root).unwrap();
    }
}
