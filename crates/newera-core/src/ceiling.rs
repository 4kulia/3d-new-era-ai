//! Declared room ceilings shared by rendering and layout analysis.
//! Heights are relative to the room's storey, never building elevations.
use geo::algorithm::triangulate_delaunay::DelaunayTriangulationConfig;
use geo::{BooleanOps, TriangulateDelaunay, TriangulateEarcut};

use crate::{Furniture, Home, Point2, Room, Wall, to_polygon};

/// One planar patch of a room's lower ceiling surface.
#[derive(Debug, Clone)]
pub struct CeilingTriangle {
    pub points: [Point2; 3],
    pub heights: [f64; 3],
    /// False when the existing roof model already draws this surface.
    pub draw: bool,
    known: bool,
}

impl CeilingTriangle {
    /// Height on the triangle's plane (also valid for clipping at its edges).
    pub fn height_at(&self, p: Point2) -> f64 {
        let [a, b, c] = self.points;
        let cross =
            |x: Point2, y: Point2, z: Point2| (y.x - x.x) * (z.y - x.y) - (y.y - x.y) * (z.x - x.x);
        let area = cross(a, b, c);
        let u = cross(p, b, c) / area;
        let v = cross(a, p, c) / area;
        u * self.heights[0] + v * self.heights[1] + (1.0 - u - v) * self.heights[2]
    }

    /// Highest excess at an actual intersection of the piece and surface.
    /// Both planes are linear, so extrema lie at intersection vertices.
    pub fn excess(&self, piece: &Furniture) -> Option<(f64, f64)> {
        let footprint = to_polygon(&self.points);
        box_faces(piece, true)
            .into_iter()
            .flat_map(|face| {
                footprint
                    .intersection(&to_polygon(&face.points))
                    .into_iter()
                    .flat_map(move |polygon| {
                        let face = face.clone();
                        polygon
                            .exterior()
                            .0
                            .iter()
                            .map(|p| {
                                let at = Point2::new(p.x, p.y);
                                (self.height_at(at), face.height_at(at))
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .filter(|(ceiling, top)| *top > *ceiling + crate::mounting::RECESS)
            .max_by(|(ca, ta), (cb, tb)| (ta - ca).total_cmp(&(tb - cb)))
    }
}

/// Actual transformed box faces, not the enclosing plan rectangle. Side
/// faces can be part of the upper/lower envelope after pitch and roll.
fn box_faces(piece: &Furniture, upper: bool) -> Vec<CeilingTriangle> {
    let corners = piece.tilted_corners();
    let center = [0.0, piece.height / 2.0, 0.0];
    let mut faces = Vec::new();
    for ids in [
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ] {
        let [a, b, c, d] = ids.map(|i| corners[i]);
        let ab = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
        let ac = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
        let normal = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let outward: f64 = (0..3)
            .map(|i| normal[i] * ((a[i] + b[i] + c[i] + d[i]) / 4.0 - center[i]))
            .sum();
        let vertical = normal[1] * outward.signum();
        if (upper && vertical <= 1e-8) || (!upper && vertical >= -1e-8) {
            continue;
        }
        for tri in [[a, b, c], [a, c, d]] {
            let points = tri.map(|p| piece.to_plan((p[0], p[2])));
            if crate::polygon_area(&points) > 1e-8 {
                faces.push(CeilingTriangle {
                    points,
                    heights: tri.map(|p| piece.elevation + p[1]),
                    draw: false,
                    known: true,
                });
            }
        }
    }
    faces
}

fn projection(p: Point2, a: Point2, b: Point2) -> (f64, Point2) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let t =
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0.0, 1.0);
    (t, Point2::new(a.x + t * dx, a.y + t * dy))
}

fn wall_height(wall: &Wall, p: Point2) -> (f64, f64) {
    let line = wall.centerline();
    #[allow(clippy::cast_precision_loss)]
    let segments = (line.len() - 1) as f64;
    line.windows(2)
        .enumerate()
        .map(|(i, edge)| {
            let (t, at) = projection(p, edge[0], edge[1]);
            #[allow(clippy::cast_precision_loss)]
            let t = (i as f64 + t) / segments;
            (
                p.distance(at),
                wall.height + (wall.height_at_end.unwrap_or(wall.height) - wall.height) * t,
            )
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("wall centerline")
}

fn boundary_height(walls: &[&Wall], p: Point2) -> Option<f64> {
    walls
        .iter()
        .filter_map(|w| {
            let (distance, height) = wall_height(w, p);
            (distance <= w.thickness / 2.0 + 2.0).then_some((distance, height, w.id))
        })
        .min_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        })
        .map(|(_, h, _)| h)
}

fn triangles(
    poly: &geo::Polygon<f64>,
    height: impl Fn(Point2) -> f64,
    draw: bool,
) -> Vec<CeilingTriangle> {
    poly.earcut_triangles()
        .into_iter()
        .filter_map(|t| {
            let points = [t.v1(), t.v2(), t.v3()].map(|p| Point2::new(p.x, p.y));
            (crate::polygon_area(&points) > 1e-8).then(|| CeilingTriangle {
                heights: points.map(&height),
                points,
                draw,
                known: true,
            })
        })
        .collect()
}

/// Clip a convex footprint to the half-plane where the roof is lower.
fn lower_region(points: &[Point2], delta: impl Fn(Point2) -> f64) -> Vec<Point2> {
    let mut out = Vec::new();
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        let (da, db) = (delta(a), delta(b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push(Point2::new(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y)));
        }
    }
    out
}

fn wall_ceiling(home: &Home, room: &Room) -> Vec<CeilingTriangle> {
    let level = home.resolve_level(room.level);
    let walls: Vec<_> = home
        .walls
        .iter()
        .filter(|w| home.resolve_level(w.level) == level)
        .collect();
    let mut boundary = Vec::new();
    for i in 0..room.points.len() {
        let (a, b) = (room.points[i], room.points[(i + 1) % room.points.len()]);
        let mut cuts = vec![(0.0, a)];
        for wall in &walls {
            for p in wall.centerline() {
                let (t, at) = projection(p, a, b);
                if t > 1e-8 && t < 1.0 - 1e-8 && p.distance(at) <= wall.thickness / 2.0 + 2.0 {
                    cuts.push((t, at));
                }
            }
        }
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        cuts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-8);
        boundary.extend(cuts.into_iter().map(|(_, p)| p));
    }
    if boundary
        .iter()
        .any(|p| boundary_height(&walls, *p).is_none())
    {
        return Vec::new();
    }
    let Ok(faces) =
        to_polygon(&boundary).constrained_triangulation(DelaunayTriangulationConfig::default())
    else {
        return Vec::new();
    };
    let Some(faces) = faces
        .into_iter()
        .map(|t| {
            let points = [t.v1(), t.v2(), t.v3()].map(|p| Point2::new(p.x, p.y));
            let heights = points.map(|p| boundary_height(&walls, p));
            Some(CeilingTriangle {
                heights: [heights[0]?, heights[1]?, heights[2]?],
                points,
                draw: true,
                known: true,
            })
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Vec::new();
    };
    faces
}

/// The storey physically above `level`: the lowest designed one over its
/// floor — not a tracing or another layout drawn at the same elevation.
pub(crate) fn storey_above(home: &Home, level: Option<crate::LevelId>) -> Option<crate::LevelId> {
    let floor = home.elevation_of(level);
    home.sorted_levels()
        .into_iter()
        .find(|l| !l.is_reference() && l.elevation > floor + 1.0)
        .map(|l| l.id)
}

/// Surface of a visible room. Sloping rooms interpolate the tops of their
/// boundary walls, preserving intermediate wall vertices (gable peaks).
/// Without those walls the slope is unknown, not an invented flat plane.
/// Roof panels below the declared surface replace only their covered area.
pub fn room_ceiling(home: &Home, room: &Room) -> Vec<CeilingTriangle> {
    if !room.ceiling_visible || room.points.len() < 3 {
        return Vec::new();
    }
    let level = home.resolve_level(room.level);
    let mut surface = if room.ceiling_flat {
        let height = level
            .and_then(|id| home.level(id))
            .map_or(home.wall_height, |l| l.height);
        if height <= 0.0 {
            return Vec::new();
        }
        triangles(&to_polygon(&room.points), |_| height, true)
    } else {
        wall_ceiling(home, room)
    };
    // Same roof-panel convention as roof fitting; storeys and visibility apply.
    let roofs: Vec<_> = home
        .furniture
        .iter()
        .filter(|f| home.resolve_level(f.level) == level && home.shown_in_3d(f.discipline, None))
        .flat_map(|top| {
            top.visible_leaves().into_iter().filter(move |leaf| {
                home.shown_in_3d(leaf.discipline, crate::layer_in_group(top, leaf))
            })
        })
        .filter(|f| {
            !f.is_opening()
                && f.light.is_none()
                && !matches!(
                    f.catalog.as_str(),
                    "pendant"
                        | "downlight"
                        | "led-panel"
                        | "led-strip"
                        | "table-lamp"
                        | "floor-lamp"
                        | "wall-lamp"
                )
                && (f.pitch != 0.0 || f.roll != 0.0)
                && f.width.min(f.depth) >= 30.0
                && f.height_range().1 > 0.0
        })
        .flat_map(|roof| box_faces(roof, false))
        .collect();
    // A roof may define the ceiling even when no wall profile is available.
    // Unknown uncovered patches are discarded, never rendered or checked.
    if surface.is_empty() && !roofs.is_empty() {
        let upper = roofs.iter().flat_map(|r| r.heights).fold(0.0, f64::max) + 1.0;
        surface = triangles(&to_polygon(&room.points), |_| upper, false);
        for patch in &mut surface {
            patch.known = false;
        }
    }
    for roof in roofs {
        let mut next = Vec::new();
        for patch in surface {
            let region = lower_region(&roof.points, |p| patch.height_at(p) - roof.height_at(p));
            let region = lower_region(&region, |p| roof.height_at(p));
            if region.len() < 3 {
                next.push(patch);
                continue;
            }
            let cut = to_polygon(&region);
            let original = to_polygon(&patch.points);
            for poly in original.difference(&cut) {
                let mut remaining = triangles(&poly, |p| patch.height_at(p), patch.draw);
                for face in &mut remaining {
                    face.known = patch.known;
                }
                next.extend(remaining);
            }
            for poly in original.intersection(&cut) {
                next.extend(triangles(&poly, |p| roof.height_at(p), false));
            }
        }
        surface = next;
    }
    // The stair climbing to the storey above goes up through this ceiling:
    // its void, the one it opens in that storey's floor, is open here too.
    if let Some(above) = storey_above(home, level) {
        for void in crate::stair_holes(home, Some(above)) {
            let cut = to_polygon(&void);
            let mut next = Vec::new();
            for patch in surface {
                let original = to_polygon(&patch.points);
                for poly in original.difference(&cut) {
                    let mut remaining = triangles(&poly, |p| patch.height_at(p), patch.draw);
                    for face in &mut remaining {
                        face.known = patch.known;
                    }
                    next.extend(remaining);
                }
            }
            surface = next;
        }
    }
    surface.retain(|p| p.known);
    surface
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FurnitureId, Issue, Level, LevelId, RoomId, WallId};

    #[test]
    fn the_stair_to_the_storey_above_opens_the_ceiling_under_it() {
        let mut home = Home::default();
        home.levels = vec![
            Level {
                id: LevelId(1),
                elevation: 0.0,
                height: 250.0,
                floor_thickness: 12.0,
                ..Level::default()
            },
            Level {
                id: LevelId(2),
                elevation: 262.0,
                height: 250.0,
                floor_thickness: 12.0,
                elevation_index: 1,
                ..Level::default()
            },
        ];
        let below = room();
        let area = |home: &Home| -> f64 {
            room_ceiling(home, &below)
                .iter()
                .map(|t| crate::polygon_area(&t.points))
                .sum()
        };
        assert!((area(&home) - 400.0 * 400.0).abs() < 1e-3);
        home.furniture.push(Furniture {
            id: FurnitureId(20),
            catalog: "stairs".into(),
            position: Point2::new(200.0, 200.0),
            width: 90.0,
            depth: 300.0,
            height: 262.0,
            ..Furniture::default()
        });
        assert!(
            (area(&home) - (400.0 * 400.0 - 90.0 * 300.0)).abs() < 1e-3,
            "{}",
            area(&home)
        );
        // A tracing of the ground floor, next in the list, is not the storey above.
        home.levels.push(Level {
            id: LevelId(3),
            elevation: 0.0,
            height: 250.0,
            elevation_index: 1,
            ..Level::default()
        });
        home.levels[2].set_reference(true);
        home.levels[1].elevation_index = 2;
        assert!(
            (area(&home) - (400.0 * 400.0 - 90.0 * 300.0)).abs() < 1e-3,
            "{}",
            area(&home)
        );
        // A step stool reaching nowhere leaves it closed.
        home.furniture[0].height = 60.0;
        assert!((area(&home) - 400.0 * 400.0).abs() < 1e-3);
    }

    fn room() -> Room {
        Room::new(
            RoomId(1),
            "Room",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(400.0, 0.0),
                Point2::new(400.0, 400.0),
                Point2::new(0.0, 400.0),
            ],
        )
    }
    fn walls(home: &mut Home, points: &[(f64, f64, f64)]) {
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            let mut wall = Wall::new(
                WallId(i as u64 + 1),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            );
            wall.height = a.2;
            wall.height_at_end = Some(b.2);
            home.walls.push(wall);
        }
    }
    fn height(surface: &[CeilingTriangle], at: Point2) -> Option<f64> {
        use geo::Intersects;
        surface
            .iter()
            .filter(|t| to_polygon(&t.points).intersects(&geo::Point::new(at.x, at.y)))
            .map(|t| t.height_at(at))
            .min_by(f64::total_cmp)
    }
    fn light(at: Point2, top: f64) -> Furniture {
        Furniture {
            id: FurnitureId(20),
            catalog: "pendant".into(),
            position: at,
            width: 20.0,
            depth: 20.0,
            height: 30.0,
            elevation: top - 30.0,
            ..Default::default()
        }
    }
    #[test]
    fn declared_flat_height_is_independent_of_walls_and_building_elevation() {
        let mut home = Home::default();
        home.wall_height = 300.0;
        let mut room = room();
        walls(
            &mut home,
            &[
                (0.0, 0.0, 450.0),
                (400.0, 0.0, 450.0),
                (400.0, 400.0, 450.0),
                (0.0, 400.0, 450.0),
            ],
        );
        assert_eq!(
            height(&room_ceiling(&home, &room), Point2::new(200.0, 200.0)),
            Some(300.0)
        );
        home.levels.push(Level {
            id: LevelId(2),
            elevation: 700.0,
            height: 280.0,
            ..Default::default()
        });
        room.level = Some(LevelId(2));
        assert_eq!(
            height(&room_ceiling(&home, &room), Point2::new(200.0, 200.0)),
            Some(280.0)
        );
        room.ceiling_visible = false;
        assert!(room_ceiling(&home, &room).is_empty());
    }
    #[test]
    fn slope_is_shared_with_layout_and_checks_the_whole_light_footprint() {
        let mut home = Home::default();
        let mut room = room();
        room.ceiling_flat = false;
        walls(
            &mut home,
            &[
                (0.0, 0.0, 200.0),
                (400.0, 0.0, 400.0),
                (400.0, 400.0, 400.0),
                (0.0, 400.0, 200.0),
            ],
        );
        let surface = room_ceiling(&home, &room);
        for (x, expected) in [(0.0, 200.0), (100.0, 250.0), (200.0, 300.0), (400.0, 400.0)] {
            assert!((height(&surface, Point2::new(x, 200.0)).unwrap() - expected).abs() < 1e-6);
        }
        home.rooms.push(room);
        // Center is below 300, but the lower edge crosses the slope at 295.
        home.furniture.push(light(Point2::new(200.0, 200.0), 299.0));
        assert!(crate::check_layout(&home).iter().any(|i|matches!(i,Issue::AboveCeiling{ceiling,top,..} if (*ceiling-295.0).abs()<1e-6 && (*top-299.0).abs()<1e-6)));
        home.furniture[0].elevation -= 4.0;
        assert!(
            !crate::check_layout(&home)
                .iter()
                .any(|i| matches!(i, Issue::AboveCeiling { .. }))
        );
        home.walls.clear();
        assert!(
            room_ceiling(&home, &home.rooms[0]).is_empty(),
            "unknown slope is not a flat ceiling"
        );
    }
    #[test]
    fn intermediate_gable_vertices_are_not_dropped_as_collinear_plan_points() {
        let mut home = Home::default();
        let mut room = room();
        room.ceiling_flat = false;
        walls(
            &mut home,
            &[
                (0.0, 0.0, 200.0),
                (200.0, 0.0, 400.0),
                (400.0, 0.0, 200.0),
                (400.0, 400.0, 200.0),
                (200.0, 400.0, 400.0),
                (0.0, 400.0, 200.0),
            ],
        );
        let surface = room_ceiling(&home, &room);
        for y in [0.0, 100.0, 200.0, 400.0] {
            assert!((height(&surface, Point2::new(200.0, y)).unwrap() - 400.0).abs() < 1e-6);
            assert!((height(&surface, Point2::new(100.0, y)).unwrap() - 300.0).abs() < 1e-6);
        }
    }
    #[test]
    fn partial_roof_replaces_only_covered_area_and_respects_storeys_and_visibility() {
        let mut home = Home::default();
        home.wall_height = 300.0;
        let room = room();
        let roof = Furniture {
            id: FurnitureId(3),
            catalog: "box".into(),
            position: Point2::new(200.0, 200.0),
            width: 200.0,
            depth: 600.0,
            height: 10.0,
            elevation: 180.0,
            pitch: 20.0,
            ..Default::default()
        };
        home.furniture.push(roof.clone());
        let surface = room_ceiling(&home, &room);
        assert!(
            (height(&surface, Point2::new(200.0, 200.0)).unwrap()
                - (185.0 - 5.0 / 20.0_f64.to_radians().cos()))
            .abs()
                < 1e-6
        );
        assert_eq!(height(&surface, Point2::new(50.0, 200.0)), Some(300.0));
        let drawn_area: f64 = surface
            .iter()
            .filter(|t| t.draw)
            .map(|t| crate::polygon_area(&t.points))
            .sum();
        assert!(
            (drawn_area - 80_000.0).abs() < 0.01,
            "uncovered half of the room keeps its ceiling"
        );
        home.furniture[0].visible = false;
        assert_eq!(
            height(&room_ceiling(&home, &room), Point2::new(200.0, 200.0)),
            Some(300.0)
        );
        home.furniture[0].visible = true;
        home.furniture[0].discipline = Some(crate::Discipline::Electrical);
        home.hidden_disciplines.push(crate::Discipline::Electrical);
        assert_eq!(
            height(&room_ceiling(&home, &room), Point2::new(200.0, 200.0)),
            Some(300.0)
        );
        home.hidden_disciplines.clear();
        home.levels = vec![
            Level {
                id: LevelId(1),
                height: 300.0,
                ..Default::default()
            },
            Level {
                id: LevelId(2),
                height: 250.0,
                elevation: 300.0,
                ..Default::default()
            },
        ];
        home.furniture[0].level = Some(LevelId(2));
        assert_eq!(
            height(&room_ceiling(&home, &room), Point2::new(200.0, 200.0)),
            Some(300.0)
        );
    }
    #[test]
    fn roof_crossing_a_flat_ceiling_is_clipped_at_the_height_intersection() {
        let mut home = Home::default();
        home.wall_height = 300.0;
        let room = room();
        let roof = Furniture {
            id: FurnitureId(3),
            catalog: "box".into(),
            position: Point2::new(200.0, 200.0),
            width: 600.0,
            depth: 600.0,
            height: 10.0,
            elevation: 300.0,
            pitch: 20.0,
            ..Default::default()
        };
        home.furniture.push(roof.clone());
        let surface = room_ceiling(&home, &room);
        for y in [50.0, 150.0, 250.0, 350.0] {
            let at = Point2::new(200.0, y);
            assert!(
                (height(&surface, at).unwrap()
                    - 300.0_f64.min(
                        305.0
                            - 5.0 / 20.0_f64.to_radians().cos()
                            - (y - 200.0) * 20.0_f64.to_radians().tan()
                    ))
                .abs()
                    < 0.001
            );
        }
        assert!(surface.iter().any(|f| f.draw));
        assert!(surface.iter().any(|f| !f.draw));
    }

    #[test]
    fn roof_without_boundary_walls_is_known_only_under_its_footprint() {
        let mut home = Home::default();
        let mut room = room();
        room.ceiling_flat = false;
        let roof = Furniture {
            id: FurnitureId(3),
            catalog: "box".into(),
            position: Point2::new(200.0, 200.0),
            width: 200.0,
            depth: 600.0,
            height: 10.0,
            elevation: 180.0,
            pitch: 20.0,
            ..Default::default()
        };
        home.furniture.push(roof.clone());
        let surface = room_ceiling(&home, &room);
        assert!(surface.iter().all(|t| !t.draw));
        assert!(height(&surface, Point2::new(200.0, 200.0)).is_some());
        assert!(height(&surface, Point2::new(50.0, 200.0)).is_none());
        home.rooms.push(room);
        home.furniture.push(light(Point2::new(200.0, 200.0), 220.0));
        assert!(
            crate::check_layout(&home)
                .iter()
                .any(|i| matches!(i, Issue::AboveCeiling { .. }))
        );
        home.furniture[0].visible = false;
        assert!(
            !crate::check_layout(&home)
                .iter()
                .any(|i| matches!(i, Issue::AboveCeiling { .. }))
        );
        // A tilted luminaire must never become its own roof.
        home.furniture[1].pitch = 20.0;
        home.furniture[1].width = 100.0;
        home.furniture[1].depth = 100.0;
        assert!(room_ceiling(&home, &home.rooms[0]).is_empty());
    }

    #[test]
    fn combined_pitch_roll_and_mirroring_use_the_transformed_box_faces() {
        let mut home = Home::default();
        let mut room = room();
        room.ceiling_flat = false;
        let roof = Furniture {
            id: FurnitureId(3),
            catalog: "box".into(),
            position: Point2::new(200.0, 200.0),
            width: 600.0,
            depth: 600.0,
            height: 40.0,
            elevation: 200.0,
            pitch: 25.0,
            roll: 30.0,
            angle: 37.0,
            mirrored: true,
            ..Default::default()
        };
        home.furniture.push(roof.clone());
        let surface = room_ceiling(&home, &room);
        let pitch = 25.0_f64.to_radians();
        let roll = 30.0_f64.to_radians();
        for (x, z) in [(0.0, 0.0), (30.0, -50.0), (-20.0, 40.0)] {
            // Inverse rotation of the bottom plane y=-height/2, independently
            // of the old approximate underside helper and clipping code.
            let expected = 220.0 + x * roll.tan() / pitch.cos()
                - z * pitch.tan()
                - 20.0 / (pitch.cos() * roll.cos());
            let at = roof.to_plan((x, z));
            assert!((height(&surface, at).unwrap() - expected).abs() < 0.001);
        }
        home.furniture.clear();
        room.ceiling_flat = true;
        home.wall_height = 250.0;
        home.rooms.push(room);
        let mut fixture = light(Point2::new(200.0, 200.0), 220.0);
        fixture.width = 100.0;
        fixture.depth = 100.0;
        fixture.height = 20.0;
        fixture.elevation = 200.0;
        fixture.pitch = 45.0;
        home.furniture.push(fixture);
        let expected_top = 210.0 + 60.0 * 45.0_f64.to_radians().cos();
        assert!(
            crate::check_layout(&home).iter().any(
                |i| matches!(i,Issue::AboveCeiling{top,..} if (*top-expected_top).abs()<0.001)
            )
        );
    }

    #[test]
    fn concave_room_does_not_acquire_a_ceiling_across_its_notch() {
        let mut home = Home::default();
        let mut room = room();
        room.ceiling_flat = false;
        let outline = [
            (0.0, 0.0, 200.0),
            (400.0, 0.0, 400.0),
            (400.0, 200.0, 400.0),
            (200.0, 200.0, 300.0),
            (200.0, 400.0, 300.0),
            (0.0, 400.0, 200.0),
        ];
        room.points = outline.iter().map(|p| Point2::new(p.0, p.1)).collect();
        walls(&mut home, &outline);
        let surface = room_ceiling(&home, &room);
        assert_eq!(height(&surface, Point2::new(300.0, 300.0)), None);
        assert!((height(&surface, Point2::new(100.0, 300.0)).unwrap() - 250.0).abs() < 1e-6);
        let area: f64 = surface.iter().map(|t| crate::polygon_area(&t.points)).sum();
        assert!((area - 120_000.0).abs() < 1e-6);
    }
}
