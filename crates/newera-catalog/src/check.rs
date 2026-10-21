//! Geometry checks inside an imported model: which of its parts pass
//! through one another (a cushion through a rail), and faces with no area.
//!
//! Only crossings count. Two surfaces that touch — a cushion resting on
//! the frame, a joint glued flat — are how furniture is built, so a
//! triangle has to reach more than `tolerance` to each side of the other's
//! plane before the pair is a crossing.

use crate::mesh::{Mesh, MeshPart};

type V3 = [f32; 3];

/// Two parts whose surfaces cross.
#[derive(Debug, Clone, PartialEq)]
pub struct Clash {
    /// Indices into `Mesh::parts`.
    pub a: usize,
    pub b: usize,
    /// Pairs of triangles that cross.
    pub pairs: usize,
    /// A point where they cross, cm in the mesh's frame.
    pub at: V3,
    /// How far one surface reaches past the other, cm (an estimate: the
    /// deepest triangle corner beyond the other triangle's plane).
    pub depth: f32,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn lerp(a: V3, b: V3, t: f32) -> V3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Unit normal and plane offset, `None` for a triangle with no area.
fn plane(t: &[V3; 3]) -> Option<(V3, f32)> {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let len = dot(n, n).sqrt();
    (len > 1e-9).then(|| {
        let n = n.map(|c| c / len);
        (n, dot(n, t[0]))
    })
}

/// Where the edges of `t` cross a plane its corners lie on both sides of,
/// given their signed distances `d`.
fn crossing(t: &[V3; 3], d: [f32; 3]) -> Option<[V3; 2]> {
    let mut points = Vec::with_capacity(2);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        if (d[i] >= 0.0) != (d[j] >= 0.0) {
            points.push(lerp(t[i], t[j], d[i] / (d[i] - d[j])));
        }
    }
    (points.len() == 2).then(|| [points[0], points[1]])
}

/// Whether two triangles pass through each other by more than `tol`, and
/// if so a point of the crossing and how deep it goes.
fn crosses(t1: &[V3; 3], t2: &[V3; 3], tol: f32) -> Option<(V3, f32)> {
    let (n1, o1) = plane(t1)?;
    let (n2, o2) = plane(t2)?;
    let d1 = t1.map(|p| dot(n2, p) - o2);
    let d2 = t2.map(|p| dot(n1, p) - o1);
    let spans = |d: [f32; 3]| {
        let (lo, hi) = d
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        (hi > tol && lo < -tol).then_some(hi.min(-lo))
    };
    let depth = spans(d1)?.max(spans(d2)?);
    let line = cross(n1, n2);
    if dot(line, line) < 1e-12 {
        return None;
    }
    let s1 = crossing(t1, d1)?;
    let s2 = crossing(t2, d2)?;
    let along = |s: [V3; 2]| {
        let (a, b) = (dot(line, s[0]), dot(line, s[1]));
        if a <= b {
            (a, b, s[0], s[1])
        } else {
            (b, a, s[1], s[0])
        }
    };
    let (a0, a1, p0, p1) = along(s1);
    let (b0, b1, ..) = along(s2);
    let (lo, hi) = (a0.max(b0), a1.min(b1));
    let scale = dot(line, line).sqrt();
    // The two segments have to share more than `tol` of the line.
    if (hi - lo) / scale <= tol {
        return None;
    }
    let mid = f32::midpoint(lo, hi);
    let t = if a1 - a0 > 1e-12 {
        (mid - a0) / (a1 - a0)
    } else {
        0.5
    };
    Some((lerp(p0, p1, t), depth))
}

fn triangle(mesh: &Mesh, k: usize) -> Option<[V3; 3]> {
    let tri = mesh.indices.get(k * 3..k * 3 + 3)?;
    Some([
        *mesh.positions.get(tri[0] as usize)?,
        *mesh.positions.get(tri[1] as usize)?,
        *mesh.positions.get(tri[2] as usize)?,
    ])
}

type Bounds = (V3, V3);

fn bounds_of(t: &[V3; 3]) -> Bounds {
    let mut min = t[0];
    let mut max = t[0];
    for p in &t[1..] {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    (min, max)
}

fn overlap(a: Bounds, b: Bounds, pad: f32) -> Option<Bounds> {
    let min: V3 = std::array::from_fn(|k| a.0[k].max(b.0[k]) - pad);
    let max: V3 = std::array::from_fn(|k| a.1[k].min(b.1[k]) + pad);
    (0..3).all(|k| min[k] <= max[k]).then_some((min, max))
}

/// The triangles of `part` whose box meets `region`.
fn inside(mesh: &Mesh, part: &MeshPart, region: Bounds) -> Vec<([V3; 3], Bounds)> {
    (part.start..part.start + part.count)
        .filter_map(|k| triangle(mesh, k))
        .map(|t| (t, bounds_of(&t)))
        .filter(|(_, b)| overlap(*b, region, 0.0).is_some())
        .collect()
}

/// The crossing between two parts, if any, with a spatial grid over the
/// region their boxes share so large parts stay quick.
fn clash(mesh: &Mesh, a: &MeshPart, b: &MeshPart, tol: f32) -> Option<(usize, V3, f32)> {
    let region = overlap(mesh.part_bounds(a)?, mesh.part_bounds(b)?, tol)?;
    let left = inside(mesh, a, region);
    let right = inside(mesh, b, region);
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let size: V3 = std::array::from_fn(|k| (region.1[k] - region.0[k]).max(1e-3));
    #[allow(clippy::cast_precision_loss)]
    let per_axis = (right.len() as f32).cbrt().clamp(1.0, 64.0);
    let cell: V3 = size.map(|s| s / per_axis);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let slot = |p: V3| -> [usize; 3] {
        std::array::from_fn(|k| {
            (((p[k] - region.0[k]) / cell[k])
                .floor()
                .clamp(0.0, per_axis - 1.0)) as usize
        })
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = per_axis as usize;
    let mut grid: Vec<Vec<usize>> = vec![Vec::new(); n * n * n];
    for (k, (_, bounds)) in right.iter().enumerate() {
        let (lo, hi) = (slot(bounds.0), slot(bounds.1));
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    grid[(x * n + y) * n + z].push(k);
                }
            }
        }
    }
    let mut seen = vec![usize::MAX; right.len()];
    let (mut pairs, mut at, mut depth) = (0, [0.0; 3], 0.0f32);
    for (i, (t1, bounds)) in left.iter().enumerate() {
        let (lo, hi) = (slot(bounds.0), slot(bounds.1));
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    for &k in &grid[(x * n + y) * n + z] {
                        if seen[k] == i {
                            continue;
                        }
                        seen[k] = i;
                        if let Some((point, d)) = crosses(t1, &right[k].0, tol) {
                            pairs += 1;
                            if d > depth {
                                depth = d;
                                at = point;
                            }
                        }
                    }
                }
            }
        }
    }
    (pairs > 0).then_some((pairs, at, depth))
}

/// Every pair of parts of `mesh` whose surfaces cross by more than
/// `tolerance` cm, deepest first.
#[must_use]
pub fn clashes(mesh: &Mesh, tolerance: f32) -> Vec<Clash> {
    let mut out = Vec::new();
    for (i, a) in mesh.parts.iter().enumerate() {
        for (j, b) in mesh.parts.iter().enumerate().skip(i + 1) {
            if let Some((pairs, at, depth)) = clash(mesh, a, b, tolerance) {
                out.push(Clash {
                    a: i,
                    b: j,
                    pairs,
                    at,
                    depth,
                });
            }
        }
    }
    out.sort_by(|x, y| y.depth.total_cmp(&x.depth));
    out
}

/// Triangles with no area, per part (`(part index, count)`), for the parts
/// that have any; `usize::MAX` stands for triangles outside every part.
#[must_use]
pub fn degenerate(mesh: &Mesh) -> Vec<(usize, usize)> {
    let owner = |k: usize| {
        mesh.parts
            .iter()
            .position(|p| (p.start..p.start + p.count).contains(&k))
            .unwrap_or(usize::MAX)
    };
    let mut counts: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for k in 0..mesh.indices.len() / 3 {
        if triangle(mesh, k).is_some_and(|t| plane(&t).is_none()) {
            *counts.entry(owner(k)).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box as a part of its own.
    fn part(mesh: &mut Mesh, name: &str, min: [f64; 3], max: [f64; 3]) {
        let start = mesh.begin_part();
        mesh.cuboid(min, max, [0.5; 3]);
        mesh.end_part(name, start);
    }

    #[test]
    fn a_cushion_through_a_rail_is_found_and_a_cushion_on_it_is_not() {
        let mut mesh = Mesh::default();
        // A rail 4 cm tall, a cushion sinking 3 cm into it, another resting on it.
        part(&mut mesh, "travessa", [0.0, 40.0, 0.0], [60.0, 44.0, 5.0]);
        part(&mut mesh, "almofada", [5.0, 41.0, -20.0], [55.0, 50.0, 3.0]);
        part(
            &mut mesh,
            "almofada_apoiada",
            [5.0, 44.0, 10.0],
            [55.0, 50.0, 30.0],
        );
        part(
            &mut mesh,
            "encosto_colado",
            [60.0, 40.0, 0.0],
            [62.0, 80.0, 5.0],
        );
        let found = clashes(&mesh, 0.05);
        assert_eq!(found.len(), 1, "{found:?}");
        let clash = &found[0];
        assert_eq!(
            (
                mesh.parts[clash.a].name.as_str(),
                mesh.parts[clash.b].name.as_str()
            ),
            ("travessa", "almofada")
        );
        // The point is where the two meet.
        assert!((0.0..=60.0).contains(&clash.at[0]), "{clash:?}");
        assert!((40.0..=50.0).contains(&clash.at[1]), "{clash:?}");
        assert!((0.0..=5.0).contains(&clash.at[2]), "{clash:?}");
        assert!(clash.depth > 1.0, "{clash:?}");
    }

    #[test]
    fn a_flat_triangle_is_counted_where_it_belongs() {
        let mut mesh = Mesh::default();
        part(&mut mesh, "pe", [0.0; 3], [2.0; 3]);
        let start = mesh.begin_part();
        mesh.positions
            .extend([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
        mesh.normals.extend([[0.0, 1.0, 0.0]; 3]);
        mesh.colors.extend([[0.5; 3]; 3]);
        let base = u32::try_from(mesh.positions.len() - 3).unwrap();
        mesh.indices.extend([base, base + 1, base + 2]);
        mesh.end_part("linha", start);
        assert_eq!(degenerate(&mesh), [(1, 1)]);
    }

    #[test]
    fn a_dense_model_is_checked_quickly() {
        // 60 parts of 1,200 triangles, side by side, touching.
        let mut mesh = Mesh::default();
        for k in 0..60 {
            let start = mesh.begin_part();
            let x = f64::from(k) * 2.0;
            for i in 0..10 {
                for j in 0..10 {
                    let (y, z) = (f64::from(i), f64::from(j));
                    mesh.cuboid([x, y, z], [x + 2.0, y + 1.0, z + 1.0], [0.5; 3]);
                }
            }
            mesh.end_part(&format!("fio{k}"), start);
        }
        assert!(mesh.indices.len() / 3 > 70_000);
        let started = std::time::Instant::now();
        assert_eq!(clashes(&mesh, 0.05), [] as [crate::check::Clash; 0]);
        assert!(started.elapsed().as_secs() < 20, "{:?}", started.elapsed());
    }
}
