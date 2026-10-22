//! Shop products on surfaces, and the quantities a project buys.
//!
//! A finish product (tile, laminate, paint, wallpaper) is put on a room's
//! floor or ceiling or on a wall side, whole or in a height band; the
//! project keeps the product id, the band, the coats and the waste. The
//! takeoff measures what each product covers, and counts the furniture
//! bought, per product id: the hosted service prices it.

use newera_core::finishes::{Band, FinishAssignment, Side, Surface};
use newera_core::{Command, Element};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::NewEraMcp;
use super::reply::{core, invalid, ok, write_action};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct FinishesParams {
    /// Only this surface, e.g. `r3:floor` or `w5:left` (default every one).
    surface: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct EditFinishesParams {
    /// `assign` puts a product on a surface, replacing the one on the same band; `remove` takes it off.
    #[schemars(extend("enum" = ["assign", "remove"]))]
    action: Option<String>,
    /// `r3:floor`, `r3:ceiling`, `w5:left` or `w5:right` (sides as seen walking from the wall's a to b).
    surface: Option<String>,
    /// For `assign`: the hosted service's product id.
    product: Option<String>,
    /// Height on a wall side, `[from, to]` cm above the floor (default all of it); `remove` without it clears the whole surface.
    band: Option<[f64; 2]>,
    /// For `assign`: coats of paint.
    coats: Option<u32>,
    /// For `assign`: waste %, bought on top for cuts and breakage.
    waste: Option<f64>,
    /// For `assign`: the surface's look, as `update` takes it (`#rrggbb`, `tiles #ffffff 30x60`, `img:…`). Hosted, a whole surface without it takes the product's: its swatch at its size, or its colour.
    mat: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct TakeoffParams {
    /// Only this product id.
    product: Option<String>,
    /// Only this room, e.g. `r3`: its floor, ceiling, the wall sides facing it, the pieces in it.
    room: Option<String>,
}

fn surface(raw: Option<&str>) -> Result<Surface, ErrorData> {
    raw.ok_or_else(|| invalid("`surface` is required: r3:floor, r3:ceiling, w5:left or w5:right"))?
        .parse()
        .map_err(|e: newera_core::finishes::ParseSurfaceError| invalid(e.to_string()))
}

fn band(raw: Option<[f64; 2]>) -> Option<Band> {
    raw.map(|[from, to]| Band { from, to })
}

/// m², to the thousandth (10 cm²).
fn m2(value: f64) -> Value {
    json!((value * 1000.0).round() / 1000.0)
}

/// A length to the millimetre.
fn cm(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[tool_router(router = finishes_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        name = "finishes",
        description = "Which shop product covers which floor, ceiling or wall side (tile, laminate, paint, wallpaper): rows [surface, room, product, band [from,to] cm, coats, waste %]; gone: surfaces deleted since. edit_finishes changes them."
    )]
    pub(crate) fn read_finishes(
        &self,
        Parameters(p): Parameters<FinishesParams>,
    ) -> Result<String, ErrorData> {
        let doc = self.document.read();
        let home = doc.home();
        let only = p.surface.as_deref().map(|s| surface(Some(s))).transpose()?;
        let mut gone = Vec::new();
        let rows: Vec<Value> = home
            .finish_assignments
            .iter()
            .filter(|a| only.is_none_or(|s| a.surface == s))
            .inspect(|a| {
                let surface = a.surface.to_string();
                if !a.surface.exists(home) && !gone.contains(&surface) {
                    gone.push(surface);
                }
            })
            .map(|a| {
                json!([
                    a.surface.to_string(),
                    newera_core::takeoff::room_of(home, a.surface).map(|r| r.to_string()),
                    a.product,
                    a.band.map(|b| [b.from, b.to]),
                    a.coats,
                    a.waste,
                ])
            })
            .collect();
        let mut out = json!({
            "columns": ["surface", "room", "product", "band", "coats", "waste"],
            "rows": rows,
        });
        if !gone.is_empty() {
            out["gone"] = json!(gone);
        }
        Ok(out.to_string())
    }

    #[tool(
        description = "Put a shop product (tile, laminate, paint, wallpaper) on a room's floor or ceiling or a wall side, or take it off. assign {surface, product, band?, coats?, waste?, mat?}: band [from,to] cm splits a wall side (tile below, paint above); mat sets the look in the same step. remove {surface, band?}. One undo step. finishes lists them."
    )]
    pub(crate) fn edit_finishes(
        &self,
        Parameters(p): Parameters<EditFinishesParams>,
    ) -> Result<String, ErrorData> {
        let action = write_action(p.action.as_deref(), &["assign", "remove"], "finishes")?;
        let mut doc = self.document.write();
        let at = surface(p.surface.as_deref())?;
        let band = band(p.band);
        let mut commands = Vec::new();
        if action == "assign" {
            let product = p
                .product
                .clone()
                .ok_or_else(|| invalid("assign: `product`, the hosted service's product id"))?;
            if product.starts_with(crate::PRODUCT_REF) {
                return Err(invalid(format!(
                    "{product}: {}",
                    crate::PRODUCTS_ARE_HOSTED
                )));
            }
            commands.push(Command::AssignFinish {
                assignment: FinishAssignment {
                    surface: at,
                    product,
                    band,
                    coats: p.coats,
                    waste: p.waste,
                },
                index: None,
            });
            if let Some(raw) = &p.mat {
                let look = crate::edit::material(raw).map_err(invalid)?;
                let element = doc
                    .home()
                    .element(at.element())
                    .ok_or_else(|| invalid(format!("{} not found", at.element())))?;
                commands.push(Command::update(match (element, at) {
                    (Element::Room(mut room), Surface::Floor(_)) => {
                        room.floor_material = look;
                        Element::Room(room)
                    }
                    (Element::Room(mut room), _) => {
                        room.ceiling_material = look;
                        Element::Room(room)
                    }
                    (Element::Wall(mut wall), Surface::Wall(_, Side::Left)) => {
                        wall.left_side = look;
                        Element::Wall(wall)
                    }
                    (Element::Wall(mut wall), _) => {
                        wall.right_side = look;
                        Element::Wall(wall)
                    }
                    (other, _) => other,
                }));
            }
        } else {
            let bands: Vec<Option<Band>> = match band {
                Some(_) => vec![band],
                None => doc
                    .home()
                    .finish_assignments
                    .iter()
                    .filter(|a| a.surface == at)
                    .map(|a| a.band)
                    .collect(),
            };
            if bands.is_empty() {
                return Err(invalid(format!("{at} has no finish (finishes lists them)")));
            }
            commands.extend(
                bands
                    .into_iter()
                    .map(|band| Command::RemoveFinish { surface: at, band }),
            );
        }
        doc.execute(Command::Batch { commands }).map_err(core)?;
        Ok(ok(&doc, &[]))
    }

    #[tool(
        name = "takeoff",
        description = "Quantities for an estimate, per shop product id: the m² of floor, ceiling and wall area each finish covers — net, gross and doors or windows deducted, per storey and room — and the pieces to buy: each with a product, unless update sets purchase=existing or excluded. Packs, waste and coats are left to the estimate. finishes says what covers what."
    )]
    pub(crate) fn takeoff(
        &self,
        Parameters(p): Parameters<TakeoffParams>,
    ) -> Result<String, ErrorData> {
        let doc = self.document.read();
        let home = doc.home();
        let room = p
            .room
            .as_deref()
            .map(|raw| {
                let id: newera_core::RoomId =
                    raw.parse().map_err(|e| invalid(format!("room: {e}")))?;
                home.room(id)
                    .map(|r| r.id)
                    .ok_or_else(|| invalid(format!("{raw} not found (home lists the rooms)")))
            })
            .transpose()?;
        let taken = newera_core::takeoff::takeoff(
            home,
            newera_core::takeoff::Only {
                product: p.product.as_deref(),
                room,
            },
        );
        let finishes: Vec<Value> = taken
            .finishes
            .iter()
            .map(|product| {
                let surfaces: Vec<Value> = product
                    .surfaces
                    .iter()
                    .map(|s| {
                        let mut row = json!({
                            "surface": s.surface.to_string(),
                            "gross_m2": m2(s.gross),
                            "openings_m2": m2(s.openings),
                            "net_m2": m2(s.net),
                        });
                        if let Some(level) = s.level.filter(|_| !home.levels.is_empty()) {
                            row["level"] = json!(level.to_string());
                        }
                        if let Some(room) = s.room {
                            row["room"] = json!(room.to_string());
                        }
                        if let Some(band) = s.band {
                            row["band"] = json!([band.from, band.to]);
                        }
                        if let Some(coats) = s.coats {
                            row["coats"] = json!(coats);
                        }
                        if let Some(waste) = s.waste {
                            row["waste"] = json!(waste);
                        }
                        // Every wall side says its stretches, none included:
                        // the estimate tells a wall with nothing to hang from
                        // an editor that does not say.
                        if matches!(s.surface, Surface::Wall(..)) {
                            row["spans_cm"] = json!(
                                s.spans
                                    .iter()
                                    .map(|(width, height)| [cm(*width), cm(*height)])
                                    .collect::<Vec<_>>()
                            );
                        }
                        if !s.cut_by.is_empty() {
                            row["openings"] =
                                json!(s.cut_by.iter().map(ToString::to_string).collect::<Vec<_>>());
                        }
                        row
                    })
                    .collect();
                json!({
                    "product": product.product,
                    "net_m2": m2(product.net),
                    "gross_m2": m2(product.gross),
                    "openings_m2": m2(product.openings),
                    "surfaces": surfaces,
                })
            })
            .collect();
        let furniture: Vec<Value> = taken
            .furniture
            .iter()
            .map(|count| {
                let mut row = json!({
                    "product": count.product,
                    "pieces": count.pieces.len(),
                    "ids": count.pieces.iter().map(ToString::to_string).collect::<Vec<_>>(),
                });
                // The rooms the pieces stand in, as the room filter finds them.
                let mut rooms: Vec<String> = count
                    .pieces
                    .iter()
                    .filter_map(|id| {
                        let piece = home.find_piece(*id)?;
                        home.room_at(piece.position, home.piece_level(*id))
                            .map(|r| r.id.to_string())
                    })
                    .collect();
                rooms.sort();
                rooms.dedup();
                if !rooms.is_empty() {
                    row["rooms"] = json!(rooms);
                }
                row
            })
            .collect();
        let mut out = json!({"finishes": finishes, "furniture": furniture});
        if !taken.missing.is_empty() {
            out["gone"] = json!(
                taken
                    .missing
                    .iter()
                    .map(|a| json!({"surface": a.surface.to_string(), "product": a.product}))
                    .fold(Vec::new(), |mut once, row| {
                        if !once.contains(&row) {
                            once.push(row);
                        }
                        once
                    })
            );
        }
        Ok(out.to_string())
    }
}

#[cfg(test)]
mod tests;
