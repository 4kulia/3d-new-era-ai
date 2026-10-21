//! Cabinets sized for a whole wall, and appliances fitted into them.
//!
//! Thin code over `newera-joinery`, but the behaviour it drives — free
//! stretches, blind corners, a sink that lands over its cabinet — is where
//! most of the tests in this crate live.

use newera_core::Point2;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::invalid;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct EmbedParams {
    /// Piece already in the plan to embed (id)…
    item: Option<String>,
    /// …or a new one from the catalog: `cooktop`, `sink-bowl`, `oven`, `microwave`.
    cat: Option<String>,
    /// Width of a new item, cm (a real product's measurements).
    w: Option<f64>,
    /// Depth of a new item, cm.
    d: Option<f64>,
    /// Height of a new item, cm.
    h: Option<f64>,
    /// Joinery countertop (sink, cooktop) or cabinet (oven, microwave: a niche).
    host: String,
    /// Center along the host's width from its left end, cm (default: where the item is, or the middle).
    at: Option<f64>,
    /// Niche floor above the room floor, cm (default: oven 80 and microwave 145 in towers).
    z: Option<f64>,
    /// Check and report only.
    #[serde(default)]
    dry: bool,
}
#[tool_router(router = cabinets_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        description = "Embed an item into joinery with an exact fit: a sink bowl or cooktop into a countertop (cutout from the item's size, generic fixture not drawn; the wooden top of a joinery cabinet under it is cut too), an oven, microwave or other appliance into a cabinet niche (doors above and below, boards around it), a TV onto a slatted panel at seated eye level (z = screen center). The item becomes part of the host and moves with it. item: id in the plan, or cat (+w/d/h) for a new one. Errors say what to change (e.g. use w = 61 no armário). Reply {host, item, kind, cutout|niche, x|bottom, cut_below, notes}."
    )]
    pub(crate) fn embed(
        &self,
        Parameters(p): Parameters<EmbedParams>,
    ) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        let host: newera_core::FurnitureId = p.host.parse().map_err(|e| invalid(format!("{e}")))?;
        let (item, existing) = match (&p.item, &p.cat) {
            (Some(id), _) => {
                let id: newera_core::FurnitureId =
                    id.parse().map_err(|e| invalid(format!("{e}")))?;
                let piece = doc
                    .home()
                    .furniture
                    .iter()
                    .find(|f| f.id == id)
                    .cloned()
                    .ok_or_else(|| invalid(format!("{id} not found (embed top-level pieces)")))?;
                (piece, true)
            }
            (None, Some(cat)) => {
                let entry = newera_catalog::find(cat).ok_or_else(|| {
                    invalid(format!("unknown catalog id `{cat}` (use the catalog tool)"))
                })?;
                let mut piece = entry.instantiate(doc.new_furniture_id(), Point2::default());
                piece.width = p.w.unwrap_or(piece.width);
                piece.depth = p.d.unwrap_or(piece.depth);
                piece.height = p.h.unwrap_or(piece.height);
                (piece, false)
            }
            (None, None) => return Err(invalid("give `item` (an id) or `cat` for a new one")),
        };
        let request = newera_joinery::EmbedRequest {
            item,
            existing,
            host,
            at: p.at,
            z: p.z,
            dry: p.dry,
        };
        newera_joinery::embed(&mut doc, &request)
            .map(|v| v.to_string())
            .map_err(invalid)
    }
    #[tool(
        description = "Fill a wall with cabinets sized for it: measures the free stretches between corners, doors, windows, fridge and stove, splits each into even modules (30-90 cm, no useless leftovers; 15-30 cm pull-outs, fillers under 15; the defaults target=60 max=90 sink_w=80 cooktop_w=60 are the nominal widths of EN 1116, which is what appliances and hardware are made for — bespoke widths in between are fine for a run that receives none), drawer unit beside the stove, countertop on base rows, cabinet over the fridge and hood gap on wall rows, wardrobes (hanging rails, shelves, drawers) on tall rows facing bedrooms; p.sink/p.cooktop place those cabinets and cutouts. Replaces the cabinets already there (keep ids stay). Reply {modules:[[id,role,from,w]],removed,notes}; dry plans only. Change one module afterwards with joinery id."
    )]
    pub(crate) fn cabinet_run(
        &self,
        Parameters(p): Parameters<newera_joinery::CabinetRunParams>,
    ) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        newera_joinery::cabinet_run(&mut doc, &p)
            .map(|v| v.to_string())
            .map_err(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::CreateParams;
    use crate::tools::check::CheckParams;
    use crate::tools::furniture::PlaceParams;
    use crate::tools::joinery::JoineryParams;
    use crate::tools::server;

    #[test]
    fn cabinet_runs_fill_a_kitchen_wall_around_its_appliances() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Cozinha","at":[200,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let place: PlaceParams = serde_json::from_str(
            r#"{"items":[{"cat":"fridge","wall":"w1","along":355},{"cat":"stove","wall":"w1","along":180},{"cat":"window","wall":"w1","along":80,"elev":110}]}"#,
        )
        .unwrap();
        s.place(Parameters(place)).unwrap();
        let run = |json: &str| -> serde_json::Value {
            let p: newera_joinery::CabinetRunParams = serde_json::from_str(json).unwrap();
            serde_json::from_str(&s.cabinet_run(Parameters(p)).unwrap()).unwrap()
        };
        let base = run(r#"{"wall":"w1"}"#);
        let modules = base["modules"].as_array().unwrap();
        let spans: Vec<(String, f64, f64)> = modules
            .iter()
            .map(|m| {
                (
                    m[1].as_str().unwrap().to_owned(),
                    m[2].as_f64().unwrap(),
                    m[3].as_f64().unwrap(),
                )
            })
            .collect();
        // Stove 150..210, fridge 320..390: cabinets stay clear of both with their gaps.
        for (role, from, w) in &spans {
            let to = from + w;
            assert!(
                to <= 148.1 || *from >= 211.9,
                "{role} {from}+{w} hits the stove: {spans:?}"
            );
            assert!(
                to <= 315.1,
                "{role} {from}+{w} takes the fridge air: {spans:?}"
            );
        }
        // The corner filler, then modules; the drawer unit touches the stove.
        assert_eq!(spans[0].0, "filler", "{spans:?}");
        let drawers = spans.iter().find(|m| m.0 == "drawers").unwrap();
        assert!(
            (drawers.1 + drawers.2 - 148.0).abs() < 0.2 || (drawers.1 - 212.0).abs() < 0.2,
            "{spans:?}"
        );
        assert!(base["notes"].to_string().contains("ventilação"), "{base}");
        // Nothing left over: every stretch is modules, pull-outs or fillers.
        let covered: f64 = spans
            .iter()
            .filter(|m| m.0 != "countertop")
            .map(|m| m.2)
            .sum();
        assert!(
            (covered - (148.0 - 7.5) - (315.0 - 212.0)).abs() < 0.3,
            "{covered} {spans:?}"
        );
        let issues = s.check_layout(Parameters(CheckParams::default())).unwrap();
        assert!(!issues.contains("overlap"), "{issues}");

        // Wall cabinets: split by the window, a gap for the hood, one over the fridge.
        // The wall named by a piece against it: the fridge (f6).
        let upper = run(r#"{"near":"f6","p":{"row":"wall"}}"#);
        assert_eq!(upper["wall"], "w1");
        let roles: Vec<&str> = upper["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m[1].as_str().unwrap())
            .collect();
        assert!(roles.contains(&"over"), "{upper}");
        assert!(upper["notes"].to_string().contains("coifa"), "{upper}");
        for m in upper["modules"].as_array().unwrap() {
            let (from, w) = (m[2].as_f64().unwrap(), m[3].as_f64().unwrap());
            assert!(from + w <= 30.1 || from >= 129.9, "window 30..130: {upper}");
            assert!(
                m[1] == "over" || from + w <= 150.1 || from >= 209.9,
                "hood: {upper}"
            );
        }

        // Again with two drawer units: the old base modules are replaced, not piled up.
        let before = s.document.read().home().furniture.len();
        let again = run(r#"{"wall":"w1","p":{"drawers":2,"front":"wood"}}"#);
        assert_eq!(
            again["removed"].as_array().unwrap().len(),
            modules.len(),
            "{again}"
        );
        assert_eq!(s.document.read().home().furniture.len(), before);
        assert_eq!(again["modules"].to_string().matches("drawers").count(), 2);
        // An L: the side wall's run stops at this one's countertop with a corner filler.
        let side = run(r#"{"wall":"w4"}"#);
        assert!(side["removed"].as_array().unwrap().is_empty(), "{side}");
        let last = side["modules"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|m| m[1] != "countertop")
            .unwrap()
            .clone();
        assert_eq!(last[1], "filler", "{side}");
        // w4 runs from y=300 up to y=0: w1's countertop front is at y = 7.5 + 58.
        assert!(
            (last[2].as_f64().unwrap() + last[3].as_f64().unwrap() - (300.0 - 65.5)).abs() < 0.6,
            "{side}"
        );
        // …and w1, planned again in the same step, turns its corner module blind.
        let adjusted = &side["adjusted"][0];
        assert_eq!(adjusted["wall"], "w1", "{side}");
        let corner = &adjusted["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m[1] != "countertop")
            .unwrap()
            .clone();
        assert_eq!(corner[1], "corner", "{adjusted}");
        assert!(
            (corner[2].as_f64().unwrap() - 7.5).abs() < 0.1,
            "{adjusted}"
        );
        let issues = s.check_layout(Parameters(CheckParams::default())).unwrap();
        assert!(!issues.contains("overlap"), "{issues}");
        s.document.write().undo().unwrap();
        // Planning w4 again with nothing new leaves w1 alone.
        run(r#"{"wall":"w4"}"#);
        let quiet = run(r#"{"wall":"w4"}"#);
        assert!(quiet.get("adjusted").is_none(), "{quiet}");
        assert!(!quiet["removed"].as_array().unwrap().is_empty(), "{quiet}");
        s.document.write().undo().unwrap();
        s.document.write().undo().unwrap();
        // Dry runs change nothing; one undo brings the previous run back.
        run(r#"{"wall":"w1","p":{"drawers":0},"dry":true}"#);
        assert_eq!(s.document.read().home().furniture.len(), before);
        s.document.write().undo().unwrap();
        let ids: Vec<String> = s
            .document
            .read()
            .home()
            .furniture
            .iter()
            .map(|f| f.id.to_string())
            .collect();
        assert!(ids.contains(&modules[1][0].as_str().unwrap().to_owned()));
        // A modulation nobody would build is built, with the note.
        let p: newera_joinery::CabinetRunParams =
            serde_json::from_str(r#"{"wall":"w1","p":{"max":10}}"#).unwrap();
        let odd = s.cabinet_run(Parameters(p)).unwrap();
        assert!(odd.contains("fora do usual"), "{odd}");
    }
    #[test]
    fn cabinet_runs_place_sink_and_cooktop_and_line_up_the_wall_row() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[420,0],[420,300],[0,300]],"closed":true}],"rooms":[{"name":"Cozinha","at":[210,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let place: PlaceParams = serde_json::from_str(
            r#"{"items":[{"cat":"fridge","wall":"w1","along":372},{"cat":"window","wall":"w1","along":130,"elev":110,"w":100}]}"#,
        )
        .unwrap();
        s.place(Parameters(place)).unwrap();
        let run = |json: &str| -> serde_json::Value {
            let p: newera_joinery::CabinetRunParams = serde_json::from_str(json).unwrap();
            serde_json::from_str(&s.cabinet_run(Parameters(p)).unwrap()).unwrap()
        };
        let base = run(r#"{"wall":"w1","p":{"sink":130,"cooktop":260}}"#);
        let modules = base["modules"].as_array().unwrap();
        let find = |role: &str| {
            modules
                .iter()
                .find(|m| m[1] == role)
                .unwrap_or_else(|| panic!("no {role}: {base}"))
        };
        let (sink, cooktop) = (find("sink"), find("cooktop"));
        let span = |m: &serde_json::Value| {
            (
                m[2].as_f64().unwrap(),
                m[2].as_f64().unwrap() + m[3].as_f64().unwrap(),
            )
        };
        assert!(span(sink).0 <= 90.0 && span(sink).1 >= 170.0, "{base}");
        assert!(
            span(cooktop).0 <= 230.0 && span(cooktop).1 >= 290.0,
            "{base}"
        );
        // The countertop carries both cutouts.
        let top_id = find("countertop")[0].as_str().unwrap().to_owned();
        let stored = s
            .document
            .read()
            .home()
            .furniture
            .iter()
            .find(|f| f.id.to_string() == top_id)
            .unwrap()
            .properties[newera_joinery::PARAMS_KEY]
            .clone();
        assert!(
            stored.contains("\"sink\"") && stored.contains("\"cooktop\""),
            "{stored}"
        );
        // Wall row: one hood gap over the cooktop, and the modules after it start where
        // the base cabinets do.
        let upper = run(r#"{"wall":"w1","p":{"row":"wall"}}"#);
        assert_eq!(
            upper["notes"].to_string().matches("coifa").count(),
            1,
            "{upper}"
        );
        let after_hood = upper["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m[1] == "doors" && m[2].as_f64().unwrap() > 200.0)
            .unwrap()
            .clone();
        assert!(
            (after_hood[2].as_f64().unwrap() - span(cooktop).1).abs() < 0.2,
            "{upper}"
        );
    }
    #[test]
    fn cabinet_runs_redo_hand_drawn_cabinets_around_the_appliances_in_them() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Cozinha","at":[200,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        // Imported-style pieces: cabinets and a cooktop known only by their names.
        let place: PlaceParams = serde_json::from_str(
            r#"{"items":[
                {"cat":"box","name":"Geladeira Electrolux 480 L","wall":"w1","along":45,"w":70,"d":72,"h":185},
                {"cat":"box","name":"7 — Armário portas ao lado cooktop","wall":"w1","along":160,"w":80,"d":58,"h":87},
                {"cat":"box","name":"Gavetões sob cooktop","wall":"w1","along":245,"w":90,"d":58,"h":87},
                {"cat":"box","name":"Cooktop Brastemp — 4 bocas","wall":"w1","along":245,"w":59,"d":48,"h":8,"elev":87},
                {"cat":"box","name":"Bancada contínua","wall":"w1","along":230,"w":220,"d":60,"h":3,"elev":87}
            ]}"#,
        )
        .unwrap();
        let ids = s.place(Parameters(place)).unwrap();
        let ids: Vec<String> = ids
            .rsplit('=')
            .next()
            .unwrap()
            .split(',')
            .map(str::to_owned)
            .collect();
        let p: newera_joinery::CabinetRunParams = serde_json::from_str(r#"{"near":"f6"}"#).unwrap();
        let reply: serde_json::Value =
            serde_json::from_str(&s.cabinet_run(Parameters(p)).unwrap()).unwrap();
        let removed = reply["removed"].to_string();
        // The cabinets and the old countertop go; the fridge and the cooktop stay.
        for id in [&ids[1], &ids[2], &ids[4]] {
            assert!(removed.contains(id.as_str()), "{id} not replaced: {reply}");
        }
        assert!(
            !removed.contains(&format!("\"{}\"", ids[0]))
                && !removed.contains(&format!("\"{}\"", ids[3])),
            "{reply}"
        );
        // The new drawer unit stands under the cooktop that was there.
        let cooktop = reply["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m[1] == "cooktop")
            .unwrap_or_else(|| panic!("{reply}"))
            .clone();
        let (from, w) = (cooktop[2].as_f64().unwrap(), cooktop[3].as_f64().unwrap());
        assert!(from <= 215.5 && from + w >= 274.5, "{reply}");
        assert!(
            reply["notes"].to_string().contains("Cooktop existente"),
            "{reply}"
        );
        assert!(
            reply["notes"]
                .to_string()
                .contains("ventilação da geladeira"),
            "{reply}"
        );
    }
    #[test]
    fn embedded_items_fit_their_host_and_survive_its_changes() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[420,0],[420,300],[0,300]],"closed":true}],"rooms":[{"name":"Cozinha","at":[210,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let run = |json: &str| -> serde_json::Value {
            let p: newera_joinery::CabinetRunParams = serde_json::from_str(json).unwrap();
            serde_json::from_str(&s.cabinet_run(Parameters(p)).unwrap()).unwrap()
        };
        run(r#"{"wall":"w1","p":{"cooktop":260}}"#);
        // Planned again without parameters, the cooktop stays where it was.
        let base = run(r#"{"wall":"w1"}"#);
        assert!(base["modules"].to_string().contains("cooktop"), "{base}");
        let top = base["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m[1] == "countertop")
            .unwrap()[0]
            .as_str()
            .unwrap()
            .to_owned();
        let embed = |json: &str| -> Result<serde_json::Value, String> {
            let p: EmbedParams = serde_json::from_str(json).unwrap();
            s.embed(Parameters(p))
                .map(|r| serde_json::from_str(&r).unwrap())
                .map_err(|e| e.message.to_string())
        };
        // A real 5-burner cooktop, 75 × 50, where the generic one was.
        let reply = embed(&format!(
            r#"{{"cat":"cooktop","w":75,"d":50,"h":6,"host":"{top}","at":252.5}}"#
        ))
        .unwrap();
        assert_eq!(reply["cutout"], serde_json::json!([71, 46]), "{reply}");
        let cooktop_id = reply["item"].as_str().unwrap().to_owned();
        let host_of = |id: &str| {
            s.document
                .read()
                .home()
                .furniture
                .iter()
                .find(|f| f.children.iter().any(|c| c.id.to_string() == id))
                .map(|f| {
                    (
                        f.id.to_string(),
                        f.properties[newera_joinery::PARAMS_KEY].clone(),
                    )
                })
        };
        let (host, params) = host_of(&cooktop_id).expect("embedded");
        assert_eq!(host, top);
        assert_eq!(
            params.matches("\"cooktop\"").count(),
            1,
            "one cooktop hole: {params}"
        );
        // Planning the wall again keeps the real cooktop in the new countertop.
        let again = run(r#"{"wall":"w1","p":{"drawers":2}}"#);
        let (new_host, params) = host_of(&cooktop_id).unwrap_or_else(|| panic!("lost: {again}"));
        assert_ne!(new_host, top);
        assert!(params.contains("\"drawn\":false"), "{params}");
        // An oven into a tower: too narrow first, then it fits and follows a change.
        let tower: serde_json::Value = serde_json::from_str(
            &s.joinery(Parameters(serde_json::from_str::<JoineryParams>(r#"{"kind":"cabinet","p":{"w":55,"h":220,"d":58,"plinth":10},"wall":"w4","along":200}"#).unwrap())).unwrap(),
        )
        .unwrap();
        let tower_id = tower["id"].as_str().unwrap().to_owned();
        // Too narrow for the oven: embedded all the same, with the width
        // that would hold it — the drawing is the user's to decide about.
        let tight = embed(&format!(
            r#"{{"cat":"oven","host":"{tower_id}","dry":true}}"#
        ))
        .unwrap();
        assert!(tight.to_string().contains("w = 61"), "{tight}");
        s.joinery(Parameters(
            serde_json::from_str::<JoineryParams>(&format!(
                r#"{{"id":"{tower_id}","p":{{"w":64}}}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        let oven = embed(&format!(r#"{{"cat":"oven","host":"{tower_id}"}}"#)).unwrap();
        let oven_id = oven["item"].as_str().unwrap().to_owned();
        s.joinery(Parameters(
            serde_json::from_str::<JoineryParams>(&format!(
                r#"{{"id":"{tower_id}","p":{{"shelves":3}}}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(
            host_of(&oven_id).map(|h| h.0),
            Some(tower_id.clone()),
            "the oven stays in the tower"
        );
    }
    #[test]
    fn cutouts_land_over_their_cabinets_on_walls_run_either_way() {
        let s = server();
        // Counter-clockwise walls: the kitchen side of w2 is to its right.
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[0,300],[420,300],[420,0]],"closed":true}],"rooms":[{"name":"Cozinha","at":[210,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        for wall in ["w1", "w2", "w3", "w4"] {
            let p: newera_joinery::CabinetRunParams = serde_json::from_str(&format!(
                r#"{{"wall":"{wall}","p":{{"cooktop":210,"sink":80}}}}"#
            ))
            .unwrap();
            let Ok(reply) = s.cabinet_run(Parameters(p)) else {
                continue;
            };
            let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
            let doc = s.document.read();
            let find = |role: &str| {
                let id = reply["modules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m[1] == role)?[0]
                    .as_str()?
                    .to_owned();
                doc.home()
                    .furniture
                    .iter()
                    .find(|f| f.id.to_string() == id)
                    .cloned()
            };
            let (Some(top), Some(cooktop)) = (find("countertop"), find("cooktop")) else {
                continue;
            };
            let params: newera_joinery::Build =
                serde_json::from_str(&top.properties[newera_joinery::PARAMS_KEY]).unwrap();
            let newera_joinery::Build::Countertop(t) = params else {
                panic!()
            };
            let cut = t
                .cutouts
                .iter()
                .find(|c| c.kind == newera_joinery::CutoutKind::Cooktop)
                .unwrap();
            let hole = top.to_plan((cut.x - t.length / 2.0, 0.0));
            // The hole is over the cooktop's drawer unit, whichever way the wall runs.
            assert!(
                hole.distance(cooktop.position) < 35.0,
                "{wall}: hole {hole:?} vs cabinet {:?}",
                cooktop.position
            );
        }
    }

    #[test]
    fn a_run_cuts_the_tops_its_sink_and_cooktop_go_through() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[420,0],[420,300],[0,300]],"closed":true}],"rooms":[{"name":"Cozinha","at":[210,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let p: newera_joinery::CabinetRunParams =
            serde_json::from_str(r#"{"wall":"w1","p":{"sink":130,"cooktop":260}}"#).unwrap();
        let base: serde_json::Value =
            serde_json::from_str(&s.cabinet_run(Parameters(p)).unwrap()).unwrap();
        let doc = s.document.read();
        for role in ["sink", "cooktop"] {
            let row = base["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m[1] == role)
                .unwrap_or_else(|| panic!("no {role}: {base}"));
            let id: newera_core::FurnitureId = row[0].as_str().unwrap().parse().unwrap();
            let group = doc.home().find_piece(id).unwrap();
            let params: serde_json::Value =
                serde_json::from_str(&group.properties[newera_joinery::PARAMS_KEY]).unwrap();
            let cut = params["top_cutout"]
                .as_array()
                .unwrap_or_else(|| panic!("{role} cabinet top not cut: {params}"));
            // Centered on the hole along the wall: 130 and 260 cm.
            let from = row[2].as_f64().unwrap();
            let center = from + cut[0].as_f64().unwrap() + cut[2].as_f64().unwrap() / 2.0;
            let want = if role == "sink" { 130.0 } else { 260.0 };
            assert!((center - want).abs() < 0.2, "{role}: {center} {params}");
            assert!(
                !group.children.iter().any(|c| c.name == "Tampo"),
                "{role}: the whole board is gone"
            );
        }
    }

    #[test]
    fn a_piece_given_a_wall_stands_on_that_walls_storey() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Cozinha","at":[200,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        // The upper storey is the one shown while the ground floor gets work.
        let (ground, upper) = {
            let mut doc = s.document.write();
            let upper = newera_core::ops::add_level(&mut doc, None, None).unwrap();
            (doc.home().base_level(), Some(upper))
        };
        assert_eq!(s.document.read().home().current_level(), upper);
        let place: PlaceParams = serde_json::from_str(
            r#"{"items":[{"cat":"door","wall":"w2","along":150},{"cat":"outlet-low","wall":"w4","along":150}]}"#,
        )
        .unwrap();
        s.place(Parameters(place)).unwrap();
        let joinery: JoineryParams =
            serde_json::from_str(r#"{"kind":"cabinet","p":{"w":80,"h":90,"d":57},"wall":"w3"}"#)
                .unwrap();
        s.joinery(Parameters(joinery)).unwrap();
        let p: newera_joinery::CabinetRunParams = serde_json::from_str(r#"{"wall":"w1"}"#).unwrap();
        let run: serde_json::Value =
            serde_json::from_str(&s.cabinet_run(Parameters(p)).unwrap()).unwrap();
        assert!(!run["modules"].as_array().unwrap().is_empty(), "{run}");
        let doc = s.document.read();
        let home = doc.home();
        assert_ne!(home.furniture, [] as [newera_core::Furniture; 0]);
        for piece in &home.furniture {
            assert_eq!(
                home.resolve_level(piece.level),
                ground,
                "{} {} is on the storey of its wall",
                piece.id,
                piece.name
            );
        }
        // And the storey shown is still the one it was.
        assert_eq!(home.current_level(), upper);
    }

    #[test]
    fn an_item_set_into_joinery_takes_its_own_finish() {
        let s = server();
        let joinery: JoineryParams = serde_json::from_str(
            r#"{"kind":"countertop","p":{"length":120,"depth":55,"height":90},"at":[100,100]}"#,
        )
        .unwrap();
        s.joinery(Parameters(joinery)).unwrap();
        let top = s.document.read().home().furniture[0].id.to_string();
        let p: EmbedParams =
            serde_json::from_str(&format!(r#"{{"cat":"sink-bowl","host":"{top}"}}"#)).unwrap();
        let reply: serde_json::Value =
            serde_json::from_str(&s.embed(Parameters(p)).unwrap()).unwrap();
        let bowl = reply["item"].as_str().unwrap().to_owned();
        let update = |json: String| {
            s.update(Parameters(serde_json::from_str(&json).unwrap()))
                .map_err(|e| e.message.to_string())
        };
        update(format!(
            r#"{{"items":[{{"id":"{bowl}","color":[20,20,20],"mat":"marble #202020 60"}}]}}"#
        ))
        .unwrap();
        // A board the host builds is still the host's to change.
        let board = s.document.read().home().furniture[0]
            .children
            .iter()
            .find(|c| !c.properties.contains_key(newera_joinery::EMBED_KEY))
            .unwrap()
            .id
            .to_string();
        let refused = update(format!(
            r#"{{"items":[{{"id":"{board}","color":[20,20,20]}}]}}"#
        ))
        .unwrap_err();
        assert!(refused.contains("belongs to the group"), "{refused}");
        assert!(
            !refused.contains("mat, opacity"),
            "a board takes no finish: {refused}"
        );
        // The item is told it takes its finish, and nothing else is said.
        let told = update(format!(r#"{{"items":[{{"id":"{bowl}","w":10}}]}}"#)).unwrap_err();
        assert!(
            told.contains(
                "takes only name, brand, model_name, url, product, purchase, layer, color, mat, opacity"
            ),
            "{told}"
        );
        assert_eq!(told.matches("takes only").count(), 1, "{told}");
        // Rebuilt longer, the host carries the bowl with its finish.
        let joinery: JoineryParams =
            serde_json::from_str(&format!(r#"{{"id":"{top}","p":{{"length":150}}}}"#)).unwrap();
        s.joinery(Parameters(joinery)).unwrap();
        let doc = s.document.read();
        let bowl = doc
            .home()
            .find_piece(bowl.parse().unwrap())
            .expect("still in its host");
        assert_eq!(bowl.color, Some([20, 20, 20]));
        assert!(bowl.texture.as_ref().is_some_and(|t| t.pattern.is_some()));
    }
}
