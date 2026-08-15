use crate::csg::{UnionMesh, UnionVertex};
use rbx_types::Vector3;

pub(crate) fn primitive_mesh(instance_shape: u32, size: Vector3, studs_per_tile: f32) -> UnionMesh {
    match instance_shape {
        0 => sphere_mesh(size, studs_per_tile),
        2 => cylinder_mesh(size, studs_per_tile),
        3 => wedge_mesh(size, studs_per_tile),
        4 => corner_wedge_mesh(size, studs_per_tile),
        _ => box_mesh(size, studs_per_tile),
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
    let segments = 24usize;
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
    let rings = 10usize;
    let segments = 24usize;
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
    let tex_coords = quad_tex_coords(positions, studs_per_tile);
    let normal = face_normal(positions);
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
    let tex_coords = triangle_tex_coords(positions, studs_per_tile);
    let normal = face_normal([positions[0], positions[1], positions[2], positions[2]]);
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

fn quad_tex_coords(positions: [[f32; 3]; 4], studs_per_tile: f32) -> [[f32; 2]; 4] {
    let tile = studs_per_tile.max(f32::EPSILON);
    let tangent = normalize(subtract(positions[1], positions[0]));
    let bitangent = normalize(subtract(positions[3], positions[0]));
    positions.map(|position| {
        let offset = subtract(position, positions[0]);
        [dot(offset, tangent) / tile, dot(offset, bitangent) / tile]
    })
}

fn triangle_tex_coords(positions: [[f32; 3]; 3], studs_per_tile: f32) -> [[f32; 2]; 3] {
    let tile = studs_per_tile.max(f32::EPSILON);
    let tangent_edge = subtract(positions[1], positions[0]);
    let tangent_length = length(tangent_edge);
    let tangent = normalize(tangent_edge);
    let second_edge = subtract(positions[2], positions[0]);
    let tangent_offset = dot(second_edge, tangent);
    let bitangent = normalize(subtract(second_edge, multiply(tangent, tangent_offset)));
    let bitangent_offset = dot(second_edge, bitangent);
    [
        [0.0, 0.0],
        [tangent_length / tile, 0.0],
        [tangent_offset / tile, bitangent_offset / tile],
    ]
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

fn length(value: [f32; 3]) -> f32 {
    dot(value, value).sqrt()
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

        assert_eq!(
            &mesh.vertices[0..4]
                .iter()
                .map(|vertex| vertex.tex_coord)
                .collect::<Vec<_>>(),
            &[[0.0, 0.0], [2.0, 0.0], [2.0, 3.0], [0.0, 3.0]]
        );
        assert_eq!(
            &mesh.vertices[8..12]
                .iter()
                .map(|vertex| vertex.tex_coord)
                .collect::<Vec<_>>(),
            &[[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [0.0, 1.0]]
        );
        assert_eq!(
            &mesh.vertices[16..20]
                .iter()
                .map(|vertex| vertex.tex_coord)
                .collect::<Vec<_>>(),
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]]
        );
    }

    #[test]
    fn cylinder_side_uvs_follow_the_full_circumference() {
        let mesh = cylinder_mesh(Vector3::new(10.0, 4.0, 4.0), 2.0);
        let segment_step = std::f32::consts::TAU * 2.0 / 24.0 / 2.0;
        let side_vertices_per_segment = 10;

        assert_close(mesh.vertices[0].tex_coord[0], 0.0);
        assert_close(mesh.vertices[1].tex_coord[0], segment_step);
        assert_close(
            mesh.vertices[side_vertices_per_segment].tex_coord[0],
            segment_step,
        );
        assert_close(
            mesh.vertices[23 * side_vertices_per_segment + 1].tex_coord[0],
            std::f32::consts::TAU,
        );
        assert_close(mesh.vertices[2].tex_coord[1], 5.0);
    }

    #[test]
    fn sphere_uvs_follow_surface_arc_lengths() {
        let mesh = sphere_mesh(Vector3::new(4.0, 4.0, 4.0), 2.0);
        let longitude_step = std::f32::consts::TAU / 24.0;

        assert_close(mesh.vertices[0].tex_coord[0], 0.0);
        assert_close(mesh.vertices[1].tex_coord[0], longitude_step);
        assert_close(mesh.vertices[0].tex_coord[1], 0.0);
        assert_close(mesh.vertices[2].tex_coord[1], std::f32::consts::PI / 10.0);
        assert_close(
            mesh.vertices[23 * 4 + 1].tex_coord[0],
            std::f32::consts::TAU,
        );
    }

    #[test]
    fn wedge_slope_uvs_follow_the_diagonal_surface() {
        let mesh = wedge_mesh(Vector3::new(2.0, 4.0, 6.0), 2.0);
        let slope = &mesh.vertices[14..18];

        assert_eq!(slope[0].tex_coord, [0.0, 0.0]);
        assert_close(slope[1].tex_coord[0], 1.0);
        assert_close(slope[3].tex_coord[1], 4.0f32.hypot(6.0) / 2.0);
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
}
