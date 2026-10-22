//! The account's library of shop products: `import_product`, `products`
//! and `edit_products`, answered here from Postgres, never by the editor.
//!
//! A product is found by what its link names (`newera_shop::identify`): a
//! link to an article already in the library returns it without a fetch.
//! A new one is fetched through `AnySite` (`newera_shop::import`) with this
//! server's key; without one, importing says the server is not set up for
//! it, and the rest of the library keeps working. What the shop did not
//! show is returned as missing, for the calling agent to fill with
//! `edit_products`; no model runs here.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use newera_shop::import::Fetch;
use newera_shop::product::{self, Evidence, Field, Fields, Kind};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::types::Json;
use tokio::sync::watch;

use crate::{AppState, secret};

/// What a piece of work that runs apart from the request that asked for it
/// came to: the answer, a refusal in words for the caller, or a failure of
/// the server.
type Done = Result<Result<Value, String>, String>;

/// Fetches in flight, by what they work on.
///
/// A fetch costs credits and takes up to a few minutes, longer than a
/// client waits. It runs as a task of its own, so a client that gives up
/// does not lose what was paid for: the product is stored when the task
/// ends, and the client's next call finds it. A second call for the same
/// thing while the first runs waits for it instead of paying again.
#[derive(Debug, Clone, Default)]
pub struct Flights(Arc<Mutex<HashMap<String, watch::Receiver<Option<Done>>>>>);

/// Takes a flight off the map when its task ends, panic included.
struct Landed(Flights, String);

impl Drop for Landed {
    fn drop(&mut self) {
        if let Ok(mut flying) = self.0.0.lock() {
            flying.remove(&self.1);
        }
    }
}

impl Flights {
    /// Runs `work` as a task apart under `key` and waits for its end; if a
    /// task is already running under `key`, waits for that one instead
    /// and says so. Dropping the wait does not stop the task.
    async fn run<F>(&self, key: String, work: F) -> (bool, Done)
    where
        F: Future<Output = anyhow::Result<Result<Value, String>>> + Send + 'static,
    {
        let (mut ended, joined) = {
            let mut flying = self.0.lock().expect("the flights");
            if let Some(ended) = flying.get(&key) {
                (ended.clone(), true)
            } else {
                let (done, ended) = watch::channel(None);
                flying.insert(key.clone(), ended.clone());
                let landed = Landed(self.clone(), key);
                tokio::spawn(async move {
                    let outcome = work.await.map_err(|err| format!("{err:#}"));
                    drop(landed);
                    let _ = done.send(Some(outcome));
                });
                (ended, false)
            }
        };
        let done = match ended.wait_for(Option::is_some).await {
            Ok(done) => done.clone().unwrap_or_else(|| Err(INTERRUPTED.to_owned())),
            Err(_) => Err(INTERRUPTED.to_owned()),
        };
        (joined, done)
    }
}

const INTERRUPTED: &str = "the fetch ended without an answer";

/// An answer of a flight as the tool's, the server's failure as an error.
fn flown(done: Done) -> anyhow::Result<Value> {
    match done {
        Ok(Ok(answer)) => Ok(said(&answer)),
        Ok(Err(why)) => Ok(refused(why)),
        Err(failure) => Err(anyhow::anyhow!(failure)),
    }
}

/// Whether the tool is one of the library's.
pub fn answers(tool: &str) -> bool {
    matches!(tool, "import_product" | "products" | "edit_products")
}

/// What the tools say when this server has no `AnySite` key.
pub const NOT_CONFIGURED: &str = "product import is not configured on this server (it has no ANYSITE_API_KEY); products already in the library can still be read and changed";

fn said(answer: &Value) -> Value {
    json!({"content": [{"type": "text", "text": answer.to_string()}], "isError": false})
}

fn refused(why: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": why.into()}], "isError": true})
}

/// Runs one of the library's tools for an account.
///
/// # Errors
///
/// When the database fails; a call the tool refuses answers `Ok` with
/// `isError`.
pub async fn call(
    app: &AppState,
    account: &str,
    tool: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    match tool {
        "import_product" => import(app, account, args).await,
        "products" => list(&app.db, account, args).await,
        "edit_products" => edit(app, account, args).await,
        other => Ok(refused(format!("no tool named {other}"))),
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: String,
    shop: String,
    market: String,
    sku: String,
    url: String,
    kind: String,
    fields: Json<Fields>,
    fetched_at: i64,
}

const COLUMNS: &str = "id, shop, market, sku, url, kind, fields, extract(epoch from fetched_at)::bigint as fetched_at";

impl Row {
    fn kind(&self) -> Kind {
        Kind::parse(&self.kind).unwrap_or(Kind::Furniture)
    }

    fn title(&self) -> Option<Value> {
        self.fields.get("title").map(|f| f.value.clone())
    }

    fn missing(&self) -> Vec<&'static str> {
        product::needed(self.kind(), &self.fields)
    }
}

async fn by_id(db: &PgPool, account: &str, id: &str) -> sqlx::Result<Option<Row>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {COLUMNS} from products where account_id = $1 and id = $2"
    )))
    .bind(account)
    .bind(id)
    .fetch_optional(db)
    .await
}

async fn by_identity(
    db: &PgPool,
    account: &str,
    identity: &newera_shop::Identity,
) -> sqlx::Result<Option<Row>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {COLUMNS} from products
         where account_id = $1 and shop = $2 and market = $3 and sku = $4"
    )))
    .bind(account)
    .bind(&identity.shop)
    .bind(&identity.market)
    .bind(&identity.sku)
    .fetch_optional(db)
    .await
}

/// A product in full, as `products id=` shows it.
async fn shown(db: &PgPool, row: &Row) -> sqlx::Result<Value> {
    let prices: Vec<(String, String, Option<String>, String, i64)> = sqlx::query_as(
        "select price::text, currency, sale_unit, origin, extract(epoch from seen_at)::bigint
         from product_prices where product_id = $1 order by seen_at desc limit 20",
    )
    .bind(&row.id)
    .fetch_all(db)
    .await?;
    let mut shown = json!({
        "id": row.id,
        "shop": row.shop,
        "market": row.market,
        "sku": row.sku,
        "url": row.url,
        "kind": row.kind,
        "fields": row.fields.0,
        "missing": row.missing(),
        "fetched_at": row.fetched_at,
        "prices": prices
            .iter()
            .map(|(price, currency, unit, origin, at)| json!([price, currency, unit, origin, at]))
            .collect::<Vec<_>>(),
    });
    let files: Vec<(String, String, Option<String>, i32, i64)> = sqlx::query_as(
        "select kind, source, generator, size, extract(epoch from created_at)::bigint
         from product_assets where product_id = $1 and kind = 'model'",
    )
    .bind(&row.id)
    .fetch_all(db)
    .await?;
    if !files.is_empty() {
        shown["files"] = files
            .iter()
            .map(|(kind, source, generator, size, at)| json!([kind, source, generator, size, at]))
            .collect();
    }
    if let Some(job) = crate::generation::latest(db, &row.id).await? {
        shown["job"] = job;
    }
    // Unknown is absent, like every default: the title is text or not there.
    if let Some(title) = row.title() {
        shown["title"] = title;
    }
    Ok(shown)
}

/// Writes the product's price into its history when it has one.
async fn remember_price(db: &PgPool, id: &str, fields: &Fields) -> sqlx::Result<()> {
    let Some(price) = fields.get("price") else {
        return Ok(());
    };
    let Some(amount) = price.value.as_str() else {
        return Ok(());
    };
    let origin = match price.evidence {
        Evidence::Adapter => "adapter",
        Evidence::Agent => "agent",
        Evidence::Text | Evidence::Market => "text",
    };
    let text = |name: &str| {
        fields
            .get(name)
            .and_then(|f| f.value.as_str())
            .map(str::to_owned)
    };
    sqlx::query(
        "insert into product_prices (product_id, price, currency, vat_included, sale_unit, origin)
         values ($1, $2::numeric, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(amount)
    .bind(text("currency").unwrap_or_else(|| "EUR".to_owned()))
    .bind(fields.get("vat_included").and_then(|f| f.value.as_bool()))
    .bind(text("sale_unit"))
    .bind(origin)
    .execute(db)
    .await?;
    Ok(())
}

fn fetched(fetches: &[Fetch]) -> Value {
    fetches
        .iter()
        .map(|f| json!([f.endpoint, f.outcome, f.credits]))
        .collect()
}

async fn import(app: &AppState, account: &str, args: &Value) -> anyhow::Result<Value> {
    let Some(url) = args["url"].as_str() else {
        return Ok(refused(
            "import_product: url is the link to the product in its shop",
        ));
    };
    let kind = match args.get("kind").and_then(Value::as_str) {
        None => None,
        Some(k) => match Kind::parse(k) {
            Some(kind) => Some(kind),
            None => return Ok(refused(format!("kind is furniture or finish, not {k}"))),
        },
    };
    let identity = match newera_shop::identify(url) {
        Ok(identity) => identity,
        Err(why) => return Ok(refused(why)),
    };
    if let Some(row) = by_identity(&app.db, account, &identity).await? {
        let mut answer = shown(&app.db, &row).await?;
        answer["stored"] = json!(true);
        answer["fetched"] = json!([]);
        return Ok(said(&answer));
    }
    if app.config.anysite.is_none() {
        return Ok(refused(NOT_CONFIGURED));
    }
    let key = format!(
        "import\0{account}\0{}\0{}\0{}",
        identity.shop, identity.market, identity.sku
    );
    let work = fetch_and_keep(app.clone(), account.to_owned(), identity, kind);
    let (joined, done) = app.flights.run(key, work).await;
    // A call that waited for another's fetch paid for nothing: the
    // product is the one that fetch kept.
    let done = match (joined, done) {
        (true, Ok(Ok(mut answer))) => {
            answer["stored"] = json!(true);
            answer["fetched"] = json!([]);
            Ok(Ok(answer))
        }
        (_, done) => done,
    };
    flown(done)
}

/// Why an import that `AnySite` failed on did not happen, and what the
/// agent may do.
fn not_imported(err: &newera_shop::anysite::FetchError, what: &str) -> String {
    if err.is_outage() {
        format!("{err}: {what}; nothing was bought on top of it, call again in a minute")
    } else {
        format!("{err}: {what}")
    }
}

/// Fetches a product that is not in the library and keeps it. Runs as a
/// task of its own (`Flights`), so it ends and stores what it paid for
/// whether or not the client is still waiting.
async fn fetch_and_keep(
    app: AppState,
    account: String,
    identity: newera_shop::Identity,
    kind: Option<Kind>,
) -> anyhow::Result<Result<Value, String>> {
    // Kept by a fetch that ended while this one was starting.
    if let Some(row) = by_identity(&app.db, &account, &identity).await? {
        let mut answer = shown(&app.db, &row).await?;
        answer["stored"] = json!(true);
        answer["fetched"] = json!([]);
        return Ok(Ok(answer));
    }
    let Some(anysite) = app.config.anysite.clone() else {
        return Ok(Err(NOT_CONFIGURED.to_owned()));
    };
    let got = match newera_shop::import::fetch(&anysite, &identity, kind).await {
        Ok(got) => got,
        Err(err) => return Ok(Err(not_imported(&err, "the product was not imported"))),
    };
    let id = secret::id();
    let made: Option<String> = sqlx::query_scalar(
        "insert into products (id, account_id, shop, market, sku, url, kind, fields, fetches)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         on conflict (account_id, shop, market, sku) do nothing
         returning id",
    )
    .bind(&id)
    .bind(&account)
    .bind(&identity.shop)
    .bind(&identity.market)
    .bind(&identity.sku)
    .bind(&identity.url)
    .bind(got.kind.as_str())
    .bind(Json(&got.fields))
    .bind(Json(fetched(&got.fetches)))
    .fetch_optional(&app.db)
    .await?;
    // `None`: kept meanwhile by another call, whose product this is.
    let stored = made.is_none();
    if !stored {
        remember_price(&app.db, &id, &got.fields).await?;
        // A piece imported with a photo and its size has its model made
        // from it.
        if got.kind == Kind::Furniture
            && app.config.generator.is_some()
            && crate::generation::photo(&got.fields).is_some()
            && crate::generation::size(&got.fields).is_some()
        {
            crate::generation::enqueue(&app, &account, &id).await?;
        }
    }
    let Some(row) = by_identity(&app.db, &account, &identity).await? else {
        anyhow::bail!("the product just written is gone");
    };
    let mut answer = shown(&app.db, &row).await?;
    answer["stored"] = json!(stored);
    answer["fetched"] = fetched(&got.fetches);
    Ok(Ok(answer))
}

async fn list(db: &PgPool, account: &str, args: &Value) -> anyhow::Result<Value> {
    if let Some(id) = args.get("id").and_then(Value::as_str) {
        return Ok(match by_id(db, account, id).await? {
            Some(row) => said(&shown(db, &row).await?),
            None => refused(format!("no product {id} (products lists them)")),
        });
    }
    let q = args
        .get("q")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty());
    let pattern = q.map(|q| {
        format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {COLUMNS} from products
         where account_id = $1
           and ($2::text is null
                or coalesce(fields #>> '{{title,value}}', '') ilike $2
                or shop ilike $2 or url ilike $2 or sku ilike $2)
         order by updated_at desc"
    )))
    .bind(account)
    .bind(pattern)
    .fetch_all(db)
    .await?;
    let value = |row: &Row, name: &str| {
        row.fields
            .get(name)
            .map_or(Value::Null, |f| f.value.clone())
    };
    let answer = json!({
        "rows": rows.iter().map(|row| json!([
            row.id,
            row.kind,
            row.shop,
            row.title().unwrap_or(Value::Null),
            value(row, "price"),
            value(row, "currency"),
            value(row, "sale_unit"),
            row.missing(),
        ])).collect::<Vec<_>>(),
        "columns": ["id", "kind", "shop", "title", "price", "currency", "sale_unit", "missing"],
    });
    Ok(said(&answer))
}

async fn edit(app: &AppState, account: &str, args: &Value) -> anyhow::Result<Value> {
    let Some(id) = args["id"].as_str() else {
        return Ok(refused(
            "edit_products: id is the product's, as products lists it",
        ));
    };
    let action = args["action"].as_str().unwrap_or_default();
    if !matches!(
        action,
        "set" | "refresh" | "attach_model" | "generate" | "remove"
    ) {
        return Ok(refused(
            "edit_products: action is set, refresh, attach_model, generate or remove",
        ));
    }
    let Some(row) = by_id(&app.db, account, id).await? else {
        return Ok(refused(format!("no product {id} (products lists them)")));
    };
    match action {
        "remove" => {
            sqlx::query("delete from products where id = $1")
                .bind(id)
                .execute(&app.db)
                .await?;
            Ok(said(&json!({"id": id, "removed": true})))
        }
        "set" => set(&app.db, row, args).await,
        "attach_model" => attach_model(app, account, &row, args).await,
        "generate" => generate(app, account, &row).await,
        _ => refresh(app, account, row).await,
    }
}

/// Downloads the .glb at `url` and keeps it as the product's model
/// (`crate::models`): the shop's own file, or — with `generator`, what made
/// it — a generated one; it counts against the plan's storage. Where the
/// product is drawn already, it replaces the model there
/// (`crate::staging::swap`); the answer says where (`drawn`).
async fn attach_model(
    app: &AppState,
    account: &str,
    row: &Row,
    args: &Value,
) -> anyhow::Result<Value> {
    let Some(url) = args["url"]
        .as_str()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    else {
        return Ok(refused(
            "edit_products action=attach_model: url is the link to the .glb file",
        ));
    };
    if row.kind() != Kind::Furniture {
        return Ok(refused(format!(
            "{} is a finish: a 3D model is for a piece of furniture",
            row.id
        )));
    }
    let bytes = match crate::models::download(&app.links, url).await? {
        Ok(bytes) => bytes,
        Err(why) => return Ok(refused(why)),
    };
    let plan = crate::accounts::plan_of(&app.db, account).await?;
    let kept = crate::engine::kept_bytes(&app.db, account, None, Some(&row.id)).await?;
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if let Some(why) = crate::engine::no_room(&plan, kept.saturating_add(size)) {
        return Ok(refused(format!("the model was not kept: {why}")));
    }
    let generator = args["generator"].as_str();
    let source = if generator.is_some_and(|g| !g.trim().is_empty()) {
        crate::models::Source::Generated
    } else {
        crate::models::Source::Shop
    };
    let bytes = match crate::models::keep(&app.db, &row.id, bytes, source, generator).await? {
        Ok(bytes) => bytes,
        Err(why) => return Ok(refused(format!("{url}: {why}"))),
    };
    let drawn = crate::staging::swap(app, account, &plan, &row.id, &bytes).await?;
    let mut answer = shown(&app.db, row).await?;
    answer["drawn"] = json!(drawn);
    Ok(said(&answer))
}

/// What the tools say when this server has no 3D generator.
pub const NO_GENERATOR: &str = "3D generation is not configured on this server (it has no NEWERA_GENERATOR_URL): edit_products action=attach_model gives the product a model";

/// Has the product's model made from its photo (`crate::generation`): a
/// job, or the one already open; `products id=` shows how it goes.
async fn generate(app: &AppState, account: &str, row: &Row) -> anyhow::Result<Value> {
    if app.config.generator.is_none() {
        return Ok(refused(NO_GENERATOR));
    }
    if row.kind() != Kind::Furniture {
        return Ok(refused(format!(
            "{} is a finish: a 3D model is for a piece of furniture",
            row.id
        )));
    }
    if crate::generation::photo(&row.fields).is_none() {
        return Ok(refused(format!(
            "{} has no photo to make a model from: edit_products action=set fields image (an https link to it) first",
            row.id
        )));
    }
    if crate::generation::size(&row.fields).is_none() {
        return Ok(refused(format!(
            "{} has no width, depth and height to make its model at: edit_products action=set fields width_cm, depth_cm, height_cm first",
            row.id
        )));
    }
    crate::generation::enqueue(app, account, &row.id).await?;
    Ok(said(&shown(&app.db, row).await?))
}

/// Fills or corrects fields as the agent's; `kind` changes what the
/// product is.
async fn set(db: &PgPool, row: Row, args: &Value) -> anyhow::Result<Value> {
    let Some(given) = args["fields"].as_object().filter(|f| !f.is_empty()) else {
        return Ok(refused(
            "edit_products action=set: fields is an object of field → value, e.g. {\"price\": \"24.54\"}",
        ));
    };
    let mut fields = row.fields.0.clone();
    let mut kind = row.kind();
    for (name, value) in given {
        if name == "kind" {
            match value.as_str().and_then(Kind::parse) {
                Some(k) => kind = k,
                None => return Ok(refused(format!("kind is furniture or finish, not {value}"))),
            }
            continue;
        }
        let value = match product::checked(name, value) {
            Ok(value) => value,
            Err(why) => return Ok(refused(why)),
        };
        fields.insert(
            name.clone(),
            Field {
                value,
                evidence: Evidence::Agent,
                from: "edit_products".to_owned(),
            },
        );
    }
    sqlx::query("update products set fields = $2, kind = $3, updated_at = now() where id = $1")
        .bind(&row.id)
        .bind(Json(&fields))
        .bind(kind.as_str())
        .execute(db)
        .await?;
    if given.contains_key("price") {
        remember_price(db, &row.id, &fields).await?;
    }
    let row = Row {
        fields: Json(fields),
        kind: kind.as_str().to_owned(),
        ..row
    };
    Ok(said(&shown(db, &row).await?))
}

/// What a refresh takes from the shop: the commercial fields. The rest of
/// the product — title, size, pack, whatever the agent corrected — is not
/// the day's price and stays. The sale unit goes with the price it
/// qualifies.
const COMMERCIAL: &[&str] = &[
    "price",
    "member_price",
    "currency",
    "vat_included",
    "unit_price",
    "unit_price_per",
    "sale_unit",
];

/// Reads the shop again for the day's price. What the shop shows of
/// [`COMMERCIAL`] replaces what was there — a read that found the price
/// replaces the whole set — and joins the price history; nothing else
/// changes. A read that found no price leaves
/// `fetched_at` and the earlier fetches as they were and adds the attempt
/// to them.
async fn refresh(app: &AppState, account: &str, row: Row) -> anyhow::Result<Value> {
    if app.config.anysite.is_none() {
        return Ok(refused(NOT_CONFIGURED));
    }
    let key = format!("refresh\0{}", row.id);
    let work = fetch_and_renew(app.clone(), account.to_owned(), row);
    let (_, done) = app.flights.run(key, work).await;
    flown(done)
}

async fn fetch_and_renew(
    app: AppState,
    account: String,
    row: Row,
) -> anyhow::Result<Result<Value, String>> {
    let Some(anysite) = app.config.anysite.clone() else {
        return Ok(Err(NOT_CONFIGURED.to_owned()));
    };
    let identity = match newera_shop::identify(&row.url) {
        Ok(identity) => identity,
        Err(why) => return Ok(Err(why)),
    };
    let got = match newera_shop::import::fetch(&anysite, &identity, Some(row.kind())).await {
        Ok(got) => got,
        Err(err) => return Ok(Err(not_imported(&err, "the product was not refreshed"))),
    };
    // The product as it is now: the agent may have changed it while the
    // shop was being read.
    let Some(row) = by_id(&app.db, &account, &row.id).await? else {
        return Ok(Err(format!("product {} was removed meanwhile", row.id)));
    };
    let mut fields = row.fields.0.clone();
    // A read that found the price is the shop's whole word on it: what it
    // no longer shows (an offer that ended, a price per unit) is unset.
    let priced = got.fields.contains_key("price");
    if priced {
        for name in COMMERCIAL {
            match got.fields.get(*name) {
                Some(field) => {
                    fields.insert((*name).to_owned(), field.clone());
                }
                None => {
                    fields.remove(*name);
                }
            }
        }
    }
    if priced {
        sqlx::query(
            "update products set fields = $2, fetches = $3, fetched_at = now(), updated_at = now()
             where id = $1",
        )
        .bind(&row.id)
        .bind(Json(&fields))
        .bind(Json(fetched(&got.fetches)))
        .execute(&app.db)
        .await?;
        remember_price(&app.db, &row.id, &fields).await?;
    } else {
        sqlx::query("update products set fetches = fetches || $2 where id = $1")
            .bind(&row.id)
            .bind(Json(fetched(&got.fetches)))
            .execute(&app.db)
            .await?;
    }
    let Some(row) = by_id(&app.db, &account, &row.id).await? else {
        anyhow::bail!("the product just refreshed is gone");
    };
    let mut answer = shown(&app.db, &row).await?;
    answer["fetched"] = fetched(&got.fetches);
    Ok(Ok(answer))
}
