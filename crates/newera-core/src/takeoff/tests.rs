//! The takeoff of one two-storey house, read whole.
//!
//! Ground floor (lv1): room r10, 400 × 300 cm. Its north wall w1 carries a
//! tile band 0–120 cm and paint 120–250 cm, and is cut by two windows that
//! overlap each other (f30, f31) and a door (f32) that runs out of the tile
//! band into the paint. Its east wall w2 slopes from 250 to 350 cm: paint
//! only above 300 cm inside, render on its whole outer side. Its west wall
//! w3 is curved, bulging into the room, plastered, and stands clear of the
//! others, so w1 starts free and ends at w2's inner face, 392.5 cm along
//! on the room side. A stair (f34) climbs to
//! the floor above. Upper floor (lv2, at 262 cm): room r11 over r10, with
//! its own north wall w4 and a window f33 — which stands over w1, at a
//! height measured from the other floor, and cuts only w4.

use super::*;
use crate::elements::{Level, Room};
use crate::finishes::{Band, FinishAssignment, Surface};
use crate::furniture::{Furniture, Opening, Purchase, align_to_wall};
use crate::ids::{FurnitureId, LevelId, RoomId, WallId};

const LV1: LevelId = LevelId(1);
const LV2: LevelId = LevelId(2);

fn rect(id: u64, level: LevelId) -> Room {
    let mut room = Room::new(
        RoomId(id),
        "Sala",
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
            Point2::new(400.0, 300.0),
            Point2::new(0.0, 300.0),
        ],
    );
    room.level = Some(level);
    room
}

fn wall(id: u64, a: (f64, f64), b: (f64, f64), level: LevelId) -> Wall {
    let mut wall = Wall::new(WallId(id), Point2::new(a.0, a.1), Point2::new(b.0, b.1));
    wall.level = Some(level);
    wall
}

/// A door or window of `width` cm, `bottom` to `top` cm, `along` cm into `wall`.
fn opening(id: u64, wall: &Wall, along: f64, width: f64, bottom: f64, top: f64) -> Furniture {
    let mut piece = Furniture {
        id: FurnitureId(id),
        catalog: "window".into(),
        width,
        depth: 10.0,
        height: top - bottom,
        elevation: bottom,
        opening: Some(Opening::default()),
        level: wall.level,
        ..Furniture::default()
    };
    align_to_wall(&mut piece, wall, along);
    piece
}

fn piece(id: u64, product: Option<&str>, purchase: Option<Purchase>, level: LevelId) -> Furniture {
    let mut piece = Furniture {
        id: FurnitureId(id),
        catalog: "sofa-3".into(),
        position: Point2::new(200.0, 150.0),
        level: Some(level),
        ..Furniture::default()
    };
    piece.info.product = product.map(str::to_owned);
    piece.info.purchase = purchase;
    piece
}

fn finish(
    surface: &str,
    product: &str,
    band: Option<(f64, f64)>,
    coats: Option<u32>,
    waste: Option<f64>,
) -> FinishAssignment {
    FinishAssignment {
        surface: surface.parse().unwrap(),
        product: product.into(),
        band: band.map(|(from, to)| Band { from, to }),
        coats,
        waste,
    }
}

fn house() -> Home {
    let mut home = Home::default();
    home.levels = vec![
        Level {
            id: LV1,
            name: "Begane grond".into(),
            ..Level::default()
        },
        Level {
            id: LV2,
            name: "Verdieping".into(),
            elevation: 262.0,
            ..Level::default()
        },
    ];
    let w1 = wall(1, (0.0, 0.0), (400.0, 0.0), LV1);
    let mut w2 = wall(2, (400.0, 0.0), (400.0, 300.0), LV1);
    w2.height_at_end = Some(350.0);
    let mut w3 = wall(3, (0.0, 250.0), (0.0, 50.0), LV1);
    w3.arc_extent = Some(-90.0);
    let w4 = wall(4, (0.0, 0.0), (400.0, 0.0), LV2);
    let stair = Furniture {
        id: FurnitureId(34),
        catalog: "stairs".into(),
        position: Point2::new(100.0, 150.0),
        width: 90.0,
        depth: 200.0,
        height: 262.0,
        level: Some(LV1),
        ..Furniture::default()
    };
    let group = Furniture {
        id: FurnitureId(44),
        catalog: "group".into(),
        position: Point2::new(300.0, 150.0),
        level: Some(LV2),
        children: vec![Furniture {
            position: Point2::new(300.0, 150.0),
            ..piece(45, Some("chair"), None, LV2)
        }],
        ..Furniture::default()
    };
    home.furniture = vec![
        opening(30, &w1, 150.0, 100.0, 90.0, 210.0),
        opening(31, &w1, 200.0, 100.0, 150.0, 230.0),
        opening(32, &w1, 320.0, 80.0, 0.0, 210.0),
        opening(33, &w4, 50.0, 60.0, 100.0, 200.0),
        stair,
        piece(40, Some("sofa"), None, LV1),
        piece(41, Some("sofa"), Some(Purchase::Existing), LV1),
        piece(42, None, None, LV1),
        piece(43, Some("chair"), Some(Purchase::Buy), LV2),
        piece(46, Some("sofa"), Some(Purchase::Excluded), LV1),
        group,
    ];
    home.walls = vec![w1, w2, w3, w4];
    home.rooms = vec![rect(10, LV1), rect(11, LV2)];
    home.finish_assignments = vec![
        finish("w1:right", "tile", Some((0.0, 120.0)), None, Some(10.0)),
        finish("w1:right", "paint", Some((120.0, 250.0)), Some(2), None),
        finish("w2:right", "paint", Some((300.0, 400.0)), Some(2), None),
        finish("w2:left", "render", None, None, None),
        finish("w3:right", "plaster", None, None, None),
        finish("w4:right", "paint", None, Some(2), None),
        finish("r10:floor", "laminate", None, None, Some(5.0)),
        finish("r11:floor", "laminate", None, None, Some(5.0)),
        finish("r10:ceiling", "paint", None, Some(2), None),
        finish("w99:left", "tile", None, None, None),
    ];
    home
}

/// Every area to a millionth of a m², so the whole answer compares at once.
fn rounded(mut takeoff: Takeoff) -> Takeoff {
    let r = |v: &mut f64| *v = (*v * 1e6).round() / 1e6;
    for product in &mut takeoff.finishes {
        r(&mut product.gross);
        r(&mut product.openings);
        r(&mut product.net);
        for s in &mut product.surfaces {
            r(&mut s.gross);
            r(&mut s.openings);
            r(&mut s.net);
            for (width, height) in &mut s.spans {
                r(width);
                r(height);
            }
        }
    }
    takeoff
}

#[allow(clippy::too_many_arguments)]
fn area(
    surface: &str,
    level: LevelId,
    room: Option<u64>,
    band: Option<(f64, f64)>,
    coats: Option<u32>,
    waste: Option<f64>,
    m2: (f64, f64),
    cut_by: &[u64],
    spans: &[(f64, f64)],
) -> SurfaceArea {
    SurfaceArea {
        surface: surface.parse::<Surface>().unwrap(),
        level: Some(level),
        room: room.map(RoomId),
        band: band.map(|(from, to)| Band { from, to }),
        coats,
        waste,
        gross: m2.0,
        openings: m2.1,
        net: ((m2.0 - m2.1) * 1e6).round() / 1e6,
        cut_by: cut_by.iter().copied().map(FurnitureId).collect(),
        spans: spans.to_vec(),
    }
}

fn product(name: &str, surfaces: Vec<SurfaceArea>) -> ProductArea {
    let sum =
        |f: fn(&SurfaceArea) -> f64| (surfaces.iter().map(f).sum::<f64>() * 1e6).round() / 1e6;
    ProductArea {
        product: name.into(),
        gross: sum(|s| s.gross),
        openings: sum(|s| s.openings),
        net: sum(|s| s.net),
        surfaces,
    }
}

/// The curved wall's room side, its convex one: a 90° arc on a 200 cm
/// chord has a radius of 100 / sin(π/4) cm on the centreline, 7.5 cm more
/// on that face, a quarter turn long, by 250 cm.
fn arc_m2() -> f64 {
    let radius = 100.0 / std::f64::consts::FRAC_PI_4.sin() + 7.5;
    (radius * std::f64::consts::FRAC_PI_2 * 250.0 / 1e4 * 1e6).round() / 1e6
}

/// That face's length, cm.
fn arc_cm() -> f64 {
    let radius = 100.0 / std::f64::consts::FRAC_PI_4.sin() + 7.5;
    (radius * std::f64::consts::FRAC_PI_2 * 1e6).round() / 1e6
}

#[test]
fn the_whole_house_is_measured_per_product() {
    let got = rounded(takeoff(&house(), Only::default()));
    let expected = Takeoff {
        finishes: vec![
            product(
                "laminate",
                vec![
                    // 400 × 300 cm, no stair through the ground floor.
                    area(
                        "r10:floor",
                        LV1,
                        Some(10),
                        None,
                        None,
                        Some(5.0),
                        (12.0, 0.0),
                        &[],
                        &[],
                    ),
                    // Less the stair's 90 × 200 cm void.
                    area(
                        "r11:floor",
                        LV2,
                        Some(11),
                        None,
                        None,
                        Some(5.0),
                        (10.2, 0.0),
                        &[],
                        &[],
                    ),
                ],
            ),
            product(
                "paint",
                vec![
                    // The stair opens the ceiling under the floor above too.
                    area(
                        "r10:ceiling",
                        LV1,
                        Some(10),
                        None,
                        Some(2),
                        None,
                        (10.2, 0.0),
                        &[],
                        &[],
                    ),
                    // 392.5 × 130 cm; the union of the two windows within the
                    // band (9 000 + 8 000 − 3 000 cm²) and the door's top
                    // 80 × 90 cm.
                    area(
                        "w1:right",
                        LV1,
                        Some(10),
                        Some((120.0, 250.0)),
                        Some(2),
                        None,
                        (5.1025, 2.12),
                        &[30, 31, 32],
                        &[(392.5, 130.0)],
                    ),
                    // The slope passes 300 cm half way: a triangle of
                    // 150 cm × 50 cm.
                    area(
                        "w2:right",
                        LV1,
                        Some(10),
                        Some((300.0, 400.0)),
                        Some(2),
                        None,
                        (0.375, 0.0),
                        &[],
                        &[(150.0, 50.0)],
                    ),
                    // 400 × 250 cm less its own window, 60 × 100 cm.
                    area(
                        "w4:right",
                        LV2,
                        Some(11),
                        None,
                        Some(2),
                        None,
                        (10.0, 0.6),
                        &[33],
                        &[(400.0, 250.0)],
                    ),
                ],
            ),
            product(
                "plaster",
                vec![area(
                    "w3:right",
                    LV1,
                    Some(10),
                    None,
                    None,
                    None,
                    (arc_m2(), 0.0),
                    &[],
                    &[(arc_cm(), 250.0)],
                )],
            ),
            product(
                "render",
                // A trapezoid, 300 cm long from 250 to 350 cm high, and
                // the 7.5 cm the outer corner with w1 adds at 250 cm; it
                // faces no room.
                vec![area(
                    "w2:left",
                    LV1,
                    None,
                    None,
                    None,
                    None,
                    (9.1875, 0.0),
                    &[],
                    &[(307.5, 350.0)],
                )],
            ),
            product(
                "tile",
                vec![
                    // 392.5 × 120 cm; window f30's bottom 30 cm and the door's
                    // 80 × 120 cm; f31 starts above the band.
                    area(
                        "w1:right",
                        LV1,
                        Some(10),
                        Some((0.0, 120.0)),
                        None,
                        Some(10.0),
                        (4.71, 1.26),
                        &[30, 32],
                        &[(392.5, 120.0)],
                    ),
                ],
            ),
        ],
        furniture: vec![
            PieceCount {
                product: "chair".into(),
                pieces: vec![FurnitureId(43), FurnitureId(45)],
            },
            PieceCount {
                product: "sofa".into(),
                pieces: vec![FurnitureId(40)],
            },
        ],
        missing: vec![finish("w99:left", "tile", None, None, None)],
    };
    assert_eq!(got, expected);
}

#[test]
fn a_room_and_a_product_narrow_it_down() {
    let home = house();
    let upstairs = rounded(takeoff(
        &home,
        Only {
            room: Some(RoomId(11)),
            product: None,
        },
    ));
    assert_eq!(
        upstairs,
        Takeoff {
            finishes: vec![
                product(
                    "laminate",
                    vec![area(
                        "r11:floor",
                        LV2,
                        Some(11),
                        None,
                        None,
                        Some(5.0),
                        (10.2, 0.0),
                        &[],
                        &[],
                    )],
                ),
                product(
                    "paint",
                    vec![area(
                        "w4:right",
                        LV2,
                        Some(11),
                        None,
                        Some(2),
                        None,
                        (10.0, 0.6),
                        &[33],
                        &[(400.0, 250.0)],
                    )],
                ),
            ],
            furniture: vec![PieceCount {
                product: "chair".into(),
                pieces: vec![FurnitureId(43), FurnitureId(45)],
            }],
            missing: vec![finish("w99:left", "tile", None, None, None)],
        }
    );
    let sofa = rounded(takeoff(
        &home,
        Only {
            room: None,
            product: Some("sofa"),
        },
    ));
    assert_eq!(
        sofa,
        Takeoff {
            finishes: vec![],
            furniture: vec![PieceCount {
                product: "sofa".into(),
                pieces: vec![FurnitureId(40)],
            }],
            missing: vec![],
        }
    );
}

#[test]
fn a_sloping_band_is_the_area_between_its_heights() {
    // 300 cm from 250 to 350 cm high.
    assert_eq!(band_area(300.0, 250.0, 350.0, 0.0, f64::INFINITY), 90_000.0);
    assert_eq!(band_area(300.0, 250.0, 350.0, 300.0, 400.0), 3_750.0);
    // Falling the other way, the same.
    assert_eq!(band_area(300.0, 350.0, 250.0, 300.0, 400.0), 3_750.0);
    // A band crossed by the top: 0–120 under a top running 100 → 200.
    // 120 high all along, less the 20 × 20 cm triangle where the top is
    // still under it.
    assert_eq!(band_area(100.0, 100.0, 200.0, 0.0, 120.0), 12_000.0 - 200.0);
    // A band the wall never reaches.
    assert_eq!(band_area(300.0, 250.0, 350.0, 360.0, 400.0), 0.0);
}

#[test]
fn overlapping_openings_are_taken_off_once() {
    assert_eq!(
        union_area(&[[100.0, 200.0, 90.0, 210.0], [150.0, 250.0, 150.0, 230.0]]),
        17_000.0
    );
    // One inside another counts as the outer one.
    assert_eq!(
        union_area(&[[0.0, 100.0, 0.0, 100.0], [20.0, 40.0, 20.0, 40.0]]),
        10_000.0
    );
    // Apart, both.
    assert_eq!(
        union_area(&[[0.0, 10.0, 0.0, 10.0], [20.0, 30.0, 0.0, 10.0]]),
        200.0
    );
    assert_eq!(union_area(&[]), 0.0);
}

#[test]
fn where_rooms_nest_the_smallest_is_the_one_a_wall_faces_and_a_piece_stands_in() {
    let mut home = house();
    let mut alcove = Room::new(
        RoomId(12),
        "Nis",
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
            Point2::new(400.0, 200.0),
            Point2::new(0.0, 200.0),
        ],
    );
    alcove.level = Some(LV1);
    home.rooms.push(alcove);
    assert_eq!(
        facing_room(&home, &home.walls[0], Side::Right),
        Some(RoomId(12))
    );
    assert_eq!(facing_room(&home, &home.walls[0], Side::Left), None);
    assert_eq!(
        home.room_at(home.furniture[5].position, Some(LV1))
            .map(|r| r.id),
        Some(RoomId(12))
    );
}

#[test]
fn a_piece_is_bought_when_it_is_a_product_unless_it_says_otherwise() {
    use crate::furniture::PieceInfo;
    let info = |product: Option<&str>, purchase| PieceInfo {
        product: product.map(str::to_owned),
        purchase,
        ..PieceInfo::default()
    };
    assert_eq!(info(Some("p"), None).purchase(), Purchase::Buy);
    assert_eq!(info(None, None).purchase(), Purchase::Excluded);
    assert_eq!(
        info(Some("p"), Some(Purchase::Existing)).purchase(),
        Purchase::Existing
    );
    assert_eq!(info(None, Some(Purchase::Buy)).purchase(), Purchase::Buy);
}

/// Walls of the default 15 cm, 250 high, welded at their ends.
fn walls(points: &[(f64, f64)]) -> Vec<Wall> {
    (0..points.len() - 1)
        .map(|i| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let id = i as u64 + 1;
            Wall::new(
                WallId(id),
                Point2::new(points[i].0, points[i].1),
                Point2::new(points[i + 1].0, points[i + 1].1),
            )
        })
        .collect()
}

fn gross_of(home: &Home, surface: &str) -> f64 {
    let mut home = home.clone();
    home.finish_assignments = vec![finish(surface, "p", None, None, None)];
    let taken = takeoff(&home, Only::default());
    (taken.finishes[0].gross * 1e6).round() / 1e6
}

#[test]
fn each_side_is_measured_on_its_own_face() {
    let mut home = Home::default();
    home.walls = walls(&[
        (0.0, 0.0),
        (400.0, 0.0),
        (400.0, 300.0),
        (0.0, 300.0),
        (0.0, 0.0),
    ]);
    // Inside, the faces run 385 and 285 cm between the corners; outside,
    // 415 and 315.
    let inside: Vec<f64> = ["w1:right", "w2:right", "w3:right", "w4:right"]
        .iter()
        .map(|s| gross_of(&home, s))
        .collect();
    assert_eq!(inside, vec![9.625, 7.125, 9.625, 7.125]);
    // The room's perimeter, 2 × (385 + 285) cm, by 250 cm.
    assert_eq!((inside.iter().sum::<f64>() * 1e6).round() / 1e6, 33.5);
    assert_eq!(gross_of(&home, "w1:left"), 10.375);
    assert_eq!(gross_of(&home, "w2:left"), 7.875);
}

#[test]
fn a_partition_runs_between_the_faces_it_meets() {
    let mut home = Home::default();
    home.walls = walls(&[
        (0.0, 0.0),
        (400.0, 0.0),
        (400.0, 300.0),
        (0.0, 300.0),
        (0.0, 0.0),
    ]);
    home.walls.push(Wall::new(
        WallId(5),
        Point2::new(200.0, 0.0),
        Point2::new(200.0, 300.0),
    ));
    // From the top wall's inner face to the bottom one's: 285 cm.
    assert_eq!(gross_of(&home, "w5:left"), 7.125);
    assert_eq!(gross_of(&home, "w5:right"), 7.125);
}

#[test]
fn a_curved_wall_is_longer_on_its_convex_side() {
    let mut home = Home::default();
    let mut wall = Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(300.0, 0.0));
    // Bulging to the left: the centre, and the concave side, on the right.
    wall.arc_extent = Some(90.0);
    home.walls.push(wall);
    let radius = 300.0 / (2.0 * std::f64::consts::FRAC_PI_4.sin());
    let face = |r: f64| ((r * std::f64::consts::FRAC_PI_2 * 250.0 / 1e4) * 1e6).round() / 1e6;
    assert_eq!(gross_of(&home, "w1:left"), face(radius + 7.5));
    assert_eq!(gross_of(&home, "w1:right"), face(radius - 7.5));
}

#[test]
fn a_reference_storey_is_traced_not_bought() {
    let mut traced = house();
    let mut tracing = Level {
        id: LevelId(3),
        name: "Scan".into(),
        ..Level::default()
    };
    tracing.set_reference(true);
    traced.levels.push(tracing);
    traced
        .walls
        .push(wall(7, (0.0, 0.0), (400.0, 0.0), LevelId(3)));
    traced
        .finish_assignments
        .push(finish("w7:right", "paint", None, Some(2), None));
    traced
        .furniture
        .push(piece(47, Some("sofa"), None, LevelId(3)));
    assert_eq!(
        rounded(takeoff(&traced, Only::default())),
        rounded(takeoff(&house(), Only::default()))
    );
}

#[test]
fn a_wall_upstairs_does_not_join_the_wall_below() {
    let mut home = Home::default();
    home.levels = vec![
        Level {
            id: LV1,
            ..Level::default()
        },
        Level {
            id: LV2,
            elevation: 262.0,
            ..Level::default()
        },
    ];
    home.walls = vec![
        wall(1, (0.0, 0.0), (400.0, 0.0), LV1),
        wall(2, (400.0, 0.0), (400.0, 300.0), LV2),
    ];
    // Free at both ends: 400 cm on either side, no corner with w2.
    assert_eq!(gross_of(&home, "w1:right"), 10.0);
    assert_eq!(gross_of(&home, "w1:left"), 10.0);
}

#[test]
fn an_opening_past_an_inner_corner_is_taken_off_only_on_the_face() {
    let mut home = Home::default();
    home.walls = walls(&[(0.0, 0.0), (400.0, 0.0), (400.0, 300.0)]);
    // A door 80 cm wide and 200 high centred 360 cm along w1: 320–400 cm
    // on the centreline, of which the inner face, ending at 392.5 cm where
    // w2 meets it, holds 72.5 cm.
    let mut door = Furniture {
        id: FurnitureId(9),
        catalog: "door".into(),
        width: 80.0,
        depth: 10.0,
        height: 200.0,
        opening: Some(Opening::default()),
        ..Furniture::default()
    };
    align_to_wall(&mut door, &home.walls[0], 360.0);
    home.furniture.push(door);
    home.finish_assignments = vec![finish("w1:right", "p", None, None, None)];
    let taken = rounded(takeoff(&home, Only::default()));
    assert_eq!(
        (taken.finishes[0].gross, taken.finishes[0].openings),
        (9.8125, 1.45)
    );
}

/// The 400 × 300 rectangle of 15 cm walls split by a partition w5 at
/// x = 200 into rooms r6 (west) and r7 (east), every inner face painted.
fn two_rooms() -> Home {
    let mut home = Home::default();
    home.walls = walls(&[
        (0.0, 0.0),
        (400.0, 0.0),
        (400.0, 300.0),
        (0.0, 300.0),
        (0.0, 0.0),
    ]);
    home.walls.push(Wall::new(
        WallId(5),
        Point2::new(200.0, 0.0),
        Point2::new(200.0, 300.0),
    ));
    let room = |id, x0: f64, x1: f64| {
        Room::new(
            RoomId(id),
            "Kamer",
            vec![
                Point2::new(x0, 0.0),
                Point2::new(x1, 0.0),
                Point2::new(x1, 300.0),
                Point2::new(x0, 300.0),
            ],
        )
    };
    home.rooms = vec![room(6, 0.0, 200.0), room(7, 200.0, 400.0)];
    home.finish_assignments = [
        "w1:right", "w2:right", "w3:right", "w4:right", "w5:left", "w5:right",
    ]
    .iter()
    .map(|s| finish(s, "paint", None, None, None))
    .collect();
    home
}

#[test]
fn where_a_partition_meets_a_face_that_stretch_is_not_painted() {
    let home = two_rooms();
    // 385 cm between the corners, less the partition's 15 cm.
    assert_eq!(gross_of(&home, "w1:right"), 9.25);
    assert_eq!(gross_of(&home, "w3:right"), 9.25);
    // Both rooms' perimeters, 2 × 2 × (185 + 285) cm, by 250 cm.
    let taken = rounded(takeoff(&home, Only::default()));
    assert_eq!(taken.finishes[0].gross, 47.0);
}

#[test]
fn a_side_facing_two_rooms_is_split_between_them() {
    let home = two_rooms();
    let west = rounded(takeoff(
        &home,
        Only {
            room: Some(RoomId(6)),
            product: None,
        },
    ));
    let rows: Vec<(String, f64)> = west.finishes[0]
        .surfaces
        .iter()
        .map(|s| (s.surface.to_string(), s.gross))
        .collect();
    // 185 cm of each long wall, 285 of the end wall and the partition.
    assert_eq!(
        rows,
        vec![
            ("w1:right".to_owned(), 4.625),
            ("w3:right".to_owned(), 4.625),
            ("w4:right".to_owned(), 7.125),
            ("w5:right".to_owned(), 7.125),
        ]
    );
    assert_eq!(west.finishes[0].gross, 23.5);
    let all = rounded(takeoff(&home, Only::default()));
    let w1: Vec<(Option<RoomId>, f64)> = all.finishes[0]
        .surfaces
        .iter()
        .filter(|s| s.surface.to_string() == "w1:right")
        .map(|s| (s.room, s.gross))
        .collect();
    assert_eq!(w1, vec![(Some(RoomId(6)), 4.625), (Some(RoomId(7)), 4.625)]);
}

/// Wallpaper hangs in drops across each stretch of face on its own: a
/// partition standing against a side cuts it in two, even where one room
/// faces both pieces, and a piece is as tall as its highest end.
#[test]
fn each_stretch_of_a_face_is_its_own_span() {
    let mut home = two_rooms();
    home.rooms.truncate(1);
    home.rooms[0].points[1].x = 400.0;
    home.rooms[0].points[2].x = 400.0;
    home.walls[0].height_at_end = Some(300.0);
    let spans = |home: &Home, surface: &str| -> Vec<(f64, f64)> {
        rounded(takeoff(home, Only::default())).finishes[0]
            .surfaces
            .iter()
            .filter(|s| s.surface.to_string() == surface)
            .flat_map(|s| s.spans.clone())
            .collect()
    };
    // 7.5 to 192.5 cm along w1 and 207.5 to 392.5, the top rising from
    // 250 cm at 0 to 300 cm at 400: 250 + 50 × 192.5 / 400 and
    // 250 + 50 × 392.5 / 400.
    assert_eq!(
        spans(&home, "w1:right"),
        vec![(185.0, 274.0625), (185.0, 299.0625)]
    );
    home.finish_assignments[0].band = Some(Band {
        from: 100.0,
        to: 280.0,
    });
    assert_eq!(
        spans(&home, "w1:right"),
        vec![(185.0, 174.0625), (185.0, 180.0)]
    );
    // Falling from 300 to 250 cm, each piece is tallest at its start; the
    // outer side runs 7.5 cm past each end, where the top holds level.
    home.finish_assignments[0].band = None;
    home.walls[0].height = 300.0;
    home.walls[0].height_at_end = Some(250.0);
    home.finish_assignments
        .push(finish("w1:left", "paint", None, None, None));
    assert_eq!(
        spans(&home, "w1:right"),
        vec![(185.0, 299.0625), (185.0, 274.0625)]
    );
    assert_eq!(spans(&home, "w1:left"), vec![(415.0, 300.0)]);
}

/// A curved wall's face is one stretch, as tall as the band lets it be.
#[test]
fn a_curved_face_is_one_span_cut_to_its_band() {
    let mut home = house();
    home.finish_assignments = vec![finish(
        "w3:right",
        "plaster",
        Some((100.0, 400.0)),
        None,
        None,
    )];
    let taken = rounded(takeoff(&home, Only::default()));
    assert_eq!(taken.finishes[0].surfaces[0].spans, vec![(arc_cm(), 150.0)]);
    // Rising from 250 to 350 cm along it, a band from 300 cm covers the
    // second half of the arc, up to 50 cm.
    home.walls[2].height_at_end = Some(350.0);
    home.finish_assignments[0].band = Some(Band {
        from: 300.0,
        to: 400.0,
    });
    let taken = rounded(takeoff(&home, Only::default()));
    let half = (arc_cm() / 2.0 * 1e6).round() / 1e6;
    assert_eq!(taken.finishes[0].surfaces[0].spans, vec![(half, 50.0)]);
}

/// A band above the whole top of a stretch leaves it no span.
#[test]
fn a_band_above_the_top_has_no_span() {
    let mut home = two_rooms();
    home.finish_assignments[0].band = Some(Band {
        from: 260.0,
        to: 300.0,
    });
    let taken = rounded(takeoff(&home, Only::default()));
    let w1: Vec<&SurfaceArea> = taken.finishes[0]
        .surfaces
        .iter()
        .filter(|s| s.surface.to_string() == "w1:right")
        .collect();
    assert!(
        w1.iter().all(|s| s.spans.is_empty() && s.gross == 0.0),
        "{w1:?}"
    );
}

#[test]
fn a_side_belongs_to_the_room_most_of_it_faces() {
    let mut home = two_rooms();
    home.walls[4].start.x = 100.0;
    home.walls[4].end.x = 100.0;
    home.rooms[0].points[1].x = 100.0;
    home.rooms[0].points[2].x = 100.0;
    home.rooms[1].points[0].x = 100.0;
    home.rooms[1].points[3].x = 100.0;
    // 85 cm of w1 face r6, 285 cm face r7.
    assert_eq!(room_of(&home, "w1:right".parse().unwrap()), Some(RoomId(7)));
    assert_eq!(room_of(&home, "w5:right".parse().unwrap()), Some(RoomId(6)));
}

#[test]
fn a_side_belongs_to_a_room_before_to_no_room() {
    let mut home = two_rooms();
    home.walls[4].start.x = 100.0;
    home.walls[4].end.x = 100.0;
    home.rooms[0].points[1].x = 100.0;
    home.rooms[0].points[2].x = 100.0;
    home.rooms.truncate(1);
    // 85 cm of w1 face r6, the other 285 cm face no room.
    assert_eq!(room_of(&home, "w1:right".parse().unwrap()), Some(RoomId(6)));
}

#[test]
fn a_side_split_evenly_belongs_to_the_room_it_meets_first() {
    let home = two_rooms();
    // w1 runs west to east and meets r6 first; w3 runs back and meets r7.
    assert_eq!(room_of(&home, "w1:right".parse().unwrap()), Some(RoomId(6)));
    assert_eq!(room_of(&home, "w3:right".parse().unwrap()), Some(RoomId(7)));
}
