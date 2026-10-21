//! Putting catalog pieces into the plan, and moving them around in batches.

use newera_core::Point2;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::{background_scale, core, invalid, ok, on_variant};
use crate::edit::{self, PlaceSpec};

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PlaceParams {
    /// The pieces to place, one object each: `id` from the catalog (or
    /// `copy`), and where (`at`, or `wall` + `along`).
    items: Vec<PlaceSpec>,
    /// Fields every item takes unless it sets them (cat, w, d, h, color, mat…).
    defaults: Option<PlaceSpec>,
    /// Coordinates (`at`, `into`, `a`, `b`, `along`) are pixels of the background image.
    #[serde(default)]
    px: bool,
    /// Plan version (tab) to write to; switches to it first.
    v: Option<usize>,
    /// Try it without applying; see `update`. `"summary"` answers short.
    dry: Option<crate::tools::reply::Dry>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct ArrangeParams {
    /// What to do; `array` makes copies in a row.
    #[schemars(extend("enum" = ["array", "align", "distribute", "flip", "rotate", "mirror", "group", "ungroup", "front", "back"]))]
    action: String,
    /// The elements to act on (ungroup: the group).
    ids: Vec<String>,
    /// array: number of copies (default 1).
    n: Option<usize>,
    /// array: step per copy along x, cm.
    dx: Option<f64>,
    /// array: step per copy along y, cm.
    dy: Option<f64>,
    /// array: step per copy upwards, cm (furniture and labels).
    dz: Option<f64>,
    /// rotate: pivot `[x,y]` (default the center of the elements).
    about: Option<Point2>,
    /// rotate: clockwise degrees.
    angle: Option<f64>,
    /// mirror: the first point of the mirror line.
    a: Option<Point2>,
    /// mirror: the second point of the mirror line.
    b: Option<Point2>,
    /// rotate/mirror: keep the originals and transform a copy.
    #[serde(default)]
    copy: bool,
    /// group: its name.
    name: Option<String>,
    /// align/distribute: `x` or `y`.
    axis: Option<String>,
    /// align: which edge to line up — `low` (left, or top of the plan),
    /// `middle`, `high`.
    edge: Option<String>,
    /// align: the coordinate, cm, to put that edge on.
    value: Option<f64>,
    /// distribute: centimeters between one piece and the next (default 0).
    gap: Option<f64>,
}
/// Where each placed piece that has a front ended up looking, and which
/// ones were turned to put their back on a wall: ` faces=f3:+y(seat)
/// turned=f3:back to w2`. The angle alone never said it; this does.
fn faces_note(doc: &newera_core::Document, ids: &[String], turned: &[String]) -> String {
    let home = doc.home();
    let faces: Vec<String> = ids
        .iter()
        .filter_map(|raw| {
            let piece = home.find_piece(raw.parse().ok()?)?;
            let front = newera_core::front::of(&piece.catalog);
            front
                .stance
                .has_front()
                .then(|| format!("{raw}:{}({})", newera_core::facing(piece), front.front))
        })
        .collect();
    let mut note = String::new();
    if !faces.is_empty() {
        note.push_str(" faces=");
        note.push_str(&faces.join(","));
    }
    if !turned.is_empty() {
        note.push_str(" turned=");
        note.push_str(&turned.join(","));
    }
    note
}

#[tool_router(router = furniture_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        description = "Place catalog items, or copy=<id> of a piece already in the project (model, finish and parts; `catalog` scope=project lists them), or import a model=<.obj/.gltf/.glb> file (unit=m|cm|mm|in), or model=product:<id>, a shop product of the hosted service. at=[x,y] is the center (doors and windows near a wall snap into it; into=[x,y] picks the swing side); wall=<id> (+along cm) puts doors and windows in a wall, or furniture with its back to it. facing=+x|-x|+y|-y|[x,y]|<id> turns the front toward a side, point or element, preferred to angle (clockwise degrees, y down: 0 front to +y, 90 to -x, 180 to -y, 270 to +x); a piece whose back belongs on a wall, put within 30 cm of one with neither, turns its back to it. The reply says where fronts ended: faces=f3:+y(seat), turned=f3:back to w2. Ceiling pieces hang from the ceiling the 3D draws and follow it: a pendant takes elev (shade bottom, ~150-160 over a table) or h (drop). w/d/h, pitch/roll, mat, opacity; defaults {…} fills every item; px=true reads background pixels. cat=beam with a,b=[x,y,z] makes rafters and posts. dry=true answers what it would add, the clearances and the findings, without writing."
    )]
    pub(crate) fn place(
        &self,
        Parameters(p): Parameters<PlaceParams>,
    ) -> Result<String, ErrorData> {
        let dry = crate::tools::reply::Dry::on(p.dry.as_ref());
        let brief = crate::tools::reply::Dry::brief(p.dry.as_ref());
        let mut items = edit::with_defaults(p.items, p.defaults.as_ref()).map_err(invalid)?;
        if p.px {
            let bg = {
                let doc = self.document.read();
                background_scale(&doc)?
            };
            for item in &mut items {
                item.at = item.at.map(|q| bg.point(q));
                item.into = item.into.map(|q| bg.point(q));
                if let Some(edit::Facing::At(q)) = &mut item.facing {
                    *q = bg.point(*q);
                }
                item.along = item.along.map(|v| v * bg.scale);
                for end in [&mut item.a, &mut item.b] {
                    *end = end.map(|[x, y, z]| {
                        let q = bg.point(Point2::new(x, y));
                        [q.x, q.y, z]
                    });
                }
            }
        }
        // A dry run does not switch version either: nothing about the plan,
        // or about what the user is looking at, moves.
        if dry {
            let doc = self.document.read();
            return crate::tools::reply::preview_with(&doc, brief, move |scratch| {
                edit::place(scratch, items).map(|_| ()).map_err(invalid)
            });
        }
        let mut doc = self.document.write();
        on_variant(&mut doc, p.v)?;
        let placed = edit::place_noting(&mut doc, items).map_err(invalid)?;
        // `ids=` stays last on the first line: it is what callers split the
        // reply on. What an import left out follows, a line per model.
        let note = faces_note(&doc, &placed.ids, &placed.turned);
        let mut reply = format!("{}{note} ids={}", ok(&doc, &[]), placed.ids.join(","));
        for line in &placed.imported {
            reply.push_str("\nimported ");
            reply.push_str(line);
            reply.push_str(" — `model` lists what it kept");
        }
        Ok(reply)
    }

    #[tool(
        description = "Arrange elements in one undo step. array {ids,n,dx,dy,dz} adds n copies stepping by dx/dy/dz cm (rafters, columns); align {ids,axis,edge:low|middle|high,value} lines pieces up by an edge — backs on one line, fronts on another — instead of by centers you work out yourself; distribute {ids,axis,gap?} sets them side by side in the order given, from where the first one is (a run of joinery); flip {ids} turns a piece back to front, rebuilding what is inside it (mirror only swaps left and right and leaves the front where it was); rotate {ids,angle clockwise,about?,copy?}; mirror {ids,a,b,copy?} across the line a-b; group {ids,name?} joins pieces into one box that moves/hides together, ungroup {ids:[group]}; front/back {ids} draws rooms or pieces on top/underneath (pool over deck, rug under sofa). Returns new ids."
    )]
    pub(crate) fn arrange(
        &self,
        Parameters(p): Parameters<ArrangeParams>,
    ) -> Result<String, ErrorData> {
        use newera_core::arrange::{self, Transform};
        let ids = edit::parse_ids(&p.ids).map_err(invalid)?;
        let mut doc = self.document.write();
        let center = || -> Point2 {
            let home = doc.home();
            let points: Vec<Point2> = ids
                .iter()
                .filter_map(|id| home.element(*id))
                .flat_map(|e| match e {
                    newera_core::Element::Wall(w) => vec![w.start, w.end],
                    newera_core::Element::Room(r) => r.points,
                    newera_core::Element::Dimension(d) => vec![d.start, d.end],
                    newera_core::Element::Label(l) => vec![l.position],
                    newera_core::Element::Polyline(l) => l.points,
                    newera_core::Element::Furniture(f) => f.footprint().to_vec(),
                    newera_core::Element::Level(_) => Vec::new(),
                })
                .collect();
            #[allow(clippy::cast_precision_loss)]
            let n = points.len().max(1) as f64;
            let (x, y) = points
                .iter()
                .fold((0.0, 0.0), |(x, y), q| (x + q.x, y + q.y));
            Point2::new(x / n, y / n)
        };
        let out: Vec<newera_core::ElementId> = match p.action.as_str() {
            "array" => {
                let (dx, dy, dz) = (
                    p.dx.unwrap_or(0.0),
                    p.dy.unwrap_or(0.0),
                    p.dz.unwrap_or(0.0),
                );
                #[allow(clippy::cast_precision_loss)]
                let step = |i: usize| Transform::Translate {
                    dx: dx * i as f64,
                    dy: dy * i as f64,
                    dz: dz * i as f64,
                };
                arrange::array(&mut doc, &ids, step, p.n.unwrap_or(1).clamp(1, 500))
                    .map_err(core)?
            }
            "rotate" => {
                let angle = p.angle.ok_or_else(|| invalid("`angle` is required"))?;
                let about = p.about.unwrap_or_else(center);
                arrange::apply(
                    &mut doc,
                    &ids,
                    Transform::Rotate {
                        about,
                        degrees: angle,
                    },
                    p.copy,
                )
                .map_err(core)?
            }
            "mirror" => {
                let (Some(a), Some(b)) = (p.a, p.b) else {
                    return Err(invalid("`a` and `b` are required"));
                };
                arrange::apply(&mut doc, &ids, Transform::Mirror { a, b }, p.copy).map_err(core)?
            }
            "group" => vec![
                arrange::group(&mut doc, &ids, p.name.as_deref().unwrap_or(""))
                    .map_err(core)?
                    .into(),
            ],
            "ungroup" => {
                let Some(newera_core::ElementId::Furniture(id)) = ids.first().copied() else {
                    return Err(invalid("`ids` must hold one group id"));
                };
                arrange::ungroup(&mut doc, id)
                    .map_err(core)?
                    .into_iter()
                    .map(Into::into)
                    .collect()
            }
            "align" | "distribute" => {
                let axis = match p.axis.as_deref().map(str::trim) {
                    Some("x" | "+x" | "-x") => newera_core::Axis::X,
                    Some("y" | "+y" | "-y") => newera_core::Axis::Y,
                    _ => return Err(invalid("`axis`: x or y")),
                };
                let pieces: Vec<newera_core::FurnitureId> = ids
                    .iter()
                    .map(|id| match id {
                        newera_core::ElementId::Furniture(f) => Ok(*f),
                        other => Err(invalid(format!("{other} is not a piece of furniture"))),
                    })
                    .collect::<Result<_, _>>()?;
                if p.action == "align" {
                    let edge = match p.edge.as_deref().map(str::trim) {
                        Some("low" | "left" | "top" | "back") => newera_core::arrange::Edge::Low,
                        Some("middle" | "center") => newera_core::arrange::Edge::Middle,
                        Some("high" | "right" | "bottom" | "front") => {
                            newera_core::arrange::Edge::High
                        }
                        _ => return Err(invalid("`edge`: low, middle or high")),
                    };
                    let value = p.value.ok_or_else(|| invalid("`value` is required"))?;
                    newera_core::arrange::align(&mut doc, &pieces, axis, edge, value)
                        .map_err(core)?
                } else {
                    newera_core::arrange::distribute(&mut doc, &pieces, axis, p.gap.unwrap_or(0.0))
                        .map_err(core)?
                }
            }
            // Back to front, the whole piece with it. `mirror` swaps left
            // and right and leaves the front where it was, which is a
            // different thing and the one that surprises people.
            "flip" => {
                let mut commands = Vec::new();
                for id in &ids {
                    let Some(newera_core::Element::Furniture(mut piece)) = doc.home().element(*id)
                    else {
                        return Err(invalid(format!("{id} is not a piece of furniture")));
                    };
                    piece.angle = (piece.angle + 180.0).rem_euclid(360.0);
                    commands.push(newera_core::Command::update(piece));
                }
                doc.execute(newera_core::Command::Batch { commands })
                    .map_err(core)?;
                ids.clone()
            }
            "front" | "back" => {
                arrange::reorder(&mut doc, &ids, p.action == "front").map_err(core)?;
                Vec::new()
            }
            other => return Err(invalid(format!("unknown action `{other}`"))),
        };
        let out: Vec<String> = out.iter().map(ToString::to_string).collect();
        Ok(ok(&doc, &out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use newera_core::Command;

    use crate::edit::CreateParams;
    use crate::tools::server;

    /// A 500 × 400 room, and a way to place into it that returns the reply
    /// and the pieces it made.
    fn room() -> crate::NewEraMcp {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s
    }

    fn placed(s: &crate::NewEraMcp, json: &str) -> (String, Vec<newera_core::Furniture>) {
        let reply = s
            .place(Parameters(serde_json::from_str(json).unwrap()))
            .unwrap();
        let home = s.document.read().home().clone();
        let pieces = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|id| home.find_piece(id.trim().parse().unwrap()).unwrap().clone())
            .collect();
        (reply, pieces)
    }

    #[test]
    fn a_piece_that_belongs_on_a_wall_turns_its_back_to_the_one_beside_it() {
        let s = room();
        let t = s.document.read().home().walls[0].thickness / 2.0;
        // A sofa dropped by the bottom wall with no angle: at 0 it would
        // look into that wall. Its back goes there, the gap it was given kept.
        let (reply, pieces) = placed(&s, r#"{"items":[{"cat":"sofa-3","at":[250,340]}]}"#);
        let sofa = &pieces[0];
        assert!((sofa.angle - 180.0).abs() < 1e-9, "{reply}");
        assert!(
            (sofa.position.y - 340.0).abs() < 1e-9,
            "{:?}",
            sofa.position
        );
        assert!(
            reply.contains(&format!("faces={}:-y(seat)", sofa.id)),
            "{reply}"
        );
        assert!(reply.contains("turned="), "{reply}");
        // A bed whose center is closer to the left wall than half its length:
        // turned, headboard on that wall, and taken out of it.
        let (_, pieces) = placed(&s, r#"{"items":[{"cat":"bed-double","at":[60,200]}]}"#);
        let bed = &pieces[0];
        assert!((bed.angle - 270.0).abs() < 1e-9, "{bed:?}");
        assert!(
            (bed.position.x - (t + bed.depth / 2.0)).abs() < 1e-9,
            "{bed:?}"
        );
        assert_eq!(newera_core::facing(bed), "+x");
        // In the middle of the room nothing is guessed, and an explicit
        // angle is always kept.
        let (reply, pieces) = placed(
            &s,
            r#"{"items":[{"cat":"sofa-2","at":[250,200]},{"cat":"sofa-2","at":[250,340],"angle":0}]}"#,
        );
        assert!(pieces.iter().all(|p| p.angle.abs() < 1e-9), "{reply}");
        assert!(!reply.contains("turned="), "{reply}");
        // So a sofa told to face the wall is reported, with the angle that fixes it.
        let layout = s
            .check_layout(Parameters(crate::tools::check::CheckParams::default()))
            .unwrap();
        assert!(
            layout.contains("backwards") && layout.contains(&pieces[1].id.to_string()),
            "{layout}"
        );
    }

    #[test]
    fn facing_turns_the_front_toward_a_side_a_point_or_an_element() {
        let s = room();
        let (_, tv) = placed(&s, r#"{"items":[{"cat":"tv","at":[250,30]}]}"#);
        let tv = tv[0].id.to_string();
        let (reply, pieces) = placed(
            &s,
            &format!(
                r#"{{"items":[
                    {{"cat":"armchair","at":[250,250],"facing":"{tv}"}},
                    {{"cat":"chair","at":[100,200],"facing":"+x"}},
                    {{"cat":"chair","at":[400,200],"facing":[100,200]}},
                    {{"cat":"office-chair","at":[250,150],"facing":"-x"}}
                ]}}"#
            ),
        );
        let sides: Vec<&str> = pieces.iter().map(newera_core::facing).collect();
        assert_eq!(sides, ["-y", "+x", "-x", "-x"], "{reply}");
        assert!((pieces[1].angle - 270.0).abs() < 1e-9);
        // With a wall, facing picks which side of it: into the room.
        let wall = s.document.read().home().walls[0].id.to_string();
        let (_, pieces) = placed(
            &s,
            &format!(r#"{{"items":[{{"cat":"wardrobe","wall":"{wall}","facing":[250,200]}}]}}"#),
        );
        assert_eq!(newera_core::facing(&pieces[0]), "+y");
        assert!(pieces[0].position.y > 0.0, "{:?}", pieces[0].position);
        // Both at once is a contradiction waiting to happen.
        let both = s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"chair","at":[100,100],"angle":0,"facing":"+x"}]}"#,
            )
            .unwrap(),
        ));
        assert!(both.is_err());
    }

    #[test]
    fn a_piece_in_the_project_is_copied_with_its_model_finish_and_parts() {
        let s = server();
        {
            let mut doc = s.document.write();
            // A crown moulding imported long ago: its file is no longer on disk.
            let crown = newera_core::Furniture {
                id: newera_core::FurnitureId(1),
                catalog: "imported".into(),
                name: "arremate de madeira".into(),
                model: Some("51/crown.obj".into()),
                texture: Some(newera_core::Material {
                    color: Some([150, 110, 70]),
                    ..newera_core::Material::default()
                }),
                position: newera_core::Point2::new(100.0, 10.0),
                width: 200.0,
                depth: 4.0,
                height: 8.0,
                elevation: 272.0,
                ..newera_core::Furniture::default()
            };
            let part = |id: u64, x: f64| newera_core::Furniture {
                id: newera_core::FurnitureId(id),
                catalog: "box".into(),
                name: format!("lateral {id}"),
                position: newera_core::Point2::new(x, 100.0),
                width: 2.0,
                depth: 60.0,
                height: 220.0,
                ..newera_core::Furniture::default()
            };
            let mut tower = part(2, 30.0);
            tower.name = "torre".into();
            tower.width = 60.0;
            tower.children = vec![part(3, 1.0), part(4, 59.0)];
            doc.execute(newera_core::Command::Batch {
                commands: vec![
                    newera_core::Command::insert(crown),
                    newera_core::Command::insert(tower),
                ],
            })
            .unwrap();
        }
        let place = |json: &str| {
            s.place(Parameters(serde_json::from_str(json).unwrap()))
                .unwrap()
        };
        place(r#"{"items":[{"copy":"f1","at":[400,10],"w":120},{"copy":"f2","at":[330,100]}]}"#);
        let doc = s.document.read();
        let home = doc.home();
        assert_eq!(home.furniture.len(), 4);
        let crown = &home.furniture[2];
        assert_ne!(crown.id.0, 1, "a new id");
        assert_eq!(crown.model.as_deref(), Some("51/crown.obj"));
        assert_eq!(crown.texture, home.furniture[0].texture, "the same finish");
        assert!((crown.width - 120.0).abs() < 1e-9 && (crown.elevation - 272.0).abs() < 1e-9);
        assert!((crown.position.x - 400.0).abs() < 1e-9);
        let tower = &home.furniture[3];
        let xs: Vec<f64> = tower.children.iter().map(|c| c.position.x).collect();
        assert_eq!(xs, vec![301.0, 359.0], "the parts came along");
        assert!(
            tower.children.iter().all(|c| c.id.0 > 4),
            "parts renumbered"
        );
        assert!(
            (home.furniture[1].children[0].position.x - 1.0).abs() < 1e-9,
            "original untouched"
        );
        drop(doc);

        // The two ways reached for first mean the same copy: the embedded
        // model path catalog(scope=project) gives, and the id in `cat`.
        place(r#"{"items":[{"model":"51/crown.obj","at":[100,300]},{"cat":"f2","at":[30,300]}]}"#);
        let doc = s.document.read();
        let home = doc.home();
        assert_eq!(home.furniture.len(), 6);
        assert_eq!(home.furniture[4].model.as_deref(), Some("51/crown.obj"));
        assert!(
            (home.furniture[4].width - 200.0).abs() < 1e-9,
            "the size of the piece copied"
        );
        assert_eq!(
            home.furniture[5].children.len(),
            2,
            "the tower with its parts"
        );
        drop(doc);
        let err = s
            .place(Parameters(
                serde_json::from_str(r#"{"items":[{"cat":"f999","at":[0,0]}]}"#).unwrap(),
            ))
            .unwrap_err();
        assert!(err.message.contains("copy=<its id>"), "{err:?}");
    }

    #[test]
    fn arrange_defaults_finishes_and_pixels() {
        let s = server();
        // Seven rafters from one template.
        let params: PlaceParams = serde_json::from_str(
            r#"{"defaults":{"cat":"box","w":8,"d":400,"h":18,"mat":"wood","color":[160,110,70]},
                "items":[{"at":[0,0]},{"at":[100,0],"opacity":0.4}]}"#,
        )
        .unwrap();
        let reply = s.place(Parameters(params)).unwrap();
        let ids: Vec<String> = reply
            .split("ids=")
            .nth(1)
            .unwrap()
            .split(',')
            .map(str::to_owned)
            .collect();
        {
            let doc = s.document.read();
            let f = &doc.home().furniture;
            assert!(
                f.iter()
                    .all(|p| p.catalog == "box" && (p.depth - 400.0).abs() < 1e-9)
            );
            assert_eq!(
                f[0].texture.as_ref().and_then(|t| t.pattern),
                Some(newera_core::Pattern::Wood)
            );
            assert_eq!(f[1].opacity, Some(0.4));
        }
        let arr = s
            .arrange(Parameters(ArrangeParams {
                action: "array".into(),
                ids: vec![ids[0].clone()],
                n: Some(5),
                dx: Some(100.0),
                ..ArrangeParams::default()
            }))
            .unwrap();
        assert_eq!(arr.split("ids=").nth(1).unwrap().split(',').count(), 5);
        assert_eq!(s.document.read().home().furniture.len(), 7);
        s.arrange(Parameters(ArrangeParams {
            action: "rotate".into(),
            ids: vec![ids[1].clone()],
            angle: Some(90.0),
            ..ArrangeParams::default()
        }))
        .unwrap();
        assert!((s.document.read().home().furniture[1].angle - 90.0).abs() < 1e-9);
        let grouped = s
            .arrange(Parameters(ArrangeParams {
                action: "group".into(),
                ids: ids.clone(),
                name: Some("Caibros".into()),
                ..ArrangeParams::default()
            }))
            .unwrap();
        assert_eq!(s.document.read().home().furniture.len(), 6);
        let group = grouped.split("ids=").nth(1).unwrap().to_owned();
        s.arrange(Parameters(ArrangeParams {
            action: "mirror".into(),
            ids: vec![group.clone()],
            a: Some(Point2::new(-100.0, 0.0)),
            b: Some(Point2::new(-100.0, 10.0)),
            copy: true,
            ..ArrangeParams::default()
        }))
        .unwrap();
        assert_eq!(s.document.read().home().furniture.len(), 7);
        assert!(
            s.arrange(Parameters(ArrangeParams {
                action: "spin".into(),
                ..ArrangeParams::default()
            }))
            .is_err()
        );

        // Pixels of a background at 2 cm/px offset by (100, 50).
        assert!(
            s.create(Parameters(
                serde_json::from_str(r#"{"px":true,"walls":[{"pts":[[0,0],[10,0]]}]}"#).unwrap()
            ))
            .is_err()
        );
        {
            let mut doc = s.document.write();
            doc.execute(Command::SetBackground {
                background: Some(newera_core::BackgroundImage {
                    path: "plan.png".into(),
                    size_px: [1000, 800],
                    cm_per_px: 2.0,
                    offset: Point2::new(100.0, 50.0),
                    opacity: 0.5,
                    visible: true,
                    ..Default::default()
                }),
            })
            .unwrap();
        }
        let params: CreateParams =
            serde_json::from_str(r#"{"px":true,"walls":[{"pts":[[0,0],[150,0]]}]}"#).unwrap();
        s.create(Parameters(params)).unwrap();
        let doc = s.document.read();
        let wall = doc.home().walls.last().unwrap();
        assert_eq!(
            (wall.start, wall.end),
            (Point2::new(100.0, 50.0), Point2::new(400.0, 50.0))
        );
    }

    /// Lining a run of joinery up by its back, setting the modules side by
    /// side, and turning one back to front: three sentences that used to be
    /// arithmetic with centers, and where a plan quietly drifted.
    #[test]
    fn align_distribute_and_flip_say_it_in_the_words_of_the_drawing() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true,"t":15}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[100,100],"w":60,"d":60,"h":90},
                             {"cat":"base-cabinet","at":[200,140],"w":80,"d":60,"h":90},
                             {"cat":"base-cabinet","at":[320,90],"w":40,"d":60,"h":90}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let arrange = |json: &str| {
            s.arrange(Parameters(serde_json::from_str(json).unwrap()))
                .unwrap()
        };
        // Backs on y = 60, whatever depth each one has.
        arrange(r#"{"action":"align","ids":["f5","f6","f7"],"axis":"y","edge":"low","value":60}"#);
        let home = s.document.read().home().clone();
        for piece in &home.furniture {
            let (min, _) = newera_core::plan_bounds(piece);
            assert!((min.y - 60.0).abs() < 1e-6, "{} at {min:?}", piece.id);
        }

        // Side by side from where the first one is, no gaps: one run.
        arrange(r#"{"action":"distribute","ids":["f5","f6","f7"],"axis":"x"}"#);
        let home = s.document.read().home().clone();
        let mut edges: Vec<(f64, f64)> = home
            .furniture
            .iter()
            .map(|f| {
                let (min, max) = newera_core::plan_bounds(f);
                (min.x, max.x)
            })
            .collect();
        edges.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!((edges[0].0 - 70.0).abs() < 1e-6, "{edges:?}");
        for pair in edges.windows(2) {
            assert!((pair[1].0 - pair[0].1).abs() < 1e-6, "{edges:?}");
        }

        // And back to front, which `mirror` does not do.
        let before = s.document.read().home().furniture[0].angle;
        arrange(r#"{"action":"flip","ids":["f5"]}"#);
        let after = s.document.read().home().furniture[0].angle;
        assert!((after - (before + 180.0).rem_euclid(360.0)).abs() < 1e-6);
    }

    /// Placing a door the wrong way round used to be found by the next
    /// check; asked first, it is found before anything is written.
    #[test]
    fn place_can_be_asked_before_it_is_done() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true,"t":15}],
                    "rooms":[{"name":"Quarto","at":[200,150]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let before = s.document.read().revision();
        let dry: serde_json::Value = serde_json::from_str(
            &s.place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"bed-double","at":[200,150]}],"dry":"summary"}"#,
                )
                .unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(dry["dry"], true, "{dry}");
        assert_eq!(dry["added_count"], 1, "{dry}");
        assert!(dry["clearances"].is_object(), "{dry}");
        assert_eq!(
            s.document.read().revision(),
            before,
            "nothing was written: {dry}"
        );
    }
}
