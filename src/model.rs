use crate::{
    csg::{self, UnionMesh, UnionVertex},
    decode_mesh_payload,
};
use rbx_dom_weak::{Instance, WeakDom, types::Ref};
use rbx_reflection::{DataType, ReflectionDatabase};
use rbx_types::{CFrame, Variant, Vector3};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeSet, HashMap},
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
        .map(|root_ref| parse_model(&dom, root_ref, download_dir, mesh_dir, &mut mesh_cache))
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
    if is_class(dom.root(), "Model") {
        roots.push(root_ref);
    }
    for instance in dom.descendants() {
        if is_class(instance, "Model")
            && !dom
                .ancestors_of(instance.referent())
                .any(|ancestor| is_class(ancestor, "Model"))
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
    mesh_cache: &mut HashMap<String, Result<UnionMesh, String>>,
) -> Result<ModelAsset, String> {
    let root = dom
        .get_by_ref(root_ref)
        .ok_or_else(|| "model root referent is missing from the DOM".to_owned())?;
    let reflection_database = reflection_database();
    let model_extras = roblox_instance_extras(dom, root_ref, reflection_database);
    let pivot = property_cframe(root, "WorldPivotData")
        .or_else(|| property_cframe(root, "ModelMeshCFrame"))
        .unwrap_or_else(CFrame::identity);

    let mut geometry_refs = Vec::new();
    if is_geometry(root) {
        geometry_refs.push(root_ref);
    }
    geometry_refs.extend(
        dom.descendants_of(root_ref)
            .filter(|instance| is_geometry(instance))
            .map(Instance::referent),
    );

    let mut primitives = Vec::new();
    let mut warnings = Vec::new();
    for geometry_ref in geometry_refs {
        let instance = dom
            .get_by_ref(geometry_ref)
            .ok_or_else(|| "model geometry referent is missing from the DOM".to_owned())?;
        let cframe = property_cframe(instance, "CFrame").unwrap_or_else(CFrame::identity);
        let matrix = relative_matrix(pivot, cframe);
        let mut material = material_for(dom, instance);
        let mesh = match instance.class.as_str() {
            "Part" => {
                if let Some(special_mesh_ref) = direct_child(dom, instance, "SpecialMesh") {
                    let special_mesh = dom.get_by_ref(special_mesh_ref).expect("valid child ref");
                    if let Some(asset_id) = property_asset_id(special_mesh, "MeshId") {
                        material.base_color_asset = property_asset_id(special_mesh, "TextureId")
                            .or_else(|| property_asset_id(special_mesh, "TextureID"));
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
                        Some(primitive_mesh(instance))
                    }
                } else {
                    Some(primitive_mesh(instance))
                }
            }
            "MeshPart" => {
                material.base_color_asset = property_asset_id(instance, "TextureID");
                let mesh_id = property_asset_id(instance, "MeshId")
                    .or_else(|| property_asset_id(instance, "MeshContent"));
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
            "UnionOperation" => match property_asset_id(instance, "AssetId") {
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

        if let Some(surface_appearance_ref) = direct_child(dom, instance, "SurfaceAppearance") {
            let surface_appearance = dom
                .get_by_ref(surface_appearance_ref)
                .expect("valid child ref");
            material.base_color_asset = property_asset_id(surface_appearance, "ColorMap")
                .or_else(|| material.base_color_asset.clone());
            material.normal_asset = property_asset_id(surface_appearance, "NormalMap");
        }

        if let Some(mesh) = mesh {
            primitives.push(ModelPrimitive {
                name: instance.name.clone(),
                mesh,
                matrix,
                material,
                extras: roblox_instance_extras(dom, geometry_ref, reflection_database),
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
        let package_path = asset_dir.join("asset.rbxm");
        (package_path, true)
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
        decode_mesh_payload(bytes)?
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
    decode_mesh_payload(mesh_data)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))
}

fn apply_instance_size(mesh: &mut UnionMesh, instance: &Instance) {
    let Some(size) =
        property_vector3(instance, "size").or_else(|| property_vector3(instance, "Size"))
    else {
        return;
    };
    let source_size =
        property_vector3(instance, "InitialSize").unwrap_or_else(|| mesh_bounds_size(mesh));
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
        vertex.normal = normalize([
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

fn material_for(dom: &WeakDom, instance: &Instance) -> ModelMaterial {
    let material_value = instance.properties.get(&"Material".into());
    let material_name = material_value
        .and_then(enum_value)
        .map(material_name)
        .unwrap_or_else(|| "plastic".to_owned());
    let color = instance
        .properties
        .get(&"Color".into())
        .or_else(|| instance.properties.get(&"Color3uint8".into()))
        .and_then(color_value)
        .unwrap_or([255, 255, 255]);
    let transparency = instance
        .properties
        .get(&"Transparency".into())
        .and_then(float_value)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);

    let mut material = ModelMaterial {
        name: material_name,
        color: [
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
            1.0 - transparency,
        ],
        base_color_asset: None,
        normal_asset: None,
    };

    if let Some(surface_appearance_ref) = direct_child(dom, instance, "SurfaceAppearance") {
        let surface_appearance = dom
            .get_by_ref(surface_appearance_ref)
            .expect("valid child ref");
        material.base_color_asset = property_asset_id(surface_appearance, "ColorMap");
        material.normal_asset = property_asset_id(surface_appearance, "NormalMap");
    }
    material
}

fn primitive_mesh(instance: &Instance) -> UnionMesh {
    let size = property_vector3(instance, "size")
        .or_else(|| property_vector3(instance, "Size"))
        .unwrap_or(Vector3::new(1.0, 1.0, 1.0));
    let shape = instance
        .properties
        .get(&"shape".into())
        .or_else(|| instance.properties.get(&"Shape".into()))
        .and_then(enum_value)
        .unwrap_or(1);

    match shape {
        0 => sphere_mesh(size),
        2 => cylinder_mesh(size),
        3 => wedge_mesh(size),
        4 => corner_wedge_mesh(size),
        _ => box_mesh(size),
    }
}

fn box_mesh(size: Vector3) -> UnionMesh {
    let x = size.x.abs() * 0.5;
    let y = size.y.abs() * 0.5;
    let z = size.z.abs() * 0.5;
    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(24),
        indices: Vec::with_capacity(36),
    };
    add_face(
        &mut mesh,
        [[x, -y, -z], [x, y, -z], [x, y, z], [x, -y, z]],
        [1.0, 0.0, 0.0],
    );
    add_face(
        &mut mesh,
        [[-x, -y, z], [-x, y, z], [-x, y, -z], [-x, -y, -z]],
        [-1.0, 0.0, 0.0],
    );
    add_face(
        &mut mesh,
        [[-x, y, -z], [-x, y, z], [x, y, z], [x, y, -z]],
        [0.0, 1.0, 0.0],
    );
    add_face(
        &mut mesh,
        [[-x, -y, z], [-x, -y, -z], [x, -y, -z], [x, -y, z]],
        [0.0, -1.0, 0.0],
    );
    add_face(
        &mut mesh,
        [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]],
        [0.0, 0.0, 1.0],
    );
    add_face(
        &mut mesh,
        [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]],
        [0.0, 0.0, -1.0],
    );
    mesh
}

fn cylinder_mesh(size: Vector3) -> UnionMesh {
    let segments = 24usize;
    let radius = size.y.abs().min(size.z.abs()) * 0.5;
    let half_height = size.x.abs() * 0.5;

    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(segments * 12),
        indices: Vec::with_capacity(segments * 12 * 3),
    };

    for segment in 0..segments {
        let next = (segment + 1) % segments;
        let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
        let next_angle = next as f32 / segments as f32 * std::f32::consts::TAU;

        let current = [angle.cos(), angle.sin()];
        let following = [next_angle.cos(), next_angle.sin()];

        // +90 degrees around Z:
        // (x, y, z) -> (-y, x, z)
        add_face(
            &mut mesh,
            [
                [half_height, radius * current[0], radius * current[1]],
                [half_height, radius * following[0], radius * following[1]],
                [-half_height, radius * following[0], radius * following[1]],
                [-half_height, radius * current[0], radius * current[1]],
            ],
            [0.0, current[0], current[1]],
        );

        // Original top cap (y = +half_height)
        // rotated to x = -half_height.
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend([
            UnionVertex {
                position: [-half_height, 0.0, 0.0],
                normal: [-1.0, 0.0, 0.0],
                tex_coord: [0.5, 0.5],
                color: [255; 4],
            },
            UnionVertex {
                position: [-half_height, radius * following[0], radius * following[1]],
                normal: [-1.0, 0.0, 0.0],
                tex_coord: [following[0] * 0.5 + 0.5, following[1] * 0.5 + 0.5],
                color: [255; 4],
            },
            UnionVertex {
                position: [-half_height, radius * current[0], radius * current[1]],
                normal: [-1.0, 0.0, 0.0],
                tex_coord: [current[0] * 0.5 + 0.5, current[1] * 0.5 + 0.5],
                color: [255; 4],
            },
        ]);
        mesh.indices.extend([base, base + 1, base + 2]);

        // Original bottom cap (y = -half_height)
        // rotated to x = +half_height.
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend([
            UnionVertex {
                position: [half_height, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
                tex_coord: [0.5, 0.5],
                color: [255; 4],
            },
            UnionVertex {
                position: [half_height, radius * current[0], radius * current[1]],
                normal: [1.0, 0.0, 0.0],
                tex_coord: [current[0] * 0.5 + 0.5, current[1] * 0.5 + 0.5],
                color: [255; 4],
            },
            UnionVertex {
                position: [half_height, radius * following[0], radius * following[1]],
                normal: [1.0, 0.0, 0.0],
                tex_coord: [following[0] * 0.5 + 0.5, following[1] * 0.5 + 0.5],
                color: [255; 4],
            },
        ]);
        mesh.indices.extend([base, base + 1, base + 2]);
    }

    mesh
}

fn sphere_mesh(size: Vector3) -> UnionMesh {
    let rings = 10usize;
    let segments = 24usize;
    let radii = [size.x.abs() * 0.5, size.y.abs() * 0.5, size.z.abs() * 0.5];
    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(rings * segments * 4),
        indices: Vec::with_capacity(rings * segments * 6),
    };
    for ring in 0..rings {
        let lower =
            -std::f32::consts::FRAC_PI_2 + ring as f32 / rings as f32 * std::f32::consts::PI;
        let upper =
            -std::f32::consts::FRAC_PI_2 + (ring + 1) as f32 / rings as f32 * std::f32::consts::PI;
        for segment in 0..segments {
            let next = (segment + 1) % segments;
            let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
            let next_angle = next as f32 / segments as f32 * std::f32::consts::TAU;
            let points = [
                sphere_point(lower, angle, radii),
                sphere_point(lower, next_angle, radii),
                sphere_point(upper, next_angle, radii),
                sphere_point(upper, angle, radii),
            ];
            let normals = points.map(|point| {
                let normal = [
                    if radii[0] == 0.0 {
                        0.0
                    } else {
                        point[0] / radii[0]
                    },
                    if radii[1] == 0.0 {
                        0.0
                    } else {
                        point[1] / radii[1]
                    },
                    if radii[2] == 0.0 {
                        0.0
                    } else {
                        point[2] / radii[2]
                    },
                ];
                normalize(normal)
            });
            let base = mesh.vertices.len() as u32;
            for (point, normal) in points.into_iter().zip(normals) {
                mesh.vertices.push(UnionVertex {
                    position: point,
                    normal,
                    tex_coord: [
                        point[0].atan2(point[2]) / std::f32::consts::TAU + 0.5,
                        point[1] / radii[1].max(f32::EPSILON) * 0.5 + 0.5,
                    ],
                    color: [255; 4],
                });
            }
            mesh.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    mesh
}

fn wedge_mesh(size: Vector3) -> UnionMesh {
    let x = size.x.abs() * 0.5;
    let y = size.y.abs() * 0.5;
    let z = size.z.abs() * 0.5;

    let p = |v: [f32; 3]| [-v[0], v[1], -v[2]];
    let n = |v: [f32; 3]| [-v[0], v[1], -v[2]];

    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(18),
        indices: Vec::with_capacity(24),
    };

    add_face(
        &mut mesh,
        [
            p([-x, -y, -z]),
            p([x, -y, -z]),
            p([x, -y, z]),
            p([-x, -y, z]),
        ],
        n([0.0, -1.0, 0.0]),
    );

    add_face(
        &mut mesh,
        [
            p([-x, -y, -z]),
            p([-x, y, -z]),
            p([x, y, -z]),
            p([x, -y, -z]),
        ],
        n([0.0, 0.0, -1.0]),
    );

    add_triangle(
        &mut mesh,
        [p([-x, -y, z]), p([-x, y, -z]), p([-x, -y, -z])],
        n([-1.0, 0.0, 0.0]),
    );

    add_triangle(
        &mut mesh,
        [p([x, -y, -z]), p([x, y, -z]), p([x, -y, z])],
        n([1.0, 0.0, 0.0]),
    );

    let base = mesh.vertices.len() as u32;

    let slope_normal = normalize([0.0, z, -y]);

    mesh.vertices.extend([
        UnionVertex {
            position: p([-x, -y, z]),
            normal: slope_normal,
            tex_coord: [0.0, 0.0],
            color: [255; 4],
        },
        UnionVertex {
            position: p([x, -y, z]),
            normal: slope_normal,
            tex_coord: [1.0, 0.0],
            color: [255; 4],
        },
        UnionVertex {
            position: p([x, y, -z]),
            normal: slope_normal,
            tex_coord: [1.0, 1.0],
            color: [255; 4],
        },
        UnionVertex {
            position: p([-x, y, -z]),
            normal: slope_normal,
            tex_coord: [0.0, 1.0],
            color: [255; 4],
        },
    ]);

    mesh.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);

    mesh
}

fn corner_wedge_mesh(size: Vector3) -> UnionMesh {
    let x = size.x.abs() * 0.5;
    let y = size.y.abs() * 0.5;
    let z = size.z.abs() * 0.5;
    let apex = [x, y, -z];
    let bottom = [[-x, -y, -z], [x, -y, -z], [x, -y, z], [-x, -y, z]];
    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(16),
        indices: Vec::with_capacity(30),
    };

    add_face(
        &mut mesh,
        [bottom[3], bottom[0], bottom[1], bottom[2]],
        [0.0, -1.0, 0.0],
    );
    add_triangle(&mut mesh, [bottom[0], apex, bottom[1]], [0.0, 0.0, -1.0]);
    add_triangle(&mut mesh, [bottom[1], apex, bottom[2]], [1.0, 0.0, 0.0]);
    add_triangle(
        &mut mesh,
        [apex, bottom[3], bottom[2]],
        normalize([0.0, z, y]),
    );
    add_triangle(
        &mut mesh,
        [bottom[3], apex, bottom[0]],
        normalize([-y, x, 0.0]),
    );

    mesh
}

fn add_face(mesh: &mut UnionMesh, positions: [[f32; 3]; 4], normal: [f32; 3]) {
    let base = mesh.vertices.len() as u32;
    for (position, tex_coord) in
        positions
            .into_iter()
            .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
    {
        mesh.vertices.push(UnionVertex {
            position,
            normal,
            tex_coord,
            color: [255; 4],
        });
    }
    mesh.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn add_triangle(mesh: &mut UnionMesh, positions: [[f32; 3]; 3], normal: [f32; 3]) {
    let base = mesh.vertices.len() as u32;
    for (position, tex_coord) in positions
        .into_iter()
        .zip([[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]])
    {
        mesh.vertices.push(UnionVertex {
            position,
            normal,
            tex_coord,
            color: [255; 4],
        });
    }
    mesh.indices.extend([base, base + 1, base + 2]);
}

fn sphere_point(latitude: f32, longitude: f32, radii: [f32; 3]) -> [f32; 3] {
    let latitude_cos = latitude.cos();
    [
        radii[0] * latitude_cos * longitude.cos(),
        radii[1] * latitude.sin(),
        radii[2] * latitude_cos * longitude.sin(),
    ]
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if length <= f32::EPSILON {
        [0.0, 1.0, 0.0]
    } else {
        [value[0] / length, value[1] / length, value[2] / length]
    }
}

fn apply_special_mesh_transform(mesh: &mut UnionMesh, instance: &Instance) {
    let scale = property_vector3(instance, "Scale").unwrap_or(Vector3::new(1.0, 1.0, 1.0));
    let offset = property_vector3(instance, "Offset").unwrap_or(Vector3::new(0.0, 0.0, 0.0));
    for vertex in &mut mesh.vertices {
        vertex.position[0] = vertex.position[0] * scale.x + offset.x;
        vertex.position[1] = vertex.position[1] * scale.y + offset.y;
        vertex.position[2] = vertex.position[2] * scale.z + offset.z;
    }
}

fn direct_child(dom: &WeakDom, instance: &Instance, class: &str) -> Option<Ref> {
    instance.children().iter().copied().find(|child_ref| {
        dom.get_by_ref(*child_ref)
            .is_some_and(|child| is_class(child, class))
    })
}

fn is_geometry(instance: &Instance) -> bool {
    matches!(
        instance.class.as_str(),
        "Part" | "MeshPart" | "UnionOperation"
    )
}

fn is_class(instance: &Instance, class: &str) -> bool {
    instance.class.as_str() == class
}

fn reflection_database() -> &'static ReflectionDatabase<'static> {
    rbx_reflection_database::get().unwrap_or_else(|_| rbx_reflection_database::get_bundled())
}

fn roblox_instance_extras(
    dom: &WeakDom,
    instance_ref: Ref,
    database: &ReflectionDatabase<'static>,
) -> Value {
    let Some(instance) = dom.get_by_ref(instance_ref) else {
        return Value::Null;
    };

    let (properties, serialized_properties, property_types) =
        discovered_properties(instance, database);
    let children = instance
        .children()
        .iter()
        .map(|child_ref| roblox_instance_extras(dom, *child_ref, database))
        .collect::<Vec<_>>();

    let mut extras = Map::new();
    extras.insert("className".to_owned(), json!(instance.class.as_str()));
    extras.insert("name".to_owned(), json!(instance.name));
    extras.insert(
        "referent".to_owned(),
        json!(instance.referent().to_string()),
    );
    extras.insert("properties".to_owned(), Value::Object(properties));
    if !serialized_properties.is_empty() {
        extras.insert(
            "serializedProperties".to_owned(),
            Value::Object(serialized_properties),
        );
    }
    if !property_types.is_empty() {
        extras.insert("propertyTypes".to_owned(), Value::Object(property_types));
    }
    if !children.is_empty() {
        extras.insert("children".to_owned(), Value::Array(children));
    }
    Value::Object(extras)
}

fn discovered_properties(
    instance: &Instance,
    database: &ReflectionDatabase<'static>,
) -> (Map<String, Value>, Map<String, Value>, Map<String, Value>) {
    let class_descriptor = database.classes.get(instance.class.as_str());
    let mut property_names = BTreeSet::new();
    let mut reflected_property_types = Map::new();

    if let Some(class_descriptor) = class_descriptor {
        for descriptor in database.superclasses_iter(class_descriptor) {
            for (name, property) in &descriptor.properties {
                let name = (*name).to_owned();
                property_names.insert(name.clone());
                reflected_property_types
                    .insert(name, json!(reflection_data_type_name(&property.data_type)));
            }
        }
    }

    for name in instance.properties.keys() {
        let name = name.to_string();
        property_names.insert(name.clone());
    }

    let mut properties = Map::new();
    let mut serialized_properties = Map::new();
    let mut property_types = Map::new();
    for name in property_names {
        let Some(value) = instance.properties.get(&name.clone().into()) else {
            continue;
        };
        let is_default = class_descriptor
            .and_then(|class_descriptor| database.find_default_property(class_descriptor, &name))
            .is_some_and(|default| value == default);
        if is_default {
            continue;
        }

        let value_json = variant_to_json(value);
        properties.insert(name.clone(), value_json.clone());
        serialized_properties.insert(name.clone(), value_json);
        property_types.insert(
            name.clone(),
            reflected_property_types
                .get(&name)
                .cloned()
                .unwrap_or_else(|| json!(variant_type_name(value))),
        );
    }

    (properties, serialized_properties, property_types)
}

fn reflection_data_type_name(data_type: &DataType<'_>) -> String {
    match data_type {
        DataType::Value(variant_type) => format!("{variant_type:?}"),
        DataType::Enum(enum_name) => format!("Enum<{enum_name}>"),
        _ => "Unknown".to_owned(),
    }
}

fn variant_type_name(value: &Variant) -> String {
    format!("{:?}", value.ty())
}

fn variant_to_json(value: &Variant) -> Value {
    serde_json::to_value(value).unwrap_or_else(|error| {
        json!({
            "type": variant_type_name(value),
            "debug": format!("{value:?}"),
            "serializationError": error.to_string()
        })
    })
}

fn property_asset_id(instance: &Instance, property: &str) -> Option<String> {
    let value = instance.properties.get(&property.into())?;
    let uri = match value {
        Variant::Content(content) => content.as_uri(),
        Variant::ContentId(content_id) => Some(content_id.as_str()),
        _ => None,
    }?;
    parse_asset_id(uri)
}

fn parse_asset_id(uri: &str) -> Option<String> {
    let query_id = uri
        .split_once("id=")
        .map(|(_, value)| value)
        .and_then(|value| value.split(['&', '#', '/']).next());
    let scheme_id = uri
        .strip_prefix("rbxassetid://")
        .or_else(|| uri.strip_prefix("rbxasset://"))
        .and_then(|value| value.split(['?', '#', '/']).next());
    query_id
        .or(scheme_id)
        .filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        })
        .map(str::to_owned)
}

fn property_cframe(instance: &Instance, property: &str) -> Option<CFrame> {
    match instance.properties.get(&property.into())? {
        Variant::CFrame(cframe) => Some(*cframe),
        Variant::OptionalCFrame(cframe) => *cframe,
        _ => None,
    }
}

fn property_vector3(instance: &Instance, property: &str) -> Option<Vector3> {
    match instance.properties.get(&property.into())? {
        Variant::Vector3(vector) => Some(*vector),
        _ => None,
    }
}

fn color_value(value: &Variant) -> Option<[u8; 3]> {
    match value {
        Variant::Color3uint8(color) => Some([color.r, color.g, color.b]),
        Variant::Color3(color) => Some([
            (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]),
        _ => None,
    }
}

fn float_value(value: &Variant) -> Option<f32> {
    match value {
        Variant::Float32(value) => Some(*value),
        Variant::Float64(value) => Some(*value as f32),
        _ => None,
    }
}

fn enum_value(value: &Variant) -> Option<u32> {
    match value {
        Variant::Enum(value) => Some(value.to_u32()),
        Variant::EnumItem(value) => Some(value.value),
        Variant::Int32(value) => (*value >= 0).then_some(*value as u32),
        Variant::Int64(value) => (*value >= 0).then_some(*value as u32),
        _ => None,
    }
}

fn material_name(value: u32) -> String {
    let name = match value {
        256 => "plastic",
        272 => "smoothplastic",
        288 => "neon",
        512 => "wood",
        528 => "woodplanks",
        784 => "marble",
        788 => "basalt",
        800 => "slate",
        804 => "crackedlava",
        816 => "concrete",
        820 => "limestone",
        832 => "granite",
        836 => "pavement",
        848 => "brick",
        864 => "pebble",
        880 => "cobblestone",
        896 => "rock",
        912 => "sandstone",
        1040 => "corrodedmetal",
        1056 => "diamondplate",
        1072 => "foil",
        1088 => "metal",
        1280 => "grass",
        1284 => "leafygrass",
        1296 => "sand",
        1312 => "fabric",
        1328 => "snow",
        1344 => "mud",
        1360 => "ground",
        1376 => "asphalt",
        1392 => "salt",
        1536 => "ice",
        1552 => "glacier",
        1568 => "glass",
        1584 => "forcefield",
        1792 => "air",
        2048 => "water",
        2304 => "cardboard",
        2305 => "carpet",
        2306 => "ceramictiles",
        2307 => "clayrooftiles",
        2308 => "roofshingles",
        2309 => "leather",
        2310 => "plaster",
        2311 => "rubber",
        _ => "plastic",
    };
    name.to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    #[test]
    fn parses_asset_ids_from_roblox_urls() {
        assert_eq!(parse_asset_id("rbxassetid://123"), Some("123".to_owned()));
        assert_eq!(
            parse_asset_id("https://www.roblox.com/asset/?id=456"),
            Some("456".to_owned())
        );
        assert_eq!(parse_asset_id("rbxassetid://not-an-id"), None);
    }

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
    fn corner_wedge_rises_toward_positive_x_negative_z() {
        let mesh = corner_wedge_mesh(Vector3::new(2.0, 4.0, 6.0));

        assert!(
            mesh.vertices
                .iter()
                .filter(|vertex| vertex.position[1] == 2.0)
                .all(|vertex| vertex.position == [1.0, 2.0, -3.0])
        );
    }

    #[test]
    fn wedge_normals_match_dimensions_and_triangle_winding() {
        let mesh = wedge_mesh(Vector3::new(2.0, 4.0, 6.0));

        assert_eq!(mesh.vertices.len(), 18);
        assert_eq!(mesh.indices.len(), 24);
        assert_eq!(mesh.vertices[14].normal, normalize([0.0, 3.0, -2.0]));
        assert!(mesh.indices.chunks_exact(3).all(|triangle| {
            let first = mesh.vertices[triangle[0] as usize].position;
            let second = mesh.vertices[triangle[1] as usize].position;
            let third = mesh.vertices[triangle[2] as usize].position;
            let first_edge = [
                second[0] - first[0],
                second[1] - first[1],
                second[2] - first[2],
            ];
            let second_edge = [
                third[0] - first[0],
                third[1] - first[1],
                third[2] - first[2],
            ];
            let cross = [
                first_edge[1] * second_edge[2] - first_edge[2] * second_edge[1],
                first_edge[2] * second_edge[0] - first_edge[0] * second_edge[2],
                first_edge[0] * second_edge[1] - first_edge[1] * second_edge[0],
            ];
            let normal = mesh.vertices[triangle[0] as usize].normal;
            let area_squared = cross.iter().map(|value| value * value).sum::<f32>();
            let winding_alignment = cross
                .iter()
                .zip(normal)
                .map(|(cross_value, normal_value)| cross_value * normal_value)
                .sum::<f32>();
            area_squared > f32::EPSILON && winding_alignment > f32::EPSILON
        }));
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
    fn discovers_serialized_unknown_and_inherited_properties() {
        let dom = WeakDom::new(
            InstanceBuilder::new("Part")
                .with_name("MetadataPart")
                .with_property("Anchored", true)
                .with_property("CustomMetadata", "kept")
                .with_child(InstanceBuilder::new("Folder").with_name("Child")),
        );
        let extras = roblox_instance_extras(&dom, dom.root_ref(), reflection_database());

        assert_eq!(extras["className"], "Part");
        assert_eq!(extras["properties"]["Anchored"]["Bool"], true);
        assert_eq!(
            extras["serializedProperties"]["CustomMetadata"]["String"],
            "kept"
        );
        assert_eq!(extras["propertyTypes"]["Anchored"], "Bool");
        assert_eq!(extras["children"][0]["name"], "Child");
    }

    #[test]
    fn omits_properties_equal_to_reflection_defaults() {
        let dom = WeakDom::new(
            InstanceBuilder::new("Part")
                .with_property("Anchored", false)
                .with_property("CustomMetadata", "kept"),
        );
        let extras = roblox_instance_extras(&dom, dom.root_ref(), reflection_database());

        assert!(extras["properties"].get("Anchored").is_none());
        assert!(extras["serializedProperties"].get("Anchored").is_none());
        assert!(extras["propertyTypes"].get("Anchored").is_none());
        assert_eq!(extras["properties"]["CustomMetadata"]["String"], "kept");
    }
}
