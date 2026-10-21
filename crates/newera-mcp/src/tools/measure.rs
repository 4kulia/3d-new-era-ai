//! Distances: clearance around a piece, the gap between two, and what a
//! straight line runs into.
//!
//! The answer an agent used to rebuild in a script from a full read.

use newera_core::Point2;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::invalid;
use crate::compact;

/// Opening spans in each straight host wall's local frame. These are wall-axis
/// distances, not floor clearances or a claim about the operable sash width.
pub(super) fn opening_measure(
    home: &newera_core::Home,
    piece: &newera_core::Furniture,
) -> serde_json::Value {
    let cuts = newera_core::wall_cuts(&home.walls, &home.furniture);
    let hosts: Vec<_> = home.walls.iter().zip(cuts).filter_map(|(wall, cuts)| {
        let cut = cuts.iter().find(|cut| cut.furniture == piece.id)?;
        let adjacent = |before: bool| {
            cuts.iter().filter(|other| other.furniture != piece.id
                && other.bottom < cut.top && other.top > cut.bottom)
                .filter(|other| if before { other.from < cut.from } else { other.from >= cut.from })
                .map(|other| (if before { cut.from - other.to } else { other.from - cut.to }, other.furniture))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(gap, id)| serde_json::json!({"id":id.to_string(),"gap":compact::num(gap)}))
        };
        Some(serde_json::json!({"wall":wall.id.to_string(),
            "span":[compact::num(cut.from),compact::num(cut.to)],
            "width":compact::num(cut.to-cut.from),
            "to_start":compact::num(cut.from),
            "to_end":compact::num(wall.start.distance(wall.end)-cut.to),
            "previous":adjacent(true),"next":adjacent(false)}))
    }).collect();
    serde_json::json!({"basis":"wall_axis","hosts":hosts})
}

/// A point in the plan, or the id of something already drawn.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum Spot {
    /// `[x, y]` in cm.
    At([f64; 2]),
    /// An element id, e.g. `f828` or `w24`.
    Id(String),
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct MeasureParams {
    /// What to measure from: an id or `[x,y]`. Alone, reports the free floor
    /// on all four sides of that piece. Openings instead report wall-axis spans, distances to wall ends and neighboring openings; no floor clearances.
    from: Option<Spot>,
    /// What to measure to: an id or `[x,y]`.
    to: Option<Spot>,
    /// Restrict to one axis, `x` or `y`. Between two boxes this is the gap
    /// along that axis (negative when they overlap).
    axis: Option<String>,
    /// Sides to measure free floor on: `+x`, `-x`, `+y`, `-y`, or, relative
    /// to the piece, `front`, `back`, `left`, `right`.
    dirs: Option<Vec<String>>,
    /// Probe line: with `axis`, the other axis' coordinate. Reports every
    /// stretch a straight line crosses, free floor and solids alike.
    at: Option<f64>,
    /// Limits of the probe along `axis`, `[from, to]` cm. Default: the plan.
    range: Option<[f64; 2]>,
    /// Height band that counts, `[z0, z1]` cm above this storey's floor.
    /// Default `[0, 200]`: what a person walking through meets.
    z: Option<[f64; 2]>,
    /// With a probe: how much free floor has to be left along it, cm.
    gap: Option<f64>,
    /// With a probe and `gap`: the piece that would grow. The answer says
    /// how big it can be along the axis and what stops it there.
    grow: Option<String>,
}

#[tool_router(router = measure_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        description = "Tape measure over the plan, in cm. from=<id> alone: free floor on all four sides, {clear:{\"+y\":[cm,id,name]}} — dirs picks sides (+x -x +y -y, or front/back/left/right of the piece). from+to (ids or [x,y]): the distance between them, {cm}, or the gap along axis. axis+at: what a straight probe runs into, {spans:[[from,to,id,name]]} with id null for free floor — the answer to \"how wide is the corridor here, and between what\". Add gap=<cm to keep free> and grow=<id> and it also answers how big that piece can be along the axis before the gap is broken, and what stops it there: {fit:{free,max,blocked_by}} — the question a layout decision actually ends in, without trying a size and undoing it. z limits the height band that counts (default 0-200)."
    )]
    pub(crate) fn measure(
        &self,
        Parameters(p): Parameters<MeasureParams>,
    ) -> Result<String, ErrorData> {
        use newera_core::measure::{self, Axis, Dir};
        let doc = self.document.read();
        // Measurements are of one storey: a wall one floor up is not in the way.
        let home = doc.home().level_view(doc.home().current_level());
        let axis = match p.axis.as_deref() {
            Some(raw) => Some(Axis::parse(raw).ok_or_else(|| invalid("axis: x or y"))?),
            None => None,
        };
        let spot = |s: &Spot| -> Result<(Point2, Option<newera_core::ElementId>), ErrorData> {
            match s {
                Spot::At([x, y]) => Ok((Point2::new(*x, *y), None)),
                Spot::Id(raw) => {
                    let id: newera_core::ElementId =
                        raw.parse().map_err(|e| invalid(format!("{e}")))?;
                    let (min, max) = measure::element_bounds(&home, id)
                        .ok_or_else(|| invalid(format!("no {raw} on this storey")))?;
                    Ok((
                        Point2::new(f64::midpoint(min.x, max.x), f64::midpoint(min.y, max.y)),
                        Some(id),
                    ))
                }
            }
        };

        // Probe: what a straight line at `at` runs into, in order.
        if let Some(at) = p.at {
            let axis = axis.ok_or_else(|| invalid("at needs axis: x or y"))?;
            let z = p.z.map_or((0.0, 200.0), |[a, b]| (a, b));
            let spans: Vec<serde_json::Value> =
                measure::free_span(&home, axis, at, p.range.map(|[a, b]| (a, b)), z)
                    .into_iter()
                    .map(|s| {
                        serde_json::json!([
                            compact::num(s.from),
                            compact::num(s.to),
                            s.what.map(|w| w.id().to_string()),
                            s.name,
                        ])
                    })
                    .collect();
            let mut out = serde_json::json!({ "spans": spans });
            // "How deep can this be?" — the question every layout decision
            // ends in, and the one that used to be answered by trying a size,
            // reading the findings and undoing.
            if let Some(gap) = p.gap {
                let probe = measure::free_span(&home, axis, at, p.range.map(|[a, b]| (a, b)), z);
                let grow: Option<newera_core::ElementId> = match &p.grow {
                    Some(raw) => Some(raw.parse().map_err(|e| invalid(format!("{e}")))?),
                    None => None,
                };
                let span = probe.iter().fold((f64::MAX, f64::MIN), |(lo, hi), s| {
                    (lo.min(s.from), hi.max(s.to))
                });
                let total = (span.1 - span.0).max(0.0);
                let solid = |s: &&newera_core::Span| s.what.is_some();
                let mine = probe
                    .iter()
                    .position(|s| s.what.as_ref().map(newera_core::Solid::id) == grow);
                let taken: f64 = probe
                    .iter()
                    .enumerate()
                    .filter(|(i, s)| s.what.is_some() && Some(*i) != mine)
                    .map(|(_, s)| s.cm())
                    .sum();
                let held: f64 = mine.map_or(0.0, |i| probe[i].cm());
                // What stops it is the first solid across the free floor it
                // grows into — not the wall it already has its back to.
                let across = |list: &[newera_core::Span]| -> (f64, Option<newera_core::Span>) {
                    let free: f64 = list
                        .iter()
                        .take_while(|s| s.what.is_none())
                        .map(newera_core::Span::cm)
                        .sum();
                    (free, list.iter().find(solid).cloned())
                };
                let (ahead, blocks_ahead) = mine.map_or((0.0, None), |i| across(&probe[i + 1..]));
                let behind = mine.map_or((0.0, None), |i| {
                    let mut before: Vec<newera_core::Span> = probe[..i].to_vec();
                    before.reverse();
                    across(&before)
                });
                let against = if ahead >= behind.0 {
                    blocks_ahead
                } else {
                    behind.1
                };
                let max = (total - taken - gap).max(0.0);
                let fits = serde_json::json!({
                    "free": compact::num(total - taken - held),
                    "max": compact::num(max),
                    "blocked_by": against.map(|s| {
                        serde_json::json!([
                            s.what.as_ref().map(|w| w.id().to_string()),
                            s.name,
                            compact::num(s.from),
                            compact::num(s.to),
                        ])
                    }),
                });
                out["fit"] = fits;
            }
            return Ok(out.to_string());
        }

        let from = p
            .from
            .as_ref()
            .ok_or_else(|| invalid("from: an id, or [x,y]"))?;

        // Distance between two things.
        if let Some(to) = &p.to {
            let (a, a_id) = spot(from)?;
            let (b, b_id) = spot(to)?;
            let boxes = a_id
                .and_then(|id| measure::element_bounds(&home, id))
                .zip(b_id.and_then(|id| measure::element_bounds(&home, id)));
            let mut out = serde_json::Map::new();
            match (boxes, axis) {
                // Between two boxes the useful number is the gap, not the
                // distance between centers: it is what fits in between.
                (Some((ba, bb)), Some(axis)) => {
                    out.insert("cm".to_owned(), compact::num(measure::gap(ba, bb, axis)));
                    out.insert("axis".to_owned(), serde_json::json!(axis.name()));
                }
                (Some((ba, bb)), None) => {
                    out.insert("x".to_owned(), compact::num(measure::gap(ba, bb, Axis::X)));
                    out.insert("y".to_owned(), compact::num(measure::gap(ba, bb, Axis::Y)));
                }
                (None, Some(Axis::X)) => {
                    out.insert("cm".to_owned(), compact::num((b.x - a.x).abs()));
                }
                (None, Some(Axis::Y)) => {
                    out.insert("cm".to_owned(), compact::num((b.y - a.y).abs()));
                }
                (None, None) => {
                    out.insert("cm".to_owned(), compact::num((b.x - a.x).hypot(b.y - a.y)));
                }
            }
            return Ok(serde_json::Value::Object(out).to_string());
        }

        // Free floor around a piece.
        let Spot::Id(raw) = from else {
            return Err(invalid("free floor is measured around a piece: from=<id>"));
        };
        let id: newera_core::FurnitureId = raw.parse().map_err(|e| invalid(format!("{e}")))?;
        let piece = home
            .find_piece(id)
            .ok_or_else(|| invalid(format!("no {raw} on this storey")))?;
        if piece.is_opening() {
            return Ok(
                serde_json::json!({"id":raw,"opening":opening_measure(&home, piece)}).to_string(),
            );
        }
        let dirs: Vec<Dir> = match &p.dirs {
            Some(raw) => raw
                .iter()
                .map(|d| {
                    Dir::parse(d, Some(piece)).ok_or_else(|| {
                        invalid(format!("dir {d}: +x -x +y -y, front/back/left/right"))
                    })
                })
                .collect::<Result<_, _>>()?,
            None => Dir::PLAN.to_vec(),
        };
        let solids = measure::obstacles(&home, &|_| false);
        let mut clear = serde_json::Map::new();
        for dir in dirs {
            let c = measure::clearance_against(&solids, piece, dir, measure::MAX_REACH);
            clear.insert(
                dir.name().to_owned(),
                serde_json::json!([
                    compact::num(c.cm),
                    c.against.map(|s| s.id().to_string()),
                    c.name,
                ]),
            );
        }
        let (min, max) = measure::plan_bounds(piece);
        Ok(serde_json::json!({
            "id": raw,
            "bounds": [compact::point(min), compact::point(max)],
            "faces": measure::facing(piece),
            "clear": clear,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::server;

    #[test]
    fn opening_measure_uses_wall_frame_for_diagonal_and_split_hosts() {
        use newera_core::{Furniture, FurnitureId, Home, Point2, Wall, WallId};
        let mut home = Home::default();
        let mut opening = Furniture {
            id: FurnitureId(3),
            width: 100.0,
            depth: 20.0,
            height: 120.0,
            elevation: 100.0,
            opening: Some(newera_core::Opening::default()),
            ..Furniture::default()
        };
        let wall = Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(300.0, 400.0));
        newera_core::align_to_wall(&mut opening, &wall, 200.0);
        home.walls.push(wall);
        home.furniture.push(opening.clone());
        let value = opening_measure(&home, &opening);
        assert_eq!(value["hosts"][0]["width"], 100);
        assert_eq!(value["hosts"][0]["to_start"], 150);
        assert_eq!(value["hosts"][0]["to_end"], 250);
        // Split the same wall through the opening: report each actual cut.
        home.walls[0].end = Point2::new(120.0, 160.0);
        home.walls.push(Wall::new(
            WallId(2),
            Point2::new(120.0, 160.0),
            Point2::new(300.0, 400.0),
        ));
        let value = opening_measure(&home, &opening);
        assert_eq!(value["hosts"].as_array().unwrap().len(), 2);
        assert_eq!(value["hosts"][0]["width"], 50);
        assert_eq!(value["hosts"][1]["width"], 50);
        home.walls.clear();
        assert_eq!(
            opening_measure(&home, &opening)["hosts"]
                .as_array()
                .unwrap()
                .as_slice(),
            [] as [serde_json::Value; 0]
        );
    }

    #[test]
    fn hosted_openings_measure_jambs_and_neighbors_without_host_penetration() {
        let s = server();
        s.create(Parameters(
            serde_json::from_value(serde_json::json!({
                "walls":[{"pts":[[0,0],[1620,0]]}]
            }))
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_value(serde_json::json!({"items":[
                {"cat":"window","wall":"w1","along":210,"w":180,"h":140,"elev":110},
                {"cat":"window","wall":"w1","along":670,"w":180,"h":140,"elev":110}
            ]}))
            .unwrap(),
        ))
        .unwrap();
        let measured: serde_json::Value = serde_json::from_str(
            &s.measure(Parameters(
                serde_json::from_value(serde_json::json!({"from":"f2"})).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        assert!(measured.get("clear").is_none(), "{measured}");
        let host = &measured["opening"]["hosts"][0];
        assert_eq!(host["wall"], "w1");
        assert_eq!(host["width"], 180);
        assert_eq!(host["to_start"], 120);
        assert_eq!(host["to_end"], 1320);
        assert_eq!(host["next"]["id"], "f3");
        assert_eq!(host["next"]["gap"], 280);
        let preview: serde_json::Value = serde_json::from_str(
            &s.update(Parameters(
                serde_json::from_value(
                    serde_json::json!({"dry":true,"items":[{"id":"f2","at":[220,0]}]}),
                )
                .unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        let clear = &preview["clearances"]["f2"];
        assert!(clear.get("+x").is_none(), "{preview}");
        assert_eq!(clear["opening"]["hosts"][0]["to_start"], 130);
        assert_eq!(clear["opening"]["hosts"][0]["next"]["gap"], 270);
    }

    #[test]
    fn measure_answers_clearances_gaps_and_corridors() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}],
                    "rooms":[{"name":"Cozinha","at":[250,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        // Two counters facing each other across the room.
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[250,40],"w":300,"d":60,"h":90},
                             {"cat":"base-cabinet","at":[250,340],"w":300,"d":60,"h":90}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let measure = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.measure(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };

        // Free floor in front of the first counter: 70 → 310 is 240 cm.
        let clear = measure(r#"{"from":"f6"}"#);
        assert!(
            (clear["clear"]["+y"][0].as_f64().unwrap() - 240.0).abs() < 0.5,
            "{clear}"
        );
        assert_eq!(clear["clear"]["+y"][1], "f7", "{clear}");
        // And behind it, 2.5 cm to the inner face of a 15 cm wall.
        assert!(
            (clear["clear"]["-y"][0].as_f64().unwrap() - 2.5).abs() < 0.5,
            "{clear}"
        );
        assert_eq!(clear["clear"]["-y"][1], "w1", "{clear}");

        // The same number as the gap between the two boxes.
        let gap = measure(r#"{"from":"f6","to":"f7","axis":"y"}"#);
        assert!((gap["cm"].as_f64().unwrap() - 240.0).abs() < 0.5, "{gap}");

        // A probe down the middle: wall, counter, corridor, counter, wall.
        let spans = measure(r#"{"axis":"y","at":250}"#);
        let rows = spans["spans"].as_array().unwrap();
        let free: Vec<f64> = rows
            .iter()
            .filter(|r| r[2].is_null())
            .map(|r| r[1].as_f64().unwrap() - r[0].as_f64().unwrap())
            .collect();
        assert!(
            free.iter().any(|cm| (cm - 240.0).abs() < 0.5),
            "the corridor is one free stretch: {spans}"
        );

        // Two points, plainly.
        let straight = measure(r#"{"from":[0,0],"to":[30,40]}"#);
        assert!(
            (straight["cm"].as_f64().unwrap() - 50.0).abs() < 0.01,
            "{straight}"
        );
    }
    /// A partition written from centerline to centerline reaches 10 cm into
    /// the walls at its ends. That bit is wall, not floor, and nothing that
    /// measures, counts or fills may treat it as usable: not the room areas,
    /// not the tape measure, not the run a wall of cabinets is sized from,
    /// not the dimension of the wall's own face. Only the wall's own length
    /// stays as drawn — that is its axis, the way plans have always read it.
    #[test]
    fn nothing_counts_the_bit_inside_another_wall() {
        let s = server();
        // Room 600 x 400 in centerlines, walls 20 cm thick; a 10 cm partition
        // across it, written the way a description reads.
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true,"t":20},
                             {"pts":[[300,0],[300,400]],"t":10}],
                    "rooms":[{"name":"Esquerda","at":[150,200]},
                             {"name":"Direita","at":[450,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let home = s.document.read().home().clone();

        // Free floor of each room: 285 x 380 cm, face to face.
        for room in &home.rooms {
            let area = newera_core::polygon_area(&room.points) / 10_000.0;
            assert!((area - 10.83).abs() < 0.01, "{}: {area} m²", room.name);
        }

        // A tape measure across both rooms: floor, wall, floor, wall, floor.
        let measured: serde_json::Value = serde_json::from_str(
            &s.measure(Parameters(
                serde_json::from_str(r#"{"axis":"x","at":200,"range":[0,600]}"#).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        let spans: Vec<(f64, f64, bool)> = measured["spans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s[0].as_f64().unwrap(),
                    s[1].as_f64().unwrap(),
                    s[2].is_null(),
                )
            })
            .collect();
        assert_eq!(
            spans,
            vec![
                (0.0, 10.0, false),
                (10.0, 295.0, true),
                (295.0, 305.0, false),
                (305.0, 590.0, true),
                (590.0, 600.0, false),
            ],
            "{measured}"
        );

        // The run a wall of cabinets is sized from: the wall is 400 cm along
        // its axis, and 380 cm of it is free between the walls at its ends.
        let partition = home.walls.last().unwrap();
        let run =
            newera_core::wall_run(&home, partition.id, 1.0, 60.0, (0.0, 90.0), &|_| false).unwrap();
        assert!((run.length - 400.0).abs() < 1e-9);
        let gaps: Vec<(f64, f64)> = run.gaps().iter().map(|g| (g.0, g.1)).collect();
        assert_eq!(gaps, vec![(10.0, 390.0)], "{:?}", run.obstacles);

        // And its face measures what it shows: 380 cm.
        let mut doc = s.document.write();
        let face = newera_core::ops::wall_side_dimension(
            &mut doc,
            partition.id,
            newera_core::ops::WallSide::Outer,
            40.0,
        )
        .unwrap();
        assert!((face.start.distance(face.end) - 380.0).abs() < 1e-9);
    }

    /// "How deep can the counter be?" — asked once, instead of trying a size,
    /// reading five new findings and undoing.
    #[test]
    fn fit_answers_how_much_room_is_left_for_the_piece_that_grows() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,500],[0,500]],"closed":true,"t":15}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        // A counter against the top wall and a tall unit against the bottom
        // one, with the corridor in between.
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[300,37.5],"w":200,"d":60,"h":90},
                             {"cat":"wardrobe","at":[300,462.5],"w":200,"d":60,"h":220}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let fit: serde_json::Value = serde_json::from_str(
            &s.measure(Parameters(
                serde_json::from_str(
                    r#"{"axis":"y","at":300,"range":[0,500],"gap":85,"grow":"f5"}"#,
                )
                .unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        // Room 7,5..492,5 between the walls; the wardrobe takes 60 and 85 has
        // to stay free, so the counter can be 340.
        assert!(
            (fit["fit"]["max"].as_f64().unwrap() - 340.0).abs() < 0.5,
            "{fit}"
        );
        assert_eq!(fit["fit"]["blocked_by"][0], "f6", "{fit}");
    }
}
