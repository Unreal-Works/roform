use crate::csg::UnionMesh;
use serde_json::json;

pub(crate) fn union_to_glb(mesh: &UnionMesh) -> Vec<u8> {
    let vertex_stride = 36usize;
    let mut binary =
        Vec::with_capacity(mesh.vertices.len() * vertex_stride + mesh.indices.len() * 4);
    for vertex in &mesh.vertices {
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
    let index_offset = binary.len();
    for index in &mesh.indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    pad_to_four(&mut binary, 0);

    let (min_position, max_position) = position_bounds(mesh);
    let json_value = json!({
        "buffers": [{ "byteLength": binary.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": index_offset, "byteStride": vertex_stride, "target": 34962 },
            { "buffer": 0, "byteOffset": index_offset, "byteLength": mesh.indices.len() * 4, "target": 34963 }
        ],
        "accessors": [
            { "bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": mesh.vertices.len(), "type": "VEC3", "min": min_position.0, "max": max_position.0 },
            { "bufferView": 0, "byteOffset": 12, "componentType": 5126, "count": mesh.vertices.len(), "type": "VEC3" },
            { "bufferView": 0, "byteOffset": 24, "componentType": 5126, "count": mesh.vertices.len(), "type": "VEC2" },
            { "bufferView": 0, "byteOffset": 32, "componentType": 5121, "normalized": true, "count": mesh.vertices.len(), "type": "VEC4" },
            { "bufferView": 1, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2, "COLOR_0": 3 }, "indices": 4, "mode": 4 }] }],
        "nodes": [{ "mesh": 0 }],
        "scenes": [{ "nodes": [0] }],
        "scene": 0
    });
    let mut json_bytes = serde_json::to_vec(&json_value).expect("glTF JSON serializes");
    pad_to_four(&mut json_bytes, b' ');

    let total_length = 12 + 8 + json_bytes.len() + 8 + binary.len();
    let mut glb = Vec::with_capacity(total_length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_length as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&binary);
    glb
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csg::UnionVertex;

    #[test]
    fn writes_a_loadable_glb_header_and_lengths() {
        let mesh = UnionMesh {
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
        };
        let glb = union_to_glb(&mesh);
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
    }
}
