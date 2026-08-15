use rbx_mesh::{
    mesh::{Face2, Lod3, Mesh, Vertex2, Vertices2},
    read_mesh_versioned, read_union_graphics_versioned,
    union_graphics::UnionGraphics,
};
use serde::Serialize;
use std::{io::Cursor, ops::Range};
use thiserror::Error;

const CSGMDL2_MAGIC: &[u8] = b"\x15\x7d\x29\x15\x75\x6c\x32\x04\x34\x69";
const CSGMDL4_MAGIC: &[u8] = b"\x15\x7d\x29\x15\x75\x6c\x34\x04\x34\x69";
const CSGMDL5_MAGIC: &[u8] = b"\x15\x7d\x29\x15\x75\x6c\x35\x04\x34\x69";
const CACHED_MESH_MAGIC: &[u8] = b"ROFMESH1";
const CACHED_MESH_HEADER_LENGTH: usize = CACHED_MESH_MAGIC.len() + 8;
const CACHED_VERTEX_LENGTH: usize = 36;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct UnionVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tex_coord: [f32; 2],
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct UnionMesh {
    pub vertices: Vec<UnionVertex>,
    pub indices: Vec<u32>,
}

pub(crate) fn encode_cached_mesh(mesh: &UnionMesh) -> Result<Vec<u8>, String> {
    let vertex_count = u32::try_from(mesh.vertices.len())
        .map_err(|_| "decoded mesh has too many vertices to cache".to_owned())?;
    let index_count = u32::try_from(mesh.indices.len())
        .map_err(|_| "decoded mesh has too many indices to cache".to_owned())?;
    let vertex_bytes = mesh
        .vertices
        .len()
        .checked_mul(CACHED_VERTEX_LENGTH)
        .ok_or_else(|| "decoded mesh cache is too large".to_owned())?;
    let index_bytes = mesh
        .indices
        .len()
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or_else(|| "decoded mesh cache is too large".to_owned())?;
    let capacity = CACHED_MESH_HEADER_LENGTH
        .checked_add(vertex_bytes)
        .and_then(|length| length.checked_add(index_bytes))
        .ok_or_else(|| "decoded mesh cache is too large".to_owned())?;

    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(CACHED_MESH_MAGIC);
    bytes.extend_from_slice(&vertex_count.to_le_bytes());
    bytes.extend_from_slice(&index_count.to_le_bytes());
    for vertex in &mesh.vertices {
        for value in vertex.position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.normal {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.tex_coord {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&vertex.color);
    }
    for index in &mesh.indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn decode_cached_mesh(bytes: &[u8]) -> Result<UnionMesh, String> {
    if !bytes.starts_with(CACHED_MESH_MAGIC) {
        return Err("cached mesh has an unsupported format".to_owned());
    }
    let mut offset = CACHED_MESH_MAGIC.len();
    let vertex_count = usize::try_from(read_cached_u32(bytes, &mut offset)?)
        .map_err(|_| "cached mesh vertex count is too large".to_owned())?;
    let index_count = usize::try_from(read_cached_u32(bytes, &mut offset)?)
        .map_err(|_| "cached mesh index count is too large".to_owned())?;
    let vertex_bytes = vertex_count
        .checked_mul(CACHED_VERTEX_LENGTH)
        .ok_or_else(|| "cached mesh is too large".to_owned())?;
    let index_bytes = index_count
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or_else(|| "cached mesh is too large".to_owned())?;
    let expected_length = CACHED_MESH_HEADER_LENGTH
        .checked_add(vertex_bytes)
        .and_then(|length| length.checked_add(index_bytes))
        .ok_or_else(|| "cached mesh is too large".to_owned())?;
    if bytes.len() != expected_length {
        return Err("cached mesh is truncated or has trailing data".to_owned());
    }

    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        vertices.push(UnionVertex {
            position: [
                read_cached_f32(bytes, &mut offset)?,
                read_cached_f32(bytes, &mut offset)?,
                read_cached_f32(bytes, &mut offset)?,
            ],
            normal: [
                read_cached_f32(bytes, &mut offset)?,
                read_cached_f32(bytes, &mut offset)?,
                read_cached_f32(bytes, &mut offset)?,
            ],
            tex_coord: [
                read_cached_f32(bytes, &mut offset)?,
                read_cached_f32(bytes, &mut offset)?,
            ],
            color: read_cached_color(bytes, &mut offset)?,
        });
    }
    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        indices.push(read_cached_u32(bytes, &mut offset)?);
    }
    let mesh = UnionMesh { vertices, indices };
    validate_cached_mesh(&mesh)?;
    Ok(mesh)
}

fn read_cached_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "cached mesh offset overflow".to_owned())?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| "cached mesh is truncated".to_owned())?;
    *offset = end;
    Ok(u32::from_le_bytes(
        value.try_into().expect("u32 slice length"),
    ))
}

fn read_cached_f32(bytes: &[u8], offset: &mut usize) -> Result<f32, String> {
    Ok(f32::from_bits(read_cached_u32(bytes, offset)?))
}

fn read_cached_color(bytes: &[u8], offset: &mut usize) -> Result<[u8; 4], String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "cached mesh offset overflow".to_owned())?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| "cached mesh is truncated".to_owned())?;
    *offset = end;
    Ok(value.try_into().expect("color slice length"))
}

fn validate_cached_mesh(mesh: &UnionMesh) -> Result<(), String> {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3)
    {
        return Err("cached mesh contains no triangle geometry".to_owned());
    }
    for index in &mesh.indices {
        if (*index as usize) >= mesh.vertices.len() {
            return Err(format!(
                "cached mesh references vertex index {index}, but has {} vertices",
                mesh.vertices.len()
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum CsgError {
    #[error("union graphics payload is empty")]
    Empty,
    #[error("union graphics payload is not a supported version: {0}")]
    Decode(String),
    #[error("union graphics payload contains no triangle geometry")]
    NoGeometry,
    #[error(
        "union graphics payload references vertex index {index}, but has {vertex_count} vertices"
    )]
    InvalidIndex { index: u32, vertex_count: usize },
}

#[derive(Debug, Error)]
pub(crate) enum MeshError {
    #[error("mesh payload is empty")]
    Empty,
    #[error("mesh payload is not a supported version: {0}")]
    Decode(String),
    #[error("mesh payload contains no triangle geometry")]
    NoGeometry,
    #[error("mesh payload references vertex index {index}, but has {vertex_count} vertices")]
    InvalidIndex { index: u32, vertex_count: usize },
    #[error("mesh payload contains invalid LOD ranges for {face_count} faces")]
    InvalidLods { face_count: usize },
}

pub(crate) fn payload_version(bytes: &[u8]) -> String {
    if bytes.starts_with(b"CSGK") {
        return "CSGK".to_owned();
    }
    if bytes.starts_with(CSGMDL2_MAGIC) {
        return "CSGMDL2".to_owned();
    }
    if bytes.starts_with(CSGMDL4_MAGIC) {
        return "CSGMDL4".to_owned();
    }
    if bytes.starts_with(CSGMDL5_MAGIC) {
        return "CSGMDL5".to_owned();
    }
    "unknown".to_owned()
}

pub(crate) fn decode_mesh_payload(bytes: &[u8]) -> Result<UnionMesh, String> {
    let version = payload_version(bytes);
    match version.as_str() {
        "CSGK" | "CSGMDL2" | "CSGMDL4" | "CSGMDL5" => {
            decode_union_graphics(bytes).map_err(|error| format!("{version}: {error}"))
        }
        _ => decode_mesh(bytes).map_err(|error| format!("{version}: {error}")),
    }
}

pub(crate) fn decode_union_graphics(bytes: &[u8]) -> Result<UnionMesh, CsgError> {
    if bytes.is_empty() {
        return Err(CsgError::Empty);
    }
    let graphics = read_union_graphics_versioned(Cursor::new(bytes))
        .map_err(|error| CsgError::Decode(error.to_string()))?;
    let mesh = match graphics {
        UnionGraphics::CSGK(_) => return Err(CsgError::NoGeometry),
        UnionGraphics::V2(value) => UnionMesh {
            vertices: value
                .mesh
                .vertices
                .into_iter()
                .map(|vertex| UnionVertex {
                    position: vertex.pos,
                    normal: vertex.norm,
                    tex_coord: vertex.tex,
                    color: vertex.color,
                })
                .collect(),
            indices: value
                .mesh
                .faces
                .into_iter()
                .flat_map(|face| face.into_iter().map(|vertex| vertex.0))
                .collect(),
        },
        UnionGraphics::V4(value) => UnionMesh {
            vertices: value
                .mesh
                .vertices
                .into_iter()
                .map(|vertex| UnionVertex {
                    position: vertex.pos,
                    normal: vertex.norm,
                    tex_coord: vertex.tex,
                    color: vertex.color,
                })
                .collect(),
            indices: value
                .mesh
                .faces
                .into_iter()
                .flat_map(|face| face.into_iter().map(|vertex| vertex.0))
                .collect(),
        },
        UnionGraphics::V5(value) => {
            let vertices = value
                .positions
                .iter()
                .enumerate()
                .map(|(index, position)| UnionVertex {
                    position: *position,
                    normal: value
                        .normals
                        .get(index)
                        .map(|normal| normal.0)
                        .unwrap_or([0.0, 1.0, 0.0]),
                    tex_coord: value.tex.get(index).copied().unwrap_or([0.0, 0.0]),
                    color: value
                        .colors
                        .get(index)
                        .copied()
                        .unwrap_or([255, 255, 255, 255]),
                })
                .collect();
            UnionMesh {
                vertices,
                indices: value.faces.indices,
            }
        }
    };

    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        return Err(CsgError::NoGeometry);
    }
    if mesh.indices.len() % 3 != 0 {
        return Err(CsgError::NoGeometry);
    }
    for index in &mesh.indices {
        if (*index as usize) >= mesh.vertices.len() {
            return Err(CsgError::InvalidIndex {
                index: *index,
                vertex_count: mesh.vertices.len(),
            });
        }
    }
    Ok(mesh)
}

pub(crate) fn decode_mesh(bytes: &[u8]) -> Result<UnionMesh, MeshError> {
    if bytes.is_empty() {
        return Err(MeshError::Empty);
    }
    let mesh = read_mesh_versioned(Cursor::new(bytes))
        .map_err(|error| MeshError::Decode(error.to_string()))?;
    let mesh = match mesh {
        Mesh::V1(value) => {
            let vertex_count = value.vertices.len();
            UnionMesh {
                vertices: value
                    .vertices
                    .into_iter()
                    .map(|vertex| UnionVertex {
                        position: vertex.pos,
                        normal: vertex.norm,
                        tex_coord: [vertex.tex[0], vertex.tex[1]],
                        color: [255, 255, 255, 255],
                    })
                    .collect(),
                indices: (0..vertex_count as u32).collect(),
            }
        }
        Mesh::V2(value) => {
            let face_range = 0..value.faces.len();
            mesh_from_vertices_and_faces(&value.vertices, &value.faces, face_range)
        }
        Mesh::V3(value) => {
            let face_range = highest_detail_face_range(&value.lods, value.faces.len())?;
            mesh_from_vertices_and_faces(&value.vertices, &value.faces, face_range)
        }
        Mesh::V4(value) => {
            let face_range = highest_detail_face_range(&value.lods, value.faces.len())?;
            mesh_from_full_vertices_and_faces(&value.vertices, &value.faces, face_range)
        }
        Mesh::V5(value) => {
            let face_range = highest_detail_face_range(&value.lods, value.faces.len())?;
            mesh_from_full_vertices_and_faces(&value.vertices, &value.faces, face_range)
        }
        Mesh::V7(value) => {
            let face_range = 0..value.faces.len();
            mesh_from_full_vertices_and_faces(&value.vertices, &value.faces, face_range)
        }
    };
    if mesh.vertices.is_empty() || mesh.indices.is_empty() || mesh.indices.len() % 3 != 0 {
        return Err(MeshError::NoGeometry);
    }
    for index in &mesh.indices {
        if (*index as usize) >= mesh.vertices.len() {
            return Err(MeshError::InvalidIndex {
                index: *index,
                vertex_count: mesh.vertices.len(),
            });
        }
    }
    Ok(mesh)
}

fn highest_detail_face_range(lods: &[Lod3], face_count: usize) -> Result<Range<usize>, MeshError> {
    if lods.iter().any(|lod| lod.0 as usize > face_count)
        || lods.windows(2).any(|window| window[0].0 > window[1].0)
    {
        return Err(MeshError::InvalidLods { face_count });
    }
    let start = lods.first().map(|lod| lod.0 as usize).unwrap_or(0);
    let end = lods.get(1).map(|lod| lod.0 as usize).unwrap_or(face_count);
    Ok(start..end)
}

fn mesh_from_vertices_and_faces(
    vertices: &Vertices2,
    faces: &[Face2],
    face_range: Range<usize>,
) -> UnionMesh {
    let vertices = match vertices {
        Vertices2::Full(vertices) => full_vertices(vertices),
        Vertices2::Truncated(vertices) => vertices
            .iter()
            .map(|vertex| UnionVertex {
                position: vertex.pos,
                normal: vertex.norm,
                tex_coord: vertex.tex,
                color: [255, 255, 255, 255],
            })
            .collect(),
    };
    let indices = faces[face_range]
        .iter()
        .flat_map(|face| face.0.iter().map(|vertex| vertex.0))
        .collect();
    UnionMesh { vertices, indices }
}

fn mesh_from_full_vertices_and_faces(
    vertices: &[Vertex2],
    faces: &[Face2],
    face_range: Range<usize>,
) -> UnionMesh {
    UnionMesh {
        vertices: full_vertices(vertices),
        indices: faces[face_range]
            .iter()
            .flat_map(|face| face.0.iter().map(|vertex| vertex.0))
            .collect(),
    }
}

fn full_vertices(vertices: &[Vertex2]) -> Vec<UnionVertex> {
    vertices
        .iter()
        .map(|vertex| UnionVertex {
            position: vertex.pos,
            normal: vertex.norm,
            tex_coord: vertex.tex,
            color: vertex.color,
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn test_v5_triangle_payload() -> Vec<u8> {
    let mut bytes = b"\x15\x7d\x29\x15\x75\x6c\x35\x04\x34\x69".to_vec();
    bytes.extend_from_slice(&3u16.to_le_bytes());
    for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&[0, 1, 1]);
    bytes.push(2);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_non_geometry_payloads() {
        assert!(matches!(decode_union_graphics(&[]), Err(CsgError::Empty)));
        let mut csgk = b"CSGK".to_vec();
        csgk.extend_from_slice(&[b'0'; 32]);
        assert!(matches!(
            decode_union_graphics(&csgk),
            Err(CsgError::NoGeometry)
        ));
    }

    #[test]
    fn decodes_v5_triangle_payload() {
        let payload = test_v5_triangle_payload();
        assert_eq!(payload_version(&payload), "CSGMDL5");
        let mesh = decode_union_graphics(&payload).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    #[test]
    fn decodes_mesh_v1_triangle_payload() {
        let payload = b"version 1.00\r\n1\r\n[0,0,0][0,1,0][0,0,0][1,0,0][0,1,0][1,0,0][0,0,1][0,1,0][0,1,1]\r\n";
        let mesh = decode_mesh(payload).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    #[test]
    fn round_trips_cached_meshes() {
        let mesh = UnionMesh {
            vertices: vec![
                UnionVertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    tex_coord: [0.0, 0.0],
                    color: [255, 128, 64, 255],
                },
                UnionVertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    tex_coord: [1.0, 0.0],
                    color: [255, 128, 64, 255],
                },
                UnionVertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    tex_coord: [0.0, 1.0],
                    color: [255, 128, 64, 255],
                },
            ],
            indices: vec![0, 1, 2],
        };
        let encoded = encode_cached_mesh(&mesh).unwrap();
        assert_eq!(&encoded[..CACHED_MESH_MAGIC.len()], CACHED_MESH_MAGIC);
        assert_eq!(decode_cached_mesh(&encoded).unwrap(), mesh);
    }

    #[test]
    fn selects_highest_detail_mesh_lod() {
        assert_eq!(
            highest_detail_face_range(&[Lod3(0), Lod3(394), Lod3(504), Lod3(576)], 576).unwrap(),
            0..394
        );
        assert_eq!(highest_detail_face_range(&[], 7).unwrap(), 0..7);
    }

    #[test]
    fn classifies_known_and_unknown_payload_headers() {
        assert_eq!(payload_version(b"CSGK\0"), "CSGK");
        assert_eq!(payload_version(CSGMDL2_MAGIC), "CSGMDL2");
        assert_eq!(payload_version(CSGMDL4_MAGIC), "CSGMDL4");
        assert_eq!(payload_version(CSGMDL5_MAGIC), "CSGMDL5");
        assert_eq!(payload_version(b"future-csg"), "unknown");
    }
}
