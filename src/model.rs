use crate::{
    csg::{self, UnionMesh},
    geometry, metadata,
};
use rbx_dom_weak::{Instance, WeakDom, types::Ref};
use rbx_types::{CFrame, Variant, Vector3};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::BufReader,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct ModelAsset {
    pub name: String,
    pub primitives: Vec<ModelPrimitive>,
    pub extras: Value,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct ModelPrimitive {
    pub name: String,
    pub mesh: UnionMesh,
    pub matrix: [f32; 16],
    pub material: ModelMaterial,
    pub extras: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelMaterial {
    pub name: String,
    pub color: [f32; 4],
    pub base_color_asset: Option<String>,
    pub normal_asset: Option<String>,
}

pub(crate) fn source_files(input: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_source_files(input, &mut files)?;
    files.sort();
    Ok(files)
}

pub(crate) fn parse_models(
    path: &Path,
    download_dir: &Path,
    mesh_dir: &Path,
    studs_per_tile: f32,
) -> Result<Vec<ModelAsset>, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let file = fs::File::open(path)
        .map_err(|error| format!("failed to open model source {}: {error}", path.display()))?;
    let dom = match extension.as_str() {
        "rbxm" | "rbxl" => rbx_binary::from_reader(file)
            .map_err(|error| format!("failed to decode {}: {error}", path.display()))?,
        "rbxmx" | "rbxlx" => {
            rbx_xml::from_reader(BufReader::new(file), rbx_xml::DecodeOptions::default())
                .map_err(|error| format!("failed to decode {}: {error}", path.display()))?
        }
        _ => {
            return Err(format!(
                "unsupported Roblox source extension for {}",
                path.display()
            ));
        }
    };

    let roots = model_roots(&dom);
    let mut mesh_cache = HashMap::new();
    roots
        .into_iter()
        .map(|root_ref| {
            parse_model(
                &dom,
                root_ref,
                download_dir,
                mesh_dir,
                studs_per_tile,
                &mut mesh_cache,
            )
        })
        .collect()
}

fn collect_source_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "rbxm" | "rbxmx" | "rbxl" | "rbxlx") {
            files.push(path.to_owned());
        }
        return Ok(());
    }

    if !path.is_dir() {
        return Err(format!("model input does not exist: {}", path.display()));
    }

    let entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read model input {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("failed to inspect model input {}: {error}", path.display())
        })?;
        collect_source_files(&entry.path(), files)?;
    }
    Ok(())
}

fn model_roots(dom: &WeakDom) -> Vec<Ref> {
    let mut roots = Vec::new();
    let root_ref = dom.root_ref();
    if metadata::is_class(dom.root(), "Model") {
        roots.push(root_ref);
    }
    for instance in dom.descendants() {
        if metadata::is_class(instance, "Model")
            && !dom
                .ancestors_of(instance.referent())
                .any(|ancestor| metadata::is_class(ancestor, "Model"))
        {
            roots.push(instance.referent());
        }
    }

    if roots.is_empty() {
        roots.push(root_ref);
    }
    roots
}

fn parse_model(
    dom: &WeakDom,
    root_ref: Ref,
    download_dir: &Path,
    mesh_dir: &Path,
    studs_per_tile: f32,
    mesh_cache: &mut HashMap<String, Result<UnionMesh, String>>,
) -> Result<ModelAsset, String> {
    let root = dom
        .get_by_ref(root_ref)
        .ok_or_else(|| "model root referent is missing from the DOM".to_owned())?;
    let reflection_database = metadata::reflection_database();
    let model_extras = metadata::roblox_instance_extras(dom, root_ref, reflection_database);
    let pivot = metadata::property_cframe(root, "WorldPivotData")
        .or_else(|| metadata::property_cframe(root, "ModelMeshCFrame"))
        .unwrap_or_else(CFrame::identity);

    let mut geometry_refs = Vec::new();
    if metadata::is_geometry(root) {
        geometry_refs.push(root_ref);
    }
    geometry_refs.extend(
        dom.descendants_of(root_ref)
            .filter(|instance| metadata::is_geometry(instance))
            .map(Instance::referent),
    );

    let mut primitives = Vec::new();
    let mut warnings = Vec::new();
    for geometry_ref in geometry_refs {
        let instance = dom
            .get_by_ref(geometry_ref)
            .ok_or_else(|| "model geometry referent is missing from the DOM".to_owned())?;
        let cframe = metadata::property_cframe(instance, "CFrame").unwrap_or_else(CFrame::identity);
        let matrix = relative_matrix(pivot, cframe);
        let mut material = metadata::material_for(dom, instance);
        let mesh = match instance.class.as_str() {
            "Part" => {
                if let Some(special_mesh_ref) = metadata::direct_child(dom, instance, "SpecialMesh")
                {
                    let special_mesh = dom.get_by_ref(special_mesh_ref).expect("valid child ref");
                    if let Some(asset_id) = metadata::property_asset_id(special_mesh, "MeshId") {
                        material.base_color_asset =
                            metadata::property_asset_id(special_mesh, "TextureId")
                                .or_else(|| metadata::property_asset_id(special_mesh, "TextureID"));
                        match load_mesh(&asset_id, download_dir, mesh_dir, mesh_cache) {
                            Ok(mut mesh) => {
                                apply_special_mesh_transform(&mut mesh, special_mesh);
                                Some(mesh)
                            }
                            Err(error) => {
                                warnings.push(format!("{} ({}): {error}", instance.name, asset_id));
                                None
                            }
                        }
                    } else {
                        Some(primitive_mesh(instance, studs_per_tile))
                    }
                } else {
                    Some(primitive_mesh(instance, studs_per_tile))
                }
            }
            "WedgePart" | "CornerWedgePart" => Some(primitive_mesh(instance, studs_per_tile)),
            "MeshPart" => {
                material.base_color_asset = metadata::property_asset_id(instance, "TextureID");
                let mesh_id = metadata::property_asset_id(instance, "MeshId")
                    .or_else(|| metadata::property_asset_id(instance, "MeshContent"));
                match mesh_id {
                    Some(asset_id) => {
                        match load_mesh(&asset_id, download_dir, mesh_dir, mesh_cache) {
                            Ok(mut mesh) => {
                                apply_instance_size(&mut mesh, instance);
                                Some(mesh)
                            }
                            Err(error) => {
                                warnings.push(format!("{} ({}): {error}", instance.name, asset_id));
                                None
                            }
                        }
                    }
                    None => {
                        warnings.push(format!("{}: MeshPart has no MeshId", instance.name));
                        None
                    }
                }
            }
            "UnionOperation" => match metadata::property_asset_id(instance, "AssetId") {
                Some(asset_id) => match load_mesh(&asset_id, download_dir, mesh_dir, mesh_cache) {
                    Ok(mut mesh) => {
                        apply_instance_size(&mut mesh, instance);
                        Some(mesh)
                    }
                    Err(error) => {
                        warnings.push(format!("{} ({}): {error}", instance.name, asset_id));
                        None
                    }
                },
                None => {
                    warnings.push(format!("{}: UnionOperation has no AssetId", instance.name));
                    None
                }
            },
            _ => None,
        };

        if let Some(surface_appearance_ref) =
            metadata::direct_child(dom, instance, "SurfaceAppearance")
        {
            let surface_appearance = dom
                .get_by_ref(surface_appearance_ref)
                .expect("valid child ref");
            material.base_color_asset = metadata::property_asset_id(surface_appearance, "ColorMap")
                .or_else(|| material.base_color_asset.clone());
            material.normal_asset = metadata::property_asset_id(surface_appearance, "NormalMap");
        }

        if let Some(mesh) = mesh {
            primitives.push(ModelPrimitive {
                name: instance.name.clone(),
                mesh,
                matrix,
                material,
                extras: metadata::roblox_instance_extras(dom, geometry_ref, reflection_database),
            });
        }
    }

    Ok(ModelAsset {
        name: root.name.clone(),
        primitives,
        extras: model_extras,
        warnings,
    })
}

fn primitive_mesh(instance: &Instance, studs_per_tile: f32) -> UnionMesh {
    let size = metadata::property_vector3(instance, "size")
        .or_else(|| metadata::property_vector3(instance, "Size"))
        .unwrap_or(Vector3::new(1.0, 1.0, 1.0));
    let shape = match instance.class.as_str() {
        "WedgePart" => 3,
        "CornerWedgePart" => 4,
        _ => instance
            .properties
            .get(&"shape".into())
            .or_else(|| instance.properties.get(&"Shape".into()))
            .and_then(metadata::enum_value)
            .unwrap_or(1),
    };
    geometry::primitive_mesh(shape, size, studs_per_tile)
}

fn load_mesh(
    asset_id: &str,
    download_dir: &Path,
    mesh_dir: &Path,
    cache: &mut HashMap<String, Result<UnionMesh, String>>,
) -> Result<UnionMesh, String> {
    if let Some(mesh) = cache.get(asset_id) {
        return mesh.clone();
    }

    let asset_dir = download_dir.join(asset_id);
    let payload_path = asset_dir.join("asset.bin");
    let (source_path, packaged) = if payload_path.is_file() {
        (payload_path, false)
    } else {
        (asset_dir.join("asset.rbxm"), true)
    };
    let result = fs::read(&source_path)
        .map_err(|error| format!("failed to read {}: {error}", source_path.display()))
        .and_then(|bytes| {
            let cache_path = mesh_dir.join(format!("{}.bin", blake3::hash(&bytes).to_hex()));
            match fs::read(&cache_path) {
                Ok(cached_bytes) => csg::decode_cached_mesh(&cached_bytes).or_else(|_| {
                    decode_and_cache_mesh(&bytes, &source_path, packaged, &cache_path)
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    decode_and_cache_mesh(&bytes, &source_path, packaged, &cache_path)
                }
                Err(error) => Err(format!(
                    "failed to read decoded mesh cache {}: {error}",
                    cache_path.display()
                )),
            }
        });
    cache.insert(asset_id.to_owned(), result.clone());
    result
}

fn decode_and_cache_mesh(
    bytes: &[u8],
    source_path: &Path,
    packaged: bool,
    cache_path: &Path,
) -> Result<UnionMesh, String> {
    let mesh = if packaged {
        load_packaged_mesh_bytes(bytes, source_path)?
    } else {
        csg::decode_mesh_payload(bytes)?
    };
    let cached_bytes = csg::encode_cached_mesh(&mesh)?;
    if let Err(error) = fs::write(cache_path, cached_bytes) {
        eprintln!(
            "warning: failed to write decoded mesh cache {}: {error}",
            cache_path.display()
        );
    }
    Ok(mesh)
}

fn load_packaged_mesh_bytes(bytes: &[u8], path: &Path) -> Result<UnionMesh, String> {
    let dom = rbx_binary::from_reader(bytes)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))?;
    let mesh_data = dom
        .descendants()
        .filter(|instance| instance.class.as_str() == "PartOperationAsset")
        .find_map(|instance| {
            instance
                .properties
                .get(&"MeshData".into())
                .and_then(|value| match value {
                    Variant::BinaryString(data) => Some(data.as_ref()),
                    _ => None,
                })
        })
        .ok_or_else(|| format!("{} contains no PartOperationAsset MeshData", path.display()))?;
    csg::decode_mesh_payload(mesh_data)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))
}

fn apply_instance_size(mesh: &mut UnionMesh, instance: &Instance) {
    let Some(size) = metadata::property_vector3(instance, "size")
        .or_else(|| metadata::property_vector3(instance, "Size"))
    else {
        return;
    };
    let source_size = metadata::property_vector3(instance, "InitialSize")
        .unwrap_or_else(|| mesh_bounds_size(mesh));
    scale_mesh_to_size(mesh, size, source_size);
}

fn scale_mesh_to_size(mesh: &mut UnionMesh, target_size: Vector3, source_size: Vector3) {
    let scale = [
        size_scale(target_size.x, source_size.x),
        size_scale(target_size.y, source_size.y),
        size_scale(target_size.z, source_size.z),
    ];
    for vertex in &mut mesh.vertices {
        for (coordinate, factor) in vertex.position.iter_mut().zip(scale) {
            *coordinate *= factor;
        }
        vertex.normal = geometry_normalize([
            inverse_scale(vertex.normal[0], scale[0]),
            inverse_scale(vertex.normal[1], scale[1]),
            inverse_scale(vertex.normal[2], scale[2]),
        ]);
    }
}

fn size_scale(target: f32, source: f32) -> f32 {
    if source.abs() <= f32::EPSILON {
        1.0
    } else {
        target.abs() / source.abs()
    }
}

fn inverse_scale(value: f32, scale: f32) -> f32 {
    if scale.abs() <= f32::EPSILON {
        value
    } else {
        value / scale
    }
}

fn mesh_bounds_size(mesh: &UnionMesh) -> Vector3 {
    if mesh.vertices.is_empty() {
        return Vector3::new(1.0, 1.0, 1.0);
    }

    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex.position[axis]);
            maximum[axis] = maximum[axis].max(vertex.position[axis]);
        }
    }
    Vector3::new(
        maximum[0] - minimum[0],
        maximum[1] - minimum[1],
        maximum[2] - minimum[2],
    )
}

fn apply_special_mesh_transform(mesh: &mut UnionMesh, instance: &Instance) {
    let scale =
        metadata::property_vector3(instance, "Scale").unwrap_or(Vector3::new(1.0, 1.0, 1.0));
    let offset =
        metadata::property_vector3(instance, "Offset").unwrap_or(Vector3::new(0.0, 0.0, 0.0));
    for vertex in &mut mesh.vertices {
        vertex.position[0] = vertex.position[0] * scale.x + offset.x;
        vertex.position[1] = vertex.position[1] * scale.y + offset.y;
        vertex.position[2] = vertex.position[2] * scale.z + offset.z;
    }
}

fn relative_matrix(root: CFrame, child: CFrame) -> [f32; 16] {
    let root_rotation = matrix_rows(root);
    let child_rotation = matrix_rows(child);
    let mut relative_rotation = [[0.0; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            relative_rotation[row][column] = (0..3)
                .map(|index| root_rotation[index][row] * child_rotation[index][column])
                .sum();
        }
    }

    let delta = [
        child.position.x - root.position.x,
        child.position.y - root.position.y,
        child.position.z - root.position.z,
    ];
    let position = [
        (0..3)
            .map(|index| root_rotation[index][0] * delta[index])
            .sum(),
        (0..3)
            .map(|index| root_rotation[index][1] * delta[index])
            .sum(),
        (0..3)
            .map(|index| root_rotation[index][2] * delta[index])
            .sum(),
    ];

    [
        relative_rotation[0][0],
        relative_rotation[1][0],
        relative_rotation[2][0],
        0.0,
        relative_rotation[0][1],
        relative_rotation[1][1],
        relative_rotation[2][1],
        0.0,
        relative_rotation[0][2],
        relative_rotation[1][2],
        relative_rotation[2][2],
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}

fn matrix_rows(cframe: CFrame) -> [[f32; 3]; 3] {
    [
        [
            cframe.orientation.x.x,
            cframe.orientation.x.y,
            cframe.orientation.x.z,
        ],
        [
            cframe.orientation.y.x,
            cframe.orientation.y.y,
            cframe.orientation.y.z,
        ],
        [
            cframe.orientation.z.x,
            cframe.orientation.z.y,
            cframe.orientation.z.z,
        ],
    ]
}

fn geometry_normalize(value: [f32; 3]) -> [f32; 3] {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if length <= f32::EPSILON {
        [0.0, 1.0, 0.0]
    } else {
        [value[0] / length, value[1] / length, value[2] / length]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csg::UnionVertex;
    use rbx_dom_weak::InstanceBuilder;

    #[test]
    fn converts_a_world_cframe_to_model_local_space() {
        let root = CFrame::new(
            Vector3::new(10.0, 20.0, 30.0),
            rbx_types::Matrix3::identity(),
        );
        let child = CFrame::new(
            Vector3::new(11.0, 22.0, 33.0),
            rbx_types::Matrix3::identity(),
        );
        let matrix = relative_matrix(root, child);
        assert_eq!(&matrix[12..15], &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn scales_imported_mesh_to_instance_size() {
        let mut mesh = UnionMesh {
            vertices: vec![
                UnionVertex {
                    position: [-5.0, -1.0, -2.0],
                    normal: [1.0, 0.0, 0.0],
                    tex_coord: [0.0, 0.0],
                    color: [255; 4],
                },
                UnionVertex {
                    position: [5.0, 1.0, 2.0],
                    normal: [0.0, 1.0, 0.0],
                    tex_coord: [1.0, 1.0],
                    color: [255; 4],
                },
            ],
            indices: Vec::new(),
        };

        scale_mesh_to_size(
            &mut mesh,
            Vector3::new(2.0, 4.0, 6.0),
            Vector3::new(10.0, 2.0, 4.0),
        );

        assert_eq!(mesh.vertices[0].position, [-1.0, -2.0, -3.0]);
        assert_eq!(mesh.vertices[1].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn finds_geometry_under_a_model_root() {
        let dom = WeakDom::new(
            InstanceBuilder::new("Model")
                .with_child(InstanceBuilder::new("Part").with_name("Part")),
        );

        assert_eq!(model_roots(&dom), vec![dom.root_ref()]);
    }

    #[test]
    fn generates_wedge_geometry_from_wedge_part_class() {
        let dom = WeakDom::new(
            InstanceBuilder::new("WedgePart").with_property("Size", Vector3::new(2.0, 4.0, 6.0)),
        );

        let mesh = primitive_mesh(dom.root(), 2.0);

        assert_eq!(mesh.vertices.len(), 18);
        assert_eq!(mesh.indices.len(), 24);
    }
}
