//! Measuring a plan the way someone with a tape measure would.
//!
//! Three questions come up over and over while reviewing a layout, and all
//! three used to force whoever asked them to rebuild the geometry by hand:
//!
//! * *Where exactly is this piece?* — [`plan_bounds`] resolves `angle`,
//!   mirroring and tilt into an axis-aligned box, and [`facing`] says which
//!   way its front looks.
//! * *How much room is there in front of it?* — [`clearance`].
//! * *What do I run into crossing the room here?* — [`free_span`].
//!
//! Everything works on one storey: pass a [`Home::level_view`] (or a home
//! without levels) so a piece upstairs never blocks a corridor downstairs.

use geo::{Area, BooleanOps, Polygon};

use crate::elements::{Dimension, Hold, Label, Wall};
use crate::furniture::Furniture;
use crate::geometry::Point2;
use crate::home::Home;
use crate::ids::{ElementId, FurnitureId, WallId};

/// Pieces this thin (rugs, mats, glass panes seen edge on) never block.
const FLAT: f64 = 2.0;
/// How far a measurement looks before giving up, cm.
pub const MAX_REACH: f64 = 2000.0;

/// A plan axis. `X` grows right, `Y` grows down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

impl Axis {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }

    fn of(self, p: Point2) -> f64 {
        match self {
            Self::X => p.x,
            Self::Y => p.y,
        }
    }

    fn across(self) -> Self {
        match self {
            Self::X => Self::Y,
            Self::Y => Self::X,
        }
    }
}

/// One of the four plan directions, as an agent writes it: `+x`, `-y`…
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dir {
    pub axis: Axis,
    /// `1.0` toward growing coordinates, `-1.0` the other way.
    pub sign: f64,
}

impl Dir {
    pub const PLAN: [Self; 4] = [
        Self {
            axis: Axis::X,
            sign: 1.0,
        },
        Self {
            axis: Axis::X,
            sign: -1.0,
        },
        Self {
            axis: Axis::Y,
            sign: 1.0,
        },
        Self {
            axis: Axis::Y,
            sign: -1.0,
        },
    ];

    /// `+x`, `-x`, `+y`, `-y`, or a piece-relative `front`, `back`, `left`,
    /// `right` resolved against `piece`'s angle.
    pub fn parse(raw: &str, piece: Option<&Furniture>) -> Option<Self> {
        let raw = raw.trim().to_ascii_lowercase();
        let plan = |axis: Axis, sign: f64| Some(Self { axis, sign });
        match raw.as_str() {
            "+x" | "x" | "right_plan" | "east" => return plan(Axis::X, 1.0),
            "-x" | "left_plan" | "west" => return plan(Axis::X, -1.0),
            "+y" | "y" | "down" | "south" => return plan(Axis::Y, 1.0),
            "-y" | "up" | "north" => return plan(Axis::Y, -1.0),
            _ => {}
        }
        // Piece-relative: measured from the side it opens to, which is what
        // was built into it — see [`facing`] — and not from `angle` alone.
        let piece = piece?;
        let quarters = match raw.as_str() {
            "front" => 0,
            "left" => 1,
            "back" => 2,
            "right" => 3,
            _ => return None,
        };
        let front = Self::parse(facing(piece), None)?;
        Some(front.turn(quarters))
    }

    /// This direction turned `quarters` times to the left of the plan, the
    /// way `angle` turns a piece: `+y` → `-x` → `-y` → `+x`.
    #[must_use]
    pub fn turn(self, quarters: u8) -> Self {
        let order = [
            Self {
                axis: Axis::Y,
                sign: 1.0,
            },
            Self {
                axis: Axis::X,
                sign: -1.0,
            },
            Self {
                axis: Axis::Y,
                sign: -1.0,
            },
            Self {
                axis: Axis::X,
                sign: 1.0,
            },
        ];
        let at = order.iter().position(|d| *d == self).unwrap_or_default();
        order[(at + quarters as usize) % 4]
    }

    pub fn name(self) -> &'static str {
        match (self.axis, self.sign >= 0.0) {
            (Axis::X, true) => "+x",
            (Axis::X, false) => "-x",
            (Axis::Y, true) => "+y",
            (Axis::Y, false) => "-y",
        }
    }
}

/// The plan direction a piece's front looks at, snapped to the nearest
/// quarter turn. A piece's front is its local `+y` before rotation.
fn snap(angle_degrees: f64) -> Dir {
    let a = angle_degrees.rem_euclid(360.0);
    // angle 0 → front at +y; 90 → front at -x (plan y grows down, x right,
    // and `to_plan` turns local +y toward -x for a clockwise angle).
    #[allow(clippy::cast_possible_truncation)]
    let quarter = ((a / 90.0).round() as i64).rem_euclid(4);
    match quarter {
        0 => Dir {
            axis: Axis::Y,
            sign: 1.0,
        },
        1 => Dir {
            axis: Axis::X,
            sign: -1.0,
        },
        2 => Dir {
            axis: Axis::Y,
            sign: -1.0,
        },
        _ => Dir {
            axis: Axis::X,
            sign: 1.0,
        },
    }
}

/// How thin a part has to be, in cm, to be a panel and not a body.
const PANEL: f64 = 6.0;

/// How wide a panel has to be, at least, to be a door, a front or a back and
/// not a post, in cm.
const PANEL_WIDE: f64 = 10.0;

/// How close to a face of the group a panel has to sit to be on it, in cm.
const AT_FACE: f64 = 4.0;

/// What a part's name says about which side it is: a door and a drawer front
/// are the opening side, a back panel and a blind panel are the other one.
/// Names come from the joinery builder and from whoever drew the group, in
/// the two languages this app speaks.
fn front_word(name: &str) -> f64 {
    let name = crate::annotations::fold(name);
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&["cego", "fundo", "costa", "back panel", "blind"]) {
        return -4.0;
    }
    if has(&[
        "frente", "porta", "gavet", "front", "door", "drawer", "rodape", "kick",
    ]) {
        return 3.0;
    }
    0.0
}

/// The face a group's fronts are on, when the parts say it clearly.
///
/// A panel across a face — a door, a drawer front, the fixed front under a
/// sink — is built on the side the piece opens to. Several of them on one
/// face and a single blind panel on the other is not ambiguous, and it is
/// exactly what `angle` cannot tell: a group brought in from another program
/// carries whatever angle it was placed with.
fn built_front(group: &Furniture) -> Option<&'static str> {
    let score = front_scores(group)?;
    let (best, runner_up) = best_face(&score);
    // Only a clear majority overrules `angle`: one door against one back panel
    // says nothing, five fronts against a blind panel says everything.
    (score[best] >= 2.0 && score[best] >= 2.0 * runner_up).then(|| FACES[best])
}

/// Faces in the order scores are kept.
const FACES: [&str; 4] = ["-x", "+x", "-y", "+y"];

/// How much a handle says about the face it is on: more than any panel, since
/// nobody puts a pull on the back of a cabinet.
const HANDLE: f64 = 6.0;

/// The best face and the score of the runner-up.
fn best_face(score: &[f64; 4]) -> (usize, f64) {
    let best = (0..4)
        .max_by(|a, b| score[*a].total_cmp(&score[*b]))
        .unwrap_or_default();
    let runner_up = (0..4)
        .filter(|f| *f != best)
        .map(|f| score[f])
        .fold(0.0_f64, f64::max);
    (best, runner_up)
}

/// What the parts of a group say about each face, `-x +x -y +y`.
fn front_scores(group: &Furniture) -> Option<[f64; 4]> {
    let parts: Vec<&Furniture> = group.flatten().into_iter().skip(1).collect();
    if parts.len() < 2 {
        return None;
    }
    let (min, max) = plan_bounds(group);
    let mut score = [0.0_f64; 4];
    for part in parts {
        let name = part
            .properties
            .get("joinery:part")
            .map_or(part.name.as_str(), String::as_str);
        let (lo, hi) = plan_bounds(part);
        // A handle sits on the face that opens: the nearest one says it.
        if is_handle(name) {
            let away = [lo.x - min.x, max.x - hi.x, lo.y - min.y, max.y - hi.y];
            let (nearest, _) = best_face(&away.map(|d| -d));
            if away[nearest] <= AT_FACE {
                score[nearest] += HANDLE;
            }
            continue;
        }
        let weight = 1.0 + front_word(name);
        if weight <= 0.0 {
            continue;
        }
        let (across_x, across_y) = (hi.x - lo.x, hi.y - lo.y);
        // A panel lies flat against one face: thin one way and clearly wide
        // the other. A 3 × 3 cm post is neither, and counting it as soon as a
        // resize made it 3.25 × 3 turned a whole table around.
        let flat = |thin: f64, wide: f64| thin <= PANEL && wide >= (3.0 * thin).max(PANEL_WIDE);
        if flat(across_y, across_x) {
            if (lo.y - min.y).abs() <= AT_FACE {
                score[2] += weight;
            }
            if (hi.y - max.y).abs() <= AT_FACE {
                score[3] += weight;
            }
        } else if flat(across_x, across_y) {
            if (lo.x - min.x).abs() <= AT_FACE {
                score[0] += weight;
            }
            if (hi.x - max.x).abs() <= AT_FACE {
                score[1] += weight;
            }
        }
    }
    Some(score)
}

/// Whether a part's name says it is a handle.
fn is_handle(name: &str) -> bool {
    let name = crate::annotations::fold(name);
    ["puxador", "handle", "knob", "pull ", "pegador"]
        .iter()
        .any(|w| name.contains(w))
}

/// A group whose parts point at a front without settling on one — fronts on
/// one face and a door-named side on another — so its `angle` is what every
/// "in front of" falls back to, unconfirmed. The candidate faces, strongest
/// first; `None` when the parts agree or say nothing.
#[must_use]
pub fn front_unclear(group: &Furniture) -> Option<Vec<&'static str>> {
    let score = front_scores(group)?;
    if built_front(group).is_some() {
        return None;
    }
    let (best, _) = best_face(&score);
    if score[best] < 2.0 + 3.0 {
        // Nothing named as a front: plain boxes say nothing, and that is fine.
        return None;
    }
    // The faces that come close to the strongest: the doubt, not the noise.
    let mut faces: Vec<usize> = (0..4)
        .filter(|f| score[*f] >= 2.0 && score[*f] * 2.0 > score[best])
        .collect();
    faces.sort_by(|a, b| score[*b].total_cmp(&score[*a]));
    (faces.len() >= 2).then(|| faces.into_iter().map(|f| FACES[f]).collect())
}

/// Which way a piece's front looks, as `+x`, `-x`, `+y` or `-y`.
///
/// For a cabinet this is the side its doors open toward, which decides
/// whether it can be used at all — and is invisible in `angle` alone.
///
/// A piece on its own is read from `angle`: a catalog model is built facing
/// its own `+y`. A group is read from what was built into it — doors, drawer
/// fronts, the blind panel on the back — because that is the thing that
/// exists; `angle` only says how the group was placed, and for one imported
/// from another program it may say nothing at all. Where the two disagree,
/// [`crate::check_layout`] reports it rather than picking a side in silence.
pub fn facing(piece: &Furniture) -> &'static str {
    // Mirroring flips the piece's local x (a left-hinged door becomes
    // right-hinged); the front stays local +y, so it does not move.
    built_front(piece).unwrap_or_else(|| snap(piece.angle).name())
}

/// Whether the parts built into a group say it opens somewhere other than
/// `angle` does: a modeling mistake, and one that quietly turns a cabinet
/// against a wall.
#[must_use]
pub fn facing_disagrees(piece: &Furniture) -> Option<(&'static str, &'static str)> {
    let built = built_front(piece)?;
    let placed = snap(piece.angle).name();
    (built != placed).then_some((built, placed))
}

/// The piece as it is built, when its `angle` says otherwise: a childless
/// copy turned so its local `+y` is the face its doors are on, width and
/// depth swapped for a quarter turn so the box stays where it is.
///
/// Anything that measures "in front of", "beside" or "behind" in a piece's
/// own frame reads that frame from this, so it agrees with [`facing`] —
/// otherwise a sink cabinet whose doors face the corridor is measured
/// against its own countertop, at its back.
#[must_use]
pub fn built_frame(piece: &Furniture) -> Option<Furniture> {
    let (built, _) = facing_disagrees(piece)?;
    let turns = (1..4).find(|k| snap(piece.angle + 90.0 * f64::from(*k)).name() == built)?;
    let mut frame = Furniture {
        children: Vec::new(),
        ..piece.clone()
    };
    frame.angle = piece.angle + 90.0 * f64::from(turns);
    if turns % 2 == 1 {
        std::mem::swap(&mut frame.width, &mut frame.depth);
    }
    Some(frame)
}

/// Axis-aligned plan box of a piece, `angle`, mirroring and tilt applied.
///
/// This is the `bounds` every read returns, so nobody has to swap width and
/// depth by hand when a piece is turned a quarter turn.
pub fn plan_bounds(piece: &Furniture) -> (Point2, Point2) {
    let corners = piece.projected_footprint();
    let mut min = Point2::new(f64::MAX, f64::MAX);
    let mut max = Point2::new(f64::MIN, f64::MIN);
    for c in corners {
        min = Point2::new(min.x.min(c.x), min.y.min(c.y));
        max = Point2::new(max.x.max(c.x), max.y.max(c.y));
    }
    (min, max)
}

/// Axis-aligned plan box of a wall, its thickness and corner joins applied.
pub fn wall_bounds(home: &Home, wall: &Wall) -> (Point2, Point2) {
    let outlines = home.wall_outlines();
    let outline = home
        .walls
        .iter()
        .position(|w| w.id == wall.id)
        .and_then(|i| outlines.get(i))
        .filter(|o| o.len() >= 3);
    // A wall with no outline (zero length, or not joined yet) still has a
    // footprint: its two ends grown by half its thickness.
    let half = wall.thickness / 2.0;
    let ends = vec![
        Point2::new(wall.start.x - half, wall.start.y - half),
        Point2::new(wall.start.x + half, wall.start.y + half),
        Point2::new(wall.end.x - half, wall.end.y - half),
        Point2::new(wall.end.x + half, wall.end.y + half),
    ];
    let mut min = Point2::new(f64::MAX, f64::MAX);
    let mut max = Point2::new(f64::MIN, f64::MIN);
    for p in outline.unwrap_or(&ends) {
        min = Point2::new(min.x.min(p.x), min.y.min(p.y));
        max = Point2::new(max.x.max(p.x), max.y.max(p.y));
    }
    (min, max)
}

/// Plan box of any element, for `measure` and for enriched reports.
pub fn element_bounds(home: &Home, id: ElementId) -> Option<(Point2, Point2)> {
    let hull = |points: &[Point2]| {
        let mut min = Point2::new(f64::MAX, f64::MAX);
        let mut max = Point2::new(f64::MIN, f64::MIN);
        for p in points {
            min = Point2::new(min.x.min(p.x), min.y.min(p.y));
            max = Point2::new(max.x.max(p.x), max.y.max(p.y));
        }
        (min, max)
    };
    match id {
        // A part of a group is a piece too: dimensions hold onto a cabinet front.
        ElementId::Furniture(f) => home.find_piece(f).map(plan_bounds),
        ElementId::Wall(w) => home.wall(w).map(|w| wall_bounds(home, w)),
        ElementId::Room(r) => home.room(r).map(|r| hull(&r.points)),
        ElementId::Dimension(d) => home.dimension(d).map(|d| hull(&[d.start, d.end])),
        ElementId::Label(l) => home.label(l).map(|l| hull(&[l.position])),
        ElementId::Polyline(p) => home.polyline(p).map(|p| hull(&p.points)),
        ElementId::Level(_) => None,
    }
}

/// What a probe ran into.
#[derive(Debug, Clone, PartialEq)]
pub enum Solid {
    Wall(WallId),
    Piece(FurnitureId),
}

impl Solid {
    pub fn id(&self) -> ElementId {
        match *self {
            Self::Wall(w) => ElementId::Wall(w),
            Self::Piece(f) => ElementId::Furniture(f),
        }
    }
}

/// Something that stops a tape measure: its outline and height range.
#[derive(Debug, Clone)]
pub struct Obstacle {
    pub what: Solid,
    pub name: String,
    pub outline: Vec<Point2>,
    /// Bottom and top above this storey's floor, cm.
    pub z: (f64, f64),
}

impl Obstacle {
    fn polygon(&self) -> Polygon<f64> {
        crate::geometry::to_polygon(&self.outline)
    }

    fn spans(&self, axis: Axis) -> (f64, f64) {
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for p in &self.outline {
            lo = lo.min(axis.of(*p));
            hi = hi.max(axis.of(*p));
        }
        (lo, hi)
    }
}

/// Whether a piece is worth measuring against: not an opening, not a rug,
/// not a part built into its host.
fn blocks(piece: &Furniture) -> bool {
    piece.visible
        && !piece.is_opening()
        && piece.height > FLAT
        && piece.width.min(piece.depth) > FLAT
        && piece.discipline.is_none()
        && !piece.properties.contains_key("joinery:embedded")
}

/// [`blocks`], leaving thinness out of it.
fn blocks_but_thin(piece: &Furniture) -> bool {
    piece.visible
        && !piece.is_opening()
        && piece.discipline.is_none()
        && !piece.properties.contains_key("joinery:embedded")
}

/// Every solid of one storey, walls first. `skip` leaves pieces out — the one
/// being measured from, typically.
pub fn obstacles(home: &Home, skip: &dyn Fn(&Furniture) -> bool) -> Vec<Obstacle> {
    let mut out = Vec::new();
    for (wall, outline) in home.walls.iter().zip(home.wall_outlines()) {
        if outline.len() < 3 {
            continue;
        }
        out.push(Obstacle {
            what: Solid::Wall(wall.id),
            name: wall
                .wall_type
                .clone()
                .unwrap_or_else(|| "parede".to_owned()),
            outline,
            z: (0.0, wall.height.max(wall.height_at_end.unwrap_or(0.0))),
        });
    }
    for top in &home.furniture {
        let grouped = !top.children.is_empty();
        for leaf in top.visible_leaves() {
            // A cabinet is drawn as its boards: a 1.8 cm side standing 280 cm
            // tall is a wall of it, not a rug, and without it a probe walks
            // through the whole cabinet as if it were air.
            let board = grouped && leaf.height > FLAT && leaf.width.max(leaf.depth) > FLAT;
            if !(blocks(leaf) || (board && blocks_but_thin(leaf))) || skip(leaf) {
                continue;
            }
            out.push(Obstacle {
                what: Solid::Piece(leaf.id),
                name: leaf.name.clone(),
                outline: leaf.projected_footprint().to_vec(),
                z: leaf.height_range(),
            });
        }
    }
    out
}

/// A stretch of a straight probe: either free floor or one solid.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Start along the axis, cm.
    pub from: f64,
    /// End along the axis, cm.
    pub to: f64,
    /// `None` for free floor.
    pub what: Option<Solid>,
    /// Name of the solid, empty for free floor.
    pub name: String,
}

impl Span {
    pub fn cm(&self) -> f64 {
        self.to - self.from
    }

    pub fn is_free(&self) -> bool {
        self.what.is_none()
    }
}

/// Where a line at `at` (the other axis' coordinate) enters and leaves a
/// polygon, as intervals along `axis`. Even-odd crossings of the edges.
fn crossings(outline: &[Point2], axis: Axis, at: f64) -> Vec<(f64, f64)> {
    let across = axis.across();
    let mut hits: Vec<f64> = Vec::new();
    for k in 0..outline.len() {
        let (a, b) = (outline[k], outline[(k + 1) % outline.len()]);
        let (ca, cb) = (across.of(a), across.of(b));
        // Half-open rule: a vertex counts for the edge below it only, so a
        // line grazing a corner crosses once, not twice.
        if (ca <= at && cb > at) || (cb <= at && ca > at) {
            let t = (at - ca) / (cb - ca);
            hits.push(axis.of(a) + t * (axis.of(b) - axis.of(a)));
        }
    }
    hits.sort_by(f64::total_cmp);
    hits.as_chunks::<2>()
        .0
        .iter()
        .map(|c| (c[0], c[1]))
        .collect()
}

/// What a straight probe runs into, in order.
///
/// The probe runs along `axis` at `at` (the other axis' coordinate), between
/// `range` if given (otherwise the whole drawing), counting only solids whose
/// height range meets `z` — so `z = (0, 200)` ignores a wall cabinet at 150
/// only if it starts above 200, and `z = (90, 91)` follows a countertop line.
///
/// This is the answer to "how wide is the corridor here, and between what?"
/// in a single call.
pub fn free_span(
    home: &Home,
    axis: Axis,
    at: f64,
    range: Option<(f64, f64)>,
    z: (f64, f64),
) -> Vec<Span> {
    let (lo, hi) = range
        .or_else(|| home.bounds().map(|(a, b)| (axis.of(a), axis.of(b))))
        .unwrap_or((0.0, 0.0));
    if hi <= lo {
        return Vec::new();
    }
    let mut hits: Vec<Span> = Vec::new();
    for o in obstacles(home, &|_| false) {
        if o.z.1 <= z.0 || o.z.0 >= z.1 {
            continue;
        }
        for (a, b) in crossings(&o.outline, axis, at) {
            let (a, b) = (a.max(lo), b.min(hi));
            if b - a > 0.05 {
                hits.push(Span {
                    from: a,
                    to: b,
                    what: Some(o.what.clone()),
                    name: o.name.clone(),
                });
            }
        }
    }
    hits.sort_by(|a, b| a.from.total_cmp(&b.from));
    let mut out: Vec<Span> = Vec::new();
    let mut cursor = lo;
    for hit in hits {
        if hit.from > cursor + 0.05 {
            out.push(Span {
                from: cursor,
                to: hit.from,
                what: None,
                name: String::new(),
            });
        }
        // Solids that overlap along the probe are reported as they come; the
        // cursor only ever moves forward so the spans stay in order.
        if hit.to > cursor {
            let from = hit.from.max(cursor);
            cursor = hit.to;
            out.push(Span {
                from,
                to: hit.to,
                ..hit
            });
        }
    }
    if hi > cursor + 0.05 {
        out.push(Span {
            from: cursor,
            to: hi,
            what: None,
            name: String::new(),
        });
    }
    out
}

/// Free floor on one side of a piece, and what stops it.
#[derive(Debug, Clone, PartialEq)]
pub struct Clearance {
    pub dir: Dir,
    /// Free centimeters from that face, capped at [`MAX_REACH`]; negative
    /// when what it faces reaches that far into the piece.
    pub cm: f64,
    /// `None` when nothing was found within reach.
    pub against: Option<Solid>,
    pub name: String,
}

/// Free floor in front of (or beside, or behind) a piece.
///
/// Measured over the whole face, from the piece's axis-aligned box, ignoring
/// whatever is built into it (a cooktop in its countertop) and whatever
/// passes over a person's head.
pub fn clearance(home: &Home, piece: &Furniture, dir: Dir, max: f64) -> Clearance {
    let own: Vec<crate::ids::FurnitureId> = piece.flatten().iter().map(|f| f.id).collect();
    let solids = obstacles(home, &|f| own.contains(&f.id));
    clearance_against(&solids, piece, dir, max)
}

/// [`clearance`] against solids already gathered.
///
/// Building the list means walking every wall and every piece of the storey,
/// so a caller measuring many pieces of the same storey — a layout check, a
/// dry run — gathers it once instead of once per piece.
///
/// The piece being measured is expected to be absent from `solids`; it would
/// otherwise be its own obstacle.
pub fn clearance_against(solids: &[Obstacle], piece: &Furniture, dir: Dir, max: f64) -> Clearance {
    let (min, max_pt) = plan_bounds(piece);
    let z = piece.height_range();
    let across = dir.axis.across();
    // The band in front of that face, 2 cm in from the corners so a
    // neighbour that merely touches a corner does not count.
    let (b0, b1) = match across {
        Axis::X => (min.x + 2.0, max_pt.x - 2.0),
        Axis::Y => (min.y + 2.0, max_pt.y - 2.0),
    };
    let face = if dir.sign >= 0.0 {
        dir.axis.of(max_pt)
    } else {
        dir.axis.of(min)
    };
    let (f0, f1) = if dir.sign >= 0.0 {
        (face + 0.1, face + max)
    } else {
        (face - max, face - 0.1)
    };
    let band = match dir.axis {
        Axis::X => [
            Point2::new(f0, b0),
            Point2::new(f1, b0),
            Point2::new(f1, b1),
            Point2::new(f0, b1),
        ],
        Axis::Y => [
            Point2::new(b0, f0),
            Point2::new(b1, f0),
            Point2::new(b1, f1),
            Point2::new(b0, f1),
        ],
    };
    if b1 <= b0 {
        return Clearance {
            dir,
            cm: max,
            against: None,
            name: String::new(),
        };
    }
    let band_poly = crate::geometry::to_polygon(&band);
    let mut best = max;
    let mut against = None;
    let mut name = String::new();
    let own: Vec<crate::ids::FurnitureId> = piece.flatten().iter().map(|f| f.id).collect();
    for o in solids {
        if matches!(o.what, Solid::Piece(id) if own.contains(&id)) {
            continue;
        }
        // Over your head or under your feet: not in the way.
        if o.z.1 <= z.0 + 1.0 || o.z.0 >= z.1 - 1.0 {
            continue;
        }
        let shared = o.polygon().intersection(&band_poly);
        if shared.unsigned_area() <= 25.0 {
            continue;
        }
        let (lo, hi) = o.spans(dir.axis);
        let d = if dir.sign >= 0.0 {
            lo - face
        } else {
            face - hi
        };
        // Negative when it reaches past the face into the piece: touching
        // and sitting 5 cm inside are opposite answers, not the same 0.
        if d < best {
            best = d;
            against = Some(o.what.clone());
            name.clone_from(&o.name);
        }
    }
    Clearance {
        dir,
        cm: (best * 10.0).round() / 10.0,
        against,
        name,
    }
}

/// Distance between two boxes along an axis: the gap between their facing
/// sides, or minus how far they overlap along it.
pub fn gap(a: (Point2, Point2), b: (Point2, Point2), axis: Axis) -> f64 {
    let (a0, a1) = (axis.of(a.0), axis.of(a.1));
    let (b0, b1) = (axis.of(b.0), axis.of(b.1));
    if b0 >= a1 {
        b0 - a1
    } else if a0 >= b1 {
        a0 - b1
    } else {
        -(a1.min(b1) - a0.max(b0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::Wall;
    use crate::ids::{FurnitureId, WallId};

    fn piece(id: u64, at: (f64, f64), size: (f64, f64, f64), angle: f64) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: "box".into(),
            name: format!("peça {id}"),
            position: Point2::new(at.0, at.1),
            width: size.0,
            depth: size.1,
            height: size.2,
            angle,
            ..Furniture::default()
        }
    }

    /// The tall unit of a kitchen island: fronts, drawers and a door built on
    /// the kitchen side, one blind panel toward the living room, and an angle
    /// that says the opposite because that is how the group was imported.
    fn island_tower(angle: f64) -> Furniture {
        // Body 80 wide × 64 deep, kitchen side at y = 421, living room at 485.
        let mut group = piece(1, (455.0, 453.0), (80.0, 64.0, 280.0), angle);
        let panel = |id: u64, name: &str, y: f64, h: f64, elev: f64| {
            let mut part = piece(id, (455.0, y), (78.0, 2.0, h), 0.0);
            part.name = name.to_owned();
            part.elevation = elev;
            part
        };
        group.children = vec![
            panel(2, "Frente fixa de madeira sob cuba", 422.0, 40.0, 0.0),
            panel(3, "Gaveta superior", 422.0, 30.0, 40.0),
            panel(4, "Gaveta inferior", 422.0, 30.0, 70.0),
            panel(5, "Porta do forno", 422.0, 60.0, 100.0),
            panel(6, "Painel cego voltado à sala", 484.0, 280.0, 0.0),
        ];
        group
    }

    #[test]
    fn a_group_opens_where_its_fronts_are_not_where_its_angle_points() {
        // `angle` 0 says the front looks at +y — the living room — but every
        // front panel was built at y = 422, the kitchen side.
        let tower = island_tower(0.0);
        assert_eq!(facing(&tower), "-y");
        assert_eq!(facing_disagrees(&tower), Some(("-y", "+y")));

        // Read as built: turned to its fronts, over the same floor.
        let frame = built_frame(&tower).expect("a frame");
        assert_eq!(snap(frame.angle).name(), "-y");
        let ((a0, a1), (b0, b1)) = (plan_bounds(&tower), plan_bounds(&frame));
        assert!(a0.distance(b0) < 1e-9 && a1.distance(b1) < 1e-9);
        let front = frame.to_plan((0.0, 1.0));
        assert!(front.y < frame.position.y, "local +y is the built front");

        // Turned to agree with what is built, there is nothing to report.
        let turned = island_tower(180.0);
        assert!(built_frame(&turned).is_none());
        assert_eq!(facing(&turned), "-y");
        assert_eq!(facing_disagrees(&turned), None);
    }

    #[test]
    fn a_piece_on_its_own_and_an_even_group_still_read_their_angle() {
        let alone = piece(1, (0.0, 0.0), (60.0, 60.0, 90.0), 90.0);
        assert_eq!(facing(&alone), "-x");
        assert_eq!(facing_disagrees(&alone), None);

        // Two panels that say nothing about themselves, one on each face:
        // the group is a box as far as anyone can tell, so `angle` decides.
        let mut even = piece(1, (0.0, 0.0), (80.0, 60.0, 90.0), 0.0);
        let mut one = piece(2, (0.0, -29.0), (78.0, 2.0, 88.0), 0.0);
        one.name = "Painel 1".into();
        let mut other = piece(3, (0.0, 29.0), (78.0, 2.0, 88.0), 0.0);
        other.name = "Painel 2".into();
        even.children = vec![one, other];
        assert_eq!(facing(&even), "+y", "the angle is all there is to go on");
        assert_eq!(facing_disagrees(&even), None);
    }

    #[test]
    fn bounds_resolve_a_quarter_turn() {
        let turned = piece(1, (100.0, 100.0), (200.0, 60.0, 90.0), 90.0);
        let (min, max) = plan_bounds(&turned);
        assert!((max.x - min.x - 60.0).abs() < 1e-6, "{min:?} {max:?}");
        assert!((max.y - min.y - 200.0).abs() < 1e-6, "{min:?} {max:?}");
        assert_eq!(facing(&piece(1, (0.0, 0.0), (10.0, 10.0, 10.0), 0.0)), "+y");
        assert_eq!(
            facing(&piece(1, (0.0, 0.0), (10.0, 10.0, 10.0), 90.0)),
            "-x"
        );
        assert_eq!(
            facing(&piece(1, (0.0, 0.0), (10.0, 10.0, 10.0), 180.0)),
            "-y"
        );
        assert_eq!(
            facing(&piece(1, (0.0, 0.0), (10.0, 10.0, 10.0), 270.0)),
            "+x"
        );
    }

    fn room(home: &mut Home, w: f64, d: f64) {
        let corners = [(0.0, 0.0), (w, 0.0), (w, d), (0.0, d)];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            let mut wall = Wall::new(
                WallId(k as u64 + 1),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            );
            wall.thickness = 20.0;
            home.walls.push(wall);
        }
    }

    #[test]
    fn free_span_reads_a_corridor_between_two_counters() {
        let mut home = Home::default();
        room(&mut home, 400.0, 300.0);
        // Two 60 cm counters facing each other, 100 cm apart.
        home.furniture
            .push(piece(10, (200.0, 40.0), (300.0, 60.0, 90.0), 0.0));
        home.furniture
            .push(piece(11, (200.0, 230.0), (300.0, 60.0, 90.0), 0.0));
        let spans = free_span(&home, Axis::Y, 200.0, Some((0.0, 300.0)), (0.0, 200.0));
        // Wall, counter, corridor, counter, the strip behind it, wall.
        let names: Vec<&str> = spans
            .iter()
            .map(|s| {
                if s.is_free() {
                    "livre"
                } else {
                    s.name.as_str()
                }
            })
            .collect();
        assert_eq!(
            names,
            ["parede", "peça 10", "livre", "peça 11", "livre", "parede"],
            "{spans:#?}"
        );
        let corridor = spans
            .iter()
            .filter(|s| s.is_free())
            .map(Span::cm)
            .fold(0.0, f64::max);
        assert!((corridor - 130.0).abs() < 0.5, "{spans:#?}");
        // And the same answer through `clearance`.
        let piece = home.piece(FurnitureId(10)).unwrap();
        let c = clearance(
            &home,
            piece,
            Dir {
                axis: Axis::Y,
                sign: 1.0,
            },
            MAX_REACH,
        );
        assert!((c.cm - 130.0).abs() < 0.5, "{c:?}");
        assert_eq!(c.against, Some(Solid::Piece(FurnitureId(11))));
    }

    #[test]
    fn overlapping_solids_do_not_make_the_probe_walk_backwards() {
        // Two pieces that overlap along the probe: the spans must still come
        // out in order, covering the run once, with no negative stretch.
        let mut home = Home::default();
        home.furniture = vec![
            piece(1, (100.0, 100.0), (100.0, 100.0, 90.0), 0.0),
            piece(2, (160.0, 100.0), (100.0, 100.0, 90.0), 0.0),
        ];

        let spans = free_span(&home, Axis::X, 100.0, Some((0.0, 300.0)), (0.0, 200.0));
        assert!(
            spans.iter().all(|s| s.cm() > 0.0),
            "no stretch runs backwards: {spans:?}"
        );
        for pair in spans.windows(2) {
            assert!(
                (pair[1].from - pair[0].to).abs() < 0.01,
                "the spans join end to end: {spans:?}"
            );
        }
        let covered: f64 = spans.iter().filter(|s| !s.is_free()).map(Span::cm).sum();
        // 50 → 210 covered once, not 100 + 100 counted twice.
        assert!((covered - 160.0).abs() < 0.1, "{spans:?}");
    }

    #[test]
    fn touching_and_sitting_inside_are_different_clearances() {
        // Two stones side by side: 0 when they touch, minus the overlap when
        // one is pushed 5.5 cm into the other — never the same number.
        let mut home = Home::default();
        home.furniture = vec![
            piece(1, (50.0, 30.0), (100.0, 60.0, 90.0), 0.0),
            piece(2, (130.0, 30.0), (60.0, 60.0, 90.0), 0.0),
        ];
        let left = Dir {
            axis: Axis::X,
            sign: -1.0,
        };
        let touching = clearance(&home, home.piece(FurnitureId(2)).unwrap(), left, MAX_REACH);
        assert!(touching.cm.abs() < 0.05, "{touching:?}");
        assert_eq!(touching.against, Some(Solid::Piece(FurnitureId(1))));

        home.furniture[1].position.x -= 5.5;
        let inside = clearance(&home, home.piece(FurnitureId(2)).unwrap(), left, MAX_REACH);
        assert!((inside.cm + 5.5).abs() < 0.05, "{inside:?}");
        assert_eq!(inside.against, Some(Solid::Piece(FurnitureId(1))));

        let (a, b) = (
            plan_bounds(home.piece(FurnitureId(1)).unwrap()),
            plan_bounds(home.piece(FurnitureId(2)).unwrap()),
        );
        assert!((gap(a, b, Axis::X) + 5.5).abs() < 1e-9, "the tape agrees");
        assert!((gap(b, a, Axis::X) + 5.5).abs() < 1e-9, "from either side");
    }

    /// The pull-out broom cupboard of a real plan, part by part from its
    /// bounds: fronts and brass pulls on -y, a right side still named after
    /// the door it used to be.
    fn broom_cupboard(with_pulls: bool) -> Furniture {
        let part = |id: u64, name: &str, a: (f64, f64), b: (f64, f64), h: f64| {
            let mut p = piece(
                id,
                (f64::midpoint(a.0, b.0), f64::midpoint(a.1, b.1)),
                (b.0 - a.0, b.1 - a.1, h),
                0.0,
            );
            p.name = name.to_owned();
            p
        };
        let mut group = part(
            1218,
            "Vassoureiro extraível",
            (615.0, 415.6),
            (645.0, 484.1),
            280.0,
        );
        group.children = vec![
            part(
                957,
                "Vassoureiro — lateral esquerda",
                (615.0, 419.1),
                (617.0, 484.1),
                280.0,
            ),
            part(
                959,
                "Vassoureiro — fundo",
                (617.0, 482.1),
                (643.0, 484.1),
                280.0,
            ),
            part(
                960,
                "Vassoureiro — base",
                (617.0, 420.1),
                (643.0, 483.1),
                2.0,
            ),
            part(
                1209,
                "Vassoureiro — fechamento lateral direito (era a porta para a passagem)",
                (643.0, 419.1),
                (645.0, 484.1),
                280.0,
            ),
            part(
                1210,
                "Vassoureiro extraível — frente 30 × 195 cm",
                (615.0, 417.1),
                (645.0, 419.1),
                195.0,
            ),
            part(
                1211,
                "Vassoureiro — frente do maleiro superior 30 × 71 cm",
                (615.0, 417.1),
                (645.0, 419.1),
                71.0,
            ),
            part(
                1214,
                "corrediça telescópica esquerda 60 cm",
                (617.0, 421.1),
                (618.5, 481.1),
                4.0,
            ),
            part(
                1215,
                "corrediça telescópica direita 60 cm",
                (641.5, 421.1),
                (643.0, 481.1),
                4.0,
            ),
            part(
                1217,
                "travessa traseira de rigidez",
                (617.0, 479.1),
                (643.0, 481.1),
                60.0,
            ),
        ];
        if with_pulls {
            group.children.push(part(
                1212,
                "Vassoureiro extraível — puxador barra latão",
                (628.5, 415.6),
                (631.5, 417.1),
                30.0,
            ));
            group.children.push(part(
                1213,
                "Vassoureiro — puxador do maleiro",
                (628.5, 415.6),
                (631.5, 417.1),
                15.0,
            ));
        }
        group
    }

    #[test]
    fn pulls_say_where_a_cupboard_opens_and_a_doubt_is_said_not_guessed() {
        // Its pulls settle it: -y, the kitchen corridor, not the angle's +y.
        let cupboard = broom_cupboard(true);
        assert_eq!(facing(&cupboard), "-y");
        assert_eq!(facing_disagrees(&cupboard), Some(("-y", "+y")));
        assert_eq!(front_unclear(&cupboard), None);

        // Without them, two fronts on -y and a side named after a door on +x
        // settle nothing — and that is said, with both candidates.
        let doubtful = broom_cupboard(false);
        assert_eq!(facing_disagrees(&doubtful), None);
        assert_eq!(front_unclear(&doubtful), Some(vec!["-y", "+x"]));
        let mut home = Home::default();
        home.furniture.push(doubtful);
        let issues = crate::check_layout(&home);
        assert!(
            issues.iter().any(|i| matches!(i, crate::Issue::UnclearFront { candidates, .. } if candidates[0] == "-y")),
            "{issues:?}"
        );

        // A plain box of boards with no front named anywhere says nothing.
        let mut plain = broom_cupboard(false);
        plain
            .children
            .retain(|p| !p.name.contains("frente") && !p.name.contains("porta"));
        assert_eq!(front_unclear(&plain), None);
    }

    #[test]
    fn resizing_a_group_does_not_turn_it_around() {
        // A folding table: its top reaches +y, two 3 × 3 cm posts hold it on
        // the -y edge, and angle 0 says it faces +y.
        let table = |w: f64| {
            let s = w / 110.0;
            let mut group = piece(1, (465.0, 497.0), (w, 36.0, 29.0), 0.0);
            let part = |id: u64, x: f64, y: f64, pw: f64, d: f64| {
                piece(id, (465.0 + (x - 465.0) * s, y), (pw * s, d, 28.0), 0.0)
            };
            group.children = vec![
                part(2, 465.0, 500.0, 110.0, 30.0),
                part(3, 432.0, 480.5, 3.0, 3.0),
                part(4, 498.0, 480.5, 3.0, 3.0),
                part(5, 432.0, 497.0, 3.0, 24.0),
                part(6, 498.0, 497.0, 3.0, 24.0),
            ];
            group
        };
        let narrow = table(110.0);
        let wide = table(119.0);
        assert_eq!(facing(&narrow), "+y");
        assert_eq!(facing(&wide), "+y", "posts are not fronts");
        assert_eq!(facing_disagrees(&wide), None);
    }

    #[test]
    fn a_probe_sees_a_cabinet_made_of_thin_boards() {
        // A broom cupboard 30 wide, 280 tall, drawn as 1.8 cm boards: two
        // sides, a back and a front, and nothing thicker.
        let mut home = Home::default();
        let board = |id: u64, at: (f64, f64), w: f64, d: f64| {
            let mut part = piece(id, at, (w, d, 280.0), 0.0);
            part.name = format!("chapa {id}");
            part
        };
        let mut cupboard = piece(1, (630.0, 450.0), (30.0, 60.0, 280.0), 0.0);
        cupboard.name = "vassoureiro".into();
        cupboard.children = vec![
            board(2, (615.9, 450.0), 1.8, 60.0),
            board(3, (644.1, 450.0), 1.8, 60.0),
            board(4, (630.0, 420.9), 26.4, 1.8),
            board(5, (630.0, 479.1), 26.4, 1.8),
        ];
        home.furniture.push(cupboard);
        // And a rug, which a probe still walks over.
        home.furniture
            .push(piece(9, (700.0, 450.0), (60.0, 40.0, 1.0), 0.0));

        let spans = free_span(&home, Axis::X, 450.0, Some((612.0, 760.0)), (0.0, 280.0));
        let solid: Vec<Option<Solid>> = spans
            .iter()
            .filter(|s| !s.is_free())
            .map(|s| s.what.clone())
            .collect();
        assert_eq!(
            solid,
            vec![
                Some(Solid::Piece(FurnitureId(2))),
                Some(Solid::Piece(FurnitureId(3)))
            ],
            "{spans:#?}"
        );
        assert!((spans[1].from - 615.0).abs() < 0.01, "{spans:#?}");
    }

    #[test]
    fn clearance_stops_at_the_wall_behind() {
        let mut home = Home::default();
        room(&mut home, 400.0, 300.0);
        home.furniture
            .push(piece(10, (200.0, 45.0), (300.0, 60.0, 90.0), 0.0));
        let piece = home.piece(FurnitureId(10)).unwrap();
        let c = clearance(
            &home,
            piece,
            Dir {
                axis: Axis::Y,
                sign: -1.0,
            },
            MAX_REACH,
        );
        assert!((c.cm - 5.0).abs() < 0.5, "{c:?}");
        assert!(matches!(c.against, Some(Solid::Wall(_))), "{c:?}");
    }

    #[test]
    fn a_wall_cabinet_does_not_shrink_the_floor_below_it() {
        let mut home = Home::default();
        room(&mut home, 400.0, 300.0);
        let mut high = piece(12, (200.0, 100.0), (200.0, 35.0, 70.0), 0.0);
        high.elevation = 150.0;
        home.furniture.push(high);
        let low = free_span(&home, Axis::Y, 200.0, Some((0.0, 300.0)), (0.0, 140.0));
        assert!(
            low.iter()
                .all(|s| s.is_free() || matches!(s.what, Some(Solid::Wall(_)))),
            "{low:#?}"
        );
        let high = free_span(&home, Axis::Y, 200.0, Some((0.0, 300.0)), (140.0, 220.0));
        assert!(
            high.iter()
                .any(|s| s.what == Some(Solid::Piece(FurnitureId(12)))),
            "{high:#?}"
        );
    }
}

/// An annotation that no longer matches the drawing under it.
///
/// A plan of joinery lives on its dimensions and notes; one that still reads
/// 66,5 over a corridor that is now 86 is worse than no note at all, and the
/// only way to catch it used to be reading every one of them by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Stale {
    pub id: ElementId,
    /// What the annotation says, cm.
    pub drawn: f64,
    /// What the drawing measures there now, cm.
    pub measured: f64,
    /// The element the measurement was taken against, when there is one.
    pub against: Option<ElementId>,
    pub text: String,
}

/// Where a dimension end that holds onto something sits now.
///
/// Only the coordinate along the dimension moves: the other one is where
/// whoever drew it put the line, and moving that would drag the annotation
/// across the drawing instead of keeping it where it reads.
#[must_use]
pub fn hold_point(home: &Home, hold: &Hold, axis: Axis, keep_across: f64) -> Option<Point2> {
    let (min, max) = element_bounds(home, hold.id)?;
    let along = match hold.edge.as_deref().map(str::trim) {
        Some("+x") => max.x,
        Some("-x") => min.x,
        Some("+y") => max.y,
        Some("-y") => min.y,
        _ => match axis {
            Axis::X => f64::midpoint(min.x, max.x),
            Axis::Y => f64::midpoint(min.y, max.y),
        },
    };
    Some(match axis {
        Axis::X => Point2::new(along, keep_across),
        Axis::Y => Point2::new(keep_across, along),
    })
}

/// Dimensions that hold onto something and no longer sit where it is: the
/// same dimension, measured again. Everything else is left alone.
///
/// A dimension between two bare points is not touched, because nothing says
/// what it was meant to mark; [`anchor_dimensions`] turns those into held
/// ones while the drawing still matches them.
#[must_use]
pub fn dimensions_following(home: &Home) -> Vec<Dimension> {
    let mut out = Vec::new();
    for dim in &home.dimensions {
        let Some(holds) = dim.holds.as_ref() else {
            continue;
        };
        let (dx, dy) = (dim.end.x - dim.start.x, dim.end.y - dim.start.y);
        let axis = if dy.abs() <= dx.abs() {
            Axis::X
        } else {
            Axis::Y
        };
        let across = |p: Point2| match axis {
            Axis::X => p.y,
            Axis::Y => p.x,
        };
        let (Some(start), Some(end)) = (
            hold_point(home, &holds[0], axis, across(dim.start)),
            hold_point(home, &holds[1], axis, across(dim.end)),
        ) else {
            continue; // what it held is gone; `stale` reports that
        };
        if start.distance(dim.start) > 1e-6 || end.distance(dim.end) > 1e-6 {
            out.push(Dimension {
                start,
                end,
                ..dim.clone()
            });
        }
    }
    out
}

/// How far from a face a dimension end may have been left and still be read
/// as meant for it, cm.
const LOOSE_REACH: f64 = 40.0;

/// A dimension that holds onto nothing and has one end left in the air.
#[derive(Debug, Clone, PartialEq)]
pub struct LooseEnd {
    /// `0` for the start, `1` for the end.
    pub end: usize,
    /// The face nearest to where that end was left.
    pub near: Hold,
    /// What the dimension would read with that end on that face, cm.
    pub measured: f64,
}

/// The axis a dimension runs along, when it runs along one.
fn dimension_axis(dim: &Dimension) -> Option<Axis> {
    let (dx, dy) = (dim.end.x - dim.start.x, dim.end.y - dim.start.y);
    if dy.abs() <= 0.5 && dx.abs() > 0.5 {
        Some(Axis::X)
    } else if dx.abs() <= 0.5 && dy.abs() > 0.5 {
        Some(Axis::Y)
    } else {
        None
    }
}

/// A dimension whose drawing moved out from under one of its ends.
///
/// One end still sits on a face; the other touches nothing, and a face is a
/// few centimeters away along the dimension — a counter recessed 19 cm under
/// the dimension of the corridor in front of it. The number on the plan is
/// the old corridor. An end inside a solid (a wall's axis) or far from any
/// face was drawn that way and is left alone.
#[must_use]
pub fn loose_end(home: &Home, dim: &Dimension) -> Option<LooseEnd> {
    if dim.holds.is_some() {
        return None;
    }
    let axis = dimension_axis(dim)?;
    let ends = [dim.start, dim.end];
    let touches = [
        touching(home, dim.start, axis, dim.end).is_some(),
        touching(home, dim.end, axis, dim.start).is_some(),
    ];
    let end = match touches {
        [true, false] => 1,
        [false, true] => 0,
        _ => return None,
    };
    let (at, other) = (ends[end], ends[1 - end]);
    let along = axis.of(at);
    let across = match axis {
        Axis::X => at.y,
        Axis::Y => at.x,
    };
    let mut best: Option<(f64, f64, Hold)> = None;
    for solid in obstacles(home, &|_| false) {
        let (min, max) = bounds_of(&solid.outline);
        let (lo, hi, across_lo, across_hi) = match axis {
            Axis::X => (min.x, max.x, min.y, max.y),
            Axis::Y => (min.y, max.y, min.x, max.x),
        };
        if across < across_lo - 1.0 || across > across_hi + 1.0 {
            continue;
        }
        if along > lo + 1.0 && along < hi - 1.0 {
            return None;
        }
        for (face, edge) in [(lo, -1.0), (hi, 1.0)] {
            let away = (face - along).abs();
            if away > LOOSE_REACH || best.as_ref().is_some_and(|(d, ..)| away >= *d) {
                continue;
            }
            let edge = match (axis, edge > 0.0) {
                (Axis::X, true) => "+x",
                (Axis::X, false) => "-x",
                (Axis::Y, true) => "+y",
                (Axis::Y, false) => "-y",
            };
            best = Some((
                away,
                face,
                Hold {
                    id: solid.what.id(),
                    edge: Some(edge.to_owned()),
                },
            ));
        }
    }
    let (_, face, near) = best?;
    Some(LooseEnd {
        end,
        near,
        measured: ((face - axis.of(other)).abs() * 10.0).round() / 10.0,
    })
}

/// Ties every axis-aligned dimension to whatever its ends touch right now, so
/// that from here on it follows the drawing.
///
/// Run while the drawing and its dimensions still agree: this reads the
/// current geometry as the intent. Dimensions that already hold onto
/// something, and ends that touch nothing, are left as they are.
///
/// A dimension whose hold died with the piece it held is not left holding a
/// ghost: it is tied again to what its ends touch now, or, touching nothing,
/// let go — returned with no hold — so it stops being reported as stale on
/// every review for a number that may well be right.
#[must_use]
pub fn anchor_dimensions(home: &Home) -> Vec<Dimension> {
    let mut out = Vec::new();
    for dim in &home.dimensions {
        let dead = dim
            .holds
            .as_ref()
            .is_some_and(|holds| holds.iter().any(|h| element_bounds(home, h.id).is_none()));
        if dim.holds.is_some() && !dead {
            continue;
        }
        if dead {
            let unheld = Dimension {
                holds: None,
                ..dim.clone()
            };
            let tied = dimension_axis(&unheld).and_then(|axis| {
                Some([
                    touching(home, unheld.start, axis, unheld.end)?,
                    touching(home, unheld.end, axis, unheld.start)?,
                ])
            });
            out.push(Dimension {
                holds: tied,
                ..unheld
            });
            continue;
        }
        let Some(axis) = dimension_axis(dim) else {
            continue;
        };
        let (Some(start), Some(end)) = (
            touching(home, dim.start, axis, dim.end),
            touching(home, dim.end, axis, dim.start),
        ) else {
            continue;
        };
        out.push(Dimension {
            holds: Some([start, end]),
            ..dim.clone()
        });
    }
    out
}

/// The element face a dimension end sits on: the side of a solid that the
/// end touches, looked for from the direction the dimension comes from.
fn touching(home: &Home, at: Point2, axis: Axis, from: Point2) -> Option<Hold> {
    let toward = if axis.of(from) >= axis.of(at) {
        1.0
    } else {
        -1.0
    };
    let near = |a: f64, b: f64| (a - b).abs() <= 1.0;
    let mut best: Option<(f64, Hold)> = None;
    for solid in obstacles(home, &|_| false) {
        let (min, max) = bounds_of(&solid.outline);
        // The end has to be within the solid across the measured axis, and on
        // one of its two faces along it.
        let (lo, hi, across_lo, across_hi) = match axis {
            Axis::X => (min.x, max.x, min.y, max.y),
            Axis::Y => (min.y, max.y, min.x, max.x),
        };
        let across = match axis {
            Axis::X => at.y,
            Axis::Y => at.x,
        };
        if across < across_lo - 1.0 || across > across_hi + 1.0 {
            continue;
        }
        let along = axis.of(at);
        // The face the dimension leaves from: the one on the side it runs to.
        let (edge, at_face) = if toward > 0.0 {
            (if axis == Axis::X { "+x" } else { "+y" }, hi)
        } else {
            (if axis == Axis::X { "-x" } else { "-y" }, lo)
        };
        let other = if toward > 0.0 { lo } else { hi };
        let (edge, face) = if near(along, at_face) {
            (edge, at_face)
        } else if near(along, other) {
            (
                match (axis, toward > 0.0) {
                    (Axis::X, true) => "-x",
                    (Axis::X, false) => "+x",
                    (Axis::Y, true) => "-y",
                    _ => "+y",
                },
                other,
            )
        } else {
            continue;
        };
        let away = (face - along).abs();
        if best.as_ref().is_none_or(|(d, _)| away < *d) {
            best = Some((
                away,
                Hold {
                    id: solid.what.id(),
                    edge: Some(edge.to_owned()),
                },
            ));
        }
    }
    best.map(|(_, hold)| hold)
}

fn bounds_of(points: &[Point2]) -> (Point2, Point2) {
    let mut min = Point2::new(f64::MAX, f64::MAX);
    let mut max = Point2::new(f64::MIN, f64::MIN);
    for p in points {
        min = Point2::new(min.x.min(p.x), min.y.min(p.y));
        max = Point2::new(max.x.max(p.x), max.y.max(p.y));
    }
    (min, max)
}

/// How far a dimension may be off before it counts as stale, cm.
const STALE_TOLERANCE: f64 = 1.0;

/// How far from a piece a note can sit and still be about it, cm.
const BESIDE: f64 = 40.0;

/// The piece a note is about, when that is not a guess.
///
/// `ref` says it outright. Standing on the piece says it too. Failing both,
/// a note right beside a single piece that still shares one of its numbers
/// is about that piece: a note reading `65,83 × 84 × 87` next to a cabinet
/// now 60 × 72 × 87 is the same note, with two numbers left behind — and
/// that is exactly the one worth reporting. A note that shares nothing with
/// what is near it is somebody else's note, and stays out of the report.
fn label_subject<'h>(home: &'h Home, label: &Label, sizes: &[Vec<f64>]) -> Option<&'h Furniture> {
    let pieces: Vec<&Furniture> = home.furniture.iter().flat_map(Furniture::flatten).collect();
    if let Some(id) = label.about {
        return pieces.into_iter().find(|f| f.id == id);
    }
    let matches = |f: &Furniture| {
        let actual = [f.width, f.depth, f.height];
        sizes
            .iter()
            .flatten()
            .any(|w| actual.iter().any(|a| (a - w).abs() <= STALE_TOLERANCE))
    };
    let standing_on = pieces.iter().copied().find(|f| {
        let (min, max) = plan_bounds(f);
        (min.x..=max.x).contains(&label.position.x) && (min.y..=max.y).contains(&label.position.y)
    });
    if standing_on.is_some() {
        return standing_on;
    }
    let mut beside = pieces.into_iter().filter(|f| {
        let (min, max) = plan_bounds(f);
        let dx = (min.x - label.position.x)
            .max(label.position.x - max.x)
            .max(0.0);
        let dy = (min.y - label.position.y)
            .max(label.position.y - max.y)
            .max(0.0);
        dx.hypot(dy) <= BESIDE && matches(f)
    });
    let first = beside.next()?;
    // Two candidates and the note could be about either: say nothing rather
    // than report the wrong piece.
    beside.next().is_none().then_some(first)
}

/// Annotations that stopped telling the truth about the drawing.
///
/// A dimension states the distance between its own two points, so it cannot
/// be wrong about that — what goes wrong is that the drawing moved out from
/// under it. One that [holds](crate::Hold) onto what it marks is measured
/// again on every change and never drifts; one whose hold is gone is
/// reported here, because it now marks nothing at all.
///
/// A label is the opposite: its numbers are typed by hand and nothing keeps
/// them. Those are checked against the piece the label is about — the one it
/// names with `ref`, the one it stands on, or the one beside it that some of
/// its numbers still match — which is where the plan of a joiner quietly goes
/// wrong after a resize.
pub fn stale_annotations(home: &Home) -> Vec<Stale> {
    check_annotations(home).stale
}

/// What [`check_annotations`] found, and how much it could look at.
///
/// An empty list reads the same for a plan whose notes all agree with the
/// drawing and for one where no note could be compared with anything — two
/// hundred index codes, none tied to a piece. The counts tell them apart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnnotationCheck {
    pub stale: Vec<Stale>,
    /// Dimensions holding onto what they mark.
    pub dimensions: usize,
    /// Dimensions holding onto nothing: only a loose end can be caught on
    /// them, and a number that is wrong with both ends in place cannot.
    pub unanchored: usize,
    /// Labels with sizes, compared with the piece they are about.
    pub labels: usize,
    /// Clauses of piece names with sizes, compared with the piece itself.
    pub names: usize,
    /// Sizes written where nothing can confirm them: a label about no piece,
    /// or a name giving an inner opening, a door leaf or a whole set — which
    /// the model has no field for, and which is why it went in the name.
    pub unverified: Vec<(ElementId, String)>,
}

/// Words that say a size is of something the piece's box does not measure.
const NOT_THE_BOX: [&str; 11] = [
    "vao", "nicho", "intern", "livre", "folha", "conjunto", "total", "abertura", "inner",
    "opening", "leaf",
];

/// Sizes a piece's name gives, clause by clause: `N × M` groups, and a lone
/// `N cm`, which is how a module is named ("módulo 70 cm").
fn named_sizes(clause: &str) -> Vec<Vec<f64>> {
    let mut sizes = written_sizes(clause);
    let tokens: Vec<&str> = clause.split_whitespace().collect();
    let in_groups: Vec<f64> = sizes.iter().flatten().copied().collect();
    for (i, token) in tokens.iter().enumerate() {
        let lower = token.to_lowercase();
        let (number, unit) = match lower.strip_suffix("cm") {
            Some(n) if !n.is_empty() => (n.to_owned(), true),
            _ => (
                lower.clone(),
                tokens.get(i + 1).is_some_and(|t| {
                    t.to_lowercase().trim_matches(|c: char| !c.is_alphabetic()) == "cm"
                }),
            ),
        };
        let Ok(value) = number
            .trim_matches(|c: char| !c.is_ascii_digit())
            .replace(',', ".")
            .parse::<f64>()
        else {
            continue;
        };
        if unit && !in_groups.contains(&value) {
            sizes.push(vec![value]);
        }
    }
    sizes
}

/// The clauses of a name: split where the writer paused — `;`, `|`, a dash,
/// brackets, and a comma — but never inside a number, where the comma is the
/// decimal point this plan is written with (`80,5 cm`, `53,9 × 43 × 30,7`).
fn clauses(name: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for (k, &(at, c)) in chars.iter().enumerate() {
        let decimal = c == ','
            && k > 0
            && chars[k - 1].1.is_ascii_digit()
            && chars.get(k + 1).is_some_and(|(_, n)| n.is_ascii_digit());
        if matches!(c, ';' | '|' | '(' | ')' | '—' | '/') || (c == ',' && !decimal) {
            out.push(&name[start..at]);
            start = at + c.len_utf8();
        }
    }
    out.push(&name[start..]);
    out
}

enum NamedMeasure {
    Envelope,
    Profile(f64),
    Unverified,
}

// Ceiling assembly names describe their section, not the room-sized group
// envelope. Read the persisted generator parameters without depending on the
// joinery crate (which itself depends on core).
fn named_measure(piece: &Furniture, clause: &str, sizes: &[Vec<f64>]) -> NamedMeasure {
    let kind = piece.properties.get("joinery:kind").map(String::as_str);
    if !matches!(kind, Some("cove" | "shadow_gap")) {
        return NamedMeasure::Envelope;
    }
    let folded = crate::annotations::fold(clause);
    if folded.contains("comprimento") || folded.contains("length") || folded.contains("envolvente")
    {
        return NamedMeasure::Envelope;
    }
    if sizes.len() != 1 || sizes[0].len() != 1 {
        return NamedMeasure::Unverified;
    }
    let key = if folded.contains("teto") || folded.contains("ceiling") {
        "ceiling"
    } else if kind == Some("cove") {
        if folded.contains("queda") || folded.contains("rebaixo") || folded.contains("drop") {
            "drop"
        } else if folded.contains("rasgo") || folded.contains("slot") {
            "slot"
        } else {
            "width"
        }
    } else if folded.contains("profundidade") || folded.contains("depth") {
        "depth"
    } else {
        "gap"
    };
    let measured = piece
        .properties
        .get("joinery:params")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .filter(|params| params["kind"].as_str() == kind)
        .and_then(|params| params[key].as_f64())
        .filter(|value| value.is_finite() && *value > 0.0);
    measured.map_or(NamedMeasure::Unverified, NamedMeasure::Profile)
}

/// Every annotation checked against the drawing, with how many were.
#[must_use]
pub fn check_annotations(home: &Home) -> AnnotationCheck {
    let mut report = AnnotationCheck::default();
    let out = &mut report.stale;
    for dim in &home.dimensions {
        let Some(holds) = dim.holds.as_ref() else {
            report.unanchored += 1;
            if let Some(loose) = loose_end(home, dim) {
                out.push(Stale {
                    id: dim.id.into(),
                    drawn: (dim.length() * 10.0).round() / 10.0,
                    measured: loose.measured,
                    against: Some(loose.near.id),
                    text: format!(
                        "its {} touches nothing; {} is {} cm from it",
                        if loose.end == 0 { "start" } else { "end" },
                        loose.near.id,
                        (((dim.length() - loose.measured).abs()) * 10.0).round() / 10.0,
                    ),
                });
            }
            continue;
        };
        report.dimensions += 1;
        for hold in holds {
            if element_bounds(home, hold.id).is_none() {
                out.push(Stale {
                    id: dim.id.into(),
                    drawn: (dim.length() * 10.0).round() / 10.0,
                    measured: (dim.length() * 10.0).round() / 10.0,
                    against: Some(hold.id),
                    text: format!("{} is gone; this no longer marks anything", hold.id),
                });
                break;
            }
        }
    }

    for label in &home.labels {
        // A note about a piece that was deleted points at nothing, whatever
        // it says: the index code of a niche now standing over the sink.
        if let Some(id) = label.about
            && home
                .furniture
                .iter()
                .flat_map(Furniture::flatten)
                .all(|f| f.id != id)
        {
            out.push(Stale {
                id: label.id.into(),
                drawn: 0.0,
                measured: 0.0,
                against: Some(id.into()),
                text: format!("{id} is gone; \"{}\" is about nothing", label.text),
            });
            continue;
        }
        let sizes = written_sizes(&label.text);
        if sizes.is_empty() {
            continue;
        }
        let Some(piece) = label_subject(home, label, &sizes) else {
            report
                .unverified
                .push((label.id.into(), label.text.clone()));
            continue;
        };
        report.labels += 1;
        let actual = [piece.width, piece.depth, piece.height];
        for written in sizes {
            // Any order: a note may read width × height × depth.
            if written
                .iter()
                .all(|w| actual.iter().any(|a| (a - w).abs() <= STALE_TOLERANCE))
            {
                continue;
            }
            // A note of three sizes is written width × depth × height, so
            // the pair to report is the one that moved in place — not the
            // nearest number, which would name the wrong dimension.
            let pairs: Vec<(f64, f64)> = if written.len() == actual.len() {
                written.iter().copied().zip(actual).collect()
            } else {
                written
                    .iter()
                    .map(|w| {
                        let closest = actual
                            .iter()
                            .copied()
                            .min_by(|a, b| (a - w).abs().total_cmp(&(b - w).abs()))
                            .unwrap_or(0.0);
                        (*w, closest)
                    })
                    .collect()
            };
            let (drawn, measured) = pairs
                .into_iter()
                .max_by(|a, b| (a.0 - a.1).abs().total_cmp(&(b.0 - b.1).abs()))
                .unwrap_or((0.0, 0.0));
            out.push(Stale {
                id: label.id.into(),
                drawn,
                measured: (measured * 10.0).round() / 10.0,
                against: Some(piece.id.into()),
                text: label.text.clone(),
            });
            break;
        }
    }

    // The names: where a plan drawn by a joiner keeps its sizes.
    for piece in home.furniture.iter().flat_map(Furniture::flatten) {
        for clause in clauses(&piece.name) {
            let sizes = named_sizes(clause);
            if sizes.is_empty() {
                continue;
            }
            let folded = crate::annotations::fold(clause);
            if NOT_THE_BOX.iter().any(|w| folded.contains(w)) {
                report
                    .unverified
                    .push((piece.id.into(), clause.trim().to_owned()));
                continue;
            }
            let actual = match named_measure(piece, clause, &sizes) {
                NamedMeasure::Envelope => vec![piece.width, piece.depth, piece.height],
                NamedMeasure::Profile(value) => vec![value],
                NamedMeasure::Unverified => {
                    report
                        .unverified
                        .push((piece.id.into(), clause.trim().to_owned()));
                    continue;
                }
            };
            report.names += 1;
            let wrong = sizes
                .iter()
                .flatten()
                .find(|w| !actual.iter().any(|a| (a - *w).abs() <= STALE_TOLERANCE));
            if let Some(written) = wrong {
                let closest = actual
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - written).abs().total_cmp(&(b - written).abs()))
                    .unwrap_or(0.0);
                report.stale.push(Stale {
                    id: piece.id.into(),
                    drawn: *written,
                    measured: (closest * 10.0).round() / 10.0,
                    against: Some(piece.id.into()),
                    text: piece.name.clone(),
                });
                break;
            }
        }
    }
    report
}

/// Size groups written in a note: `80 × 65 × 280`, `58x50`, `0,80 × 0,65`.
///
/// Only numbers joined by `×` or `x` are read, so a note that merely mentions
/// a number ("53A", "2 portas") is left alone.
fn written_sizes(text: &str) -> Vec<Vec<f64>> {
    // `80×65` and `58x50` are written without spaces as often as with them.
    let chars: Vec<char> = text.chars().collect();
    let mut spaced = String::with_capacity(text.len());
    for (i, c) in chars.iter().enumerate() {
        let times = *c == '×'
            || ((*c == 'x' || *c == 'X')
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(char::is_ascii_digit));
        if times {
            spaced.push_str(" × ");
        } else {
            spaced.push(*c);
        }
    }

    // Each group, and the unit written after it, if any, as centimeters per
    // unit: 100 for meters ("7,40 × 3,00 m"), 0.1 for millimeters.
    let mut out: Vec<(Vec<f64>, Option<f64>)> = Vec::new();
    let mut group: Vec<f64> = Vec::new();
    let mut linked = false;
    let flush =
        |group: &mut Vec<f64>, unit: Option<f64>, out: &mut Vec<(Vec<f64>, Option<f64>)>| {
            if group.len() > 1 {
                out.push((std::mem::take(group), unit));
            }
            group.clear();
        };
    let unit_of = |word: &str| match word.to_lowercase().as_str() {
        "m" => Some(100.0),
        "cm" => Some(1.0),
        "mm" => Some(0.1),
        _ => None,
    };
    for token in spaced.split_whitespace() {
        let clean = token.trim_matches(|c: char| !c.is_ascii_digit());
        if let Ok(value) = clean.replace(',', ".").parse::<f64>() {
            if !linked {
                flush(&mut group, None, &mut out);
            }
            group.push(value);
            linked = false;
            // A unit written onto the last number: `3,00m`.
            let unit = token
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == ',' || c == '.')
                .trim_end_matches(|c: char| !c.is_alphabetic());
            if !unit.is_empty() {
                flush(&mut group, unit_of(unit), &mut out);
            }
        } else if token == "×" {
            linked = true;
        } else {
            let unit = token.trim_matches(|c: char| !c.is_alphabetic());
            flush(&mut group, unit_of(unit), &mut out);
            linked = false;
        }
    }
    flush(&mut group, None, &mut out);

    // A note in meters ("0,80 × 0,65", "7,40 × 3,00 m") describes the same
    // piece in other units. Whole small numbers ("2 × 3 gavetas") are counts,
    // not sizes; a unit written says it outright.
    out.into_iter()
        .map(|(g, unit)| {
            let per = unit.unwrap_or_else(|| {
                if g.iter().all(|v| *v < 10.0 && v.fract() > 0.0) {
                    100.0
                } else {
                    1.0
                }
            });
            g.into_iter().map(|v| v * per).collect()
        })
        .collect()
}

#[cfg(test)]
mod stale_tests {
    use super::*;

    #[test]
    fn ceiling_profile_names_use_the_generator_section_not_the_group_box() {
        for (kind, name, params) in [
            (
                "cove",
                "Sanca perimetral do estar — 18 cm",
                serde_json::json!({"kind":"cove","width":18,"drop":12,"ceiling":300}),
            ),
            (
                "shadow_gap",
                "Tabica da galeria — 2 cm",
                serde_json::json!({"kind":"shadow_gap","gap":2,"depth":4}),
            ),
            (
                "cove",
                "Sanca 18 cm; queda 12 cm; teto 300 cm; rasgo 6 cm",
                serde_json::json!({"kind":"cove","width":18,"drop":12,"ceiling":300,"slot":6}),
            ),
            (
                "shadow_gap",
                "Tabica 2 cm; profundidade 4 cm",
                serde_json::json!({"kind":"shadow_gap","gap":2,"depth":4}),
            ),
            (
                "cove",
                "Sanca comprimento 720 cm",
                serde_json::json!({"kind":"cove","width":18}),
            ),
        ] {
            let mut piece = Furniture {
                id: crate::FurnitureId(1),
                name: name.into(),
                width: 720.0,
                depth: 450.0,
                height: 300.0,
                ..Furniture::default()
            };
            piece.properties.insert("joinery:kind".into(), kind.into());
            piece
                .properties
                .insert("joinery:params".into(), params.to_string());
            let mut home = Home::default();
            home.furniture.push(piece);
            let check = check_annotations(&home);
            assert!(check.names > 0);
            assert!(check.stale.is_empty(), "{name}: {:?}", check.stale);
            assert_eq!(
                check.unverified,
                [] as [(crate::ids::ElementId, std::string::String); 0]
            );
        }
    }

    #[test]
    fn changed_or_unverifiable_ceiling_profiles_are_not_silently_accepted() {
        let mut piece = Furniture {
            id: crate::FurnitureId(1),
            name: "Sanca — 18 cm".into(),
            width: 720.0,
            depth: 18.0,
            height: 300.0,
            ..Furniture::default()
        };
        piece
            .properties
            .insert("joinery:kind".into(), "cove".into());
        piece.properties.insert(
            "joinery:params".into(),
            r#"{"kind":"cove","width":24,"drop":18}"#.into(),
        );
        let mut home = Home::default();
        home.furniture.push(piece);
        let check = check_annotations(&home);
        assert_eq!(
            check.stale.len(),
            1,
            "matching the envelope/drop must not hide a changed width"
        );
        assert!((check.stale[0].measured - 24.0).abs() < 1e-9);
        for invalid in [
            "{",
            r#"{"kind":"cove"}"#,
            r#"{"kind":"shadow_gap","width":18}"#,
        ] {
            home.furniture[0]
                .properties
                .insert("joinery:params".into(), invalid.into());
            let check = check_annotations(&home);
            assert_eq!(check.names, 0);
            assert_eq!(check.unverified.len(), 1);
            assert_eq!(check.stale, [] as [crate::measure::Stale; 0]);
        }
        home.furniture[0].name = "Sanca — 18 × 12 cm".into();
        assert_eq!(check_annotations(&home).unverified.len(), 1);
    }

    #[test]
    fn a_name_keeps_its_decimal_commas() {
        let piece = |name: &str, w: f64, d: f64, h: f64| Furniture {
            id: crate::ids::FurnitureId(1),
            name: name.to_owned(),
            width: w,
            depth: d,
            height: h,
            ..Furniture::default()
        };
        for right in [
            piece(
                "48 — Gabinete do tanque integrado — 80,5 cm, duas portas",
                80.5,
                56.8,
                85.0,
            ),
            piece(
                "Aéreo geladeira — 79,9 cm; ventilação inferior preservada",
                79.9,
                35.0,
                40.0,
            ),
            piece("Micro-ondas embutido 53,9 × 43 × 30,7", 53.9, 43.0, 30.7),
        ] {
            let mut home = Home::default();
            home.furniture = vec![right.clone()];
            let check = check_annotations(&home);
            assert_eq!(check.stale, Vec::new(), "{}", right.name);
            assert_eq!(check.names, 1, "{}", right.name);
        }
        // And the one that is really wrong is still caught.
        let mut home = Home::default();
        home.furniture = vec![piece("tampo aberto 110 × 30", 119.0, 30.0, 3.0)];
        let check = check_annotations(&home);
        assert_eq!(check.stale.len(), 1);
        assert!((check.stale[0].drawn - 110.0).abs() < 1e-9);
        assert_eq!(
            clauses("módulo 70,75 cm; limpeza, duas portas"),
            vec!["módulo 70,75 cm", " limpeza", " duas portas"]
        );
    }

    #[test]
    fn written_sizes_reads_notes_the_way_a_joiner_writes_them() {
        assert_eq!(
            written_sizes("TORRE 80 × 65 × 280 | 53A 30 × 65 × 280"),
            vec![vec![80.0, 65.0, 280.0], vec![30.0, 65.0, 280.0]]
        );
        assert_eq!(
            written_sizes("Forno: vão interno 58x50x63 P"),
            vec![vec![58.0, 50.0, 63.0]]
        );
        assert_eq!(written_sizes("Bancada 0,80 × 0,65"), vec![vec![80.0, 65.0]]);
        // Meters said: whole numbers too.
        assert_eq!(
            written_sizes("Piscina — 7,40 × 3,00 m"),
            vec![vec![740.0, 300.0]]
        );
        assert_eq!(written_sizes("Deck 6 × 4m"), vec![vec![600.0, 400.0]]);
        assert_eq!(
            written_sizes("Painel 210 × 270 cm"),
            vec![vec![210.0, 270.0]]
        );
        assert_eq!(written_sizes("Tampo 600 × 600 mm"), vec![vec![60.0, 60.0]]);
        // A unit written wins over the look of the numbers.
        assert_eq!(
            written_sizes("Recorte 0,80 × 0,65 cm"),
            vec![vec![0.8, 0.65]]
        );
        let chapa = written_sizes("Chapa 8,5 × 6,5 mm");
        assert!((chapa[0][0] - 0.85).abs() < 1e-9 && (chapa[0][1] - 0.65).abs() < 1e-9);
        // Numbers that are not sizes are left where they are.
        assert_eq!(
            written_sizes("Cozinha: corredor 72 cm"),
            [] as [std::vec::Vec<f64>; 0]
        );
        assert_eq!(
            written_sizes("53A e 2 portas"),
            [] as [std::vec::Vec<f64>; 0]
        );
    }
}
