//! What a product is made of: its kind, its fields, and where each field
//! came from.
//!
//! Every field carries its evidence — the adapter's field, the line of the
//! page it was read from, the market's rule, or the calling agent — so the
//! agent can check a value against the page and correct it. A field nobody
//! could read stays out, and [`needed`] says which of those the product's
//! kind cannot do without.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::decimal::Decimal;

/// What a product is for in a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A piece with a width, a depth and a height.
    Furniture,
    /// A floor, wall or ceiling finish: tile, laminate, paint, wallpaper.
    Finish,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "furniture" => Some(Self::Furniture),
            "finish" => Some(Self::Finish),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Furniture => "furniture",
            Self::Finish => "finish",
        }
    }
}

/// Where a field's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Evidence {
    /// A field of an `AnySite` shop adapter (`ikea/products`).
    Adapter,
    /// The text of the page.
    Text,
    /// A rule of the market, not the page: Dutch shop prices include VAT.
    Market,
    /// The calling agent, through `edit_products`.
    Agent,
}

/// One value and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub value: Value,
    pub evidence: Evidence,
    /// The adapter's field, or the words of the page, it was read from.
    pub from: String,
}

/// A product's fields by name.
pub type Fields = BTreeMap<String, Field>;

/// What a field holds, for checking a value the agent sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Text,
    /// An exact decimal, written as a string: money.
    Money,
    /// A number above zero: a measure.
    Measure,
    /// A whole number above zero: a count.
    Count,
    Flag,
    /// `#rrggbb`.
    Hex,
    Link,
    /// One of a few words.
    Choice(&'static [&'static str]),
}

/// The units a product is sold in.
pub const SALE_UNITS: &[&str] = &["piece", "pack", "m2", "litre", "roll", "metre"];
/// What a finish is, for how much of it a surface takes.
pub const FINISH_TYPES: &[&str] = &["covering", "paint", "wallpaper"];

/// Every field a product can carry, with its shape. Names are the
/// contract: the tools say them, and the estimate reads them.
pub const FIELDS: &[(&str, Shape)] = &[
    ("title", Shape::Text),
    ("brand", Shape::Text),
    ("article", Shape::Text),
    ("ean", Shape::Text),
    ("product_type", Shape::Text),
    ("image", Shape::Link),
    ("colour", Shape::Text),
    ("colour_hex", Shape::Hex),
    ("price", Shape::Money),
    ("member_price", Shape::Money),
    ("currency", Shape::Text),
    ("vat_included", Shape::Flag),
    ("sale_unit", Shape::Choice(SALE_UNITS)),
    ("unit_price", Shape::Money),
    ("unit_price_per", Shape::Choice(SALE_UNITS)),
    ("width_cm", Shape::Measure),
    ("depth_cm", Shape::Measure),
    ("height_cm", Shape::Measure),
    ("finish_type", Shape::Choice(FINISH_TYPES)),
    ("pack_m2", Shape::Measure),
    ("pack_litres", Shape::Measure),
    ("pack_pieces", Shape::Count),
    ("spread_m2_per_litre", Shape::Measure),
    ("roll_width_cm", Shape::Measure),
    ("roll_length_m", Shape::Measure),
    ("pattern_repeat_cm", Shape::Measure),
    ("tile_width_cm", Shape::Measure),
    ("tile_length_cm", Shape::Measure),
    ("swatch_image", Shape::Link),
];

/// The shape of a field, `None` for a name that is no field.
pub fn shape(name: &str) -> Option<Shape> {
    FIELDS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// A value the agent sets, checked against the field's shape and written
/// the way the fetched ones are. `Err` says what the field takes.
pub fn checked(name: &str, value: &Value) -> Result<Value, String> {
    let Some(shape) = shape(name) else {
        let names: Vec<&str> = FIELDS.iter().map(|(n, _)| *n).collect();
        return Err(format!(
            "no field {name}: the fields are {}",
            names.join(", ")
        ));
    };
    let text = value.as_str().map(str::trim);
    let number = value
        .as_f64()
        .or_else(|| text.and_then(Decimal::parse).map(Decimal::to_f64));
    let ok = match shape {
        Shape::Text => text.filter(|t| !t.is_empty()).map(Value::from),
        Shape::Money => match value {
            Value::Number(n) => Decimal::parse(&n.to_string()),
            Value::String(s) => Decimal::parse(s),
            _ => None,
        }
        .filter(|d| d.is_positive())
        .map(|d| Value::from(d.to_string())),
        Shape::Measure => number.filter(|n| *n > 0.0).map(Value::from),
        Shape::Count => value.as_u64().filter(|n| *n > 0).map(Value::from),
        Shape::Flag => value.as_bool().map(Value::from),
        Shape::Hex => text
            .map(str::to_lowercase)
            .filter(|t| {
                t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit())
            })
            .map(Value::from),
        Shape::Link => text
            .filter(|t| t.starts_with("https://") || t.starts_with("http://"))
            .map(Value::from),
        Shape::Choice(choices) => text.filter(|t| choices.contains(t)).map(Value::from),
    };
    ok.ok_or_else(|| {
        let takes = match shape {
            Shape::Text => "text".to_owned(),
            Shape::Money => "an amount above zero, like \"24.54\"".to_owned(),
            Shape::Measure => "a number above zero".to_owned(),
            Shape::Count => "a whole number above zero".to_owned(),
            Shape::Flag => "true or false".to_owned(),
            Shape::Hex => "a colour like #eeebe1".to_owned(),
            Shape::Link => "an http(s) link".to_owned(),
            Shape::Choice(choices) => format!("one of {}", choices.join(", ")),
        };
        format!("{name} takes {takes}, not {value}")
    })
}

/// The fields a product of this kind cannot do without, that it lacks:
/// what the agent is asked to fill. A finish needs what its type is sold
/// by — m² per pack for a covering, litres per can and m² per litre for
/// paint, the roll for wallpaper — so a finish of unknown type lacks its
/// type first.
pub fn needed(kind: Kind, fields: &Fields) -> Vec<&'static str> {
    let wanted: Vec<&'static str> = match kind {
        Kind::Furniture => vec!["title", "price", "width_cm", "depth_cm", "height_cm"],
        Kind::Finish => {
            let mut wanted = vec!["title", "price", "finish_type"];
            match fields.get("finish_type").and_then(|f| f.value.as_str()) {
                Some("covering") => wanted.push("pack_m2"),
                Some("paint") => wanted.extend(["pack_litres", "spread_m2_per_litre"]),
                Some("wallpaper") => wanted.extend(["roll_width_cm", "roll_length_m"]),
                _ => {}
            }
            wanted
        }
    };
    wanted
        .into_iter()
        .filter(|name| !fields.contains_key(*name))
        .collect()
}

/// Whether a page read by `webparser/parse` lacks what makes the product
/// usable, so `webparser/render` is worth its price: for furniture the
/// title, price, width, depth and height; for a finish the title, price and
/// pack content. A paint's spread rate is asked of the agent, not of a
/// second fetch.
pub fn render_worth_it(kind: Kind, fields: &Fields) -> bool {
    needed(kind, fields)
        .iter()
        .any(|name| *name != "spread_m2_per_litre")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn with(names: &[(&str, Value)]) -> Fields {
        names
            .iter()
            .map(|(n, v)| {
                (
                    (*n).to_owned(),
                    Field {
                        value: v.clone(),
                        evidence: Evidence::Text,
                        from: String::new(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn furniture_needs_its_box_and_price() {
        assert_eq!(
            needed(Kind::Furniture, &Fields::new()),
            ["title", "price", "width_cm", "depth_cm", "height_cm"]
        );
        let full = with(&[
            ("title", json!("t")),
            ("price", json!("1")),
            ("width_cm", json!(1)),
            ("depth_cm", json!(1)),
            ("height_cm", json!(1)),
        ]);
        assert_eq!(needed(Kind::Furniture, &full), [] as [&str; 0]);
        assert!(!render_worth_it(Kind::Furniture, &full));
        let mut no_depth = full.clone();
        no_depth.remove("depth_cm");
        assert_eq!(needed(Kind::Furniture, &no_depth), ["depth_cm"]);
        assert!(render_worth_it(Kind::Furniture, &no_depth));
    }

    #[test]
    fn a_finish_needs_what_its_type_is_sold_by() {
        let base = [("title", json!("t")), ("price", json!("1"))];
        let of = |kind: &str, extra: &[(&str, Value)]| {
            let mut all = base.to_vec();
            all.push(("finish_type", json!(kind)));
            all.extend_from_slice(extra);
            needed(Kind::Finish, &with(&all))
        };
        assert_eq!(of("covering", &[]), ["pack_m2"]);
        assert_eq!(of("paint", &[]), ["pack_litres", "spread_m2_per_litre"]);
        assert_eq!(of("wallpaper", &[]), ["roll_width_cm", "roll_length_m"]);
        assert_eq!(of("covering", &[("pack_m2", json!(1.6))]), [] as [&str; 0]);
        assert_eq!(
            needed(Kind::Finish, &with(&base)),
            ["finish_type"],
            "the type comes first"
        );
    }

    /// The spread rate alone is no reason to pay for a render; anything
    /// else missing is.
    #[test]
    fn a_render_is_for_what_a_page_shows() {
        let paint = with(&[
            ("title", json!("t")),
            ("price", json!("1")),
            ("finish_type", json!("paint")),
            ("pack_litres", json!(2.5)),
        ]);
        assert!(!render_worth_it(Kind::Finish, &paint));
        let mut no_litres = paint.clone();
        no_litres.remove("pack_litres");
        assert!(render_worth_it(Kind::Finish, &no_litres));
        assert!(render_worth_it(
            Kind::Finish,
            &with(&[("title", json!("t"))])
        ));
    }

    #[test]
    fn a_set_value_is_checked_and_written_like_a_fetched_one() {
        let cases = [
            ("price", json!("24,54"), Ok(json!("24.54"))),
            ("price", json!(24.5), Ok(json!("24.5"))),
            ("price", json!("0"), Err("price takes an amount above zero")),
            ("width_cm", json!(80), Ok(json!(80.0))),
            ("width_cm", json!("80,5"), Ok(json!(80.5))),
            (
                "width_cm",
                json!(-1),
                Err("width_cm takes a number above zero"),
            ),
            ("pack_pieces", json!(9), Ok(json!(9))),
            (
                "pack_pieces",
                json!(1.5),
                Err("pack_pieces takes a whole number"),
            ),
            ("sale_unit", json!("pack"), Ok(json!("pack"))),
            (
                "sale_unit",
                json!("box"),
                Err("sale_unit takes one of piece, pack"),
            ),
            ("colour_hex", json!("#EEEBE1"), Ok(json!("#eeebe1"))),
            (
                "colour_hex",
                json!("eeebe1"),
                Err("colour_hex takes a colour"),
            ),
            ("vat_included", json!(true), Ok(json!(true))),
            (
                "swatch_image",
                json!("https://x/a.jpg"),
                Ok(json!("https://x/a.jpg")),
            ),
            (
                "swatch_image",
                json!("file:///a"),
                Err("swatch_image takes an http(s) link"),
            ),
            ("title", json!("  "), Err("title takes text")),
            ("colour", json!("wit"), Ok(json!("wit"))),
            (
                "nope",
                json!(1),
                Err("no field nope: the fields are title, brand"),
            ),
        ];
        for (name, value, want) in cases {
            match (checked(name, &value), want) {
                (Ok(got), Ok(want)) => assert_eq!(got, want, "{name} {value}"),
                (Err(why), Err(start)) => assert!(why.starts_with(start), "{name}: {why}"),
                (got, want) => panic!("{name} {value}: {got:?}, wanted {want:?}"),
            }
        }
    }

    #[test]
    fn every_choice_is_listed_once() {
        for (n, (name, _)) in FIELDS.iter().enumerate() {
            assert!(
                !FIELDS[n + 1..].iter().any(|(other, _)| other == name),
                "{name} twice"
            );
        }
        assert_eq!(Kind::parse("finish"), Some(Kind::Finish));
        assert_eq!(Kind::parse("furniture"), Some(Kind::Furniture));
        assert_eq!(Kind::parse("tile"), None);
    }
}
