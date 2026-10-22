//! The arithmetic of an estimate, line by line, each line compared whole.

use serde_json::{Value, json};

use super::*;
use crate::product::{Evidence, Field};

fn product(id: &str, shop: &str, kind: Kind, fields: &[(&str, Value)]) -> Product {
    Product {
        id: id.to_owned(),
        shop: shop.to_owned(),
        url: format!("https://{shop}/{id}"),
        kind,
        fields: fields
            .iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    Field {
                        value: value.clone(),
                        evidence: Evidence::Agent,
                        from: String::new(),
                    },
                )
            })
            .collect(),
    }
}

fn x(text: &str) -> Exact {
    Exact::of(Decimal::parse(text).unwrap())
}

fn covered(
    surface: &str,
    room: &str,
    net: &str,
    coats: Option<u32>,
    waste: Option<&str>,
    spans: &[(&str, &str)],
) -> Covered {
    Covered {
        surface: surface.to_owned(),
        room: Some(room.to_owned()),
        net: x(net),
        coats,
        waste: waste.map(x),
        // The takeoff gives a floor or ceiling no stretches.
        spans: (!spans.is_empty()).then(|| spans.iter().map(|(w, h)| (x(w), x(h))).collect()),
    }
}

fn library(products: Vec<Product>) -> BTreeMap<String, Product> {
    products.into_iter().map(|p| (p.id.clone(), p)).collect()
}

/// The one line `surfaces` of `product` come to, as JSON.
fn finish(product: &Product, surfaces: Vec<Covered>) -> Value {
    let takeoff = Takeoff {
        finishes: vec![Covers {
            product: product.id.clone(),
            surfaces,
        }],
        ..Takeoff::default()
    };
    let lines = lines(&takeoff, &library(vec![product.clone()]));
    assert_eq!(lines.len(), 1);
    serde_json::to_value(&lines[0]).unwrap()
}

fn tile(fields: &[(&str, Value)]) -> Product {
    let mut all = vec![
        ("title", json!("Vloertegel Beige 60x60")),
        ("finish_type", json!("covering")),
        ("price", json!("24.54")),
        ("currency", json!("EUR")),
        ("vat_included", json!(true)),
        ("sale_unit", json!("pack")),
        ("pack_m2", json!(1.62)),
    ];
    for (name, value) in fields {
        all.retain(|(n, _)| n != name);
        if !value.is_null() {
            all.push((name, value.clone()));
        }
    }
    product("tile", "praxis.nl", Kind::Finish, &all)
}

/// The line every tile test starts from: `quantity` packs of 1.62 m² at
/// 24.54, for `need` m² on two walls of r1; the packs it buys said beside.
fn tile_line(need: &str, quantity: &str, total: Option<&str>) -> Value {
    json!({
        "kind": "covering",
        "product": "tile",
        "shop": "praxis.nl",
        "title": "Vloertegel Beige 60x60",
        "url": "https://praxis.nl/tile",
        "quantity": quantity,
        "unit": "pack",
        "packs": quantity,
        "need": [need, "m2"],
        "unit_price": total.map(|_| "24.54"),
        "currency": "EUR",
        "vat_included": true,
        "total": total,
        "rooms": ["r1"],
        "surfaces": ["w1:right", "w2:right"],
    })
}

/// Two walls of 1 m² and 2.24 m² are 3.24 m²: two packs of 1.62, where
/// rounding each wall up first would buy three.
#[test]
fn packs_are_counted_once_for_all_surfaces() {
    let walls = vec![
        covered("w1:right", "r1", "1", None, None, &[]),
        covered("w2:right", "r1", "2.24", None, None, &[]),
    ];
    assert_eq!(
        finish(&tile(&[]), walls),
        tile_line("3.24", "2", Some("49.08"))
    );
}

/// Waste is added to each surface's area before the sum.
#[test]
fn waste_comes_on_each_surface() {
    let walls = |waste: &str| {
        vec![
            covered("w1:right", "r1", "1", None, Some(waste), &[]),
            covered("w2:right", "r1", "2", None, None, &[]),
        ]
    };
    // 1 × 1.24 + 2 = 3.24: two packs, exactly.
    assert_eq!(
        finish(&tile(&[]), walls("24")),
        tile_line("3.24", "2", Some("49.08"))
    );
    // 1 × 1.25 + 2 = 3.25: a third pack.
    assert_eq!(
        finish(&tile(&[]), walls("25")),
        tile_line("3.25", "3", Some("73.62"))
    );
}

/// A price per m² buys whole packs all the same, and costs what they hold.
#[test]
fn a_price_per_m2_is_for_what_the_packs_hold() {
    let per_m2 = tile(&[
        ("sale_unit", json!("m2")),
        ("price", json!("11.19")),
        ("pack_m2", json!(1.395)),
    ]);
    let walls = vec![
        covered("w1:right", "r1", "1.5", None, None, &[]),
        covered("w2:right", "r1", "1", None, None, &[]),
    ];
    let mut want = tile_line("2.5", "2.79", Some("31.22"));
    want["unit"] = json!("m2");
    want["packs"] = json!("2");
    want["unit_price"] = json!("11.19");
    assert_eq!(finish(&per_m2, walls), want);
}

/// Without m² per pack, the line is the m² needed, rounded up to the
/// hundredth; priced when the price is per m², unpriced when per pack.
#[test]
fn without_pack_content_the_quantity_is_the_need() {
    let walls = || {
        vec![
            covered("w1:right", "r1", "1.001", None, None, &[]),
            covered("w2:right", "r1", "2", None, None, &[]),
        ]
    };
    let mut want = tile_line("3.01", "3.01", None);
    want["unit"] = json!("m2");
    want.as_object_mut().unwrap().remove("packs");
    want["notes"] = json!([
        "pack content unknown (pack_m2): the quantity is what the surfaces need",
        "sold per pack, which the quantity in m2 cannot be counted in",
    ]);
    assert_eq!(finish(&tile(&[("pack_m2", Value::Null)]), walls()), want);
    // A pack of nothing is a pack of unknown content.
    assert_eq!(finish(&tile(&[("pack_m2", json!(0))]), walls()), want);
    let per_m2 = tile(&[
        ("pack_m2", Value::Null),
        ("sale_unit", json!("m2")),
        ("price", json!("11.19")),
    ]);
    let mut want = tile_line("3.01", "3.01", Some("33.68"));
    want["unit"] = json!("m2");
    want.as_object_mut().unwrap().remove("packs");
    want["unit_price"] = json!("11.19");
    want["notes"] =
        json!(["pack content unknown (pack_m2): the quantity is what the surfaces need"]);
    assert_eq!(finish(&per_m2, walls()), want);
}

/// A tile sold by the piece, with its size known, is bought in tiles.
#[test]
fn a_tile_sold_by_the_piece_is_counted_in_tiles() {
    let tiles = tile(&[
        ("pack_m2", Value::Null),
        ("sale_unit", json!("piece")),
        ("price", json!("3.95")),
        ("tile_width_cm", json!(30)),
        ("tile_length_cm", json!(60.0)),
    ]);
    // 3.24 m² of 0.18 m² tiles: 18.
    let walls = vec![
        covered("w1:right", "r1", "1.62", None, None, &[]),
        covered("w2:right", "r1", "1.62", None, None, &[]),
    ];
    let mut want = tile_line("3.24", "18", Some("71.10"));
    want["unit"] = json!("piece");
    want["unit_price"] = json!("3.95");
    assert_eq!(finish(&tiles, walls.clone()), want);
    // Sold per pack of an unknown number of them: no tiles to count in.
    let mut per_pack = tiles.clone();
    per_pack.fields.get_mut("sale_unit").unwrap().value = json!("pack");
    let got = finish(&per_pack, walls);
    assert_eq!(
        (&got["quantity"], &got["unit"], &got["total"]),
        (&json!("3.24"), &json!("m2"), &Value::Null)
    );
}

/// Unknown price, sale unit or currency: unpriced, never zero.
#[test]
fn what_cannot_be_priced_is_unpriced() {
    let walls = || vec![covered("w1:right", "r1", "3.24", None, None, &[])];
    let unpriced = |note: &str| {
        let mut want = tile_line("3.24", "2", None);
        want["surfaces"] = json!(["w1:right"]);
        want["notes"] = json!([note]);
        want
    };
    assert_eq!(
        finish(&tile(&[("price", Value::Null)]), walls()),
        unpriced("no price: edit_products sets it")
    );
    assert_eq!(
        finish(&tile(&[("sale_unit", Value::Null)]), walls()),
        unpriced("sale unit unknown (sale_unit): the price cannot be matched to the quantity")
    );
    let mut want = unpriced("currency unknown: edit_products sets it");
    want["currency"] = Value::Null;
    assert_eq!(finish(&tile(&[("currency", Value::Null)]), walls()), want);
    let mut want = unpriced("sold per litre, which the quantity in pack cannot be counted in");
    want["notes"] = json!(["sold per litre, which the quantity in pack cannot be counted in"]);
    assert_eq!(
        finish(&tile(&[("sale_unit", json!("litre"))]), walls()),
        want
    );
}

fn paint(fields: &[(&str, Value)]) -> Product {
    let mut all = vec![
        ("title", json!("Muurverf Wit 2,5 L")),
        ("finish_type", json!("paint")),
        ("price", json!("39.99")),
        ("currency", json!("EUR")),
        ("sale_unit", json!("pack")),
        ("pack_litres", json!(2.5)),
        ("spread_m2_per_litre", json!(10)),
    ];
    for (name, value) in fields {
        all.retain(|(n, _)| n != name);
        if !value.is_null() {
            all.push((name, value.clone()));
        }
    }
    product("paint", "gamma.nl", Kind::Finish, &all)
}

/// 20 m² twice and 5 m² once, with 10 % waste on the first wall, at 10 m²
/// per litre: (44 + 5) ÷ 10 = 4.9 L, two cans of 2.5 L.
#[test]
fn paint_is_coats_over_the_spread_rate_in_whole_cans() {
    let walls = vec![
        covered("w1:right", "r1", "20", Some(2), Some("10"), &[]),
        covered("r1:ceiling", "r1", "5", None, None, &[]),
    ];
    assert_eq!(
        finish(&paint(&[]), walls),
        json!({
            "kind": "paint",
            "product": "paint",
            "shop": "gamma.nl",
            "title": "Muurverf Wit 2,5 L",
            "url": "https://gamma.nl/paint",
            "quantity": "2",
            "unit": "pack",
            "packs": "2",
            "need": ["4.9", "litre"],
            "unit_price": "39.99",
            "currency": "EUR",
            "total": "79.98",
            "rooms": ["r1"],
            "surfaces": ["w1:right", "r1:ceiling"],
            "notes": ["coats not set on some surfaces: one coat counted there (edit_finishes sets coats)"],
        })
    );
    // A third coat: 66 ÷ 10 = 6.6 L, three cans.
    let walls = vec![covered("w1:right", "r1", "20", Some(3), Some("10"), &[])];
    let got = finish(&paint(&[]), walls);
    assert_eq!(
        (&got["need"], &got["quantity"], &got["total"]),
        (&json!(["6.6", "litre"]), &json!("3"), &json!("119.97"))
    );
    // Sold by the litre: what the cans hold.
    let walls = vec![covered("w1:right", "r1", "20", Some(2), None, &[])];
    let got = finish(
        &paint(&[("sale_unit", json!("litre")), ("price", json!("16.48"))]),
        walls,
    );
    assert_eq!(
        (&got["quantity"], &got["unit"], &got["packs"], &got["total"]),
        (&json!("5"), &json!("litre"), &json!("2"), &json!("82.40"))
    );
}

/// Without a spread rate there are no litres: the line is the m² to paint,
/// coats included, and unpriced.
#[test]
fn paint_without_a_spread_rate_is_m2_to_paint() {
    let walls = vec![covered("w1:right", "r1", "20", Some(2), None, &[])];
    assert_eq!(
        finish(&paint(&[("spread_m2_per_litre", Value::Null)]), walls),
        json!({
            "kind": "paint",
            "product": "paint",
            "shop": "gamma.nl",
            "title": "Muurverf Wit 2,5 L",
            "url": "https://gamma.nl/paint",
            "quantity": "40",
            "unit": "m2",
            "need": ["40", "m2"],
            "unit_price": null,
            "currency": "EUR",
            "total": null,
            "rooms": ["r1"],
            "surfaces": ["w1:right"],
            "notes": [
                "spread rate unknown (spread_m2_per_litre): the quantity is m² to paint, coats included, not litres",
                "sold per pack, which the quantity in m2 cannot be counted in",
            ],
        })
    );
}

fn wallpaper(fields: &[(&str, Value)]) -> Product {
    let mut all = vec![
        ("title", json!("Vliesbehang Bloem")),
        ("finish_type", json!("wallpaper")),
        ("price", json!("29.95")),
        ("currency", json!("EUR")),
        ("sale_unit", json!("pack")),
        ("roll_width_cm", json!(53)),
        ("roll_length_m", json!(10.05)),
        ("pattern_repeat_cm", json!(64)),
    ];
    for (name, value) in fields {
        all.retain(|(n, _)| n != name);
        if !value.is_null() {
            all.push((name, value.clone()));
        }
    }
    product("paper", "praxis.nl", Kind::Finish, &all)
}

/// A wall 371 cm wide takes 7 drops of 53 cm, one 200 cm wide 4; a drop of
/// 250 cm and a 64 cm repeat is 314 cm, so a roll of 1 005 cm gives 3: 11
/// drops are ⌈11 ÷ 3⌉ = 4 rolls, where rolls per wall would be 3 + 2.
#[test]
fn wallpaper_is_drops_per_wall_in_rolls_for_all_walls() {
    let walls = || {
        vec![
            covered("w1:right", "r1", "9.275", None, None, &[("371", "250")]),
            covered("w2:right", "r1", "5", None, None, &[("200", "250")]),
        ]
    };
    let want = json!({
        "kind": "wallpaper",
        "product": "paper",
        "shop": "praxis.nl",
        "title": "Vliesbehang Bloem",
        "url": "https://praxis.nl/paper",
        "quantity": "4",
        "unit": "roll",
        "packs": "4",
        "need": ["3.67", "roll"],
        "unit_price": "29.95",
        "currency": "EUR",
        "total": "119.80",
        "rooms": ["r1"],
        "surfaces": ["w1:right", "w2:right"],
    });
    assert_eq!(finish(&wallpaper(&[]), walls()), want);
    // Without the repeat a drop is the wall's height and 10 cm of trim,
    // 260 cm: still 3 a roll.
    let mut plain = want.clone();
    plain["notes"] = json!([
        "pattern repeat unknown (pattern_repeat_cm): drops are the wall's height and the trim"
    ]);
    assert_eq!(
        finish(&wallpaper(&[("pattern_repeat_cm", Value::Null)]), walls()),
        plain
    );
    // 10 % on the first wall: 7/3 × 1.1 + 4/3 = 3.9, still 4 rolls; 30 %:
    // 4.37, 5 rolls.
    let wasted = |waste: &str| {
        let mut walls = walls();
        walls[0].waste = Some(x(waste));
        let got = finish(&wallpaper(&[]), walls);
        (got["need"].clone(), got["quantity"].clone())
    };
    assert_eq!(wasted("10"), (json!(["3.9", "roll"]), json!("4")));
    assert_eq!(wasted("30"), (json!(["4.37", "roll"]), json!("5")));
}

/// A 400 × 250 cm wall takes 8 drops of 53 cm; at 260 cm with the trim a
/// 10.05 m roll gives 3: three rolls.
#[test]
fn a_drop_is_cut_with_its_trim() {
    let got = finish(
        &wallpaper(&[("pattern_repeat_cm", Value::Null)]),
        vec![covered(
            "w1:right",
            "r1",
            "10",
            None,
            None,
            &[("400", "250")],
        )],
    );
    assert_eq!(
        (&got["need"], &got["quantity"], &got["total"]),
        (&json!(["2.67", "roll"]), &json!("3"), &json!("89.85"))
    );
}

/// A wall the takeoff gave no stretches for — an editor older than them —
/// is counted by area, and the line says why; a wall whose band covers
/// nothing takes no drops.
#[test]
fn a_wall_without_stretches_is_counted_by_area() {
    let got = finish(
        &wallpaper(&[]),
        vec![
            covered("w1:right", "r1", "5.3265", None, None, &[]),
            Covered {
                spans: Some(Vec::new()),
                ..covered("w2:right", "r1", "0", None, None, &[])
            },
        ],
    );
    assert_eq!(
        (&got["quantity"], &got["notes"]),
        (
            &json!("1"),
            &json!([
                "counted by area, not drops: w1:right (the takeoff gave no face widths: an editor older than this service)"
            ])
        )
    );
}

/// Without the repeat, walls whose bands cover nothing — no stretches, or
/// one above the wall's top — take no drops, and nothing is said of a
/// repeat no drop was cut by.
#[test]
fn a_band_that_covers_nothing_says_nothing_of_the_repeat() {
    let got = finish(
        &wallpaper(&[("pattern_repeat_cm", Value::Null)]),
        vec![
            Covered {
                spans: Some(Vec::new()),
                ..covered("w1:right", "r1", "0", None, None, &[])
            },
            covered("w2:right", "r1", "0", None, None, &[("300", "0")]),
        ],
    );
    assert_eq!(
        (&got["need"], &got["quantity"], &got["notes"]),
        (&json!(["0", "roll"]), &json!("0"), &Value::Null)
    );
}

/// A stretch is drops of its own; a ceiling has no drops and is counted by
/// the roll's area; a drop longer than the roll is too.
#[test]
fn wallpaper_without_drops_is_counted_by_area() {
    // 2 drops on each of two 100 cm stretches: 4 of 3 a roll; the ceiling's
    // 9.6845 m² and 10 % is 10.65295 m², just under two 0.53 × 10.05 m
    // rolls.
    let surfaces = vec![
        covered(
            "w1:right",
            "r1",
            "5",
            None,
            None,
            &[("100", "250"), ("100", "250")],
        ),
        covered("r1:ceiling", "r1", "9.6845", None, Some("10"), &[]),
        covered("w2:right", "r1", "5.3", None, None, &[("53", "1000")]),
    ];
    let mut surfaces = surfaces;
    // A band above the wall's top: a stretch with nothing to cover.
    surfaces.push(covered("w3:right", "r1", "0", None, None, &[("300", "0")]));
    let got = finish(&wallpaper(&[]), surfaces);
    // 4/3 + 1.99999… + 5.3 ÷ 5.3265 = 4.328…
    assert_eq!(
        (&got["need"], &got["quantity"], &got["notes"]),
        (
            &json!(["4.33", "roll"]),
            &json!("5"),
            &json!([
                "counted by area, not drops: r1:ceiling (a floor or ceiling), w2:right (a drop longer than the roll)"
            ])
        )
    );
    // All by area, the repeat is nothing to mention.
    let got = finish(
        &wallpaper(&[("pattern_repeat_cm", Value::Null)]),
        vec![covered("r1:ceiling", "r1", "5.3265", None, None, &[])],
    );
    assert_eq!(
        (&got["quantity"], &got["notes"]),
        (
            &json!("1"),
            &json!(["counted by area, not drops: r1:ceiling (a floor or ceiling)"])
        )
    );
    // No roll size: m² by area, unpriced per roll.
    let got = finish(
        &wallpaper(&[("roll_width_cm", Value::Null)]),
        vec![covered(
            "w1:right",
            "r1",
            "5",
            None,
            Some("10"),
            &[("100", "250")],
        )],
    );
    assert_eq!(
        (&got["quantity"], &got["unit"], &got["total"], &got["notes"]),
        (
            &json!("5.5"),
            &json!("m2"),
            &Value::Null,
            &json!([
                "roll size unknown (roll_width_cm, roll_length_m): measured by area",
                "sold per pack, which the quantity in m2 cannot be counted in",
            ])
        )
    );
}

fn furniture_line(product: &Product, ids: &[&str]) -> Value {
    let takeoff = Takeoff {
        furniture: vec![Bought {
            product: product.id.clone(),
            ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            rooms: vec!["r1".to_owned()],
        }],
        ..Takeoff::default()
    };
    serde_json::to_value(&lines(&takeoff, &library(vec![product.clone()]))[0]).unwrap()
}

/// Five chairs sold in fours are two packs at the pack's price; one sold
/// alone is a piece each.
#[test]
fn furniture_is_pieces_or_whole_multipacks() {
    let chairs = product(
        "chairs",
        "ikea.com",
        Kind::Furniture,
        &[
            ("title", json!("TEODORES stoel, 4 st")),
            ("price", json!("79.96")),
            ("currency", json!("EUR")),
            ("vat_included", json!(true)),
            ("sale_unit", json!("piece")),
            ("pack_pieces", json!(4)),
        ],
    );
    assert_eq!(
        furniture_line(&chairs, &["f1", "f2", "f3", "f4", "f5"]),
        json!({
            "kind": "furniture",
            "product": "chairs",
            "shop": "ikea.com",
            "title": "TEODORES stoel, 4 st",
            "url": "https://ikea.com/chairs",
            "quantity": "2",
            "unit": "pack",
            "packs": "2",
            "need": ["5", "piece"],
            "unit_price": "79.96",
            "currency": "EUR",
            "vat_included": true,
            "total": "159.92",
            "rooms": ["r1"],
            "pieces": ["f1", "f2", "f3", "f4", "f5"],
            "notes": ["sold in packs of 4 pieces"],
        })
    );
    let mut single = chairs.clone();
    single.fields.remove("pack_pieces");
    let got = furniture_line(&single, &["f1", "f2"]);
    assert_eq!(
        (&got["quantity"], &got["unit"], &got["total"], &got["notes"]),
        (&json!("2"), &json!("piece"), &json!("159.92"), &Value::Null)
    );
    single.fields.remove("price");
    let got = furniture_line(&single, &["f1"]);
    assert_eq!(
        (&got["total"], &got["notes"]),
        (&Value::Null, &json!(["no price: edit_products sets it"]))
    );
}

/// A product the document names that the library does not have is a line
/// all the same, unpriced, saying so.
#[test]
fn a_product_not_in_the_library_is_an_unpriced_line() {
    let takeoff = Takeoff {
        finishes: vec![Covers {
            product: "gone".to_owned(),
            surfaces: vec![covered("r1:floor", "r1", "12.5", None, Some("5"), &[])],
        }],
        furniture: vec![Bought {
            product: "sofa".to_owned(),
            ids: vec!["f9".to_owned()],
            rooms: Vec::new(),
        }],
        gone: Vec::new(),
    };
    let got: Vec<Value> = lines(&takeoff, &BTreeMap::new())
        .iter()
        .map(|l| serde_json::to_value(l).unwrap())
        .collect();
    assert_eq!(
        got,
        vec![
            json!({
                "kind": "unknown", "product": "gone", "shop": null, "title": null,
                "quantity": "13.13", "unit": "m2", "need": ["13.13", "m2"],
                "unit_price": null, "currency": null, "total": null,
                "rooms": ["r1"], "surfaces": ["r1:floor"],
                "notes": ["product gone is not in this account's library (products lists it)"],
            }),
            json!({
                "kind": "unknown", "product": "sofa", "shop": null, "title": null,
                "quantity": "1", "unit": "piece",
                "unit_price": null, "currency": null, "total": null,
                "pieces": ["f9"],
                "notes": ["product sofa is not in this account's library (products lists it)"],
            }),
        ]
    );
}

fn extra(id: &str, shop: Option<&str>, quantity: &str, price: &str, currency: &str) -> Line {
    extra_line(&Extra {
        id: id.to_owned(),
        title: format!("{id} work, by the hour"),
        shop: shop.map(str::to_owned),
        quantity: Decimal::parse(quantity).unwrap(),
        unit: "hour".to_owned(),
        unit_price: Decimal::parse(price).unwrap(),
        currency: currency.to_owned(),
        vat_included: None,
        note: None,
    })
}

#[test]
fn an_extra_line_is_its_quantity_times_its_price() {
    assert_eq!(
        serde_json::to_value(extra("x1", None, "2.5", "45.333", "EUR")).unwrap(),
        json!({
            "kind": "extra", "line": "x1", "shop": null, "title": "x1 work, by the hour",
            "quantity": "2.5", "unit": "hour", "unit_price": "45.333",
            "currency": "EUR", "total": "113.33",
        })
    );
}

/// Totals per shop and currency, and per currency, never converted; one
/// unpriced line makes its shop's totals and every currency total
/// incomplete.
#[test]
fn totals_are_per_shop_and_currency_and_say_when_incomplete() {
    let tile = tile(&[]);
    let unpriced = paint(&[("price", Value::Null)]);
    let takeoff = Takeoff {
        finishes: vec![
            Covers {
                product: "tile".to_owned(),
                surfaces: vec![covered("w1:right", "r1", "3.24", None, None, &[])],
            },
            Covers {
                product: "paint".to_owned(),
                surfaces: vec![covered("w2:right", "r2", "10", Some(1), None, &[])],
            },
        ],
        ..Takeoff::default()
    };
    let mut all = lines(&takeoff, &library(vec![tile, unpriced]));
    let mut x1 = extra("x1", Some("praxis.nl"), "1", "0.92", "EUR");
    x1.vat_included = Some(true);
    all.push(x1);
    all.push(extra("x2", None, "2", "100", "GBP"));
    let got = estimate(&all, GroupBy::Shop);
    assert_eq!(
        got["totals"],
        json!([
            {"shop": null, "currency": "GBP", "vat": "unknown", "total": "200.00", "complete": true},
            {"shop": "gamma.nl", "currency": null, "vat": null, "total": null, "complete": false},
            {"shop": "praxis.nl", "currency": "EUR", "vat": "included", "total": "50.00", "complete": true},
        ])
    );
    assert_eq!(
        got["by_currency"],
        json!([
            {"currency": null, "vat": null, "total": null, "complete": false},
            {"currency": "EUR", "vat": "included", "total": "50.00", "complete": false},
            {"currency": "GBP", "vat": "unknown", "total": "200.00", "complete": false},
        ])
    );
    assert_eq!(got["complete"], false);
    let groups: Vec<(&str, usize)> = got["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["name"].as_str().unwrap(),
                g["lines"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        vec![("praxis.nl", 2), ("gamma.nl", 1), ("no shop", 1)]
    );
    assert_eq!(
        got["groups"][0]["totals"],
        json!([{"currency": "EUR", "vat": "included", "total": "50.00", "complete": true}])
    );
    let complete = estimate(&all[..1], GroupBy::Shop);
    assert_eq!(complete["complete"], true);
    assert_eq!(
        complete["by_currency"],
        json!([{"currency": "EUR", "vat": "included", "total": "49.08", "complete": true}])
    );
}

/// Prices with VAT, without it and not known either way are summed apart.
#[test]
fn totals_never_add_prices_with_and_without_vat() {
    let mut excluded = extra("x1", Some("praxis.nl"), "1", "100", "EUR");
    excluded.vat_included = Some(false);
    let all = vec![
        lines(
            &Takeoff {
                finishes: vec![Covers {
                    product: "tile".to_owned(),
                    surfaces: vec![covered("w1:right", "r1", "3.24", None, None, &[])],
                }],
                ..Takeoff::default()
            },
            &library(vec![tile(&[])]),
        )[0]
        .clone(),
        excluded,
        extra("x2", Some("praxis.nl"), "1", "10", "EUR"),
    ];
    let got = estimate(&all, GroupBy::Shop);
    assert_eq!(
        got["by_currency"],
        json!([
            {"currency": "EUR", "vat": "excluded", "total": "100.00", "complete": true},
            {"currency": "EUR", "vat": "included", "total": "49.08", "complete": true},
            {"currency": "EUR", "vat": "unknown", "total": "10.00", "complete": true},
        ])
    );
    assert_eq!(got["totals"].as_array().map(Vec::len), Some(3));
    assert!(csv(&got).ends_with(
        "total,,,,,,,,,EUR,excluded,100.00,,,\r\n\
         total,,,,,,,,,EUR,included,49.08,,,\r\n\
         total,,,,,,,,,EUR,unknown,10.00,,,\r\n"
    ));
}

/// A cell a spreadsheet would run as a formula is written as text.
#[test]
fn the_csv_writes_formulas_as_text() {
    let mut line = extra("x1", Some("+cmd"), "1", "10", "EUR");
    line.title = Some("=HYPERLINK(\"http://evil\",\"x\")".to_owned());
    line.notes = vec!["@SUM(1+1)".to_owned()];
    let mut other = extra("x2", Some("-x"), "1", "1", "EUR");
    other.title = Some("\tTab".to_owned());
    let sheet = csv(&estimate(&[line, other], GroupBy::Kind));
    let rows: Vec<&str> = sheet.split("\r\n").collect();
    assert_eq!(
        rows[1],
        "extra,extra,'+cmd,\"'=HYPERLINK(\"\"http://evil\"\",\"\"x\"\")\",,1,hour,,10,EUR,unknown,10.00,,,'@SUM(1+1)"
    );
    assert_eq!(
        rows[2],
        "extra,extra,'-x,'\tTab,,1,hour,,1,EUR,unknown,1.00,,,"
    );
}

/// A product is bought once for all its rooms: by room, a line of one room
/// is in that room, one of several in a group of its own.
#[test]
fn by_room_a_line_of_several_rooms_stays_whole() {
    let tile = tile(&[]);
    let takeoff = Takeoff {
        finishes: vec![Covers {
            product: "tile".to_owned(),
            surfaces: vec![
                covered("w1:right", "r1", "1", None, None, &[]),
                covered("w2:left", "r2", "1", None, None, &[]),
            ],
        }],
        ..Takeoff::default()
    };
    let mut all = lines(&takeoff, &library(vec![tile]));
    all.push(extra("x1", None, "1", "10", "EUR"));
    let names = |all: &[Line], by| -> Vec<String> {
        estimate(all, by)["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(&all, GroupBy::Room), ["several rooms", "no room"]);
    assert_eq!(names(&all, GroupBy::Kind), ["covering", "extra"]);
    let one = lines(
        &Takeoff {
            finishes: vec![Covers {
                product: "tile".to_owned(),
                surfaces: vec![covered("w1:right", "r1", "1", None, None, &[])],
            }],
            ..Takeoff::default()
        },
        &library(vec![self::tile(&[])]),
    );
    assert_eq!(names(&one, GroupBy::Room), ["r1"]);
}

#[test]
fn the_csv_has_a_row_per_line_and_the_totals() {
    let mut all = lines(
        &Takeoff {
            finishes: vec![Covers {
                product: "tile".to_owned(),
                surfaces: vec![covered("w1:right", "r1", "3.24", None, None, &[])],
            }],
            ..Takeoff::default()
        },
        &library(vec![tile(&[("title", json!("Tegel \"Beige\", 60x60"))])]),
    );
    all.push(extra("x1", None, "1", "10", "EUR"));
    all.push(
        lines(
            &Takeoff {
                furniture: vec![Bought {
                    product: "sofa".to_owned(),
                    ids: vec!["f9".to_owned()],
                    rooms: Vec::new(),
                }],
                ..Takeoff::default()
            },
            &BTreeMap::new(),
        )[0]
        .clone(),
    );
    assert_eq!(
        csv(&estimate(&all, GroupBy::Shop)),
        "group,kind,shop,title,url,quantity,unit,packs,unit_price,currency,vat,total,rooms,surfaces,notes\r\n\
         praxis.nl,covering,praxis.nl,\"Tegel \"\"Beige\"\", 60x60\",https://praxis.nl/tile,2,pack,2,24.54,EUR,included,49.08,r1,w1:right,\r\n\
         no shop,extra,,\"x1 work, by the hour\",,1,hour,,10,EUR,unknown,10.00,,,\r\n\
         no shop,unknown,,sofa,,1,piece,,,,unknown,unpriced,,,product sofa is not in this account's library (products lists it)\r\n\
         total,,,,,,,,,,,unpriced,,,\r\n\
         total,,,,,,,,,EUR,included,49.08 (incomplete),,,\r\n\
         total,,,,,,,,,EUR,unknown,10.00 (incomplete),,,\r\n"
    );
}

#[test]
fn a_takeoff_answer_is_read_as_it_is_written() {
    let answer = json!({
        "finishes": [{
            "product": "p1", "net_m2": 4.3, "gross_m2": 5.2, "openings_m2": 0.9,
            "surfaces": [
                {"surface": "w1:right", "room": "r2", "band": [120.0, 250.0], "coats": 2,
                 "gross_m2": 5.2, "openings_m2": 0.9, "net_m2": 4.3, "openings": ["f3"],
                 "spans_cm": [[392.5, 130.0]]},
                {"surface": "r2:floor", "waste": 7.5, "gross_m2": 12.0, "openings_m2": 0.0, "net_m2": 12.0},
            ],
        }],
        "furniture": [{"product": "p2", "pieces": 2, "ids": ["f4", "f5"], "rooms": ["r2"]}],
        "gone": [{"surface": "w9:left", "product": "p1"}],
    });
    assert_eq!(
        read_takeoff(&answer),
        Ok(Takeoff {
            finishes: vec![Covers {
                product: "p1".to_owned(),
                surfaces: vec![
                    Covered {
                        surface: "w1:right".to_owned(),
                        room: Some("r2".to_owned()),
                        net: x("4.3"),
                        coats: Some(2),
                        waste: None,
                        spans: Some(vec![(x("392.5"), x("130"))]),
                    },
                    Covered {
                        surface: "r2:floor".to_owned(),
                        room: None,
                        net: x("12"),
                        coats: None,
                        waste: Some(x("7.5")),
                        spans: None,
                    },
                ],
            }],
            furniture: vec![Bought {
                product: "p2".to_owned(),
                ids: vec!["f4".to_owned(), "f5".to_owned()],
                rooms: vec!["r2".to_owned()],
            }],
            gone: vec![json!({"surface": "w9:left", "product": "p1"})],
        })
    );
    assert_eq!(
        read_takeoff(&json!({"furniture": []})),
        Err("the takeoff has no finishes".to_owned())
    );
    assert_eq!(
        read_takeoff(
            &json!({"finishes": [{"product": "p", "surfaces": [{"net_m2": "x"}]}], "furniture": []})
        ),
        Err("a takeoff surface has no net_m2".to_owned())
    );
}

/// The sheet's `packs` column holds the whole packs (cans, rolls, tiles)
/// every line that counted them buys — also where the quantity already is
/// those packs, which the line's JSON says once: 8 packs of 1.44 m², 62
/// tiles of 30×60 cm, 2 multipacks of chairs. A piece of furniture sold alone, a line by m² with
/// no pack and a line added by hand count no packs.
#[test]
fn the_csv_says_the_packs_of_every_line_that_counted_them() {
    let wall = || vec![covered("w1:right", "r1", "11", None, None, &[])];
    let by_pack = tile(&[("pack_m2", json!(1.44)), ("price", json!("20.15"))]);
    let mut per_m2 = tile(&[
        ("sale_unit", json!("m2")),
        ("price", json!("13.99")),
        ("pack_m2", json!(1.44)),
    ]);
    per_m2.id = "tile2".to_owned();
    let mut no_pack = tile(&[("pack_m2", Value::Null), ("sale_unit", json!("m2"))]);
    no_pack.id = "tile3".to_owned();
    let mut by_tile = tile(&[
        ("pack_m2", Value::Null),
        ("sale_unit", json!("piece")),
        ("price", json!("2.50")),
        ("tile_width_cm", json!(30)),
        ("tile_length_cm", json!(60)),
    ]);
    by_tile.id = "tile4".to_owned();
    let can = paint(&[]);
    // 11 m² by area, 0.53 × 10.05 m a roll: ⌈2.07⌉ = 3 rolls.
    let paper = wallpaper(&[]);
    let chairs = product(
        "chairs",
        "ikea.com",
        Kind::Furniture,
        &[
            ("title", json!("TEODORES stoel, 4 st")),
            ("price", json!("79.96")),
            ("currency", json!("EUR")),
            ("sale_unit", json!("piece")),
            ("pack_pieces", json!(4)),
        ],
    );
    let mut sofa = chairs.clone();
    sofa.id = "sofa".to_owned();
    sofa.fields.remove("pack_pieces");
    let takeoff = Takeoff {
        finishes: [&by_pack, &per_m2, &no_pack, &by_tile, &can, &paper]
            .iter()
            .map(|p| Covers {
                product: p.id.clone(),
                surfaces: wall(),
            })
            .collect(),
        furniture: vec![
            Bought {
                product: "chairs".to_owned(),
                ids: (1..=5).map(|i| format!("f{i}")).collect(),
                rooms: Vec::new(),
            },
            Bought {
                product: "sofa".to_owned(),
                ids: vec!["f9".to_owned()],
                rooms: Vec::new(),
            },
        ],
        ..Takeoff::default()
    };
    let mut all = lines(
        &takeoff,
        &library(vec![
            by_pack, per_m2, no_pack, by_tile, can, paper, chairs, sofa,
        ]),
    );
    all.push(extra("x1", None, "3", "10", "EUR"));
    let sheet = csv(&estimate(&all, GroupBy::Kind));
    let rows: Vec<(String, String, String, String)> = sheet
        .lines()
        .skip(1)
        .filter(|l| !l.starts_with("total,"))
        .map(|l| {
            // Cells split at the commas outside quotes.
            let mut cells = vec![String::new()];
            let mut quoted = false;
            for c in l.chars() {
                match c {
                    '"' => quoted = !quoted,
                    ',' if !quoted => cells.push(String::new()),
                    c => cells.last_mut().unwrap().push(c),
                }
            }
            let at = |name: &str| cells[COLUMNS.iter().position(|c| *c == name).unwrap()].clone();
            (at("kind"), at("quantity"), at("unit"), at("packs"))
        })
        .collect();
    let row = |kind: &str, q: &str, unit: &str, packs: &str| {
        (
            kind.to_owned(),
            q.to_owned(),
            unit.to_owned(),
            packs.to_owned(),
        )
    };
    assert_eq!(
        rows,
        [
            row("covering", "8", "pack", "8"),
            row("covering", "11.52", "m2", "8"),
            row("covering", "11", "m2", ""),
            row("covering", "62", "piece", "62"),
            row("paint", "1", "pack", "1"),
            row("wallpaper", "3", "roll", "3"),
            row("furniture", "2", "pack", "2"),
            row("furniture", "1", "piece", ""),
            row("extra", "3", "hour", ""),
        ]
    );
}
