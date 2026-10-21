//! Composite edits built from [`Command`]s, shared by the editor and MCP.

use crate::command::Command;
use crate::document::Document;
use crate::elements::{Dimension, Element, Wall};
use crate::error::{CoreError, CoreResult};
use crate::geometry::Point2;
use crate::ids::{ElementId, WallId};
use crate::joins::JOIN_TOLERANCE;

/// How far (cm) a moved door or window looks for a wall to sit in.
pub const OPENING_REACH: f64 = 60.0;

/// Splits a wall at parameter `t` (0..1) into two joined walls. Returns the
/// id of the new second half.
pub fn split_wall(doc: &mut Document, id: WallId, t: f64) -> CoreResult<WallId> {
    let wall = doc
        .home()
        .wall(id)
        .cloned()
        .ok_or(CoreError::NotFound(id.into()))?;
    if !(t > 0.0 && t < 1.0) {
        return Err(CoreError::InvalidGeometry(
            "split point must be strictly inside the wall (0 < t < 1)".into(),
        ));
    }
    let at = wall.point_at(t);
    let new_id = doc.new_wall_id();
    let first = Wall {
        end: at,
        arc_extent: wall.arc_extent.map(|a| a * t),
        ..wall.clone()
    };
    let second = Wall {
        id: new_id,
        start: at,
        arc_extent: wall.arc_extent.map(|a| a * (1.0 - t)),
        ..wall
    };
    let mut commands = vec![Command::update(first), Command::insert(second)];
    // Both halves keep the products on their sides.
    commands.extend(crate::finishes::copied(doc.home(), id, new_id, false));
    doc.execute(Command::Batch { commands })?;
    Ok(new_id)
}

/// Joins walls that run along the same line into a single one.
///
/// The undo of [`split_wall`], and the fix for a wall that was drawn in
/// pieces: an outer wall interrupted where a partition meets it, a stretch
/// imported as a dozen segments, the same wall drawn twice. The longest wall
/// keeps its id and its build — thickness, height, type, finishes — and grows
/// to span everything given; the others are deleted in the same step. Doors
/// and windows stay where they are: they sit in whatever wall is under them.
///
/// The walls must be straight, on the same storey, and their centerlines must
/// run inside one another. A gap between them is closed, so two stretches
/// with a doorway between them become one wall with no doorway.
pub fn merge_walls(doc: &mut Document, ids: &[WallId]) -> CoreResult<WallId> {
    let mut walls: Vec<Wall> = Vec::new();
    for id in ids {
        let wall = doc
            .home()
            .wall(*id)
            .cloned()
            .ok_or(CoreError::NotFound((*id).into()))?;
        if walls.iter().any(|w| w.id == wall.id) {
            continue;
        }
        if wall.is_arc() {
            return Err(CoreError::InvalidGeometry(format!(
                "{id} is curved: only straight walls can be joined"
            )));
        }
        walls.push(wall);
    }
    if walls.len() < 2 {
        return Err(CoreError::InvalidGeometry(
            "joining walls needs at least two of them".into(),
        ));
    }
    if walls.iter().any(|w| w.level != walls[0].level) {
        return Err(CoreError::InvalidGeometry(
            "those walls are on different storeys".into(),
        ));
    }

    // The line they all have to be on: the two ends furthest apart.
    let ends: Vec<Point2> = walls.iter().flat_map(|w| [w.start, w.end]).collect();
    let (from, to) = ends
        .iter()
        .flat_map(|a| ends.iter().map(move |b| (*a, *b)))
        .max_by(|(a1, b1), (a2, b2)| a1.distance(*b1).total_cmp(&a2.distance(*b2)))
        .ok_or_else(|| CoreError::InvalidGeometry("walls with no length".into()))?;
    let span = from.distance(to);
    if span < 1.0 {
        return Err(CoreError::InvalidGeometry("walls with no length".into()));
    }
    let dir = ((to.x - from.x) / span, (to.y - from.y) / span);
    let across = |p: Point2| {
        let (dx, dy) = (p.x - from.x, p.y - from.y);
        (-dx * dir.1 + dy * dir.0).abs()
    };
    // Every centerline has to run inside the thinnest of them.
    let reach = walls
        .iter()
        .map(|w| w.thickness / 2.0)
        .fold(f64::MAX, f64::min)
        .max(JOIN_TOLERANCE);
    if let Some(off) = ends.iter().find(|p| across(**p) > reach) {
        return Err(CoreError::InvalidGeometry(format!(
            "those walls are not on the same line: one end is {:.0} cm off it",
            across(*off)
        )));
    }

    // The longest one keeps its id and everything else about it.
    let keep = walls
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.length().total_cmp(&b.length()))
        .map(|(i, _)| i)
        .unwrap_or_default();
    // …including which way it faces, so its two finishes stay on their sides.
    let forward = {
        let w = &walls[keep];
        (w.end.x - w.start.x) * dir.0 + (w.end.y - w.start.y) * dir.1 >= 0.0
    };
    let (start, end) = if forward { (from, to) } else { (to, from) };
    // A sloping wall keeps the heights of the ends that survive.
    let height_at = |p: Point2| {
        walls
            .iter()
            .flat_map(|w| {
                [
                    (w.start, w.height),
                    (w.end, w.height_at_end.unwrap_or(w.height)),
                ]
            })
            .min_by(|(a, _), (b, _)| a.distance(p).total_cmp(&b.distance(p)))
            .map_or(walls[keep].height, |(_, h)| h)
    };
    let (height, at_end) = (height_at(start), height_at(end));
    let merged = Wall {
        start,
        end,
        height,
        height_at_end: ((at_end - height).abs() > 1e-6).then_some(at_end),
        ..walls[keep].clone()
    };
    let id = merged.id;
    let mut commands = vec![Command::update(merged)];
    commands.extend(
        walls
            .iter()
            .filter(|w| w.id != id)
            .map(|w| Command::remove(ElementId::from(w.id))),
    );
    doc.execute(Command::Batch { commands })?;
    Ok(id)
}

/// Moves elements by `(dx, dy)` cm as one undoable step.
///
/// With `drag_joined`, endpoints of other walls that were joined to a moved
/// wall follow it, so rooms stay closed — the behavior users expect when
/// dragging walls in the editor.
pub fn translate(
    doc: &mut Document,
    ids: &[ElementId],
    dx: f64,
    dy: f64,
    drag_joined: bool,
) -> CoreResult<()> {
    let home = doc.home();
    let shift = |p: Point2| Point2::new(p.x + dx, p.y + dy);
    let mut commands = Vec::with_capacity(ids.len());
    let mut moved_points = Vec::new();

    for id in ids {
        let element = home.element(*id).ok_or(CoreError::NotFound(*id))?;
        let moved = match element {
            Element::Wall(w) => {
                moved_points.extend([w.start, w.end]);
                Element::Wall(Wall {
                    start: shift(w.start),
                    end: shift(w.end),
                    ..w
                })
            }
            Element::Room(mut r) => {
                r.points = r.points.into_iter().map(shift).collect();
                Element::Room(r)
            }
            Element::Dimension(d) => Element::Dimension(Dimension {
                start: shift(d.start),
                end: shift(d.end),
                ..d
            }),
            Element::Label(mut l) => {
                l.position = shift(l.position);
                Element::Label(l)
            }
            Element::Level(level) => Element::Level(level),
            Element::Polyline(mut p) => {
                p.points = p.points.into_iter().map(shift).collect();
                Element::Polyline(p)
            }
            Element::Furniture(mut f) => {
                f.translate(dx, dy);
                // Doors and windows stay seated in the nearest wall.
                if f.is_opening()
                    && let Some((wall_id, along)) =
                        nearest_wall(&home.level_view(f.level), f.position, OPENING_REACH)
                    && let Some(wall) = home.wall(wall_id)
                {
                    crate::furniture::reseat_in_wall(&mut f, wall, along);
                }
                Element::Furniture(f)
            }
        };
        commands.push(Command::Update { element: moved });
    }

    if drag_joined {
        let moved_levels: Vec<_> = ids
            .iter()
            .filter_map(|id| home.element(*id))
            .map(|e| home.resolve_level(e.level()))
            .collect();
        for wall in home
            .walls
            .iter()
            .filter(|w| !ids.contains(&w.id.into()))
            .filter(|w| moved_levels.contains(&home.resolve_level(w.level)))
        {
            let near = |p: Point2| moved_points.iter().any(|q| q.distance(p) <= JOIN_TOLERANCE);
            let (s, e) = (near(wall.start), near(wall.end));
            if s || e {
                let updated = Wall {
                    start: if s { shift(wall.start) } else { wall.start },
                    end: if e { shift(wall.end) } else { wall.end },
                    ..wall.clone()
                };
                commands.push(Command::update(updated));
            }
        }
    }
    doc.execute(Command::Batch { commands })
}

/// Moves one wall endpoint, dragging every other wall endpoint joined to it.
pub fn move_wall_point(
    doc: &mut Document,
    id: WallId,
    at_start: bool,
    to: Point2,
) -> CoreResult<()> {
    let home = doc.home();
    let wall = home.wall(id).ok_or(CoreError::NotFound(id.into()))?;
    let from = if at_start { wall.start } else { wall.end };
    let level = wall.level;
    let commands = home
        .walls
        .iter()
        .filter(|w| home.on_level(w.level, level))
        .filter_map(|w| {
            let s = w.start.distance(from) <= JOIN_TOLERANCE;
            let e = w.end.distance(from) <= JOIN_TOLERANCE;
            (s || e).then(|| {
                Command::update(Wall {
                    start: if s { to } else { w.start },
                    end: if e { to } else { w.end },
                    ..w.clone()
                })
            })
        })
        .collect();
    doc.execute(Command::Batch { commands })
}

/// A dimension measuring a wall, placed on the side facing away from the
/// middle of the drawing (outside the house), `gap` cm beyond the wall face.
pub fn wall_dimension(doc: &mut Document, id: WallId, gap: f64) -> CoreResult<Dimension> {
    let home = doc.home();
    let wall = home.wall(id).ok_or(CoreError::NotFound(id.into()))?.clone();
    let center = home.bounds().map_or(wall.start, |(min, max)| {
        Point2::new(min.x.midpoint(max.x), min.y.midpoint(max.y))
    });
    let mid = wall.point_at(0.5);
    let len = wall.start.distance(wall.end).max(1e-9);
    // Left normal in plan axes (y down), matching Dimension::offset.
    let normal = (
        (wall.end.y - wall.start.y) / len,
        -(wall.end.x - wall.start.x) / len,
    );
    let outward = (mid.x - center.x) * normal.0 + (mid.y - center.y) * normal.1;
    let sign = if outward >= 0.0 { 1.0 } else { -1.0 };
    Ok(Dimension {
        id: doc.new_dimension_id(),
        start: wall.start,
        end: wall.end,
        offset: sign * (wall.thickness / 2.0 + gap),
        level: wall.level,
        ..Dimension::default()
    })
}

/// Which line of a wall a dimension measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WallSide {
    /// Along the centerline, end to end.
    #[default]
    Axis,
    /// Along the face away from the middle of the drawing.
    Outer,
    /// Along the face toward the middle of the drawing.
    Inner,
}

/// The two ends of a wall face (as joined with its neighbors) and the sign of
/// the offset that points away from the wall on that side.
fn wall_face(
    home: &crate::home::Home,
    id: WallId,
    side: WallSide,
) -> CoreResult<(Point2, Point2, f64)> {
    let wall = home.wall(id).ok_or(CoreError::NotFound(id.into()))?.clone();
    let view = home.level_view(wall.level);
    let index = view
        .walls
        .iter()
        .position(|w| w.id == id)
        .ok_or(CoreError::NotFound(id.into()))?;
    let outline = view.wall_outlines().swap_remove(index);
    let len = wall.start.distance(wall.end).max(1e-9);
    let dir = (
        (wall.end.x - wall.start.x) / len,
        (wall.end.y - wall.start.y) / len,
    );
    // Left normal (plan axes), matching Dimension::offset.
    let normal = (dir.1, -dir.0);
    let center = home.bounds().map_or(wall.start, |(min, max)| {
        Point2::new(min.x.midpoint(max.x), min.y.midpoint(max.y))
    });
    let mid = wall.point_at(0.5);
    let outward = if (mid.x - center.x) * normal.0 + (mid.y - center.y) * normal.1 >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let sign = match side {
        WallSide::Axis | WallSide::Outer => outward,
        WallSide::Inner => -outward,
    };
    if side == WallSide::Axis {
        return Ok((wall.start, wall.end, sign));
    }
    let target = sign * wall.thickness / 2.0;
    let Some((lo, hi)) = face_span(&wall, &outline, sign) else {
        return Err(CoreError::InvalidGeometry(format!(
            "{id} has no {side:?} face to measure"
        )));
    };
    let at = |s: f64| {
        Point2::new(
            wall.start.x + dir.0 * s + normal.0 * target,
            wall.start.y + dir.1 * s + normal.1 * target,
        )
    };
    Ok((at(lo), at(hi), sign))
}

/// Where one face of a straight wall runs, as joined with its neighbours:
/// `(from, to)` cm along the wall from its start (beyond `0..length` where a
/// corner adds to it, inside where a joining wall takes from it). `sign` is
/// `1.0` for the left side of start → end, `-1.0` for the right; `outline`
/// is the wall's joined outline ([`crate::Home::wall_outlines`] of its
/// storey). `None` when the outline has no such face.
pub(crate) fn face_span(wall: &Wall, outline: &[Point2], sign: f64) -> Option<(f64, f64)> {
    let len = wall.start.distance(wall.end).max(1e-9);
    let dir = (
        (wall.end.x - wall.start.x) / len,
        (wall.end.y - wall.start.y) / len,
    );
    let normal = (dir.1, -dir.0);
    let along = |p: Point2| (p.x - wall.start.x) * dir.0 + (p.y - wall.start.y) * dir.1;
    let across = |p: Point2| (p.x - wall.start.x) * normal.0 + (p.y - wall.start.y) * normal.1;
    let target = sign * wall.thickness / 2.0;
    let on_face: Vec<f64> = outline
        .iter()
        .filter(|p| (across(**p) - target).abs() < 0.5)
        .map(|p| along(*p))
        .collect();
    let (lo, hi) = on_face
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    (on_face.len() >= 2 && hi - lo >= 0.1).then_some((lo, hi))
}

/// A dimension along one line of a wall, `gap` cm off the measured line on
/// its own side (outside for the outer face and axis, inside for the inner).
pub fn wall_side_dimension(
    doc: &mut Document,
    id: WallId,
    side: WallSide,
    gap: f64,
) -> CoreResult<Dimension> {
    let level = doc.home().wall(id).and_then(|w| w.level);
    let (start, end, sign) = wall_face(doc.home(), id, side)?;
    let extra = if side == WallSide::Axis {
        doc.home().wall(id).map_or(0.0, |w| w.thickness / 2.0)
    } else {
        0.0
    };
    Ok(Dimension {
        id: doc.new_dimension_id(),
        start,
        end,
        offset: sign * (gap + extra),
        level,
        ..Dimension::default()
    })
}

/// A chain of dimensions along a wall face, split at every door and window
/// edge: wall piece, opening, wall piece…
pub fn wall_chain_dimensions(
    doc: &mut Document,
    id: WallId,
    side: WallSide,
    gap: f64,
) -> CoreResult<Vec<Dimension>> {
    let home = doc.home();
    let wall = home.wall(id).ok_or(CoreError::NotFound(id.into()))?.clone();
    let side = if side == WallSide::Axis {
        WallSide::Outer
    } else {
        side
    };
    let (start, end, sign) = wall_face(home, id, side)?;
    let view = home.level_view(wall.level);
    let cuts = view
        .walls
        .iter()
        .position(|w| w.id == id)
        .map(|i| view.wall_cuts().swap_remove(i))
        .unwrap_or_default();
    let len = wall.start.distance(wall.end).max(1e-9);
    let dir = (
        (wall.end.x - wall.start.x) / len,
        (wall.end.y - wall.start.y) / len,
    );
    let along = |p: Point2| (p.x - wall.start.x) * dir.0 + (p.y - wall.start.y) * dir.1;
    let (a, b) = (along(start), along(end));
    let mut stops = vec![a, b];
    for cut in &cuts {
        stops.extend([cut.from, cut.to].into_iter().filter(|s| *s > a && *s < b));
    }
    stops.sort_by(f64::total_cmp);
    stops.dedup_by(|x, y| (*x - *y).abs() < 0.5);
    let at = |s: f64| {
        let t = (s - a) / (b - a).max(1e-9);
        Point2::new(
            start.x + (end.x - start.x) * t,
            start.y + (end.y - start.y) * t,
        )
    };
    let level = wall.level;
    let pairs: Vec<(f64, f64)> = stops.windows(2).map(|w| (w[0], w[1])).collect();
    Ok(pairs
        .into_iter()
        .map(|(s0, s1)| Dimension {
            id: doc.new_dimension_id(),
            start: at(s0),
            end: at(s1),
            offset: sign * gap,
            level,
            ..Dimension::default()
        })
        .collect())
}

/// Clear width and depth of a room (its bounding box), crossing inside it.
pub fn room_dimensions(doc: &mut Document, id: crate::ids::RoomId) -> CoreResult<Vec<Dimension>> {
    let room = doc
        .home()
        .room(id)
        .ok_or(CoreError::NotFound(id.into()))?
        .clone();
    let (lo, hi) = room.points.iter().fold(
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
    let y = lo.y.midpoint(hi.y) + (hi.y - lo.y) * 0.25;
    let x = lo.x.midpoint(hi.x) + (hi.x - lo.x) * 0.25;
    Ok(vec![
        Dimension {
            id: doc.new_dimension_id(),
            start: Point2::new(lo.x, y),
            end: Point2::new(hi.x, y),
            level: room.level,
            ..Dimension::default()
        },
        Dimension {
            id: doc.new_dimension_id(),
            start: Point2::new(x, lo.y),
            end: Point2::new(x, hi.y),
            level: room.level,
            ..Dimension::default()
        },
    ])
}

/// Nearest straight wall to `p` within `max_distance` cm of its centerline,
/// with the distance along it from its start.
pub fn nearest_wall(
    home: &crate::home::Home,
    p: Point2,
    max_distance: f64,
) -> Option<(WallId, f64)> {
    home.walls
        .iter()
        .filter(|w| !w.is_arc())
        .filter_map(|w| {
            let len = w.start.distance(w.end);
            if len < 1e-9 {
                return None;
            }
            let dir = ((w.end.x - w.start.x) / len, (w.end.y - w.start.y) / len);
            let along = ((p.x - w.start.x) * dir.0 + (p.y - w.start.y) * dir.1).clamp(0.0, len);
            let foot = Point2::new(w.start.x + dir.0 * along, w.start.y + dir.1 * along);
            let distance = foot.distance(p);
            (distance <= max_distance).then_some((w.id, along, distance))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(id, along, _)| (id, along))
}

/// Re-seats doors and windows in the wall nearest to them (after a move).
pub fn snap_openings(doc: &mut Document, ids: &[ElementId], max_distance: f64) -> CoreResult<()> {
    let home = doc.home();
    let commands: Vec<Command> = ids
        .iter()
        .filter_map(|id| match home.element(*id) {
            Some(Element::Furniture(f)) if f.is_opening() => Some(f),
            _ => None,
        })
        .filter_map(|mut f| {
            let (wall_id, along) =
                nearest_wall(&home.level_view(f.level), f.position, max_distance)?;
            let wall = home.wall(wall_id)?;
            crate::furniture::reseat_in_wall(&mut f, wall, along);
            Some(Command::update(f))
        })
        .collect();
    if commands.is_empty() {
        return Ok(());
    }
    doc.execute(Command::Batch { commands })
}

/// Adds a storey on top of the highest one and selects it. The first call on
/// a single-level house also creates the ground level ("Térreo") for the
/// existing elements. Returns the new level's id.
///
/// # Panics
/// Never: the level list is non-empty once the ground level exists.
pub fn add_level(
    doc: &mut Document,
    name: Option<String>,
    height: Option<f64>,
) -> CoreResult<crate::ids::LevelId> {
    use crate::elements::Level;
    let mut commands = Vec::new();
    let mut levels: Vec<Level> = doc.home().levels.clone();
    if levels.is_empty() {
        let ground = Level {
            id: doc.new_level_id(),
            name: "Térreo".to_owned(),
            elevation: 0.0,
            height: crate::elements::Wall::DEFAULT_HEIGHT,
            floor_thickness: Level::DEFAULT_FLOOR_THICKNESS,
            ..Level::default()
        };
        levels.push(ground.clone());
        commands.push(Command::insert(ground));
    }
    let top = levels
        .iter()
        .max_by(|a, b| a.elevation.total_cmp(&b.elevation))
        .expect("at least one level");
    let floor = Level::DEFAULT_FLOOR_THICKNESS;
    let count = levels.len();
    let level = Level {
        id: doc.new_level_id(),
        name: name.unwrap_or_else(|| format!("{count}º andar")),
        elevation: top.elevation + top.height + floor,
        height: height.unwrap_or(Level::DEFAULT_HEIGHT),
        floor_thickness: floor,
        ..Level::default()
    };
    let id = level.id;
    commands.push(Command::insert(level));
    doc.execute(Command::Batch { commands })?;
    doc.select_level(Some(id));
    Ok(id)
}

/// Deletes a storey and everything on it, as one undoable step.
pub fn delete_level(doc: &mut Document, id: crate::ids::LevelId) -> CoreResult<()> {
    let home = doc.home();
    if home.level(id).is_none() {
        return Err(CoreError::NotFound(id.into()));
    }
    let mut commands: Vec<Command> = home
        .elements()
        .filter(|e| !matches!(e, Element::Level(_)) && home.on_level(e.level(), Some(id)))
        .map(|e| Command::remove(e.id()))
        .collect();
    commands.push(Command::remove(id));
    doc.execute(Command::Batch { commands })?;
    let still_selected = doc
        .home()
        .selected_level
        .is_some_and(|s| doc.home().level(s).is_some());
    if !still_selected {
        let base = doc.home().base_level();
        doc.select_level(base);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::Label;

    fn doc_with_l() -> (Document, WallId, WallId) {
        let mut doc = Document::default();
        let a = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
        );
        let b = Wall::new(
            doc.new_wall_id(),
            Point2::new(400.0, 0.0),
            Point2::new(400.0, 300.0),
        );
        let (ia, ib) = (a.id, b.id);
        doc.execute(Command::Batch {
            commands: vec![Command::insert(a), Command::insert(b)],
        })
        .unwrap();
        (doc, ia, ib)
    }

    #[test]
    fn split_keeps_the_join_and_undoes_in_one_step() {
        let (mut doc, a, _) = doc_with_l();
        let second = split_wall(&mut doc, a, 0.25).unwrap();
        let home = doc.home();
        assert_eq!(home.wall(a).unwrap().end, Point2::new(100.0, 0.0));
        assert_eq!(home.wall(second).unwrap().start, Point2::new(100.0, 0.0));
        assert_eq!(home.wall(second).unwrap().end, Point2::new(400.0, 0.0));
        doc.undo().unwrap();
        assert_eq!(doc.home().walls.len(), 2);
        assert!(split_wall(&mut doc, a, 1.0).is_err());
    }

    #[test]
    fn translating_a_wall_drags_joined_walls() {
        let (mut doc, a, b) = doc_with_l();
        translate(&mut doc, &[a.into()], 0.0, -50.0, true).unwrap();
        let home = doc.home();
        assert_eq!(home.wall(a).unwrap().start, Point2::new(0.0, -50.0));
        assert_eq!(
            home.wall(b).unwrap().start,
            Point2::new(400.0, -50.0),
            "joined end followed"
        );
        assert_eq!(
            home.wall(b).unwrap().end,
            Point2::new(400.0, 300.0),
            "far end stayed"
        );
    }

    #[test]
    fn merge_puts_split_walls_back_together() {
        let (mut doc, a, _) = doc_with_l();
        let second = split_wall(&mut doc, a, 0.25).unwrap();
        let kept = merge_walls(&mut doc, &[second, a]).unwrap();
        // The longest piece keeps its id, and the wall spans the whole line.
        assert_eq!(kept, second);
        let wall = doc.home().wall(kept).unwrap();
        assert_eq!(
            (wall.start, wall.end),
            (Point2::new(0.0, 0.0), Point2::new(400.0, 0.0))
        );
        assert!(doc.home().wall(a).is_none(), "the other piece is gone");
        // And it undoes as one step.
        doc.undo().unwrap();
        assert_eq!(doc.home().walls.len(), 3);
    }

    #[test]
    fn merge_closes_the_gap_and_keeps_the_build_of_the_longest() {
        let mut doc = Document::default();
        let mut a = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(300.0, 0.0),
        );
        a.thickness = 25.0;
        let mut b = Wall::new(
            doc.new_wall_id(),
            Point2::new(380.0, 0.0),
            Point2::new(500.0, 0.0),
        );
        b.thickness = 10.0;
        let (ia, ib) = (a.id, b.id);
        doc.execute(Command::Batch {
            commands: vec![Command::insert(a), Command::insert(b)],
        })
        .unwrap();
        let kept = merge_walls(&mut doc, &[ib, ia]).unwrap();
        assert_eq!(kept, ia);
        let wall = doc.home().wall(kept).unwrap();
        assert_eq!(
            (wall.start, wall.end),
            (Point2::new(0.0, 0.0), Point2::new(500.0, 0.0))
        );
        assert!((wall.thickness - 25.0).abs() < 1e-9);
    }

    #[test]
    fn merge_refuses_walls_that_are_not_on_the_same_line() {
        let (mut doc, a, b) = doc_with_l();
        let err = merge_walls(&mut doc, &[a, b]).unwrap_err().to_string();
        assert!(err.contains("same line"), "{err}");
        assert_eq!(doc.home().walls.len(), 2, "nothing was touched");
    }

    #[test]
    fn translate_moves_any_element_kind() {
        let mut doc = Document::default();
        let label = Label {
            id: doc.new_label_id(),
            text: "A".into(),
            position: Point2::new(1.0, 2.0),
            size: 20.0,
            angle: 0.0,
            level: None,
            ..Default::default()
        };
        let id = label.id;
        doc.execute(Command::insert(label)).unwrap();
        translate(&mut doc, &[id.into()], 10.0, 20.0, false).unwrap();
        assert_eq!(
            doc.home().label(id).unwrap().position,
            Point2::new(11.0, 22.0)
        );
    }

    #[test]
    fn moving_a_corner_moves_both_walls() {
        let (mut doc, a, b) = doc_with_l();
        move_wall_point(&mut doc, a, false, Point2::new(450.0, 20.0)).unwrap();
        assert_eq!(doc.home().wall(a).unwrap().end, Point2::new(450.0, 20.0));
        assert_eq!(doc.home().wall(b).unwrap().start, Point2::new(450.0, 20.0));
    }

    #[test]
    fn levels_stack_and_views_keep_storeys_apart() {
        let (mut doc, a, _) = doc_with_l();
        let upper = add_level(&mut doc, None, None).unwrap();
        let home = doc.home();
        assert_eq!(
            home.levels.len(),
            2,
            "ground level created for existing walls"
        );
        let ground = home.base_level().unwrap();
        assert_eq!(home.selected_level, Some(upper));
        assert!((home.level(upper).unwrap().elevation - 262.0).abs() < 1e-9);

        // New wall on the upper level.
        let mut wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(100.0, 0.0),
        );
        wall.level = Some(upper);
        doc.execute(Command::insert(wall)).unwrap();
        let home = doc.home();
        assert_eq!(
            home.level_view(Some(ground)).walls.len(),
            2,
            "unassigned walls sit on the ground"
        );
        assert_eq!(home.level_view(Some(upper)).walls.len(), 1);
        assert!(home.level_view(Some(upper)).wall(a).is_none());

        delete_level(&mut doc, upper).unwrap();
        assert_eq!(
            doc.home().walls.len(),
            2,
            "upper walls removed with their level"
        );
        assert_eq!(doc.home().current_level(), Some(ground));
        doc.undo().unwrap();
        assert_eq!(
            doc.home().walls.len(),
            3,
            "deleting a level is one undo step"
        );
    }

    #[test]
    fn wall_dimensions_go_outside() {
        let (mut doc, a, b) = doc_with_l();
        // Bounds center is (200, 150): wall a (top) should be dimensioned above.
        let dim = wall_dimension(&mut doc, a, 30.0).unwrap();
        assert!(dim.offset > 0.0, "left of +x is up (-y), away from center");
        let dim_b = wall_dimension(&mut doc, b, 30.0).unwrap();
        // b goes down (+y); left normal points +x, away from the center.
        assert!(dim_b.offset > 0.0);
        assert!((dim.offset - 37.5).abs() < 1e-9);
    }

    #[test]
    fn wall_faces_chains_and_rooms_by_intent() {
        let mut doc = Document::default();
        let pts = [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)];
        let mut commands = Vec::new();
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            commands.push(Command::insert(Wall::new(
                doc.new_wall_id(),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            )));
        }
        doc.execute(Command::Batch { commands }).unwrap();
        let top = WallId(1);
        let outer = wall_side_dimension(&mut doc, top, WallSide::Outer, 40.0).unwrap();
        let inner = wall_side_dimension(&mut doc, top, WallSide::Inner, 40.0).unwrap();
        let axis = wall_side_dimension(&mut doc, top, WallSide::Axis, 40.0).unwrap();
        assert!(
            (outer.length() - 415.0).abs() < 1e-9 && outer.offset > 0.0,
            "{outer:?}"
        );
        assert!(
            (inner.length() - 385.0).abs() < 1e-9 && inner.offset < 0.0,
            "{inner:?}"
        );
        assert!((axis.length() - 400.0).abs() < 1e-9);

        let mut window = crate::Furniture {
            id: doc.new_furniture_id(),
            catalog: "window".into(),
            width: 100.0,
            depth: 15.0,
            height: 120.0,
            opening: Some(crate::Opening::default()),
            ..crate::Furniture::default()
        };
        let wall = doc.home().wall(top).unwrap().clone();
        crate::furniture::align_to_wall(&mut window, &wall, 200.0);
        doc.execute(Command::insert(window)).unwrap();
        let chain = wall_chain_dimensions(&mut doc, top, WallSide::Outer, 60.0).unwrap();
        let lengths: Vec<f64> = chain
            .iter()
            .map(|d| (d.length() * 10.0).round() / 10.0)
            .collect();
        assert_eq!(lengths, vec![157.5, 100.0, 157.5]);

        let room = crate::Room::new(
            doc.new_room_id(),
            "Sala",
            vec![
                Point2::new(7.5, 7.5),
                Point2::new(392.5, 7.5),
                Point2::new(392.5, 292.5),
                Point2::new(7.5, 292.5),
            ],
        );
        let rid = room.id;
        doc.execute(Command::insert(room)).unwrap();
        let dims = room_dimensions(&mut doc, rid).unwrap();
        assert!((dims[0].length() - 385.0).abs() < 1e-9 && (dims[1].length() - 285.0).abs() < 1e-9);
    }
}
