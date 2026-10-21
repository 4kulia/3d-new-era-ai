//! Quantities of what a project buys: the area of every surface a shop
//! product covers, and the pieces of furniture bought, per product id.
//!
//! Areas are what is there to cover, in m² (the model is in cm). A wall side
//! is measured on its own face: a straight wall's face runs between the
//! corners its joined outline makes with the walls of its storey (an inner
//! corner shortens it, an outer one lengthens it), less where another wall
//! meets it in a T, and a curved wall's face is its arc on radius r ± t/2,
//! longer on the convex side. Each stretch of a face between those
//! junctions is counted for the room it faces, so a side facing two rooms
//! is split between them. The face's height rises or falls from start to
//! end on a sloping wall, is taken within the band, and the union of the
//! doors and windows cut through it in that band and stretch comes off. A
//! floor or ceiling is the room's plan polygon less the stair voids through
//! it. Waste, coats and packs are the estimate's: each surface carries its
//! coats and waste for it, and nothing is rounded here.

use std::collections::BTreeMap;

use geo::{Area, BooleanOps};

use crate::elements::Wall;
use crate::finishes::{Band, FinishAssignment, Side, Surface};
use crate::furniture::Purchase;
use crate::geometry::{Point2, to_polygon};
use crate::home::Home;
use crate::ids::{FurnitureId, LevelId, RoomId};

/// cm² in a m².
const CM2_PER_M2: f64 = 10_000.0;

/// How far past a wall face a room is looked for, cm.
const PROBE: f64 = 1.0;

/// One covered surface.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceArea {
    pub surface: Surface,
    /// Its storey (`None` without storeys).
    pub level: Option<LevelId>,
    /// The room it is in, or faces; `None` for a wall side facing no room.
    pub room: Option<RoomId>,
    pub band: Option<Band>,
    pub coats: Option<u32>,
    /// Percent.
    pub waste: Option<f64>,
    /// m² before openings.
    pub gross: f64,
    /// m² of doors and windows taken off.
    pub openings: f64,
    /// m² to cover.
    pub net: f64,
    /// The doors and windows taken off.
    pub cut_by: Vec<FurnitureId>,
}

/// Everything one finish product covers.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductArea {
    pub product: String,
    pub gross: f64,
    pub openings: f64,
    pub net: f64,
    pub surfaces: Vec<SurfaceArea>,
}

/// The pieces of one furniture product that are bought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PieceCount {
    pub product: String,
    pub pieces: Vec<FurnitureId>,
}

/// What a project buys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Takeoff {
    /// By product id.
    pub finishes: Vec<ProductArea>,
    /// By product id.
    pub furniture: Vec<PieceCount>,
    /// Assignments whose surface is gone (a wall deleted, a room redrawn).
    pub missing: Vec<FinishAssignment>,
}

/// What a takeoff is limited to.
#[derive(Debug, Clone, Copy, Default)]
pub struct Only<'a> {
    pub product: Option<&'a str>,
    pub room: Option<RoomId>,
}

/// The quantities of `home`, limited to `only`.
#[must_use]
pub fn takeoff(home: &Home, only: Only<'_>) -> Takeoff {
    let cuts = home.wall_cuts();
    let outlines = home.storey_wall_outlines();
    let storey = |level: Option<LevelId>| {
        let level = home.resolve_level(level);
        home.sorted_levels()
            .iter()
            .position(|l| Some(l.id) == level)
            .unwrap_or(0)
    };
    let wanted = |product: &str| only.product.is_none_or(|p| p == product);
    let mut missing = Vec::new();
    let mut by_product: BTreeMap<String, Vec<SurfaceArea>> = BTreeMap::new();
    // A reference storey is a drawing to trace over, not something built.
    let traced = |surface: Surface| {
        home.element(surface.element())
            .is_some_and(|e| home.is_reference_level(e.level()))
    };
    for assignment in home
        .finish_assignments
        .iter()
        .filter(|a| wanted(&a.product) && !traced(a.surface))
    {
        let Some(areas) = surface_area(home, &cuts, &outlines, assignment) else {
            missing.push(assignment.clone());
            continue;
        };
        for area in areas {
            if only.room.is_none_or(|room| area.room == Some(room)) {
                by_product
                    .entry(assignment.product.clone())
                    .or_default()
                    .push(area);
            }
        }
    }
    let finishes = by_product
        .into_iter()
        .map(|(product, mut surfaces)| {
            surfaces.sort_by_key(|s| (storey(s.level), s.room, s.surface));
            ProductArea {
                product,
                gross: surfaces.iter().map(|s| s.gross).sum(),
                openings: surfaces.iter().map(|s| s.openings).sum(),
                net: surfaces.iter().map(|s| s.net).sum(),
                surfaces,
            }
        })
        .collect();
    let mut pieces: BTreeMap<String, Vec<FurnitureId>> = BTreeMap::new();
    for top in home
        .furniture
        .iter()
        .filter(|top| !home.is_reference_level(top.level))
    {
        for piece in top.flatten() {
            let Some(product) = piece.info.product.as_deref() else {
                continue;
            };
            if piece.info.purchase() != Purchase::Buy || !wanted(product) {
                continue;
            }
            if let Some(room) = only.room
                && home.room_at(piece.position, top.level).map(|r| r.id) != Some(room)
            {
                continue;
            }
            pieces.entry(product.to_owned()).or_default().push(piece.id);
        }
    }
    Takeoff {
        finishes,
        furniture: pieces
            .into_iter()
            .map(|(product, pieces)| PieceCount { product, pieces })
            .collect(),
        missing,
    }
}

/// The area of the surface `assignment` covers, one row for each room it
/// faces; `None` when the surface is gone.
fn surface_area(
    home: &Home,
    cuts: &[Vec<crate::WallCut>],
    outlines: &[Vec<Point2>],
    assignment: &FinishAssignment,
) -> Option<Vec<SurfaceArea>> {
    let (level, stretches) = match assignment.surface {
        Surface::Floor(id) | Surface::Ceiling(id) => {
            let room = home.room(id)?;
            let voids = match assignment.surface {
                Surface::Floor(_) => crate::stair_holes(home, room.level),
                _ => crate::ceiling::storey_above(home, room.level)
                    .map(|above| crate::stair_holes(home, Some(above)))
                    .unwrap_or_default(),
            };
            (
                room.level,
                vec![Stretch {
                    room: Some(id),
                    gross: plan_area(&room.points, &voids),
                    openings: 0.0,
                    cut_by: Vec::new(),
                }],
            )
        }
        Surface::Wall(id, side) => {
            let at = home.walls.iter().position(|w| w.id == id)?;
            let band = assignment
                .band
                .map_or((0.0, f64::INFINITY), |b| (b.from, b.to));
            (
                home.walls[at].level,
                wall_area(home, at, side, outlines, &cuts[at], band),
            )
        }
    };
    Some(
        stretches
            .into_iter()
            .map(|stretch| {
                let (gross, openings) = (stretch.gross / CM2_PER_M2, stretch.openings / CM2_PER_M2);
                SurfaceArea {
                    surface: assignment.surface,
                    level: home.resolve_level(level),
                    room: stretch.room,
                    band: assignment.band,
                    coats: assignment.coats,
                    waste: assignment.waste,
                    gross,
                    openings,
                    net: gross - openings,
                    cut_by: stretch.cut_by,
                }
            })
            .collect(),
    )
}

/// What one room sees of a surface, cm².
struct Stretch {
    room: Option<RoomId>,
    gross: f64,
    openings: f64,
    cut_by: Vec<FurnitureId>,
}

/// The plan area of `points` less what `voids` take out of it, cm².
fn plan_area(points: &[Point2], voids: &[Vec<Point2>]) -> f64 {
    let mut shape = geo::MultiPolygon::new(vec![to_polygon(points)]);
    for void in voids.iter().filter(|v| v.len() >= 3) {
        shape = shape.difference(&to_polygon(void));
    }
    shape.unsigned_area()
}

/// Wall `at`'s side within the band `lo..hi` cm of height, one stretch for
/// each room it faces, in the order they come along the wall. The face is
/// the one its joined outline (`outlines`, in `walls` order) has, less
/// where the outline of another wall of its storey meets it; each piece
/// left is counted for the room just past its middle. Its height runs
/// straight from `height` at the start to `height_at_end` at the end and
/// holds past the ends. The openings are the union of the holes `cuts`
/// makes within each piece, so two that overlap are taken off once. A
/// curved wall is one stretch on its own radius.
fn wall_area(
    home: &Home,
    at: usize,
    side: Side,
    outlines: &[Vec<Point2>],
    cuts: &[crate::WallCut],
    (lo, hi): (f64, f64),
) -> Vec<Stretch> {
    let wall = &home.walls[at];
    let (start, end) = (wall.height, wall.height_at_end.unwrap_or(wall.height));
    if wall.is_arc() {
        // Openings never cut a curved wall (`wall_cuts`).
        return vec![Stretch {
            room: facing_room(home, wall, side),
            gross: band_area(arc_face(wall, side), start, end, lo, hi),
            openings: 0.0,
            cut_by: Vec::new(),
        }];
    }
    let length = wall.start.distance(wall.end);
    let sign = match side {
        Side::Left => 1.0,
        Side::Right => -1.0,
    };
    let span = crate::ops::face_span(wall, &outlines[at], sign).unwrap_or((0.0, length));
    let dir = (
        (wall.end.x - wall.start.x) / length,
        (wall.end.y - wall.start.y) / length,
    );
    // Left of start → end on the plan (y down) is (dy, -dx).
    let normal = (dir.1 * sign, -dir.0 * sign);
    let point = |along: f64, out: f64| {
        Point2::new(
            wall.start.x + dir.0 * along + normal.0 * out,
            wall.start.y + dir.1 * along + normal.1 * out,
        )
    };
    let face = wall.thickness / 2.0;
    let mut stretches: Vec<Stretch> = Vec::new();
    for (from, to) in free_face(home, at, outlines, span, |along| point(along, face + 0.5)) {
        let room = home
            .room_at(point(f64::midpoint(from, to), face + PROBE), wall.level)
            .map(|r| r.id);
        let clipped: Vec<(FurnitureId, [f64; 4])> = cuts
            .iter()
            .map(|c| {
                (
                    c.furniture,
                    [
                        c.from.max(from),
                        c.to.min(to),
                        c.bottom.max(lo),
                        c.top.min(hi),
                    ],
                )
            })
            .filter(|(_, [from, to, bottom, top])| to > from && top > bottom)
            .collect();
        let holes: Vec<[f64; 4]> = clipped.iter().map(|(_, hole)| *hole).collect();
        let gross = span_area(from, to, length, start, end, lo, hi);
        let openings = union_area(&holes);
        let cut = clipped.into_iter().map(|(id, _)| id);
        if !stretches.iter().any(|s| s.room == room) {
            stretches.push(Stretch {
                room,
                gross: 0.0,
                openings: 0.0,
                cut_by: Vec::new(),
            });
        }
        let at = stretches
            .iter()
            .position(|s| s.room == room)
            .expect("pushed above");
        let stretch = &mut stretches[at];
        stretch.gross += gross;
        stretch.openings += openings;
        for id in cut {
            if !stretch.cut_by.contains(&id) {
                stretch.cut_by.push(id);
            }
        }
    }
    stretches
}

/// The pieces of the face `span` (cm along wall `at`) that no other wall of
/// its storey covers: `probe` is the point just past the face at a distance
/// along the wall, and where it falls inside another wall's outline, that
/// wall meets the face there.
fn free_face(
    home: &Home,
    at: usize,
    outlines: &[Vec<Point2>],
    (from, to): (f64, f64),
    probe: impl Fn(f64) -> Point2,
) -> Vec<(f64, f64)> {
    let wall = &home.walls[at];
    let storey = home.resolve_level(wall.level);
    let (a, b) = (probe(from), probe(to));
    let line = geo::MultiLineString::new(vec![geo::LineString::from(vec![(a.x, a.y), (b.x, b.y)])]);
    let along = |x: f64, y: f64| {
        from + (to - from) * ((x - a.x) * (b.x - a.x) + (y - a.y) * (b.y - a.y))
            / a.distance(b).powi(2).max(1e-12)
    };
    let mut covered: Vec<(f64, f64)> = Vec::new();
    for (j, other) in home.walls.iter().enumerate() {
        if j == at || outlines[j].len() < 3 || home.resolve_level(other.level) != storey {
            continue;
        }
        for piece in to_polygon(&outlines[j]).clip(&line, false) {
            let ends: Vec<f64> = piece.coords().map(|c| along(c.x, c.y)).collect();
            let (lo, hi) = ends
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            if hi > lo {
                covered.push((lo, hi));
            }
        }
    }
    covered.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut free = Vec::new();
    let mut reach = from;
    for (lo, hi) in covered {
        if lo > reach + 0.1 {
            free.push((reach, lo.min(to)));
        }
        reach = reach.max(hi);
    }
    if to > reach + 0.1 {
        free.push((reach, to));
    }
    free
}

/// The length of a curved wall's face on `side`: its arc on the radius of
/// that face, the centreline's less half the thickness on the concave side
/// and plus it on the convex one.
fn arc_face(wall: &Wall, side: Side) -> f64 {
    let sweep = wall.arc_extent.unwrap_or(0.0).to_radians();
    let radius = wall.start.distance(wall.end) / (2.0 * (sweep.abs() / 2.0).sin());
    // A positive extent bulges to the left: the centre is on the right.
    let concave = if sweep > 0.0 { Side::Right } else { Side::Left };
    let offset = if side == concave {
        -wall.thickness / 2.0
    } else {
        wall.thickness / 2.0
    };
    (radius + offset) * sweep.abs()
}

/// The area between the heights `lo` and `hi` under a wall's top along
/// `from..to` cm of a wall `length` long: the top runs straight from
/// `start` to `end` along the wall and holds level past its ends, where an
/// outer corner carries the face on. cm².
fn span_area(from: f64, to: f64, length: f64, start: f64, end: f64, lo: f64, hi: f64) -> f64 {
    let covered = |h: f64| h.clamp(lo, hi) - lo;
    let height = |s: f64| start + (end - start) * (s / length).clamp(0.0, 1.0);
    let (a, b) = (from.max(0.0), to.min(length));
    let middle = if b > a {
        band_area(b - a, height(a), height(b), lo, hi)
    } else {
        0.0
    };
    covered(start) * (to.min(0.0) - from).max(0.0)
        + middle
        + covered(end) * (to - from.max(length)).max(0.0)
}

/// The area under a height that runs straight from `start` to `end` over
/// `length`, between the heights `lo` and `hi`, cm².
fn band_area(length: f64, start: f64, end: f64, lo: f64, hi: f64) -> f64 {
    // Covered height at a fraction `t` along the wall.
    let covered = |t: f64| (start + (end - start) * t).clamp(lo, hi) - lo;
    // Straight between the points where the top crosses `lo` or `hi`.
    let mut breaks = vec![0.0, 1.0];
    for edge in [lo, hi] {
        if (start - edge) * (end - edge) < 0.0 {
            breaks.push((edge - start) / (end - start));
        }
    }
    breaks.sort_by(f64::total_cmp);
    breaks
        .windows(2)
        .map(|w| covered(w[0]).midpoint(covered(w[1])) * (w[1] - w[0]) * length)
        .sum()
}

/// The area of the union of rectangles `[from, to, bottom, top]`, cm².
fn union_area(rects: &[[f64; 4]]) -> f64 {
    let mut xs: Vec<f64> = rects.iter().flat_map(|r| [r[0], r[1]]).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let mut area = 0.0;
    for w in xs.windows(2) {
        let (x0, x1) = (w[0], w[1]);
        let mut spans: Vec<(f64, f64)> = rects
            .iter()
            .filter(|r| r[0] <= x0 && r[1] >= x1)
            .map(|r| (r[2], r[3]))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut covered = 0.0;
        let mut reach = f64::NEG_INFINITY;
        for (bottom, top) in spans {
            let bottom = bottom.max(reach);
            if top > bottom {
                covered += top - bottom;
                reach = top;
            }
        }
        area += covered * (x1 - x0);
    }
    area
}

/// The room a surface is in: a floor's or ceiling's own; for a wall side,
/// the room facing its longest stretch (the takeoff splits it between
/// them), a room before none, and on a tie the one met first walking from
/// the wall's start. `None` for a wall
/// side facing no room, and a surface gone.
#[must_use]
pub fn room_of(home: &Home, surface: Surface) -> Option<RoomId> {
    match surface {
        Surface::Floor(room) | Surface::Ceiling(room) => home.room(room).map(|r| r.id),
        Surface::Wall(wall, side) => {
            let at = home.walls.iter().position(|w| w.id == wall)?;
            let cuts = home.wall_cuts();
            let stretches = wall_area(
                home,
                at,
                side,
                &home.storey_wall_outlines(),
                &cuts[at],
                (0.0, f64::INFINITY),
            );
            // A room before no room; the longest stretch; on a tie, the
            // first along the wall from its start.
            let mut best: Option<Stretch> = None;
            for stretch in stretches {
                let better = best.as_ref().is_none_or(|b| {
                    (stretch.room.is_some(), stretch.gross) > (b.room.is_some(), b.gross)
                });
                if better {
                    best = Some(stretch);
                }
            }
            best?.room
        }
    }
}

/// The room a curved wall's side faces: the one just past that face, at
/// the middle of the wall, on the wall's storey. The smallest wins where rooms nest.
fn facing_room(home: &Home, wall: &Wall, side: Side) -> Option<RoomId> {
    let (before, after) = (wall.point_at(0.49), wall.point_at(0.51));
    let middle = wall.point_at(0.5);
    let (dx, dy) = (after.x - before.x, after.y - before.y);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return None;
    }
    // Left of start → end on the plan (y down) is (dy, -dx).
    let (nx, ny) = match side {
        Side::Left => (dy / len, -dx / len),
        Side::Right => (-dy / len, dx / len),
    };
    let reach = wall.thickness / 2.0 + PROBE;
    let probe = Point2::new(middle.x + nx * reach, middle.y + ny * reach);
    home.room_at(probe, wall.level).map(|r| r.id)
}

#[cfg(test)]
mod tests;
