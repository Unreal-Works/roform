use crate::{
    csg::UnionMesh,
    model::{ModelAsset, ModelMaterial},
};
use base64::Engine;
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn model_to_gltf(
    model: &ModelAsset,
    download_dir: &Path,
    assets_dir: &Path,
    gltf_output_dir: &Path,
    asset_output_dir: &Path,
) -> Result<Vec<u8>, String> {
    if model.primitives.is_empty() {
        return Err("model contains no renderable geometry".to_owned());
    }

    let vertex_stride = 36usize;
    let mut binary = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let mut material_indices = HashMap::<String, usize>::new();
    let mut image_indices = HashMap::<PathBuf, usize>::new();

    for primitive in &model.primitives {
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

        pad_to_four(&mut binary, 0);
        let index_offset = binary.len();
        for index in &primitive.mesh.indices {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let index_length = binary.len() - index_offset;
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": index_offset,
            "byteLength": index_length,
            "target": 34963
        }));
        let index_view = buffer_views.len() - 1;
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
        let index_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": index_view,
            "componentType": 5125,
            "count": primitive.mesh.indices.len(),
            "type": "SCALAR"
        }));

        let material_index = material_index(
            &primitive.material,
            download_dir,
            assets_dir,
            gltf_output_dir,
            asset_output_dir,
            &mut material_indices,
            &mut materials,
            &mut image_indices,
            &mut images,
            &mut textures,
        )?;
        let mesh_index = meshes.len();
        meshes.push(json!({
            "name": primitive.name.as_str(),
            "primitives": [{
                "attributes": {
                    "POSITION": position_accessor,
                    "NORMAL": normal_accessor,
                    "TEXCOORD_0": tex_coord_accessor,
                    "COLOR_0": color_accessor
                },
                "indices": index_accessor,
                "material": material_index,
                "mode": 4
            }]
        }));
        nodes.push(json!({
            "name": primitive.name.as_str(),
            "mesh": mesh_index,
            "matrix": primitive.matrix
        }));
    }

    let binary_uri = format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&binary)
    );
    let json_value = json!({
        "asset": { "version": "2.0", "generator": "roform" },
        "buffers": [{ "byteLength": binary.len(), "uri": binary_uri }],
        "bufferViews": buffer_views,
        "accessors": accessors,
        "images": images,
        "textures": textures,
        "materials": materials,
        "meshes": meshes,
        "nodes": nodes,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "scene": 0
    });
    serde_json::to_vec_pretty(&json_value)
        .map_err(|error| format!("failed to serialize glTF: {error}"))
}

fn material_index(
    material: &ModelMaterial,
    download_dir: &Path,
    assets_dir: &Path,
    gltf_output_dir: &Path,
    asset_output_dir: &Path,
    material_indices: &mut HashMap<String, usize>,
    materials: &mut Vec<serde_json::Value>,
    image_indices: &mut HashMap<PathBuf, usize>,
    images: &mut Vec<serde_json::Value>,
    textures: &mut Vec<serde_json::Value>,
) -> Result<usize, String> {
    let key = format!(
        "{}:{:?}:{:?}:{:?}",
        material.name, material.color, material.base_color_asset, material.normal_asset
    );
    if let Some(index) = material_indices.get(&key) {
        return Ok(*index);
    }

    let base_color = material_image(
        material,
        true,
        download_dir,
        assets_dir,
        gltf_output_dir,
        asset_output_dir,
        image_indices,
        images,
        textures,
    )?;
    let normal = material_image(
        material,
        false,
        download_dir,
        assets_dir,
        gltf_output_dir,
        asset_output_dir,
        image_indices,
        images,
        textures,
    )?;
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
    if let Some(image_index) = normal {
        material_json["normalTexture"] = json!({ "index": image_index });
    }
    if material.color[3] < 1.0 {
        material_json["alphaMode"] = json!("BLEND");
    }

    let index = materials.len();
    materials.push(material_json);
    material_indices.insert(key, index);
    Ok(index)
}

fn material_image(
    material: &ModelMaterial,
    base_color: bool,
    download_dir: &Path,
    assets_dir: &Path,
    gltf_output_dir: &Path,
    asset_output_dir: &Path,
    image_indices: &mut HashMap<PathBuf, usize>,
    images: &mut Vec<serde_json::Value>,
    textures: &mut Vec<serde_json::Value>,
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
    let asset_path = asset_id.map(|asset_id| download_dir.join(asset_id).join("asset.bin"));
    let fallback_path = assets_dir.join("material").join(fallback_name);
    let source_path = asset_path
        .filter(|path| path.is_file())
        .or_else(|| fallback_path.is_file().then_some(fallback_path));
    let Some(source_path) = source_path else {
        return Ok(None);
    };

    if let Some(index) = image_indices.get(&source_path) {
        return Ok(Some(*index));
    }
    let bytes = fs::read(&source_path)
        .map_err(|error| format!("failed to read texture {}: {error}", source_path.display()))?;
    let Some(mime_type) = image_mime_type(&bytes) else {
        return Ok(None);
    };
    let output_path = stage_asset(&source_path, assets_dir, asset_output_dir)?;
    let uri = relative_uri(gltf_output_dir, &output_path)?;
    let image_index = images.len();
    images.push(json!({
        "name": output_path.file_name().and_then(|name| name.to_str()),
        "mimeType": mime_type,
        "uri": uri
    }));
    let texture_index = textures.len();
    textures.push(json!({ "source": image_index }));
    image_indices.insert(source_path, texture_index);
    Ok(Some(texture_index))
}

fn stage_asset(source_path: &Path, assets_dir: &Path, output_dir: &Path) -> Result<PathBuf, String> {
    let Ok(relative_path) = source_path.strip_prefix(assets_dir) else {
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
    while bytes.len() % 4 != 0 {
        bytes.push(value);
    }
}
