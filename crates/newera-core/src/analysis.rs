//! Layout checks that make a plan usable, not just drawable: pieces that
//! overlap, pieces stuck in walls and doors that cannot open.
//!
//! An overlap on its own says almost nothing — a cooktop set into its
//! countertop and a sink eating into the dishwasher next to it look the same
//! from a pair of ids. So every overlap comes classified ([`Overlap`]) and
//! measured ([`Issue::extent`]), and pairs that only "collide" because they
//! live on two storeys drawn at the same elevation are named for what they
//! are instead of being reported as real problems.

use geo::{Area, BooleanOps, BoundingRect, Polygon};

use crate::furniture::Furniture;
use crate::geometry::{Point2, to_polygon};
use crate::home::Home;
use crate::ids::{FurnitureId, LevelId, WallId};

/// What an overlap between two pieces really is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    /// Two pieces fight for the same space: a real defect.
    Collision,
    /// One is built into, resting on or tucked under the other — a cooktop in
    /// its countertop, a chair under a table, a face panel on a drawer front.
    Nesting,
    /// The pieces are on different storeys; they never meet in the building.
    CrossLevel,
    /// A point of a project inside a piece, as designed: the water point in
    /// the basin, the sewer under the toilet, the outlet behind the fridge
    /// or set into a cabinet.
    Served,
}

impl Overlap {
    pub fn name(self) -> &'static str {
        match self {
            Self::Collision => "collision",
            Self::Nesting => "nesting",
            Self::CrossLevel => "cross_level",
            Self::Served => "served",
        }
    }

    /// Only a collision needs fixing.
    pub fn is_defect(self) -> bool {
        self == Self::Collision
    }
}

/// Which storeys a check looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Storeys {
    /// The storey shown in the editor.
    #[default]
    Active,
    /// One named storey.
    One(LevelId),
    /// Every storey that is not a reference layer, with cross-storey pairs
    /// marked [`Overlap::CrossLevel`].
    All,
}

/// Something worth fixing in a layout.
#[derive(Debug, Clone, PartialEq)]
pub enum Issue {
    /// Two pieces occupy the same space, classified and measured.
    Overlap {
        a: FurnitureId,
        b: FurnitureId,
        kind: Overlap,
        /// Size of the shared space, `[x, y, z]` cm.
        extent: [f64; 3],
    },
    /// A cabinet, a fridge or a wardrobe whose front is against a solid:
    /// it cannot be opened, and no other check notices.
    Blocked {
        piece: FurnitureId,
        /// What the front runs into.
        against: crate::ids::ElementId,
        /// Free centimeters in front of it.
        cm: f64,
    },
    /// A piece whose back belongs on a wall with its front on one instead:
    /// a sofa facing the wall, a bed with its foot where the headboard
    /// goes, a toilet with its seat to the tiles. Turned half around from
    /// what was meant — the angle's sums gone the wrong way.
    Backwards {
        piece: FurnitureId,
        /// The wall its front is against.
        wall: WallId,
        /// Free centimeters in front of it.
        cm: f64,
        /// The angle that puts its back on that wall instead.
        angle: f64,
    },
    /// A piece that is not a door or window goes through a wall.
    InWall(FurnitureId, WallId),
    /// A piece blocks a door leaf or the approach to a door/passage.
    BlocksDoor { door: FurnitureId, by: FurnitureId },
    /// A tall or elevated solid masks a window. Extent is width and height
    /// of the obstruction in the window's local frame, in centimeters.
    BlocksWindow {
        window: FurnitureId,
        by: FurnitureId,
        extent: [f64; 2],
    },
    /// A piece is outside every room (only reported when rooms exist).
    OutsideRooms(FurnitureId),
    /// A fixed point with nothing to be fixed to: loose in a room, on glass,
    /// in an opening's span, hanging under the ceiling.
    Loose { piece: FurnitureId, why: String },
    /// An appliance built into joinery that its host no longer holds: the
    /// niche was resized around it and nobody said anything, because a piece
    /// that is built in is excluded from every overlap check by design.
    OutgrewNiche {
        piece: FurnitureId,
        host: FurnitureId,
        /// How far it sticks out, `[x, y, z]` cm.
        over: [f64; 3],
    },
    /// A door or a window that is in no wall: a hole drawn as a panel, or a
    /// leaf left leaning where a passage was meant to be. It reads as an
    /// opening everywhere — schedules, quantities, the 3D — and opens
    /// nothing.
    LooseOpening(FurnitureId),
    /// The fronts built into a group face one way and its `angle` another.
    /// The piece opens where its panels are; whoever set the angle meant the
    /// other side. Every check that measures "in front of" has to pick one,
    /// so this says it out loud instead of choosing in silence.
    Turned {
        piece: FurnitureId,
        /// Where the doors, drawer fronts and kick actually are.
        built: &'static str,
        /// Where `angle` says the piece looks.
        placed: &'static str,
    },
    /// A light fixture with no rated output — neither lumens nor watts, only
    /// the relative power an import carries — so every illuminance computed
    /// from it is a guess, and a plan that looks lit can be dark.
    UnratedLight(FurnitureId),
    /// A luminaire extends above the ceiling surface of its room.
    AboveCeiling {
        piece: FurnitureId,
        room: crate::ids::RoomId,
        ceiling: f64,
        top: f64,
    },
    /// A bedroom or a bathroom with no door: reached only through open
    /// passages, or not reached at all. Only said once the storey has doors
    /// somewhere — a sketch with no openings yet is not a sealed flat.
    NoDoor {
        room: crate::ids::RoomId,
        /// The open passages that are its only way in; empty when there is none.
        passages: Vec<FurnitureId>,
    },
    /// A group whose parts name fronts on more than one face with no clear
    /// winner, so which way it opens is `angle`'s guess. Said instead of
    /// picked in silence: a guess that happens to agree with a wrong angle
    /// would otherwise raise nothing at all.
    UnclearFront {
        piece: FurnitureId,
        /// The faces the parts point at, strongest first.
        candidates: Vec<&'static str>,
        placed: &'static str,
    },
}

impl Issue {
    /// Ids the issue is about, in report order.
    pub fn ids(&self) -> Vec<crate::ids::ElementId> {
        match self {
            Self::Overlap { a, b, .. } => vec![(*a).into(), (*b).into()],
            Self::Blocked { piece, against, .. } => vec![(*piece).into(), *against],
            Self::Backwards { piece, wall, .. } => vec![(*piece).into(), (*wall).into()],
            Self::InWall(f, w) => vec![(*f).into(), (*w).into()],
            Self::BlocksDoor { door, by } => vec![(*door).into(), (*by).into()],
            Self::BlocksWindow { window, by, .. } => vec![(*window).into(), (*by).into()],
            Self::OutsideRooms(f) | Self::LooseOpening(f) | Self::UnratedLight(f) => {
                vec![(*f).into()]
            }
            Self::Loose { piece, .. } => vec![(*piece).into()],
            Self::AboveCeiling { piece, room, .. } => vec![(*piece).into(), (*room).into()],
            Self::OutgrewNiche { piece, host, .. } => vec![(*piece).into(), (*host).into()],
            Self::Turned { piece, .. } | Self::UnclearFront { piece, .. } => {
                vec![(*piece).into()]
            }
            Self::NoDoor { room, passages } => std::iter::once((*room).into())
                .chain(passages.iter().map(|p| (*p).into()))
                .collect(),
        }
    }

    /// Whether this is a real defect rather than a classified non-problem.
    pub fn is_defect(&self) -> bool {
        match self {
            Self::Overlap { kind, .. } => kind.is_defect(),
            _ => true,
        }
    }

    /// The report section it belongs to: `overlap`, `blocked`, `in_wall`…
    pub fn family(&self) -> &'static str {
        match self {
            Self::Overlap { .. } => "overlap",
            Self::Blocked { .. } => "blocked",
            Self::Backwards { .. } => "backwards",
            Self::InWall(..) => "in_wall",
            Self::BlocksDoor { .. } => "blocks_door",
            Self::BlocksWindow { .. } => "blocks_window",
            Self::OutsideRooms(_) => "outside_rooms",
            Self::Loose { .. } => "loose",
            Self::OutgrewNiche { .. } => "outgrew_niche",
            Self::LooseOpening(_) => "loose_opening",
            Self::Turned { .. } => "turned",
            Self::UnclearFront { .. } => "unclear_front",
            Self::NoDoor { .. } => "no_door",
            Self::UnratedLight(_) => "unrated_light",
            Self::AboveCeiling { .. } => "above_ceiling",
        }
    }

    /// The name it is accepted by: its family and its ids, sorted, so the
    /// same pair keeps the same name whichever piece the check met first —
    /// `overlap:f817+f830`.
    pub fn key(&self) -> String {
        if let Self::NoDoor { room, .. } = self {
            return format!("no_door:{room}");
        }
        let mut ids: Vec<String> = self.ids().iter().map(ToString::to_string).collect();
        ids.sort();
        format!("{}:{}", self.family(), ids.join("+"))
    }

    /// The reason it was accepted with, when someone looked at it and
    /// decided the drawing is right — a model whose box is bigger than the
    /// piece it draws, say.
    pub fn accepted<'a>(&self, home: &'a Home) -> Option<&'a str> {
        home.accepted.get(&self.key()).map(String::as_str)
    }

    /// A defect nobody has accepted yet: what a count of things to fix means.
    pub fn is_pending(&self, home: &Home) -> bool {
        self.is_defect() && self.accepted(home).is_none()
    }

    /// Whether an accepted key names a layout finding (and not an
    /// ergonomics one, which shares the project's list of acceptances).
    pub fn is_layout_key(key: &str) -> bool {
        const FAMILIES: [&str; 15] = [
            "backwards",
            "loose",
            "unrated_light",
            "above_ceiling",
            "no_door",
            "unclear_front",
            "overlap",
            "blocked",
            "in_wall",
            "blocks_door",
            "blocks_window",
            "outside_rooms",
            "outgrew_niche",
            "loose_opening",
            "turned",
        ];
        key.split_once(':')
            .is_some_and(|(family, _)| FAMILIES.contains(&family))
    }

    /// Acceptances of layout findings that no longer exist on any storey.
    ///
    /// The reason stays in the project after what it explained is gone, and
    /// if the problem comes back it comes back already silenced, with a
    /// reason about a drawing that no longer exists.
    pub fn orphaned(home: &Home) -> Vec<(String, String)> {
        let live: std::collections::BTreeSet<String> = check_layout_in(home, Storeys::All)
            .iter()
            .map(Issue::key)
            .collect();
        home.accepted
            .iter()
            .filter(|(key, _)| Self::is_layout_key(key) && !live.contains(*key))
            .map(|(key, why)| (key.clone(), why.clone()))
            .collect()
    }

    /// Writes a key the way [`Issue::key`] does, so `f830+f817` and a bare
    /// pair of ids name the same overlap.
    pub fn normalize_key(raw: &str) -> String {
        let raw = raw.trim();
        let (family, ids) = raw.split_once(':').unwrap_or(("overlap", raw));
        let mut ids: Vec<&str> = ids.split('+').map(str::trim).collect();
        ids.sort_unstable();
        format!("{}:{}", family.trim(), ids.join("+"))
    }

    /// What it is, as precisely as a word says it: an overlap answers with
    /// its classification (`collision`, `nesting`, `cross_level`), anything
    /// else with its family.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Overlap { kind, .. } => kind.name(),
            _ => self.family(),
        }
    }
}

/// Area below which a contact is ignored (touching pieces are fine), cm².
const MIN_OVERLAP: f64 = 25.0;
/// How far a piece may press into a wall face and still count as against it, cm.
const WALL_TOLERANCE: f64 = 2.0;
/// Pieces this thin (rugs, mats) never collide.
const FLAT: f64 = 2.0;
/// Less shared height than this, cm, and two pieces only touch.
const TOUCH: f64 = 1.0;

fn polygon(points: &[Point2]) -> Polygon<f64> {
    to_polygon(points)
}

fn heights_overlap(a: &Furniture, b: &Furniture) -> bool {
    let ((a0, a1), (b0, b1)) = (a.height_range(), b.height_range());
    a0 < b1 && b0 < a1
}

/// How much two height ranges share, cm (0 or less when apart or touching).
fn shared_height((a0, a1): (f64, f64), (b0, b1): (f64, f64)) -> f64 {
    a1.min(b1) - a0.max(b0)
}

fn round_mm(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Height of a wall's top above the floor at a plan point.
fn wall_top_at(wall: &crate::elements::Wall, p: Point2) -> f64 {
    let len = wall.start.distance(wall.end).max(1e-9);
    let t = (((p.x - wall.start.x) * (wall.end.x - wall.start.x)
        + (p.y - wall.start.y) * (wall.end.y - wall.start.y))
        / (len * len))
        .clamp(0.0, 1.0);
    wall.height + (wall.height_at_end.unwrap_or(wall.height) - wall.height) * t
}

/// Floor area swept by a hinged door leaf, as a polygon (quarter discs).
pub fn door_swing(door: &Furniture) -> Option<Vec<Point2>> {
    let opening = door.opening.as_ref()?;
    if opening.sliding || opening.kind != crate::furniture::OpeningKind::Door {
        return None;
    }
    let leaves = f64::from(opening.leaves.clamp(1, 2));
    let radius = door.width / leaves;
    let front = door.depth / 2.0;
    let quarter = |hinge_x: f64, toward: f64| -> Vec<(f64, f64)> {
        let mut pts = vec![(hinge_x, front)];
        for i in 0..=8 {
            let a = f64::from(i) / 8.0 * std::f64::consts::FRAC_PI_2;
            pts.push((
                hinge_x + toward * radius * a.cos(),
                front + radius * a.sin(),
            ));
        }
        pts
    };
    let half = door.width / 2.0;
    let local: Vec<(f64, f64)> = if opening.leaves >= 2 {
        let mut pts = quarter(-half, 1.0);
        pts.extend(quarter(half, -1.0).into_iter().rev());
        pts
    } else if opening.hinge_right {
        quarter(half, -1.0)
    } else {
        quarter(-half, 1.0)
    };
    Some(local.into_iter().map(|p| door.to_plan(p)).collect())
}

/// Layout problems on the storey the editor is showing.
///
/// For a plan drawn as stacked layers, or to see every storey at once, use
/// [`check_layout_in`].
pub fn check_layout(home: &Home) -> Vec<Issue> {
    check_layout_in(home, Storeys::Active)
}

/// Layout problems on the storeys `scope` selects.
pub fn check_layout_in(home: &Home, scope: Storeys) -> Vec<Issue> {
    let mut issues = Vec::new();
    // Pieces inside groups are checked one by one; pieces of the same group
    // (roof slopes, a table and its chairs) are meant to touch.
    let mut pieces: Vec<&Furniture> = Vec::new();
    let mut groups: Vec<usize> = Vec::new();
    let mut levels: Vec<Option<LevelId>> = Vec::new();
    let wanted = |level: Option<LevelId>| match scope {
        Storeys::Active => home.on_level(level, home.current_level()),
        Storeys::One(id) => home.on_level(level, Some(id)),
        // Reference layers are drawing, not building: never checked.
        Storeys::All => !home.is_reference_level(level),
    };
    for (g, top) in home.furniture.iter().enumerate() {
        if !wanted(top.level) {
            continue;
        }
        for leaf in top.visible_leaves() {
            // Items embedded in joinery (a cooktop in its countertop) sit in
            // their host's cutout or niche by design.
            if leaf.properties.contains_key("joinery:embedded") {
                continue;
            }
            pieces.push(leaf);
            groups.push(g);
            levels.push(home.resolve_level(top.level));
        }
    }
    // Inspect luminous leaves, never a group's floor-to-ceiling envelope.
    // Use the same storey-relative surface the renderer draws.
    let ceilings: Vec<_> = home
        .rooms
        .iter()
        .filter(|r| wanted(r.level))
        .map(|r| (r, crate::room_ceiling(home, r)))
        .collect();
    for (i, piece) in pieces.iter().enumerate() {
        let luminaire = piece.light.is_some()
            || matches!(
                piece.catalog.as_str(),
                "pendant"
                    | "downlight"
                    | "led-panel"
                    | "led-strip"
                    | "table-lamp"
                    | "floor-lamp"
                    | "wall-lamp"
            );
        if !luminaire {
            continue;
        }
        for (room, surfaces) in &ceilings {
            if !home.on_level(room.level, levels[i]) {
                continue;
            }
            if let Some((ceiling, top)) = surfaces
                .iter()
                .filter_map(|s| s.excess(piece))
                .max_by(|(ca, ta), (cb, tb)| (ta - ca).total_cmp(&(tb - cb)))
            {
                issues.push(Issue::AboveCeiling {
                    piece: piece.id,
                    room: room.id,
                    ceiling,
                    top,
                });
            }
        }
    }

    let footprints: Vec<Polygon<f64>> = pieces
        .iter()
        .map(|f| polygon(&f.projected_footprint()))
        .collect();

    // Heights in the building, for pieces of two storeys: a sofa downstairs
    // and a bed upstairs share a plan, not a space.
    let building = |i: usize| {
        let floor = home.elevation_of(levels[i]);
        let (lo, hi) = pieces[i].height_range();
        (lo + floor, hi + floor)
    };
    for (i, a) in pieces.iter().enumerate() {
        if a.is_opening() || a.height <= FLAT {
            continue;
        }
        for (j, b) in pieces.iter().enumerate().skip(i + 1) {
            let cross_level = levels[i] != levels[j];
            if b.is_opening()
                || b.height <= FLAT
                || groups[i] == groups[j]
                // Two storeys are checked against each other only for their
                // heights in the building, not for their drawn elevations.
                || (!cross_level && !heights_overlap(a, b))
                || (cross_level && shared_height(building(i), building(j)) < TOUCH)
            {
                continue;
            }
            // A wall or panel cut to the roof's profile sits under its slopes.
            let tilted = |f: &Furniture| f.pitch != 0.0 || f.roll != 0.0;
            let fitted = |f: &Furniture| f.properties.contains_key(crate::roof_fit::ROOF_FIT_KEY);
            if (fitted(a) && tilted(b)) || (fitted(b) && tilted(a)) {
                continue;
            }
            let shared = footprints[i].intersection(&footprints[j]);
            let area = shared.unsigned_area();
            if area <= MIN_OVERLAP {
                continue;
            }
            // A contact near an edge may be absent at the overlap centroid.
            let meet =
                cross_level || (!tilted(a) && !tilted(b)) || crate::solid::boxes_overlap(a, b);
            if !meet {
                continue;
            }
            let mut extent = extent_of(&shared, a, b);
            if cross_level {
                extent[2] = round_mm(shared_height(building(i), building(j)));
            }
            let kind = if cross_level {
                Overlap::CrossLevel
            } else if let Some(kind) = point_in_piece(a, b) {
                kind
            } else {
                classify(a, b, area, extent, &footprints[i], &footprints[j])
            };
            issues.push(Issue::Overlap {
                a: a.id,
                b: b.id,
                kind,
                extent,
            });
        }
    }

    let outlines = outlines_by_level(home);
    for (i, piece) in pieces.iter().enumerate() {
        if piece.is_opening() {
            continue;
        }
        for (wall, outline) in home.walls.iter().zip(&outlines) {
            // Only walls of the piece's own storey can have it inside them.
            if outline.len() < 3 || home.resolve_level(wall.level) != levels[i] {
                continue;
            }
            let shared = footprints[i].intersection(&polygon(outline));
            if shared.unsigned_area() <= MIN_OVERLAP {
                continue;
            }
            // Resting against a wall is not being in it: only count pieces that
            // go more than a couple of centimeters into it.
            let depth = shared
                .iter()
                .filter_map(geo::MinimumRotatedRect::minimum_rotated_rect)
                .map(|r| {
                    let c: Vec<_> = r.exterior().coords().copied().collect();
                    let a = (c[1].x - c[0].x).hypot(c[1].y - c[0].y);
                    let b = (c[2].x - c[1].x).hypot(c[2].y - c[1].y);
                    a.min(b)
                })
                .fold(0.0, f64::max);
            if depth <= WALL_TOLERANCE {
                continue;
            }
            if crate::solid::hits_prism(piece, &polygon(outline), 0.0, |at| {
                wall_top_at(wall, at) - 1.0
            }) {
                issues.push(Issue::InWall(piece.id, wall.id));
            }
        }
    }

    // Embedded appliances are allowed to touch their host, but must still
    // respect windows and access to doors outside that host.
    let opening_solids: Vec<_> = home
        .furniture
        .iter()
        .filter(|top| wanted(top.level))
        .flat_map(|top| {
            top.visible_leaves().into_iter().map(move |leaf| {
                (
                    home.resolve_level(top.level),
                    leaf,
                    polygon(&leaf.projected_footprint()),
                )
            })
        })
        .collect();
    for (d, opening) in pieces.iter().enumerate() {
        let Some(spec) = &opening.opening else {
            continue;
        };
        let swing = door_swing(opening).map(|pts| polygon(&pts));
        for (level, piece, footprint) in &opening_solids {
            if *level != levels[d] || piece.is_opening() {
                continue;
            }
            if spec.kind == crate::furniture::OpeningKind::Window {
                if let Some(extent) = window_obstruction(opening, piece) {
                    issues.push(Issue::BlocksWindow {
                        window: opening.id,
                        by: piece.id,
                        extent,
                    });
                }
            } else if passage_hits(opening, piece, footprint)
                || swing
                    .as_ref()
                    .is_some_and(|s| leaf_hits(opening, s, piece, footprint))
            {
                issues.push(Issue::BlocksDoor {
                    door: opening.id,
                    by: piece.id,
                });
            }
        }
    }

    // A cabinet turned the wrong way is a modelling slip that survives every
    // other check: the boxes do not overlap, nothing is in a wall, and the
    // piece is simply impossible to open. `angle` alone does not show it.
    // The solids of a storey are gathered once and measured against many
    // times: rebuilding them per cabinet would walk the whole plan again for
    // each one, and this runs on every read of the issue count.
    let mut solids: Vec<(Option<LevelId>, Vec<crate::measure::Obstacle>)> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        if !opens_at_the_front(piece) {
            continue;
        }
        let Some(front) = crate::measure::Dir::parse("front", Some(piece)) else {
            continue;
        };
        if !solids.iter().any(|(level, _)| *level == levels[i]) {
            let view = home.level_view(levels[i]);
            solids.push((levels[i], crate::measure::obstacles(&view, &|_| false)));
        }
        let here = solids
            .iter()
            .find(|(level, _)| *level == levels[i])
            .map(|(_, list)| list.as_slice())
            .unwrap_or_default();
        let clear = crate::measure::clearance_against(here, piece, front, OPENING_ROOM);
        if clear.cm < OPENING_ROOM
            && let Some(against) = clear.against
        {
            issues.push(Issue::Blocked {
                piece: piece.id,
                against: against.id(),
                cm: (clear.cm * 10.0).round() / 10.0,
            });
        }
    }

    // A sofa, a bed, a toilet turned to face the wall its back belongs on.
    // Nothing overlaps and nothing is in a wall, so no other check sees it;
    // cabinets that open at the front are `Blocked` above instead.
    let mut walls_only: Vec<(Option<LevelId>, Vec<crate::measure::Obstacle>)> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        if crate::front::of(&piece.catalog).stance != crate::front::Stance::Wall
            || opens_at_the_front(piece)
            || piece.is_opening()
        {
            continue;
        }
        let (Some(front), Some(back)) = (
            crate::measure::Dir::parse("front", Some(piece)),
            crate::measure::Dir::parse("back", Some(piece)),
        ) else {
            continue;
        };
        if !walls_only.iter().any(|(level, _)| *level == levels[i]) {
            let view = home.level_view(levels[i]);
            let walls = crate::measure::obstacles(&view, &|_| false)
                .into_iter()
                .filter(|o| matches!(o.what, crate::measure::Solid::Wall(_)))
                .collect();
            walls_only.push((levels[i], walls));
        }
        // Only a wall as tall as the piece is one its back could be meant
        // for: a sofa's seat by an 18 cm curb is not facing a wall.
        let top = piece.height_range().1;
        let here: Vec<crate::measure::Obstacle> = walls_only
            .iter()
            .find(|(level, _)| *level == levels[i])
            .map(|(_, list)| list.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|o| o.z.1 >= top - 1.0)
            .cloned()
            .collect();
        let here = here.as_slice();
        let ahead = crate::measure::clearance_against(here, piece, front, BACKWARDS_REACH);
        let Some(crate::measure::Solid::Wall(wall)) = ahead.against else {
            continue;
        };
        if ahead.cm >= BACKWARDS_REACH {
            continue;
        }
        // Its back has to be clearly freer than its front: between two walls
        // (a niche) or one step from both, the angle is as good as its
        // opposite, and nothing says which was meant.
        let behind =
            crate::measure::clearance_against(here, piece, back, ahead.cm + BACKWARDS_MARGIN);
        if behind.against.is_some() && behind.cm < ahead.cm + BACKWARDS_MARGIN {
            continue;
        }
        issues.push(Issue::Backwards {
            piece: piece.id,
            wall,
            cm: (ahead.cm * 10.0).round() / 10.0,
            angle: (piece.angle + 180.0).rem_euclid(360.0),
        });
    }

    // An appliance whose niche was resized under it. Built-in pieces are
    // left out of the overlap check on purpose — an oven in its niche is
    // meant to touch it — and that is exactly why nothing noticed when the
    // niche stopped fitting the oven.
    for top in home.furniture.iter().filter(|f| wanted(f.level)) {
        let (host_min, host_max) = crate::measure::plan_bounds(top);
        let (host_low, host_high) = top.height_range();
        for built_in in top.flatten().into_iter().skip(1) {
            if !built_in.properties.contains_key("joinery:embedded") {
                continue;
            }
            let (min, max) = crate::measure::plan_bounds(built_in);
            let (low, high) = built_in.height_range();
            // The front of a built-in stands proud of its host by design, so
            // only what sticks out on both sides of an axis is a misfit.
            let over = |lo: f64, hi: f64, host_lo: f64, host_hi: f64| {
                let out = (host_lo - lo).max(0.0) + (hi - host_hi).max(0.0);
                let one_side = (host_lo - lo).max(hi - host_hi).max(0.0);
                if out - one_side > WALL_TOLERANCE {
                    (out * 10.0).round() / 10.0
                } else {
                    0.0
                }
            };
            let sticks = [
                over(min.x, max.x, host_min.x, host_max.x),
                over(min.y, max.y, host_min.y, host_max.y),
                over(low, high, host_low, host_high),
            ];
            if sticks.iter().any(|v| *v > 0.0) {
                issues.push(Issue::OutgrewNiche {
                    piece: built_in.id,
                    host: top.id,
                    over: sticks,
                });
            }
        }
    }

    // A door or window standing in no wall at all.
    for (i, piece) in pieces.iter().enumerate() {
        if !piece.is_opening() {
            continue;
        }
        let in_a_wall = home
            .walls
            .iter()
            .zip(&outlines)
            .filter(|(wall, outline)| {
                outline.len() >= 3 && home.resolve_level(wall.level) == levels[i]
            })
            .any(|(_, outline)| {
                footprints[i]
                    .intersection(&polygon(outline))
                    .unsigned_area()
                    > MIN_OVERLAP
            });
        if !in_a_wall {
            issues.push(Issue::LooseOpening(piece.id));
        }
    }

    // A group whose `angle` and whose fronts point different ways: the score
    // would reward turning it to silence a clearance warning, and the piece
    // would end up opening against a wall. Better to name the contradiction.
    for top in home.furniture.iter().filter(|f| wanted(f.level)) {
        for group in top.flatten().into_iter().filter(|p| p.is_group()) {
            if let Some((built, placed)) = crate::measure::facing_disagrees(group) {
                issues.push(Issue::Turned {
                    piece: group.id,
                    built,
                    placed,
                });
            } else if let Some(candidates) = crate::measure::front_unclear(group) {
                issues.push(Issue::UnclearFront {
                    piece: group.id,
                    candidates,
                    placed: crate::measure::facing(group),
                });
            }
        }
    }

    // Fixtures that light with a guessed output.
    for top in home.furniture.iter().filter(|f| wanted(f.level)) {
        for piece in top.visible_leaves() {
            if let Some(light) = &piece.light
                && light.lumens.is_none()
                && light.watts.is_none()
            {
                issues.push(Issue::UnratedLight(piece.id));
            }
        }
    }

    // Fixed points with nothing to hold them, on the storey shown.
    for (i, piece) in pieces.iter().enumerate() {
        if levels[i] != home.current_level() || crate::mounting::mount_of(&piece.catalog).is_none()
        {
            continue;
        }
        if let Some(why) = crate::mounting::blocked(home, piece) {
            issues.push(Issue::Loose {
                piece: piece.id,
                why: why.to_string(),
            });
        }
    }

    // Bedrooms and bathrooms with no door, storey by storey.
    for (id, passages) in rooms_without_door(home, &wanted) {
        issues.push(Issue::NoDoor { room: id, passages });
    }

    if !home.rooms.is_empty() {
        let rooms: Vec<(Option<LevelId>, Polygon<f64>)> = home
            .rooms
            .iter()
            .map(|r| (home.resolve_level(r.level), polygon(&r.points)))
            .collect();
        // Outside, per group: a facade panel of 42 slats is one piece
        // outside, said once, and a shelf of a cabinet sticking out is that
        // shelf.
        let mut outside: Vec<(usize, FurnitureId, bool)> = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            if piece.is_opening() {
                continue;
            }
            let center = geo::Point::new(piece.position.x, piece.position.y);
            let covered = rooms
                .iter()
                .any(|(level, r)| *level == levels[i] && geo::Contains::contains(r, &center));
            // A storey with no rooms drawn yet has nothing to be outside of.
            let has_rooms = rooms.iter().any(|(level, _)| *level == levels[i]);
            outside.push((groups[i], piece.id, has_rooms && !covered));
        }
        // A part accepted as outside on its own, before groups were said
        // once, keeps its group said part by part: the acceptance still
        // answers to its finding.
        let accepted_part = |group: usize| {
            outside
                .iter()
                .any(|(g, id, _)| *g == group && Issue::OutsideRooms(*id).accepted(home).is_some())
        };
        let mut said: Vec<usize> = Vec::new();
        for &(group, id, out) in &outside {
            if !out {
                continue;
            }
            let parts = outside.iter().filter(|(g, ..)| *g == group);
            if parts.clone().count() > 1
                && parts.clone().all(|(.., out)| *out)
                && !accepted_part(group)
            {
                if !said.contains(&group) {
                    said.push(group);
                    issues.push(Issue::OutsideRooms(home.furniture[group].id));
                }
            } else {
                issues.push(Issue::OutsideRooms(id));
            }
        }
    }
    issues
}

/// A conservative approach zone on both faces also covers sliding doors and
/// passages, which have no swing. It is not a full circulation certification.
fn passage_hits(door: &Furniture, piece: &Furniture, footprint: &Polygon<f64>) -> bool {
    let x = door.width / 2.0;
    let y = door.depth / 2.0 + 60.0;
    let zone = polygon(&[(-x, -y), (x, -y), (x, y), (-x, y)].map(|p| door.to_plan(p)));
    leaf_hits(door, &zone, piece, footprint)
}

/// Nearby solids projected onto the actual opening, not just a plan overlap.
/// Small countertop objects may sit against a window: this check reserves
/// bulk obstruction for tall/elevated pieces, not every kettle or air fryer.
/// Sash operation and appliance ventilation require separate use checks.
fn window_obstruction(window: &Furniture, piece: &Furniture) -> Option<[f64; 2]> {
    // Their bounding boxes are mostly air or transparent glass. Treating
    // them as opaque cupboards would reject ordinary window-side decor —
    // and a guard of bars or glass in front of a window, which is what
    // guards are for.
    if piece.light.is_some()
        || matches!(piece.catalog.as_str(), "plant" | "shower-glass")
        || piece.opacity.is_some_and(|opacity| opacity < 0.5)
        || crate::guard::guard_of(piece).is_some()
    {
        return None;
    }
    let (low, high) = piece.height_range();
    let (sill, head) = window.height_range();
    let height = high.min(head) - low.max(sill);
    if height <= WALL_TOLERANCE || piece.height <= FLAT {
        return None;
    }
    if piece.height <= 50.0
        && piece.width <= 60.0
        && piece.depth <= 60.0
        && low <= 110.0
        && high <= sill + 40.0
    {
        return None;
    }
    let x = window.width / 2.0;
    let y = window.depth / 2.0 + 60.0;
    let zone = polygon(&[(-x, -y), (x, -y), (x, y), (-x, y)].map(|p| window.to_plan(p)));
    let shared = zone.intersection(&polygon(&piece.projected_footprint()));
    if shared.unsigned_area() <= MIN_OVERLAP {
        return None;
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for c in shared.iter().flat_map(|p| p.exterior().coords()) {
        let along = window.to_local(Point2::new(c.x, c.y)).0;
        min = min.min(along);
        max = max.max(along);
    }
    let width = max - min;
    (width > WALL_TOLERANCE + 1e-6).then_some([width, height])
}

/// Whether a door's leaf, sweeping `swing`, runs into `piece`.
fn leaf_hits(
    door: &Furniture,
    swing: &Polygon<f64>,
    piece: &Furniture,
    footprint: &Polygon<f64>,
) -> bool {
    // Below the door's sill (footings under a raised floor) is out of its way.
    if piece.is_opening() || piece.height <= FLAT || piece.height_range().1 <= door.elevation + 1.0
    {
        return false;
    }
    let shared = swing.intersection(footprint);
    shared.unsigned_area() > MIN_OVERLAP
        && crate::solid::hits_prism(piece, swing, door.elevation + 1.0, |_| {
            door.elevation + door.height
        })
}

/// The pieces of `door`'s storey blocking its leaf or approach, by the rule
/// [`check_layout`] reports `blocks_door` with — so a suggested fix, a hinge
/// on the other jamb say, is checked against the same test it has to pass.
pub fn door_blocked_by(home: &Home, door: &Furniture) -> Vec<FurnitureId> {
    if door
        .opening
        .as_ref()
        .is_none_or(|o| o.kind == crate::furniture::OpeningKind::Window)
    {
        return Vec::new();
    }
    let swing = door_swing(door).map(|pts| polygon(&pts));
    let level = home.resolve_level(door.level);
    home.furniture
        .iter()
        .filter(|top| top.id != door.id && home.resolve_level(top.level) == level)
        .flat_map(Furniture::visible_leaves)
        .filter(|leaf| leaf.id != door.id)
        .filter(|leaf| {
            let footprint = polygon(&leaf.projected_footprint());
            passage_hits(door, leaf, &footprint)
                || swing
                    .as_ref()
                    .is_some_and(|s| leaf_hits(door, s, leaf, &footprint))
        })
        .map(|leaf| leaf.id)
        .collect()
}

/// Bedrooms and bathrooms, by name, whose only ways in are open passages —
/// or that have none — on storeys that have doors somewhere.
fn rooms_without_door(
    home: &Home,
    wanted: &dyn Fn(Option<LevelId>) -> bool,
) -> Vec<(crate::ids::RoomId, Vec<FurnitureId>)> {
    use crate::furniture::OpeningKind;
    let private = |says: crate::vocabulary::Mentions| {
        use crate::vocabulary::Mention as M;
        says.any(&[M::Bedroom, M::Bathroom, M::Lavatory]) && !says.has(M::Closet)
    };
    let mut out = Vec::new();
    for room in home
        .rooms
        .iter()
        .filter(|r| r.points.len() >= 3 && wanted(r.level) && private(r.mentions()))
    {
        let level = home.resolve_level(room.level);
        let openings: Vec<&Furniture> = home
            .furniture
            .iter()
            .filter(|f| f.visible && home.resolve_level(f.level) == level)
            .filter(|f| {
                f.opening
                    .as_ref()
                    .is_some_and(|o| o.kind != OpeningKind::Window)
            })
            .collect();
        if openings.is_empty() {
            continue;
        }
        let near: Vec<&&Furniture> = openings
            .iter()
            .filter(|f| distance_to_outline(&room.points, f.position) <= 25.0)
            .collect();
        let passages: Vec<FurnitureId> = near
            .iter()
            .filter(|f| {
                f.opening
                    .as_ref()
                    .is_some_and(|o| o.kind == OpeningKind::Passage)
            })
            .map(|f| f.id)
            .collect();
        if near.is_empty() || passages.len() == near.len() {
            out.push((room.id, passages));
        }
    }
    out
}

fn distance_to_outline(points: &[Point2], p: Point2) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = (dx * dx + dy * dy).max(1e-9);
            let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
            Point2::new(a.x + t * dx, a.y + t * dy).distance(p)
        })
        .fold(f64::MAX, f64::min)
}

/// Room a door or a drawer needs in front of it before it is unusable, cm.
const OPENING_ROOM: f64 = 5.0;
/// A front this close to a wall, on a piece whose back belongs on one, is
/// a piece turned around: nobody sits with their knees 30 cm from a wall.
const BACKWARDS_REACH: f64 = 30.0;
/// How much freer its back must be than its front to say which was meant, cm.
const BACKWARDS_MARGIN: f64 = 20.0;

/// Whether this piece is opened from the front — a cabinet, a fridge, a
/// wardrobe, an oven — so that a solid against its face makes it useless.
///
/// Pieces that are simply approached from the front (a bed, a sofa, a table)
/// are left out: they lose comfort, not function, and ergonomics already
/// says so.
fn opens_at_the_front(piece: &Furniture) -> bool {
    const CATALOGS: [&str; 9] = [
        "base-cabinet",
        "wall-cabinet",
        "tall-cabinet",
        "wardrobe",
        "fridge",
        "dishwasher",
        "washer",
        "oven",
        "cabinet",
    ];
    if piece.is_opening() || piece.height <= FLAT {
        return false;
    }
    CATALOGS.iter().any(|c| piece.catalog.contains(c))
}

/// Wall outlines with corners joined **within each storey**.
///
/// [`Home::wall_outlines`] knows nothing about levels, so in a plan drawn as
/// stacked layers it joins a new wall to the traced copy underneath it and
/// the resulting outline covers ground neither wall does. Joining one storey
/// at a time keeps every outline honest; the result stays in `home.walls`
/// order so callers can zip it.
fn outlines_by_level(home: &Home) -> Vec<Vec<Point2>> {
    let mut levels: Vec<Option<LevelId>> = Vec::new();
    for wall in &home.walls {
        let level = home.resolve_level(wall.level);
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    if levels.len() < 2 {
        return home.wall_outlines();
    }
    let mut out = vec![Vec::new(); home.walls.len()];
    for level in levels {
        let view = home.level_view(level);
        for (wall, outline) in view.walls.iter().zip(view.wall_outlines()) {
            if let Some(at) = home.walls.iter().position(|w| w.id == wall.id) {
                out[at] = outline;
            }
        }
    }
    out
}

/// Size of what two pieces share, `[x, y, z]` cm.
fn extent_of(shared: &geo::MultiPolygon<f64>, a: &Furniture, b: &Furniture) -> [f64; 3] {
    let plan = shared.bounding_rect().map_or([0.0, 0.0], |r| {
        [r.max().x - r.min().x, r.max().y - r.min().y]
    });
    let z = shared_height(a.height_range(), b.height_range()).max(0.0);
    [plan[0], plan[1], z].map(round_mm)
}

/// When one of two overlapping pieces is a point of a project and the other
/// is not, the point is where it was put on purpose: the water point in the
/// basin, the sewer under the toilet, an outlet behind the fridge or set
/// into a cabinet — a technique, not a clash. Loose points are said apart
/// (`loose`), and whether a point can be set there by `mounting`.
fn point_in_piece(a: &Furniture, b: &Furniture) -> Option<Overlap> {
    match (a.discipline, b.discipline) {
        (Some(_), None) | (None, Some(_)) => Some(Overlap::Served),
        _ => None,
    }
}

/// Is this overlap a defect, or is one piece simply built into the other?
///
/// The rules are about shape and size only — no catalog names, no guessing
/// what a piece is called — so they hold for imported models and joinery
/// alike:
///
/// * a face panel, a back or a glass pane (3 cm or thinner) is part of what
///   it is stuck to;
/// * a lid over a box — a countertop on its cabinets — rests on it;
/// * a piece mostly inside a bigger one is built into it (an oven in its
///   tower, a cooktop in its worktop, a dishwasher under the counter);
/// * a couple of centimeters of drawing slack (a chair pushed under a table)
///   is how plans are drawn, not a clash.
fn classify(
    a: &Furniture,
    b: &Furniture,
    shared_area: f64,
    extent: [f64; 3],
    fa: &Polygon<f64>,
    fb: &Polygon<f64>,
) -> Overlap {
    let thin = |f: &Furniture| f.width.min(f.depth) <= 3.0;
    if thin(a) || thin(b) {
        return Overlap::Nesting;
    }
    let (area_a, area_b) = (fa.unsigned_area(), fb.unsigned_area());
    // A lid resting on a box: flat, and sitting at the other's top.
    let lid_on = |lid: &Furniture, box_: &Furniture, lid_area: f64, box_area: f64| {
        let (lo, _) = lid.height_range();
        lid.height <= 8.0 && (lo - box_.height_range().1).abs() <= 6.0 && lid_area >= 0.5 * box_area
    };
    if lid_on(a, b, area_a, area_b) || lid_on(b, a, area_b, area_a) {
        return Overlap::Nesting;
    }
    // Mostly inside the other: built in.
    if shared_area >= 0.6 * area_a.min(area_b) {
        return Overlap::Nesting;
    }
    // A seat pushed under the table it serves.
    let tucked = |seat: &Furniture, table: &Furniture| seat.is_seat() && table.is_table_height();
    if tucked(a, b) || tucked(b, a) {
        return Overlap::Nesting;
    }
    // Touching with drawing slack, or tucked under.
    if extent[0].min(extent[1]) <= 2.0 || extent[2] <= 1.0 {
        return Overlap::Nesting;
    }
    Overlap::Collision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::{Room, Wall};
    use crate::furniture::{Opening, align_to_wall};
    use crate::ids::RoomId;

    fn piece(id: u64, at: (f64, f64), size: (f64, f64, f64)) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: "box".into(),
            name: "Caixa".into(),
            position: Point2::new(at.0, at.1),
            elevation: 0.0,
            angle: 0.0,
            width: size.0,
            depth: size.1,
            height: size.2,
            mirrored: false,
            color: None,
            opening: None,
            model: None,
            visible: true,
            level: None,
            ..Default::default()
        }
    }

    fn room_home() -> Home {
        let mut home = Home::default();
        let pts = [(0.0, 0.0), (500.0, 0.0), (500.0, 400.0), (0.0, 400.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            let id = home.new_wall_id();
            home.walls
                .push(Wall::new(id, Point2::new(a.0, a.1), Point2::new(b.0, b.1)));
        }
        home.rooms.push(Room::new(
            RoomId(90),
            "Sala",
            pts.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        ));
        home
    }

    #[test]
    fn pendants_above_flat_ceilings_report_excess_and_follow_storey_height() {
        let mut home = Home::default();
        home.wall_height = 300.0;
        home.rooms.push(crate::Room::new(
            crate::RoomId(1),
            "Sala",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(400.0, 0.0),
                Point2::new(400.0, 400.0),
                Point2::new(0.0, 400.0),
            ],
        ));
        home.furniture.push(Furniture {
            id: FurnitureId(2),
            catalog: "pendant".into(),
            position: Point2::new(200.0, 200.0),
            elevation: 250.0,
            height: 90.0,
            ..Furniture::default()
        });
        let excess = |h: &Home| {
            check_layout(h)
                .into_iter()
                .find(|i| matches!(i, Issue::AboveCeiling { .. }))
        };
        let issue = excess(&home).unwrap();
        assert!(
            matches!(issue,Issue::AboveCeiling {ceiling,top,..} if (top-ceiling-40.0).abs()<1e-6)
        );
        assert!(Issue::is_layout_key(&issue.key()));
        home.accepted
            .insert(issue.key(), "reviewed model bounds".into());
        assert!(!excess(&home).unwrap().is_pending(&home));
        home.furniture[0].elevation = 210.0;
        assert!(excess(&home).is_none(), "touching the ceiling is valid");
        home.levels.push(crate::Level {
            id: LevelId(3),
            elevation: 500.0,
            height: 280.0,
            ..crate::Level::default()
        });
        home.selected_level = Some(LevelId(3));
        home.rooms[0].level = Some(LevelId(3));
        home.furniture[0].level = Some(LevelId(3));
        assert!(
            matches!(excess(&home),Some(Issue::AboveCeiling {ceiling,top,..}) if (top-ceiling-20.0).abs()<1e-6)
        );
        home.furniture[0].visible = false;
        assert!(excess(&home).is_none());
        home.furniture[0].visible = true;
        home.rooms[0].ceiling_flat = false;
        assert!(
            excess(&home).is_none(),
            "do not invent a flat surface for a sloped ceiling"
        );
        home.rooms[0].ceiling_flat = true;
        home.rooms[0].ceiling_visible = false;
        assert!(excess(&home).is_none());
    }

    #[test]
    fn finds_overlaps_but_not_stacked_or_touching_pieces() {
        let mut home = room_home();
        home.furniture
            .push(piece(10, (200.0, 200.0), (100.0, 100.0, 75.0)));
        home.furniture
            .push(piece(11, (250.0, 200.0), (100.0, 100.0, 75.0))); // overlaps 10
        home.furniture
            .push(piece(12, (350.0, 200.0), (100.0, 100.0, 75.0))); // touches 11
        let mut shelf = piece(13, (200.0, 200.0), (100.0, 40.0, 50.0));
        shelf.elevation = 150.0; // above 10
        home.furniture.push(shelf);
        home.furniture
            .push(piece(14, (200.0, 200.0), (200.0, 140.0, 1.0))); // rug

        let issues = check_layout(&home);
        assert_eq!(
            issues,
            vec![Issue::Overlap {
                a: FurnitureId(10),
                b: FurnitureId(11),
                kind: Overlap::Collision,
                extent: [50.0, 100.0, 75.0],
            }],
            "{issues:?}"
        );
    }

    /// The oven that stopped fitting: the tower was made 8 cm shallower and
    /// nothing said a word, because an appliance built into joinery is left
    /// out of every overlap check on purpose.
    #[test]
    fn an_appliance_its_niche_no_longer_holds_is_reported() {
        let mut home = room_home();
        let mut tower = piece(60, (200.0, 150.0), (80.0, 64.0, 220.0));
        let mut oven = piece(61, (200.0, 150.0), (60.0, 56.0, 60.0));
        oven.elevation = 90.0;
        oven.properties
            .insert("joinery:embedded".into(), "oven".into());
        tower.children = vec![oven];
        home.furniture.push(tower);
        assert!(
            !check_layout(&home)
                .iter()
                .any(|i| matches!(i, Issue::OutgrewNiche { .. })),
            "it fits"
        );

        // The tower is made shallower around it.
        if let Some(tower) = home.furniture.last_mut() {
            tower.depth = 40.0;
        }
        let issues = check_layout(&home);
        let found = issues
            .iter()
            .find(|i| matches!(i, Issue::OutgrewNiche { .. }))
            .unwrap_or_else(|| panic!("{issues:?}"));
        let Issue::OutgrewNiche { piece, host, over } = found else {
            unreachable!()
        };
        assert_eq!((*piece, *host), (FurnitureId(61), FurnitureId(60)));
        assert!((over[1] - 16.0).abs() < 0.1, "{over:?}");
    }

    /// The trap the score itself set: a group whose fronts were built toward
    /// the kitchen carried an `angle` pointing at the living room, so every
    /// clearance was measured on the blind side and turning the piece
    /// "fixed" the warning while breaking the kitchen.
    #[test]
    fn a_group_that_opens_where_its_angle_does_not_is_reported() {
        let mut home = room_home();
        let mut tower = piece(50, (200.0, 150.0), (80.0, 64.0, 220.0));
        tower.angle = 0.0; // says it looks at +y
        let panel = |id: u64, name: &str, y: f64, h: f64, elev: f64| {
            let mut part = piece(id, (200.0, y), (78.0, 2.0, h));
            part.name = name.to_owned();
            part.elevation = elev;
            part
        };
        tower.children = vec![
            panel(51, "Frente fixa", 119.0, 40.0, 0.0),
            panel(52, "Gaveta", 119.0, 30.0, 40.0),
            panel(53, "Porta", 119.0, 60.0, 70.0),
            panel(54, "Painel cego", 181.0, 220.0, 0.0),
        ];
        home.furniture.push(tower);
        let issues = check_layout(&home);
        let turned = issues
            .iter()
            .find(|i| matches!(i, Issue::Turned { .. }))
            .unwrap_or_else(|| panic!("{issues:?}"));
        assert_eq!(
            *turned,
            Issue::Turned {
                piece: FurnitureId(50),
                built: "-y",
                placed: "+y",
            }
        );
        // And what is in front of it is measured on the side it opens to.
        let front = crate::measure::Dir::parse("front", home.furniture.last()).unwrap();
        assert_eq!(front.name(), "-y");
    }

    #[test]
    fn a_cabinet_turned_against_a_wall_is_reported_as_unusable() {
        let mut home = room_home();
        // A base cabinet against the top wall, opening into the room.
        let mut right_way = piece(60, (200.0, 40.0), (80.0, 60.0, 90.0));
        right_way.catalog = "base-cabinet".into();
        right_way.name = "Gabinete".into();
        // The same cabinet turned a half turn: its doors face the wall.
        let mut wrong_way = piece(61, (350.0, 40.0), (80.0, 60.0, 90.0));
        wrong_way.catalog = "base-cabinet".into();
        wrong_way.name = "Gabinete virado".into();
        wrong_way.angle = 180.0;
        home.furniture.extend([right_way, wrong_way]);

        let blocked: Vec<FurnitureId> = check_layout(&home)
            .into_iter()
            .filter_map(|i| match i {
                Issue::Blocked { piece, .. } => Some(piece),
                _ => None,
            })
            .collect();
        assert_eq!(blocked, vec![FurnitureId(61)], "{blocked:?}");
    }

    #[test]
    fn a_sofa_or_a_bed_turned_to_face_its_wall_is_backwards() {
        let mut home = room_home();
        let t = home.walls[0].thickness / 2.0;
        let sofa = |id, y: f64, angle| {
            let mut f = piece(id, (150.0, y), (210.0, 90.0, 85.0));
            f.catalog = "sofa-3".into();
            f.angle = angle;
            f
        };
        // Back on the top wall, seat to the room: right.
        let right = sofa(70, t + 45.0, 0.0);
        // Back to the room at the bottom wall, seat on the wall: the angle
        // 0 meant for the top wall, used at the bottom one.
        let wrong = sofa(71, 400.0 - t - 45.0, 0.0);
        // A bed with its foot on the right wall, headboard to the room.
        let mut bed = piece(72, (500.0 - t - 100.0, 250.0), (160.0, 200.0, 50.0));
        bed.catalog = "bed-double".into();
        bed.angle = 270.0;
        // A rug on the wall means nothing: it has no front.
        let mut rug = piece(73, (300.0, t + 30.0), (200.0, 60.0, 1.0));
        rug.catalog = "rug".into();
        rug.angle = 180.0;
        home.furniture.extend([right, wrong, bed, rug]);

        let backwards: Vec<(FurnitureId, f64)> = check_layout(&home)
            .into_iter()
            .filter_map(|i| match i {
                Issue::Backwards { piece, angle, .. } => Some((piece, angle)),
                _ => None,
            })
            .collect();
        // The sofa at the bottom wall looks down the plan into that wall;
        // the bed at 270 looks +x into the right wall. Both turn half around.
        assert_eq!(
            backwards,
            vec![(FurnitureId(71), 180.0), (FurnitureId(72), 90.0)],
            "{backwards:?}"
        );
    }

    #[test]
    fn a_curb_lower_than_the_piece_is_not_a_wall_it_faces() {
        let mut home = room_home();
        let t = home.walls[2].thickness / 2.0;
        // The bottom wall is a kick board, lower than the sofa's seat.
        home.walls[2].height = 18.0;
        let mut sofa = piece(81, (250.0, 400.0 - t - 50.0), (210.0, 90.0, 85.0));
        sofa.catalog = "sofa-3".into();
        home.furniture.push(sofa);
        assert!(
            !check_layout(&home)
                .iter()
                .any(|i| matches!(i, Issue::Backwards { .. })),
            "an 18 cm curb is not the wall its back was meant for"
        );
    }

    #[test]
    fn a_piece_between_two_walls_is_not_called_backwards() {
        let mut home = room_home();
        let t = home.walls[0].thickness / 2.0;
        // A 400 cm room minus two half walls, filled front to back.
        let mut sofa = piece(80, (250.0, 200.0), (210.0, 400.0 - 2.0 * t, 85.0));
        sofa.catalog = "sofa-3".into();
        home.furniture.push(sofa);
        assert!(
            !check_layout(&home)
                .iter()
                .any(|i| matches!(i, Issue::Backwards { .. })),
            "a niche says nothing about which way was meant"
        );
    }

    #[test]
    fn built_in_pieces_and_layers_are_named_instead_of_reported_as_clashes() {
        use crate::elements::Level;
        use crate::ids::LevelId;

        let mut home = room_home();
        // A worktop resting on its cabinet.
        let mut cabinet = piece(40, (100.0, 200.0), (120.0, 60.0, 84.0));
        cabinet.name = "Gabinete".into();
        let mut top = piece(41, (100.0, 200.0), (124.0, 62.0, 4.0));
        top.elevation = 84.0;
        // A cooktop set into it.
        let mut cooktop = piece(42, (100.0, 200.0), (58.0, 50.0, 10.0));
        cooktop.elevation = 82.0;
        // A chair pushed 1 cm under a table.
        let mut table = piece(43, (300.0, 200.0), (140.0, 80.0, 75.0));
        table.elevation = 0.0;
        let mut chair = piece(44, (300.0, 240.5), (45.0, 45.0, 90.0));
        chair.catalog = "chair".into();
        chair.name = "Cadeira".into();
        home.furniture
            .extend([cabinet, top, cooktop, table.clone(), chair]);
        let kinds: Vec<Overlap> = check_layout(&home)
            .into_iter()
            .filter_map(|i| match i {
                Issue::Overlap { kind, .. } => Some(kind),
                _ => None,
            })
            .collect();
        assert!(
            !kinds.is_empty() && kinds.iter().all(|k| *k == Overlap::Nesting),
            "{kinds:?} — built-in pieces are not clashes"
        );

        // But a worktop eating 5 cm into the dishwasher beside it is one.
        let mut sink = piece(45, (410.0, 60.0), (70.0, 65.0, 35.7));
        sink.name = "Pia centralizada".into();
        sink.elevation = 72.0;
        let mut dishwasher = piece(46, (469.5, 60.0), (59.8, 63.5, 84.5));
        dishwasher.name = "Lava-louças".into();
        home.furniture.extend([sink, dishwasher]);
        let clash = check_layout(&home)
            .into_iter()
            .find(|i| {
                matches!(i, Issue::Overlap { a, b, .. }
                if *a == FurnitureId(45) && *b == FurnitureId(46))
            })
            .expect("the sink invading the dishwasher is reported");
        let Issue::Overlap { kind, extent, .. } = clash else {
            unreachable!()
        };
        assert_eq!(kind, Overlap::Collision);
        assert!((extent[0] - 5.4).abs() < 0.2, "{extent:?}");

        // The same plan drawn twice, as two storeys at the same elevation.
        let mut layered = Home::default();
        layered.levels = vec![
            Level {
                id: LevelId(1),
                name: "Planta reproduzida".into(),
                elevation: 0.0,
                height: 280.0,
                ..Level::default()
            },
            Level {
                id: LevelId(2),
                name: "Novo layout".into(),
                elevation: 0.0,
                height: 280.0,
                elevation_index: 1,
                ..Level::default()
            },
        ];
        let mut old_table = table.clone();
        old_table.id = FurnitureId(50);
        old_table.level = Some(LevelId(1));
        let mut new_table = table;
        new_table.id = FurnitureId(51);
        new_table.level = Some(LevelId(2));
        layered.furniture = vec![old_table, new_table];
        let all = check_layout_in(&layered, Storeys::All);
        assert_eq!(
            all,
            vec![Issue::Overlap {
                a: FurnitureId(50),
                b: FurnitureId(51),
                kind: Overlap::CrossLevel,
                extent: [140.0, 80.0, 75.0],
            }],
            "{all:?}"
        );
        assert!(all.iter().all(|i| !i.is_defect()));
        // Only one storey at a time: nothing to report.
        assert_eq!(
            check_layout_in(&layered, Storeys::One(LevelId(2))),
            [] as [crate::analysis::Issue; 0]
        );
        // Marking the traced plan as a reference drops it from `All` too.
        layered.levels[0].set_reference(true);
        assert_eq!(
            check_layout_in(&layered, Storeys::All),
            [] as [crate::analysis::Issue; 0]
        );
        assert_eq!(layered.stacked_levels(), Vec::new());
    }

    #[test]
    fn pieces_of_two_storeys_overlap_only_where_they_share_height_in_the_building() {
        use crate::elements::Level;
        let mut home = Home::default();
        home.levels = vec![
            Level {
                id: LevelId(1),
                name: "Térreo".into(),
                elevation: 0.0,
                height: 280.0,
                ..Level::default()
            },
            Level {
                id: LevelId(2),
                name: "Superior".into(),
                elevation: 292.0,
                height: 280.0,
                elevation_index: 1,
                ..Level::default()
            },
        ];
        let piece = |id: u64, level: u64, elevation: f64, height: f64| Furniture {
            id: FurnitureId(id),
            catalog: "table".into(),
            name: format!("f{id}"),
            position: Point2::new(100.0, 100.0),
            width: 140.0,
            depth: 80.0,
            height,
            elevation,
            level: Some(LevelId(level)),
            ..Furniture::default()
        };
        // A table downstairs, one right above it upstairs, and a shelf whose
        // top meets the upper floor: a plan shared, no space shared.
        home.furniture = vec![
            piece(1, 1, 0.0, 75.0),
            piece(2, 2, 0.0, 75.0),
            piece(3, 1, 212.0, 80.0),
        ];
        assert_eq!(check_layout_in(&home, Storeys::All), Vec::new());
        // A cabinet reaching 18 cm above the upper floor does meet the table there.
        home.furniture[2].height = 98.0;
        assert_eq!(
            check_layout_in(&home, Storeys::All),
            vec![Issue::Overlap {
                a: FurnitureId(2),
                b: FurnitureId(3),
                kind: Overlap::CrossLevel,
                extent: [140.0, 80.0, 18.0],
            }]
        );
    }

    #[test]
    fn windows_reject_hoods_and_tall_furniture_but_allow_countertop_appliances() {
        use crate::furniture::OpeningKind;
        for angle in [0.0, 90.0, 37.0] {
            let mut window = piece(1, (200.0, 200.0), (180.0, 20.0, 140.0));
            window.angle = angle;
            window.elevation = 110.0;
            window.opening = Some(Opening {
                kind: OpeningKind::Window,
                ..Opening::default()
            });
            let mut hood = piece(2, (0.0, 0.0), (60.0, 50.0, 60.0));
            hood.position = window.to_plan((70.0, 40.0));
            hood.angle = angle;
            hood.elevation = 180.0;
            let mut home = Home::default();
            home.furniture = vec![window.clone(), hood.clone()];
            let issues = check_layout(&home);
            let extent = issues
                .iter()
                .find_map(|i| match i {
                    Issue::BlocksWindow {
                        window: FurnitureId(1),
                        by: FurnitureId(2),
                        extent,
                    } => Some(*extent),
                    _ => None,
                })
                .expect("partial hood obstruction must be reported at any angle");
            assert!((extent[0] - 50.0).abs() < 0.001, "{extent:?}");
            assert!((extent[1] - 60.0).abs() < 0.001, "{extent:?}");
            let mut airfryer = hood.clone();
            airfryer.name = "Airfryer sobre bancada".into();
            airfryer.width = 35.0;
            airfryer.depth = 40.0;
            airfryer.height = 35.0;
            airfryer.elevation = 90.0;
            assert!(window_obstruction(&window, &airfryer).is_none());
            let mut glass = hood.clone();
            glass.opacity = Some(0.2);
            assert!(window_obstruction(&window, &glass).is_none());
            glass.opacity = None;
            glass.catalog = "plant".into();
            assert!(window_obstruction(&window, &glass).is_none());
            // Bars or glass under a handrail, rising past the sill (a landing
            // guard in front of a stair window).
            let mut guard = hood.clone();
            guard.elevation = 40.0;
            guard.height = 110.0;
            guard.width = 180.0;
            guard.depth = 5.0;
            assert!(window_obstruction(&window, &guard).is_some(), "a solid box");
            for catalog in ["railing", "glass-railing", "balcony-glazing"] {
                guard.catalog = catalog.into();
                assert!(window_obstruction(&window, &guard).is_none(), "{catalog}");
            }
            guard.catalog = "box".into();
            guard.name = "Guarda-corpo de barras do patamar".into();
            assert!(
                window_obstruction(&window, &guard).is_none(),
                "named a guard"
            );
            let mut fridge = hood.clone();
            fridge.elevation = 0.0;
            fridge.height = 180.0;
            assert!(window_obstruction(&window, &fridge).is_some());
            hood.position = window.to_plan((130.0, 40.0));
            assert!(
                window_obstruction(&window, &hood).is_none(),
                "beside window"
            );
            hood.position = window.to_plan((0.0, 150.0));
            assert!(
                window_obstruction(&window, &hood).is_none(),
                "away from window"
            );
            hood.position = window.to_plan((0.0, 40.0));
            hood.elevation = 260.0;
            assert!(window_obstruction(&window, &hood).is_none(), "above window");
            home.furniture[1]
                .properties
                .insert("joinery:embedded".into(), "hood".into());
            assert!(
                check_layout(&home)
                    .iter()
                    .any(|i| matches!(i, Issue::BlocksWindow { .. })),
                "embedded appliances still block windows"
            );
            home.furniture[1].visible = false;
            assert!(
                !check_layout(&home)
                    .iter()
                    .any(|i| matches!(i, Issue::BlocksWindow { .. }))
            );
        }
    }

    #[test]
    fn sliding_doors_and_passages_require_an_unobstructed_approach_on_both_sides() {
        use crate::furniture::OpeningKind;
        for kind in [OpeningKind::Door, OpeningKind::Passage] {
            for side in [-1.0, 1.0] {
                let mut door = piece(1, (200.0, 200.0), (90.0, 20.0, 210.0));
                door.opening = Some(Opening {
                    kind,
                    sliding: true,
                    ..Opening::default()
                });
                let blocker = piece(2, (200.0, 200.0 + side * 40.0), (60.0, 40.0, 90.0));
                let mut home = Home::default();
                home.furniture = vec![door.clone(), blocker];
                assert!(check_layout(&home).contains(&Issue::BlocksDoor {
                    door: door.id,
                    by: FurnitureId(2)
                }));
                assert_eq!(door_blocked_by(&home, &door), vec![FurnitureId(2)]);
                home.furniture[1].elevation = 220.0;
                assert!(door_blocked_by(&home, &door).is_empty(), "above door");
                home.furniture[1].elevation = 0.0;
                home.furniture[1].height = 1.0;
                assert!(door_blocked_by(&home, &door).is_empty(), "rug");
            }
        }
    }

    #[test]
    fn pieces_against_a_wall_or_below_a_raised_door_are_fine() {
        let mut home = room_home();
        // A wardrobe pressing 1 cm into the top wall's face: against it, not in it.
        let face = home.walls[0].thickness / 2.0;
        home.furniture
            .push(piece(30, (250.0, face - 1.0 + 30.0), (180.0, 60.0, 220.0)));
        // 5 cm in: that one is in the wall.
        home.furniture
            .push(piece(31, (100.0, face - 5.0 + 20.0), (60.0, 40.0, 80.0)));
        let mut door = piece(32, (0.0, 0.0), (80.0, 15.0, 210.0));
        door.opening = Some(Opening::default());
        door.elevation = 55.0;
        let left_wall = home.walls[3].clone();
        align_to_wall(&mut door, &left_wall, 200.0);
        home.furniture.push(door);
        // A footing under the raised floor, right in the swing.
        home.furniture
            .push(piece(33, (40.0, 180.0), (40.0, 40.0, 50.0)));
        let issues = check_layout(&home);
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, Issue::InWall(FurnitureId(30), _))),
            "{issues:?}"
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, Issue::InWall(FurnitureId(31), _))),
            "{issues:?}"
        );
        assert!(
            !issues.iter().any(|i| matches!(
                i,
                Issue::BlocksDoor {
                    by: FurnitureId(33),
                    ..
                }
            )),
            "{issues:?}"
        );
    }

    #[test]
    fn finds_pieces_in_walls_blocked_doors_and_outside_rooms() {
        let mut home = room_home();
        home.furniture
            .push(piece(20, (250.0, 10.0), (100.0, 60.0, 80.0))); // pokes into top wall
        let mut door = piece(21, (0.0, 0.0), (80.0, 15.0, 210.0));
        door.opening = Some(Opening::default());
        let left_wall = home.walls[3].clone(); // (0,400) -> (0,0)
        align_to_wall(&mut door, &left_wall, 200.0);
        home.furniture.push(door);
        home.furniture
            .push(piece(22, (40.0, 180.0), (40.0, 40.0, 80.0))); // in the swing
        home.furniture
            .push(piece(23, (900.0, 900.0), (40.0, 40.0, 40.0))); // outside
        // A panel of slats outside is one piece outside; a shelf of a
        // cabinet sticking out of the room is that shelf.
        let mut panel = piece(24, (900.0, 600.0), (100.0, 5.0, 200.0));
        panel.children = [(25, 860.0), (26, 900.0), (27, 940.0)]
            .map(|(id, x)| piece(id, (x, 600.0), (4.0, 5.0, 200.0)))
            .to_vec();
        home.furniture.push(panel);
        let mut cabinet = piece(30, (100.0, 380.0), (100.0, 40.0, 200.0));
        cabinet.children = vec![
            piece(31, (100.0, 380.0), (100.0, 40.0, 200.0)),
            piece(32, (100.0, 420.0), (100.0, 30.0, 2.0)),
        ];
        home.furniture.push(cabinet);

        let issues = check_layout(&home);
        assert!(
            issues.contains(&Issue::InWall(FurnitureId(20), WallId(1))),
            "{issues:?}"
        );
        assert!(
            issues.contains(&Issue::OutsideRooms(FurnitureId(23))),
            "{issues:?}"
        );
        let outside: Vec<FurnitureId> = issues
            .iter()
            .filter_map(|i| match i {
                Issue::OutsideRooms(f) => Some(*f),
                _ => None,
            })
            .collect();
        assert_eq!(
            outside,
            [FurnitureId(23), FurnitureId(24), FurnitureId(32)],
            "{issues:?}"
        );
        // A slat accepted as outside before keeps the panel said slat by slat.
        home.accepted
            .insert("outside_rooms:f26".into(), "fachada".into());
        let issues = check_layout(&home);
        let panel: Vec<&Issue> = issues
            .iter()
            .filter(|i| matches!(i, Issue::OutsideRooms(f) if [24, 25, 26, 27].contains(&f.0)))
            .collect();
        assert_eq!(panel.len(), 3, "{issues:?}");
        assert!(panel.iter().any(|i| i.accepted(&home) == Some("fachada")));
        let swing_side = door_swing(&home.furniture[1]).unwrap();
        assert!(
            swing_side.iter().all(|p| p.x >= -8.0),
            "door swings into the room: {swing_side:?}"
        );
        assert!(
            issues.contains(&Issue::BlocksDoor {
                door: FurnitureId(21),
                by: FurnitureId(22)
            }),
            "{issues:?}"
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, Issue::InWall(FurnitureId(21), _))),
            "doors belong in walls"
        );
    }
}

#[cfg(test)]
mod tilt_tests {
    use super::*;
    use crate::elements::Wall;
    use crate::furniture::{Opening, align_to_wall, wall_cuts};
    use crate::ids::WallId;

    fn edge_slope() -> Furniture {
        Furniture {
            id: FurnitureId(3),
            catalog: "box".into(),
            width: 200.0,
            depth: 80.0,
            height: 10.0,
            elevation: 100.0,
            roll: 45.0,
            ..Furniture::default()
        }
    }

    #[test]
    fn tilted_box_collision_at_an_edge_is_not_missed() {
        let mut home = Home::default();
        home.furniture = vec![
            edge_slope(),
            Furniture {
                id: FurnitureId(4),
                catalog: "box".into(),
                width: 100.0,
                depth: 80.0,
                height: 90.0,
                ..Furniture::default()
            },
        ];
        assert!(
            check_layout(&home).iter().any(|i| matches!(
                i,
                Issue::Overlap {
                    a: FurnitureId(3),
                    b: FurnitureId(4),
                    ..
                }
            )),
            "{:?}",
            check_layout(&home)
        );
    }

    #[test]
    fn tilted_wall_collision_at_an_edge_is_not_missed() {
        let mut home = Home::default();
        let mut wall = Wall::new(WallId(1), Point2::new(-100.0, 0.0), Point2::new(100.0, 0.0));
        wall.height = 90.0;
        wall.thickness = 20.0;
        home.walls.push(wall);
        home.furniture.push(edge_slope());
        assert!(
            check_layout(&home).contains(&Issue::InWall(FurnitureId(3), WallId(1))),
            "{:?}",
            check_layout(&home)
        );
    }

    #[test]
    fn tilted_door_collision_at_an_edge_is_not_missed() {
        let door = Furniture {
            height: 90.0,
            ..Furniture::default()
        };
        let slope = edge_slope();
        let region = polygon(&[
            Point2::new(-50.0, -40.0),
            Point2::new(50.0, -40.0),
            Point2::new(50.0, 40.0),
            Point2::new(-50.0, 40.0),
        ]);
        assert!(leaf_hits(
            &door,
            &region,
            &slope,
            &polygon(&slope.projected_footprint())
        ));
    }

    #[test]
    fn openings_fit_sloping_walls_and_raised_slopes_do_not_collide() {
        // Gable wall rising from 1 cm to 675 cm over 300 cm.
        let mut wall = Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(300.0, 0.0));
        wall.height = 1.0;
        wall.height_at_end = Some(675.0);
        let mut door = Furniture {
            id: FurnitureId(2),
            catalog: "door".into(),
            width: 80.0,
            depth: 15.0,
            height: 210.0,
            opening: Some(Opening::default()),
            ..Furniture::default()
        };
        align_to_wall(&mut door, &wall, 200.0);
        let cuts = wall_cuts(std::slice::from_ref(&wall), &[door.clone()]);
        let cut = &cuts[0][0];
        // At 160 cm along, the wall is ~360 cm high: the whole door fits.
        assert!((cut.top - 210.0).abs() < 1e-9, "{cut:?}");

        // A roof slope over the room: 45°, centered 300 cm up.
        let slope = Furniture {
            id: FurnitureId(3),
            catalog: "box".into(),
            position: Point2::new(0.0, 0.0),
            width: 400.0,
            depth: 400.0,
            height: 12.0,
            elevation: 300.0,
            pitch: -45.0,
            ..Furniture::default()
        };
        let low = slope.underside_at(Point2::new(0.0, -100.0));
        let high = slope.underside_at(Point2::new(0.0, 100.0));
        assert!(high > low + 150.0, "{low} {high}");
        let fp = slope.projected_footprint();
        let depth = fp[0].distance(fp[3]);
        assert!(depth < 400.0 * 0.8, "projected depth {depth}");
        let (bottom, top) = slope.height_range();
        assert!(bottom < 200.0 && top > 450.0, "{bottom} {top}");
        // A table under the high end of the slope does not collide with it.
        let table = Furniture {
            id: FurnitureId(4),
            catalog: "box".into(),
            position: Point2::new(0.0, 100.0),
            width: 60.0,
            depth: 60.0,
            height: 75.0,
            ..Furniture::default()
        };
        let mut home = Home::default();
        home.furniture = vec![slope, table];
        assert!(check_layout(&home).is_empty(), "{:?}", check_layout(&home));
    }
}
