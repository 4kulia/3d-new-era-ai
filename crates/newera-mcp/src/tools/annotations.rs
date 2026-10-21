//! What the drawing says about itself: dimensions, notes, legends, and the
//! electrical and plumbing layers drawn over the plan.
//!
//! A cote that stopped matching the wall it measures is worse than no cote,
//! so `stale` hunts for them.

use newera_core::Command;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::{core, invalid, ok, write_action};
use crate::compact;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct AnnotationParams {
    /// Dimensions and notes that no longer match the drawing: rows
    /// [id, written, measured, against, text]. A plan of joinery is read
    /// off its notes, so one that still says 66,5 over a corridor of 86 is
    /// worse than no note at all.
    stale: Option<bool>,
    /// Tie every straight dimension to what its ends touch right now, so from
    /// here on they follow the drawing. Run it while the numbers are right.
    anchor: Option<bool>,
    /// Search label text, accent- and case-insensitive, e.g. `porta`; or
    /// `re:<pattern>` for a regular expression.
    q: Option<String>,
    /// Show engineering dimension chains (`auto_dimensions` in the project JSON).
    #[serde(alias = "auto_dimensions")]
    dims: Option<bool>,
    /// Show the room reference schedule and tags (`references` in the JSON).
    #[serde(alias = "references")]
    refs: Option<bool>,
    /// Include brand, model and link in references (`reference_details`).
    #[serde(alias = "reference_details")]
    details: Option<bool>,
    /// Convert the automatic dimension chains into editable dimensions.
    bake: Option<bool>,
    /// Number the schedule again in reading order, closing the gaps pieces
    /// left behind. Numbers are otherwise kept by each piece for good.
    renumber: Option<bool>,
    /// Legend of electrical/plumbing symbols with counts.
    legend: Option<bool>,
}
/// What the annotations tool reads.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnnotationReadParams {
    /// Dimensions and notes that no longer match the drawing: rows
    /// [id, written, measured, against, text].
    stale: Option<bool>,
    /// Search label text, accent- and case-insensitive, e.g. `porta`; or
    /// `re:<pattern>` for a regular expression.
    q: Option<String>,
}
/// What `edit_annotations` changes.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnnotationEditParams {
    /// Tie every straight dimension to what its ends touch right now, so from
    /// here on they follow the drawing. Run it while the numbers are right.
    anchor: Option<bool>,
    /// Show engineering dimension chains (`auto_dimensions` in the project JSON).
    #[serde(alias = "auto_dimensions")]
    dims: Option<bool>,
    /// Show the room reference schedule and tags (`references` in the JSON).
    #[serde(alias = "references")]
    refs: Option<bool>,
    /// Include brand, model and link in references (`reference_details`).
    #[serde(alias = "reference_details")]
    details: Option<bool>,
    /// Convert the automatic dimension chains into editable dimensions.
    bake: Option<bool>,
    /// Number the schedule again in reading order, closing the gaps pieces
    /// left behind. Numbers are otherwise kept by each piece for good.
    renumber: Option<bool>,
    /// Legend of electrical/plumbing symbols with counts.
    legend: Option<bool>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct DisciplineParams {
    /// What to do with discipline `d`.
    #[schemars(extend("enum" = ["select", "show", "hide"]))]
    action: Option<String>,
    /// Whether the 3D shows everything (true) or hides what the plan hides
    /// (false). Applies with any action.
    show_all_3d: Option<bool>,
    /// `electrical`, `plumbing` or `architecture`; for show/hide also a
    /// plan layer: `lighting`, `appliances` or `joinery`.
    d: Option<String>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisciplineReadParams {
    /// Default `active`.
    #[schemars(extend("enum" = ["active", "quantities"]))]
    action: Option<String>,
}
#[tool_router(router = annotations_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        name = "disciplines",
        description = "Electrical and plumbing projects and plan layers over the plan. active (default): {active, hidden, layers {key:{pieces, hidden}}} — the discipline new symbols go to, what is hidden, and the layers (lighting, appliances, joinery); home gives each piece its plan_layer. quantities: {electrical:[[kind,count,names?]], plumbing:[...], lines_cm:{...}} — grouped by catalog kind (every low outlet together), with the names of the points as the detail. Change what is shown with edit_disciplines."
    )]
    pub(crate) fn read_disciplines(
        &self,
        Parameters(p): Parameters<DisciplineReadParams>,
    ) -> Result<String, ErrorData> {
        let action = match p.action.as_deref().unwrap_or("active") {
            read @ ("active" | "quantities") => read,
            other => {
                return Err(invalid(format!(
                    "unknown action `{other}`: active or quantities (to change what is shown, use edit_disciplines)"
                )));
            }
        };
        self.disciplines(Parameters(DisciplineParams {
            action: Some(action.to_owned()),
            ..DisciplineParams::default()
        }))
    }
    #[tool(
        description = "Change what the plan shows. select {d: electrical|plumbing|architecture}: new symbols (catalog cat electrical/plumbing) and lines go there and the rest is dimmed; select architecture hides the electrical and plumbing projects (show brings one back); select a project shows it with the architecture dimmed. show/hide {d}, where d can also be a layer of the plan — lighting (lamps, spots, LED), appliances (fridge, stove, oven, hood, washer…) or joinery (cabinets, wardrobes, countertops): hidden from the plan, its exports and the 3D. The 3D follows the plan unless show_all_3d=true, which makes it show everything. Pieces are in a layer by what they are; update(layer=lighting|appliances|joinery|none) overrides it. What is shown now is the disciplines tool."
    )]
    pub(crate) fn edit_disciplines(
        &self,
        Parameters(p): Parameters<DisciplineParams>,
    ) -> Result<String, ErrorData> {
        // show_all_3d on its own is a change too, and needs no other action.
        if p.action.is_none() && p.show_all_3d.is_some() {
            return self.disciplines(Parameters(p));
        }
        write_action(
            p.action.as_deref(),
            &["select", "show", "hide"],
            "disciplines",
        )?;
        self.disciplines(Parameters(p))
    }
    /// Disciplines and plan layers: what is shown, and every change to it.
    pub(crate) fn disciplines(
        &self,
        Parameters(p): Parameters<DisciplineParams>,
    ) -> Result<String, ErrorData> {
        use newera_core::Discipline;
        let mut doc = self.document.write();
        let parse = |raw: Option<&str>| -> Result<Option<Discipline>, ErrorData> {
            match raw {
                Some("electrical") => Ok(Some(Discipline::Electrical)),
                Some("plumbing") => Ok(Some(Discipline::Plumbing)),
                Some("architecture") => Ok(None),
                _ => Err(invalid(
                    "`d` must be electrical, plumbing or architecture — or, for show/hide, a layer: lighting, appliances or joinery",
                )),
            }
        };
        if let Some(all) = p.show_all_3d {
            doc.set_show_all_in_3d(all);
        }
        match p.action.as_deref().unwrap_or("active") {
            "active" => {}
            "select" => {
                let d = parse(p.d.as_deref())?;
                doc.choose_view(d);
            }
            "show" | "hide" => {
                let visible = p.action.as_deref() == Some("show");
                // Layers of the plan — lighting, appliances, joinery — hide
                // from the drawing and, unless it shows all, from the 3D.
                if let Some(layer) = p.d.as_deref().and_then(newera_core::PlanLayer::parse) {
                    doc.set_layer_visible(layer, visible);
                } else {
                    let d = parse(p.d.as_deref())?
                        .ok_or_else(|| invalid("architecture is always shown"))?;
                    doc.set_discipline_visible(d, visible);
                }
            }
            "quantities" => {
                let home = doc.home();
                let mut out = serde_json::Map::new();
                let mut lengths = serde_json::Map::new();
                for d in Discipline::ALL {
                    let key = serde_json::to_value(d)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    // By what each point is, with the names — where each one
                    // goes — as the detail: 31 lines of "1" help nobody buy.
                    let mut counts: std::collections::BTreeMap<String, (usize, Vec<String>)> =
                        std::collections::BTreeMap::new();
                    for top in &home.furniture {
                        for piece in top.flatten() {
                            if piece.discipline.or(top.discipline) == Some(d) {
                                let kind = newera_catalog::quantity_name(piece);
                                let row = counts.entry(kind.clone()).or_default();
                                row.0 += 1;
                                if piece.name != kind {
                                    row.1.push(piece.name.clone());
                                }
                            }
                        }
                    }
                    out.insert(
                        key.clone(),
                        serde_json::json!(
                            counts
                                .into_iter()
                                .map(|(kind, (count, names))| if names.is_empty() {
                                    serde_json::json!([kind, count])
                                } else {
                                    serde_json::json!([kind, count, names])
                                })
                                .collect::<Vec<_>>()
                        ),
                    );
                    let length: f64 = home
                        .polylines
                        .iter()
                        .filter(|l| l.discipline == Some(d))
                        .map(|l| {
                            l.points
                                .windows(2)
                                .map(|s| s[0].distance(s[1]))
                                .sum::<f64>()
                        })
                        .sum();
                    lengths.insert(key, compact::num(length));
                }
                out.insert("lines_cm".into(), serde_json::Value::Object(lengths));
                return Ok(serde_json::Value::Object(out).to_string());
            }
            other => return Err(invalid(format!("unknown action `{other}`"))),
        }
        let home = doc.home();
        // What each layer holds, so hiding one is a decision, not a guess.
        let mut layers = serde_json::Map::new();
        for layer in newera_core::PlanLayer::ALL {
            let count = home
                .furniture
                .iter()
                .flat_map(|top| {
                    top.visible_leaves()
                        .into_iter()
                        .map(move |leaf| newera_core::layer_in_group(top, leaf))
                })
                .filter(|l| *l == Some(layer))
                .count();
            layers.insert(
                layer.key().to_owned(),
                serde_json::json!({"pieces": count, "hidden": home.hidden_layers.contains(&layer)}),
            );
        }
        Ok(serde_json::json!({
            "active": home.active_discipline,
            "hidden": home.hidden_disciplines,
            "layers": layers,
            "show_all_3d": home.show_all_in_3d,
        })
        .to_string())
    }
    #[tool(
        name = "annotations",
        description = "Plan annotations, read. stale=true lists what no longer matches: notes whose numbers differ from their piece, piece names whose sizes (`módulo 70 cm`, `80 × 60`) differ, dimensions whose anchor is gone, and unanchored ones left with an end in the air near a face — rows [id, written, measured, against, text]; checked {dims, dims_unanchored, labels, names} counts what was compared (nothing checked is not a clean plan), and unverified [[id, text]] lists sizes nothing can confirm. It is the check after moving geometry. A note is tied to its piece with update(id=t1, about=f5). q=<text> searches label text on every storey, q=re:<pattern> by regex. With neither, answers the schedule {dims,refs,details,legend,rooms:[[room,[[tag,name,w,d,h,brand?,model?,url?]]]]}. `edit_annotations` switches modes, ties dimensions and bakes chains; brand, model and url go on pieces with `update`."
    )]
    pub(crate) fn read_annotations(
        &self,
        Parameters(p): Parameters<AnnotationReadParams>,
    ) -> Result<String, ErrorData> {
        self.annotations(Parameters(AnnotationParams {
            stale: p.stale,
            q: p.q,
            ..AnnotationParams::default()
        }))
    }
    #[tool(
        description = "Change the plan annotations. anchor=true ties every straight dimension to what its ends touch now, so it is measured again on every change instead of drifting (tie them while the numbers are right); one already off its face is not tied and comes back in left [[id, written, measured, near]], and one whose anchor died with a deleted piece is tied again or released. Switch dims (dimension chains), refs (the room reference schedule, whose tags stay with their pieces; renumber=true numbers them again in reading order), details (brand/model/link in refs) and legend (symbols with counts): a switch answers with the modes, changed, and what it shows — chains [[from,to,cm]] for dims, symbols {discipline:[[name,count]]} for legend; refs=true returns the schedule {dims,refs,details,legend,rooms:[[room,[[tag,name,w,d,h,brand?,model?,url?]]]]}. bake=true turns the automatic chains into editable dimensions (ids returned). `annotations` reads them."
    )]
    pub(crate) fn edit_annotations(
        &self,
        Parameters(p): Parameters<AnnotationEditParams>,
    ) -> Result<String, ErrorData> {
        let p = AnnotationParams {
            anchor: p.anchor,
            dims: p.dims,
            refs: p.refs,
            details: p.details,
            bake: p.bake,
            renumber: p.renumber,
            legend: p.legend,
            ..AnnotationParams::default()
        };
        let asked = [
            p.anchor, p.dims, p.refs, p.details, p.bake, p.renumber, p.legend,
        ];
        if asked.iter().all(Option::is_none) {
            return Err(invalid(
                "nothing to change: anchor, bake, renumber, or a mode (dims, refs, details, legend); to read, use annotations",
            ));
        }
        self.annotations(Parameters(p))
    }
    /// Plan annotations: every read and change, as one call.
    pub(crate) fn annotations(
        &self,
        Parameters(p): Parameters<AnnotationParams>,
    ) -> Result<String, ErrorData> {
        if p.anchor.unwrap_or(false) {
            let mut doc = self.document.write();
            let view = doc.home().level_view(doc.home().current_level());
            let held = newera_core::anchor_dimensions(&view);
            let ids: Vec<String> = held
                .iter()
                .filter(|d| d.holds.is_some())
                .map(|d| d.id.to_string())
                .collect();
            // Held a piece that is gone and touch nothing now: let go.
            let released: Vec<String> = held
                .iter()
                .filter(|d| d.holds.is_none())
                .map(|d| d.id.to_string())
                .collect();
            // Anchoring reads the drawing as the intent, so a dimension the
            // drawing already moved away from is not tied to it: it is named,
            // with what it would measure, to be fixed by hand first.
            let left: Vec<serde_json::Value> = view
                .dimensions
                .iter()
                .filter_map(|d| {
                    let loose = newera_core::loose_end(&view, d)?;
                    Some(serde_json::json!([
                        d.id.to_string(),
                        compact::num(d.length()),
                        compact::num(loose.measured),
                        loose.near.id.to_string(),
                    ]))
                })
                .collect();
            if !held.is_empty() {
                let commands = held.into_iter().map(Command::update).collect();
                doc.execute(Command::Batch { commands }).map_err(core)?;
            }
            let mut out = serde_json::json!({"anchored": ids});
            if !released.is_empty() {
                out["released"] = serde_json::json!(released);
            }
            if !left.is_empty() {
                out["left"] = serde_json::json!(left);
            }
            return Ok(out.to_string());
        }
        if p.stale.unwrap_or(false) || p.q.is_some() {
            let doc = self.document.read();
            let view = doc.home().level_view(doc.home().current_level());
            let mut out = serde_json::Map::new();
            if p.stale.unwrap_or(false) {
                let check = newera_core::check_annotations(&view);
                let rows: Vec<serde_json::Value> = check
                    .stale
                    .into_iter()
                    .map(|s| {
                        serde_json::json!([
                            s.id.to_string(),
                            compact::num(s.drawn),
                            compact::num(s.measured),
                            s.against.map(|a| a.to_string()),
                            s.text,
                        ])
                    })
                    .collect();
                out.insert("stale".to_owned(), serde_json::json!(rows));
                // An empty list after comparing nothing is not a clean plan.
                out.insert(
                    "checked".to_owned(),
                    serde_json::json!({
                        "dims": check.dimensions,
                        "dims_unanchored": check.unanchored,
                        "labels": check.labels,
                        "names": check.names,
                    }),
                );
                if !check.unverified.is_empty() {
                    let rows: Vec<serde_json::Value> = check
                        .unverified
                        .into_iter()
                        .map(|(id, text)| serde_json::json!([id.to_string(), text]))
                        .collect();
                    out.insert("unverified".to_owned(), serde_json::json!(rows));
                }
            }
            if let Some(query) = &p.q {
                // `re:` searches by a pattern — the way to sweep a naming
                // convention like `[09]` — and anything else by the words,
                // accent- and case-insensitive. Every storey is searched.
                let matches: Box<dyn Fn(&str) -> bool> =
                    if let Some(pattern) = query.strip_prefix("re:") {
                        let re = regex_lite::Regex::new(pattern)
                            .map_err(|e| invalid(format!("q: {e}")))?;
                        Box::new(move |text| re.is_match(text))
                    } else {
                        let needle = newera_core::fold(query);
                        Box::new(move |text| newera_core::fold(text).contains(&needle))
                    };
                let rows: Vec<serde_json::Value> = doc
                    .home()
                    .labels
                    .iter()
                    .filter(|l| matches(&l.text))
                    .map(compact::label)
                    .collect();
                out.insert("labels".to_owned(), serde_json::json!(rows));
            }
            return Ok(serde_json::Value::Object(out).to_string());
        }
        let mut doc = self.document.write();
        if p.bake.unwrap_or(false) {
            let view = doc.home().level_view(doc.home().current_level());
            let mut commands = Vec::new();
            let mut ids = Vec::new();
            for mut dim in newera_core::auto_dimensions(&view) {
                dim.id = doc.new_dimension_id();
                ids.push(dim.id.to_string());
                commands.push(Command::insert(dim));
            }
            let mut annotations = doc.home().annotations;
            annotations.auto_dimensions = false;
            commands.push(Command::SetAnnotations { annotations });
            doc.execute(Command::Batch { commands }).map_err(core)?;
            return Ok(ok(&doc, &ids));
        }
        if p.renumber.unwrap_or(false) {
            let cleared = newera_core::cleared_references(doc.home());
            if !cleared.is_empty() {
                let commands = cleared.into_iter().map(Command::update).collect();
                doc.execute(Command::Batch { commands }).map_err(core)?;
            }
        }
        let was = doc.home().annotations;
        let mut next = was;
        next.auto_dimensions = p.dims.unwrap_or(next.auto_dimensions);
        next.references = p.refs.unwrap_or(next.references);
        next.reference_details = p.details.unwrap_or(next.reference_details);
        next.legend = p.legend.unwrap_or(next.legend);
        if next != was {
            doc.execute(Command::SetAnnotations { annotations: next })
                .map_err(core)?;
        }
        let modes = serde_json::json!({
            "rev": doc.revision(),
            "dims": next.auto_dimensions,
            "refs": next.references,
            "details": next.reference_details,
            "legend": next.legend,
        });
        // Switching a mode answers with the switch and what that mode shows,
        // not with the whole schedule every time: three switches used to cost
        // the same 126-row list three times, and none showed what was asked.
        let switched = p.dims.is_some() || p.legend.is_some() || p.details.is_some();
        if switched && p.refs != Some(true) {
            let mut out = modes;
            let changed: Vec<&str> = [
                ("dims", was.auto_dimensions != next.auto_dimensions),
                ("refs", was.references != next.references),
                ("details", was.reference_details != next.reference_details),
                ("legend", was.legend != next.legend),
            ]
            .into_iter()
            .filter_map(|(name, moved)| moved.then_some(name))
            .collect();
            out["changed"] = serde_json::json!(changed);
            let view = doc.home().level_view(doc.home().current_level());
            if p.dims == Some(true) {
                // The chains the plan now draws: [from, to, cm].
                let chains: Vec<serde_json::Value> = newera_core::auto_dimensions(&view)
                    .iter()
                    .map(|d| {
                        serde_json::json!([
                            compact::point(d.start),
                            compact::point(d.end),
                            compact::num(d.length())
                        ])
                    })
                    .collect();
                out["chains"] = serde_json::json!(chains);
            }
            if p.legend == Some(true) {
                // The symbols the legend counts: [name, count] per discipline.
                let mut legend = serde_json::Map::new();
                for d in newera_core::Discipline::ALL {
                    let mut counts: std::collections::BTreeMap<String, usize> =
                        std::collections::BTreeMap::new();
                    for top in &view.furniture {
                        for piece in top.flatten() {
                            if piece.discipline.or(top.discipline) == Some(d) {
                                *counts
                                    .entry(newera_catalog::quantity_name(piece))
                                    .or_default() += 1;
                            }
                        }
                    }
                    if !counts.is_empty() {
                        let key = serde_json::to_value(d)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        legend.insert(
                            key,
                            serde_json::json!(counts.into_iter().collect::<Vec<_>>()),
                        );
                    }
                }
                out["symbols"] = serde_json::Value::Object(legend);
            }
            return Ok(out.to_string());
        }
        let view = doc.home().level_view(doc.home().current_level());
        let rooms: Vec<serde_json::Value> = newera_core::room_references(&view)
            .into_iter()
            .map(|g| {
                let items: Vec<serde_json::Value> = g
                    .items
                    .iter()
                    .map(|i| {
                        let mut row = vec![
                            serde_json::json!(i.tag),
                            serde_json::json!(i.name),
                            compact::num(i.size[0]),
                            compact::num(i.size[1]),
                            compact::num(i.size[2]),
                        ];
                        if i.brand.is_some() || i.model.is_some() || i.url.is_some() {
                            row.extend([
                                serde_json::json!(i.brand),
                                serde_json::json!(i.model),
                                serde_json::json!(i.url),
                            ]);
                        }
                        serde_json::Value::Array(row)
                    })
                    .collect();
                serde_json::json!([g.name, items])
            })
            .collect();
        let mut out = modes;
        out["rooms"] = serde_json::json!(rooms);
        Ok(out.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{CreateParams, UpdateSpec};
    use crate::tools::elements::UpdateParams;
    use crate::tools::furniture::PlaceParams;
    use crate::tools::read::GetHomeParams;
    use crate::tools::render::RenderParams;
    use crate::tools::server;

    #[test]
    fn a_dimension_anchored_on_a_part_of_a_group_is_alive_and_follows_it() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}],
                    "dims":[{"a":[250,7.5],"b":[250,98.5]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        // A pull-out broom cupboard grouped from drawn solids; its front is
        // what the dimension ends on, 91 cm from the wall.
        {
            let mut doc = s.document.write();
            let board = |id: u64, name: &str, y: f64, d: f64| newera_core::Furniture {
                id: newera_core::FurnitureId(id),
                catalog: "box".into(),
                name: name.to_owned(),
                position: newera_core::Point2::new(250.0, y),
                width: 30.0,
                depth: d,
                height: 195.0,
                ..newera_core::Furniture::default()
            };
            let mut cupboard = board(100, "vassoureiro", 60.0, 77.0);
            cupboard.children = vec![
                board(101, "corpo", 52.0, 89.0),
                board(102, "frente 30 × 195", 97.5, 2.0),
            ];
            doc.execute(newera_core::Command::insert(cupboard)).unwrap();
        }
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        let anchored = annotations(r#"{"anchor":true}"#);
        assert_eq!(
            anchored["anchored"].as_array().unwrap().len(),
            1,
            "{anchored}"
        );
        let holds = s.document.read().home().dimensions[0]
            .holds
            .clone()
            .unwrap();
        assert!(
            holds.iter().any(|h| h.id.to_string() == "f102"),
            "{holds:?}"
        );

        let stale = annotations(r#"{"stale":true}"#);
        assert_eq!(
            stale["stale"],
            serde_json::json!([]),
            "the front is there: {stale}"
        );
        assert_eq!(stale["checked"]["dims"], 1, "{stale}");

        // And it follows the front when the cupboard moves.
        s.move_elements(Parameters(
            serde_json::from_str(r#"{"ids":["f100"],"dy":10,"dx":0}"#).unwrap(),
        ))
        .unwrap();
        let length = s.document.read().home().dimensions[0].length();
        assert!((length - 101.0).abs() < 0.01, "{length}");
    }

    #[test]
    fn an_anchor_that_died_with_its_piece_is_tied_again_or_let_go() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}],
                    "dims":[{"a":[250,70],"b":[250,310]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let place = || {
            s.place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"base-cabinet","at":[250,40],"w":300,"d":60,"h":90}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
            s.document
                .read()
                .home()
                .furniture
                .last()
                .unwrap()
                .id
                .to_string()
        };
        let first = place();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[250,340],"w":300,"d":60,"h":90,"angle":180}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        let dim = s.document.read().home().dimensions[0].id.to_string();
        assert_eq!(
            annotations(r#"{"anchor":true}"#)["anchored"][0],
            dim.as_str()
        );

        // The cabinet goes and a new one takes its place: the number is right,
        // only the anchor is dead.
        s.delete(Parameters(
            serde_json::from_str(&format!(r#"{{"ids":["{first}"]}}"#)).unwrap(),
        ))
        .unwrap();
        let dead = annotations(r#"{"stale":true}"#);
        assert!(
            dead["stale"][0][4].as_str().unwrap().contains("is gone"),
            "{dead}"
        );
        let second = place();
        let again = annotations(r#"{"anchor":true}"#);
        assert_eq!(again["anchored"][0], dim.as_str(), "tied again: {again}");
        let clean = annotations(r#"{"stale":true}"#);
        assert_eq!(clean["stale"], serde_json::json!([]), "{clean}");
        let holds = s.document.read().home().dimensions[0]
            .holds
            .clone()
            .unwrap();
        assert!(
            holds.iter().any(|h| h.id.to_string() == second),
            "{holds:?}"
        );

        // Gone with nothing in its place: let go, not stale forever.
        s.delete(Parameters(
            serde_json::from_str(&format!(r#"{{"ids":["{second}"]}}"#)).unwrap(),
        ))
        .unwrap();
        let released = annotations(r#"{"anchor":true}"#);
        assert_eq!(released["released"][0], dim.as_str(), "{released}");
        let after = annotations(r#"{"stale":true}"#);
        assert!(
            after["stale"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| !r[4].as_str().unwrap().contains("is gone")),
            "{after}"
        );
        assert_eq!(after["checked"]["dims_unanchored"], 1, "{after}");
    }

    #[test]
    fn deleting_a_piece_names_the_labels_left_pointing_at_it() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"labels":[{"text":"[09]","at":[100,30]},{"text":"lixeira embutida","at":[400,400]},{"text":"[10]","at":[600,30]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"box","name":"nicho da lixeira","at":[100,30],"w":40,"d":60,"h":85},
                             {"cat":"box","name":"torre","at":[600,30],"w":60,"d":60,"h":220}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let (niche, labels) = {
            let doc = s.document.read();
            let home = doc.home();
            (
                home.furniture[0].id.to_string(),
                home.labels
                    .iter()
                    .map(|l| l.id.to_string())
                    .collect::<Vec<_>>(),
            )
        };
        s.update(Parameters(UpdateParams {
            items: serde_json::from_str(&format!(
                r#"[{{"id":"{}","about":"{niche}"}}]"#,
                labels[1]
            ))
            .unwrap(),
            rename: None,
            v: None,
            dry: None,
        }))
        .unwrap();
        let delete = |json: String| {
            s.delete(Parameters(serde_json::from_str(&json).unwrap()))
                .unwrap()
        };
        let reply = delete(format!(r#"{{"ids":["{niche}"]}}"#));
        let left: serde_json::Value =
            serde_json::from_str(&reply[reply.find('{').expect(&reply)..]).unwrap();
        assert_eq!(
            left["labels_left"],
            serde_json::json!([[labels[0], "[09]"], [labels[1], "lixeira embutida"]]),
            "the one on the tower stays out of it: {reply}"
        );

        // The note about it is stale too, for whoever did not read the reply.
        let stale: serde_json::Value = serde_json::from_str(
            &s.annotations(Parameters(
                serde_json::from_str(r#"{"stale":true}"#).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(stale["stale"][0][0], labels[1].as_str(), "{stale}");

        // Or they go in the same step.
        s.document.write().undo().unwrap();
        let reply = delete(format!(r#"{{"ids":["{niche}"],"labels":true}}"#));
        assert!(reply.contains("labels_deleted"), "{reply}");
        let doc = s.document.read();
        assert_eq!(doc.home().labels.len(), 1);
        assert_eq!(doc.home().labels[0].text, "[10]");
    }

    #[test]
    fn a_dimension_the_counter_moved_away_from_is_caught_and_not_anchored() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}],
                    "dims":[{"a":[250,91.5],"b":[250,310]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        // An 84 cm counter on the top wall and a 60 cm one on the bottom
        // wall: the dimension reads the corridor between them.
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[250,49.5],"w":300,"d":84,"h":90},
                             {"cat":"base-cabinet","at":[250,340],"w":300,"d":60,"h":90,"angle":180}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        let before = annotations(r#"{"stale":true}"#);
        assert_eq!(before["stale"], serde_json::json!([]), "{before}");
        assert_eq!(before["checked"]["dims"], 0, "{before}");
        assert_eq!(
            before["checked"]["dims_unanchored"], 1,
            "an empty list over an unanchored dimension says so: {before}"
        );

        // The counter is recessed to 65 cm; the dimension still says 218.5.
        let (counter, dim) = {
            let doc = s.document.read();
            (
                doc.home().furniture[0].id.to_string(),
                doc.home().dimensions[0].id.to_string(),
            )
        };
        s.update(Parameters(UpdateParams {
            items: serde_json::from_str(&format!(
                r#"[{{"id":"{counter}","d":65,"anchor":"back"}}]"#
            ))
            .unwrap(),
            rename: None,
            v: None,
            dry: None,
        }))
        .unwrap();
        let after = annotations(r#"{"stale":true}"#);
        let row = &after["stale"][0];
        assert_eq!(row[0], dim.as_str(), "{after}");
        assert_eq!(row[1], 218.5, "{after}");
        assert_eq!(row[2], 237.5, "what the corridor measures now: {after}");
        assert_eq!(row[3], counter.as_str(), "{after}");

        // Anchoring now would freeze the wrong number: it is left out, named.
        let anchored = annotations(r#"{"anchor":true}"#);
        assert_eq!(anchored["anchored"], serde_json::json!([]), "{anchored}");
        assert_eq!(anchored["left"][0][0], dim.as_str(), "{anchored}");
        assert_eq!(anchored["left"][0][2], 237.5, "{anchored}");
        assert!(
            s.document.read().home().dimensions[0].holds.is_none(),
            "not tied to the wrong face"
        );
    }

    #[test]
    fn stale_says_how_much_it_compared_and_reads_the_names() {
        let s = server();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        // Index codes only: nothing to compare, and the answer says so.
        s.create(Parameters(
            serde_json::from_str(r#"{"labels":[{"text":"[09]","at":[900,900]}]}"#).unwrap(),
        ))
        .unwrap();
        let empty = annotations(r#"{"stale":true}"#);
        assert_eq!(empty["stale"], serde_json::json!([]), "{empty}");
        assert_eq!(
            empty["checked"],
            serde_json::json!({"dims": 0, "dims_unanchored": 0, "labels": 0, "names": 0}),
            "{empty}"
        );

        // A joiner's plan keeps its sizes in the names.
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[
                    {"cat":"box","name":"Cuba — módulo 70 cm","at":[35,30],"w":64.5,"d":60,"h":92},
                    {"cat":"box","name":"Torre quente 60 × 60 × 220; nicho 61 × 87 cm","at":[200,30],"w":60,"d":60,"h":220},
                    {"cat":"box","name":"Aéreo de 118 cm; duas folhas de 59 cm","at":[400,30],"w":118,"d":35,"h":70}
                ]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let ids: Vec<String> = s
            .document
            .read()
            .home()
            .furniture
            .iter()
            .map(|f| f.id.to_string())
            .collect();
        let report = annotations(r#"{"stale":true}"#);
        let rows = report["stale"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{report}");
        assert_eq!(
            rows[0][0],
            ids[0].as_str(),
            "the sink was narrowed: {report}"
        );
        assert_eq!(rows[0][1], 70, "{report}");
        assert_eq!(rows[0][2], 64.5, "{report}");
        assert_eq!(report["checked"]["names"], 3, "{report}");
        let unverified = report["unverified"].as_array().unwrap();
        let said = |id: &str, text: &str| {
            unverified
                .iter()
                .any(|r| r[0] == id && r[1].as_str().unwrap().contains(text))
        };
        assert!(said(&ids[1], "nicho 61 × 87"), "{report}");
        assert!(said(&ids[2], "folhas de 59"), "{report}");
    }

    #[test]
    fn annotations_report_the_notes_that_stopped_being_true() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}],
                    "dims":[{"a":[250,70],"b":[250,310]}],
                    "labels":[{"text":"TORRE 300 × 60 × 90","at":[250,40]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[250,40],"w":300,"d":60,"h":90},
                             {"cat":"base-cabinet","at":[250,340],"w":300,"d":60,"h":90}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        // The dimension marks the corridor and still agrees with it.
        assert!(
            annotations(r#"{"stale":true}"#)["stale"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{:?}",
            annotations(r#"{"stale":true}"#)
        );

        // Tie the dimension to the two counters it runs between, while the
        // drawing and the number still agree.
        let anchored = annotations(r#"{"anchor":true}"#);
        assert_eq!(anchored["anchored"][0], "d5", "{anchored}");

        // Deepen the counter: the dimension follows it, the note does not.
        s.update(Parameters(UpdateParams {
            items: serde_json::from_str(r#"[{"id":"f7","d":100,"anchor":"back"}]"#).unwrap(),
            rename: None,
            v: None,
            dry: None,
        }))
        .unwrap();
        let home: serde_json::Value = serde_json::from_str(
            &s.get_home(Parameters(
                serde_json::from_str(r#"{"kinds":["dims"]}"#).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        let dim = &home["dims"][0];
        assert_eq!(dim["a"], serde_json::json!([250, 110]), "{home}");
        assert!(
            (dim["len"].as_f64().unwrap() - 200.0).abs() < 0.5,
            "the dimension measured itself again: {home}"
        );

        let stale = annotations(r#"{"stale":true}"#);
        let rows = stale["stale"].as_array().unwrap();
        assert!(
            !rows.iter().any(|r| r[0] == "d5"),
            "a dimension that follows the drawing is never stale: {stale}"
        );
        let note = rows
            .iter()
            .find(|r| r[0] == "t6")
            .unwrap_or_else(|| panic!("{stale}"));
        assert_eq!(note[3], "f7", "the piece the note sits on: {stale}");
        assert!(
            (note[1].as_f64().unwrap() - 60.0).abs() < 0.01,
            "written: {stale}"
        );
        assert!(
            (note[2].as_f64().unwrap() - 100.0).abs() < 0.01,
            "measured: {stale}"
        );

        // And notes can be found by their text, which no read could do.
        let found = annotations(r#"{"q":"torre"}"#);
        assert_eq!(found["labels"].as_array().unwrap().len(), 1, "{found}");
        assert_eq!(
            annotations(r#"{"q":"varanda"}"#)["labels"]
                .as_array()
                .unwrap()
                .as_slice(),
            [] as [serde_json::Value; 0]
        );
    }
    #[test]
    fn electrical_project_over_the_plan() {
        let s = server();
        s.disciplines(Parameters(DisciplineParams {
            action: Some("select".into()),
            d: Some("electrical".into()),
            show_all_3d: None,
        }))
        .unwrap();
        // Outlets and switches go into walls.
        let walls: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[200,0],[200,200],[0,200]],"closed":true}]}"#,
        )
        .unwrap();
        s.create(Parameters(walls)).unwrap();
        let params: PlaceParams = serde_json::from_str(
            r#"{"items":[{"cat":"outlet-low","at":[10,10]},{"cat":"outlet-low","at":[60,10]},{"cat":"switch","at":[100,10]}]}"#,
        )
        .unwrap();
        s.place(Parameters(params)).unwrap();
        let lines: CreateParams =
            serde_json::from_str(r#"{"polylines":[{"pts":[[10,10],[110,10]]}]}"#).unwrap();
        s.create(Parameters(lines)).unwrap();
        let home = s.get_home(Parameters(GetHomeParams::default())).unwrap();
        assert!(home.contains(r#""layer":"electrical""#), "{home}");
        let q = s
            .disciplines(Parameters(DisciplineParams {
                action: Some("quantities".into()),
                d: None,
                show_all_3d: None,
            }))
            .unwrap();
        assert!(q.contains(r#"["Tomada baixa (30 cm)",2]"#), "{q}");
        // Points named for where they go still count together, names as detail.
        {
            let mut doc = s.document.write();
            let mut named = doc.home().furniture[0].clone();
            named.name = "Tomada — cozinha, bancada (centro)".into();
            doc.execute(Command::update(named)).unwrap();
        }
        let q = s
            .disciplines(Parameters(DisciplineParams {
                action: Some("quantities".into()),
                d: None,
                show_all_3d: None,
            }))
            .unwrap();
        assert!(
            q.contains(r#"["Tomada baixa (30 cm)",2,["Tomada — cozinha, bancada (centro)"]]"#),
            "{q}"
        );
        assert!(q.contains(r#""electrical":100"#), "{q}");
        let png = s
            .render_plan(Parameters(RenderParams {
                w: Some(200),
                h: Some(150),
                ..RenderParams::default()
            }))
            .unwrap();
        assert_ne!(png.content, [] as [rmcp::model::ContentBlock; 0]);
    }
    /// The note that lies is a legend beside its cabinet, not on it: the one
    /// a joiner reads to cut. Three rounds of a real session were spent
    /// finding those by eye.
    #[test]
    fn a_legend_beside_its_piece_is_checked_against_it() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true}],
                    "labels":[{"text":"07 Armário portas: 65,83 × 84 × 87","at":[300,150]},
                              {"text":"Escala 1:50","at":[560,380]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[300,200],"w":65.83,"d":84,"h":87}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        assert!(
            annotations(r#"{"stale":true}"#)["stale"]
                .as_array()
                .unwrap()
                .is_empty(),
            "the legend still matches the cabinet"
        );

        // The cabinet is narrowed and made shallower; the legend stays as
        // typed, which is how a plan ends up lying to the workshop.
        s.update(Parameters(UpdateParams {
            items: serde_json::from_str(r#"[{"id":"f7","w":59.85,"d":72}]"#).unwrap(),
            rename: None,
            v: None,
            dry: None,
        }))
        .unwrap();
        let stale = annotations(r#"{"stale":true}"#);
        let rows = stale["stale"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{stale}");
        assert_eq!(rows[0][0], "t5", "{stale}");
        assert_eq!(rows[0][3], "f7", "the piece it is about: {stale}");

        // Said outright, the tie holds wherever the note sits.
        s.update(Parameters(UpdateParams {
            items: serde_json::from_str(r#"[{"id":"t5","at":[560,20],"about":"f7"}]"#).unwrap(),
            rename: None,
            v: None,
            dry: None,
        }))
        .unwrap();
        let stale = annotations(r#"{"stale":true}"#);
        assert_eq!(stale["stale"].as_array().unwrap().len(), 1, "{stale}");

        // And a note about nothing in particular is nobody's business.
        assert!(
            !stale["stale"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r[0] == "t6"),
            "{stale}"
        );
    }

    #[test]
    fn lighting_appliances_and_joinery_are_layers_born_with_the_piece() {
        let s = server();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"pendant","at":[0,0]},{"cat":"fridge","at":[100,0]},
                             {"cat":"base-cabinet","at":[200,0]},{"cat":"sofa-3","at":[400,0]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let disciplines = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.disciplines(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        let active = disciplines("{}");
        assert_eq!(active["layers"]["lighting"]["pieces"], 1, "{active}");
        assert_eq!(active["layers"]["appliances"]["pieces"], 1, "{active}");
        assert_eq!(active["layers"]["joinery"]["pieces"], 1, "{active}");

        // Architecture shows architecture: the electrical project steps out,
        // from the plan and the 3D, until it is chosen or shown again.
        disciplines(r#"{"action":"select","d":"electrical"}"#);
        let architecture = disciplines(r#"{"action":"select","d":"architecture"}"#);
        assert_eq!(
            architecture["active"],
            serde_json::Value::Null,
            "{architecture}"
        );
        assert!(
            architecture["hidden"].to_string().contains("electrical"),
            "{architecture}"
        );
        let all = disciplines(r#"{"show_all_3d":true}"#);
        assert_eq!(all["show_all_3d"], true, "{all}");
        assert!(
            s.document
                .read()
                .home()
                .shown_in_3d(Some(newera_core::Discipline::Electrical), None)
        );
        disciplines(r#"{"show_all_3d":false}"#);

        let hidden = disciplines(r#"{"action":"hide","d":"lighting"}"#);
        assert_eq!(hidden["layers"]["lighting"]["hidden"], true, "{hidden}");
        assert!(
            s.document
                .read()
                .home()
                .hidden_layers
                .contains(&newera_core::PlanLayer::Lighting)
        );

        // Read back, each piece says its layer; one moved by hand says the new one.
        let sofa = s.document.read().home().furniture[3].id.to_string();
        s.update(Parameters(
            serde_json::from_str(&format!(
                r#"{{"items":[{{"id":"{sofa}","layer":"joinery"}}]}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        let home: serde_json::Value = serde_json::from_str(
            &s.get_home(Parameters(
                serde_json::from_str(r#"{"kinds":["furniture"]}"#).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        let layers: Vec<&str> = home["furniture"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["plan_layer"].as_str().unwrap_or("-"))
            .collect();
        assert_eq!(
            layers,
            ["lighting", "appliances", "joinery", "joinery"],
            "{home}"
        );
        let err = s
            .disciplines(Parameters(
                serde_json::from_str(r#"{"action":"hide","d":"kitchen"}"#).unwrap(),
            ))
            .unwrap_err();
        assert!(
            err.message.contains("lighting"),
            "the layers are named: {err:?}"
        );
    }

    #[test]
    fn a_reference_number_stays_with_its_piece() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true}],"rooms":[{"name":"Cozinha","at":[300,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"box","name":"mesa","at":[100,100]},{"cat":"box","name":"aéreo","at":[300,100]},{"cat":"box","name":"arremate","at":[500,100]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        let tags = |report: &serde_json::Value| -> Vec<(String, u64)> {
            report["rooms"][0][1]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| (r[1].as_str().unwrap().to_owned(), r[0].as_u64().unwrap()))
                .collect()
        };
        let first = tags(&annotations(r#"{"refs":true}"#));
        assert_eq!(
            first,
            vec![
                ("mesa".into(), 1),
                ("aéreo".into(), 2),
                ("arremate".into(), 3)
            ]
        );

        // The table goes; the others keep their numbers, gap and all.
        let table = s.document.read().home().furniture[0].id.to_string();
        s.delete(Parameters(
            serde_json::from_str(&format!(r#"{{"ids":["{table}"]}}"#)).unwrap(),
        ))
        .unwrap();
        // A new piece, placed before them in reading order, takes the next number.
        s.place(Parameters(
            serde_json::from_str(r#"{"items":[{"cat":"box","name":"banco","at":[50,50]}]}"#)
                .unwrap(),
        ))
        .unwrap();
        let later = tags(&annotations(r#"{"refs":true}"#));
        assert_eq!(
            later,
            vec![
                ("banco".into(), 4),
                ("aéreo".into(), 2),
                ("arremate".into(), 3)
            ]
        );

        // Undoing the new piece takes its number with it.
        s.document.write().undo().unwrap();
        assert!(
            s.document
                .read()
                .home()
                .furniture
                .iter()
                .all(|f| f.name != "banco")
        );

        // Asked for, the gaps close in reading order.
        let renumbered = tags(&annotations(r#"{"refs":true,"renumber":true}"#));
        assert_eq!(
            renumbered,
            vec![("aéreo".into(), 1), ("arremate".into(), 2)]
        );
    }

    #[test]
    fn a_mode_switch_answers_with_what_it_shows_not_the_schedule() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Sala","at":[200,150]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(r#"{"items":[{"cat":"sofa-3","at":[200,200]}]}"#).unwrap(),
        ))
        .unwrap();
        s.disciplines(Parameters(DisciplineParams {
            action: Some("select".into()),
            d: Some("electrical".into()),
            show_all_3d: None,
        }))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"outlet-low","at":[10,10]},{"cat":"outlet-low","at":[60,10]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let annotations = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.annotations(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };

        let legend = annotations(r#"{"legend":true}"#);
        assert!(legend.get("rooms").is_none(), "no schedule: {legend}");
        assert_eq!(legend["changed"], serde_json::json!(["legend"]), "{legend}");
        assert_eq!(
            legend["symbols"]["electrical"],
            serde_json::json!([["Tomada baixa (30 cm)", 2]]),
            "{legend}"
        );

        // The project JSON's name works as well as the MCP's.
        let dims = annotations(r#"{"auto_dimensions":true}"#);
        assert!(dims.get("rooms").is_none(), "{dims}");
        assert_eq!(dims["dims"], true, "{dims}");
        assert!(
            !dims["chains"].as_array().unwrap().is_empty(),
            "the chains drawn: {dims}"
        );

        // Asked for, the schedule comes.
        let refs = annotations(r#"{"references":true}"#);
        assert!(refs["rooms"].is_array(), "{refs}");
        let again = annotations(r#"{"legend":true}"#);
        assert_eq!(
            again["changed"],
            serde_json::json!([]),
            "nothing moved: {again}"
        );
    }

    #[test]
    fn annotations_list_rooms_with_details() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[500,0],[500,400],[0,400]],"closed":true}],"rooms":[{"name":"Sala","at":[250,200]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let place: PlaceParams =
            serde_json::from_str(r#"{"items":[{"cat":"sofa-3","at":[250,300]}]}"#).unwrap();
        let ids = s.place(Parameters(place)).unwrap();
        let id = ids.rsplit('=').next().unwrap().to_owned();
        let spec: UpdateSpec = serde_json::from_str(&format!(
            r#"{{"id":"{id}","brand":"Example Furniture","url":"https://example.com/sofa"}}"#
        ))
        .unwrap();
        s.update(Parameters(UpdateParams {
            items: vec![spec],
            rename: None,
            v: None,
            dry: None,
        }))
        .unwrap();
        let reply = s
            .annotations(Parameters(AnnotationParams {
                anchor: None,
                stale: None,
                q: None,
                dims: Some(true),
                refs: Some(true),
                details: Some(true),
                bake: None,
                legend: None,
                renumber: None,
            }))
            .unwrap();
        assert!(reply.contains(r#""rooms":[["Sala",[[1,"#), "{reply}");
        assert!(
            reply.contains("Example Furniture") && reply.contains(r#""dims":true"#),
            "{reply}"
        );
        let png = s.render_plan(Parameters(RenderParams {
            w: Some(320),
            h: Some(240),
            ..RenderParams::default()
        }));
        assert!(png.is_ok());
    }
    #[test]
    fn dimensions_by_intent_in_one_call() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Sala","at":[200,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let place: PlaceParams =
            serde_json::from_str(r#"{"items":[{"cat":"window","wall":"w1","along":200}]}"#)
                .unwrap();
        s.place(Parameters(place)).unwrap();
        let dims: CreateParams = serde_json::from_str(
            r#"{"dims":[{"wall":"w1","side":"out"},{"wall":"w1","side":"in"},{"wall":"w1","chain":true,"off":70},{"room":"r5"}]}"#,
        )
        .unwrap();
        let reply = s.create(Parameters(dims)).unwrap();
        assert_eq!(reply.matches(",d").count() + 1, 7, "{reply}");
        let doc = s.document.read();
        let lengths: Vec<f64> = doc
            .home()
            .dimensions
            .iter()
            .map(|d| (d.length() * 10.0).round() / 10.0)
            .collect();
        assert_eq!(&lengths[..2], &[415.0, 385.0]);
        assert!(lengths.contains(&285.0));
        drop(doc);
        let baked = s
            .annotations(Parameters(AnnotationParams {
                anchor: None,
                bake: Some(true),
                ..AnnotationParams::default()
            }))
            .unwrap();
        assert!(
            baked.starts_with("ok rev=") && baked.contains("ids=d"),
            "{baked}"
        );
    }
}
