//! The extractor on recorded `AnySite` responses of real Dutch shop pages
//! (`tests/fixtures/`, recorded 2026-10-21): what each page yields, every
//! field with its evidence and the words it came from, compared whole.

use newera_shop::extract::{from_ikea, from_page};
use newera_shop::product::{Kind, needed, render_worth_it};
use serde_json::Value;

fn recorded(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("JSON");
    record["body"][0].clone()
}

fn page(name: &str, kind: Option<Kind>) -> (Kind, Value) {
    let (kind, fields) = from_page(&recorded(name), kind);
    (kind, serde_json::to_value(fields).expect("fields"))
}

fn expected(literal: &str) -> Value {
    serde_json::from_str(literal).expect("expectation")
}

/// IKEA through its adapter: the box from `measurements`, the price for
/// everyone (the item's `price` is the IKEA Family price that week), the
/// colour with its hex.
#[test]
fn ikea_billy_from_the_adapter() {
    let fields = from_ikea(&recorded("ikea_billy_00263850.json"));
    assert_eq!(serde_json::to_value(&fields).unwrap(), expected(IKEA));
    assert_eq!(needed(Kind::Furniture, &fields), [] as [&str; 0]);
}

/// Praxis laminate: sold by the pack (`Pakket prijs 24,54`), 17.59 per m²
/// from the price's accessibility label, 1.395 m² per pack from
/// `Rendement per verpakking`, plank size from the table.
#[test]
fn praxis_laminate_is_a_covering_sold_by_the_pack() {
    let (kind, fields) = page("praxis_laminate_10043467.parse.json", None);
    assert_eq!(kind, Kind::Finish);
    assert_eq!(fields, expected(PRAXIS_LAMINATE));
}

/// Praxis paint: the can's price, 16.48 per litre, 2.5 l, 10 m² per litre,
/// the colour from its sRGB code. `Rendement per verpakking` (25 m²) is a
/// can's coverage, not a pack of floor, and the can's own size is not a
/// piece of furniture.
#[test]
fn praxis_paint_is_paint_by_the_can() {
    let (kind, fields) = page("praxis_paint_10049393.parse.json", None);
    assert_eq!(kind, Kind::Finish);
    assert_eq!(fields, expected(PRAXIS_PAINT));
}

/// Leen Bakker: the size string says its order (`101x107x35 (hxbxd)`), the
/// price is the line that is only an amount after the heading (the
/// `Normale prijs 169,00` above it is the old one).
#[test]
fn leen_bakker_cabinet_box_from_its_ordered_size() {
    let (kind, fields) = page("leenbakker_kast_alaska.parse.json", None);
    assert_eq!(kind, Kind::Furniture);
    assert_eq!(fields, expected(LEEN_BAKKER));
}

/// Hornbach behind its bot check: the parse is a challenge page with a
/// title and no heading, so nothing is read and the render is worth it;
/// the render has the pack price, the price per m² and the pack's m² and
/// tiles from the text.
#[test]
fn hornbach_tile_needs_the_render_and_reads_from_it() {
    let (kind, fields) = page("hornbach_tile_10656129.parse.json", None);
    assert_eq!((kind, fields), (Kind::Furniture, serde_json::json!({})));
    let (kind, parsed) = from_page(&recorded("hornbach_tile_10656129.parse.json"), None);
    assert!(render_worth_it(kind, &parsed));

    let (kind, fields) = page("hornbach_tile_10656129.render.json", None);
    assert_eq!(kind, Kind::Finish);
    assert_eq!(fields, expected(HORNBACH_RENDER));
}

/// Tegeldepot is behind Cloudflare for `AnySite` too: the parse is "Just a
/// moment..." and nothing is read from it.
#[test]
fn tegeldepot_challenge_yields_nothing() {
    let (kind, fields) = page("tegeldepot_search.parse.json", Some(Kind::Finish));
    assert_eq!((kind, fields), (Kind::Finish, serde_json::json!({})));
}

/// The agent's word for the kind wins over what the page suggests: a paint
/// page read as furniture yields the can's box and no finish type.
#[test]
fn the_agents_kind_is_kept() {
    let (kind, fields) = page("praxis_paint_10049393.parse.json", Some(Kind::Furniture));
    assert_eq!(kind, Kind::Furniture);
    assert_eq!(fields["width_cm"]["value"], 19.1);
    assert_eq!(fields["depth_cm"]["value"], 16.7);
    assert_eq!(fields["height_cm"]["value"], 15.7);
    assert!(fields.get("finish_type").is_none());
    assert_eq!(fields["sale_unit"]["value"], "piece");
}

/// What `ikea_billy_00263850.json` reads as, field by field.
const IKEA: &str = r##"{
 "article": {
  "value": "00263850",
  "evidence": "adapter",
  "from": "id"
 },
 "brand": {
  "value": "IKEA",
  "evidence": "adapter",
  "from": "ikea/products"
 },
 "colour": {
  "value": "wit",
  "evidence": "adapter",
  "from": "colors[0].name"
 },
 "colour_hex": {
  "value": "#ffffff",
  "evidence": "adapter",
  "from": "colors[0].hex"
 },
 "currency": {
  "value": "EUR",
  "evidence": "adapter",
  "from": "currency"
 },
 "depth_cm": {
  "value": 28.0,
  "evidence": "adapter",
  "from": "measurements Diepte 28 cm"
 },
 "height_cm": {
  "value": 202.0,
  "evidence": "adapter",
  "from": "measurements Hoogte 202 cm"
 },
 "image": {
  "value": "https://www.ikea.com/nl/nl/images/products/billy-boekenkast-wit__1590272_pe1038911_s5.jpg",
  "evidence": "adapter",
  "from": "images MAIN_PRODUCT_IMAGE"
 },
 "member_price": {
  "value": "42.49",
  "evidence": "adapter",
  "from": "price (IKEA Family)"
 },
 "price": {
  "value": "49.99",
  "evidence": "adapter",
  "from": "previous_price (price is the IKEA Family price)"
 },
 "product_type": {
  "value": "Boekenkast",
  "evidence": "adapter",
  "from": "type_name"
 },
 "sale_unit": {
  "value": "piece",
  "evidence": "adapter",
  "from": "unit PIECE"
 },
 "title": {
  "value": "BILLY Boekenkast, wit, 80x28x202 cm",
  "evidence": "adapter",
  "from": "name, type_name, variant_text, measure_text"
 },
 "width_cm": {
  "value": 80.0,
  "evidence": "adapter",
  "from": "measurements Breedte 80 cm"
 }
}"##;

/// What `praxis_laminate_10043467.parse.json` reads as, field by field.
const PRAXIS_LAMINATE: &str = r#"{
 "brand": {
  "value": "DecoMode - Praxis Eigen Merk",
  "evidence": "text",
  "from": "Merk DecoMode - Praxis Eigen Merk"
 },
 "colour": {
  "value": "Natuur eiken",
  "evidence": "text",
  "from": "Kleur Natuur eiken"
 },
 "currency": {
  "value": "EUR",
  "evidence": "text",
  "from": "€ / EUR beside the price"
 },
 "ean": {
  "value": "5414628081245",
  "evidence": "text",
  "from": "EAN 5414628081245"
 },
 "finish_type": {
  "value": "covering",
  "evidence": "text",
  "from": "m² per pack or a price per m² on the page"
 },
 "image": {
  "value": "https://d2fb1ew6v6wv87.cloudfront.net/products/10043467/s01/424x424/origin.webp",
  "evidence": "text",
  "from": "the first image described as the heading"
 },
 "pack_m2": {
  "value": 1.395,
  "evidence": "text",
  "from": "Rendement per verpakking 1.395 m²"
 },
 "price": {
  "value": "24.54",
  "evidence": "text",
  "from": "Pakket prijs 24,54"
 },
 "sale_unit": {
  "value": "pack",
  "evidence": "text",
  "from": "a price per pack"
 },
 "tile_length_cm": {
  "value": 129.2,
  "evidence": "text",
  "from": "Lengte plank 1292 mm"
 },
 "tile_width_cm": {
  "value": 13.5,
  "evidence": "text",
  "from": "Breedte plank 135 mm"
 },
 "title": {
  "value": "DecoMode laminaat - Medium - Dortmund - 4-zijdige V-groef- 8mm - 1,394m²",
  "evidence": "text",
  "from": "h1"
 },
 "unit_price": {
  "value": "17.59",
  "evidence": "text",
  "from": "aria-label EUR 17.59 per m²"
 },
 "unit_price_per": {
  "value": "m2",
  "evidence": "text",
  "from": "aria-label EUR 17.59 per m²"
 },
 "vat_included": {
  "value": true,
  "evidence": "text",
  "from": "Alle prijzen zijn incl. BTW en excl. eventuele verzendkosten"
 }
}"#;

/// What `praxis_paint_10049393.parse.json` reads as, field by field.
const PRAXIS_PAINT: &str = r##"{
 "brand": {
  "value": "Flexa",
  "evidence": "text",
  "from": "Merk Flexa"
 },
 "colour": {
  "value": "Gebroken wit / RAL 9010",
  "evidence": "text",
  "from": "Kleur Gebroken wit / RAL 9010"
 },
 "colour_hex": {
  "value": "#eeebe1",
  "evidence": "text",
  "from": "Kleurcode 238-235-225"
 },
 "currency": {
  "value": "EUR",
  "evidence": "text",
  "from": "€ / EUR beside the price"
 },
 "ean": {
  "value": "8711113175449",
  "evidence": "text",
  "from": "EAN 8711113175449"
 },
 "finish_type": {
  "value": "paint",
  "evidence": "text",
  "from": "litres or m² per litre on the page"
 },
 "image": {
  "value": "https://d2fb1ew6v6wv87.cloudfront.net/products/10049393/s00/424x424/origin.webp",
  "evidence": "text",
  "from": "the first image described as the heading"
 },
 "pack_litres": {
  "value": 2.5,
  "evidence": "text",
  "from": "Inhoud 2.5 l"
 },
 "price": {
  "value": "41.19",
  "evidence": "text",
  "from": "aria-label EUR 41.19"
 },
 "sale_unit": {
  "value": "pack",
  "evidence": "text",
  "from": "a finish priced as one item"
 },
 "spread_m2_per_litre": {
  "value": 10.0,
  "evidence": "text",
  "from": "Rendement per liter 10 m²"
 },
 "title": {
  "value": "Flexa Easycare Muurverf - Mat - Gebroken Wit / RAL 9010 - 2,5L",
  "evidence": "text",
  "from": "h1"
 },
 "unit_price": {
  "value": "16.48",
  "evidence": "text",
  "from": "per l 16,48"
 },
 "unit_price_per": {
  "value": "litre",
  "evidence": "text",
  "from": "per l 16,48"
 },
 "vat_included": {
  "value": true,
  "evidence": "text",
  "from": "Alle prijzen zijn incl. BTW en excl. eventuele verzendkosten"
 }
}"##;

/// What `leenbakker_kast_alaska.parse.json` reads as, field by field.
const LEEN_BAKKER: &str = r#"{
 "article": {
  "value": "11901825",
  "evidence": "text",
  "from": "Artikelnummer 11901825"
 },
 "colour": {
  "value": "Wit",
  "evidence": "text",
  "from": "Kleur Wit"
 },
 "currency": {
  "value": "EUR",
  "evidence": "text",
  "from": "€ / EUR beside the price"
 },
 "depth_cm": {
  "value": 35.0,
  "evidence": "text",
  "from": "Product afmetingen (cm) 101x107x35 (hxbxd)"
 },
 "ean": {
  "value": "8714901913216",
  "evidence": "text",
  "from": "EAN nummer 8714901913216"
 },
 "height_cm": {
  "value": 101.0,
  "evidence": "text",
  "from": "Product afmetingen (cm) 101x107x35 (hxbxd)"
 },
 "image": {
  "value": "https://static.leenbakker.nl/products/960x960/11901825.webp",
  "evidence": "text",
  "from": "the first image described as the heading"
 },
 "price": {
  "value": "135.20",
  "evidence": "text",
  "from": "135,20"
 },
 "sale_unit": {
  "value": "piece",
  "evidence": "text",
  "from": "a furniture priced as one item"
 },
 "title": {
  "value": "Kast Alaska - wit - 101x107x35 cm",
  "evidence": "text",
  "from": "h1"
 },
 "vat_included": {
  "value": true,
  "evidence": "text",
  "from": "Alle prijzen zijn incl. BTW"
 },
 "width_cm": {
  "value": 107.0,
  "evidence": "text",
  "from": "Product afmetingen (cm) 101x107x35 (hxbxd)"
 }
}"#;

/// What `hornbach_tile_10656129.render.json` reads as, field by field.
const HORNBACH_RENDER: &str = r#"{
 "article": {
  "value": "10656129",
  "evidence": "text",
  "from": "Art.nr. 10656129"
 },
 "currency": {
  "value": "EUR",
  "evidence": "text",
  "from": "€ / EUR beside the price"
 },
 "ean": {
  "value": "8697497116622",
  "evidence": "text",
  "from": "EAN 8697497116622"
 },
 "finish_type": {
  "value": "covering",
  "evidence": "text",
  "from": "m² per pack or a price per m² on the page"
 },
 "pack_m2": {
  "value": 1.62,
  "evidence": "text",
  "from": "Inhoud per pak voor 1,62 m²"
 },
 "pack_pieces": {
  "value": 9,
  "evidence": "text",
  "from": "Inhoud per pak 9 Stuks"
 },
 "price": {
  "value": "18.13",
  "evidence": "text",
  "from": "Komt overeen met 18,13 € per Pak"
 },
 "sale_unit": {
  "value": "pack",
  "evidence": "text",
  "from": "a price per pack"
 },
 "tile_length_cm": {
  "value": 60.0,
  "evidence": "text",
  "from": "Nominale afmeting in cm 30 x 60"
 },
 "tile_width_cm": {
  "value": 30.0,
  "evidence": "text",
  "from": "Nominale afmeting in cm 30 x 60"
 },
 "title": {
  "value": "Wandtegel Elegance wit mat 30 x 60 cm gerectificeerd",
  "evidence": "text",
  "from": "h1"
 },
 "unit_price": {
  "value": "11.19",
  "evidence": "text",
  "from": "prijs — 11,19 € * Alle prijzen zijn incl. BTW en excl. verzendkosten. per m²"
 },
 "unit_price_per": {
  "value": "m2",
  "evidence": "text",
  "from": "prijs — 11,19 € * Alle prijzen zijn incl. BTW en excl. verzendkosten. per m²"
 },
 "vat_included": {
  "value": true,
  "evidence": "text",
  "from": "prijs — 11,19 € * Alle prijzen zijn incl. BTW en excl. verzendkosten. per m²"
 }
}"#;
