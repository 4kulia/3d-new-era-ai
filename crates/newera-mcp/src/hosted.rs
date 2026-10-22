//! The tools the hosted service answers itself, from its own database: the
//! account's projects, its library of shop products and the estimate.
//!
//! They are defined here, beside every other tool, so they go through the
//! same rules — hints, output schema, budget, search, naming — and are
//! checked against their schema the same way; but they are never run by
//! [`crate::call`]: no document holds a project list or a product's price.
//! The hosted service runs them (`newera-cloud`), and [`crate::surface`]
//! offers them only there.

use rmcp::handler::server::common::schema_for_type;
use rmcp::model::Tool;
use schemars::JsonSchema;

/// What `import_product` takes.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct ImportProductSchema {
    /// The link to the product in the shop, e.g. `https://www.ikea.com/nl/nl/p/…`, `https://www.praxis.nl/…`
    url: String,
    /// What it is: a piece that stands in a room, or a finish (tile, laminate, paint, wallpaper). Default: what the shop shows
    #[schemars(extend("enum" = ["furniture", "finish"]))]
    kind: Option<String>,
}

/// What `products` takes.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct ProductsSchema {
    /// One product in full: every field with its evidence, and its price history
    id: Option<String>,
    /// Words to find in the title, shop or link
    q: Option<String>,
}

/// What `edit_products` takes.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct EditProductsSchema {
    /// The product, as `import_product` or `products` gave it
    id: String,
    /// `set` fills in or corrects fields; `refresh` reads the shop again for today's price; `attach_model` gives it a 3D model; `generate` has the server make one; `remove` deletes the product
    #[schemars(extend("enum" = ["set", "refresh", "attach_model", "generate", "remove"]))]
    action: String,
    /// For `set`: field → value, e.g. `{"price": "24.54", "pack_m2": 1.62, "width_cm": 80}`
    fields: Option<serde_json::Map<String, serde_json::Value>>,
    /// For `attach_model`: the .glb file's link
    url: Option<String>,
    /// For `attach_model`: what made it from the photo, e.g. `TRELLIS`; none: the shop's own file
    generator: Option<String>,
}

/// What `estimate` takes.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct EstimateSchema {
    /// How the lines are grouped; default shop. A product bought for several rooms stays one line, under `several rooms`
    #[schemars(extend("enum" = ["shop", "room", "kind"]))]
    group_by: Option<String>,
    /// A saved estimate, by its name or id: the lines and totals as they were saved
    saved: Option<String>,
    /// `csv`: a download link to the sheet, good for a day, instead of the lines
    #[schemars(extend("enum" = ["csv"]))]
    format: Option<String>,
}

/// What `edit_estimate` takes.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct EditEstimateSchema {
    /// `add_line` adds a line of your own; `remove_line` takes one off; `save` keeps the estimate as it is now; `remove_saved` deletes a saved one
    #[schemars(extend("enum" = ["add_line", "remove_line", "save", "remove_saved"]))]
    action: String,
    /// For `add_line`: what it is, e.g. `Tiling labour`, `Delivery`, `Tile adhesive 25 kg`
    title: Option<String>,
    /// For `add_line`: how many; default 1
    quantity: Option<f64>,
    /// For `add_line`: what the quantity counts, e.g. `hour`; default `item`
    unit: Option<String>,
    /// For `add_line`: the price of one, as text, e.g. `"45.00"`
    unit_price: Option<String>,
    /// For `add_line`: default EUR
    currency: Option<String>,
    /// For `add_line`: whether the price includes VAT; unsaid, its totals are apart from both
    vat_included: Option<bool>,
    /// For `add_line`: who it is bought from or paid to
    shop: Option<String>,
    /// For `add_line`: a remark kept with the line
    note: Option<String>,
    /// For `remove_line`: the line's id, as estimate lists it
    line: Option<String>,
    /// For `save`: the name it is kept under; default the date and time
    name: Option<String>,
    /// For `remove_saved`: the saved estimate's name or id
    saved: Option<String>,
}

const PROJECTS: &str = "The projects kept in the account: rows [name, id, kb, updated, active], the space used and the plan's limits. file action=open switches the active one; action=new starts one.";

const IMPORT_PRODUCT: &str = "Import a product from a shop's link into the account's library: a piece (a cabinet, a sofa…) or a finish (tile, laminate, paint, wallpaper) with its price, pack and coverage. The same article imported again is the one kept, not fetched twice. A piece whose shop shows a picture and its width, depth and height gets a 3D model made on the server at that size. Reply: id, shop, kind, fields {name: {value, evidence, from}} — evidence is adapter, text, market or agent — missing (what the shop did not show, for edit_products to fill), fetched (calls made and their credits), job (the model being made). products lists the library.";

const PRODUCTS: &str = "The account's library of shop products: rows [id, kind, shop, title, price, currency, sale_unit, missing], or with id one product in full — its fields and where each came from, what is missing, and its price history. import_product adds one; edit_products fills fields, fetches today's price or removes one.";

const EDIT_PRODUCTS: &str = "Change a product in the library. set {fields}: fill in or correct fields (price, pack_m2, width_cm…), kept as the agent's — swatch_image, with tile_width_cm/tile_length_cm or roll_width_cm/pattern_repeat_cm, is the look on a surface, replaced where it shows; the set that completes a piece's image and sizes starts generate (job); refresh: read the shop again for today's price, VAT and price per unit, which replace the old ones and are added to the price history (title, size and pack stay as they are); attach_model {url, generator?}: keep the .glb at url as its 3D file, which place draws and which replaces its placeholder where it is open; generate: have the server make (or remake, after a resize) its 3D file from its picture (field image, https) at its width, depth and height (all three set), replacing the model when done (products shows the job); remove: delete it. Reply: the product as products shows it. products reads the library.";

const ESTIMATE: &str = "What the project costs: every shop product it uses — finishes by their m² with waste and coats, in whole packs, cans or rolls; furniture by the piece or pack — priced from the library, with the lines added by hand. Reply: groups {name, lines, totals}; lines {kind, shop, title, url, quantity, unit, unit_price, currency, vat_included, total, rooms, surfaces, notes}; totals per shop, currency and VAT (included, excluded, unknown), complete false while a line is unpriced; note when an open editor's project is not the one priced; saved lists the saved ones. takeoff measures; edit_estimate adds lines and saves.";

const EDIT_ESTIMATE: &str = "Change the project's estimate. add_line {title, unit_price, quantity?, unit?, currency?, vat_included?, shop?, note?}: a line of your own (labour, delivery, glue, primer); remove_line {line}; save {name?}: keep the estimate as it is now, never changed after; remove_saved {saved}: delete a saved one. Reply: the line, or the saved estimate with its totals. estimate reads it.";

/// The hosted service's own tools, laid out like every other.
pub(crate) fn tools() -> Vec<Tool> {
    [
        ("projects", PROJECTS, no_arguments()),
        (
            "import_product",
            IMPORT_PRODUCT,
            schema_for_type::<ImportProductSchema>(),
        ),
        ("products", PRODUCTS, schema_for_type::<ProductsSchema>()),
        (
            "edit_products",
            EDIT_PRODUCTS,
            schema_for_type::<EditProductsSchema>(),
        ),
        ("estimate", ESTIMATE, schema_for_type::<EstimateSchema>()),
        (
            "edit_estimate",
            EDIT_ESTIMATE,
            schema_for_type::<EditEstimateSchema>(),
        ),
    ]
    .into_iter()
    .map(|(name, description, schema)| {
        let mut schema = serde_json::Value::Object((*schema).clone());
        crate::schema::compact(&mut schema);
        let serde_json::Value::Object(mut schema) = schema else {
            unreachable!("a schema is an object")
        };
        // The struct's own doc comment says what it is to us; the tool's
        // description says it to the agent.
        schema.remove("description");
        let mut tool = Tool::new(name, description, std::sync::Arc::new(schema));
        crate::hints::apply(&mut tool);
        crate::output::apply(&mut tool);
        tool
    })
    .collect()
}

/// A read that takes nothing, and refuses anything given.
fn no_arguments() -> std::sync::Arc<serde_json::Map<String, serde_json::Value>> {
    let serde_json::Value::Object(schema) = serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    }) else {
        unreachable!("an object")
    };
    std::sync::Arc::new(schema)
}

/// The first argument a call to a hosted tool was given that its schema
/// does not declare, as the sentence that says so. `None` for a call that
/// fits, and for a tool that is not hosted.
pub(crate) fn unknown_argument(name: &str, args: &serde_json::Value) -> Option<String> {
    let tool = tools().into_iter().find(|t| t.name == name)?;
    let schema = serde_json::Value::Object((*tool.input_schema).clone());
    crate::args::unknown(&schema, args)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_hosted_call_is_checked_against_its_schema() {
        let why = super::unknown_argument("import_product", &json!({"link": "x"})).unwrap();
        assert_eq!(why, "unknown argument `link`: it takes kind, url");
        assert_eq!(
            super::unknown_argument(
                "edit_products",
                &json!({"id": "p", "action": "refresh", "fields": {}})
            ),
            Some("`fields` is for `set`, not `refresh`".to_owned())
        );
        assert_eq!(
            super::unknown_argument(
                "edit_products",
                &json!({"id": "p", "action": "set", "fields": {"price": "1"}})
            ),
            None
        );
        assert_eq!(super::unknown_argument("projects", &json!({})), None);
        assert_eq!(
            super::unknown_argument("projects", &json!({"x": 1})),
            Some("unknown argument `x`: it takes no arguments".to_owned())
        );
        assert_eq!(super::unknown_argument("home", &json!({"x": 1})), None);
        assert_eq!(
            super::unknown_argument("edit_estimate", &json!({"action": "save", "title": "x"})),
            Some("`title` is for `add_line`, not `save`".to_owned())
        );
        assert_eq!(
            super::unknown_argument(
                "edit_estimate",
                &json!({"action": "add_line", "title": "Labour", "unit_price": "45", "quantity": 8, "vat_included": false})
            ),
            None
        );
        assert_eq!(
            super::unknown_argument("estimate", &json!({"group": "room"})),
            Some("unknown argument `group`: it takes format, group_by, saved".to_owned())
        );
    }
}
