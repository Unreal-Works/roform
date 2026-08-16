use crate::csg::{UnionMesh, UnionVertex};
use rbx_types::Vector3;

const CYLINDER_SEGMENTS: usize = 64;
const SPHERE_RINGS: usize = CYLINDER_SEGMENTS;
const SPHERE_SEGMENTS: usize = CYLINDER_SEGMENTS * 2;

pub(crate) fn primitive_mesh(instance_shape: u32, size: Vector3, studs_per_tile: f32) -> UnionMesh {
    match instance_shape {
        0 => sphere_mesh(size, studs_per_tile),
        2 => cylinder_mesh(size, studs_per_tile),
        3 => wedge_mesh(size, studs_per_tile),
        4 => corner_wedge_mesh(size, studs_per_tile),
        _ => box_mesh(size, studs_per_tile),
    }
}

pub(crate) fn mesh_tangents(mesh: &UnionMesh) -> Vec<[f32; 4]> {
    let mut tangent_sums = vec![[0.0; 3]; mesh.vertices.len()];
    let mut bitangent_sums = vec![[0.0; 3]; mesh.vertices.len()];
    for triangle in mesh.indices.chunks_exact(3) {
        let [first, second, third] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (Some(first_vertex), Some(second_vertex), Some(third_vertex)) = (
            mesh.vertices.get(first),
            mesh.vertices.get(second),
            mesh.vertices.get(third),
        ) else {
            continue;
        };
        let edge_a = subtract(second_vertex.position, first_vertex.position);
        let edge_b = subtract(third_vertex.position, first_vertex.position);
        let uv_a = [
            second_vertex.tex_coord[0] - first_vertex.tex_coord[0],
            second_vertex.tex_coord[1] - first_vertex.tex_coord[1],
        ];
        let uv_b = [
            third_vertex.tex_coord[0] - first_vertex.tex_coord[0],
            third_vertex.tex_coord[1] - first_vertex.tex_coord[1],
        ];
        let determinant = uv_a[0] * uv_b[1] - uv_a[1] * uv_b[0];
        if determinant.abs() <= f32::EPSILON {
            continue;
        }
        let inverse = determinant.recip();
        let tangent = multiply(
            subtract(multiply(edge_a, uv_b[1]), multiply(edge_b, uv_a[1])),
            inverse,
        );
        let bitangent = multiply(
            subtract(multiply(edge_b, uv_a[0]), multiply(edge_a, uv_b[0])),
            inverse,
        );
        for index in [first, second, third] {
            add_assign(&mut tangent_sums[index], tangent);
            add_assign(&mut bitangent_sums[index], bitangent);
        }
    }

    mesh.vertices
        .iter()
        .zip(tangent_sums)
        .zip(bitangent_sums)
        .map(|((vertex, tangent), bitangent)| {
            let normal = normalize(vertex.normal);
            let orthogonal = subtract(tangent, multiply(normal, dot(normal, tangent)));
            let tangent = if dot(orthogonal, orthogonal) <= f32::EPSILON {
                perpendicular(normal)
            } else {
                normalize(orthogonal)
            };
            let handedness = if dot(cross(normal, tangent), bitangent) < 0.0 {
                -1.0
            } else {
                1.0
            };
            [tangent[0], tangent[1], tangent[2], handedness]
        })
        .collect()
}

pub(crate) fn project_flat_mesh_uvs(mesh: &mut UnionMesh, matrix: [f32; 16], studs_per_tile: f32) {
    let tile = studs_per_tile.max(f32::EPSILON);
    for vertex in &mut mesh.vertices {
        let position = transform_position(matrix, vertex.position);
        let normal = normalize(transform_direction(matrix, vertex.normal));
        let (tangent, bitangent) = face_basis(normal);
        vertex.tex_coord = [
            dot(position, tangent) / tile,
            dot(position, bitangent) / tile,
        ];
    }
}

fn box_mesh(size: Vector3, studs_per_tile: f32) -> UnionMesh {
    let x = size.x.abs() * 0.5;
    let y = size.y.abs() * 0.5;
    let z = size.z.abs() * 0.5;
    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(24),
        indices: Vec::with_capacity(36),
    };
    add_face_tiled(
        &mut mesh,
        [[x, -y, -z], [x, y, -z], [x, y, z], [x, -y, z]],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [[-x, -y, z], [-x, y, z], [-x, y, -z], [-x, -y, -z]],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [[-x, y, -z], [-x, y, z], [x, y, z], [x, y, -z]],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [[-x, -y, z], [-x, -y, -z], [x, -y, -z], [x, -y, z]],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]],
        studs_per_tile,
    );
    mesh
}

fn cylinder_mesh(size: Vector3, studs_per_tile: f32) -> UnionMesh {
    let segments = CYLINDER_SEGMENTS;
    let radius = size.y.abs().min(size.z.abs()) * 0.5;
    let half_height = size.x.abs() * 0.5;
    let tile = studs_per_tile.max(f32::EPSILON);
    let circumference = std::f32::consts::TAU * radius;
    let height = size.x.abs();

    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(segments * 12),
        indices: Vec::with_capacity(segments * 12 * 3),
    };

    for segment in 0..segments {
        let next = (segment + 1) % segments;
        let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
        let next_angle = if next == 0 {
            std::f32::consts::TAU
        } else {
            next as f32 / segments as f32 * std::f32::consts::TAU
        };

        let current = [angle.cos(), angle.sin()];
        let following = [next_angle.cos(), next_angle.sin()];

        // +90 degrees around Z: (x, y, z) -> (-y, x, z)
        add_face_with_uvs(
            &mut mesh,
            [
                [half_height, radius * current[0], radius * current[1]],
                [half_height, radius * following[0], radius * following[1]],
                [-half_height, radius * following[0], radius * following[1]],
                [-half_height, radius * current[0], radius * current[1]],
            ],
            [
                [angle / std::f32::consts::TAU * circumference / tile, 0.0],
                [
                    next_angle / std::f32::consts::TAU * circumference / tile,
                    0.0,
                ],
                [
                    next_angle / std::f32::consts::TAU * circumference / tile,
                    height / tile,
                ],
                [
                    angle / std::f32::consts::TAU * circumference / tile,
                    height / tile,
                ],
            ],
            [0.0, current[0], current[1]],
        );

        // Original top cap (y = +half_height) rotated to x = -half_height.
        let cap_positions = [
            [-half_height, 0.0, 0.0],
            [-half_height, radius * following[0], radius * following[1]],
            [-half_height, radius * current[0], radius * current[1]],
        ];
        let cap_uvs = cap_positions
            .map(|position| [(position[1] + radius) / tile, (position[2] + radius) / tile]);
        add_triangle_with_uvs(&mut mesh, cap_positions, cap_uvs, [-1.0, 0.0, 0.0]);
        let base = mesh.vertices.len() as u32 - 3;
        mesh.indices.extend([base, base + 1, base + 2]);

        // Original bottom cap (y = -half_height) rotated to x = +half_height.
        let cap_positions = [
            [half_height, 0.0, 0.0],
            [half_height, radius * current[0], radius * current[1]],
            [half_height, radius * following[0], radius * following[1]],
        ];
        let cap_uvs = cap_positions
            .map(|position| [(position[1] + radius) / tile, (position[2] + radius) / tile]);
        add_triangle_with_uvs(&mut mesh, cap_positions, cap_uvs, [1.0, 0.0, 0.0]);
        let base = mesh.vertices.len() as u32 - 3;
        mesh.indices.extend([base, base + 1, base + 2]);
    }

    mesh
}

fn sphere_mesh(size: Vector3, studs_per_tile: f32) -> UnionMesh {
    let rings = SPHERE_RINGS;
    let segments = SPHERE_SEGMENTS;
    let radii = [size.x.abs() * 0.5, size.y.abs() * 0.5, size.z.abs() * 0.5];
    let tile = studs_per_tile.max(f32::EPSILON);
    let equatorial_radius = (radii[0] + radii[2]) * 0.5;
    let meridian_radius = (radii[0] + radii[1] + radii[2]) / 3.0;
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
            let next_angle = if next == 0 {
                std::f32::consts::TAU
            } else {
                next as f32 / segments as f32 * std::f32::consts::TAU
            };
            let points = [
                sphere_point(lower, angle, radii),
                sphere_point(lower, next_angle, radii),
                sphere_point(upper, next_angle, radii),
                sphere_point(upper, angle, radii),
            ];
            let normals = points.map(|point| {
                normalize([
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
                ])
            });
            let base = mesh.vertices.len() as u32;
            for ((point, normal), (latitude, longitude)) in points.into_iter().zip(normals).zip([
                (lower, angle),
                (lower, next_angle),
                (upper, next_angle),
                (upper, angle),
            ]) {
                mesh.vertices.push(UnionVertex {
                    position: point,
                    normal,
                    tex_coord: [
                        longitude / std::f32::consts::TAU
                            * (std::f32::consts::TAU * equatorial_radius)
                            / tile,
                        (latitude + std::f32::consts::FRAC_PI_2) * meridian_radius / tile,
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

fn wedge_mesh(size: Vector3, studs_per_tile: f32) -> UnionMesh {
    let x = size.x.abs() * 0.5;
    let y = size.y.abs() * 0.5;
    let z = size.z.abs() * 0.5;

    let p = |v: [f32; 3]| [-v[0], v[1], -v[2]];
    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(18),
        indices: Vec::with_capacity(24),
    };

    add_face_tiled(
        &mut mesh,
        [
            p([-x, -y, -z]),
            p([x, -y, -z]),
            p([x, -y, z]),
            p([-x, -y, z]),
        ],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [
            p([-x, -y, -z]),
            p([-x, y, -z]),
            p([x, y, -z]),
            p([x, -y, -z]),
        ],
        studs_per_tile,
    );
    add_triangle_tiled(
        &mut mesh,
        [p([-x, -y, z]), p([-x, y, -z]), p([-x, -y, -z])],
        studs_per_tile,
    );
    add_triangle_tiled(
        &mut mesh,
        [p([x, -y, -z]), p([x, y, -z]), p([x, -y, z])],
        studs_per_tile,
    );
    add_face_tiled(
        &mut mesh,
        [p([-x, -y, z]), p([x, -y, z]), p([x, y, -z]), p([-x, y, -z])],
        studs_per_tile,
    );

    mesh
}

fn corner_wedge_mesh(size: Vector3, studs_per_tile: f32) -> UnionMesh {
    let x = size.x.abs() * 0.5;
    let y = size.y.abs() * 0.5;
    let z = size.z.abs() * 0.5;
    let apex = [x, y, -z];
    let bottom = [[-x, -y, -z], [x, -y, -z], [x, -y, z], [-x, -y, z]];
    let mut mesh = UnionMesh {
        vertices: Vec::with_capacity(16),
        indices: Vec::with_capacity(30),
    };

    add_face_tiled(
        &mut mesh,
        [bottom[3], bottom[0], bottom[1], bottom[2]],
        studs_per_tile,
    );
    add_triangle_tiled(&mut mesh, [bottom[0], apex, bottom[1]], studs_per_tile);
    add_triangle_tiled(&mut mesh, [bottom[1], apex, bottom[2]], studs_per_tile);
    add_triangle_tiled(&mut mesh, [apex, bottom[3], bottom[2]], studs_per_tile);
    add_triangle_tiled(&mut mesh, [bottom[3], apex, bottom[0]], studs_per_tile);

    mesh
}

fn add_face_tiled(mesh: &mut UnionMesh, positions: [[f32; 3]; 4], studs_per_tile: f32) {
    let normal = face_normal(positions);
    let tex_coords = face_tex_coords(positions, normal, studs_per_tile);
    add_face_with_uvs(mesh, positions, tex_coords, normal);
}

fn add_face_with_uvs(
    mesh: &mut UnionMesh,
    positions: [[f32; 3]; 4],
    tex_coords: [[f32; 2]; 4],
    normal: [f32; 3],
) {
    let base = mesh.vertices.len() as u32;
    for (position, tex_coord) in positions.into_iter().zip(tex_coords) {
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

fn add_triangle_tiled(mesh: &mut UnionMesh, positions: [[f32; 3]; 3], studs_per_tile: f32) {
    let normal = face_normal([positions[0], positions[1], positions[2], positions[2]]);
    let tex_coords = face_tex_coords(positions, normal, studs_per_tile);
    add_triangle_with_uvs(mesh, positions, tex_coords, normal);
}

fn add_triangle_with_uvs(
    mesh: &mut UnionMesh,
    positions: [[f32; 3]; 3],
    tex_coords: [[f32; 2]; 3],
    normal: [f32; 3],
) {
    let base = mesh.vertices.len() as u32;
    for (position, tex_coord) in positions.into_iter().zip(tex_coords) {
        mesh.vertices.push(UnionVertex {
            position,
            normal,
            tex_coord,
            color: [255; 4],
        });
    }
    mesh.indices.extend([base, base + 1, base + 2]);
}

fn face_tex_coords<const N: usize>(
    positions: [[f32; 3]; N],
    normal: [f32; 3],
    studs_per_tile: f32,
) -> [[f32; 2]; N] {
    let tile = studs_per_tile.max(f32::EPSILON);
    let (tangent, bitangent) = face_basis(normal);
    let mut tex_coords = positions.map(|position| {
        [
            dot(position, tangent) / tile,
            dot(position, bitangent) / tile,
        ]
    });
    let minimum = tex_coords.iter().fold([f32::INFINITY; 2], |minimum, uv| {
        [minimum[0].min(uv[0]), minimum[1].min(uv[1])]
    });
    for tex_coord in &mut tex_coords {
        tex_coord[0] -= minimum[0];
        tex_coord[1] -= minimum[1];
    }
    tex_coords
}

fn face_basis(normal: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let reference = least_aligned_axis(normal);
    let tangent = normalize(subtract(
        reference,
        multiply(normal, dot(reference, normal)),
    ));
    let bitangent = normalize(cross(normal, tangent));
    (tangent, bitangent)
}

fn face_normal(positions: [[f32; 3]; 4]) -> [f32; 3] {
    normalize(cross(
        subtract(positions[1], positions[0]),
        subtract(positions[2], positions[0]),
    ))
}

fn subtract(first: [f32; 3], second: [f32; 3]) -> [f32; 3] {
    [
        first[0] - second[0],
        first[1] - second[1],
        first[2] - second[2],
    ]
}

fn multiply(value: [f32; 3], factor: f32) -> [f32; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

fn add_assign(target: &mut [f32; 3], value: [f32; 3]) {
    for (target, value) in target.iter_mut().zip(value) {
        *target += value;
    }
}

fn dot(first: [f32; 3], second: [f32; 3]) -> f32 {
    first[0] * second[0] + first[1] * second[1] + first[2] * second[2]
}

fn cross(first: [f32; 3], second: [f32; 3]) -> [f32; 3] {
    [
        first[1] * second[2] - first[2] * second[1],
        first[2] * second[0] - first[0] * second[2],
        first[0] * second[1] - first[1] * second[0],
    ]
}

fn perpendicular(normal: [f32; 3]) -> [f32; 3] {
    normalize(cross(least_aligned_axis(normal), normal))
}

fn least_aligned_axis(normal: [f32; 3]) -> [f32; 3] {
    let absolute = normal.map(f32::abs);
    if absolute[0] <= absolute[1] && absolute[0] <= absolute[2] {
        [1.0, 0.0, 0.0]
    } else if absolute[1] <= absolute[2] {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn transform_position(matrix: [f32; 16], position: [f32; 3]) -> [f32; 3] {
    [
        matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
        matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
        matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
    ]
}

fn transform_direction(matrix: [f32; 16], direction: [f32; 3]) -> [f32; 3] {
    [
        matrix[0] * direction[0] + matrix[4] * direction[1] + matrix[8] * direction[2],
        matrix[1] * direction[0] + matrix[5] * direction[1] + matrix[9] * direction[2],
        matrix[2] * direction[0] + matrix[6] * direction[1] + matrix[10] * direction[2],
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_wedge_rises_toward_positive_x_negative_z() {
        let mesh = corner_wedge_mesh(Vector3::new(2.0, 4.0, 6.0), 2.0);

        assert!(
            mesh.vertices
                .iter()
                .filter(|vertex| vertex.position[1] == 2.0)
                .all(|vertex| vertex.position == [1.0, 2.0, -3.0])
        );
    }

    #[test]
    fn scales_box_uvs_by_planar_dimensions() {
        let mesh = box_mesh(Vector3::new(2.0, 4.0, 6.0), 2.0);

        assert_uv_extent(&mesh.vertices[0..4], [2.0, 3.0]);
        assert_uv_extent(&mesh.vertices[8..12], [1.0, 3.0]);
        assert_uv_extent(&mesh.vertices[16..20], [1.0, 2.0]);
    }

    #[test]
    fn cylinder_uses_high_resolution_radial_topology() {
        let mesh = cylinder_mesh(Vector3::new(10.0, 4.0, 4.0), 2.0);

        assert_eq!(mesh.vertices.len(), CYLINDER_SEGMENTS * 10);
        assert_eq!(mesh.indices.len(), CYLINDER_SEGMENTS * 18);

        let angular_step = std::f32::consts::TAU / CYLINDER_SEGMENTS as f32;
        assert_close(mesh.vertices[10].normal[1], angular_step.cos());
        assert_close(mesh.vertices[10].normal[2], angular_step.sin());
    }

    #[test]
    fn cylinder_side_uvs_follow_the_full_circumference() {
        let mesh = cylinder_mesh(Vector3::new(10.0, 4.0, 4.0), 2.0);
        let segment_step = std::f32::consts::TAU * 2.0 / CYLINDER_SEGMENTS as f32 / 2.0;
        let side_vertices_per_segment = 10;

        assert_close(mesh.vertices[0].tex_coord[0], 0.0);
        assert_close(mesh.vertices[1].tex_coord[0], segment_step);
        assert_close(
            mesh.vertices[side_vertices_per_segment].tex_coord[0],
            segment_step,
        );
        assert_close(
            mesh.vertices[(CYLINDER_SEGMENTS - 1) * side_vertices_per_segment + 1].tex_coord[0],
            std::f32::consts::TAU,
        );
        assert_close(mesh.vertices[2].tex_coord[1], 5.0);
    }

    #[test]
    fn sphere_uvs_follow_surface_arc_lengths() {
        let mesh = sphere_mesh(Vector3::new(4.0, 4.0, 4.0), 2.0);
        let longitude_step = std::f32::consts::TAU / SPHERE_SEGMENTS as f32;

        assert_close(mesh.vertices[0].tex_coord[0], 0.0);
        assert_close(mesh.vertices[1].tex_coord[0], longitude_step);
        assert_close(mesh.vertices[0].tex_coord[1], 0.0);
        assert_close(
            mesh.vertices[2].tex_coord[1],
            std::f32::consts::PI / SPHERE_RINGS as f32,
        );
        assert_close(
            mesh.vertices[(SPHERE_SEGMENTS - 1) * 4 + 1].tex_coord[0],
            std::f32::consts::TAU,
        );
    }

    #[test]
    fn sphere_facets_are_at_least_as_full_as_cylinder_facets() {
        let size = Vector3::new(4.0, 4.0, 4.0);
        let sphere = sphere_mesh(size, 2.0);
        let cylinder = cylinder_mesh(size, 2.0);
        let equator = SPHERE_RINGS / 2 * SPHERE_SEGMENTS * 4;
        let sphere_diagonal_midpoint = midpoint(
            sphere.vertices[equator].position,
            sphere.vertices[equator + 2].position,
        );
        let cylinder_edge_midpoint =
            midpoint(cylinder.vertices[0].position, cylinder.vertices[1].position);

        assert!(
            vector_length(sphere_diagonal_midpoint)
                >= radial_length([cylinder_edge_midpoint[1], cylinder_edge_midpoint[2]]),
            "sphere diagonal is more recessed than the cylinder side"
        );
    }

    #[test]
    fn wedge_slope_uvs_follow_the_diagonal_surface() {
        let mesh = wedge_mesh(Vector3::new(2.0, 4.0, 6.0), 2.0);
        let slope = &mesh.vertices[14..18];

        assert_uv_extent(slope, [1.0, 4.0f32.hypot(6.0) / 2.0]);
    }

    #[test]
    fn wedge_and_box_end_faces_share_uv_axes() {
        let size = Vector3::new(2.0, 4.0, 6.0);
        let box_face = box_mesh(size, 2.0);
        let wedge = wedge_mesh(size, 2.0);

        assert_uv_axes(&box_face.vertices[0..4], 1, 2, 2.0);
        assert_uv_axes(&wedge.vertices[8..11], 1, 2, 2.0);
    }

    #[test]
    fn wedge_and_box_end_faces_share_tangent_axes() {
        let size = Vector3::new(2.0, 4.0, 6.0);
        let box_mesh = box_mesh(size, 2.0);
        let wedge_mesh = wedge_mesh(size, 2.0);
        let box_tangents = mesh_tangents(&box_mesh);
        let wedge_tangents = mesh_tangents(&wedge_mesh);

        for tangent in &box_tangents[0..4] {
            assert_eq!(*tangent, [0.0, 1.0, 0.0, 1.0]);
        }
        for tangent in &wedge_tangents[8..11] {
            assert_eq!(*tangent, [0.0, 1.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn model_space_projection_aligns_rotated_wedge_and_box_faces() {
        let size = Vector3::new(2.0, 4.0, 6.0);
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let rotated = [
            -1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 3.0, 0.0, 0.0, 1.0,
        ];
        let mut box_mesh = box_mesh(size, 2.0);
        let mut wedge_mesh = wedge_mesh(size, 2.0);

        project_flat_mesh_uvs(&mut box_mesh, identity, 2.0);
        project_flat_mesh_uvs(&mut wedge_mesh, rotated, 2.0);

        assert_model_space_uv_axes(&box_mesh.vertices[20..24], identity, 2.0);
        assert_model_space_uv_axes(&wedge_mesh.vertices[14..18], rotated, 2.0);
        for (mesh, matrix, range) in [
            (&box_mesh, identity, 20..24),
            (&wedge_mesh, rotated, 14..18),
        ] {
            let tangents = mesh_tangents(mesh);
            for tangent in &tangents[range] {
                assert_vector_close(
                    transform_direction(matrix, [tangent[0], tangent[1], tangent[2]]),
                    [1.0, 0.0, 0.0],
                );
                assert_close(tangent[3], 1.0);
            }
        }
    }

    #[test]
    fn wedge_normals_match_dimensions_and_triangle_winding() {
        let mesh = wedge_mesh(Vector3::new(2.0, 4.0, 6.0), 2.0);

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

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "expected {expected}, got {actual}"
        );
    }

    fn midpoint(first: [f32; 3], second: [f32; 3]) -> [f32; 3] {
        [
            (first[0] + second[0]) * 0.5,
            (first[1] + second[1]) * 0.5,
            (first[2] + second[2]) * 0.5,
        ]
    }

    fn radial_length(value: [f32; 2]) -> f32 {
        value[0].hypot(value[1])
    }

    fn vector_length(value: [f32; 3]) -> f32 {
        value[0].hypot(value[1]).hypot(value[2])
    }

    fn assert_uv_extent(vertices: &[UnionVertex], expected: [f32; 2]) {
        let minimum = vertices.iter().fold([f32::INFINITY; 2], |minimum, vertex| {
            [
                minimum[0].min(vertex.tex_coord[0]),
                minimum[1].min(vertex.tex_coord[1]),
            ]
        });
        let maximum = vertices
            .iter()
            .fold([f32::NEG_INFINITY; 2], |maximum, vertex| {
                [
                    maximum[0].max(vertex.tex_coord[0]),
                    maximum[1].max(vertex.tex_coord[1]),
                ]
            });
        assert_close(maximum[0] - minimum[0], expected[0]);
        assert_close(maximum[1] - minimum[1], expected[1]);
    }

    fn assert_uv_axes(
        vertices: &[UnionVertex],
        tangent_axis: usize,
        bitangent_axis: usize,
        studs_per_tile: f32,
    ) {
        for pair in vertices.windows(2) {
            assert_close(
                pair[1].tex_coord[0] - pair[0].tex_coord[0],
                (pair[1].position[tangent_axis] - pair[0].position[tangent_axis]) / studs_per_tile,
            );
            assert_close(
                pair[1].tex_coord[1] - pair[0].tex_coord[1],
                (pair[1].position[bitangent_axis] - pair[0].position[bitangent_axis])
                    / studs_per_tile,
            );
        }
    }

    fn assert_model_space_uv_axes(
        vertices: &[UnionVertex],
        matrix: [f32; 16],
        studs_per_tile: f32,
    ) {
        let normal = normalize(transform_direction(matrix, vertices[0].normal));
        let (tangent, bitangent) = face_basis(normal);
        for pair in vertices.windows(2) {
            let first = transform_position(matrix, pair[0].position);
            let second = transform_position(matrix, pair[1].position);
            let offset = subtract(second, first);
            assert_close(
                pair[1].tex_coord[0] - pair[0].tex_coord[0],
                dot(offset, tangent) / studs_per_tile,
            );
            assert_close(
                pair[1].tex_coord[1] - pair[0].tex_coord[1],
                dot(offset, bitangent) / studs_per_tile,
            );
        }
    }

    fn assert_vector_close(actual: [f32; 3], expected: [f32; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert_close(actual, expected);
        }
    }
}
