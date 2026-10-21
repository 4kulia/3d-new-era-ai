//! Wall joins: the floor outline of each wall, shaped by its neighbors.
//!
//! Walls meet at *nodes*. A node is where ends coincide, but also where ends
//! come close enough to have meant to (a corner left open), where a wall dies
//! against the body of another (a T) and where two walls cross (an X). The
//! crossed wall is cut at the node, so a junction in the middle of a wall
//! becomes the same problem as one at its ends.
//!
//! At a node the ends are sorted by angle, and each end's side edge is
//! extended until it meets the facing side edge of the next end around the
//! node. That single rule covers L corners (miter), straight continuations, T
//! and X junctions and free ends (square cap). What is left is the middle of
//! the junction: a wall running through the node covers it, and the ends
//! around it stop at its faces; where no wall runs through, every end reaches
//! the node point and they share the middle out between them.
//!
//! The result: walls never lie over each other and never leave a gap, even
//! when the coordinates are a few centimetres out — which is how they arrive
//! when they are written from a description rather than drawn with magnetism.
//! [`weld_ends`] moves such ends onto what they touch, so the model agrees
//! with the drawing.

use crate::elements::Wall;
use crate::geometry::Point2;
use crate::ids::WallId;

/// Endpoints closer than this (cm) are treated as connected.
pub const JOIN_TOLERANCE: f64 = 0.5;

/// Miters longer than this many half-thicknesses are cut square, so very
/// sharp angles don't produce long spikes.
const MITER_LIMIT: f64 = 6.0;

/// Sine of the smallest angle between two walls that still counts as a T or X
/// junction. Below it they are running alongside each other, and the join
/// would slide far away from where they touch.
const MIN_JOIN_SIN: f64 = 0.2;

/// An end that stops within this many cm of another wall's face — short of it
/// or inside it — was meant to touch that wall. Wider gaps are left alone:
/// they are passages, not sloppy drawing.
pub const TOUCH_TOLERANCE: f64 = 5.0;

#[derive(Debug, Clone, Copy)]
struct End {
    wall: usize,
    /// Segment of the wall between its cuts, 0 for an uncut wall.
    piece: usize,
    /// True for the piece's start point, false for its end point.
    at_start: bool,
    /// Unit direction pointing away from the node, along the wall.
    dir: (f64, f64),
    half: f64,
    angle: f64,
}

/// Where one end of a wall piece stops. `left`/`right` are relative to the
/// direction pointing *away* from the node; `center` is the node point, kept
/// only when this end takes a share of the middle of the junction.
#[derive(Debug, Clone, Copy)]
struct Station {
    left: Point2,
    right: Point2,
    center: Option<Point2>,
}

/// What brings a wall to a node, before the cuts are numbered.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ref {
    /// The start (`true`) or end (`false`) of a whole wall.
    Tip(usize, bool),
    /// A cut across a wall at `t` (0..1) of its centerline; the pieces on both
    /// sides of the cut end at the node.
    Cut(usize, f64),
}

impl Ref {
    fn wall(self) -> usize {
        match self {
            Self::Tip(w, _) | Self::Cut(w, _) => w,
        }
    }
}

#[derive(Debug, Clone)]
struct Node {
    center: Point2,
    refs: Vec<Ref>,
}

/// Floor outline of every wall, in the same order as `walls`.
///
/// Each outline is a simple polygon in plan coordinates (cm). It may include
/// the node point itself, which fills the center of junctions with three or
/// more walls; for two walls that point is collinear and harmless.
pub fn wall_outlines(walls: &[Wall]) -> Vec<Vec<Point2>> {
    let boxes: Vec<[f64; 4]> = walls.iter().map(reach_box).collect();
    let mut nodes = tip_nodes(walls);
    near_tips(walls, &mut nodes);
    tees(walls, &boxes, &mut nodes);
    crossings(walls, &boxes, &mut nodes);
    merge_close_cuts(walls, &mut nodes);

    // Cut positions of each wall, in order along it.
    let mut cuts: Vec<Vec<f64>> = vec![Vec::new(); walls.len()];
    for node in &nodes {
        for r in &node.refs {
            if let Ref::Cut(w, t) = *r {
                cuts[w].push(t);
            }
        }
    }
    for list in &mut cuts {
        list.sort_by(f64::total_cmp);
    }

    let mut corners: Vec<Vec<[Option<Station>; 2]>> = cuts
        .iter()
        .map(|list| vec![[None, None]; list.len() + 1])
        .collect();

    for node in &nodes {
        let mut ends: Vec<End> = node
            .refs
            .iter()
            .flat_map(|r| ends_at(walls, &cuts, *r))
            .collect();
        if ends.is_empty() {
            continue;
        }
        ends.sort_by(|a, b| a.angle.total_cmp(&b.angle));
        // Exactly one wall running through the node already covers the middle
        // of the junction, and the ends around it stop at its faces. A wall
        // runs through either because it is cut here, or because it was drawn
        // as two pieces that carry on into each other. Otherwise every end
        // reaches the node point, which shares the middle out between them:
        // fans that meet without overlapping.
        let carries_on = pairs(&ends);
        let through = node
            .refs
            .iter()
            .filter(|r| matches!(r, Ref::Cut(..)))
            .count()
            + carries_on.iter().flatten().count() / 2;
        let m = ends.len();
        for k in 0..m {
            let this = ends[k];
            // A piece that carries on through the node is squared off against
            // the one it carries on into, not mitered around the junction.
            let (next, prev) = match carries_on[k].filter(|_| through == 1) {
                Some(partner) => (ends[partner], ends[partner]),
                None => (ends[(k + 1) % m], ends[(k + m - 1) % m]),
            };
            let left = side_corner(node.center, this, 1.0, next, -1.0);
            let right = side_corner(node.center, this, -1.0, prev, 1.0);
            let fan = through != 1 && left.distance(right) > 1e-6;
            let slot = usize::from(!this.at_start);
            corners[this.wall][this.piece][slot] = Some(Station {
                left,
                right,
                center: fan.then_some(node.center),
            });
        }
    }

    walls
        .iter()
        .zip(&corners)
        .map(|(wall, pieces)| outline(wall, pieces))
        .collect()
}

/// For each end, the one it carries on into: the other half of a wall drawn
/// in two pieces — same thickness, opposite directions, one line.
fn pairs(ends: &[End]) -> Vec<Option<usize>> {
    let mut found = vec![None; ends.len()];
    for a in 0..ends.len() {
        for b in (a + 1)..ends.len() {
            let (one, other) = (ends[a], ends[b]);
            let straight = one.dir.0 * other.dir.1 - one.dir.1 * other.dir.0;
            let facing = one.dir.0 * other.dir.0 + one.dir.1 * other.dir.1;
            if found[a].is_none()
                && found[b].is_none()
                && one.wall != other.wall
                && (one.half - other.half).abs() < JOIN_TOLERANCE
                && facing < 0.0
                && straight.abs() < MIN_JOIN_SIN
            {
                found[a] = Some(b);
                found[b] = Some(a);
            }
        }
    }
    found
}

/// Walks the pieces of one wall, plus side first and minus side back.
fn outline(wall: &Wall, pieces: &[[Option<Station>; 2]]) -> Vec<Point2> {
    // Wall frame: "+" is the left of start → end. At the start of a piece the
    // outward direction is start → end, so its left corner is on "+"; at the
    // end it is reversed, so its right corner is on "+".
    let straight = pieces.len() == 1;
    let (inner_plus, inner_minus) = if straight {
        inner_offsets(&wall.centerline(), wall.thickness / 2.0)
    } else {
        (Vec::new(), Vec::new()) // only straight walls are ever cut
    };
    let (mut plus, mut minus) = (Vec::new(), Vec::new());
    let (mut head, mut tail) = (None, None);
    for (i, [start, end]) in pieces.iter().enumerate() {
        let (Some(s), Some(e)) = (start, end) else {
            return Vec::new(); // degenerate wall
        };
        match (i, s.center) {
            (0, center) => head = Some(center),
            // A cut in the middle of the wall: both sides pinch at the node.
            (_, Some(center)) => {
                plus.push(center);
                minus.push(center);
            }
            _ => {}
        }
        tail = Some(e.center);
        plus.push(s.left);
        minus.push(s.right);
        plus.extend(inner_plus.iter().copied());
        minus.extend(inner_minus.iter().copied());
        plus.push(e.right);
        minus.push(e.left);
    }
    let (Some(head), Some(tail)) = (head, tail) else {
        return Vec::new();
    };
    let mut outline: Vec<Point2> = head.into_iter().collect();
    outline.extend(plus);
    outline.extend(tail);
    outline.extend(minus.into_iter().rev());
    outline.dedup_by(|a, b| a.distance(*b) < 1e-6);
    if outline.len() > 1 && outline[0].distance(outline[outline.len() - 1]) < 1e-6 {
        outline.pop();
    }
    outline
}

/// Offsets of the interior centerline vertices (arcs only) on both sides.
fn inner_offsets(line: &[Point2], half: f64) -> (Vec<Point2>, Vec<Point2>) {
    let mut plus = Vec::new();
    let mut minus = Vec::new();
    for i in 1..line.len().saturating_sub(1) {
        let (a, b, c) = (line[i - 1], line[i], line[i + 1]);
        let n1 = left_normal(a, b);
        let n2 = left_normal(b, c);
        let (mx, my) = (n1.0 + n2.0, n1.1 + n2.1);
        let len = mx.hypot(my);
        if len < 1e-9 {
            continue;
        }
        let (mx, my) = (mx / len, my / len);
        let scale = half / (mx * n1.0 + my * n1.1).max(0.2);
        plus.push(Point2::new(b.x + mx * scale, b.y + my * scale));
        minus.push(Point2::new(b.x - mx * scale, b.y - my * scale));
    }
    (plus, minus)
}

fn left_normal(a: Point2, b: Point2) -> (f64, f64) {
    let len = a.distance(b).max(1e-12);
    (-(b.y - a.y) / len, (b.x - a.x) / len)
}

/// The ends that one `Ref` brings to its node: one for a wall tip, and the
/// two pieces facing each other for a cut.
fn ends_at(walls: &[Wall], cuts: &[Vec<f64>], r: Ref) -> Vec<End> {
    match r {
        Ref::Tip(wall, at_start) => {
            let piece = if at_start { 0 } else { cuts[wall].len() };
            tip(walls, wall, piece, at_start).into_iter().collect()
        }
        Ref::Cut(wall, t) => {
            // Cuts are sorted, and never two at the same place on one wall.
            let rank = cuts[wall].partition_point(|v| *v < t);
            let w = &walls[wall];
            let len = w.start.distance(w.end);
            if len < 1e-9 {
                return Vec::new();
            }
            let dir = ((w.end.x - w.start.x) / len, (w.end.y - w.start.y) / len);
            let back = (-dir.0, -dir.1);
            vec![
                End {
                    wall,
                    piece: rank,
                    at_start: false,
                    dir: back,
                    half: w.thickness / 2.0,
                    angle: back.1.atan2(back.0),
                },
                End {
                    wall,
                    piece: rank + 1,
                    at_start: true,
                    dir,
                    half: w.thickness / 2.0,
                    angle: dir.1.atan2(dir.0),
                },
            ]
        }
    }
}

fn tip(walls: &[Wall], wall: usize, piece: usize, at_start: bool) -> Option<End> {
    let w = &walls[wall];
    let line = w.centerline();
    // Direction away from the node, along the first centerline segment.
    let (from, to) = if at_start {
        (line[0], line[1])
    } else {
        (line[line.len() - 1], line[line.len() - 2])
    };
    let len = from.distance(to);
    if len < 1e-9 {
        return None;
    }
    let dir = ((to.x - from.x) / len, (to.y - from.y) / len);
    Some(End {
        wall,
        piece,
        at_start,
        dir,
        half: w.thickness / 2.0,
        angle: dir.1.atan2(dir.0),
    })
}

/// Groups wall endpoints that coincide within [`JOIN_TOLERANCE`].
fn tip_nodes(walls: &[Wall]) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    for (i, wall) in walls.iter().enumerate() {
        for (at_start, p) in [(true, wall.start), (false, wall.end)] {
            match nodes
                .iter_mut()
                .find(|n| n.center.distance(p) <= JOIN_TOLERANCE)
            {
                Some(node) => node.refs.push(Ref::Tip(i, at_start)),
                None => nodes.push(Node {
                    center: p,
                    refs: vec![Ref::Tip(i, at_start)],
                }),
            }
        }
    }
    nodes
}

/// Straight wall as a line: origin, unit direction, length.
fn line_of(wall: &Wall) -> Option<(Point2, (f64, f64), f64)> {
    if wall.is_arc() {
        return None;
    }
    let len = wall.start.distance(wall.end);
    (len >= 1e-9).then(|| {
        let dir = (
            (wall.end.x - wall.start.x) / len,
            (wall.end.y - wall.start.y) / len,
        );
        (wall.start, dir, len)
    })
}

/// Two junctions on one wall closer together than it is thick leave a sliver
/// of a piece between them, whose two mitered ends cross each other. They
/// become a single junction.
fn merge_close_cuts(walls: &[Wall], nodes: &mut Vec<Node>) {
    let cut_of = |node: &Node, wall: usize| {
        node.refs
            .iter()
            .any(|r| matches!(r, Ref::Cut(w, _) if *w == wall))
    };
    let mut i = 0;
    while i < nodes.len() {
        let mut j = i + 1;
        while j < nodes.len() {
            let shared = nodes[i]
                .refs
                .iter()
                .filter_map(|r| match r {
                    Ref::Cut(w, _) => Some(*w),
                    Ref::Tip(..) => None,
                })
                .find(|w| cut_of(&nodes[j], *w));
            let sliver = shared.is_some_and(|w| {
                nodes[i].center.distance(nodes[j].center) <= walls[w].thickness / 2.0
            });
            if !sliver {
                j += 1;
                continue;
            }
            let (keep, drop) = if nodes[i].refs.len() >= nodes[j].refs.len() {
                (i, j)
            } else {
                (j, i)
            };
            let center = nodes[keep].center;
            let other = nodes.remove(drop);
            let keep = if drop < keep { keep - 1 } else { keep };
            nodes[keep].center = center;
            for r in other.refs {
                if !nodes[keep].refs.iter().any(|have| have.wall() == r.wall()) {
                    nodes[keep].refs.push(r);
                }
            }
            if drop == i {
                break; // node i is gone; carry on with whatever took its place
            }
        }
        i += 1;
    }
}

/// Box around a wall's body, grown by how far an end may reach into it: a
/// cheap first test before the line arithmetic of a junction.
fn reach_box(wall: &Wall) -> [f64; 4] {
    let pad = wall.thickness / 2.0 + TOUCH_TOLERANCE;
    [
        wall.start.x.min(wall.end.x) - pad,
        wall.start.y.min(wall.end.y) - pad,
        wall.start.x.max(wall.end.x) + pad,
        wall.start.y.max(wall.end.y) + pad,
    ]
}

fn holds(b: &[f64; 4], p: Point2) -> bool {
    p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3]
}

fn overlap(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Distance along and across a wall's line, from its start.
fn project(a: Point2, dir: (f64, f64), p: Point2) -> (f64, f64) {
    let (dx, dy) = (p.x - a.x, p.y - a.y);
    (dx * dir.0 + dy * dir.1, -dx * dir.1 + dy * dir.0)
}

/// Where two lines meet, as the distance from `p` along `u`.
fn meet(p: Point2, u: (f64, f64), a: Point2, e: (f64, f64)) -> Option<f64> {
    let denom = u.0 * e.1 - u.1 * e.0;
    if denom.abs() < MIN_JOIN_SIN {
        return None;
    }
    let (_, across) = project(a, e, p);
    Some(across / denom)
}

/// The wall whose body an end at `p` dies against, with `t` of the landing
/// point along its centerline and the point itself. `along` is the direction
/// of the end's own wall, when there is a single one: the end then slides
/// along its own line, so the wall only gets longer or shorter instead of
/// leaning. Walls in `skip` — the ones already meeting at that point — are not
/// candidates.
fn host_at(
    walls: &[Wall],
    boxes: &[[f64; 4]],
    p: Point2,
    skip: &[usize],
    along: Option<(f64, f64)>,
) -> Option<(usize, f64, Point2)> {
    let mut best: Option<(f64, usize, f64, Point2)> = None;
    for (h, host) in walls.iter().enumerate() {
        if skip.contains(&h) || !holds(&boxes[h], p) {
            continue;
        }
        let Some((a, e, len)) = line_of(host) else {
            continue;
        };
        let (_, across) = project(a, e, p);
        if across.abs() > host.thickness / 2.0 + TOUCH_TOLERANCE {
            continue; // the end does not reach this wall
        }
        let at = if let Some(u) = along {
            let Some(s) = meet(p, u, a, e) else { continue };
            Point2::new(p.x + u.0 * s, p.y + u.1 * s)
        } else {
            let (on, _) = project(a, e, p);
            Point2::new(a.x + e.0 * on, a.y + e.1 * on)
        };
        let moved = at.distance(p);
        if moved > host.thickness + TOUCH_TOLERANCE {
            continue; // too far to be the wall this end dies against
        }
        let (on, _) = project(a, e, at);
        if on <= JOIN_TOLERANCE || on >= len - JOIN_TOLERANCE {
            continue; // a tip of the crossed wall: not its body
        }
        if best.is_none_or(|(d, ..)| moved < d) {
            best = Some((moved, h, on / len, at));
        }
    }
    best.map(|(_, h, t, at)| (h, t, at))
}

/// Corners left open — the two ends a few centimetres apart instead of on
/// each other — become one node. Two ends of their own are carried to where
/// their walls' centerlines cross, so both walls only grow or shrink along
/// themselves; anything else joins at the node that already has the most
/// walls. Walls running alongside each other are never pulled together.
fn near_tips(walls: &[Wall], nodes: &mut Vec<Node>) {
    let mut i = 0;
    while i < nodes.len() {
        // Closest first: in a cluster of loose ends, the two that were meant
        // to meet are the two nearest each other.
        while let Some(j) = (i + 1..nodes.len())
            .filter(|j| corner_of(walls, &nodes[i], &nodes[*j]).is_some())
            .min_by(|a, b| {
                let d = |k: &usize| nodes[i].center.distance(nodes[*k].center);
                d(a).total_cmp(&d(b))
            })
        {
            let Some(center) = corner_of(walls, &nodes[i], &nodes[j]) else {
                break;
            };
            let other = nodes.remove(j);
            nodes[i].center = center;
            nodes[i].refs.extend(other.refs);
        }
        i += 1;
    }
}

/// Where two nodes close to each other should meet, if they should at all.
fn corner_of(walls: &[Wall], a: &Node, b: &Node) -> Option<Point2> {
    // Inside a wall's own body there is no room for two ends to be separate
    // things, so the thinner wall of the two sets how far apart they may be.
    let thinnest = a
        .refs
        .iter()
        .chain(&b.refs)
        .map(|r| walls[r.wall()].thickness)
        .fold(f64::MAX, f64::min);
    if a.center.distance(b.center) > TOUCH_TOLERANCE + thinnest / 2.0 {
        return None;
    }
    let pairs = a
        .refs
        .iter()
        .flat_map(|ra| b.refs.iter().map(move |rb| (ra.wall(), rb.wall())));
    for (wa, wb) in pairs {
        if wa == wb || alongside(&walls[wa], &walls[wb]) {
            return None;
        }
    }
    // One end each: extend both along themselves to where their lines cross.
    if let ([Ref::Tip(wa, from_start_a)], [Ref::Tip(wb, from_start_b)]) =
        (a.refs.as_slice(), b.refs.as_slice())
        && let (Some(ea), Some(eb)) = (
            tip(walls, *wa, 0, *from_start_a),
            tip(walls, *wb, 0, *from_start_b),
        )
        && let Some(s) = meet(a.center, ea.dir, b.center, eb.dir)
    {
        let at = Point2::new(a.center.x + ea.dir.0 * s, a.center.y + ea.dir.1 * s);
        let reach = TOUCH_TOLERANCE + walls[*wa].thickness.max(walls[*wb].thickness);
        if at.distance(a.center) <= reach && at.distance(b.center) <= reach {
            return Some(at);
        }
    }
    Some(if a.refs.len() >= b.refs.len() {
        a.center
    } else {
        b.center
    })
}

/// Ends that die inside the body of another wall become real T junctions: the
/// node moves onto that wall's centerline and cuts it there, so both walls
/// share the junction instead of overlapping.
fn tees(walls: &[Wall], boxes: &[[f64; 4]], nodes: &mut Vec<Node>) {
    let mut k = 0;
    while k < nodes.len() {
        let along = match nodes[k].refs.as_slice() {
            [Ref::Tip(w, at_start)] => tip(walls, *w, 0, *at_start).map(|e| e.dir),
            _ => None,
        };
        let members: Vec<usize> = nodes[k].refs.iter().map(|r| r.wall()).collect();
        let Some((host, t, at)) = host_at(walls, boxes, nodes[k].center, &members, along) else {
            k += 1;
            continue;
        };
        // Cutting right beside a junction the wall already has would leave a
        // sliver of a piece, whose two mitered ends would cross each other.
        // Such an end joins that junction instead.
        let reach = walls[host].thickness / 2.0 + JOIN_TOLERANCE;
        let beside = nodes.iter().position(|n| {
            n.center.distance(at) <= reach
                && n.refs.iter().any(|r| r.wall() == host)
                // A nearby junction can sit beside the incoming wall's axis
                // (e.g. the end of a wide structural pier). Sharing that node
                // would skew its entire long face instead of just joining it.
                && nodes[k].refs.iter().all(|r| {
                    line_of(&walls[r.wall()]).is_none_or(|(origin, dir, _)| {
                        project(origin, dir, n.center).1.abs() <= 1e-7
                    })
                })
        });
        match beside {
            Some(m) if m != k => {
                let joined = nodes.remove(k);
                let m = if m > k { m - 1 } else { m };
                nodes[m].refs.extend(joined.refs);
            }
            _ => {
                nodes[k].center = at;
                nodes[k].refs.push(Ref::Cut(host, t));
                k += 1;
            }
        }
    }
}

/// Two walls running alongside each other, one along the body of the other —
/// the two sides of a shaft, a wall doubled by mistake. However close their
/// ends are, they are not a corner and must never be pulled together.
fn alongside(wall: &Wall, other: &Wall) -> bool {
    let (Some((a, u, len)), Some((b, e, other_len))) = (line_of(wall), line_of(other)) else {
        return false; // an arc bends away from any line
    };
    if (u.0 * e.1 - u.1 * e.0).abs() >= MIN_JOIN_SIN {
        return false; // they meet at an angle
    }
    let (_, across) = project(b, e, a);
    if across.abs() >= wall.thickness.midpoint(other.thickness) {
        return false; // too far apart to be one over the other
    }
    // Do they run along each other, or does one carry on where the other ends?
    let (from, _) = project(b, e, wall.start);
    let (to, _) = project(b, e, wall.end);
    let shared = from.max(to).min(other_len) - from.min(to).max(0.0);
    shared > JOIN_TOLERANCE && len > 0.0
}

/// Where wall ends should sit so that they really join what they nearly touch.
///
/// Drawing by eye — or writing coordinates from a description — leaves ends a
/// few centimetres short of a wall, past it, or beside a corner instead of on
/// it. [`wall_outlines`] draws those as proper junctions anyway, and this
/// moves the points themselves so the rest of the model (lengths, runs,
/// openings, room detection) agrees with the drawing. Only the ends of
/// `moving` walls are pulled, onto whatever is within [`TOUCH_TOLERANCE`];
/// everything else stays exactly where it is.
///
/// Returns the ends to move as `(wall, at_start, to)`.
pub fn weld_ends(walls: &[Wall], moving: &[WallId]) -> Vec<(WallId, bool, Point2)> {
    let mut moves = Vec::new();
    let boxes: Vec<[f64; 4]> = walls.iter().map(reach_box).collect();
    for (i, wall) in walls
        .iter()
        .enumerate()
        .filter(|(_, w)| moving.contains(&w.id))
    {
        for at_start in [true, false] {
            let p = if at_start { wall.start } else { wall.end };
            // A corner: the end of another wall right there. Walls running
            // side by side are not corners, however close their ends are.
            let corner = walls
                .iter()
                .enumerate()
                .filter(|(j, other)| *j != i && !alongside(wall, other))
                .flat_map(|(_, w)| [w.start, w.end])
                .filter(|q| q.distance(p) <= TOUCH_TOLERANCE)
                .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)));
            let to = corner.or_else(|| {
                let along = tip(walls, i, 0, at_start).map(|e| e.dir);
                host_at(walls, &boxes, p, &[i], along).map(|(_, _, at)| at)
            });
            if let Some(to) = to
                && to.distance(p) > 1e-9
            {
                moves.push((wall.id, at_start, to));
            }
        }
    }
    moves
}

/// Walls whose bodies cross each other are cut at the crossing, so the two
/// hatched bodies merge into one junction instead of drawing over each other.
fn crossings(walls: &[Wall], boxes: &[[f64; 4]], nodes: &mut Vec<Node>) {
    // How much of each wall is still free, after the tees trimmed its ends.
    let mut span: Vec<(f64, f64)> = walls
        .iter()
        .map(|w| (0.0, w.start.distance(w.end)))
        .collect();
    for node in nodes.iter() {
        for r in &node.refs {
            let Ref::Tip(w, at_start) = *r else { continue };
            let Some((a, e, len)) = line_of(&walls[w]) else {
                continue;
            };
            let (on, _) = project(a, e, node.center);
            if at_start {
                span[w].0 = on.clamp(0.0, len);
            } else {
                span[w].1 = on.clamp(0.0, len);
            }
        }
    }

    let mut found: Vec<(Point2, usize, f64, usize, f64)> = Vec::new();
    for i in 0..walls.len() {
        let Some((a, u, len_i)) = line_of(&walls[i]) else {
            continue;
        };
        for j in (i + 1)..walls.len() {
            let Some((b, v, len_j)) = line_of(&walls[j]) else {
                continue;
            };
            if !overlap(&boxes[i], &boxes[j]) {
                continue;
            }
            let Some(s) = meet(a, u, b, v) else { continue };
            let at = Point2::new(a.x + u.0 * s, a.y + u.1 * s);
            let (r, _) = project(b, v, at);
            // Far enough from the ends of both that neither is left with a
            // sliver of a piece.
            let inside = |on: f64, (lo, hi): (f64, f64), wall: &Wall| {
                let margin = wall.thickness / 2.0 + JOIN_TOLERANCE;
                on > lo + margin && on < hi - margin
            };
            if inside(s, span[i], &walls[i]) && inside(r, span[j], &walls[j]) {
                found.push((at, i, s / len_i, j, r / len_j));
            }
        }
    }

    for (at, i, ti, j, tj) in found {
        attach(nodes, at, &[Ref::Cut(i, ti), Ref::Cut(j, tj)]);
    }
}

/// Adds `refs` to the node at `at`, or makes one there. A wall is never cut
/// twice at the same node.
fn attach(nodes: &mut Vec<Node>, at: Point2, refs: &[Ref]) {
    let here = nodes
        .iter()
        .position(|n| n.center.distance(at) <= JOIN_TOLERANCE)
        .unwrap_or_else(|| {
            nodes.push(Node {
                center: at,
                refs: Vec::new(),
            });
            nodes.len() - 1
        });
    for r in refs {
        if !nodes[here].refs.iter().any(|have| have.wall() == r.wall()) {
            nodes[here].refs.push(*r);
        }
    }
}

/// Offset line of `end` on `side` (+1 left, -1 right).
fn offset(center: Point2, end: End, side: f64) -> (Point2, (f64, f64)) {
    let normal = (-end.dir.1 * side, end.dir.0 * side);
    let origin = Point2::new(
        center.x + normal.0 * end.half,
        center.y + normal.1 * end.half,
    );
    (origin, end.dir)
}

/// Where `this`'s side edge meets `other`'s facing edge; square cap when the
/// edges are parallel, the neighbor is the end itself, or the miter is too long.
fn side_corner(center: Point2, this: End, this_side: f64, other: End, other_side: f64) -> Point2 {
    let (p, d) = offset(center, this, this_side);
    if (other.wall, other.piece, other.at_start) == (this.wall, this.piece, this.at_start) {
        return p;
    }
    let (q, e) = offset(center, other, other_side);
    let denom = d.0 * e.1 - d.1 * e.0;
    if denom.abs() < 1e-9 {
        return p;
    }
    let t = ((q.x - p.x) * e.1 - (q.y - p.y) * e.0) / denom;
    let corner = Point2::new(p.x + d.0 * t, p.y + d.1 * t);
    if corner.distance(center) > MITER_LIMIT * this.half.max(other.half) {
        return p;
    }
    corner
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{polygon_area, signed_area};
    use crate::ids::WallId;

    fn wall(id: u64, a: (f64, f64), b: (f64, f64), thickness: f64) -> Wall {
        Wall {
            thickness,
            ..Wall::new(WallId(id), Point2::new(a.0, a.1), Point2::new(b.0, b.1))
        }
    }

    fn has_point(outline: &[Point2], x: f64, y: f64) -> bool {
        outline.iter().any(|p| p.distance(Point2::new(x, y)) < 1e-6)
    }

    #[test]
    fn free_wall_is_a_rectangle() {
        let outlines = wall_outlines(&[wall(1, (0.0, 0.0), (400.0, 0.0), 20.0)]);
        let o = &outlines[0];
        assert!((polygon_area(o) - 400.0 * 20.0).abs() < 1e-6);
        for (x, y) in [(0.0, 10.0), (0.0, -10.0), (400.0, 10.0), (400.0, -10.0)] {
            assert!(has_point(o, x, y), "missing corner ({x}, {y}) in {o:?}");
        }
    }

    #[test]
    fn l_corner_is_mitered_without_overlap() {
        // Outer corner of a 20 cm L at (400, 0) lands on (410, -10).
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (400.0, 0.0), (400.0, 300.0), 20.0),
        ];
        let outlines = wall_outlines(&walls);
        assert!(has_point(&outlines[0], 410.0, -10.0));
        assert!(has_point(&outlines[0], 390.0, 10.0));
        assert!(has_point(&outlines[1], 410.0, -10.0));
        assert!(has_point(&outlines[1], 390.0, 10.0));
        // Together they cover exactly the L footprint (no overlap, no gap).
        let total: f64 = outlines.iter().map(|o| polygon_area(o)).sum();
        // Bar [0,410]x[-10,10] plus stem [390,410]x[10,300].
        let footprint = 410.0 * 20.0 + 20.0 * 290.0;
        assert!((total - footprint).abs() < 1e-6, "{total} vs {footprint}");
    }

    #[test]
    fn closed_square_has_matching_corners_and_consistent_outlines() {
        let s = [(0.0, 0.0), (400.0, 0.0), (400.0, 400.0), (0.0, 400.0)];
        let walls: Vec<Wall> = (0..4_usize)
            .map(|i| wall(i as u64 + 1, s[i], s[(i + 1) % 4], 10.0))
            .collect();
        let outlines = wall_outlines(&walls);
        let total: f64 = outlines.iter().map(|o| polygon_area(o)).sum();
        // Outer 410x410 minus inner 390x390.
        assert!(
            (total - (410.0 * 410.0 - 390.0 * 390.0)).abs() < 1e-6,
            "{total}"
        );
        for o in &outlines {
            assert!(signed_area(o).abs() > 0.0);
        }
    }

    #[test]
    fn t_junction_leaves_no_gap_at_the_center() {
        let walls = [
            wall(1, (-200.0, 0.0), (0.0, 0.0), 20.0),
            wall(2, (0.0, 0.0), (200.0, 0.0), 20.0),
            wall(3, (0.0, 0.0), (0.0, 200.0), 20.0),
        ];
        let total: f64 = wall_outlines(&walls).iter().map(|o| polygon_area(o)).sum();
        // A 400x20 bar plus a 20x190 stem.
        assert!(
            (total - (400.0 * 20.0 + 20.0 * 190.0)).abs() < 1e-6,
            "{total}"
        );
    }

    #[test]
    fn arc_wall_outline_has_the_area_of_a_thick_arc() {
        let mut w = wall(1, (0.0, 0.0), (400.0, 0.0), 20.0);
        w.arc_extent = Some(90.0);
        let outline = &wall_outlines(std::slice::from_ref(&w))[0];
        let expected = w.length() * 20.0;
        let area = polygon_area(outline);
        assert!(
            (area - expected).abs() / expected < 0.01,
            "{area} vs {expected}"
        );
    }

    #[test]
    fn straight_continuation_stays_flat() {
        let walls = [
            wall(1, (0.0, 0.0), (200.0, 0.0), 20.0),
            wall(2, (200.0, 0.0), (500.0, 0.0), 20.0),
        ];
        let outlines = wall_outlines(&walls);
        assert!(has_point(&outlines[0], 200.0, 10.0) && has_point(&outlines[0], 200.0, -10.0));
        assert!((polygon_area(&outlines[1]) - 300.0 * 20.0).abs() < 1e-6);
    }

    /// Total area of the outlines: with the joins right it is exactly the
    /// footprint, with no overlap between walls and no gap at the junction.
    fn covered(walls: &[Wall]) -> f64 {
        wall_outlines(walls).iter().map(|o| polygon_area(o)).sum()
    }

    #[test]
    fn stepped_bathroom_mass_does_not_tilt_the_long_wall_faces() {
        let walls = [
            wall(1, (601.882, 420.603), (892.107, 420.603), 10.716),
            wall(2, (601.882, 433.5515), (983.193, 433.5515), 15.181),
            wall(3, (888.535, 463.9135), (983.193, 463.9135), 45.543),
            wall(4, (976.4955, 486.685), (976.4955, 654.569), 13.395),
            wall(5, (601.882, 659.4805), (983.193, 659.4805), 9.823),
        ];
        let outlines = wall_outlines(&walls);
        for edge in outlines[3].iter().zip(outlines[3].iter().cycle().skip(1)) {
            if edge.0.distance(*edge.1) > 40.0 {
                assert!(
                    (edge.0.x - edge.1.x).abs() < 1e-7,
                    "vertical bathroom wall acquired a sloping face: {:?}",
                    outlines[3]
                );
            }
        }
    }

    #[test]
    fn a_wall_dying_in_the_middle_of_another_makes_a_t() {
        // Stub drawn onto the centerline of the long wall: it must stop at the
        // face (y = 10), not run 10 cm into the hatch.
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (200.0, 0.0), (200.0, 300.0), 10.0),
        ];
        let outlines = wall_outlines(&walls);
        assert!(has_point(&outlines[1], 195.0, 10.0), "{:?}", outlines[1]);
        assert!(has_point(&outlines[1], 205.0, 10.0));
        // The crossed wall keeps its straight faces.
        for (x, y) in [(0.0, 10.0), (400.0, 10.0), (0.0, -10.0), (400.0, -10.0)] {
            assert!(has_point(&outlines[0], x, y), "{:?}", outlines[0]);
        }
        assert!((covered(&walls) - (400.0 * 20.0 + 10.0 * 290.0)).abs() < 1e-6);
    }

    #[test]
    fn stubs_that_stop_short_or_run_through_still_make_the_same_t() {
        let footprint = 400.0 * 20.0 + 10.0 * 290.0;
        for end in [14.0, 12.0, 8.0, 0.0, -6.0, -9.0] {
            let walls = [
                wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
                wall(2, (200.0, end), (200.0, 300.0), 10.0),
            ];
            let outlines = wall_outlines(&walls);
            assert!(has_point(&outlines[1], 195.0, 10.0), "end {end}");
            let area: f64 = covered(&walls);
            assert!((area - footprint).abs() < 1e-6, "end {end}: {area}");
        }
    }

    #[test]
    fn an_oblique_stub_keeps_its_own_direction() {
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (200.0, 4.0), (400.0, 300.0), 10.0),
        ];
        let outline = &wall_outlines(&walls)[1];
        // Both end corners sit on the face of the crossed wall.
        let on_face = outline.iter().filter(|p| (p.y - 10.0).abs() < 1e-6).count();
        assert!(on_face >= 2, "{outline:?}");
        // The stub stays parallel to itself: its two long sides are 10 apart.
        let width = polygon_area(outline) / 200.0f64.hypot(296.0 - 4.0);
        assert!((width - 10.0).abs() < 0.2, "{width}");
    }

    #[test]
    fn two_walls_crossing_share_the_junction() {
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (200.0, -150.0), (200.0, 150.0), 10.0),
        ];
        // Bar plus the two arms of the crossing wall, which stop at the faces.
        let footprint = 400.0 * 20.0 + 10.0 * (300.0 - 20.0);
        let area = covered(&walls);
        assert!((area - footprint).abs() < 1e-6, "{area}");
    }

    #[test]
    fn several_stubs_on_one_wall_cut_it_in_order() {
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (100.0, 0.0), (100.0, 200.0), 10.0),
            wall(3, (300.0, 0.0), (300.0, 200.0), 10.0),
            wall(4, (200.0, 0.0), (200.0, -200.0), 10.0),
        ];
        let footprint = 400.0 * 20.0 + 3.0 * 10.0 * 190.0;
        let area = covered(&walls);
        assert!((area - footprint).abs() < 1e-6, "{area}");
    }

    #[test]
    fn a_room_with_a_partition_has_no_overlap() {
        let s = [(0.0, 0.0), (600.0, 0.0), (600.0, 400.0), (0.0, 400.0)];
        let mut walls: Vec<Wall> = (0..4_usize)
            .map(|i| wall(i as u64 + 1, s[i], s[(i + 1) % 4], 20.0))
            .collect();
        // Partition drawn from centerline to centerline, as an AI would write it.
        walls.push(wall(5, (300.0, 0.0), (300.0, 400.0), 10.0));
        let ring = 620.0 * 420.0 - 580.0 * 380.0;
        let area = covered(&walls);
        assert!((area - (ring + 10.0 * 380.0)).abs() < 1e-6, "{area}");
    }

    #[test]
    fn a_gap_wide_enough_to_walk_through_is_not_closed() {
        // 40 cm short of the wall: a passage the drawing means to keep.
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (200.0, 50.0), (200.0, 300.0), 10.0),
        ];
        let outlines = wall_outlines(&walls);
        assert!((polygon_area(&outlines[1]) - 250.0 * 10.0).abs() < 1e-6);
    }

    #[test]
    fn walls_running_alongside_each_other_are_left_alone() {
        // 2 cm apart and parallel: not a junction, no cut, no move.
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 10.0),
            wall(2, (0.0, 2.0), (400.0, 2.0), 10.0),
        ];
        let outlines = wall_outlines(&walls);
        assert!((polygon_area(&outlines[0]) - 400.0 * 10.0).abs() < 1e-6);
        assert!((polygon_area(&outlines[1]) - 400.0 * 10.0).abs() < 1e-6);
    }

    #[test]
    fn an_end_near_the_corner_of_another_wall_is_not_torn_off() {
        // The stub reaches the very end of the long wall: that is the corner
        // node, not a cut through its body.
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (400.0, 0.0), (400.0, 300.0), 20.0),
        ];
        let outlines = wall_outlines(&walls);
        assert!(has_point(&outlines[0], 410.0, -10.0));
        assert!(has_point(&outlines[1], 410.0, -10.0));
    }

    #[test]
    fn welding_pulls_ends_onto_the_walls_they_nearly_touch() {
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (200.0, 13.0), (200.0, 300.0), 10.0), // 3 cm short of the face
            wall(3, (398.0, 2.0), (398.0, 300.0), 10.0),  // beside the corner
            wall(4, (100.0, 60.0), (100.0, 300.0), 10.0), // on its own
        ];
        let ids: Vec<crate::ids::WallId> = walls[1..].iter().map(|w| w.id).collect();
        let moves = weld_ends(&walls, &ids);
        assert_eq!(
            moves,
            vec![
                (WallId(2), true, Point2::new(200.0, 0.0)),
                (WallId(3), true, Point2::new(400.0, 0.0)),
            ],
            "{moves:?}"
        );
    }

    #[test]
    fn welding_leaves_walls_running_side_by_side_apart() {
        // A 4 cm shaft between two parallel walls: their ends are close, but
        // pulling them together would fold one onto the other.
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 10.0),
            wall(2, (0.0, 4.0), (400.0, 4.0), 10.0),
        ];
        assert_eq!(
            weld_ends(&walls, &[WallId(2)]),
            [] as [(crate::ids::WallId, bool, crate::geometry::Point2); 0]
        );
    }

    #[test]
    fn welding_closes_a_gap_between_walls_on_the_same_line() {
        let walls = [
            wall(1, (0.0, 0.0), (200.0, 0.0), 10.0),
            wall(2, (203.0, 0.0), (400.0, 0.0), 10.0),
        ];
        assert_eq!(
            weld_ends(&walls, &[WallId(2)]),
            vec![(WallId(2), true, Point2::new(200.0, 0.0))]
        );
    }

    #[test]
    fn welding_only_moves_the_walls_it_is_given() {
        let walls = [
            wall(1, (0.0, 0.0), (400.0, 0.0), 20.0),
            wall(2, (200.0, 4.0), (200.0, 300.0), 10.0),
        ];
        assert_eq!(
            weld_ends(&walls, &[WallId(1)]),
            [] as [(crate::ids::WallId, bool, crate::geometry::Point2); 0]
        );
        assert_eq!(weld_ends(&walls, &[WallId(2)]).len(), 1);
    }
    /// Every plan redraw asks for the outlines, so the junctions have to be
    /// cheap. Release build: ~7 ms for a grid no home ever has.
    #[test]
    #[ignore = "timing, and only meaningful in a release build"]
    fn a_grid_of_walls_stays_quick() {
        // 840 walls, 400 of them crossing each other.
        let mut walls = Vec::new();
        let mut id = 0;
        for i in 0..21 {
            let v = f64::from(i) * 300.0;
            for j in 0..20 {
                let a = f64::from(j) * 300.0;
                id += 1;
                walls.push(wall(id, (a, v), (a + 300.0, v), 15.0));
                id += 1;
                walls.push(wall(id, (v, a), (v, a + 300.0), 15.0));
            }
        }
        let start = std::time::Instant::now();
        let outlines = wall_outlines(&walls);
        let took = start.elapsed();
        println!("{} walls in {took:?}", walls.len());
        assert_eq!(outlines.len(), walls.len());
        assert!(took.as_millis() < 500, "{took:?}");
    }

    /// Area covered by all the outlines together, counting shared ground once.
    fn union_area(outlines: &[Vec<Point2>]) -> f64 {
        use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};
        let mut all = MultiPolygon::new(Vec::new());
        for outline in outlines.iter().filter(|o| o.len() >= 3) {
            let mut coords: Vec<Coord<f64>> =
                outline.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
            coords.push(coords[0]);
            let poly = Polygon::new(LineString::new(coords), vec![]);
            all = all.union(&MultiPolygon::new(vec![poly]));
        }
        geo::Area::unsigned_area(&all)
    }

    /// Deterministic 0..1 stream, so a failure can be looked at again.
    fn rolls() -> impl FnMut() -> f64 {
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            #[allow(clippy::cast_precision_loss)]
            {
                (seed >> 11) as f64 / (1_u64 << 53) as f64
            }
        }
    }

    /// Whether `wall` runs along the body of one already drawn — two walls on
    /// top of each other, which no junction can pull apart.
    fn doubles_up(wall: &Wall, drawn: &[Wall]) -> bool {
        drawn.iter().any(|other| alongside(wall, other))
    }

    /// One drawing of `count` walls, from the roll of `next`: ends on a coarse
    /// grid so they land on corners and bodies, each a few centimetres off the
    /// way written coordinates are. `angled` throws 45° walls in as well.
    fn random_walls(next: &mut impl FnMut() -> f64, count: u64, angled: bool) -> Vec<Wall> {
        let mut walls: Vec<Wall> = Vec::new();
        for i in 0..count {
            let at = |v: f64| (v * 6.0).floor() * 100.0;
            let (x, y) = (at(next()), at(next()));
            let slip = (next() * 8.0).round() - 4.0;
            let length = at(next()) - 200.0;
            let thickness = 10.0 + (next() * 20.0).round();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let shape = (next() * if angled { 3.0 } else { 2.0 }) as u8;
            let (a, b) = match shape {
                0 => ((x, y + slip), (x + length, y + slip)),
                1 => ((x + slip, y), (x + slip, y + length)),
                _ => ((x + slip, y), (x + slip + length, y + length)),
            };
            if (a.0 - b.0).abs() + (a.1 - b.1).abs() < 50.0 {
                continue;
            }
            let candidate = wall(i, a, b, thickness);
            if !doubles_up(&candidate, &walls) {
                walls.push(candidate);
            }
        }
        walls
    }

    /// The pairs of walls whose outlines share ground, for a failing case.
    fn overlapping_pairs(walls: &[Wall]) {
        use geo::{BooleanOps, Coord, LineString, Polygon};
        let outlines = wall_outlines(walls);
        let poly = |o: &Vec<Point2>| {
            let mut c: Vec<Coord<f64>> = o.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
            c.push(c[0]);
            Polygon::new(LineString::new(c), vec![])
        };
        for a in 0..outlines.len() {
            for b in (a + 1)..outlines.len() {
                if outlines[a].len() < 3 || outlines[b].len() < 3 {
                    continue;
                }
                let shared =
                    geo::Area::unsigned_area(&poly(&outlines[a]).intersection(&poly(&outlines[b])));
                if shared > 0.01 {
                    println!(
                        "  {shared:.2} cm2: {:?}-{:?} t{} over {:?}-{:?} t{}",
                        walls[a].start,
                        walls[a].end,
                        walls[a].thickness,
                        walls[b].start,
                        walls[b].end,
                        walls[b].thickness
                    );
                }
            }
        }
    }

    /// How much wall lies over wall: cm², and the share of what they cover.
    fn overlap(walls: &[Wall]) -> (f64, f64) {
        let outlines = wall_outlines(walls);
        assert_eq!(outlines.len(), walls.len());
        for outline in &outlines {
            assert!(
                outline.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
                "{outline:?}"
            );
        }
        let covered: f64 = outlines.iter().map(|o| polygon_area(o)).sum();
        let over = covered - union_area(&outlines);
        (over, over / covered.max(1.0))
    }

    /// The drawings people actually make — walls along the two axes, ends
    /// landing on corners, on bodies, and a few centimetres off them. There no
    /// wall may lie over another: what they cover together is what they cover
    /// apart, give or take the slivers left where a cluster of loose ends is
    /// pulled to one point — a few square centimetres in a drawing of tens of
    /// square metres of wall, and nothing anyone can see.
    #[test]
    fn walls_along_the_axes_never_lie_over_each_other() {
        let mut next = rolls();
        for round in 0..500 {
            let walls = random_walls(&mut next, 14, false);
            let (over, _) = overlap(&walls);
            if over >= 5.0 {
                overlapping_pairs(&walls);
            }
            assert!(
                over < 5.0,
                "round {round}: {over:.2} cm2 of wall over wall in {:?}",
                walls
                    .iter()
                    .map(|w| (w.start, w.end, w.thickness))
                    .collect::<Vec<_>>()
            );
        }
    }

    /// With 45° walls in the mix, a corner buried inside a thick diagonal
    /// wall can still share a sliver of ground: a fifth of a thousandth of
    /// what the walls cover, in drawings this random. Gross overlap — a whole
    /// stub sitting inside a wall — is what this guards against.
    #[test]
    fn walls_at_an_angle_barely_lie_over_each_other() {
        let mut next = rolls();
        let mut worst: f64 = 0.0;
        for round in 0..500 {
            let walls = random_walls(&mut next, 14, true);
            let (_, share) = overlap(&walls);
            worst = worst.max(share);
            assert!(share < 0.003, "round {round}: {:.4}%", share * 100.0);
        }
        assert!(worst < 0.003, "worst {:.4}%", worst * 100.0);
    }

    /// Junctions have to hold for drawings nobody would plan: ends a hair
    /// apart, walls crossing at every angle, stubs dying inside each other.
    /// Nothing may panic, run off to infinity, or cover much more ground than
    /// the walls themselves take up.
    #[test]
    fn random_drawings_stay_sane() {
        let mut next = rolls();
        for round in 0..200 {
            let mut walls = Vec::new();
            for i in 0..12 {
                let at = |v: f64| (v * 5.0).floor() * 100.0;
                let (x, y) = (at(next()), at(next()));
                let (dx, dy) = (at(next()) - 200.0, at(next()) - 200.0);
                let thickness = 10.0 + (next() * 20.0).round();
                let (a, b) = ((x, y), (x + dx, y + dy));
                if (a.0 - b.0).abs() + (a.1 - b.1).abs() < 1.0 {
                    continue;
                }
                walls.push(wall(i, a, b, thickness));
            }
            let outlines = wall_outlines(&walls);
            assert_eq!(outlines.len(), walls.len());
            let (mut covered, mut slabs) = (0.0, 0.0);
            for (wall, outline) in walls.iter().zip(&outlines) {
                assert!(
                    outline.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
                    "round {round}: {outline:?}"
                );
                covered += polygon_area(outline);
                // The wall itself, plus what a junction may add at each end.
                slabs += wall.length() * wall.thickness
                    + 2.0 * MITER_LIMIT * wall.thickness * wall.thickness;
            }
            assert!(covered <= slabs, "round {round}: {covered} > {slabs}");
        }
    }
}
