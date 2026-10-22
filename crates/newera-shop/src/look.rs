//! What a finish looks like on a surface, read from its fields.
//!
//! Only two things make a finish's look: a swatch picture whose physical
//! size is known, or the shop's colour. A picture of the pack or of a room
//! is no tile, so the product's `image` is never used; `swatch_image` is the
//! picture of one tile or plank (`tile_width_cm` × `tile_length_cm`) or of
//! one pattern repeat of a wallpaper across its roll (`roll_width_cm` ×
//! `pattern_repeat_cm`). Paint is its colour (`colour_hex`). The size a
//! picture is drawn at is the sample's, never the pack's coverage.

use crate::product::Fields;

/// A finish's look.
#[derive(Debug, Clone, PartialEq)]
pub enum Look {
    /// The swatch at `url`, which shows `size` cm: `[tile width, tile
    /// length]` for a tile or plank, whichever way the picture lies;
    /// `[across, up]` — the roll's width and the pattern's repeat — for a
    /// wallpaper, which hangs one way only.
    Picture {
        url: String,
        size: [f64; 2],
        hangs: bool,
    },
    /// `#rrggbb` as the shop gives it.
    Colour([u8; 3]),
}

/// The look a finish can have now, and the fields that would give it a
/// better one: for a tile or wallpaper drawn in its colour, or not at all,
/// what its picture needs; for a paint without colour, its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub look: Option<Look>,
    pub to_set: Vec<&'static str>,
    /// The shop's colour, whatever the look: what a picture that cannot be
    /// used gives way to.
    pub colour: Option<[u8; 3]>,
}

fn measure(fields: &Fields, name: &str) -> Option<f64> {
    fields
        .get(name)
        .and_then(|f| f.value.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn text<'a>(fields: &'a Fields, name: &str) -> Option<&'a str> {
    fields
        .get(name)
        .and_then(|f| f.value.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// `#rrggbb` as three bytes.
#[must_use]
pub fn hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The look of a finish product with `fields`.
#[must_use]
pub fn appearance(fields: &Fields) -> Appearance {
    let rgb = text(fields, "colour_hex").and_then(hex);
    let colour = rgb.map(Look::Colour);
    let picture = |sizes: [&'static str; 2], hangs: bool| {
        let wanted = ["swatch_image", sizes[0], sizes[1]];
        let to_set: Vec<&'static str> = wanted
            .into_iter()
            .filter(|name| !fields.contains_key(*name))
            .collect();
        match (
            text(fields, "swatch_image"),
            measure(fields, sizes[0]),
            measure(fields, sizes[1]),
        ) {
            (Some(url), Some(a), Some(b)) => Appearance {
                look: Some(Look::Picture {
                    url: url.to_owned(),
                    size: [a, b],
                    hangs,
                }),
                to_set: Vec::new(),
                colour: rgb,
            },
            _ => Appearance {
                look: colour.clone(),
                to_set,
                colour: rgb,
            },
        }
    };
    match text(fields, "finish_type") {
        Some("covering") => picture(["tile_width_cm", "tile_length_cm"], false),
        Some("wallpaper") => picture(["roll_width_cm", "pattern_repeat_cm"], true),
        Some("paint") => Appearance {
            to_set: if colour.is_some() {
                Vec::new()
            } else {
                vec!["colour_hex"]
            },
            look: colour,
            colour: rgb,
        },
        _ => Appearance {
            look: colour,
            to_set: vec!["finish_type"],
            colour: rgb,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::product::{Evidence, Field};

    fn with(names: &[(&str, Value)]) -> Fields {
        names
            .iter()
            .map(|(n, v)| {
                (
                    (*n).to_owned(),
                    Field {
                        value: v.clone(),
                        evidence: Evidence::Agent,
                        from: String::new(),
                    },
                )
            })
            .collect()
    }

    fn picture(url: &str, size: [f64; 2], hangs: bool) -> Look {
        Look::Picture {
            url: url.to_owned(),
            size,
            hangs,
        }
    }

    #[test]
    fn a_tile_or_plank_is_its_swatch_at_the_sample_s_size() {
        let tile = with(&[
            ("finish_type", json!("covering")),
            ("swatch_image", json!("https://x/t.jpg")),
            ("tile_width_cm", json!(30.0)),
            ("tile_length_cm", json!(60.0)),
            ("colour_hex", json!("#808080")),
            ("pack_m2", json!(1.44)),
            ("image", json!("https://x/pack.jpg")),
        ]);
        assert_eq!(
            appearance(&tile),
            Appearance {
                look: Some(picture("https://x/t.jpg", [30.0, 60.0], false)),
                to_set: vec![],
                colour: Some([0x80, 0x80, 0x80]),
            }
        );
    }

    #[test]
    fn a_wallpaper_is_its_repeat_across_the_roll() {
        let paper = with(&[
            ("finish_type", json!("wallpaper")),
            ("swatch_image", json!("https://x/w.jpg")),
            ("roll_width_cm", json!(53.0)),
            ("roll_length_m", json!(10.05)),
            ("pattern_repeat_cm", json!(64.0)),
        ]);
        assert_eq!(
            appearance(&paper),
            Appearance {
                look: Some(picture("https://x/w.jpg", [53.0, 64.0], true)),
                to_set: vec![],
                colour: None,
            }
        );
        // No repeat: the picture's height is no known size.
        let plain = with(&[
            ("finish_type", json!("wallpaper")),
            ("swatch_image", json!("https://x/w.jpg")),
            ("roll_width_cm", json!(53.0)),
            ("colour_hex", json!("#EEEBE1")),
        ]);
        assert_eq!(
            appearance(&plain),
            Appearance {
                look: Some(Look::Colour([0xee, 0xeb, 0xe1])),
                to_set: vec!["pattern_repeat_cm"],
                colour: Some([0xee, 0xeb, 0xe1]),
            }
        );
    }

    #[test]
    fn without_a_sized_swatch_a_covering_is_its_colour_or_nothing() {
        let no_size = with(&[
            ("finish_type", json!("covering")),
            ("swatch_image", json!("https://x/t.jpg")),
            ("tile_width_cm", json!(30.0)),
            ("colour_hex", json!("#102030")),
        ]);
        assert_eq!(
            appearance(&no_size),
            Appearance {
                look: Some(Look::Colour([0x10, 0x20, 0x30])),
                to_set: vec!["tile_length_cm"],
                colour: Some([0x10, 0x20, 0x30]),
            }
        );
        // The product photo is not a swatch.
        let photo_only = with(&[
            ("finish_type", json!("covering")),
            ("image", json!("https://x/room.jpg")),
            ("tile_width_cm", json!(30.0)),
            ("tile_length_cm", json!(60.0)),
        ]);
        assert_eq!(
            appearance(&photo_only),
            Appearance {
                look: None,
                to_set: vec!["swatch_image"],
                colour: None,
            }
        );
        assert_eq!(
            appearance(&with(&[("finish_type", json!("covering"))])),
            Appearance {
                look: None,
                to_set: vec!["swatch_image", "tile_width_cm", "tile_length_cm"],
                colour: None,
            }
        );
    }

    #[test]
    fn paint_is_the_shop_s_colour_and_a_colour_name_is_not_one() {
        assert_eq!(
            appearance(&with(&[
                ("finish_type", json!("paint")),
                ("colour_hex", json!("#f2efe6")),
                ("swatch_image", json!("https://x/p.jpg")),
                ("tile_width_cm", json!(30.0)),
                ("tile_length_cm", json!(30.0)),
            ])),
            Appearance {
                look: Some(Look::Colour([0xf2, 0xef, 0xe6])),
                to_set: vec![],
                colour: Some([0xf2, 0xef, 0xe6]),
            }
        );
        assert_eq!(
            appearance(&with(&[
                ("finish_type", json!("paint")),
                ("colour", json!("Gebroken wit")),
            ])),
            Appearance {
                look: None,
                to_set: vec!["colour_hex"],
                colour: None,
            }
        );
    }

    #[test]
    fn a_finish_of_unknown_type_is_its_colour_until_its_type_is_set() {
        let untyped = with(&[
            ("swatch_image", json!("https://x/t.jpg")),
            ("tile_width_cm", json!(30.0)),
            ("tile_length_cm", json!(60.0)),
            ("colour_hex", json!("#000000")),
        ]);
        assert_eq!(
            appearance(&untyped),
            Appearance {
                look: Some(Look::Colour([0, 0, 0])),
                to_set: vec!["finish_type"],
                colour: Some([0, 0, 0]),
            }
        );
        assert_eq!(hex("#0a0B0c"), Some([10, 11, 12]));
        assert_eq!(hex("0a0b0c"), None);
        assert_eq!(hex("#0a0b0"), None);
        assert_eq!(hex("#0a0b0g"), None);
    }
}
