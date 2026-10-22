//! What a project costs: the takeoff of the project (what each product
//! covers, the pieces bought) joined with each product's price and pack.
//!
//! The arithmetic is exact ([`Exact`]): no amount here is a float, and money
//! is written as decimal text. The rules, in order:
//!
//! 1. **Each surface's need**, in the product's base unit:
//!    - a covering (tile, laminate, and a finish of unknown type): net m² ×
//!      (1 + waste %);
//!    - paint: net m² × (1 + waste %) × coats ÷ spread rate (m²/L), litres;
//!      a surface without coats is painted once, and the line says so;
//!    - wallpaper, when the roll's width and length are known and the
//!      surface is a wall side (the takeoff gives its stretches of face):
//!      each stretch the band covers takes ⌈width ÷ roll width⌉ drops of its
//!      tallest covered height plus 10 cm of trim ([`TRIM_CM`]) plus one
//!      pattern repeat (none when the repeat is unknown, which the line says),
//!      a roll gives ⌊roll length ÷ drop⌋ drops, and the need is drops ÷
//!      drops per roll × (1 + waste %) rolls; a floor or ceiling, a wall the
//!      takeoff gave no stretches for, or a drop longer than the roll is
//!      counted by area — net m² × (1 + waste %) ÷ the roll's m² — and the
//!      line says which and why. Openings do not reduce drops: a drop is cut
//!      around them.
//! 2. **One sum per product** over all its surfaces, unrounded.
//! 3. **Whole packs once**: ⌈sum ÷ pack content⌉ — m² per pack, litres per
//!    can, one roll; a covering sold by the piece with a known tile size is
//!    bought in tiles.
//! 4. **Without pack content** the quantity is the need in its base unit,
//!    rounded up to 0.01, and the line says what is missing.
//! 5. **Price**: per the product's sale unit. A price per pack (or roll,
//!    or tile) times the packs; a price per m², litre or metre times what
//!    the packs hold of it; with no pack content, a price per the base unit
//!    times the quantity. A price whose unit cannot be matched to the
//!    quantity, a missing price, sale unit or currency leaves the line
//!    unpriced — never zero — and every total it belongs to incomplete.
//! 6. **Furniture**: the pieces bought; a product listed as a pack of
//!    `pack_pieces` is bought in ⌈pieces ÷ `pack_pieces`⌉ packs, and its
//!    price is the listing's — one piece, or one pack.
//! 7. **Money**: a line total is unit price × quantity, rounded half away
//!    from zero to the cent; totals are the sums of line totals, per shop,
//!    currency and VAT basis (included, excluded, unknown), and per currency
//!    and VAT basis — never converted, and prices with and without VAT
//!    never added together.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::decimal::Decimal;
use crate::exact::Exact;
use crate::product::{Fields, Kind};

/// A product of the library, as the estimate reads it.
#[derive(Debug, Clone)]
pub struct Product {
    pub id: String,
    pub shop: String,
    pub url: String,
    pub kind: Kind,
    pub fields: Fields,
}

impl Product {
    fn text(&self, name: &str) -> Option<&str> {
        self.fields.get(name).and_then(|f| f.value.as_str())
    }

    fn amount(&self, name: &str) -> Option<Exact> {
        self.fields
            .get(name)
            .and_then(|f| Exact::of_json(&f.value))
            .filter(|e| e.is_positive())
    }
}

/// What the takeoff says a project buys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Takeoff {
    pub finishes: Vec<Covers>,
    pub furniture: Vec<Bought>,
    /// Finishes whose surface is gone: `{surface, product}` as the takeoff
    /// gave them.
    pub gone: Vec<Value>,
}

/// Everything one finish product covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Covers {
    pub product: String,
    pub surfaces: Vec<Covered>,
}

/// One surface a finish covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Covered {
    pub surface: String,
    pub room: Option<String>,
    /// m².
    pub net: Exact,
    pub coats: Option<u32>,
    /// Percent.
    pub waste: Option<Exact>,
    /// A wall side's stretches of face the band covers: (width, tallest
    /// height), cm. `None` when the takeoff gave none: a floor or ceiling,
    /// or an editor older than the stretches.
    pub spans: Option<Vec<(Exact, Exact)>>,
}

/// The pieces of one furniture product bought.
#[derive(Debug, Clone, PartialEq)]
pub struct Bought {
    pub product: String,
    pub ids: Vec<String>,
    pub rooms: Vec<String>,
}

/// Reads the answer of the `takeoff` tool.
///
/// # Errors
///
/// When it is not a takeoff: says which part.
pub fn read_takeoff(answer: &Value) -> Result<Takeoff, String> {
    let rows = |name: &str| -> Result<&Vec<Value>, String> {
        answer[name]
            .as_array()
            .ok_or_else(|| format!("the takeoff has no {name}"))
    };
    let text = |row: &Value, name: &str| row[name].as_str().map(str::to_owned);
    let mut finishes = Vec::new();
    for row in rows("finishes")? {
        let product = text(row, "product").ok_or("a takeoff finish has no product")?;
        let mut surfaces = Vec::new();
        for s in row["surfaces"]
            .as_array()
            .ok_or("a takeoff finish has no surfaces")?
        {
            let net = Exact::of_json(&s["net_m2"]).ok_or("a takeoff surface has no net_m2")?;
            let spans = s["spans_cm"]
                .as_array()
                .map(|spans| {
                    spans
                        .iter()
                        .map(|span| Some((Exact::of_json(&span[0])?, Exact::of_json(&span[1])?)))
                        .collect::<Option<Vec<_>>>()
                        .ok_or("a takeoff span is not [width, height]")
                })
                .transpose()?;
            surfaces.push(Covered {
                surface: text(s, "surface").unwrap_or_default(),
                room: text(s, "room"),
                net,
                coats: s["coats"].as_u64().and_then(|c| u32::try_from(c).ok()),
                waste: Exact::of_json(&s["waste"]),
                spans,
            });
        }
        finishes.push(Covers { product, surfaces });
    }
    let mut furniture = Vec::new();
    for row in rows("furniture")? {
        let strings = |name: &str| -> Vec<String> {
            row[name]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        furniture.push(Bought {
            product: text(row, "product").ok_or("a takeoff piece count has no product")?,
            ids: strings("ids"),
            rooms: strings("rooms"),
        });
    }
    Ok(Takeoff {
        finishes,
        furniture,
        gone: answer["gone"].as_array().cloned().unwrap_or_default(),
    })
}

/// One line of an estimate. Amounts are decimal text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// furniture, covering, paint, wallpaper, extra, or unknown (a product
    /// not in the library).
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// An extra line's id, for `remove_line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    pub shop: Option<String>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub quantity: String,
    pub unit: String,
    /// Whole packs (cans, rolls, tiles, multipacks) bought, wherever they
    /// were counted — also when the quantity is those packs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packs: Option<String>,
    /// What the surfaces need, before packs: `[amount, unit]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need: Option<(String, String)>,
    pub unit_price: Option<String>,
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vat_included: Option<bool>,
    /// `None`: unpriced.
    pub total: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// What a line is about, before it has a quantity.
fn line(kind: &str, id: &str, product: Option<&Product>) -> Line {
    let text = |name: &str| product.and_then(|p| p.text(name)).map(str::to_owned);
    Line {
        kind: kind.to_owned(),
        product: Some(id.to_owned()),
        line: None,
        shop: product.map(|p| p.shop.clone()),
        title: text("title"),
        url: product.map(|p| p.url.clone()),
        quantity: String::new(),
        unit: String::new(),
        packs: None,
        need: None,
        unit_price: None,
        currency: text("currency"),
        vat_included: product
            .and_then(|p| p.fields.get("vat_included"))
            .and_then(|f| f.value.as_bool()),
        total: None,
        rooms: Vec::new(),
        surfaces: Vec::new(),
        pieces: Vec::new(),
        notes: Vec::new(),
    }
}

/// What one pack holds, by unit: the pack itself (`pack`, and its own name
/// — `roll`, `piece`), and the m², litres, metres or pieces in it.
#[derive(Debug, Clone)]
struct Pack {
    name: &'static str,
    holds: Vec<(&'static str, Exact)>,
}

impl Pack {
    fn of(name: &'static str, more: &[(&'static str, Option<Exact>)]) -> Self {
        let mut holds = vec![("pack", Exact::ONE), (name, Exact::ONE)];
        holds.extend(more.iter().filter_map(|(unit, n)| n.map(|n| (*unit, n))));
        Self { name, holds }
    }

    fn holds(&self, unit: &str) -> Option<Exact> {
        self.holds.iter().find(|(u, _)| *u == unit).map(|(_, n)| *n)
    }
}

/// What a product's surfaces need, and what it is bought in.
struct Need {
    amount: Exact,
    /// m2, litre, roll or piece.
    unit: &'static str,
    pack: Option<Pack>,
    /// The product field that would give the pack content, when it lacks.
    pack_field: Option<&'static str>,
    notes: Vec<String>,
}

const OVERFLOW: &str = "the amounts are too large to compute";

/// What each drop of wallpaper is cut longer than the height it covers, cm:
/// trimmed off at the ceiling and the skirting. No product field says it.
const TRIM_CM: i128 = 10;

/// `1 + waste %`.
fn with_waste(waste: Option<Exact>) -> Option<Exact> {
    Exact::ONE.plus(waste.unwrap_or(Exact::ZERO).over(Exact::int(100))?)
}

/// Net m² × (1 + waste %), summed.
fn area(surfaces: &[&Covered]) -> Option<Exact> {
    surfaces.iter().try_fold(Exact::ZERO, |sum, s| {
        sum.plus(s.net.times(with_waste(s.waste)?)?)
    })
}

fn covering(product: &Product, surfaces: &[&Covered]) -> Option<Need> {
    let pack_m2 = product.amount("pack_m2");
    let tile = product
        .amount("tile_width_cm")
        .zip(product.amount("tile_length_cm"))
        .and_then(|(w, l)| w.times(l)?.over(Exact::int(10_000)));
    let by_tile = product.text("sale_unit") == Some("piece");
    let pack = match (pack_m2, tile) {
        (Some(m2), _) => Some(Pack::of(
            "pack",
            &[("m2", Some(m2)), ("piece", product.amount("pack_pieces"))],
        )),
        (None, Some(tile)) if by_tile => Some(Pack::of("piece", &[("m2", Some(tile))])),
        _ => None,
    };
    Some(Need {
        amount: area(surfaces)?,
        unit: "m2",
        pack,
        pack_field: Some("pack_m2"),
        notes: Vec::new(),
    })
}

fn paint(product: &Product, surfaces: &[&Covered]) -> Option<Need> {
    let mut notes = Vec::new();
    if surfaces.iter().any(|s| s.coats.is_none()) {
        notes.push(
            "coats not set on some surfaces: one coat counted there (edit_finishes sets coats)"
                .to_owned(),
        );
    }
    let painted = surfaces.iter().try_fold(Exact::ZERO, |sum, s| {
        let coats = Exact::int(i128::from(s.coats.unwrap_or(1)));
        sum.plus(s.net.times(with_waste(s.waste)?)?.times(coats)?)
    })?;
    let Some(spread) = product.amount("spread_m2_per_litre") else {
        notes.push("spread rate unknown (spread_m2_per_litre): the quantity is m² to paint, coats included, not litres".to_owned());
        return Some(Need {
            amount: painted,
            unit: "m2",
            pack: None,
            pack_field: None,
            notes,
        });
    };
    let pack = product
        .amount("pack_litres")
        .map(|litres| Pack::of("pack", &[("litre", Some(litres))]));
    Some(Need {
        amount: painted.over(spread)?,
        unit: "litre",
        pack,
        pack_field: Some("pack_litres"),
        notes,
    })
}

fn wallpaper(product: &Product, surfaces: &[&Covered]) -> Option<Need> {
    let width = product.amount("roll_width_cm");
    let length = product.amount("roll_length_m");
    let (Some(width), Some(length)) = (width, length) else {
        // No roll to count drops in: by area, in what the shop packs.
        let pack = product
            .amount("pack_m2")
            .map(|m2| Pack::of("roll", &[("m2", Some(m2))]));
        return Some(Need {
            amount: area(surfaces)?,
            unit: "m2",
            pack,
            pack_field: None,
            notes: vec![
                "roll size unknown (roll_width_cm, roll_length_m): measured by area".to_owned(),
            ],
        });
    };
    let length_cm = length.times(Exact::int(100))?;
    let roll_m2 = width.times(length_cm)?.over(Exact::int(10_000))?;
    let repeat = product.amount("pattern_repeat_cm");
    let mut notes = Vec::new();
    let mut rolls = Exact::ZERO;
    let mut by_area: Vec<String> = Vec::new();
    let mut hung = false;
    for s in surfaces {
        let why = match &s.spans {
            None if s.surface.ends_with(":floor") || s.surface.ends_with(":ceiling") => {
                Some("a floor or ceiling")
            }
            None => Some("the takeoff gave no face widths: an editor older than this service"),
            Some(spans) => {
                let mut drops_of = Exact::ZERO;
                let mut fits = true;
                // Whether a drop was cut here: the repeat is worth a note
                // only then.
                let mut cut = false;
                for (span_width, height) in spans {
                    // A band above the wall's top covers nothing of this stretch.
                    if !height.is_positive() {
                        continue;
                    }
                    cut = true;
                    let drops = span_width.over(width)?.ceil();
                    let drop = height
                        .plus(Exact::int(TRIM_CM))?
                        .plus(repeat.unwrap_or(Exact::ZERO))?;
                    let per_roll = length_cm.over(drop)?.floor();
                    if per_roll < 1 {
                        fits = false;
                        break;
                    }
                    drops_of = drops_of.plus(Exact::ratio(drops, per_roll)?)?;
                }
                if fits {
                    hung |= cut;
                    rolls = rolls.plus(drops_of.times(with_waste(s.waste)?)?)?;
                    None
                } else {
                    Some("a drop longer than the roll")
                }
            }
        };
        if let Some(why) = why {
            by_area.push(format!("{} ({why})", s.surface));
            rolls = rolls.plus(s.net.times(with_waste(s.waste)?)?.over(roll_m2)?)?;
        }
    }
    if repeat.is_none() && hung {
        notes.push(
            "pattern repeat unknown (pattern_repeat_cm): drops are the wall's height and the trim"
                .to_owned(),
        );
    }
    if !by_area.is_empty() {
        notes.push(format!(
            "counted by area, not drops: {}",
            by_area.join(", ")
        ));
    }
    Some(Need {
        amount: rolls,
        unit: "roll",
        pack: Some(Pack::of(
            "roll",
            &[("m2", Some(roll_m2)), ("metre", Some(length))],
        )),
        pack_field: None,
        notes,
    })
}

/// Prices `need` by the product's price and sale unit into `line`.
fn priced(mut line: Line, product: Option<&Product>, need: Need) -> Option<Line> {
    line.notes.extend(need.notes);
    let price = product.and_then(|p| p.amount("price"));
    let sale_unit = product.and_then(|p| p.text("sale_unit"));
    // Written with as few decimals as it has: 2.5, not 2.50.
    let asked = Exact::of(need.amount.round_up(2)?).decimal()?;
    line.need = Some((asked.to_string(), need.unit.to_owned()));
    // How many of the sale unit are bought, and how many packs that is.
    let bought: Option<(Exact, &str, Option<i128>)> = if let Some(pack) = &need.pack {
        {
            let content = pack.holds(need.unit)?;
            let packs = need.amount.over(content)?.ceil();
            line.packs = Some(packs.to_string());
            line.quantity = packs.to_string();
            pack.name.clone_into(&mut line.unit);
            sale_unit.and_then(|unit| {
                Some((
                    Exact::int(packs).times(pack.holds(unit)?)?,
                    if unit == "pack" { pack.name } else { unit },
                    Some(packs),
                ))
            })
        }
    } else {
        line.quantity = asked.to_string();
        need.unit.clone_into(&mut line.unit);
        if product.is_some()
            && let Some(field) = need.pack_field
        {
            line.notes.push(format!(
                "pack content unknown ({field}): the quantity is what the surfaces need"
            ));
        }
        (sale_unit == Some(need.unit)).then(|| (Exact::of(asked), need.unit, None))
    };
    if product.is_none() {
        return Some(line);
    }
    let Some(price) = price else {
        line.notes
            .push("no price: edit_products sets it".to_owned());
        return Some(line);
    };
    let Some(sale_unit) = sale_unit else {
        line.notes.push(
            "sale unit unknown (sale_unit): the price cannot be matched to the quantity".to_owned(),
        );
        return Some(line);
    };
    let Some((amount, unit, packs)) = bought else {
        line.notes.push(format!(
            "sold per {sale_unit}, which the quantity in {} cannot be counted in",
            line.unit
        ));
        return Some(line);
    };
    if line.currency.is_none() {
        line.notes
            .push("currency unknown: edit_products sets it".to_owned());
        return Some(line);
    }
    // The quantity in the unit the price is for: packs, or what they hold.
    line.quantity = amount.decimal()?.to_string();
    unit.clone_into(&mut line.unit);
    line.packs = packs.map(|p| p.to_string());
    line.unit_price = Some(price.decimal()?.to_string());
    line.total = Some(amount.times(price)?.round_half(2)?.to_string());
    Some(line)
}

fn finish_line(covers: &Covers, product: Option<&Product>) -> Line {
    let surfaces: Vec<&Covered> = covers.surfaces.iter().collect();
    let finish_type = product.and_then(|p| p.text("finish_type"));
    let kind = match (product, finish_type) {
        (None, _) => "unknown",
        (Some(_), Some("paint")) => "paint",
        (Some(_), Some("wallpaper")) => "wallpaper",
        (Some(_), _) => "covering",
    };
    let mut out = line(kind, &covers.product, product);
    for s in &covers.surfaces {
        if !out.surfaces.contains(&s.surface) {
            out.surfaces.push(s.surface.clone());
        }
        if let Some(room) = &s.room
            && !out.rooms.contains(room)
        {
            out.rooms.push(room.clone());
        }
    }
    out.rooms.sort();
    let need = match product {
        None => area(&surfaces).map(|amount| Need {
            amount,
            unit: "m2",
            pack: None,
            pack_field: None,
            notes: Vec::new(),
        }),
        Some(product) => match kind {
            "paint" => paint(product, &surfaces),
            "wallpaper" => wallpaper(product, &surfaces),
            _ => covering(product, &surfaces).map(|mut need| {
                if finish_type.is_none() {
                    need.notes.push(
                        "finish type unknown (finish_type): measured by area like a covering"
                            .to_owned(),
                    );
                }
                need
            }),
        },
    };
    let missing = |mut out: Line| {
        out.notes.insert(
            0,
            format!(
                "product {} is not in this account's library (products lists it)",
                covers.product
            ),
        );
        out
    };
    match need.and_then(|need| priced(out.clone(), product, need)) {
        Some(done) if product.is_none() => missing(done),
        Some(done) => done,
        None => {
            out.notes.push(OVERFLOW.to_owned());
            "0".clone_into(&mut out.quantity);
            out
        }
    }
}

fn furniture_line(bought: &Bought, product: Option<&Product>) -> Line {
    let kind = if product.is_some() {
        "furniture"
    } else {
        "unknown"
    };
    let mut out = line(kind, &bought.product, product);
    out.pieces.clone_from(&bought.ids);
    out.rooms.clone_from(&bought.rooms);
    let pieces = i128::try_from(bought.ids.len()).unwrap_or(i128::MAX);
    let Some(product) = product else {
        out.quantity = pieces.to_string();
        "piece".clone_into(&mut out.unit);
        out.notes.push(format!(
            "product {} is not in this account's library (products lists it)",
            bought.product
        ));
        return out;
    };
    let per_pack = product
        .amount("pack_pieces")
        .map_or(1, |n| n.floor().max(1));
    let packs = Exact::int(pieces)
        .over(Exact::int(per_pack))
        .map_or(pieces, Exact::ceil);
    out.quantity = packs.to_string();
    if per_pack > 1 {
        "pack".clone_into(&mut out.unit);
        out.packs = Some(packs.to_string());
        out.need = Some((pieces.to_string(), "piece".to_owned()));
        out.notes
            .push(format!("sold in packs of {per_pack} pieces"));
    } else {
        "piece".clone_into(&mut out.unit);
    }
    match (product.amount("price"), &out.currency) {
        (None, _) => out.notes.push("no price: edit_products sets it".to_owned()),
        (Some(_), None) => out
            .notes
            .push("currency unknown: edit_products sets it".to_owned()),
        (Some(price), Some(_)) => {
            out.unit_price = price.decimal().map(|d| d.to_string());
            if let Some(total) = Exact::int(packs).times(price).and_then(|t| t.round_half(2)) {
                out.total = Some(total.to_string());
            } else {
                out.unit_price = None;
                out.notes.push(OVERFLOW.to_owned());
            }
        }
    }
    out
}

/// The lines of what the project buys: one per product, finishes first.
pub fn lines(takeoff: &Takeoff, products: &BTreeMap<String, Product>) -> Vec<Line> {
    takeoff
        .finishes
        .iter()
        .map(|c| finish_line(c, products.get(&c.product)))
        .chain(
            takeoff
                .furniture
                .iter()
                .map(|b| furniture_line(b, products.get(&b.product))),
        )
        .collect()
}

/// A line added to the estimate by hand: labour, delivery, glue, primer.
#[derive(Debug, Clone)]
pub struct Extra {
    pub id: String,
    pub title: String,
    pub shop: Option<String>,
    pub quantity: Decimal,
    pub unit: String,
    pub unit_price: Decimal,
    pub currency: String,
    /// Whether the price includes VAT; `None`: not said.
    pub vat_included: Option<bool>,
    pub note: Option<String>,
}

/// An extra line as the estimate shows it.
pub fn extra_line(extra: &Extra) -> Line {
    let total = Exact::of(extra.quantity)
        .times(Exact::of(extra.unit_price))
        .and_then(|t| t.round_half(2));
    let mut notes: Vec<String> = extra.note.iter().cloned().collect();
    if total.is_none() {
        notes.push(OVERFLOW.to_owned());
    }
    Line {
        kind: "extra".to_owned(),
        product: None,
        line: Some(extra.id.clone()),
        shop: extra.shop.clone(),
        title: Some(extra.title.clone()),
        url: None,
        quantity: extra.quantity.to_string(),
        unit: extra.unit.clone(),
        packs: None,
        need: None,
        unit_price: Some(extra.unit_price.to_string()),
        currency: Some(extra.currency.clone()),
        vat_included: extra.vat_included,
        total: total.map(|t| t.to_string()),
        rooms: Vec::new(),
        surfaces: Vec::new(),
        pieces: Vec::new(),
        notes,
    }
}

/// How lines are grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Shop,
    Room,
    Kind,
}

impl GroupBy {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "shop" => Some(Self::Shop),
            "room" => Some(Self::Room),
            "kind" => Some(Self::Kind),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shop => "shop",
            Self::Room => "room",
            Self::Kind => "kind",
        }
    }

    /// The group a line falls in. A product is bought once for all its
    /// rooms, so a line of several rooms is a group of its own, not split.
    fn of(self, line: &Line) -> String {
        match self {
            Self::Shop => line.shop.clone().unwrap_or_else(|| "no shop".to_owned()),
            Self::Kind => line.kind.clone(),
            Self::Room => match line.rooms.as_slice() {
                [] => "no room".to_owned(),
                [room] => room.clone(),
                _ => "several rooms".to_owned(),
            },
        }
    }
}

fn amount(text: Option<&String>) -> Option<Exact> {
    text.and_then(|t| Decimal::parse(t)).map(Exact::of)
}

/// What a line's price says of VAT: `included`, `excluded` or `unknown`.
fn vat_basis(line: &Line) -> &'static str {
    match line.vat_included {
        Some(true) => "included",
        Some(false) => "excluded",
        None => "unknown",
    }
}

/// Sums by currency and VAT basis: `[{currency, vat, total, complete}]` —
/// prices with VAT, without it and not known either way are never added
/// together. A line without a total makes every sum incomplete, and the
/// unpriced lines are a sum of their own, with no currency, VAT or total.
fn sums<'a>(lines: impl IntoIterator<Item = &'a Line>) -> Vec<Value> {
    let lines: Vec<&Line> = lines.into_iter().collect();
    let complete = lines.iter().all(|l| l.total.is_some());
    let mut by: BTreeMap<Option<(String, &'static str)>, Option<Exact>> = BTreeMap::new();
    for line in &lines {
        let key = line
            .total
            .as_ref()
            .and(line.currency.clone())
            .map(|c| (c, vat_basis(line)));
        let sum = by.entry(key).or_insert(Some(Exact::ZERO));
        *sum = match (*sum, amount(line.total.as_ref())) {
            (Some(sum), Some(total)) => sum.plus(total),
            _ => None,
        };
    }
    by.into_iter()
        .map(|(key, sum)| {
            // A sum of cents is exact: this only writes it to the cent.
            let total = key
                .as_ref()
                .and(sum)
                .and_then(|sum| sum.round_half(2))
                .map(|d| d.to_string());
            let (currency, vat) = key.unzip();
            json!({"currency": currency, "vat": vat, "total": total, "complete": complete && total.is_some()})
        })
        .collect()
}

/// An estimate's lines, grouped, with totals per shop and currency and per
/// currency.
pub fn estimate(lines: &[Line], group_by: GroupBy) -> Value {
    let mut groups: Vec<(String, Vec<&Line>)> = Vec::new();
    for line in lines {
        let key = group_by.of(line);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(line),
            None => groups.push((key, vec![line])),
        }
    }
    let mut shops: BTreeMap<Option<String>, Vec<&Line>> = BTreeMap::new();
    for line in lines {
        shops.entry(line.shop.clone()).or_default().push(line);
    }
    let totals: Vec<Value> = shops
        .into_iter()
        .flat_map(|(shop, members)| {
            sums(members).into_iter().map(move |mut sum| {
                sum["shop"] = json!(shop);
                sum
            })
        })
        .collect();
    json!({
        "group_by": group_by.as_str(),
        "groups": groups
            .into_iter()
            .map(|(name, members)| json!({
                "name": name,
                "totals": sums(members.iter().copied()),
                "lines": members,
            }))
            .collect::<Vec<_>>(),
        "totals": totals,
        "by_currency": sums(lines),
        "complete": lines.iter().all(|l| l.total.is_some()),
    })
}

/// The columns of the CSV sheet.
const COLUMNS: [&str; 15] = [
    "group",
    "kind",
    "shop",
    "title",
    "url",
    "quantity",
    "unit",
    "packs",
    "unit_price",
    "currency",
    "vat",
    "total",
    "rooms",
    "surfaces",
    "notes",
];

/// The lines of an estimate as a CSV sheet, in its groups, with the totals
/// per currency under them.
pub fn csv(estimate: &Value) -> String {
    let cell = |value: &Value| -> String {
        let text = match value {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            Value::Array(items) => items
                .iter()
                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                .collect::<Vec<_>>()
                .join(" "),
            other => other.to_string(),
        };
        // A cell a spreadsheet would read as a formula (OWASP: CSV
        // injection) — a shop's title, an agent's line — is text.
        let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
            format!("'{text}")
        } else {
            text
        };
        if text.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text
        }
    };
    let mut out = COLUMNS.join(",");
    out.push_str("\r\n");
    for group in estimate["groups"].as_array().into_iter().flatten() {
        for line in group["lines"].as_array().into_iter().flatten() {
            let row: Vec<String> = COLUMNS
                .iter()
                .map(|c| match *c {
                    "group" => cell(&group["name"]),
                    "title" => cell(if line["title"].is_null() {
                        &line["product"]
                    } else {
                        &line["title"]
                    }),
                    "total" if line["total"].is_null() => "unpriced".to_owned(),
                    "vat" => match line["vat_included"].as_bool() {
                        Some(true) => "included".to_owned(),
                        Some(false) => "excluded".to_owned(),
                        None => "unknown".to_owned(),
                    },
                    "notes" => line["notes"]
                        .as_array()
                        .map(|n| {
                            cell(&json!(
                                n.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            ))
                        })
                        .unwrap_or_default(),
                    other => cell(&line[other]),
                })
                .collect();
            out.push_str(&row.join(","));
            out.push_str("\r\n");
        }
    }
    for sum in estimate["by_currency"].as_array().into_iter().flatten() {
        let total = if sum["complete"] == true {
            cell(&sum["total"])
        } else if sum["total"].is_null() {
            "unpriced".to_owned()
        } else {
            format!("{} (incomplete)", cell(&sum["total"]))
        };
        let mut row = vec![String::new(); COLUMNS.len()];
        "total".clone_into(&mut row[0]);
        row[9] = cell(&sum["currency"]);
        row[10] = cell(&sum["vat"]);
        row[11] = total;
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests;
