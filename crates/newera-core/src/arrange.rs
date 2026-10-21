//! Arranging elements: copies in a row, rotation about a point, mirroring
//! across a line, groups and drawing order. Every operation is one undoable
//! command batch.

use crate::command::Command;
use crate::document::Document;
use crate::elements::Element;
use crate::error::{CoreError, CoreResult};
use crate::furniture::Furniture;
use crate::geometry::Point2;
use crate::ids::{ElementId, FurnitureId};

/// How points move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    Translate {
        dx: f64,
        dy: f64,
        dz: f64,
    },
    /// Clockwise on the plan (y down), degrees.
    Rotate {
        about: Point2,
        degrees: f64,
    },
    /// Across the line through `a` and `b`.
    Mirror {
        a: Point2,
        b: Point2,
    },
}

impl Transform {
    pub fn point(self, p: Point2) -> Point2 {
        match self {
            Self::Translate { dx, dy, .. } => Point2::new(p.x + dx, p.y + dy),
            Self::Rotate { about, degrees } => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                let (x, y) = (p.x - about.x, p.y - about.y);
                Point2::new(about.x + x * cos - y * sin, about.y + x * sin + y * cos)
            }
            Self::Mirror { a, b } => {
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let len2 = (dx * dx + dy * dy).max(1e-12);
                let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2;
                let foot = Point2::new(a.x + dx * t, a.y + dy * t);
                Point2::new(2.0 * foot.x - p.x, 2.0 * foot.y - p.y)
            }
        }
    }

    /// New clockwise angle of something that was at `angle` degrees.
    fn angle(self, angle: f64) -> f64 {
        match self {
            Self::Translate { .. } => angle,
            Self::Rotate { degrees, .. } => angle + degrees,
            Self::Mirror { a, b } => {
                let axis = (b.y - a.y).atan2(b.x - a.x).to_degrees();
                2.0 * axis - angle
            }
        }
    }

    fn flips(self) -> bool {
        matches!(self, Self::Mirror { .. })
    }

    fn dz(self) -> f64 {
        match self {
            Self::Translate { dz, .. } => dz,
            _ => 0.0,
        }
    }
}

fn move_piece(piece: &mut Furniture, t: Transform) {
    piece.position = t.point(piece.position);
    piece.angle = t.angle(piece.angle);
    piece.elevation += t.dz();
    if t.flips() {
        piece.mirrored = !piece.mirrored;
    }
    for child in &mut piece.children {
        move_piece(child, t);
    }
}

/// The element moved by `t`.
pub fn transformed(element: Element, t: Transform) -> Element {
    let map = |points: Vec<Point2>| points.into_iter().map(|p| t.point(p)).collect();
    match element {
        Element::Wall(mut w) => {
            w.start = t.point(w.start);
            w.end = t.point(w.end);
            if t.flips() {
                w.arc_extent = w.arc_extent.map(|a| -a);
                std::mem::swap(&mut w.left_side, &mut w.right_side);
                std::mem::swap(&mut w.left_baseboard, &mut w.right_baseboard);
            }
            Element::Wall(w)
        }
        Element::Room(mut r) => {
            r.points = map(r.points);
            Element::Room(r)
        }
        Element::Dimension(mut d) => {
            d.start = t.point(d.start);
            d.end = t.point(d.end);
            if t.flips() {
                d.offset = -d.offset;
            }
            Element::Dimension(d)
        }
        Element::Label(mut l) => {
            l.position = t.point(l.position);
            if !matches!(t, Transform::Translate { .. }) {
                // Text stays readable: only its direction follows.
                l.angle = t.angle(l.angle).rem_euclid(360.0);
                if t.flips() {
                    l.angle = (l.angle + 180.0).rem_euclid(360.0);
                }
            }
            l.elevation += t.dz();
            Element::Label(l)
        }
        Element::Polyline(mut p) => {
            p.points = map(p.points);
            Element::Polyline(p)
        }
        Element::Furniture(mut f) => {
            move_piece(&mut f, t);
            Element::Furniture(f)
        }
        Element::Level(level) => Element::Level(level),
    }
}

/// Gives a piece, and every piece inside it, fresh ids: what a copy needs
/// before it can live next to its original.
pub fn renumber_piece(doc: &mut Document, piece: &mut Furniture) {
    piece.id = doc.new_furniture_id();
    for child in &mut piece.children {
        renumber_piece(doc, child);
    }
}

/// The element with fresh ids (pieces inside groups too).
fn with_new_id(doc: &mut Document, element: Element) -> Element {
    match element {
        Element::Wall(mut w) => {
            w.id = doc.new_wall_id();
            Element::Wall(w)
        }
        Element::Room(mut r) => {
            r.id = doc.new_room_id();
            Element::Room(r)
        }
        Element::Dimension(mut d) => {
            d.id = doc.new_dimension_id();
            Element::Dimension(d)
        }
        Element::Label(mut l) => {
            l.id = doc.new_label_id();
            Element::Label(l)
        }
        Element::Polyline(mut p) => {
            p.id = doc.new_polyline_id();
            Element::Polyline(p)
        }
        Element::Furniture(mut f) => {
            renumber_piece(doc, &mut f);
            Element::Furniture(f)
        }
        Element::Level(mut level) => {
            level.id = doc.new_level_id();
            Element::Level(level)
        }
    }
}

fn elements(doc: &Document, ids: &[ElementId]) -> CoreResult<Vec<Element>> {
    ids.iter()
        .map(|id| {
            doc.home()
                .element(*id)
                .filter(|e| !matches!(e, Element::Level(_)))
                .ok_or(CoreError::NotFound(*id))
        })
        .collect()
}

/// Moves the elements by `t`, or with `copy` adds a moved copy. Returns the
/// ids of the elements that ended up moved (the copies when copying).
///
/// # Errors
/// Unknown ids or invalid resulting geometry.
pub fn apply(
    doc: &mut Document,
    ids: &[ElementId],
    t: Transform,
    copy: bool,
) -> CoreResult<Vec<ElementId>> {
    array(doc, ids, |_| t, usize::from(copy))
}

/// `count` copies, the `i`-th (from 1) moved by `step(i)`; with `count` 0
/// the originals themselves move by `step(1)`.
///
/// # Errors
/// Unknown ids or invalid resulting geometry.
pub fn array(
    doc: &mut Document,
    ids: &[ElementId],
    step: impl Fn(usize) -> Transform,
    count: usize,
) -> CoreResult<Vec<ElementId>> {
    let originals = elements(doc, ids)?;
    let mut commands = Vec::new();
    let mut out = Vec::new();
    if count == 0 {
        // A mirror turns a wall's sides over, and the products on them go
        // with their side: off every side first, then onto the other, so a
        // wall finished on both sides never meets itself half way.
        let flipped: Vec<crate::finishes::FinishAssignment> = if step(1).flips() {
            doc.home()
                .finish_assignments
                .iter()
                .filter(|a| ids.contains(&a.surface.element()))
                .filter(|a| matches!(a.surface, crate::finishes::Surface::Wall(..)))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        for element in originals {
            let moved = transformed(element, step(1));
            out.push(moved.id());
            commands.push(Command::update(moved));
        }
        for assignment in &flipped {
            commands.push(Command::RemoveFinish {
                surface: assignment.surface,
                band: assignment.band,
            });
        }
        for mut assignment in flipped {
            if let crate::finishes::Surface::Wall(wall, side) = assignment.surface {
                assignment.surface = crate::finishes::Surface::Wall(wall, side.other());
            }
            commands.push(Command::AssignFinish {
                assignment,
                index: None,
            });
        }
    } else {
        for i in 1..=count {
            for element in &originals {
                let copy = with_new_id(doc, transformed(element.clone(), step(i)));
                out.push(copy.id());
                let finishes = match (element, &copy) {
                    (Element::Wall(from), Element::Wall(to)) => {
                        crate::finishes::copied(doc.home(), from.id, to.id, step(i).flips())
                    }
                    _ => Vec::new(),
                };
                commands.push(Command::insert(copy));
                // A copied wall is finished as the one it copies.
                commands.extend(finishes);
            }
        }
    }
    doc.execute(Command::Batch { commands })?;
    Ok(out)
}

/// Groups pieces into one box that moves, turns and hides as a unit.
///
/// # Errors
/// Fewer than two pieces, or ids that are not top-level furniture.
/// Which edge of a piece an alignment holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The lower coordinate along the axis: left on x, the top of the plan on y.
    Low,
    /// The middle.
    Middle,
    /// The higher coordinate.
    High,
}

/// Lines up pieces on one edge, at `value` cm along `axis`.
///
/// What a run of joinery needs is its backs on one line and its fronts on
/// another, and that is a sentence about edges — not about centers, which is
/// what every write takes. Doing it by hand means recomputing a center per
/// piece, per resize, and it is where a plan drifts.
///
/// # Errors
/// When an id is not a piece of furniture, or is not on this storey.
pub fn align(
    doc: &mut Document,
    ids: &[FurnitureId],
    axis: crate::measure::Axis,
    edge: Edge,
    value: f64,
) -> CoreResult<Vec<ElementId>> {
    let mut commands = Vec::new();
    let mut moved = Vec::new();
    for id in ids {
        let piece = doc
            .home()
            .piece(*id)
            .cloned()
            .ok_or(CoreError::NotFound((*id).into()))?;
        let (min, max) = crate::measure::plan_bounds(&piece);
        let (lo, hi, center) = match axis {
            crate::measure::Axis::X => (min.x, max.x, piece.position.x),
            crate::measure::Axis::Y => (min.y, max.y, piece.position.y),
        };
        let at = match edge {
            Edge::Low => lo,
            Edge::Middle => f64::midpoint(lo, hi),
            Edge::High => hi,
        };
        let shift = value - at;
        if shift.abs() < 1e-9 {
            continue;
        }
        let mut piece = piece;
        let moved_to = center + shift;
        match axis {
            crate::measure::Axis::X => piece.translate(moved_to - piece.position.x, 0.0),
            crate::measure::Axis::Y => piece.translate(0.0, moved_to - piece.position.y),
        }
        moved.push(ElementId::from(piece.id));
        commands.push(Command::update(piece));
    }
    if !commands.is_empty() {
        doc.execute(Command::Batch { commands })?;
    }
    Ok(moved)
}

/// Sets the pieces side by side along `axis`, in the order they are given,
/// leaving `gap` cm between them and starting where the first one is.
///
/// A run of cabinets is drawn like this and only like this: touching, in
/// order, from one end. Spelling it as centers is arithmetic nobody should
/// be doing twice.
///
/// # Errors
/// When an id is not a piece of furniture, or is not on this storey.
pub fn distribute(
    doc: &mut Document,
    ids: &[FurnitureId],
    axis: crate::measure::Axis,
    gap: f64,
) -> CoreResult<Vec<ElementId>> {
    let mut cursor: Option<f64> = None;
    let mut commands = Vec::new();
    let mut moved = Vec::new();
    for id in ids {
        let piece = doc
            .home()
            .piece(*id)
            .cloned()
            .ok_or(CoreError::NotFound((*id).into()))?;
        let (min, max) = crate::measure::plan_bounds(&piece);
        let (lo, hi) = match axis {
            crate::measure::Axis::X => (min.x, max.x),
            crate::measure::Axis::Y => (min.y, max.y),
        };
        let start = cursor.unwrap_or(lo);
        let shift = start - lo;
        cursor = Some(start + (hi - lo) + gap);
        if shift.abs() < 1e-9 {
            continue;
        }
        let mut piece = piece;
        match axis {
            crate::measure::Axis::X => piece.translate(shift, 0.0),
            crate::measure::Axis::Y => piece.translate(0.0, shift),
        }
        moved.push(ElementId::from(piece.id));
        commands.push(Command::update(piece));
    }
    if !commands.is_empty() {
        doc.execute(Command::Batch { commands })?;
    }
    Ok(moved)
}

pub fn group(doc: &mut Document, ids: &[ElementId], name: &str) -> CoreResult<FurnitureId> {
    let pieces: Vec<Furniture> = ids
        .iter()
        .map(|id| match (id, doc.home().element(*id)) {
            (ElementId::Furniture(_), Some(Element::Furniture(f)))
                if doc.home().furniture.iter().any(|t| t.id == f.id) =>
            {
                Ok(f)
            }
            _ => Err(CoreError::InvalidGeometry(format!(
                "{id} is not a piece of furniture outside groups"
            ))),
        })
        .collect::<CoreResult<_>>()?;
    if pieces.len() < 2 {
        return Err(CoreError::InvalidGeometry(
            "a group needs at least two pieces".into(),
        ));
    }
    let corners: Vec<Point2> = pieces.iter().flat_map(Furniture::footprint).collect();
    let (min, max) = corners.iter().fold(
        (
            Point2::new(f64::MAX, f64::MAX),
            Point2::new(f64::MIN, f64::MIN),
        ),
        |(lo, hi), p| {
            (
                Point2::new(lo.x.min(p.x), lo.y.min(p.y)),
                Point2::new(hi.x.max(p.x), hi.y.max(p.y)),
            )
        },
    );
    let bottom = pieces.iter().map(|p| p.elevation).fold(f64::MAX, f64::min);
    let top = pieces
        .iter()
        .map(|p| p.elevation + p.height)
        .fold(f64::MIN, f64::max);
    let group = Furniture {
        id: doc.new_furniture_id(),
        catalog: "group".into(),
        name: if name.trim().is_empty() {
            "Grupo".into()
        } else {
            name.trim().into()
        },
        position: Point2::new(f64::midpoint(min.x, max.x), f64::midpoint(min.y, max.y)),
        elevation: bottom,
        width: (max.x - min.x).max(1.0),
        depth: (max.y - min.y).max(1.0),
        height: (top - bottom).max(1.0),
        level: pieces[0].level,
        children: pieces.clone(),
        ..Furniture::default()
    };
    let id = group.id;
    let mut commands: Vec<Command> = pieces.iter().map(|p| Command::remove(p.id)).collect();
    commands.push(Command::insert(group));
    doc.execute(Command::Batch { commands })?;
    Ok(id)
}

/// Puts the pieces of a group back on the plan.
///
/// # Errors
/// When `id` is not a group.
pub fn ungroup(doc: &mut Document, id: FurnitureId) -> CoreResult<Vec<FurnitureId>> {
    let group = doc
        .home()
        .furniture
        .iter()
        .find(|f| f.id == id && f.is_group())
        .cloned()
        .ok_or_else(|| CoreError::InvalidGeometry(format!("{id} is not a group")))?;
    let ids = group.children.iter().map(|c| c.id).collect();
    let mut commands = vec![Command::remove(id)];
    commands.extend(group.children.into_iter().map(Command::insert));
    doc.execute(Command::Batch { commands })?;
    Ok(ids)
}

/// Draws the elements last (on top) or first (underneath) among their kind:
/// overlapping floors, rugs under furniture.
///
/// # Errors
/// Unknown ids.
pub fn reorder(doc: &mut Document, ids: &[ElementId], front: bool) -> CoreResult<()> {
    let originals = elements(doc, ids)?;
    let mut commands: Vec<Command> = originals.iter().map(|e| Command::remove(e.id())).collect();
    for (k, element) in originals.into_iter().enumerate() {
        commands.push(Command::Insert {
            element,
            index: Some(if front { usize::MAX } else { k }),
        });
    }
    doc.execute(Command::Batch { commands })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Room, Wall};

    fn close(a: Point2, b: (f64, f64)) -> bool {
        (a.x - b.0).abs() < 1e-6 && (a.y - b.1).abs() < 1e-6
    }

    fn doc_with_piece() -> (Document, ElementId) {
        let mut doc = Document::default();
        let piece = Furniture {
            id: doc.new_furniture_id(),
            catalog: "box".into(),
            position: Point2::new(100.0, 0.0),
            width: 40.0,
            depth: 20.0,
            height: 10.0,
            ..Furniture::default()
        };
        let id = piece.id.into();
        doc.execute(Command::insert(piece)).unwrap();
        (doc, id)
    }

    #[test]
    fn arrays_rotations_and_mirrors() {
        let (mut doc, id) = doc_with_piece();
        // Rafters every 100 cm, 5 copies, the last one 50 cm higher.
        let copies = array(
            &mut doc,
            &[id],
            |i| {
                let i = f64::from(u32::try_from(i).unwrap());
                Transform::Translate {
                    dx: 0.0,
                    dy: 100.0 * i,
                    dz: 10.0 * i,
                }
            },
            5,
        )
        .unwrap();
        assert_eq!(copies.len(), 5);
        let last = doc.home().furniture.last().unwrap();
        assert!(close(last.position, (100.0, 500.0)) && (last.elevation - 50.0).abs() < 1e-9);
        doc.undo().unwrap();
        assert_eq!(
            doc.home().furniture.len(),
            1,
            "one undo removes the whole array"
        );

        // A quarter turn about the origin: (100, 0) → (0, 100), angle 90.
        apply(
            &mut doc,
            &[id],
            Transform::Rotate {
                about: Point2::new(0.0, 0.0),
                degrees: 90.0,
            },
            false,
        )
        .unwrap();
        let piece = &doc.home().furniture[0];
        assert!(close(piece.position, (0.0, 100.0)) && (piece.angle - 90.0).abs() < 1e-9);

        // Mirror copy across the vertical line x = 50.
        let (mut doc, id) = doc_with_piece();
        let mut wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 300.0),
        );
        wall.arc_extent = Some(30.0);
        wall.left_side = Some(crate::Material::paint([200, 0, 0]));
        let wall_id = wall.id.into();
        doc.execute(Command::insert(wall)).unwrap();
        let copies = apply(
            &mut doc,
            &[id, wall_id],
            Transform::Mirror {
                a: Point2::new(50.0, 0.0),
                b: Point2::new(50.0, 10.0),
            },
            true,
        )
        .unwrap();
        let home = doc.home();
        let piece = home.furniture.last().unwrap();
        assert!(close(piece.position, (0.0, 0.0)) && piece.mirrored);
        let mirrored = home
            .walls
            .iter()
            .find(|w| ElementId::from(w.id) == copies[1])
            .unwrap();
        assert!(close(mirrored.start, (100.0, 0.0)));
        assert_eq!(mirrored.arc_extent, Some(-30.0));
        assert!(mirrored.right_side.is_some() && mirrored.left_side.is_none());
    }

    #[test]
    fn groups_and_drawing_order() {
        let (mut doc, a) = doc_with_piece();
        let b = Furniture {
            id: doc.new_furniture_id(),
            catalog: "box".into(),
            position: Point2::new(300.0, 100.0),
            ..Furniture::default()
        };
        let b_id = b.id.into();
        doc.execute(Command::insert(b)).unwrap();
        let group = group(&mut doc, &[a, b_id], "Mesa e cadeiras").unwrap();
        assert_eq!(doc.home().furniture.len(), 1);
        let g = doc.home().piece(group).unwrap();
        assert_eq!(g.children.len(), 2);
        assert!((g.width - 270.0).abs() < 1e-6, "{}", g.width);
        // Copying a group renumbers its pieces.
        let copy = apply(
            &mut doc,
            &[group.into()],
            Transform::Translate {
                dx: 0.0,
                dy: 500.0,
                dz: 0.0,
            },
            true,
        )
        .unwrap();
        let ids: Vec<_> = doc
            .home()
            .furniture
            .iter()
            .flat_map(Furniture::flatten)
            .map(|f| f.id)
            .collect();
        let mut unique = ids.clone();
        unique.sort_by_key(|i| i.0);
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "no duplicated ids");
        let ElementId::Furniture(copy) = copy[0] else {
            panic!()
        };
        assert_eq!(ungroup(&mut doc, copy).unwrap().len(), 2);
        assert_eq!(doc.home().furniture.len(), 3);
        assert!(group_err(&mut doc, a));

        // Rooms: the pool drawn last stays on top of the deck.
        let mut doc = Document::default();
        for name in ["Piscina", "Deck"] {
            let room = Room::new(
                doc.new_room_id(),
                name,
                vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(100.0, 0.0),
                    Point2::new(0.0, 100.0),
                ],
            );
            doc.execute(Command::insert(room)).unwrap();
        }
        let pool = doc.home().rooms[0].id;
        reorder(&mut doc, &[pool.into()], true).unwrap();
        let names: Vec<_> = doc.home().rooms.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Deck", "Piscina"]);
        reorder(&mut doc, &[pool.into()], false).unwrap();
        assert_eq!(doc.home().rooms[0].name, "Piscina");
        doc.undo().unwrap();
        assert_eq!(doc.home().rooms[1].name, "Piscina");
    }

    fn group_err(doc: &mut Document, id: ElementId) -> bool {
        group(doc, &[id], "").is_err()
    }
}
