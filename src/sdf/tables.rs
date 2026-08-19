#[cfg_attr(not(test), allow(dead_code))]
pub const CORNER_OFFSETS: [[u32; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];

pub const EDGE_CORNERS: [[u32; 2]; 12] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

const FACES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [3, 7, 6, 2],
    [0, 4, 7, 3],
    [1, 2, 6, 5],
];

pub const MAX_TRIS_PER_CELL: usize = 5;
pub const TRI_TABLE_ROW: usize = MAX_TRIS_PER_CELL * 3 + 1;

const NO_EDGE: usize = usize::MAX;

fn edge_index(a: usize, b: usize) -> usize {
    EDGE_CORNERS
        .iter()
        .position(|e| {
            (e[0] as usize == a && e[1] as usize == b) || (e[0] as usize == b && e[1] as usize == a)
        })
        .expect("corner pair is not a cube edge")
}

fn corner_is_inside(mask: u8, corner: usize) -> bool {
    mask & (1 << corner) != 0
}

fn cell_triangles(mask: u8) -> Vec<usize> {
    let mut next = [NO_EDGE; 12];

    for face in FACES {
        let mut crossings: Vec<(usize, bool)> = Vec::new();
        for k in 0..4 {
            let a = face[k];
            let b = face[(k + 1) % 4];
            let inside_a = corner_is_inside(mask, a);
            let inside_b = corner_is_inside(mask, b);
            if inside_a != inside_b {
                crossings.push((edge_index(a, b), !inside_a && inside_b));
            }
        }

        let n = crossings.len();
        for i in 0..n {
            let (edge, is_entry) = crossings[i];
            if !is_entry {
                continue;
            }
            for step in 1..=n {
                let (exit_edge, exit_is_entry) = crossings[(i + step) % n];
                if !exit_is_entry {
                    next[edge] = exit_edge;
                    break;
                }
            }
        }
    }

    let mut visited = [false; 12];
    let mut triangles = Vec::new();

    for start in 0..12 {
        if next[start] == NO_EDGE || visited[start] {
            continue;
        }
        let mut ring = Vec::new();
        let mut current = start;
        loop {
            assert!(
                !visited[current],
                "marching cubes ring is not a simple cycle"
            );
            visited[current] = true;
            ring.push(current);
            current = next[current];
            assert_ne!(current, NO_EDGE, "marching cubes ring is not closed");
            if current == start {
                break;
            }
        }
        for i in 1..ring.len() - 1 {
            triangles.push(ring[0]);
            triangles.push(ring[i]);
            triangles.push(ring[i + 1]);
        }
    }

    triangles
}

pub fn build_tri_table() -> Vec<i32> {
    let mut table = vec![-1i32; 256 * TRI_TABLE_ROW];
    for mask in 0..256usize {
        let triangles = cell_triangles(mask as u8);
        assert!(
            triangles.len() <= MAX_TRIS_PER_CELL * 3,
            "case {mask} produced {} indices, more than MAX_TRIS_PER_CELL allows",
            triangles.len()
        );
        let row = mask * TRI_TABLE_ROW;
        table[row] = (triangles.len() / 3) as i32;
        for (i, edge) in triangles.iter().enumerate() {
            table[row + 1 + i] = *edge as i32;
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    const RES: usize = 16;

    fn sample(x: usize, y: usize, z: usize) -> f32 {
        let p = [
            x as f32 / (RES - 1) as f32 - 0.5,
            y as f32 / (RES - 1) as f32 - 0.5,
            z as f32 / (RES - 1) as f32 - 0.5,
        ];
        (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 0.31
    }

    fn grid_edge_key(cell: [usize; 3], edge: usize) -> ([usize; 3], usize) {
        let [ca, cb] = EDGE_CORNERS[edge];
        let a = CORNER_OFFSETS[ca as usize];
        let b = CORNER_OFFSETS[cb as usize];
        let axis = (0..3)
            .find(|&i| a[i] != b[i])
            .expect("edge varies on one axis");
        let lo = core::array::from_fn(|i| cell[i] + a[i].min(b[i]) as usize);
        (lo, axis)
    }

    fn march() -> (Vec<[f32; 3]>, Vec<[usize; 3]>) {
        let table = build_tri_table();
        let mut vertices = Vec::new();
        let mut vertex_ids: HashMap<([usize; 3], usize), usize> = HashMap::new();
        let mut triangles = Vec::new();

        for z in 0..RES - 1 {
            for y in 0..RES - 1 {
                for x in 0..RES - 1 {
                    let values: [f32; 8] = core::array::from_fn(|i| {
                        let o = CORNER_OFFSETS[i];
                        sample(x + o[0] as usize, y + o[1] as usize, z + o[2] as usize)
                    });
                    let mut mask = 0u8;
                    for (i, v) in values.iter().enumerate() {
                        if *v < 0.0 {
                            mask |= 1 << i;
                        }
                    }

                    let row = mask as usize * TRI_TABLE_ROW;
                    let tri_count = table[row] as usize;
                    for t in 0..tri_count {
                        let mut corners = [0usize; 3];
                        for (k, slot) in corners.iter_mut().enumerate() {
                            let edge = table[row + 1 + t * 3 + k] as usize;
                            let key = grid_edge_key([x, y, z], edge);
                            *slot = *vertex_ids.entry(key).or_insert_with(|| {
                                let [ca, cb] = EDGE_CORNERS[edge];
                                let (ca, cb) = (ca as usize, cb as usize);
                                let (va, vb) = (values[ca], values[cb]);
                                let f = va / (va - vb);
                                let pa = CORNER_OFFSETS[ca];
                                let pb = CORNER_OFFSETS[cb];
                                let position = core::array::from_fn(|i| {
                                    let base = [x, y, z][i] as f32;
                                    let a = base + pa[i] as f32;
                                    let b = base + pb[i] as f32;
                                    (a + (b - a) * f) / (RES - 1) as f32 - 0.5
                                });
                                vertices.push(position);
                                vertices.len() - 1
                            });
                        }
                        triangles.push(corners);
                    }
                }
            }
        }

        (vertices, triangles)
    }

    #[test]
    fn every_case_stays_within_the_declared_triangle_budget() {
        let table = build_tri_table();
        assert_eq!(table.len(), 256 * TRI_TABLE_ROW);
        for mask in 0..256usize {
            let count = table[mask * TRI_TABLE_ROW];
            assert!((0..=MAX_TRIS_PER_CELL as i32).contains(&count));
        }
        assert_eq!(table[0], 0);
        assert_eq!(table[255 * TRI_TABLE_ROW], 0);
    }

    #[test]
    fn sphere_surface_is_a_closed_manifold() {
        let (_, triangles) = march();
        assert!(!triangles.is_empty());

        let mut directed: HashSet<(usize, usize)> = HashSet::new();
        for t in &triangles {
            for k in 0..3 {
                let edge = (t[k], t[(k + 1) % 3]);
                assert!(
                    directed.insert(edge),
                    "directed edge {edge:?} emitted twice"
                );
            }
        }
        for &(a, b) in &directed {
            assert!(
                directed.contains(&(b, a)),
                "edge ({a}, {b}) has no opposite twin, so the surface has a boundary"
            );
        }
    }

    #[test]
    fn sphere_triangles_wind_outward() {
        let (vertices, triangles) = march();
        for t in &triangles {
            let (a, b, c) = (vertices[t[0]], vertices[t[1]], vertices[t[2]]);
            let ab = core::array::from_fn::<f32, 3, _>(|i| b[i] - a[i]);
            let ac = core::array::from_fn::<f32, 3, _>(|i| c[i] - a[i]);
            let normal = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let centroid: [f32; 3] = core::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0);
            let dot: f32 = (0..3).map(|i| normal[i] * centroid[i]).sum();
            assert!(dot > 0.0, "triangle {t:?} winds inward");
        }
    }
}
