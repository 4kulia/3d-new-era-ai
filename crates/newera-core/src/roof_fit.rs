//! Walls, glass and panels that follow the roof above them: under an
//! A-frame or a shed roof the top of a wall is a slope or a peak, and a
//! glass gable is a triangle. Nobody should compute those by hand.
//!
//! An element marked with [`ROOF_FIT_KEY`] keeps following: whenever the
//! roof changes, [`crate::Document::execute`] fits it again in the same
//! undo step.

use crate::command::Command;
use crate::elements::Wall;
use crate::error::{CoreError, CoreResult};
use crate::furniture::{Furniture, SolidShape};
use crate::geometry::Point2;
use crate::home::Home;
use crate::ids::{ElementId, WallId};

/// Property marking an element that follows the roof; its value is the
/// lowest height (cm) a roof surface must have to count, so floors and
/// counters never do.
pub const ROOF_FIT_KEY: &str = "roof:fit";

/// Default lowest roof height considered, cm: any sloping piece above the floor.
pub const ROOF_FIT_ABOVE: f64 = 5.0;

/// Sloped surfaces that can be over something: tilted pieces (roof panels,
/// rafters) of the home.
fn slopes(home: &Home) -> Vec<&Furniture> {
    home.furniture
        .iter()
        .flat_map(Furniture::visible_leaves)
        // Roof panels, not rafters or braces: a surface is wide both ways.
        .filter(|f| {
            !f.is_opening() && (f.pitch != 0.0 || f.roll != 0.0) && f.width.min(f.depth) >= 30.0
        })
        .collect()
}

/// Height of the lowest roof surface over `p`, cm above the storey floor,
/// counting only surfaces at least `above` high.
pub fn roof_height_at(home: &Home, p: Point2, above: f64) -> Option<f64> {
    lowest(&slopes(home), p, above)
}

fn lowest(slopes: &[&Furniture], p: Point2, above: f64) -> Option<f64> {
    slopes
        .iter()
        .filter_map(|f| f.vertical_range_at(p).map(|range| range.0))
        .filter(|h| *h >= above)
        .min_by(f64::total_cmp)
}

/// Heights along `a`→`b` where a roof is above: `(distance, height)` at the
/// corners of the profile only (straight stretches collapsed), or `None` when
/// no part of the segment is under a roof. Stretches without a roof take
/// `fallback`.
fn profile(
    slopes: &[&Furniture],
    a: Point2,
    b: Point2,
    above: f64,
    fallback: f64,
) -> Option<Vec<(f64, f64)>> {
    let length = a.distance(b);
    if length < 1.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = (length.ceil() as usize).clamp(2, 4000);
    // A selected roof can descend below the reference near its eaves. Keep
    // following that surface there; falling back to the original wall height
    // would create tall spikes through the roof. Entirely low panels remain
    // excluded, and a segment wholly below the reference is not fitted.
    let eligible: Vec<_> = slopes
        .iter()
        .copied()
        .filter(|piece| piece.height_range().1 >= above)
        .collect();
    let mut any = false;
    let samples: Vec<(f64, f64)> = (0..=n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f64 / n as f64;
            let p = Point2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
            let h = lowest(&eligible, p, f64::NEG_INFINITY);
            any |= h.is_some_and(|height| height >= above);
            (t * length, h.unwrap_or(fallback))
        })
        .collect();
    if !any {
        return None;
    }
    // Keep the corners: Douglas–Peucker with a few millimeters of tolerance.
    let mut corners = Vec::new();
    simplify(&samples, &mut corners);
    corners.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-9);
    Some(corners)
}

/// Douglas–Peucker: the points where the line bends more than 3 mm.
fn simplify(points: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    let (first, last) = (points[0], points[points.len() - 1]);
    let (mut worst, mut at) = (0.0, 0);
    for (i, p) in points
        .iter()
        .enumerate()
        .skip(1)
        .take(points.len().saturating_sub(2))
    {
        let t = (p.0 - first.0) / (last.0 - first.0).max(1e-9);
        let d = (p.1 - (first.1 + (last.1 - first.1) * t)).abs();
        if d > worst {
            worst = d;
            at = i;
        }
    }
    if worst > 0.3 {
        simplify(&points[..=at], out);
        simplify(&points[at..], out);
    } else if out.last() != Some(&first) {
        out.push(first);
        out.push(last);
    } else {
        out.push(last);
    }
}

fn fits_wall(wall: &Wall, slopes: &[&Furniture], above: f64) -> Option<Vec<Wall>> {
    if wall.is_arc() {
        return None;
    }
    let corners = profile(slopes, wall.start, wall.end, above, wall.height)?;
    let length = wall.start.distance(wall.end);
    let point = |s: f64| {
        let t = s / length;
        Point2::new(
            wall.start.x + (wall.end.x - wall.start.x) * t,
            wall.start.y + (wall.end.y - wall.start.y) * t,
        )
    };
    let mut marked = wall.clone();
    marked
        .properties
        .insert(ROOF_FIT_KEY.into(), format!("{above}"));
    Some(
        corners
            .windows(2)
            .map(|pair| Wall {
                start: point(pair[0].0),
                end: point(pair[1].0),
                height: pair[0].1.max(1.0),
                height_at_end: Some(pair[1].1.max(1.0)),
                ..marked.clone()
            })
            .collect(),
    )
}

fn fits_piece(piece: &Furniture, slopes: &[&Furniture], above: f64) -> Option<Furniture> {
    if piece.is_opening() || !piece.children.is_empty() || piece.pitch != 0.0 || piece.roll != 0.0 {
        return None;
    }
    let (a, b) = (
        piece.to_plan((-piece.width / 2.0, 0.0)),
        piece.to_plan((piece.width / 2.0, 0.0)),
    );
    let fallback = piece.elevation + piece.height;
    let corners = profile(slopes, a, b, above, fallback)?;
    let top = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max) - piece.elevation;
    if top <= 1.0 {
        return None;
    }
    let mut fitted = piece.clone();
    fitted
        .properties
        .insert(ROOF_FIT_KEY.into(), format!("{above}"));
    fitted.height = top;
    let flat = corners.iter().all(|c| (c.1 - corners[0].1).abs() < 0.3);
    fitted.shape = if flat {
        None
    } else {
        // Cross-section across the width, bottom edge then the roof line back.
        let mut ring = vec![[-piece.width / 2.0, 0.0], [piece.width / 2.0, 0.0]];
        ring.extend(
            corners
                .iter()
                .rev()
                .map(|(s, h)| [s - piece.width / 2.0, (h - piece.elevation).max(0.0)]),
        );
        Some(SolidShape::Profile(ring))
    };
    Some(fitted)
}

/// Commands that fit `ids` (walls and pieces) to the roof above them, with
/// ids for walls split at a ridge from `next_wall_id`. Elements with no roof
/// over them are left out.
pub fn fit_commands(
    home: &Home,
    ids: &[ElementId],
    above: f64,
    next_wall_id: &mut dyn FnMut() -> WallId,
) -> Vec<Command> {
    let view = home.level_view(home.current_level());
    let slopes = slopes(&view);
    let mut commands = Vec::new();
    for id in ids {
        match id {
            ElementId::Wall(w) => {
                let Some(wall) = home.wall(*w) else { continue };
                let Some(parts) = fits_wall(wall, &slopes, above) else {
                    continue;
                };
                for (k, mut part) in parts.into_iter().enumerate() {
                    if k == 0 {
                        commands.push(Command::update(part));
                    } else {
                        part.id = next_wall_id();
                        commands.push(Command::insert(part));
                    }
                }
            }
            ElementId::Furniture(f) => {
                if let Some(fitted) = home
                    .furniture
                    .iter()
                    .find(|p| p.id == *f)
                    .and_then(|p| fits_piece(p, &slopes, above))
                {
                    commands.push(Command::update(fitted));
                }
            }
            _ => {}
        }
    }
    commands
}

/// Fits walls and pieces to the roof above them as one undoable step; they
/// keep following it. Returns how many elements changed.
///
/// # Errors
/// When none of them has a roof above.
pub fn fit_to_roof(doc: &mut crate::Document, ids: &[ElementId], above: f64) -> CoreResult<usize> {
    let home = doc.home().clone();
    let mut fresh = home.clone();
    let commands = fit_commands(&home, ids, above, &mut || fresh.new_wall_id());
    if commands.is_empty() {
        return Err(CoreError::InvalidGeometry(
            "nothing selected has a roof above it (roof surfaces lower than the limit don't count)"
                .into(),
        ));
    }
    let count = commands.len();
    // Take the same ids from the document itself.
    let used = commands
        .iter()
        .filter(|c| matches!(c, Command::Insert { .. }))
        .count();
    for _ in 0..used {
        doc.new_wall_id();
    }
    doc.execute(Command::Batch { commands })?;
    Ok(count)
}

/// Commands that fit again everything marked to follow the roof.
pub(crate) fn refit_marked(home: &Home, next_wall_id: &mut dyn FnMut() -> WallId) -> Vec<Command> {
    let mut commands = Vec::new();
    let above_of = |p: &crate::style::Properties| {
        p.get(ROOF_FIT_KEY)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(ROOF_FIT_ABOVE)
    };
    for wall in home
        .walls
        .iter()
        .filter(|w| w.properties.contains_key(ROOF_FIT_KEY))
    {
        let before = commands.len();
        commands.extend(fit_commands(
            home,
            &[ElementId::Wall(wall.id)],
            above_of(&wall.properties),
            next_wall_id,
        ));
        // Unchanged walls need no command.
        if commands.len() == before + 1
            && let Some(Command::Update { element, .. }) = commands.last()
            && element == &crate::elements::Element::Wall(wall.clone())
        {
            commands.pop();
        }
    }
    for piece in home
        .furniture
        .iter()
        .filter(|f| f.properties.contains_key(ROOF_FIT_KEY))
    {
        let fitted = fit_commands(
            home,
            &[ElementId::Furniture(piece.id)],
            above_of(&piece.properties),
            next_wall_id,
        );
        if let Some(Command::Update { element, .. }) = fitted.first()
            && element != &crate::elements::Element::Furniture(piece.clone())
        {
            commands.extend(fitted);
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FurnitureId;

    /// A roof panel from `(x, y0)` rising to `(x, y1)` at heights z0 → z1,
    /// `w` wide along x.
    fn slope(id: u64, x: f64, (y0, z0): (f64, f64), (y1, z1): (f64, f64), w: f64) -> Furniture {
        let run = y1 - y0;
        let rise = z1 - z0;
        let length = run.hypot(rise);
        let pitch = rise.atan2(run).to_degrees();
        Furniture {
            id: FurnitureId(id),
            catalog: "box".into(),
            name: "Água".into(),
            position: Point2::new(x, f64::midpoint(y0, y1)),
            // Centered at mid height; `depth` along the slope.
            elevation: f64::midpoint(z0, z1) - 1.0,
            width: w,
            depth: length,
            height: 2.0,
            pitch: -pitch,
            ..Furniture::default()
        }
    }

    #[test]
    fn a_wall_under_an_a_frame_becomes_two_slopes_meeting_at_the_ridge() {
        let mut home = Home::default();
        // Ridge along x at y = 200, 400 cm high; eaves at y = 0 and 400, 0 cm.
        home.furniture
            .push(slope(1, 300.0, (0.0, 0.0), (200.0, 400.0), 600.0));
        home.furniture
            .push(slope(2, 300.0, (400.0, 0.0), (200.0, 400.0), 600.0));
        // Probe the surfaces: halfway up, 200 cm.
        let h = roof_height_at(&home, Point2::new(300.0, 100.0), 10.0).unwrap();
        assert!((h - 199.0).abs() < 3.0, "{h}");
        let mut wall = Wall::new(
            WallId(10),
            Point2::new(100.0, 50.0),
            Point2::new(100.0, 350.0),
        );
        wall.height = 250.0;
        home.walls.push(wall);
        let mut next = 11;
        let commands = fit_commands(&home, &[ElementId::Wall(WallId(10))], 10.0, &mut || {
            next += 1;
            WallId(next)
        });
        let walls: Vec<Wall> = commands
            .into_iter()
            .filter_map(|c| match c {
                Command::Update {
                    element: crate::elements::Element::Wall(w),
                    ..
                }
                | Command::Insert {
                    element: crate::elements::Element::Wall(w),
                    ..
                } => Some(w),
                _ => None,
            })
            .collect();
        assert_eq!(walls.len(), 2, "{walls:?}");
        // Split right under the ridge, rising to about 400 there.
        assert!((walls[0].end.y - 200.0).abs() < 2.0, "{:?}", walls[0].end);
        assert!(walls[0].height < walls[0].height_at_end.unwrap());
        assert!((walls[0].height_at_end.unwrap() - 399.0).abs() < 4.0);
        assert!(
            walls
                .iter()
                .all(|w| w.properties.contains_key(ROOF_FIT_KEY))
        );

        // A glass gable panel across the same span becomes a triangle.
        let mut glass = Furniture {
            id: FurnitureId(20),
            catalog: "panel".into(),
            name: "Vidro".into(),
            position: Point2::new(500.0, 200.0),
            angle: 90.0,
            width: 400.0,
            depth: 2.0,
            height: 100.0,
            ..Furniture::default()
        };
        glass.opacity = Some(0.35);
        home.furniture.push(glass);
        let commands = fit_commands(
            &home,
            &[ElementId::Furniture(FurnitureId(20))],
            10.0,
            &mut || WallId(99),
        );
        let Some(Command::Update {
            element: crate::elements::Element::Furniture(fitted),
            ..
        }) = commands.first()
        else {
            panic!("{commands:?}")
        };
        let Some(SolidShape::Profile(ring)) = &fitted.shape else {
            panic!("{fitted:?}")
        };
        assert!((fitted.height - 399.0).abs() < 4.0, "{}", fitted.height);
        // Bottom corners, then the roof line: low, peak, low.
        assert!(ring.len() >= 5, "{ring:?}");
        // Its box reaches into the slopes, but it was cut to fit under them.
        let mut shaped = home.clone();
        shaped.furniture[2] = fitted.clone();
        // The original wall has not been fitted yet: it cuts through both
        // slopes. Only the glass was fitted in this copy; exact heights expose
        // the two pre-existing wall intersections without flagging the glass.
        assert_eq!(
            crate::check_layout(&shaped),
            vec![
                crate::analysis::Issue::InWall(FurnitureId(1), WallId(10)),
                crate::analysis::Issue::InWall(FurnitureId(2), WallId(10)),
            ]
        );
        // Fitted for real, it keeps following: raising the roof raises the wall.
        let mut doc = crate::Document::new(home.clone());
        fit_to_roof(&mut doc, &[ElementId::Wall(WallId(10))], 10.0).unwrap();
        let peak = |doc: &crate::Document| {
            doc.home()
                .walls
                .iter()
                .filter(|w| w.properties.contains_key(ROOF_FIT_KEY))
                .map(|w| w.height.max(w.height_at_end.unwrap_or(0.0)))
                .fold(0.0, f64::max)
        };
        let before = peak(&doc);
        let mut higher = doc.home().furniture[0].clone();
        higher.elevation += 50.0;
        doc.execute(Command::update(higher)).unwrap();
        assert!(peak(&doc) > before + 10.0, "{} → {}", before, peak(&doc));
        doc.undo().unwrap();
        assert!((peak(&doc) - before).abs() < 1e-9, "one undo step");
        // Nothing overhead: nothing to do.
        let mut away = Wall::new(
            WallId(30),
            Point2::new(900.0, 0.0),
            Point2::new(1000.0, 0.0),
        );
        away.height = 250.0;
        home.walls.push(away);
        assert_eq!(
            fit_commands(&home, &[ElementId::Wall(WallId(30))], 10.0, &mut || WallId(
                98
            )),
            [] as [crate::command::Command; 0]
        );
    }
}
