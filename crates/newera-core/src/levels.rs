//! Floors of multi-storey houses: room floors with stair holes.

use geo::{BooleanOps, Contains, Coord, LineString, Polygon, TriangulateEarcut};

use crate::elements::{Room, Wall};
use crate::geometry::{Point2, to_polygon};
use crate::home::Home;
use crate::ids::{LevelId, RoomId};

/// How far past its edge a room's floor is looked for a wall (cm). A point
/// this far outside the room is already in the wall, not in the room.
const PROBE: f64 = 1.0;

/// The floor under a wall is decided every this many cm along a room edge, so
/// one long edge can meet a corridor on one stretch and a bedroom on the next.
const UNDER_WALL_STEP: f64 = 20.0;

/// A floor area ready to render: outline, holes and triangles (plan cm).
#[derive(Debug, Clone, PartialEq)]
pub struct FloorShape {
    /// Room this floor belongs to.
    pub room: RoomId,
    pub exterior: Vec<Point2>,
    pub holes: Vec<Vec<Point2>>,
    pub triangles: Vec<[Point2; 3]>,
}

fn ring(points: &[Point2]) -> LineString<f64> {
    let mut coords: Vec<Coord<f64>> = points.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
    if let Some(first) = coords.first().copied() {
        coords.push(first);
    }
    LineString::new(coords)
}

fn open_ring(line: &LineString<f64>) -> Vec<Point2> {
    let mut pts: Vec<Point2> = line.coords().map(|c| Point2::new(c.x, c.y)).collect();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    pts
}

/// Footprints of stairs that climb through the floor of `level`: stairs on a
/// lower storey whose top reaches this storey's floor.
pub fn stair_holes(home: &Home, level: Option<LevelId>) -> Vec<Vec<Point2>> {
    let floor = home.elevation_of(level);
    home.furniture
        .iter()
        .filter(|f| f.visible && f.is_stairs())
        .filter(|f| {
            let base = home.elevation_of(f.level);
            base < floor - 1.0 && base + f.elevation + f.height >= floor - 1.0
        })
        .map(|f| f.footprint().to_vec())
        .collect()
}

/// Where the ray leaves `polygon`: the farthest crossing within `max` cm.
/// The crossing at the very start — the face the ray sets off from — is not
/// one of them.
fn ray_exit(polygon: &[Point2], from: Point2, dir: (f64, f64), max: f64) -> f64 {
    let mut exit = 0.0f64;
    let n = polygon.len();
    for k in 0..n {
        let (a, b) = (polygon[k], polygon[(k + 1) % n]);
        let (ex, ey) = (b.x - a.x, b.y - a.y);
        let denominator = dir.0 * ey - dir.1 * ex;
        if denominator.abs() < 1e-9 {
            continue; // Parallel to this edge.
        }
        let (dx, dy) = (a.x - from.x, a.y - from.y);
        let t = (dx * ey - dy * ex) / denominator;
        let u = (dx * dir.1 - dy * dir.0) / denominator;
        if (0.0..=1.0).contains(&u) && (PROBE..=max).contains(&t) {
            exit = exit.max(t);
        }
    }
    exit
}

/// How far the floor runs into the wall the room edge touches at `mid`: to the
/// middle of the wall when another room's floor comes to meet it there, and
/// through to the far face when nothing does.
fn depth_into_wall(
    mid: Point2,
    outward: (f64, f64),
    walls: &[(&Wall, Vec<Point2>, Polygon<f64>)],
    neighbours: impl Fn(geo::Point<f64>) -> bool,
) -> Option<(usize, f64)> {
    let probe = geo::Point::new(mid.x + outward.0 * PROBE, mid.y + outward.1 * PROBE);
    let (index, (wall, outline, _)) = walls
        .iter()
        .enumerate()
        .find(|(_, (_, _, polygon))| polygon.contains(&probe))?;
    // An edge that runs into the length of a wall instead of across it would
    // otherwise drag the floor along the whole wall.
    let far = ray_exit(outline, mid, outward, wall.thickness * 2.0);
    if far <= PROBE {
        return None;
    }
    let beyond = geo::Point::new(
        mid.x + outward.0 * (far + PROBE),
        mid.y + outward.1 * (far + PROBE),
    );
    Some((index, if neighbours(beyond) { far / 2.0 } else { far }))
}

/// A floor stops at the inner face of the walls around it, so the band under a
/// wall belongs to no room — and where a door or a passage cuts the wall away,
/// the ground shows through the gap. Real floors are laid under the walls, and
/// so are these: every stretch of a room edge that touches a wall carries its
/// floor on into it.
fn floors_under_walls(
    room: &Room,
    walls: &[(&Wall, Vec<Point2>, Polygon<f64>)],
    others: &[(RoomId, Polygon<f64>)],
) -> Vec<Polygon<f64>> {
    let neighbours = |p: geo::Point<f64>| {
        others
            .iter()
            .any(|(id, polygon)| *id != room.id && polygon.contains(&p))
    };
    let inside = to_polygon(&room.points);
    let mut pieces = Vec::new();
    let count = room.points.len();
    for k in 0..count {
        let (a, b) = (room.points[k], room.points[(k + 1) % count]);
        let length = a.distance(b);
        if length < 1e-6 {
            continue;
        }
        let along = ((b.x - a.x) / length, (b.y - a.y) / length);
        // The outward normal is the one that does not point into the room.
        let mut outward = (along.1, -along.0);
        let centre = Point2::new(a.x.midpoint(b.x), a.y.midpoint(b.y));
        if inside.contains(&geo::Point::new(
            centre.x + outward.0 * PROBE,
            centre.y + outward.1 * PROBE,
        )) {
            outward = (-outward.0, -outward.1);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = (length / UNDER_WALL_STEP).ceil().max(1.0) as usize;
        #[allow(clippy::cast_precision_loss)]
        let at = |s: usize| {
            let t = length * s as f64 / steps as f64;
            Point2::new(a.x + along.0 * t, a.y + along.1 * t)
        };
        // Neighbouring stretches that run the same depth into the same wall
        // become one piece, so a plain edge costs one polygon, not a dozen.
        let mut run: Option<(usize, usize, usize, f64)> = None;
        for step in 0..steps {
            let (from, to) = (at(step), at(step + 1));
            let mid = Point2::new(from.x.midpoint(to.x), from.y.midpoint(to.y));
            let found = depth_into_wall(mid, outward, walls, neighbours);
            match (&mut run, found) {
                (Some(open), Some((wall, depth)))
                    if open.2 == wall && (open.3 - depth).abs() < 0.1 =>
                {
                    open.1 = step + 1;
                }
                (_, found) => {
                    if let Some((first, last, wall, depth)) = run.take() {
                        pieces.extend(band(
                            at(first),
                            at(last),
                            along,
                            outward,
                            depth,
                            &walls[wall].2,
                        ));
                    }
                    run = found.map(|(wall, depth)| (step, step + 1, wall, depth));
                }
            }
        }
        if let Some((first, last, wall, depth)) = run {
            pieces.extend(band(
                at(first),
                at(last),
                along,
                outward,
                depth,
                &walls[wall].2,
            ));
        }
    }
    pieces
}

/// The floor under one stretch of wall: the stretch pushed `depth` into the
/// wall, and past its ends by as much, so the corners of a room fill in too.
/// Clipping it to the wall keeps the floor from ever reaching past it.
fn band(
    from: Point2,
    to: Point2,
    along: (f64, f64),
    outward: (f64, f64),
    depth: f64,
    wall: &Polygon<f64>,
) -> Vec<Polygon<f64>> {
    let out = |p: Point2| Point2::new(p.x + outward.0 * depth, p.y + outward.1 * depth);
    let start = Point2::new(from.x - along.0 * depth, from.y - along.1 * depth);
    let end = Point2::new(to.x + along.0 * depth, to.y + along.1 * depth);
    let quad = [start, end, out(end), out(start)];
    to_polygon(&quad).intersection(wall).0
}

/// Floor shapes of every visible room floor on `level`, with stair holes cut out.
pub fn floor_shapes(home: &Home, level: Option<LevelId>) -> Vec<FloorShape> {
    let holes: Vec<Polygon<f64>> = stair_holes(home, level)
        .iter()
        .map(|h| Polygon::new(ring(h), vec![]))
        .collect();
    let floored: Vec<&Room> = home
        .rooms
        .iter()
        .filter(|r| r.floor_visible && r.points.len() >= 3 && home.on_level(r.level, level))
        .collect();
    let on_level: Vec<Wall> = home
        .walls
        .iter()
        .filter(|w| home.on_level(w.level, level))
        .cloned()
        .collect();
    let walls: Vec<(&Wall, Vec<Point2>, Polygon<f64>)> = on_level
        .iter()
        .zip(crate::joins::wall_outlines(&on_level))
        .map(|(wall, outline)| {
            let polygon = to_polygon(&outline);
            (wall, outline, polygon)
        })
        .collect();
    let shapes: Vec<(RoomId, Polygon<f64>)> = floored
        .iter()
        .map(|room| (room.id, to_polygon(&room.points)))
        .collect();
    floored
        .iter()
        .flat_map(|room| {
            let mut shape = geo::MultiPolygon::new(vec![Polygon::new(ring(&room.points), vec![])]);
            for under in floors_under_walls(room, &walls, &shapes) {
                shape = shape.union(&under);
            }
            for hole in &holes {
                shape = shape.difference(hole);
            }
            let room_id = room.id;
            shape.into_iter().map(move |poly| {
                let raw = poly.earcut_triangles_raw();
                let vertex = |i: usize| {
                    let [x, y] = raw.vertices[i];
                    Point2::new(x, y)
                };
                let triangles = raw
                    .triangle_indices
                    .chunks(3)
                    .map(|t| [vertex(t[0]), vertex(t[1]), vertex(t[2])])
                    .collect();
                FloorShape {
                    room: room_id,
                    exterior: open_ring(poly.exterior()),
                    holes: poly.interiors().iter().map(open_ring).collect(),
                    triangles,
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::{Level, Room, Wall};
    use crate::furniture::Furniture;
    use crate::geometry::polygon_area;
    use crate::ids::FurnitureId;

    #[test]
    fn stairs_cut_a_hole_in_the_floor_above_only() {
        let mut home = Home::default();
        let ground = Level {
            id: LevelId(1),
            name: "Térreo".into(),
            elevation: 0.0,
            height: 250.0,
            floor_thickness: 12.0,
            ..Default::default()
        };
        let upper = Level {
            id: LevelId(2),
            name: "1º".into(),
            elevation: 262.0,
            height: 250.0,
            floor_thickness: 12.0,
            ..Default::default()
        };
        home.levels = vec![ground, upper];
        let square = |id, level| {
            let mut r = Room::new(
                RoomId(id),
                "Sala",
                vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(500.0, 0.0),
                    Point2::new(500.0, 400.0),
                    Point2::new(0.0, 400.0),
                ],
            );
            r.level = level;
            r
        };
        home.rooms = vec![square(10, None), square(11, Some(LevelId(2)))];
        home.furniture.push(Furniture {
            id: FurnitureId(20),
            catalog: "stairs".into(),
            name: "Escada".into(),
            position: Point2::new(250.0, 200.0),
            elevation: 0.0,
            angle: 0.0,
            width: 90.0,
            depth: 300.0,
            height: 270.0,
            mirrored: false,
            color: None,
            opening: None,
            model: None,
            visible: true,
            level: None,
            ..Default::default()
        });

        let ground_floor = floor_shapes(&home, Some(LevelId(1)));
        assert_eq!(ground_floor.len(), 1);
        assert_eq!(
            ground_floor[0].holes,
            [] as [std::vec::Vec<crate::geometry::Point2>; 0]
        );

        let upper_floor = floor_shapes(&home, Some(LevelId(2)));
        assert_eq!(upper_floor.len(), 1);
        assert_eq!(upper_floor[0].holes.len(), 1, "stairs open the upper floor");
        let area: f64 = upper_floor[0]
            .triangles
            .iter()
            .map(|t| polygon_area(t))
            .sum();
        assert!(
            (area - (500.0 * 400.0 - 90.0 * 300.0)).abs() < 1e-3,
            "{area}"
        );
        // A flight built by hand opens it only once it says it is one.
        home.furniture[0].catalog = "group".into();
        assert_eq!(
            floor_shapes(&home, Some(LevelId(2)))[0].holes,
            [] as [std::vec::Vec<crate::geometry::Point2>; 0]
        );
        home.furniture[0]
            .properties
            .insert(Furniture::ROLE_KEY.into(), "stair".into());
        assert_eq!(floor_shapes(&home, Some(LevelId(2)))[0].holes.len(), 1);
    }

    /// A room 400 wide either side of a wall 20 thick lying on y = 0.
    fn walled_home(rooms: usize) -> Home {
        let mut home = Home::default();
        let mut wall = Wall::new(
            crate::ids::WallId(1),
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
        );
        wall.thickness = 20.0;
        home.walls.push(wall);
        let strip = |id, from: f64, to: f64| {
            Room::new(
                RoomId(id),
                "Sala",
                vec![
                    Point2::new(0.0, from),
                    Point2::new(400.0, from),
                    Point2::new(400.0, to),
                    Point2::new(0.0, to),
                ],
            )
        };
        home.rooms.push(strip(1, 10.0, 210.0));
        if rooms > 1 {
            home.rooms.push(strip(2, -210.0, -10.0));
        }
        home
    }

    fn floor_area(home: &Home, room: RoomId) -> f64 {
        floor_shapes(home, None)
            .iter()
            .filter(|s| s.room == room)
            .flat_map(|s| s.triangles.iter())
            .map(|t| polygon_area(t))
            .sum()
    }

    #[test]
    fn floors_meet_in_the_middle_of_the_wall_between_them() {
        let home = walled_home(2);
        for room in [RoomId(1), RoomId(2)] {
            let area = floor_area(&home, room);
            // The room itself, plus its half of the 20 cm band under the wall.
            assert!(
                (area - 400.0 * (200.0 + 10.0)).abs() < 1.0,
                "{room:?} floors {area}"
            );
        }
    }

    #[test]
    fn a_floor_runs_through_a_wall_with_nothing_behind_it() {
        let home = walled_home(1);
        let area = floor_area(&home, RoomId(1));
        // Nothing comes to meet it, so the floor reaches the outer face: a
        // doorway in an outside wall shows floor, not the ground.
        assert!((area - 400.0 * (200.0 + 20.0)).abs() < 1.0, "{area}");
    }

    #[test]
    fn a_floor_away_from_every_wall_is_left_alone() {
        let mut home = walled_home(1);
        home.rooms[0].points = vec![
            Point2::new(0.0, 100.0),
            Point2::new(400.0, 100.0),
            Point2::new(400.0, 300.0),
            Point2::new(0.0, 300.0),
        ];
        let area = floor_area(&home, RoomId(1));
        assert!((area - 400.0 * 200.0).abs() < 1.0, "{area}");
    }
}
