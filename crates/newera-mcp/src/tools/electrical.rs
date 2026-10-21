//! The electrical and telecom project: circuits, the load schedule, and
//! what NBR 5410 asks of each room.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::{applied, core, invalid, write_action};
use crate::compact;
use newera_core::electrical;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct ElectricalParams {
    /// `assign` puts `ids` on `circuit` (empty takes them off), `va` sets
    /// their power; `voltage` is the supply, 127 or 220 V.
    #[schemars(extend("enum" = ["assign", "voltage", "cable", "route", "wifi"]))]
    action: Option<String>,
    /// Point ids for `assign`.
    #[serde(default)]
    ids: Vec<String>,
    /// Circuit name, e.g. `C3`.
    circuit: Option<String>,
    /// For `assign`: the whole division at once, `{"C1": [ids], "C2": [ids]}`,
    /// in one undoable step.
    circuits: Option<std::collections::BTreeMap<String, Vec<String>>>,
    /// Power per point, VA, instead of the norm's default.
    va: Option<f64>,
    /// For `assign` on automation points: standby consumption, W.
    standby_w: Option<f64>,
    /// For `assign` on a dimmer: the most lighting it takes, W.
    max_w: Option<f64>,
    /// For `assign` on a distribution panel: the DIN modules it holds.
    modules: Option<u32>,
    /// For `voltage`: the presumed short-circuit current at the delivery
    /// point, kA, as the utility informs it.
    short_ka: Option<f64>,
    /// For `voltage`: the earthing scheme, `TN-S`, `TN-C-S` or `TT`.
    earthing: Option<String>,
    /// Supply voltage, V; with `assign`, the points' own, 127 or 220.
    volts: Option<f64>,
    /// Findings looked at (the accept tool, for clients).
    #[serde(default)]
    #[schemars(skip)]
    accept: Vec<Vec<String>>,
    /// Drop orphaned acceptances (the accept tool, for clients).
    #[serde(default)]
    #[schemars(skip)]
    prune: bool,
    /// For `cable` and `route`: what the run carries.
    #[schemars(extend("enum" = ["power", "data", "tv"]))]
    kind: Option<String>,
    /// For `cable`: the run's points `[[x,y], …]`, cm.
    #[serde(default)]
    pts: Vec<newera_core::Point2>,
    /// For `route`: where the run passes, `ceiling` (slab, dropping in the
    /// walls), `floor` or `wall`. Omitted: the cheapest that can be built.
    via: Option<String>,
    /// For `route` of data: `cat5e`, `cat6` (default) or `cat6a`.
    cat: Option<String>,
    /// For `route`: the panel it starts from; default the nearest of its kind.
    from: Option<String>,
    /// For `wifi`: the access points' standard, `wifi5`, `wifi6`, `wifi6e` or
    /// `wifi7`, written on `ids`.
    standard: Option<String>,
    /// For `wifi`: the access points' wired uplink, Gbps (1, 2.5, 5, 10).
    uplink: Option<f64>,
    /// For `wifi`: the bands they radiate when fewer than their generation's,
    /// e.g. `2.4,5` for a dual-band Wi-Fi 7.
    bands: Option<String>,
    /// For `wifi`: whether `ids` are fed by their data cable (`PoE`).
    poe: Option<bool>,
    /// For `wifi`: the band the suggestion aims at, `2.4`, `5` (default) or `6`.
    band: Option<String>,
}

/// What the electrical tool reads.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ElectricalReadParams {
    /// `check` (default): points by kind and what the norm finds;
    /// `circuits`: the load schedule and the panel; `wifi`: coverage and a
    /// suggestion.
    #[schemars(extend("enum" = ["check", "circuits", "wifi"]))]
    action: Option<String>,
    /// For `wifi`: the band the suggestion aims at, `2.4`, `5` (default) or `6`.
    band: Option<String>,
    /// Storey to review, an id like `lv3` (default: the one shown).
    level: Option<String>,
}

#[tool_router(router = electrical_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        name = "electrical",
        description = "Electrical and telecom review, NBR 5410 and NBR 16264: points are the catalog's electrical pieces (outlets, switches, lighting points, panels, RJ45, TV, Wi-Fi, automation) plus every fixture that lights. action check (default) answers {points:{kind:count}, findings:[[sev, place, msg, src, key, accepted?]], pending, orphaned, sources}: outlets and lighting per room, circuits, DR, the panel, voltage drop, access points' cables. circuits answers the load schedule, rows [name, kinds, points, VA, V, A, wire mm², breaker A, DR], with main_breaker {a, phases, load_a_per_phase} and panel {devices, modules, dps, earthing, icn_ka, selective}. wifi {band?} answers access_points, coverage per room and band [[room, band, median dBm, worst dBm, share at -67 dBm or better, grade]] and suggested ceiling points. What each check demands: `rules` topic=electrical. Changes are `edit_electrical`; `accept` marks a finding right as drawn."
    )]
    pub(crate) fn read_electrical(
        &self,
        Parameters(p): Parameters<ElectricalReadParams>,
    ) -> Result<String, ErrorData> {
        let action = match p.action.as_deref().unwrap_or("check") {
            read @ ("check" | "circuits" | "wifi") => read,
            other => {
                return Err(invalid(format!(
                    "unknown action `{other}`: check, circuits or wifi (to change the project, use edit_electrical)"
                )));
            }
        };
        let params = ElectricalParams {
            action: Some(action.to_owned()),
            band: p.band,
            ..ElectricalParams::default()
        };
        match self.on_storey(p.level.as_deref())? {
            Some(scratch) => scratch.electrical(Parameters(params)),
            None => self.electrical(Parameters(params)),
        }
    }
    #[tool(
        description = "Change the electrical and telecom project (`electrical` reads it); each action is one undoable step. assign {ids, circuit, va?}, or circuits {\"C1\": [ids], …} for the whole division, writes circuits, power and volts (127|220; a 220 V outlet makes its circuit 220 V) on points; on automation points it takes standby_w, on a dimmer max_w, on a panel modules. voltage {volts?, short_ka?, earthing?}. cable {kind, pts} draws a run by hand (with ids or circuit instead of pts it is a route). route {kind, ids? | circuit?, via?: ceiling|floor|wall|tape, cat?, from?} lays the run as it is built, from the nearest panel along and inside the walls, replacing the earlier run of the circuit, and answers {via, suggested, length_m, by_premise_m, bends, materials: [[item, qty, unit]]}; without via it takes the cheapest premise that can be built. via=tape is surface flat wiring, power only (`rules` topic=electrical). wifi {ids, standard?, poe?, uplink?, bands?} writes those on access points."
    )]
    pub(crate) fn edit_electrical(
        &self,
        Parameters(p): Parameters<ElectricalParams>,
    ) -> Result<String, ErrorData> {
        write_action(
            p.action.as_deref(),
            &["assign", "voltage", "cable", "route", "wifi"],
            "electrical",
        )?;
        self.electrical(Parameters(p))
    }
    /// The electrical project: every read and change, as one call.
    pub(crate) fn electrical(
        &self,
        Parameters(p): Parameters<ElectricalParams>,
    ) -> Result<String, ErrorData> {
        if !p.accept.is_empty() || p.prune {
            let mut doc = self.document.write();
            let mut accepted = doc.home().accepted.clone();
            if p.prune {
                for (key, _) in electrical::orphaned(doc.home()) {
                    accepted.remove(&key);
                }
            }
            for pair in &p.accept {
                let key = pair
                    .first()
                    .map(|k| k.trim().to_owned())
                    .unwrap_or_default();
                if !key.starts_with("elec:") {
                    return Err(invalid(format!(
                        "accept: electrical keys start with elec: (not {key})"
                    )));
                }
                match pair.get(1).map(|w| w.trim()) {
                    None | Some("") => accepted.remove(&key),
                    Some(why) => accepted.insert(key, why.to_owned()),
                };
            }
            if accepted != doc.home().accepted {
                doc.execute(newera_core::Command::SetAccepted { accepted })
                    .map_err(core)?;
            }
        }
        // A cable given by its points, not its path, is laid out like a route.
        let action = match p.action.as_deref().unwrap_or("check") {
            "cable" if p.pts.is_empty() && (!p.ids.is_empty() || p.circuit.is_some()) => "route",
            other => other,
        };
        match action {
            "check" => {
                let doc = self.document.read();
                let home = doc.home();
                let mut kinds: std::collections::BTreeMap<&str, usize> =
                    std::collections::BTreeMap::new();
                for point in electrical::points(home) {
                    *kinds.entry(point.kind.name()).or_default() += 1;
                }
                let findings = electrical::check(home);
                let mut codes: Vec<&str> = findings.iter().map(|f| f.source).collect();
                codes.sort_unstable();
                codes.dedup();
                let rows: Vec<serde_json::Value> = findings
                    .iter()
                    .map(|f| {
                        let mut row =
                            serde_json::json!([f.severity, f.place, f.message, f.source, f.key]);
                        if let (Some(why), Some(cells)) = (&f.accepted, row.as_array_mut()) {
                            cells.push(serde_json::json!(why));
                        }
                        row
                    })
                    .collect();
                let pending = findings.iter().filter(|f| f.accepted.is_none()).count();
                let live: Vec<String> = findings.iter().map(|f| f.key.clone()).collect();
                let orphaned = super::orphan_rows(electrical::orphaned(home), &live);
                let cables: serde_json::Map<String, serde_json::Value> =
                    electrical::cable_lengths(home)
                        .into_iter()
                        .map(|(cable, m)| (cable.key().to_owned(), serde_json::json!(m)))
                        .collect();
                Ok(serde_json::json!({
                    "points": kinds,
                    "cables_m": cables,
                    "findings": rows,
                    "pending": pending,
                    "orphaned": orphaned,
                    "sources": super::sources(&codes, &home.compass.place()),
                })
                .to_string())
            }
            "circuits" => {
                let doc = self.document.read();
                let rows: Vec<serde_json::Value> = electrical::circuits(doc.home())
                    .iter()
                    .map(|c| {
                        serde_json::json!([
                            c.name,
                            c.kinds.iter().map(|k| k.name()).collect::<Vec<_>>(),
                            c.points.len(),
                            c.va,
                            c.volts,
                            c.amps,
                            c.wire_mm2,
                            c.breaker_a,
                            c.rcd,
                        ])
                    })
                    .collect();
                let total: f64 = electrical::circuits(doc.home()).iter().map(|c| c.va).sum();
                let standby: f64 = electrical::points(doc.home())
                    .iter()
                    .filter(|pt| pt.kind == electrical::PointKind::Automation)
                    .map(|pt| pt.va)
                    .sum();
                // `+ 0.0` turns the empty sum (-0.0) into 0.0.
                let mut reply = serde_json::json!({"circuits": rows, "total_va": total + 0.0});
                if standby > 0.0 {
                    reply["standby_w"] = compact::num(standby);
                }
                if let Some(panel) = electrical::panel(doc.home()) {
                    reply["panel"] = serde_json::json!({
                        "devices": panel.devices.iter().map(|d| serde_json::json!([d.0, d.1, d.2])).collect::<Vec<_>>(),
                        "modules": {"used": panel.used, "capacity": panel.capacity, "capacity_written": panel.capacity_written, "spare": panel.spare},
                        "dps": panel.dps,
                        "earthing": panel.earthing,
                        "icn_ka": panel.icn_ka,
                        "selective": panel.main_a >= panel.largest_partial_a * 2,
                    });
                }
                if let Some(supply) = electrical::main_breaker(doc.home()) {
                    reply["main_breaker"] = serde_json::json!({
                        "a": supply.breaker_a,
                        "phases": supply.phases,
                        "load_a_per_phase": supply.amps_per_phase,
                    });
                }
                Ok(reply.to_string())
            }
            "assign" => {
                // One circuit and its ids, or the whole division as a map.
                let mut plan: Vec<(Option<String>, Vec<String>)> = Vec::new();
                if let Some(map) = &p.circuits {
                    plan.extend(
                        map.iter()
                            .map(|(c, ids)| (Some(c.trim().to_owned()), ids.clone())),
                    );
                }
                if !p.ids.is_empty() {
                    plan.push((
                        p.circuit.as_deref().map(|c| c.trim().to_owned()),
                        p.ids.clone(),
                    ));
                }
                if plan.is_empty() {
                    return Err(invalid(
                        "assign needs ids with circuit (empty to clear) or va, or circuits {C1: [ids]}",
                    ));
                }
                if p.circuits.is_none()
                    && p.circuit.is_none()
                    && p.va.is_none()
                    && p.volts.is_none()
                    && p.standby_w.is_none()
                    && p.max_w.is_none()
                    && p.modules.is_none()
                {
                    return Err(invalid(
                        "assign needs circuit (empty to clear), va or volts",
                    ));
                }
                if let Some(volts) = p.volts
                    && (volts - 127.0).abs() > 0.5
                    && (volts - 220.0).abs() > 0.5
                {
                    return Err(invalid(format!(
                        "volts on a point: 127 or 220 (not {volts})"
                    )));
                }
                let mut doc = self.document.write();
                let before = doc.home().clone();
                let mut pieces: std::collections::BTreeMap<
                    newera_core::FurnitureId,
                    newera_core::Furniture,
                > = std::collections::BTreeMap::new();
                for (circuit, ids) in plan {
                    for raw in ids {
                        let id: newera_core::FurnitureId =
                            raw.parse().map_err(|e| invalid(format!("{raw}: {e}")))?;
                        let piece = match pieces.remove(&id) {
                            Some(piece) => piece,
                            None => doc.home().piece(id).cloned().ok_or_else(|| {
                                invalid(format!("{raw} not found (a point of the plan)"))
                            })?,
                        };
                        let mut piece = piece;
                        match circuit.as_deref() {
                            Some("") => {
                                piece.properties.remove(electrical::CIRCUIT_KEY);
                            }
                            Some(name) => {
                                piece
                                    .properties
                                    .insert(electrical::CIRCUIT_KEY.into(), name.to_owned());
                            }
                            None => {}
                        }
                        if let Some(va) = p.va {
                            piece
                                .properties
                                .insert(electrical::VA_KEY.into(), va.to_string());
                        }
                        if let Some(w) = p.standby_w {
                            piece
                                .properties
                                .insert(electrical::STANDBY_KEY.into(), w.to_string());
                        }
                        if let Some(m) = p.modules {
                            piece
                                .properties
                                .insert(electrical::MODULES_KEY.into(), m.to_string());
                        }
                        if let Some(w) = p.max_w {
                            piece
                                .properties
                                .insert(electrical::MAX_W_KEY.into(), w.to_string());
                        }
                        if let Some(volts) = p.volts {
                            piece
                                .properties
                                .insert(electrical::VOLTS_KEY.into(), volts.to_string());
                        }
                        pieces.insert(id, piece);
                    }
                }
                let commands = pieces
                    .into_values()
                    .map(newera_core::Command::update)
                    .collect();
                doc.execute(newera_core::Command::Batch { commands })
                    .map_err(core)?;
                Ok(applied(&doc, &before))
            }
            "route" => {
                let cable = p
                    .kind
                    .as_deref()
                    .and_then(electrical::Cable::parse)
                    .ok_or_else(|| invalid("route kind: power, data or tv"))?;
                let category = match p.cat.as_deref() {
                    Some(raw) => electrical::Category::parse(raw)
                        .ok_or_else(|| invalid("cat: cat5e, cat6 or cat6a"))?,
                    None => electrical::Category::Cat6,
                };
                let mut doc = self.document.write();
                let before = doc.home().clone();
                let home = doc.home();
                let view = home.level_view(home.current_level());
                let all = electrical::points(home);
                // The points: given, or those of the circuit, or every point of the kind.
                let wanted: Vec<&electrical::Point> = if !p.ids.is_empty() {
                    p.ids
                        .iter()
                        .map(|raw| {
                            all.iter()
                                .find(|pt| pt.id.to_string() == *raw)
                                .ok_or_else(|| {
                                    invalid(format!(
                                        "{raw} is not a point of the electrical project"
                                    ))
                                })
                        })
                        .collect::<Result<_, _>>()?
                } else if let Some(circuit) = &p.circuit {
                    all.iter()
                        .filter(|pt| pt.circuit.as_deref() == Some(circuit.as_str()))
                        .collect()
                } else {
                    all.iter()
                        .filter(|pt| match cable {
                            electrical::Cable::Power => pt.kind.loads(),
                            electrical::Cable::Data => matches!(
                                pt.kind,
                                electrical::PointKind::Network | electrical::PointKind::Wifi
                            ),
                            electrical::Cable::Tv => pt.kind == electrical::PointKind::Tv,
                        })
                        .collect()
                };
                if wanted.is_empty() {
                    return Err(invalid(
                        "route: no points (give ids, a circuit, or place points of the kind)",
                    ));
                }
                let panel_kind = match cable {
                    electrical::Cable::Power => electrical::PointKind::Panel,
                    _ => electrical::PointKind::TelecomPanel,
                };
                let terminal = |id: newera_core::FurnitureId| {
                    view.find_piece(id).map(|f| newera_core::routing::Terminal {
                        id: Some(f.id),
                        at: f.position,
                        z: f.elevation + f.height / 2.0,
                    })
                };
                let source = if let Some(raw) = &p.from {
                    all.iter()
                        .find(|pt| pt.id.to_string() == *raw)
                        .and_then(|pt| terminal(pt.id))
                        .ok_or_else(|| {
                            invalid(format!("from: {raw} is not a panel of this storey"))
                        })?
                } else {
                    let first = terminal(wanted[0].id)
                        .ok_or_else(|| invalid("point not on this storey"))?;
                    all.iter()
                        .filter(|pt| pt.kind == panel_kind)
                        .filter_map(|pt| terminal(pt.id))
                        .min_by(|a, b| a.at.distance(first.at).total_cmp(&b.at.distance(first.at)))
                        .ok_or_else(|| {
                            invalid(match cable {
                                electrical::Cable::Power => {
                                    "route: place a distribution panel first (electrical-panel)"
                                }
                                _ => "route: place a telecom panel first (telecom-panel)",
                            })
                        })?
                };
                let points: Vec<newera_core::routing::Terminal> =
                    wanted.iter().filter_map(|pt| terminal(pt.id)).collect();
                let storey = home
                    .current_level()
                    .and_then(|id| home.level(id))
                    .map_or(280.0, |l| l.height);
                let (best, all_routes) =
                    newera_core::routing::cheapest(&view, source, &points, storey);
                // Adhesive tape runs on the surface of the walls or the ceiling.
                let tape = p.via.as_deref() == Some("tape");
                if tape && cable != electrical::Cable::Power {
                    return Err(invalid(
                        "via tape: the adhesive tape carries power only; network and TV go in cable",
                    ));
                }
                let via = match p.via.as_deref() {
                    Some("tape") => all_routes
                        .iter()
                        .filter(|(v, r)| {
                            *v != newera_core::routing::Via::Floor && r.impossible.is_empty()
                        })
                        .min_by(|a, b| a.1.length().total_cmp(&b.1.length()))
                        .map(|(v, _)| *v)
                        .ok_or_else(|| {
                            invalid("via tape: some point is in no wall and below the ceiling; the tape runs on walls and ceilings only")
                        })?,
                    Some(raw) => newera_core::routing::Via::parse(raw)
                        .ok_or_else(|| invalid("via: ceiling, floor, wall or tape"))?,
                    None => best,
                };
                let route = all_routes
                    .iter()
                    .find(|(v, _)| *v == via)
                    .map(|(_, r)| r.clone())
                    .expect("every premise is laid out");
                if !route.impossible.is_empty() {
                    let ids: Vec<String> = route
                        .impossible
                        .iter()
                        .filter_map(|t| t.id.map(|i| i.to_string()))
                        .collect();
                    return Err(invalid(format!(
                        "route via {}: {} {}",
                        via.key(),
                        ids.join(", "),
                        match via {
                            newera_core::routing::Via::Ceiling =>
                                "stand low in no wall: from the ceiling there is no wall to drop in; use via floor, or move the point to a wall",
                            _ =>
                                "stand in no wall and cannot be run inside one; use via ceiling or floor, or move the point to a wall",
                        }
                    )));
                }
                let kinds: Vec<electrical::PointKind> = wanted.iter().map(|pt| pt.kind).collect();
                let section = match (&p.circuit, cable) {
                    (Some(c), electrical::Cable::Power) => electrical::circuits(home)
                        .into_iter()
                        .find(|x| x.name == *c)
                        .map_or(2.5, |x| x.wire_mm2),
                    _ => {
                        if kinds.iter().all(|k| *k == electrical::PointKind::Lighting) {
                            1.5
                        } else {
                            2.5
                        }
                    }
                };
                let mut bill = electrical::materials(&route, cable, section, category, &kinds);
                let mut tape_reply = serde_json::Value::Null;
                if tape {
                    use newera_core::tape;
                    let wet: Vec<String> = wanted
                        .iter()
                        .filter(|pt| electrical::in_wet_room(home, pt))
                        .map(|pt| pt.id.to_string())
                        .collect();
                    if !wet.is_empty() {
                        return Err(invalid(format!(
                            "via tape: {} in a bathroom or outdoors, where the maker gives the tape no rating; run it in wire and conduit",
                            wet.join(", ")
                        )));
                    }
                    let volts = electrical::circuits(home)
                        .iter()
                        .filter(|c| wanted.iter().any(|pt| c.points.contains(&pt.id)))
                        .map(|c| c.volts)
                        .fold(0.0, f64::max);
                    let volts = if volts > 0.0 { volts } else { 127.0 };
                    let amps = wanted.iter().map(|pt| pt.va).sum::<f64>() / volts;
                    let needs_earth = kinds.iter().any(|k| {
                        matches!(
                            k,
                            electrical::PointKind::Outlet | electrical::PointKind::Dedicated
                        )
                    });
                    let model = tape::model_for(amps, needs_earth).map_err(invalid)?;
                    // A piece per link, with its share of the drops and 20 cm at each end.
                    let links = route.paths.len().max(1);
                    #[allow(clippy::cast_precision_loss)]
                    let drop_share = route.vertical / links as f64;
                    let pieces: Vec<f64> = route
                        .paths
                        .iter()
                        .map(|path| {
                            path.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>()
                                + drop_share
                                + 2.0 * tape::PER_CONNECTION_CM
                        })
                        .collect();
                    let (buy, splices) = tape::purchase(model, &pieces);
                    let total: f64 = buy
                        .iter()
                        .map(|b| f64::from(b.quantity) * b.unit_price)
                        .sum();
                    bill = buy
                        .iter()
                        .map(|b| newera_core::electrical::Material {
                            item: format!(
                                "{} (Leroy {}, R$ {})",
                                b.item,
                                b.code,
                                format!("{:.2}", b.unit_price).replace('.', ",")
                            ),
                            quantity: f64::from(b.quantity),
                            unit: "un",
                        })
                        .collect();
                    tape_reply = serde_json::json!({
                        "model": model.model,
                        "tracks": model.tracks,
                        "rated_a": model.amps,
                        "load_a": compact::num(amps),
                        "earthed": model.earthed(),
                        "pieces_m": pieces.iter().map(|cm| compact::num(cm / 100.0)).collect::<Vec<_>>(),
                        "splices": splices,
                        "total_brl": compact::num(total),
                        "notes": [
                            format!("Disjuntor do circuito de no máximo {} A, a corrente da fita.", model.amps),
                            "Fita sempre coberta: malha de fibra de vidro, massa acrílica numa faixa de uns 50 cm, depois pintura; nunca aparente.",
                            "A fita não é um condutor previsto na NBR 5410 e não tem certificação Inmetro: combine com o responsável técnico antes de usar.",
                            "Uma pista danificada é a fita inteira trocada e o acabamento refeito.",
                        ],
                    });
                }
                // Replaces the run drawn before for the same thing.
                // Named by what it serves, so routing the same points again
                // replaces it and routing others adds a run beside it.
                let run = format!(
                    "{}:{}",
                    cable.key(),
                    p.circuit.clone().unwrap_or_else(|| {
                        let mut ids: Vec<String> =
                            wanted.iter().map(|pt| pt.id.to_string()).collect();
                        ids.sort();
                        if p.ids.is_empty() {
                            "all".into()
                        } else {
                            ids.join("+")
                        }
                    })
                );
                let mut ends: Vec<newera_core::Point2> = points.iter().map(|t| t.at).collect();
                ends.push(source.at);
                let replaced = electrical::drawn_runs(home, cable, &ends);
                let mut commands: Vec<newera_core::Command> = home
                    .polylines
                    .iter()
                    .filter(|l| {
                        l.properties.get(electrical::RUN_KEY) == Some(&run)
                            || replaced.contains(&l.id)
                    })
                    .map(|l| newera_core::Command::remove(newera_core::ElementId::Polyline(l.id)))
                    .collect();
                let (dash, color) = cable.style();
                let paths = route.paths.clone();
                for path in paths {
                    let mut line = newera_core::Polyline::new(doc.new_polyline_id(), path);
                    line.dash = dash;
                    line.color = color;
                    line.thickness = 1.5;
                    line.discipline = Some(newera_core::Discipline::Electrical);
                    line.properties
                        .insert(electrical::CABLE_KEY.into(), cable.key().into());
                    line.properties
                        .insert(electrical::RUN_KEY.into(), run.clone());
                    line.properties.insert(
                        electrical::ENDS_KEY.into(),
                        wanted
                            .iter()
                            .map(|pt| pt.id.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    if cable == electrical::Cable::Data {
                        line.properties
                            .insert(electrical::CATEGORY_KEY.into(), category.key().into());
                    }
                    line.properties
                        .insert(electrical::RUN_CM_KEY.into(), route.length().to_string());
                    let far = route.reach.iter().copied().fold(0.0, f64::max);
                    line.properties
                        .insert(electrical::RUN_FAR_KEY.into(), far.to_string());
                    line.properties.insert("elec:via".into(), via.key().into());
                    commands.push(newera_core::Command::insert(line));
                }
                doc.execute(newera_core::Command::Batch { commands })
                    .map_err(core)?;
                let alternatives: serde_json::Map<String, serde_json::Value> = all_routes
                    .iter()
                    .map(|(v, r)| {
                        (
                            v.key().to_owned(),
                            if r.impossible.is_empty() {
                                compact::num(r.length() / 100.0)
                            } else {
                                serde_json::json!("impossível")
                            },
                        )
                    })
                    .collect();
                let reply = serde_json::json!({
                    "run": run,
                    "replaced_drawn": replaced.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    "via": via.key(),
                    "suggested": best.key(),
                    "length_m": {
                        "horizontal": compact::num(route.horizontal / 100.0),
                        "vertical": compact::num(route.vertical / 100.0),
                        "total": compact::num(route.length() / 100.0),
                    },
                    "by_premise_m": alternatives,
                    "bends": route.bends,
                    "materials": bill.iter().map(|m| serde_json::json!([m.item, m.quantity, m.unit])).collect::<Vec<_>>(),
                });
                let mut reply = reply;
                if tape {
                    reply["via"] = serde_json::json!("tape");
                    reply["tape"] = tape_reply;
                }
                // A network link past 90 m (the permanent link, NBR 16264 /
                // TIA-568), counting the slack left at the rack and the outlet
                // does not carry its category: say which points.
                if cable == electrical::Cable::Data {
                    let far: Vec<String> = route
                        .terminals
                        .iter()
                        .zip(&route.reach)
                        .filter(|(_, cm)| **cm + 330.0 > 9000.0)
                        .filter_map(|(t, _)| t.id.map(|i| i.to_string()))
                        .collect();
                    if !far.is_empty() {
                        reply["warnings"] = serde_json::json!([format!(
                            "{} passam de 90 m do rack: o enlace não garante a categoria; aproxime o rack ou use um switch no caminho",
                            far.join(", ")
                        )]);
                    }
                }
                reply["rev"] = serde_json::json!(doc.revision());
                let _ = before;
                Ok(reply.to_string())
            }
            "wifi" => {
                use newera_core::wifi;
                let standard = match p.standard.as_deref() {
                    Some(raw) => Some(
                        wifi::Standard::parse(raw)
                            .ok_or_else(|| invalid("standard: wifi5, wifi6, wifi6e or wifi7"))?,
                    ),
                    None => None,
                };
                let band = match p.band.as_deref() {
                    Some(raw) => {
                        wifi::Band::parse(raw).ok_or_else(|| invalid("band: 2.4, 5 or 6"))?
                    }
                    None => wifi::Band::G5,
                };
                let mut doc = self.document.write();
                if !p.ids.is_empty()
                    && (standard.is_some()
                        || p.poe.is_some()
                        || p.uplink.is_some()
                        || p.bands.is_some())
                {
                    let mut commands = Vec::new();
                    for raw in &p.ids {
                        let piece = doc
                            .home()
                            .furniture
                            .iter()
                            .flat_map(newera_core::Furniture::flatten)
                            .find(|f| f.id.to_string() == *raw && f.catalog == "wifi-point")
                            .cloned()
                            .ok_or_else(|| invalid(format!("{raw} is not a wifi-point")))?;
                        let mut piece = piece;
                        if let Some(s) = standard {
                            piece
                                .properties
                                .insert(wifi::STANDARD_KEY.into(), s.key().into());
                        }
                        if let Some(bands) = &p.bands {
                            piece
                                .properties
                                .insert(wifi::BANDS_KEY.into(), bands.replace(' ', ""));
                        }
                        if let Some(gbps) = p.uplink {
                            piece
                                .properties
                                .insert(wifi::UPLINK_KEY.into(), gbps.to_string());
                        }
                        if let Some(poe) = p.poe {
                            piece
                                .properties
                                .insert(wifi::POE_KEY.into(), poe.to_string());
                        }
                        commands.push(newera_core::Command::update(piece));
                    }
                    doc.execute(newera_core::Command::Batch { commands })
                        .map_err(core)?;
                }
                let home = doc.home();
                let aps = wifi::access_points(home);
                let storey = home
                    .current_level()
                    .and_then(|id| home.level(id))
                    .map_or(280.0, |l| l.height);
                let aimed = standard
                    .or_else(|| aps.first().map(|a| a.standard))
                    .unwrap_or(wifi::Standard::Wifi6);
                if !aimed.bands().contains(&band) {
                    return Err(invalid(format!(
                        "{} has no {} GHz band",
                        aimed.name(),
                        band.key()
                    )));
                }
                // Access points on other storeys reach this one through the slab.
                let near = wifi::access_points_near(home);
                let rows: Vec<serde_json::Value> = wifi::coverage(home, &near)
                    .iter()
                    .map(|r| {
                        serde_json::json!([
                            format!("{} {}", r.name, r.room),
                            r.band.key(),
                            r.median,
                            r.worst,
                            r.good,
                            wifi::grade(r.worst)
                        ])
                    })
                    .collect();
                let (points, short) = wifi::suggest(home, aimed, band, storey);
                Ok(serde_json::json!({
                    "access_points": aps.iter().map(|a| serde_json::json!([
                        a.id.map(|i| i.to_string()),
                        a.standard.key(),
                        wifi::Band::ALL.iter().filter(|b| a.radiates(**b)).map(|b| b.key()).collect::<Vec<_>>(),
                        format!("{} GbE", a.uplink_gbps),
                    ])).collect::<Vec<_>>(),
                    "coverage": rows,
                    "suggested": {
                        "standard": aimed.key(),
                        "band": band.key(),
                        "points": points.iter().map(|(a, room)| serde_json::json!([compact::num(a.at.x), compact::num(a.at.y), compact::num(a.z), room])).collect::<Vec<_>>(),
                        "short": short,
                    },
                    "rev": doc.revision(),
                })
                .to_string())
            }
            "cable" => {
                let cable = p
                    .kind
                    .as_deref()
                    .and_then(electrical::Cable::parse)
                    .ok_or_else(|| invalid("cable kind: power, data or tv"))?;
                if p.pts.len() < 2 {
                    return Err(invalid("cable pts: at least two points"));
                }
                let mut doc = self.document.write();
                let before = doc.home().clone();
                let mut line = newera_core::Polyline::new(doc.new_polyline_id(), p.pts.clone());
                let (dash, color) = cable.style();
                line.dash = dash;
                line.color = color;
                line.thickness = 1.5;
                line.discipline = Some(newera_core::Discipline::Electrical);
                line.properties
                    .insert(electrical::CABLE_KEY.into(), cable.key().into());
                doc.execute(newera_core::Command::insert(line))
                    .map_err(core)?;
                Ok(applied(&doc, &before))
            }
            "voltage" => {
                if p.volts.is_none() && p.short_ka.is_none() && p.earthing.is_none() {
                    return Err(invalid(
                        "voltage needs volts (127 or 220), short_ka or earthing",
                    ));
                }
                let mut doc = self.document.write();
                let before = doc.home().clone();
                let mut properties = doc.home().properties.clone();
                if let Some(volts) = p.volts {
                    if !(100.0..=400.0).contains(&volts) {
                        return Err(invalid("volts: a supply voltage, 127 or 220"));
                    }
                    properties.insert(electrical::VOLTAGE_KEY.into(), volts.to_string());
                }
                if let Some(ka) = p.short_ka {
                    if !(1.0..=50.0).contains(&ka) {
                        return Err(invalid(
                            "short_ka: the presumed short-circuit current, kA (e.g. 4.5, 6, 10)",
                        ));
                    }
                    properties.insert(electrical::SHORT_KA_KEY.into(), ka.to_string());
                }
                if let Some(scheme) = &p.earthing {
                    let scheme = scheme.trim().to_uppercase();
                    if !["TN-S", "TN-C-S", "TT"].contains(&scheme.as_str()) {
                        return Err(invalid("earthing: TN-S, TN-C-S or TT"));
                    }
                    properties.insert(electrical::EARTHING_KEY.into(), scheme);
                }
                doc.execute(newera_core::Command::SetProperties { properties })
                    .map_err(core)?;
                Ok(applied(&doc, &before))
            }
            other => Err(invalid(format!(
                "action: check, circuits, assign, voltage, cable, route or wifi (not {other})"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::server;

    #[test]
    fn a_small_flat_is_wired_checked_and_scheduled() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true},{"pts":[[300,0],[300,400]]}],
                    "rooms":[{"name":"Cozinha","at":[150,200]},{"name":"Quarto","at":[450,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.disciplines(Parameters(
            serde_json::from_str(r#"{"action":"select","d":"electrical"}"#).unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"electrical-panel","at":[20,20]},
                                 {"cat":"light-ceiling","at":[150,200]},{"cat":"light-ceiling","at":[450,200]},
                                 {"cat":"outlet-mid","at":[100,10]},{"cat":"outlet-mid","at":[200,10]},
                                 {"cat":"outlet-low","at":[310,100]},{"cat":"network-outlet","at":[590,200]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let ids: Vec<String> = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let electrical = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.electrical(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap()
        };
        let check = electrical("{}");
        assert_eq!(check["points"]["Rede"], 1, "{check}");
        let text = check["findings"].to_string();
        assert!(text.contains("pontos sem circuito"), "{check}");
        assert!(text.contains("quadro de telecom"), "{check}");
        assert!(check["sources"]["nbr5410"].is_array(), "{check}");

        let assign = |ids: &[&String], circuit: &str| {
            s.electrical(Parameters(
                serde_json::from_str(&format!(
                    r#"{{"action":"assign","ids":{},"circuit":"{circuit}"}}"#,
                    serde_json::json!(ids)
                ))
                .unwrap(),
            ))
            .unwrap()
        };
        assign(&[&ids[1], &ids[2]], "C1");
        assign(&[&ids[3], &ids[4]], "C2");
        assign(&[&ids[5]], "C3");
        let schedule = electrical(r#"{"action":"circuits"}"#);
        let c2 = schedule["circuits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c[0] == "C2")
            .unwrap()
            .clone();
        assert_eq!(c2[3], 1200.0, "{schedule}");
        assert_eq!(c2[6], 2.5, "{schedule}");
        assert_eq!(c2[8], true, "kitchen outlets need a DR: {schedule}");
        assert!(
            !electrical("{}")["findings"]
                .to_string()
                .contains("pontos sem circuito")
        );

        // The whole division in one call, one undo step.
        s.electrical(Parameters(
            serde_json::from_str(&format!(
                r#"{{"action":"assign","circuits":{{"L1":["{}","{}"],"T1":["{}","{}"],"T2":["{}"]}}}}"#,
                ids[1], ids[2], ids[3], ids[4], ids[5]
            ))
            .unwrap(),
        ))
        .unwrap();
        let names: Vec<String> = electrical(r#"{"action":"circuits"}"#)["circuits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c[0].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["L1", "T1", "T2"]);
        s.document.write().undo().unwrap();
        assert_eq!(
            electrical(r#"{"action":"circuits"}"#)["circuits"]
                .as_array()
                .unwrap()
                .len(),
            3,
            "one undo takes the whole division back to C1–C3"
        );

        // A network cable from the point to where the rack will be.
        s.electrical(Parameters(
            serde_json::from_str(
                r#"{"action":"cable","kind":"data","pts":[[590,200],[590,20],[20,20]]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let check = electrical("{}");
        assert!(check["cables_m"]["data"].as_f64().unwrap() > 7.5, "{check}");
        let line = s.document.read().home().polylines.last().cloned().unwrap();
        assert_eq!(line.discipline, Some(newera_core::Discipline::Electrical));
        assert_eq!(line.dash, newera_core::DashStyle::Dash);
        s.document.write().undo().unwrap();

        // Undo takes the circuit off in one step.
        s.document.write().undo().unwrap();
        assert_eq!(
            electrical(r#"{"action":"circuits"}"#)["circuits"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        // A finding looked at and accepted stays, with its reason.
        let check = electrical("{}");
        let first = check["findings"][0].clone();
        let key = first[4].as_str().unwrap().to_owned();
        assert!(key.starts_with("elec:"), "{check}");
        let before = check["pending"].as_u64().unwrap();
        let accepted = electrical(&format!(
            r#"{{"accept":[["{key}","rede sai pelo Wi-Fi da sala"]]}}"#
        ));
        let row = accepted["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r[4] == key.as_str())
            .unwrap_or_else(|| panic!("{accepted}"))
            .clone();
        assert_eq!(row[5], "rede sai pelo Wi-Fi da sala", "{accepted}");
        assert_eq!(
            accepted["pending"].as_u64().unwrap(),
            before - 1,
            "{accepted}"
        );
        let wrong = s
            .electrical(Parameters(
                serde_json::from_str(r#"{"accept":[["overlap:f1+f2","x"]]}"#).unwrap(),
            ))
            .unwrap_err();
        assert!(wrong.message.contains("elec:"), "{wrong:?}");
    }

    #[test]
    fn a_run_is_routed_inside_walls_and_slab_with_its_materials() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true}],
                    "rooms":[{"name":"Sala","at":[300,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.disciplines(Parameters(
            serde_json::from_str(r#"{"action":"select","d":"electrical"}"#).unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"electrical-panel","at":[10,200]},{"cat":"telecom-panel","at":[10,300]},
                                 {"cat":"outlet-low","at":[300,10]},{"cat":"outlet-low","at":[590,200]},
                                 {"cat":"light-ceiling","at":[300,200]},
                                 {"cat":"network-outlet","at":[590,300]},{"cat":"network-outlet","at":[300,390]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let ids: Vec<String> = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let electrical = |json: &str| {
            s.electrical(Parameters(serde_json::from_str(json).unwrap()))
                .map(|r| serde_json::from_str(&r).unwrap_or(serde_json::Value::String(r)))
        };

        // Outlets in the walls: every premise can be built, the cheapest is chosen.
        let outlets = electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}","{}"]}}"#,
            ids[2], ids[3]
        ))
        .unwrap();
        let by = &outlets["by_premise_m"];
        let chosen = outlets["length_m"]["total"].as_f64().unwrap();
        for via in ["ceiling", "floor", "wall"] {
            assert!(by[via].as_f64().unwrap() + 1e-9 >= chosen, "{outlets}");
        }
        assert_eq!(outlets["via"], outlets["suggested"]);
        let bill = outlets["materials"].to_string();
        assert!(
            bill.contains("Eletroduto") && bill.contains("(terra)"),
            "{outlets}"
        );
        let drawn: Vec<_> = s
            .document
            .read()
            .home()
            .polylines
            .iter()
            .filter(|l| l.properties.contains_key(electrical::RUN_KEY))
            .cloned()
            .collect();
        assert_ne!(drawn, [] as [newera_core::Polyline; 0]);
        assert!(
            drawn
                .iter()
                .all(|l| l.discipline == Some(newera_core::Discipline::Electrical))
        );

        // Routing the same run again replaces it, and check measures the route.
        electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}","{}"],"via":"wall"}}"#,
            ids[2], ids[3]
        ))
        .unwrap();
        let runs = s
            .document
            .read()
            .home()
            .polylines
            .iter()
            .filter(|l| l.properties.contains_key(electrical::RUN_KEY))
            .count();
        assert!(runs <= drawn.len().max(3), "the earlier run was replaced");
        let check = electrical("{}").unwrap();
        assert!(
            check["cables_m"]["power"].as_f64().unwrap() > 0.0,
            "{check}"
        );

        // A ceiling light is in no wall: a wall run to it is refused, naming it.
        let refused = electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}"],"via":"wall"}}"#,
            ids[4]
        ))
        .unwrap_err();
        assert!(refused.message.contains(&ids[4]), "{refused:?}");
        let light = electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}"]}}"#,
            ids[4]
        ))
        .unwrap();
        assert_ne!(light["suggested"], "wall", "{light}");

        // cable with ids and no path is the same route.
        let by_ids = electrical(&format!(
            r#"{{"action":"cable","kind":"data","ids":["{}"]}}"#,
            ids[5]
        ))
        .unwrap();
        assert!(by_ids["materials"].to_string().contains("RJ45"), "{by_ids}");

        // A network cable drawn by hand to the outlet goes when data is routed.
        let (rack, outlet) = {
            let doc = s.document.read();
            let at = |cat: &str| {
                doc.home()
                    .furniture
                    .iter()
                    .find(|f| f.catalog == cat)
                    .unwrap()
                    .position
            };
            (at("telecom-panel"), at("network-outlet"))
        };
        electrical(&format!(
            r#"{{"action":"cable","kind":"data","pts":[[{},{}],[{},{}]]}}"#,
            rack.x, rack.y, outlet.x, outlet.y
        ))
        .unwrap();
        let hand = s
            .document
            .read()
            .home()
            .polylines
            .last()
            .unwrap()
            .id
            .to_string();
        // Network in star from the telecom panel, Cat 6A as asked.
        let data = electrical(r#"{"action":"route","kind":"data","cat":"cat6a"}"#).unwrap();
        assert!(data["replaced_drawn"].to_string().contains(&hand), "{data}");
        assert!(
            !s.document
                .read()
                .home()
                .polylines
                .iter()
                .any(|l| l.id.to_string() == hand)
        );
        let bill = data["materials"].to_string();
        assert!(bill.contains("Cat 6A") && bill.contains("RJ45"), "{data}");
        assert!(!bill.contains("750 V"), "{data}");

        // 220 V on a point, and the main breaker in the schedule.
        electrical(&format!(
            r#"{{"action":"assign","ids":["{}"],"circuit":"C2","volts":220}}"#,
            ids[3]
        ))
        .unwrap();
        let schedule = electrical(r#"{"action":"circuits"}"#).unwrap();
        assert_eq!(schedule["circuits"][0][4], 220.0, "{schedule}");
        assert!(
            schedule["main_breaker"]["a"].as_u64().unwrap() >= 10,
            "{schedule}"
        );
        assert!(
            electrical(&format!(
                r#"{{"action":"assign","ids":["{}"],"volts":110}}"#,
                ids[3]
            ))
            .is_err()
        );
    }

    #[test]
    fn an_access_point_asks_for_cable_power_and_category_and_its_coverage_is_given() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[800,0],[800,400],[0,400]],"closed":true},{"pts":[[400,0],[400,400]]}],
                    "rooms":[{"name":"Sala","at":[200,200]},{"name":"Quarto","at":[600,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.disciplines(Parameters(
            serde_json::from_str(r#"{"action":"select","d":"electrical"}"#).unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"telecom-panel","at":[10,200]},{"cat":"wifi-point","at":[200,200]},
                                 {"cat":"network-outlet","at":[390,300]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let ids: Vec<String> = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let ap = ids[1].clone();
        let electrical = |json: &str| {
            s.electrical(Parameters(serde_json::from_str(json).unwrap()))
                .map(|r| serde_json::from_str(&r).unwrap_or(serde_json::Value::String(r)))
        };
        let keys = |check: &serde_json::Value| check["findings"].to_string();
        let check = electrical("{}").unwrap();
        assert!(
            keys(&check).contains(&format!("elec:wifi-power:{ap}")),
            "{check}"
        );

        // A data run that reaches the network outlet but not the access point.
        electrical(&format!(
            r#"{{"action":"route","kind":"data","ids":["{}"]}}"#,
            ids[2]
        ))
        .unwrap();
        let check = electrical("{}").unwrap();
        assert!(
            keys(&check).contains(&ap),
            "the access point has no cable: {check}"
        );

        // Wi-Fi 7 on PoE over Cat 6: powered, cabled, but short of 10 GbE.
        electrical(&format!(
            r#"{{"action":"wifi","ids":["{ap}"],"standard":"wifi7","poe":true,"uplink":10}}"#
        ))
        .unwrap();
        electrical(r#"{"action":"route","kind":"data","cat":"cat6"}"#).unwrap();
        let check = electrical("{}").unwrap();
        let text = keys(&check);
        assert!(!text.contains("elec:wifi-power"), "{check}");
        assert!(!text.contains("elec:unreached:data"), "{check}");
        assert!(text.contains(&format!("elec:wifi-uplink:{ap}")), "{check}");
        electrical(r#"{"action":"route","kind":"data","cat":"cat6a"}"#).unwrap();
        assert!(!keys(&electrical("{}").unwrap()).contains("elec:wifi-uplink"));

        // Coverage per room and band, and where to put the points.
        let wifi = electrical(r#"{"action":"wifi","band":"6"}"#).unwrap();
        let rows = wifi["coverage"].as_array().unwrap();
        assert_eq!(rows.len(), 6, "two rooms × three bands: {wifi}");
        let at = |room: &str, band: &str| {
            rows.iter()
                .find(|r| r[0].as_str().unwrap().starts_with(room) && r[1] == band)
                .unwrap()[2]
                .as_f64()
                .unwrap()
        };
        assert!(at("Sala", "5") > at("Quarto", "5"), "{wifi}");
        assert!(
            at("Quarto", "2.4") - at("Quarto", "6") > at("Sala", "2.4") - at("Sala", "6"),
            "the wall costs the high band more: {wifi}"
        );
        assert_eq!(wifi["suggested"]["band"], "6");
        assert!(
            !wifi["suggested"]["points"].as_array().unwrap().is_empty(),
            "{wifi}"
        );
        assert!(
            electrical(r#"{"action":"wifi","standard":"wifi6","band":"6"}"#).is_err(),
            "Wi-Fi 6 has no 6 GHz"
        );
    }

    #[test]
    fn automation_is_placed_from_the_catalog_and_its_standby_reaches_the_schedule() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,400],[0,400]],"closed":true}],"rooms":[{"name":"Hall","at":[200,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"light-ceiling","at":[200,200]},{"cat":"smart-relay","at":[10,100]},{"cat":"presence-sensor","at":[200,200]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let ids: Vec<String> = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let electrical = |json: &str| {
            s.electrical(Parameters(serde_json::from_str(json).unwrap()))
                .map(|r| serde_json::from_str(&r).unwrap_or(serde_json::Value::String(r)))
        };
        electrical(&format!(
            r#"{{"action":"assign","circuits":{{"C1":["{}","{}","{}"]}}}}"#,
            ids[0], ids[1], ids[2]
        ))
        .unwrap();
        electrical(&format!(
            r#"{{"action":"assign","ids":["{}"],"standby_w":1.2}}"#,
            ids[1]
        ))
        .unwrap();
        let schedule = electrical(r#"{"action":"circuits"}"#).unwrap();
        assert!(
            (schedule["standby_w"].as_f64().unwrap() - 2.2).abs() < 1e-9,
            "{schedule}"
        );
        assert_eq!(
            schedule["circuits"][0][1],
            serde_json::json!(["Iluminação"]),
            "{schedule}"
        );
        let check = electrical("{}").unwrap();
        assert_eq!(check["points"]["Automação"], 2, "{check}");
        assert!(
            check["findings"]
                .to_string()
                .contains(&format!("elec:neutral:{}", ids[1])),
            "{check}"
        );
    }

    #[test]
    fn the_panel_is_filled_and_its_protection_given_in_the_schedule() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,400],[0,400]],"closed":true}],"rooms":[{"name":"Cozinha","at":[200,200]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"electrical-panel","at":[10,200]},{"cat":"outlet-mid","at":[100,10]},{"cat":"shower-point","at":[300,10]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let ids: Vec<String> = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let electrical = |json: &str| {
            s.electrical(Parameters(serde_json::from_str(json).unwrap()))
                .map(|r| serde_json::from_str(&r).unwrap_or(serde_json::Value::String(r)))
        };
        electrical(&format!(
            r#"{{"action":"assign","circuits":{{"C1":["{}"],"C2":["{}"]}}}}"#,
            ids[1], ids[2]
        ))
        .unwrap();
        let schedule = electrical(r#"{"action":"circuits"}"#).unwrap();
        let panel = &schedule["panel"];
        assert_eq!(panel["modules"]["spare"], 2, "{schedule}");
        assert!(
            panel["devices"].to_string().contains("bipolar (220 V"),
            "the shower is between phases: {schedule}"
        );
        assert!(
            panel["dps"].as_str().unwrap().contains("Classe II"),
            "{schedule}"
        );
        assert_eq!(
            panel["earthing"], "TN-C-S",
            "the utility's PEN split at the entry"
        );
        assert!(
            electrical("{}").unwrap()["findings"]
                .to_string()
                .contains("elec:short-circuit")
        );

        electrical(r#"{"action":"voltage","short_ka":10,"earthing":"tt"}"#).unwrap();
        electrical(&format!(
            r#"{{"action":"assign","ids":["{}"],"modules":4}}"#,
            ids[0]
        ))
        .unwrap();
        let schedule = electrical(r#"{"action":"circuits"}"#).unwrap();
        assert_eq!(schedule["panel"]["icn_ka"], 10.0, "{schedule}");
        assert_eq!(schedule["panel"]["earthing"], "TT");
        let text = electrical("{}").unwrap()["findings"].to_string();
        assert!(
            text.contains("elec:panel-full") && !text.contains("elec:short-circuit"),
            "{text}"
        );
    }

    #[test]
    fn an_outlet_is_never_placed_on_a_window_pane_or_in_a_door() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"window","at":[100,0],"w":120,"elev":110,"h":100},{"cat":"door","at":[300,0]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let place = |json: &str| s.place(Parameters(serde_json::from_str(json).unwrap()));
        let pane = place(r#"{"items":[{"cat":"outlet-mid","at":[100,3]}]}"#).unwrap_err();
        assert!(pane.message.contains("vidro da janela"), "{pane:?}");
        let door = place(r#"{"items":[{"cat":"switch","at":[300,3],"elev":30}]}"#).unwrap_err();
        assert!(door.message.contains("vão da porta"), "{door:?}");
        // Beside the frame: refused, and the nearest free place is given.
        let framed = place(r#"{"items":[{"cat":"outlet-low","at":[258,3]}]}"#).unwrap_err();
        assert!(
            framed.message.contains("batente") && framed.message.contains("at=["),
            "{framed:?}"
        );
        place(r#"{"items":[{"cat":"outlet-low","at":[100,3]}]}"#)
            .expect("under the sill is a wall");
    }

    #[test]
    fn a_run_in_adhesive_tape_picks_its_model_and_buys_its_kits() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true},{"pts":[[400,0],[600,0],[600,300],[400,300]]}],
                    "rooms":[{"name":"Sala","at":[200,150]},{"name":"Banheiro","at":[500,150]}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"electrical-panel","at":[10,150]},{"cat":"outlet-low","at":[200,5]},{"cat":"outlet-low","at":[390,150]},
                                 {"cat":"outlet-mid","at":[590,150]},{"cat":"network-outlet","at":[200,295]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let ids: Vec<String> = reply
            .rsplit("ids=")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let electrical = |json: &str| {
            s.electrical(Parameters(serde_json::from_str(json).unwrap()))
                .map(|r| serde_json::from_str(&r).unwrap_or(serde_json::Value::String(r)))
        };
        let run = electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}","{}"],"via":"tape"}}"#,
            ids[1], ids[2]
        ))
        .unwrap();
        assert_eq!(run["via"], "tape", "{run}");
        assert_eq!(
            run["tape"]["model"], "EF18.9.18",
            "sockets take the earthed tape: {run}"
        );
        let bill = run["materials"].to_string();
        assert!(
            bill.contains("Leroy 9192080") || bill.contains("Leroy 91920815"),
            "{run}"
        );
        assert!(!bill.contains("Eletroduto"), "no conduit on tape: {run}");
        assert!(run["tape"]["total_brl"].as_f64().unwrap() > 0.0);
        // In the bathroom, and for network: refused.
        let wet = electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}"],"via":"tape"}}"#,
            ids[3]
        ))
        .unwrap_err();
        assert!(wet.message.contains("bathroom"), "{wet:?}");
        assert!(electrical(r#"{"action":"route","kind":"data","via":"tape"}"#).is_err());
        // A load past 20 A: refused, pointing to wire.
        electrical(&format!(
            r#"{{"action":"assign","ids":["{}"],"va":3500}}"#,
            ids[1]
        ))
        .unwrap();
        let heavy = electrical(&format!(
            r#"{{"action":"route","kind":"power","ids":["{}","{}"],"via":"tape"}}"#,
            ids[1], ids[2]
        ))
        .unwrap_err();
        assert!(heavy.message.contains("fio"), "{heavy:?}");
    }

    #[test]
    fn a_counter_and_its_outlet_tower_are_placed_in_one_call() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(r#"{"walls":[{"pts":[[0,0],[300,0],[300,250],[0,250]],"closed":true}],"rooms":[{"name":"Cozinha","at":[150,125]}]}"#).unwrap(),
        ))
        .unwrap();
        let reply = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"base-cabinet","at":[150,38],"w":200,"d":60,"h":90},{"cat":"outlet-tower","at":[90,38]}]}"#,
                )
                .unwrap(),
            ))
            .unwrap();
        let home = s.document.read().home().clone();
        let tower = home
            .furniture
            .iter()
            .find(|f| f.catalog == "outlet-tower")
            .unwrap();
        assert!(
            (tower.elevation - 90.0).abs() < 1e-9,
            "{reply}: seated on the top"
        );
        // A network point asked behind the counter comes out above its top.
        s.place(Parameters(
            serde_json::from_str(r#"{"items":[{"cat":"network-outlet","at":[150,5]}]}"#).unwrap(),
        ))
        .unwrap();
        let rj = s
            .document
            .read()
            .home()
            .furniture
            .iter()
            .find(|f| f.catalog == "network-outlet")
            .unwrap()
            .clone();
        assert!((rj.elevation - 105.0).abs() < 1e-9, "{}", rj.elevation);
        let loose = s
            .place(Parameters(
                serde_json::from_str(r#"{"items":[{"cat":"outlet-tower","at":[150,200]}]}"#)
                    .unwrap(),
            ))
            .unwrap_err();
        assert!(loose.message.contains("embutida"), "{loose:?}");
    }

    #[test]
    fn a_top_named_after_its_appliance_hosts_a_tower_and_update_can_declare_one_fixed() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(r#"{"walls":[{"pts":[[0,0],[600,0],[600,250],[0,250]],"closed":true}],"rooms":[{"name":"Cozinha","at":[300,125]}]}"#).unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"base-cabinet","at":[150,38],"w":200,"d":60,"h":91,"name":"Bancada contínua junto à geladeira"},{"cat":"base-cabinet","at":[450,38],"w":200,"d":60,"h":91,"name":"Geladeira duplex"}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(r#"{"items":[{"cat":"outlet-tower-auto","at":[110,38]}]}"#)
                .unwrap(),
        ))
        .unwrap();
        let tower = |s: &NewEraMcp| {
            s.place(Parameters(
                serde_json::from_str(r#"{"items":[{"cat":"outlet-tower-auto","at":[420,38]}]}"#)
                    .unwrap(),
            ))
        };
        assert!(tower(&s).is_err(), "a fridge is no host");
        let fridge = s
            .document
            .read()
            .home()
            .furniture
            .iter()
            .find(|f| f.name == "Geladeira duplex")
            .unwrap()
            .id;
        s.update(Parameters(
            serde_json::from_str(&format!(
                r#"{{"items":[{{"id":"{fridge}","fixed":true}}]}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        tower(&s).unwrap();
    }

    #[test]
    fn a_balcony_railing_and_its_glass_closure_take_no_outlet_and_are_checked_apart() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(r#"{"walls":[{"pts":[[0,0],[300,0],[300,150]]},{"pts":[[0,0],[0,150]]}],"rooms":[{"name":"Varanda","pts":[[0,0],[300,0],[300,150],[0,150]]}]}"#).unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(
                r#"{"items":[{"cat":"railing","at":[150,148],"h":110},{"cat":"balcony-glazing","at":[150,152],"glass":"tempered"}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        // An outlet on the railing: refused, saying what it is.
        let on_rail = s
            .place(Parameters(
                serde_json::from_str(r#"{"items":[{"cat":"outlet-low","at":[150,148]}]}"#).unwrap(),
            ))
            .unwrap_err();
        assert!(
            on_rail.message.contains("gradil") || on_rail.message.contains("parede"),
            "{on_rail:?}"
        );
        let home = s.document.read().home().clone();
        let glazing = home
            .furniture
            .iter()
            .find(|f| f.catalog == "balcony-glazing")
            .unwrap();
        assert_eq!(
            glazing
                .properties
                .get(newera_core::guard::GLASS_KEY)
                .map(String::as_str),
            Some("tempered")
        );
        // Railing and closure together: nothing to say about the guard.
        let guards = newera_core::guard::check(&home);
        assert!(guards.is_empty(), "{guards:?}");
    }
}
