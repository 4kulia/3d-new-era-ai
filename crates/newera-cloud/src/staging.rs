//! Shop products named in a call to the editor's tools, made ready before
//! the call goes to the tab or the engine.
//!
//! `place` takes `model=product:<id>` and `edit_finishes` takes
//! `product=<id>` (or `product:<id>`). The editor has no product library: here
//! each id is checked against the account's products, a piece's model is
//! fetched (a placeholder built when it has none, `crate::models`), and the
//! call is rewritten to what the editor reads — `model=` the file's path
//! inside the project, `products/<id>/model.glb`, and `product=<id>`, which
//! `place` keeps on the piece. A width, depth or height the call does not
//! give is the shop's, and the model is stretched to the piece's box.
//!
//! A finish assigned to a whole surface without a `mat` takes the product's
//! look (`newera_shop::look`): its swatch picture, kept as
//! `products/<id>/finish.jpg` (`crate::textures`) and drawn tiled at the
//! sample's real size, or the shop's colour. The agent is told what to set
//! when the product has neither. A band is a purchase only: the side's look
//! is left as it is.
//!
//! The files go to whichever side runs the call, before it runs: into the
//! engine's copy of the project, or to the open editor tab over the relay
//! (`mcp.rs`). Either way they are among the project's files, and the next
//! save carries them inside the `.newera`. A model attached later, or a
//! finish's new swatch, replaces the file where it is open now ([`swap`],
//! [`swap_texture`]), and in every other project the next time it is
//! opened (`crate::current`).

use std::time::Duration;

use newera_shop::look::Look;
use newera_shop::product::Fields;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::types::Json;

use crate::AppState;
use crate::accounts::Plan;
use crate::models::{self, Source};
use crate::textures::Unused;

/// How long the tab may take to keep a product's file: one socket message
/// of up to `models::MODEL_LIMIT` as base64. A chosen bound, not a measured
/// one; past it the file is not in the tab.
pub const STAGE_WAIT: Duration = Duration::from_secs(30);

/// When a project that is not open takes a product's new file: the next
/// time it is opened (`crate::current`).
const LATER: &str = "the next time it is opened";

/// A call made ready: its arguments as the editor reads them, the files to
/// put in the project first (path inside it, bytes), and what to tell the
/// agent beside the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Staged {
    pub args: Value,
    pub files: Vec<(String, Vec<u8>)>,
    pub notes: Vec<String>,
}

/// The id a `product:<id>` reference names.
#[must_use]
pub fn product_ref(value: &str) -> Option<&str> {
    value
        .strip_prefix(newera_mcp::PRODUCT_REF)
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Where a product's model sits in a project.
#[must_use]
pub fn model_path(product: &str) -> String {
    format!("products/{product}/model.glb")
}

/// Points a `place` spec at a product's staged model: `model=` its path,
/// `product=` its id, its catalog id dropped (it chose the placeholder's
/// shape), and each of w, d, h that neither it nor `given` (the defaults an
/// item takes) says is the shop's. When any was filled the model is
/// stretched to the box, unless the call says otherwise. Items and the
/// defaults are pointed alike.
fn point_at(spec: &mut Value, product: &str, size: [Option<f64>; 3], given: &Value) {
    let Some(spec) = spec.as_object_mut() else {
        return;
    };
    spec.insert("model".into(), json!(model_path(product)));
    spec.insert("product".into(), json!(product));
    spec.remove("cat");
    let mut filled = false;
    for (key, shop) in ["w", "d", "h"].into_iter().zip(size) {
        let said = spec.get(key).is_some_and(|v| !v.is_null()) || !given[key].is_null();
        if let Some(cm) = shop
            && !said
        {
            spec.insert(key.into(), json!(cm));
            filled = true;
        }
    }
    if filled {
        spec.entry("stretch").or_insert(json!(true));
    }
}

/// The products a `place` call names, in its items and its defaults: the
/// pointer of each spec (`/items/0`, `/defaults`) and the id.
#[must_use]
pub fn placed(args: &Value) -> Vec<(String, String)> {
    let items = args["items"].as_array().map_or(0, Vec::len);
    (0..items)
        .map(|i| format!("/items/{i}"))
        .chain(std::iter::once("/defaults".to_owned()))
        .filter_map(|at| {
            let id = args.pointer(&at)?["model"].as_str().and_then(product_ref)?;
            Some((at, id.to_owned()))
        })
        .collect()
}

/// The account's product `id`: its kind and fields.
async fn product(db: &PgPool, account: &str, id: &str) -> sqlx::Result<Option<(String, Fields)>> {
    let row: Option<(String, Json<Fields>)> =
        sqlx::query_as("select kind, fields from products where account_id = $1 and id = $2")
            .bind(account)
            .bind(id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(kind, fields)| (kind, fields.0)))
}

fn no_product(id: &str) -> String {
    format!("no product {id} (products lists them)")
}

/// Makes a call ready, when it names products: `Ok(None)` for a call that
/// names none, which goes as it came; `Ok(Err(why))` for one that names a
/// product the account does not have, or one that cannot be placed.
///
/// # Errors
///
/// When the database fails.
pub async fn stage(
    app: &AppState,
    account: &str,
    tool: &str,
    args: &Value,
) -> anyhow::Result<Result<Option<Staged>, String>> {
    match tool {
        "place" => place(&app.db, account, args).await,
        "edit_finishes" => finish(app, account, args).await,
        _ => Ok(Ok(None)),
    }
}

async fn place(
    db: &PgPool,
    account: &str,
    args: &Value,
) -> anyhow::Result<Result<Option<Staged>, String>> {
    let named = placed(args);
    if named.is_empty() {
        return Ok(Ok(None));
    }
    let mut staged = Staged {
        args: args.clone(),
        files: Vec::new(),
        notes: Vec::new(),
    };
    let defaults = args["defaults"].clone();
    for (at, id) in named {
        let Some((kind, fields)) = product(db, account, &id).await? else {
            return Ok(Err(no_product(&id)));
        };
        if kind == "finish" {
            return Ok(Err(format!(
                "{id} is a finish: edit_finishes puts it on a floor, a ceiling or a wall side"
            )));
        }
        let spec = staged.args.pointer_mut(&at).expect("found above");
        let asked = spec["cat"].as_str().filter(|c| !c.is_empty());
        let model = match models::for_placement(db, &id, &fields, asked).await? {
            Ok(model) => model,
            Err(why) => return Ok(Err(why)),
        };
        let path = model_path(&id);
        if !staged.files.iter().any(|(p, _)| *p == path) {
            if model.source == Source::Placeholder {
                staged.notes.push(placeholder_note(
                    &id,
                    model.shape.as_deref().unwrap_or("box"),
                ));
            }
            staged.files.push((path, model.bytes));
        }
        let given = if at == "/defaults" {
            Value::Null
        } else {
            defaults.clone()
        };
        point_at(spec, &id, models::dimensions(&fields), &given);
    }
    Ok(Ok(Some(staged)))
}

/// What the agent is told of a product placed as a placeholder.
fn placeholder_note(id: &str, shape: &str) -> String {
    format!(
        "{id} has no 3D model yet: it is drawn as a placeholder ({shape}), a generated approximation stretched to the piece's box. edit_products action=attach_model gives it a model, which replaces the placeholder at once in the project open now (the editor tab, or the server's copy) and in any other project the next time that project is opened"
    )
}

/// Where an attached model is drawn now, for `attach_model`'s answer: the
/// file at the product's path is replaced in the open editor tab when one
/// holds the room and its document draws the product, else in the server's
/// open copy of the active project when it draws the product (and saved). Any other project takes it the
/// next time it is opened (`crate::current`).
///
/// # Errors
///
/// When the database fails, or the engine cannot write the file.
pub async fn swap(
    app: &AppState,
    account: &str,
    plan: &Plan,
    product: &str,
    bytes: &[u8],
) -> anyhow::Result<String> {
    swap_file(app, account, plan, &model_path(product), bytes, None).await
}

/// Where a finish's new texture is drawn now, for `edit_products`' answer:
/// as [`swap`] does with a model, at the texture's path; with `tile`, the
/// surfaces drawn with it take that size, as its new picture may lie the
/// other way. Any other project takes it, at that size, the next time it
/// is opened (`crate::current`).
///
/// # Errors
///
/// When the database fails, or the engine cannot write the file.
pub async fn swap_texture(
    app: &AppState,
    account: &str,
    plan: &Plan,
    product: &str,
    bytes: &[u8],
    tile: Option<[f64; 2]>,
) -> anyhow::Result<String> {
    swap_file(
        app,
        account,
        plan,
        &crate::textures::texture_path(product),
        bytes,
        tile,
    )
    .await
}

/// Replaces the file at `path` where the open document draws with it: the
/// editor tab holding the room, else the server's open copy (saved); with
/// `tile`, the surfaces drawn with it take that size there. The answer says
/// where, and that any other project takes it the next time it is opened.
async fn swap_file(
    app: &AppState,
    account: &str,
    plan: &Plan,
    path: &str,
    bytes: &[u8],
    tile: Option<[f64; 2]>,
) -> anyhow::Result<String> {
    if let Some(room) = app.rooms.owned_by(account) {
        return Ok(
            match newera_relay::replace_in(&app.rooms, &room, path, bytes, tile, STAGE_WAIT).await {
                // A texture sent with its size is drawn right only when the
                // tab says how many elements it resized: an older editor
                // says nothing, a failed resize says why.
                Ok(kept) if kept["kept"] == true && tile.is_some() && !kept["retiled"].is_u64() => {
                    let why = kept["retiled"]
                        .as_str()
                        .unwrap_or("the tab did not resize them");
                    format!(
                        "the picture is replaced in the open editor tab, but the surfaces drawn with it keep their size ({why}): reload the tab, which opens them at their size; any other project takes it {LATER}"
                    )
                }
                Ok(kept) if kept["kept"] == true => {
                    format!("in the open editor tab now; any other project takes it {LATER}")
                }
                // The tab's document does not draw the product.
                Ok(_) => format!("each project takes it {LATER}"),
                Err(why) => {
                    format!("not in the open editor tab ({why}); each project takes it {LATER}")
                }
            },
        );
    }
    Ok(
        if app
            .engine
            .swap_in(&app.db, account, plan, path, bytes, tile)
            .await?
        {
            format!("in the active project now, saved with it; any other project takes it {LATER}")
        } else {
            format!("each project takes it {LATER}")
        },
    )
}

async fn finish(
    app: &AppState,
    account: &str,
    args: &Value,
) -> anyhow::Result<Result<Option<Staged>, String>> {
    let Some(given) = args["product"].as_str() else {
        return Ok(Ok(None));
    };
    let id = product_ref(given).unwrap_or_else(|| given.trim());
    let Some((kind, fields)) = product(&app.db, account, id).await? else {
        return Ok(Err(no_product(id)));
    };
    if kind == "furniture" {
        return Ok(Err(format!(
            "{id} is a piece of furniture: place model=product:{id} puts it in the room"
        )));
    }
    let mut staged = Staged {
        args: args.clone(),
        files: Vec::new(),
        notes: Vec::new(),
    };
    staged.args["product"] = json!(id);
    // The look the call gives is the one the surface takes; a band is a
    // purchase only, and the side's look stays as it is.
    let assigns = args["action"] == "assign";
    let said = args.get("mat").is_some_and(|m| !m.is_null());
    let banded = args.get("band").is_some_and(|b| !b.is_null());
    if !assigns || said {
        return Ok(Ok(Some(staged)));
    }
    if banded {
        staged.notes.push(
            "a band carries only the purchase: the side's look stays as it is (mat sets it, for the whole side)"
                .to_owned(),
        );
        return Ok(Ok(Some(staged)));
    }
    let appearance = newera_shop::look::appearance(&fields);
    let colour = appearance.colour;
    let mut drawn_as_colour = false;
    match appearance.look {
        Some(Look::Picture { url, size, hangs }) => {
            let plan = crate::accounts::plan_of(&app.db, account).await?;
            match crate::textures::for_swatch(&app.db, &app.links, account, &plan, id, &url).await?
            {
                Ok(bytes) => {
                    let px = crate::textures::dimensions(&bytes)
                        .ok_or_else(|| anyhow::anyhow!("a kept texture of {id} has no size"))?;
                    staged.args["mat"] = json!(crate::textures::material(
                        id,
                        crate::textures::drawn_size(size, hangs, px)
                    ));
                    staged
                        .files
                        .push((crate::textures::texture_path(id), bytes));
                }
                Err(unused) => {
                    if let Some(rgb) = colour {
                        staged.args["mat"] = json!(crate::textures::colour(rgb));
                    }
                    let now = if colour.is_some() {
                        "in its colour"
                    } else {
                        "with the look the surface had"
                    };
                    staged.notes.push(match unused {
                        Unused::Picture(why) => format!(
                            "{id}'s swatch picture could not be used ({why}), so it is drawn {now}: edit_products action=set fields swatch_image gives it another"
                        ),
                        Unused::NoRoom(why) => format!(
                            "{id}'s texture was not kept ({why}), so it is drawn {now}: make room in the plan's storage (projects, products), then edit_finishes assign again"
                        ),
                    });
                }
            }
        }
        Some(Look::Colour(rgb)) => {
            staged.args["mat"] = json!(crate::textures::colour(rgb));
            drawn_as_colour = true;
        }
        None => {}
    }
    if !appearance.to_set.is_empty() {
        staged
            .notes
            .push(look_note(id, drawn_as_colour, &appearance.to_set));
    }
    Ok(Ok(Some(staged)))
}

/// What the agent is told of a finish drawn without its picture, or with
/// no look of its own: which fields would give it one.
fn look_note(id: &str, drawn_as_colour: bool, to_set: &[&str]) -> String {
    let now = if drawn_as_colour {
        "it is drawn in its colour"
    } else {
        "the surface keeps the look it had"
    };
    if to_set.contains(&"finish_type") {
        return format!(
            "{id} has no finish_type, which says what its look is made of, so {now}: edit_products action=set fields finish_type (covering, paint or wallpaper), then edit_finishes assign again"
        );
    }
    let fields = to_set.join(", ");
    let swatch = if to_set.contains(&"swatch_image") {
        " (swatch_image: a picture of one tile or plank, or of one wallpaper repeat across the roll, never of the pack or a room)"
    } else {
        ""
    };
    if drawn_as_colour {
        format!(
            "{id} is drawn in its colour: edit_products action=set fields {fields}{swatch} draws it as it looks, at its size, once edit_finishes assigns it again"
        )
    } else {
        format!(
            "{id} has no look of its own, so the surface keeps the one it had: edit_products action=set fields {fields}{swatch} gives it one, drawn once edit_finishes assigns it again"
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_product_reference_names_an_id() {
        assert_eq!(super::product_ref("product:p1"), Some("p1"));
        assert_eq!(super::product_ref("product: p1 "), Some("p1"));
        assert_eq!(super::product_ref("product:"), None);
        assert_eq!(super::product_ref("models/chair.glb"), None);
    }

    #[test]
    fn the_products_a_placement_names_are_found_in_items_and_defaults() {
        let args = json!({
            "items": [
                {"model": "product:p1", "at": [0, 0]},
                {"cat": "sofa-3", "at": [0, 0]},
                {"model": "chair.glb", "at": [0, 0]},
                {"model": "product:p2", "at": [0, 0]}
            ],
            "defaults": {"model": "product:p3"}
        });
        assert_eq!(
            super::placed(&args),
            [
                ("/items/0".to_owned(), "p1".to_owned()),
                ("/items/3".to_owned(), "p2".to_owned()),
                ("/defaults".to_owned(), "p3".to_owned()),
            ]
        );
        assert_eq!(
            super::placed(&json!({"items": [{"cat": "bed"}]})),
            Vec::<(String, String)>::new()
        );
    }

    #[test]
    fn a_placed_product_takes_the_shops_size_where_the_call_gives_none() {
        let size = [Some(80.0), Some(28.0), Some(202.0)];
        let mut bare = json!({"model": "product:p1", "cat": "wardrobe", "at": [10, 20]});
        super::point_at(&mut bare, "p1", size, &json!(null));
        assert_eq!(
            bare,
            json!({"model": "products/p1/model.glb", "product": "p1", "at": [10, 20],
                   "w": 80.0, "d": 28.0, "h": 202.0, "stretch": true})
        );
        let mut tall = json!({"model": "product:p1", "at": [0, 0], "h": 100});
        super::point_at(&mut tall, "p1", size, &json!(null));
        assert_eq!(
            tall,
            json!({"model": "products/p1/model.glb", "product": "p1", "at": [0, 0],
                   "w": 80.0, "d": 28.0, "h": 100, "stretch": true})
        );
        // An item's defaults count as what it says, as the defaults' own do.
        let mut from_defaults = json!({"model": "product:p1", "at": [0, 0]});
        super::point_at(&mut from_defaults, "p1", size, &json!({"w": 60}));
        assert_eq!(
            from_defaults,
            json!({"model": "products/p1/model.glb", "product": "p1", "at": [0, 0],
                   "d": 28.0, "h": 202.0, "stretch": true})
        );
        let mut defaults = json!({"model": "product:p1", "w": 60});
        super::point_at(&mut defaults, "p1", size, &json!(null));
        assert_eq!(
            defaults,
            json!({"model": "products/p1/model.glb", "product": "p1",
                   "w": 60, "d": 28.0, "h": 202.0, "stretch": true})
        );
        let mut all_given =
            json!({"model": "product:p1", "at": [0, 0], "w": 1, "d": 2, "h": 3, "stretch": false});
        super::point_at(&mut all_given, "p1", size, &json!(null));
        assert_eq!(
            all_given,
            json!({"model": "products/p1/model.glb", "product": "p1", "at": [0, 0],
                   "w": 1, "d": 2, "h": 3, "stretch": false})
        );
        let mut unsized_product = json!({"model": "product:p1", "at": [0, 0]});
        super::point_at(
            &mut unsized_product,
            "p1",
            [None, Some(28.0), None],
            &json!(null),
        );
        assert_eq!(
            unsized_product,
            json!({"model": "products/p1/model.glb", "product": "p1", "at": [0, 0],
                   "d": 28.0, "stretch": true})
        );
    }
}
