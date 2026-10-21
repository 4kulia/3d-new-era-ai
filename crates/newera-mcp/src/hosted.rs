//! The tools the hosted service answers itself, from its own database: the
//! account's projects and its library of shop products.
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
    /// `set` fills in or corrects fields; `refresh` reads the shop again for today's price; `remove` deletes the product
    #[schemars(extend("enum" = ["set", "refresh", "remove"]))]
    action: String,
    /// For `set`: field → value, e.g. `{"price": "24.54", "pack_m2": 1.62, "width_cm": 80}`
    fields: Option<serde_json::Map<String, serde_json::Value>>,
}

const PROJECTS: &str = "The projects kept in the account: rows [name, id, kb, updated, active], the space used and the plan's limits. file action=open switches the active one; action=new starts one.";

const IMPORT_PRODUCT: &str = "Import a product from a shop's link into the account's library: a piece (a cabinet, a sofa…) or a finish (tile, laminate, paint, wallpaper) with its price, pack and coverage. The same article imported again is the one kept, not fetched twice. Reply: id, shop, kind, fields {name: {value, evidence, from}} — evidence is adapter, text, market or agent — missing (what the shop did not show, for edit_products to fill), fetched (calls made and their credits). products lists the library.";

const PRODUCTS: &str = "The account's library of shop products: rows [id, kind, shop, title, price, currency, sale_unit, missing], or with id one product in full — its fields and where each came from, what is missing, and its price history. import_product adds one; edit_products fills fields, fetches today's price or removes one.";

const EDIT_PRODUCTS: &str = "Change a product in the library. set {fields}: fill in or correct fields (price, pack_m2, width_cm…), kept as the agent's; refresh: read the shop again for today's price, VAT and price per unit, which replace the old ones and are added to the price history (title, size and pack stay as they are); remove: delete it. Reply: the product as products shows it. products reads the library.";

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
    }
}
