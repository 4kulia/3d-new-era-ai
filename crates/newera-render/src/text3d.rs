//! Text as geometry for the 3D scene: glyph outlines of the bundled font are
//! flattened and triangulated, so labels and dimension lengths look the same
//! in the GPU view, software renders, photos and exported models.

use ab_glyph::{Font, FontRef, OutlineCurve};
use geo::{Contains, TriangulateEarcut};

/// Font size (em) to the height of the letters, like the plan drawing.
const EM_TO_HEIGHT: f32 = 1.12;
/// Segments per quadratic or cubic curve.
const CURVE_STEPS: usize = 4;

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("bundled font is valid")
}

/// Triangles of `text` in a local plane (x right, y up), `size` being the
/// font size in the same units. The last line's baseline sits on y = 0 and
/// `align` places it horizontally: 0 left, 0.5 centered, 1 right of x = 0.
pub fn text_triangles(text: &str, size: f32, align: f32) -> Vec<[[f32; 2]; 3]> {
    let font = font();
    let scale = size * EM_TO_HEIGHT / font.height_unscaled();
    let line_height = (font.height_unscaled() + font.line_gap_unscaled()) * scale;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let width: f32 = line
            .chars()
            .map(|c| font.h_advance_unscaled(font.glyph_id(c)) * scale)
            .sum();
        #[allow(clippy::cast_precision_loss)]
        let baseline = line_height * (lines.len() - 1 - row) as f32;
        let mut x = -width * align;
        for c in line.chars() {
            let id = font.glyph_id(c);
            if let Some(outline) = font.outline(id) {
                let contours = contours(&outline.curves, |p| {
                    (x + p.x * scale, baseline + p.y * scale)
                });
                glyph_triangles(&contours, &mut out);
            }
            x += font.h_advance_unscaled(id) * scale;
        }
    }
    out
}

/// Splits curves into closed, flattened contours.
fn contours(
    curves: &[OutlineCurve],
    at: impl Fn(ab_glyph::Point) -> (f32, f32),
) -> Vec<Vec<(f32, f32)>> {
    let mut contours: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut last: Option<ab_glyph::Point> = None;
    for curve in curves {
        let (start, end) = match curve {
            OutlineCurve::Line(a, b)
            | OutlineCurve::Quad(a, _, b)
            | OutlineCurve::Cubic(a, _, _, b) => (*a, *b),
        };
        if last.is_none_or(|p| (p.x - start.x).abs() > 1e-3 || (p.y - start.y).abs() > 1e-3) {
            contours.push(vec![at(start)]);
        }
        let contour = contours.last_mut().expect("a contour was started");
        #[allow(clippy::cast_precision_loss)]
        let steps = |f: &dyn Fn(f32) -> ab_glyph::Point, contour: &mut Vec<(f32, f32)>| {
            for i in 1..=CURVE_STEPS {
                contour.push(at(f(i as f32 / CURVE_STEPS as f32)));
            }
        };
        match curve {
            OutlineCurve::Line(_, b) => contour.push(at(*b)),
            OutlineCurve::Quad(a, c, b) => steps(
                &|t| {
                    let s = 1.0 - t;
                    ab_glyph::point(
                        s * s * a.x + 2.0 * s * t * c.x + t * t * b.x,
                        s * s * a.y + 2.0 * s * t * c.y + t * t * b.y,
                    )
                },
                contour,
            ),
            OutlineCurve::Cubic(a, c1, c2, b) => steps(
                &|t| {
                    let s = 1.0 - t;
                    let (k0, k1, k2, k3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
                    ab_glyph::point(
                        k0 * a.x + k1 * c1.x + k2 * c2.x + k3 * b.x,
                        k0 * a.y + k1 * c1.y + k2 * c2.y + k3 * b.y,
                    )
                },
                contour,
            ),
        }
        last = Some(end);
    }
    for contour in &mut contours {
        if contour.len() > 1 && contour.first() == contour.last() {
            contour.pop();
        }
    }
    contours.retain(|c| c.len() >= 3);
    contours
}

/// Groups contours into outlines with holes (nesting depth decides) and
/// triangulates them.
fn glyph_triangles(contours: &[Vec<(f32, f32)>], out: &mut Vec<[[f32; 2]; 3]>) {
    let ring = |c: &Vec<(f32, f32)>| {
        geo::LineString::from(
            c.iter()
                .map(|&(x, y)| (f64::from(x), f64::from(y)))
                .collect::<Vec<_>>(),
        )
    };
    let polygons: Vec<geo::Polygon<f64>> = contours
        .iter()
        .map(|c| geo::Polygon::new(ring(c), vec![]))
        .collect();
    let inside = |inner: usize, outer: usize| {
        inner != outer
            && contours[inner].iter().any(|&(x, y)| {
                polygons[outer].contains(&geo::Point::new(f64::from(x), f64::from(y)))
            })
    };
    let depth: Vec<usize> = (0..contours.len())
        .map(|i| (0..contours.len()).filter(|&j| inside(i, j)).count())
        .collect();
    for (i, _) in contours
        .iter()
        .enumerate()
        .filter(|(i, _)| depth[*i].is_multiple_of(2))
    {
        let holes = (0..contours.len())
            .filter(|&j| depth[j] == depth[i] + 1 && inside(j, i))
            .map(|j| ring(&contours[j]))
            .collect();
        let raw = geo::Polygon::new(ring(&contours[i]), holes).earcut_triangles_raw();
        #[allow(clippy::cast_possible_truncation)]
        let vertex = |k: usize| raw.vertices[k].map(|v| v as f32);
        out.extend(
            raw.triangle_indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| [vertex(t[0]), vertex(t[1]), vertex(t[2])]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(triangles: &[[[f32; 2]; 3]]) -> f32 {
        triangles
            .iter()
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn letters_have_holes_and_sit_on_the_baseline() {
        let o = text_triangles("O", 100.0, 0.0);
        let l = text_triangles("l", 100.0, 0.0);
        assert!(!o.is_empty() && !l.is_empty());
        // The counter of the O is left empty: its ink is a ring, much less
        // than its bounding box.
        let (min, max) =
            o.iter()
                .flatten()
                .fold(([f32::MAX; 2], [f32::MIN; 2]), |(min, max), p| {
                    (
                        [min[0].min(p[0]), min[1].min(p[1])],
                        [max[0].max(p[0]), max[1].max(p[1])],
                    )
                });
        let box_area = (max[0] - min[0]) * (max[1] - min[1]);
        assert!(area(&o) < box_area * 0.6, "{} vs {box_area}", area(&o));
        assert!(min[1] > -2.0 && max[1] > 60.0, "{min:?} {max:?}");
    }

    #[test]
    fn alignment_and_lines() {
        let centered = text_triangles("123", 20.0, 0.5);
        let xs: Vec<f32> = centered.iter().flatten().map(|p| p[0]).collect();
        let (lo, hi) = xs
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
        assert!((lo + hi).abs() < 3.0, "{lo} {hi}");
        let two = text_triangles("a\nb", 20.0, 0.0);
        let top = two.iter().flatten().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!(top > 25.0, "first line is above the last: {top}");
        assert_eq!(text_triangles("", 20.0, 0.0), [] as [[[f32; 2]; 3]; 0]);
        assert_eq!(text_triangles("   ", 20.0, 0.0), [] as [[[f32; 2]; 3]; 0]);
    }
}
