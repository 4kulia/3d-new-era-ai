//! A product's fields, read without judgement from what `AnySite` returned.
//!
//! Two sources, each read by fixed rules measured on recorded responses
//! (`tests/fixtures/`):
//!
//! - an adapter's item ([`from_ikea`]): named fields, taken as they are;
//! - a cleaned page ([`from_page`]): a heading, the price as shown, and the
//!   labels of the specification table in Dutch — `Inhoud per pak voor`,
//!   `Rendement per verpakking`, `Rendement per liter`, `Product afmetingen
//!   (cm)` — read as lines ([`crate::page`]).
//!
//! A rule that does not match leaves the field out; nothing is guessed.
//! Every field says which rule and which words it came from.

use regex_lite::Regex;
use serde_json::{Value, json};

use crate::decimal::Decimal;
use crate::page::Page;
use crate::product::{Evidence, Field, Fields, Kind};

fn put(fields: &mut Fields, name: &str, value: Value, evidence: Evidence, from: impl Into<String>) {
    fields.entry(name.to_owned()).or_insert(Field {
        value,
        evidence,
        from: from.into(),
    });
}

// ------------------------------------------------------------------ IKEA

/// An `ikea/products` item.
///
/// The price is what anyone pays: when the item's `price` is the IKEA Family
/// members' price (`price_tag` `FAMILY_PRICE`), `previous_price` is the
/// price for everyone else and becomes `price`, and the members' price is
/// kept as `member_price`.
pub fn from_ikea(item: &Value) -> Fields {
    let mut fields = Fields::new();
    let text = |key: &str| item[key].as_str().map(str::trim).filter(|s| !s.is_empty());
    let mut title = Vec::new();
    title.extend(text("name"));
    title.extend(text("type_name"));
    let mut title = title.join(" ");
    for part in [text("variant_text"), text("measure_text")]
        .into_iter()
        .flatten()
    {
        title.push_str(", ");
        title.push_str(part);
    }
    if !title.is_empty() {
        put(
            &mut fields,
            "title",
            json!(title),
            Evidence::Adapter,
            "name, type_name, variant_text, measure_text",
        );
    }
    if let Some(id) = text("id") {
        put(&mut fields, "article", json!(id), Evidence::Adapter, "id");
    }
    if let Some(kind) = text("type_name") {
        put(
            &mut fields,
            "product_type",
            json!(kind),
            Evidence::Adapter,
            "type_name",
        );
    }
    put(
        &mut fields,
        "brand",
        json!("IKEA"),
        Evidence::Adapter,
        "ikea/products",
    );
    let amount = |key: &str| {
        item[key]
            .as_f64()
            .and_then(|n| Decimal::parse(&n.to_string()))
            .filter(|d| d.is_positive())
    };
    let family = item["price_tag"].as_str() == Some("FAMILY_PRICE");
    match (family, amount("price"), amount("previous_price")) {
        (true, Some(members), Some(everyone)) => {
            put(
                &mut fields,
                "price",
                json!(everyone.to_string()),
                Evidence::Adapter,
                "previous_price (price is the IKEA Family price)",
            );
            put(
                &mut fields,
                "member_price",
                json!(members.to_string()),
                Evidence::Adapter,
                "price (IKEA Family)",
            );
        }
        (_, Some(price), _) => {
            put(
                &mut fields,
                "price",
                json!(price.to_string()),
                Evidence::Adapter,
                "price",
            );
        }
        _ => {}
    }
    if let Some(currency) = text("currency") {
        put(
            &mut fields,
            "currency",
            json!(currency),
            Evidence::Adapter,
            "currency",
        );
    }
    if item["unit"].as_str() == Some("PIECE") {
        put(
            &mut fields,
            "sale_unit",
            json!("piece"),
            Evidence::Adapter,
            "unit PIECE",
        );
    }
    for (name, labels) in [
        ("width_cm", ["Breedte", "Width"]),
        ("depth_cm", ["Diepte", "Depth"]),
        ("height_cm", ["Hoogte", "Height"]),
    ] {
        let found = item["measurements"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["name"].as_str().is_some_and(|n| labels.contains(&n)));
        if let Some(m) = found
            && let Some(metric) = m["metric"].as_str()
            && let Some(cm) = centimetres(metric, None)
        {
            put(
                &mut fields,
                name,
                json!(cm),
                Evidence::Adapter,
                format!(
                    "measurements {} {metric}",
                    m["name"].as_str().unwrap_or_default()
                ),
            );
        }
    }
    if let Some(colour) = item["colors"].as_array().and_then(|c| c.first()) {
        if let Some(name) = colour["name"].as_str() {
            put(
                &mut fields,
                "colour",
                json!(name),
                Evidence::Adapter,
                "colors[0].name",
            );
        }
        if let Some(hex) = colour["hex"]
            .as_str()
            .filter(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()))
        {
            put(
                &mut fields,
                "colour_hex",
                json!(format!("#{}", hex.to_lowercase())),
                Evidence::Adapter,
                "colors[0].hex",
            );
        }
    }
    let main = item["images"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["image_type"].as_str() == Some("MAIN_PRODUCT_IMAGE"));
    if let Some(image) = main.and_then(|i| i["image"].as_str()) {
        put(
            &mut fields,
            "image",
            json!(image),
            Evidence::Adapter,
            "images MAIN_PRODUCT_IMAGE",
        );
    }
    fields
}

// ------------------------------------------------------------------ pages

/// A `webparser` result: its `cleaned_html` read as lines. `kind` is the
/// calling agent's word for what the product is; without it, a page that
/// shows how a finish is sold (m² per pack, litres, m² per litre, a price
/// per m²) is a finish and any other a piece of furniture. Returns the kind
/// with the fields.
pub fn from_page(result: &Value, kind: Option<Kind>) -> (Kind, Fields) {
    let page = Page::read(result["cleaned_html"].as_str().unwrap_or_default());
    let mut fields = Fields::new();
    // The heading, never the `<title>`: a challenge page ("Just a
    // moment...", "Client Challenge") has a title and no heading.
    if let Some((_, heading)) = page.heading() {
        put(&mut fields, "title", json!(heading), Evidence::Text, "h1");
    }
    labels(&page, &mut fields);
    prices(&page, &mut fields);
    let kind =
        kind.unwrap_or_else(|| finish_type(&fields).map_or(Kind::Furniture, |_| Kind::Finish));
    if kind == Kind::Finish {
        if let Some(t) = finish_type(&fields) {
            let from = match t {
                "paint" => "litres or m² per litre on the page",
                _ => "m² per pack or a price per m² on the page",
            };
            put(&mut fields, "finish_type", json!(t), Evidence::Text, from);
            if t == "covering"
                && let Some((_, heading)) = page.heading()
            {
                heading_tile(heading, &mut fields);
            }
        }
        if fields.get("finish_type").and_then(|f| f.value.as_str()) != Some("paint")
            && let Some((value, from)) = page.value("Rendement per verpakking")
            && let Some((m2, "m2")) = measure(value)
        {
            put(&mut fields, "pack_m2", json!(m2), Evidence::Text, from);
        }
    } else {
        furniture_box(&page, &mut fields);
    }
    sale_unit(kind, &mut fields);
    if let Some((_, heading)) = page.heading()
        && let Some(src) = page.image(heading)
    {
        put(
            &mut fields,
            "image",
            json!(src),
            Evidence::Text,
            "the first image described as the heading",
        );
    }
    (kind, fields)
}

/// The specification labels measured on the recorded pages.
fn labels(page: &Page, fields: &mut Fields) {
    for (label, name) in [
        ("Merk", "brand"),
        ("Artikelnummer", "article"),
        ("EAN", "ean"),
        ("EAN nummer", "ean"),
        ("Kleur", "colour"),
    ] {
        if let Some((value, from)) = page.value(label) {
            put(fields, name, json!(value), Evidence::Text, from);
        }
    }
    // Hornbach writes the number after the label on one line.
    let art = Regex::new(r"^Art\.\s?nr\.?\s*(\d+)$").expect("regex");
    if let Some(caps) = page.lines.iter().find_map(|l| art.captures(l)) {
        put(
            fields,
            "article",
            json!(&caps[1]),
            Evidence::Text,
            caps[0].to_owned(),
        );
    }
    // Praxis gives a paint's colour as sRGB components.
    if let Some((code, from)) = page.value("Kleurcode")
        && page
            .value("Kleurcode systeem")
            .is_some_and(|(s, _)| s.eq_ignore_ascii_case("sRGB"))
        && let Some(hex) = rgb_hex(code)
    {
        put(fields, "colour_hex", json!(hex), Evidence::Text, from);
    }
    for label in ["Inhoud per pak voor", "m² per pak", "Inhoud"] {
        if let Some((value, from)) = page.value(label)
            && let Some((m2, "m2")) = measure(value)
        {
            put(fields, "pack_m2", json!(m2), Evidence::Text, from);
        }
    }
    if let Some((value, from)) = page.value("Inhoud")
        && let Some((litres, "l")) = measure(value)
    {
        put(fields, "pack_litres", json!(litres), Evidence::Text, from);
    }
    if let Some((value, from)) = page.value("Inhoud per pak")
        && let Some((count, "piece")) = measure(value)
        && count.fract() == 0.0
        && count > 0.0
    {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a whole count above zero"
        )]
        let count = count as u64;
        put(fields, "pack_pieces", json!(count), Evidence::Text, from);
    }
    if let Some((value, from)) = page.value("Rendement per liter")
        && let Some((m2, "m2")) = measure(value)
    {
        put(
            fields,
            "spread_m2_per_litre",
            json!(m2),
            Evidence::Text,
            from,
        );
    }
    // The visual size of one tile or plank, apart from what a pack covers.
    for label in ["Nominale afmeting in cm", "Afmetingen fabrikant ca. (BxL)"] {
        if let Some((value, from)) = page.value(label)
            && let Some([w, l]) = pair(value, label.ends_with("cm"))
        {
            put(
                fields,
                "tile_width_cm",
                json!(w),
                Evidence::Text,
                from.clone(),
            );
            put(fields, "tile_length_cm", json!(l), Evidence::Text, from);
        }
    }
    for (label, name) in [
        ("Breedte plank", "tile_width_cm"),
        ("Lengte plank", "tile_length_cm"),
    ] {
        if let Some((value, from)) = page.value(label)
            && let Some(cm) = centimetres(value, None)
        {
            put(fields, name, json!(cm), Evidence::Text, from);
        }
    }
}

/// The price as the page shows it, by the first rule that finds one:
///
/// 1. a pack price in words: `Komt overeen met 18,13 € per Pak`,
///    `Pakket prijs 24,54`;
/// 2. a price per unit in words: `prijs — 11,19 € … per m²`;
/// 3. an amount labelled for screen readers (`aria-label="EUR 17.59"`),
///    outside a struck-out price: per m² when the next lines say `per m²`;
///    a line `per l 16,48` after it is the price per litre;
/// 4. a line that is only an amount, after the heading (`135,20`).
///
/// The first pack price is the price and the pack the unit; a price per
/// m² or litre is `unit_price`. Without a pack price, an item price (rule
/// 3 or 4) is the price; with neither, a price per m² is.
fn prices(page: &Page, fields: &mut Fields) {
    let amount = r"(\d{1,3}(?:\.\d{3})+,(?:\d{2}|-)|\d+,\d{2}|\d+,-)";
    let pack = Regex::new(&format!(
        r"(?i)^(?:komt overeen met|pakket ?prijs|pakprijs)\s*(?:€\s*)?{amount}\s*(?:€)?\s*\*?\s*(?:per (?:pak|pakket|doos|verpakking))?$"
    ))
    .expect("regex");
    let per = Regex::new(&format!(
        r"(?i)^prijs\s*[—–-]?\s*(?:€\s*)?{amount}\s*€?.*\bper (m²|m2|l|liter|stuk|rol|m)$"
    ))
    .expect("regex");
    let per_litre =
        Regex::new(&format!(r"(?i)^per (?:l|liter)\s*(?:€\s*)?{amount}$")).expect("regex");
    let bare = Regex::new(&format!(r"^(?:€\s*)?{amount}(?:\s*€)?$")).expect("regex");

    let mut pack_price = None;
    let mut unit_price: Option<(Decimal, &'static str, String)> = None;
    let mut item_price = None;
    for line in &page.lines {
        if pack_price.is_none()
            && let Some(caps) = pack.captures(line)
        {
            pack_price = Decimal::parse(&caps[1]).map(|d| (d, line.clone()));
        }
        if unit_price.is_none()
            && let Some(caps) = per.captures(line)
            && let Some(d) = Decimal::parse(&caps[1])
        {
            unit_price = Some((d, unit_of(&caps[2]), line.clone()));
        }
    }
    if let Some(labelled) = page.labelled.first()
        && let Some(d) = Decimal::parse(
            labelled
                .label
                .trim_start_matches("EUR")
                .trim_start_matches('€'),
        )
    {
        let after = &page.lines[labelled.line.min(page.lines.len())..];
        let after = &after[..after.len().min(4)];
        let per_m2 = after.iter().any(|l| {
            let l = l.to_lowercase();
            l == "per m²" || l == "per m2"
        });
        if per_m2 {
            if unit_price.is_none() {
                unit_price = Some((d, "m2", format!("aria-label {} per m²", labelled.label)));
            }
        } else {
            item_price = Some((d, format!("aria-label {}", labelled.label)));
            if unit_price.is_none()
                && let Some((line, caps)) = after
                    .iter()
                    .find_map(|l| per_litre.captures(l).map(|c| (l, c)))
                && let Some(per) = Decimal::parse(&caps[1])
            {
                unit_price = Some((per, "litre", line.clone()));
            }
        }
    }
    if item_price.is_none()
        && let Some((at, _)) = page.heading()
        && let Some(line) = page.lines[at + 1..].iter().find(|l| bare.is_match(l))
        && let Some(d) = Decimal::parse(line)
    {
        item_price = Some((d, line.clone()));
    }
    let shown = pack_price.is_some() || item_price.is_some() || unit_price.is_some();
    if let Some((d, from)) = pack_price {
        put(fields, "price", json!(d.to_string()), Evidence::Text, from);
        put(
            fields,
            "sale_unit",
            json!("pack"),
            Evidence::Text,
            "a price per pack",
        );
    } else if let Some((d, from)) = item_price {
        put(fields, "price", json!(d.to_string()), Evidence::Text, from);
    } else if let Some((d, unit, from)) = &unit_price {
        put(
            fields,
            "price",
            json!(d.to_string()),
            Evidence::Text,
            from.clone(),
        );
        put(
            fields,
            "sale_unit",
            json!(unit),
            Evidence::Text,
            format!("a price per {unit} only"),
        );
    }
    if let Some((d, unit, from)) = unit_price {
        put(
            fields,
            "unit_price",
            json!(d.to_string()),
            Evidence::Text,
            from.clone(),
        );
        put(fields, "unit_price_per", json!(unit), Evidence::Text, from);
    }
    if shown {
        put(
            fields,
            "currency",
            json!("EUR"),
            Evidence::Text,
            "€ / EUR beside the price",
        );
        let said = page.lines.iter().find(|l| {
            let l = l.to_lowercase();
            l.contains("incl. btw") || l.contains("inclusief btw")
        });
        let without =
            Regex::new(r"(?i)\bex(?:cl(?:usief|\.)?|\.)?\s*(?:\d+\s*%\s*)?btw").expect("regex");
        let excluded = page.lines.iter().find(|l| without.is_match(l));
        // A page that says nothing of VAT leaves it to the market's rule,
        // which the import applies ([`dutch_vat`]).
        match (said, excluded) {
            (Some(line), _) => put(
                fields,
                "vat_included",
                json!(true),
                Evidence::Text,
                line.clone(),
            ),
            (None, Some(line)) => put(
                fields,
                "vat_included",
                json!(false),
                Evidence::Text,
                line.clone(),
            ),
            (None, None) => {}
        }
    }
}

/// The Dutch market's rule where the shop says nothing of VAT: a consumer
/// price includes it. Only a product with a price takes it, and a field the
/// shop gave stays. `crate::import` applies it to a product of the Dutch
/// market alone.
pub fn dutch_vat(fields: &mut Fields) {
    if fields.contains_key("price") {
        put(
            fields,
            "vat_included",
            json!(true),
            Evidence::Market,
            "Dutch consumer prices include VAT",
        );
    }
}

/// The size of one tile or plank as a covering's heading writes it
/// (`Wandtegel Bianca - Wit - Keramiek - 30x60 cm - 1,44 m²`), when the
/// specification table gave none (what it gave stays): the first number
/// is the width.
fn heading_tile(heading: &str, fields: &mut Fields) {
    let size =
        Regex::new(r"(?i)\b(\d+(?:[.,]\d+)?)\s*x\s*(\d+(?:[.,]\d+)?)\s*cm\b").expect("regex");
    let Some(c) = size.captures(heading) else {
        return;
    };
    let (Some(w), Some(l)) = (Decimal::parse(&c[1]), Decimal::parse(&c[2])) else {
        return;
    };
    let from = format!("h1 {}", &c[0]);
    put(
        fields,
        "tile_width_cm",
        json!(w.to_f64()),
        Evidence::Text,
        from.clone(),
    );
    put(
        fields,
        "tile_length_cm",
        json!(l.to_f64()),
        Evidence::Text,
        from,
    );
}

/// What a finish is, by how the page says it is sold.
fn finish_type(fields: &Fields) -> Option<&'static str> {
    let per = fields.get("unit_price_per").and_then(|f| f.value.as_str());
    if fields.contains_key("spread_m2_per_litre")
        || fields.contains_key("pack_litres")
        || per == Some("litre")
    {
        Some("paint")
    } else if fields.contains_key("pack_m2") || per == Some("m2") {
        Some("covering")
    } else {
        None
    }
}

/// What `from` of a sale unit says when it is only the kind's default.
const DEFAULT_UNIT: &str = " priced as one item";

/// Gives the sale unit its kind's default again after the kind changed
/// under it: a default read for another kind goes, and the one of this
/// kind takes its place; a unit the page said stays.
pub fn settle_sale_unit(kind: Kind, fields: &mut Fields) {
    forget_default_unit(fields);
    sale_unit(kind, fields);
}

/// Drops a sale unit that is only a kind's default.
pub fn forget_default_unit(fields: &mut Fields) {
    if fields
        .get("sale_unit")
        .is_some_and(|f| f.evidence == Evidence::Text && f.from.ends_with(DEFAULT_UNIT))
    {
        fields.remove("sale_unit");
    }
}

/// A finish without a price per pack is sold by the can, box or roll it
/// comes in; furniture by the piece.
fn sale_unit(kind: Kind, fields: &mut Fields) {
    if !fields.contains_key("price") {
        return;
    }
    let unit = match kind {
        Kind::Furniture => "piece",
        Kind::Finish => "pack",
    };
    put(
        fields,
        "sale_unit",
        json!(unit),
        Evidence::Text,
        format!("a {}{DEFAULT_UNIT}", kind.as_str()),
    );
}

/// Width, depth and height of a piece: a size whose order the page names
/// (`101x107x35 (hxbxd)`, `H 83 x B 181 x D 39 cm`), or the three labels.
/// A size without its order (`82x204x67 cm`) is not read: which number is
/// the depth is a guess.
fn furniture_box(page: &Page, fields: &mut Fields) {
    for label in ["Product afmetingen (cm)", "Afmetingen (cm)", "Afmetingen"] {
        let Some((value, from)) = page.value(label) else {
            continue;
        };
        let unit = label.ends_with("(cm)").then_some("cm");
        if let Some([w, d, h]) = ordered_box(value, unit) {
            put(fields, "width_cm", json!(w), Evidence::Text, from.clone());
            put(fields, "depth_cm", json!(d), Evidence::Text, from.clone());
            put(fields, "height_cm", json!(h), Evidence::Text, from);
            return;
        }
    }
    for (label, name) in [
        ("Breedte", "width_cm"),
        ("Diepte", "depth_cm"),
        ("Hoogte", "height_cm"),
    ] {
        if let Some((value, from)) = page.value(label)
            && let Some(cm) = centimetres(value, None)
        {
            put(fields, name, json!(cm), Evidence::Text, from);
        }
    }
}

/// `[width, depth, height]` in cm from a size that names its order.
fn ordered_box(value: &str, unit: Option<&str>) -> Option<[f64; 3]> {
    let number = r"(\d+(?:[.,]\d+)?)";
    let trailing = Regex::new(&format!(
        r"(?i)^{number}\s*x\s*{number}\s*x\s*{number}\s*(cm|mm)?\s*\(([hbdl])x([hbdl])x([hbdl])\)$"
    ))
    .expect("regex");
    let leading = Regex::new(&format!(
        r"(?i)^([hbdl])\s*:?\s*{number}\s*x\s*([hbdl])\s*:?\s*{number}\s*x\s*([hbdl])\s*:?\s*{number}\s*(cm|mm)?$"
    ))
    .expect("regex");
    let (pairs, written): (Vec<(String, String)>, Option<String>) =
        if let Some(c) = trailing.captures(value.trim()) {
            (
                (0..3)
                    .map(|i| (c[5 + i].to_lowercase(), c[1 + i].to_owned()))
                    .collect(),
                c.get(4).map(|m| m.as_str().to_lowercase()),
            )
        } else {
            let c = leading.captures(value.trim())?;
            (
                (0..3)
                    .map(|i| (c[1 + 2 * i].to_lowercase(), c[2 + 2 * i].to_owned()))
                    .collect(),
                c.get(7).map(|m| m.as_str().to_lowercase()),
            )
        };
    let unit = written.as_deref().or(unit)?;
    let scale = if unit == "mm" { 0.1 } else { 1.0 };
    let get = |letter: &str| {
        pairs
            .iter()
            .find(|(l, _)| l == letter)
            .and_then(|(_, n)| Decimal::parse(n))
            .map(|d| d.to_f64() * scale)
    };
    Some([get("b")?, get("d")?, get("h")?])
}

/// `30 x 60` (cm said by the label) or `30 cm x 60 cm` → `[30, 60]`.
fn pair(value: &str, label_says_cm: bool) -> Option<[f64; 2]> {
    let re = Regex::new(r"(?i)^(\d+(?:[.,]\d+)?)\s*(cm)?\s*x\s*(\d+(?:[.,]\d+)?)\s*(cm)?$")
        .expect("regex");
    let c = re.captures(value.trim())?;
    if !label_says_cm && c.get(4).is_none() {
        return None;
    }
    Some([
        Decimal::parse(&c[1])?.to_f64(),
        Decimal::parse(&c[3])?.to_f64(),
    ])
}

/// A length in cm: `80 cm`, `135 mm`, `1,2 m`. Without a unit, `unit`.
fn centimetres(value: &str, unit: Option<&str>) -> Option<f64> {
    let (n, u) = measure(value).or_else(|| Some((Decimal::parse(value)?.to_f64(), unit?)))?;
    let cm = match u {
        "cm" => n,
        "mm" => n / 10.0,
        "m" => n * 100.0,
        _ => return None,
    };
    (cm > 0.0).then_some(cm)
}

/// A number and its unit, the unit one of `mm cm m m2 l piece kg`.
fn measure(value: &str) -> Option<(f64, &'static str)> {
    let re = Regex::new(r"(?i)^(\d+(?:[.,]\d+)*)\s*(mm|cm|m²|m2|m|liter|l|ml|stuks?|kg)$")
        .expect("regex");
    let c = re.captures(value.trim())?;
    let n = Decimal::parse(&c[1])?.to_f64();
    let unit = match c[2].to_lowercase().as_str() {
        "mm" => "mm",
        "cm" => "cm",
        "m²" | "m2" => "m2",
        "m" => "m",
        "liter" | "l" => "l",
        "ml" => return Some((n / 1000.0, "l")),
        "kg" => "kg",
        _ => "piece",
    };
    Some((n, unit))
}

fn unit_of(word: &str) -> &'static str {
    match word.to_lowercase().as_str() {
        "m²" | "m2" => "m2",
        "l" | "liter" => "litre",
        "rol" => "roll",
        "m" => "metre",
        _ => "piece",
    }
}

/// `238-235-225` → `#eeebe1`.
fn rgb_hex(code: &str) -> Option<String> {
    let parts: Vec<u8> = code
        .split(['-', ',', ' '])
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    (parts.len() == 3).then(|| format!("#{:02x}{:02x}{:02x}", parts[0], parts[1], parts[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each way a page says how a finish is sold is enough on its own.
    #[test]
    fn every_signal_of_a_finish_counts_alone() {
        let one = |name: &str, value: Value| {
            let mut fields = Fields::new();
            put(&mut fields, name, value, Evidence::Text, "");
            finish_type(&fields)
        };
        assert_eq!(one("spread_m2_per_litre", json!(10.0)), Some("paint"));
        assert_eq!(one("pack_litres", json!(2.5)), Some("paint"));
        assert_eq!(one("unit_price_per", json!("litre")), Some("paint"));
        assert_eq!(one("pack_m2", json!(1.62)), Some("covering"));
        assert_eq!(one("unit_price_per", json!("m2")), Some("covering"));
        assert_eq!(one("unit_price_per", json!("piece")), None);
        assert_eq!(one("width_cm", json!(80.0)), None);
    }

    /// A size in the heading is a tile's only on a covering: a paint's
    /// heading keeps its size to itself.
    #[test]
    fn only_a_covering_takes_its_tile_size_from_the_heading() {
        let tile = serde_json::json!({
            "cleaned_html": "<h1>Wandtegel Wit 30x60 cm</h1><dl><dt>Inhoud</dt><dd>1,44 m²</dd></dl>"
        });
        let (_, fields) = from_page(&tile, None);
        assert_eq!(
            (
                &fields["tile_width_cm"].value,
                &fields["tile_length_cm"].value
            ),
            (&json!(30.0), &json!(60.0))
        );
        let paint = serde_json::json!({
            "cleaned_html": "<h1>Muurverf proefvlak 30x60 cm</h1><dl><dt>Inhoud</dt><dd>2,5 l</dd></dl>"
        });
        let (kind, fields) = from_page(&paint, None);
        assert_eq!(kind, Kind::Finish);
        assert_eq!(fields["finish_type"].value, "paint");
        assert!(!fields.contains_key("tile_width_cm"));
        assert!(!fields.contains_key("tile_length_cm"));
    }

    /// The price is looked for after the heading: a mini-cart's total in
    /// the page's header is not the product's price.
    #[test]
    fn a_bare_amount_counts_only_after_the_heading() {
        let page = serde_json::json!({
            "cleaned_html": "<header><div>€ 0,00</div></header><h1>Kast Alaska</h1><div>Normale prijs 169,00</div><div>135,20</div>"
        });
        let (_, fields) = from_page(&page, None);
        assert_eq!(fields["price"].value, "135.20");
        assert_eq!(fields["price"].from, "135,20");
    }

    #[test]
    fn sizes_are_read_only_with_their_order() {
        assert_eq!(
            ordered_box("101x107x35 (hxbxd)", Some("cm")),
            Some([107.0, 35.0, 101.0])
        );
        assert_eq!(
            ordered_box("H 83 x B 181 x D 39 cm", None),
            Some([181.0, 39.0, 83.0])
        );
        assert_eq!(
            ordered_box("200x120x36 (hxbxd)", Some("cm")),
            Some([120.0, 36.0, 200.0])
        );
        assert_eq!(ordered_box("82x204x67 cm", None), None, "no order, no box");
        assert_eq!(ordered_box("190x85x45", Some("cm")), None);
        assert_eq!(
            ordered_box("101x107x35 (hxbxd)", None),
            None,
            "no unit, no box"
        );
        assert_eq!(
            ordered_box("1010x1070x350 mm (hxbxd)", None),
            Some([107.0, 35.0, 101.0])
        );
    }

    #[test]
    fn measures_carry_their_unit() {
        assert_eq!(measure("1,62 m²"), Some((1.62, "m2")));
        assert_eq!(measure("1.395 m²"), Some((1.395, "m2")));
        assert_eq!(measure("2.5 l"), Some((2.5, "l")));
        assert_eq!(measure("750 ml"), Some((0.75, "l")));
        assert_eq!(measure("9 Stuks"), Some((9.0, "piece")));
        assert_eq!(measure("10 m²"), Some((10.0, "m2")));
        assert_eq!(measure("ca. 10 m²"), None);
        assert_eq!(centimetres("135 mm", None), Some(13.5));
        assert_eq!(centimetres("80 cm", None), Some(80.0));
        assert_eq!(centimetres("1,2 m", None), Some(120.0));
        assert_eq!(centimetres("10 kg", None), None);
        assert_eq!(pair("30 x 60", true), Some([30.0, 60.0]));
        assert_eq!(pair("30 cm x 60 cm", false), Some([30.0, 60.0]));
        assert_eq!(pair("30 x 60", false), None);
        assert_eq!(rgb_hex("238-235-225").as_deref(), Some("#eeebe1"));
        assert_eq!(rgb_hex("238-235"), None);
    }
}
