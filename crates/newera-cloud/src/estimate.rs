//! What the account's project costs: `estimate` and `edit_estimate`,
//! answered here, never by the editor.
//!
//! The quantities are the document's: `takeoff`, measured where the
//! document is. While the account's editor tab holds the room and shows the
//! active project, the tab is asked for it the way any tool call is
//! relayed — it answers from what is on its screen; otherwise the engine
//! measures the active project as stored, and says why ([`takeoff`]). The prices and
//! packs are the library's (`products`), the lines added by hand and the
//! saved estimates are the project's (`estimate_extra`, `estimates`), and
//! the arithmetic is `newera_shop::estimate`'s.
//!
//! The project is the account's active one, and every estimate — read or
//! saved — is of that project's document, never of another a tab shows.

use std::collections::BTreeMap;

use newera_shop::decimal::Decimal;
use newera_shop::estimate::{self, Extra, GroupBy, Line, Product};
use newera_shop::{Fields, Kind};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::types::Json;

use crate::{AppState, secret};

/// Whether the tool is one of the estimate's.
pub fn answers(tool: &str) -> bool {
    matches!(tool, "estimate" | "edit_estimate")
}

fn said(answer: &Value) -> Value {
    json!({"content": [{"type": "text", "text": answer.to_string()}], "isError": false})
}

fn refused(why: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": why.into()}], "isError": true})
}

/// Runs one of the estimate's tools for an account.
///
/// # Errors
///
/// When the database or the engine fails; a call the tool refuses answers
/// `Ok` with `isError`.
pub async fn call(
    app: &AppState,
    account: &str,
    tool: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    match tool {
        "estimate" => read(app, account, args).await,
        "edit_estimate" => edit(app, account, args).await,
        other => Ok(refused(format!("no tool named {other}"))),
    }
}

fn group_by(args: &Value) -> Result<GroupBy, String> {
    match args.get("group_by").and_then(Value::as_str) {
        None => Ok(GroupBy::Shop),
        Some(g) => {
            GroupBy::parse(g).ok_or_else(|| format!("group_by is shop, room or kind, not {g}"))
        }
    }
}

/// The account's active project: `(id, name)`.
async fn active(db: &PgPool, account: &str) -> sqlx::Result<Option<(String, String)>> {
    sqlx::query_as(
        "select p.id, p.name from active_projects a join projects p on p.id = a.project_id
         where a.account_id = $1",
    )
    .bind(account)
    .fetch_optional(db)
    .await
}

/// What the project buys, measured, and where.
struct Measured {
    takeoff: estimate::Takeoff,
    /// `tab` or `server`.
    from: &'static str,
    /// The project the tab measured: the active one, when it was asked.
    project: Option<String>,
    /// What the agent should know about the copy that was measured.
    note: Option<String>,
}

/// What the project buys, measured where the document is.
///
/// While a tab holds the room it is first asked which project it shows,
/// saving what it has (`flush`, as before a render). When that is the
/// active project, the tab measures it — what is on screen. Otherwise the
/// tab's quantities are of another document than the one the estimate is
/// filed under: the engine measures the active project as stored, and the
/// note says why the tab's was not used. `Err` with the reason in words
/// when the project could not be measured.
async fn takeoff(app: &AppState, account: &str) -> anyhow::Result<Result<Measured, String>> {
    let mut note = None;
    if let Some(room) = app.rooms.owned_by(account) {
        let answered = newera_relay::flush_in(&app.rooms, &room, crate::mcp::FLUSH_WAIT).await;
        let active = active(&app.db, account).await?;
        match answered {
            Ok(saved) => match saved["project"].as_str() {
                Some(shown) if active.as_ref().is_some_and(|(id, _)| id == shown) => {
                    let message = json!({
                        "jsonrpc": "2.0",
                        "id": format!("estimate-{}", secret::id()),
                        "method": "tools/call",
                        "params": {"name": "takeoff", "arguments": {}},
                    });
                    let answered = newera_relay::answer_in(&app.rooms, &room, &message).await;
                    let result = answered.and_then(|mut a| a.get_mut("result").map(Value::take));
                    return Ok(read_answer(&result.unwrap_or(Value::Null)).map(|takeoff| {
                        Measured {
                            takeoff,
                            from: "tab",
                            project: Some(shown.to_owned()),
                            note: None,
                        }
                    }));
                }
                Some(shown) => {
                    let name: Option<String> = sqlx::query_scalar(
                        "select name from projects where id = $1 and account_id = $2",
                    )
                    .bind(shown)
                    .bind(account)
                    .fetch_optional(&app.db)
                    .await?;
                    let shown = name.map_or_else(
                        || format!("a project this account does not have ({shown})"),
                        |name| format!("another project, {name}"),
                    );
                    note = Some(format!(
                        "This estimate is of the active project as last saved; the editor tab shows {shown}, which is not priced here. Reloading the editor tab makes the project it shows the active one; then ask again."
                    ));
                }
                None => {
                    note = Some(
                        "This estimate is of the active project as last saved; the editor tab shows a project that is not saved on the server, which is not priced here."
                            .to_owned(),
                    );
                }
            },
            Err(why) => {
                tracing::warn!("flush before an estimate: {why}");
                note = Some(format!(
                    "The editor tab did not say which project it shows ({why}): this estimate is of the active project as last saved, and edits made in the tab since may be missing."
                ));
            }
        }
    }
    let plan = crate::accounts::plan_of(&app.db, account).await?;
    let answer = app
        .engine
        .call(
            &app.db,
            &app.config.public_url,
            &app.config.editor_url,
            account,
            &plan,
            "takeoff",
            json!({}),
        )
        .await?;
    Ok(read_answer(&answer).map(|takeoff| Measured {
        takeoff,
        from: "server",
        project: None,
        note,
    }))
}

/// A `takeoff` tool answer, as what the project buys.
fn read_answer(answer: &Value) -> Result<estimate::Takeoff, String> {
    let text = answer["content"][0]["text"].as_str().unwrap_or_default();
    if answer["isError"] == true || text.is_empty() {
        let why = if text.is_empty() { "no answer" } else { text };
        return Err(format!(
            "the project's quantities could not be measured ({why})"
        ));
    }
    let parsed: Value = serde_json::from_str(text)
        .map_err(|err| format!("the takeoff answered something else: {err}"))?;
    estimate::read_takeoff(&parsed)
}

/// The account's products the takeoff names, by id.
async fn products(
    db: &PgPool,
    account: &str,
    takeoff: &estimate::Takeoff,
) -> sqlx::Result<BTreeMap<String, Product>> {
    let ids: Vec<String> = takeoff
        .finishes
        .iter()
        .map(|c| c.product.clone())
        .chain(takeoff.furniture.iter().map(|b| b.product.clone()))
        .collect();
    let rows: Vec<(String, String, String, String, Json<Fields>)> = sqlx::query_as(
        "select id, shop, url, kind, fields from products where account_id = $1 and id = any($2)",
    )
    .bind(account)
    .bind(&ids)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, shop, url, kind, fields)| {
            let product = Product {
                id: id.clone(),
                shop,
                url,
                kind: Kind::parse(&kind).unwrap_or(Kind::Finish),
                fields: fields.0,
            };
            (id, product)
        })
        .collect())
}

/// The lines added by hand to the project, in the order they were added.
async fn extras(db: &PgPool, account: &str, project: &str) -> anyhow::Result<Vec<Line>> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        Option<bool>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select id, title, shop, quantity::text, unit, unit_price::text, currency, vat_included, note
         from estimate_extra where account_id = $1 and project_id = $2
         order by created_at, id",
    )
    .bind(account)
    .bind(project)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(
            |(id, title, shop, quantity, unit, price, currency, vat_included, note)| {
                let amount = |text: &str| {
                    Decimal::parse(text)
                        .ok_or_else(|| anyhow::anyhow!("the database holds {text} as an amount"))
                };
                Ok(estimate::extra_line(&Extra {
                    id,
                    title,
                    shop,
                    quantity: amount(&quantity)?,
                    unit,
                    unit_price: amount(&price)?,
                    currency,
                    vat_included,
                    note,
                }))
            },
        )
        .collect()
}

/// The project's saved estimates: `[id, name, saved]`, newest first.
async fn saved_list(db: &PgPool, account: &str, project: &str) -> sqlx::Result<Vec<Value>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "select id, name, extract(epoch from created_at)::bigint from estimates
         where account_id = $1 and project_id = $2 order by created_at desc, id",
    )
    .bind(account)
    .bind(project)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, at)| json!([id, name, at]))
        .collect())
}

/// The estimate of the project as it is now: its lines, and where its
/// quantities were measured.
struct Live {
    project: (String, String),
    lines: Vec<Line>,
    gone: Vec<Value>,
    from: &'static str,
    note: Option<String>,
}

async fn live(app: &AppState, account: &str) -> anyhow::Result<Result<Live, String>> {
    let measured = match takeoff(app, account).await? {
        Ok(measured) => measured,
        Err(why) => return Ok(Err(why)),
    };
    // Measured first: the engine makes the account its first project.
    let Some(project) = active(&app.db, account).await? else {
        return Ok(Err(
            "the account has no project on the server yet: file action=new starts one".to_owned(),
        ));
    };
    // The lines by hand and a saved copy are the project's the quantities
    // are of, never another's.
    if measured.project.as_ref().is_some_and(|p| *p != project.0) {
        return Ok(Err(
            "the active project changed while the estimate was being measured: call again"
                .to_owned(),
        ));
    }
    let library = products(&app.db, account, &measured.takeoff).await?;
    let mut lines = estimate::lines(&measured.takeoff, &library);
    lines.extend(extras(&app.db, account, &project.0).await?);
    Ok(Ok(Live {
        project,
        lines,
        gone: measured.takeoff.gone,
        from: measured.from,
        note: measured.note,
    }))
}

/// A saved estimate of the account, by its id, or by its name among the
/// active project's.
async fn find_saved(
    db: &PgPool,
    account: &str,
    wanted: &str,
) -> sqlx::Result<Option<(String, String, String, String, i64, Json<Value>)>> {
    sqlx::query_as(
        "select id, name, project_id, project_name, extract(epoch from created_at)::bigint, body
         from estimates e
         where account_id = $1
           and (id = $2 or (lower(name) = lower($2) and project_id =
                (select project_id from active_projects where account_id = $1)))
         order by (id = $2) desc limit 1",
    )
    .bind(account)
    .bind(wanted)
    .fetch_optional(db)
    .await
}

async fn read(app: &AppState, account: &str, args: &Value) -> anyhow::Result<Value> {
    let group_by = match group_by(args) {
        Ok(g) => g,
        Err(why) => return Ok(refused(why)),
    };
    let csv = match args.get("format").and_then(Value::as_str) {
        None => false,
        Some("csv") => true,
        Some(other) => return Ok(refused(format!("format is csv, not {other}"))),
    };
    let mut answer = if let Some(wanted) = args.get("saved").and_then(Value::as_str) {
        let Some((id, name, project, project_name, at, body)) =
            find_saved(&app.db, account, wanted).await?
        else {
            return Ok(refused(format!(
                "no saved estimate {wanted} (estimate lists the project's saved ones under saved)"
            )));
        };
        let lines: Vec<Line> = serde_json::from_value(body["lines"].clone())?;
        let mut answer = estimate::estimate(&lines, group_by);
        // The totals as they were kept: a saved estimate is not computed
        // again.
        for key in ["totals", "by_currency", "complete", "gone", "from", "note"] {
            if let Some(kept) = body.get(key) {
                answer[key] = kept.clone();
            }
        }
        answer["project"] = json!({"id": project, "name": project_name});
        answer["saved_as"] = json!({"id": id, "name": name, "saved": at, "project": project});
        answer
    } else {
        let live = match live(app, account).await? {
            Ok(live) => live,
            Err(why) => return Ok(refused(why)),
        };
        let mut answer = estimate::estimate(&live.lines, group_by);
        answer["project"] = json!({"id": live.project.0, "name": live.project.1});
        answer["from"] = json!(live.from);
        if !live.gone.is_empty() {
            answer["gone"] = json!(live.gone);
        }
        answer["saved"] = json!(saved_list(&app.db, account, &live.project.0).await?);
        if let Some(note) = live.note {
            answer["note"] = json!(note);
        }
        answer
    };
    if csv {
        let name = format!(
            "{} estimate.csv",
            answer["project"]["name"].as_str().unwrap_or("project")
        );
        let sheet = estimate::csv(&answer).into_bytes();
        let url = crate::engine::keep_file(
            &app.db,
            &app.config.public_url,
            account,
            &name,
            "csv",
            sheet,
        )
        .await?;
        let mut short = json!({
            "csv": url,
            "project": answer["project"],
            "by_currency": answer["by_currency"],
            "complete": answer["complete"],
        });
        if let Some(saved) = answer.get("saved_as") {
            short["saved_as"] = saved.clone();
        }
        answer = short;
    }
    Ok(said(&answer))
}

async fn edit(app: &AppState, account: &str, args: &Value) -> anyhow::Result<Value> {
    let action = args["action"].as_str().unwrap_or_default();
    if !matches!(action, "add_line" | "remove_line" | "save" | "remove_saved") {
        return Ok(refused(
            "edit_estimate: action is add_line, remove_line, save or remove_saved",
        ));
    }
    match action {
        "save" => return save(app, account, args).await,
        "remove_saved" => return remove_saved(&app.db, account, args).await,
        _ => {}
    }
    let Some((project, _)) = active(&app.db, account).await? else {
        return Ok(refused(
            "the account has no project on the server yet: file action=new starts one",
        ));
    };
    if action == "remove_line" {
        let Some(line) = args["line"].as_str() else {
            return Ok(refused(
                "edit_estimate action=remove_line: line is the line's id, as estimate lists it",
            ));
        };
        let removed = sqlx::query(
            "delete from estimate_extra where id = $1 and account_id = $2 and project_id = $3",
        )
        .bind(line)
        .bind(account)
        .bind(&project)
        .execute(&app.db)
        .await?
        .rows_affected();
        return Ok(if removed == 0 {
            refused(format!(
                "no line {line} added to this project's estimate (estimate lists them, with their line ids)"
            ))
        } else {
            said(&json!({"removed": true, "id": line}))
        });
    }
    add_line(&app.db, account, &project, args).await
}

/// A decimal as an agent sends one: a JSON number, written as its text, or
/// a string.
fn decimal_of(value: &Value) -> Option<Decimal> {
    match value {
        Value::Number(n) => Decimal::parse(&n.to_string()),
        Value::String(s) => Decimal::parse(s),
        _ => None,
    }
}

async fn add_line(
    db: &PgPool,
    account: &str,
    project: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let text = |name: &str| {
        args.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let Some(title) = text("title") else {
        return Ok(refused(
            "edit_estimate action=add_line: title says what the line is, e.g. \"Tiling labour\"",
        ));
    };
    let vat_included = match args.get("vat_included") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(other) => {
            return Ok(refused(format!(
                "edit_estimate action=add_line: vat_included is true or false, not {other}"
            )));
        }
    };
    let Some(price) = args
        .get("unit_price")
        .and_then(decimal_of)
        .filter(|d| d.parts().0 >= 0)
    else {
        return Ok(refused(
            "edit_estimate action=add_line: unit_price is the price of one, an amount of 0 or more like \"45.00\"",
        ));
    };
    let quantity = match args.get("quantity") {
        None | Some(Value::Null) => Decimal::new(1, 0),
        Some(given) => match decimal_of(given).filter(|d| d.is_positive()) {
            Some(q) => q,
            None => {
                return Ok(refused(format!(
                    "edit_estimate action=add_line: quantity is a number above zero, not {given}"
                )));
            }
        },
    };
    let currency = text("currency").map_or_else(|| "EUR".to_owned(), |c| c.to_uppercase());
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Ok(refused(format!(
            "edit_estimate action=add_line: currency is a three-letter code like EUR, not {currency}"
        )));
    }
    let extra = Extra {
        id: secret::id(),
        title,
        shop: text("shop"),
        quantity,
        unit: text("unit").unwrap_or_else(|| "item".to_owned()),
        unit_price: price,
        currency,
        vat_included,
        note: text("note"),
    };
    sqlx::query(
        "insert into estimate_extra
           (id, account_id, project_id, title, shop, quantity, unit, unit_price, currency,
            vat_included, note)
         values ($1, $2, $3, $4, $5, $6::numeric, $7, $8::numeric, $9, $10, $11)",
    )
    .bind(&extra.id)
    .bind(account)
    .bind(project)
    .bind(&extra.title)
    .bind(&extra.shop)
    .bind(extra.quantity.to_string())
    .bind(&extra.unit)
    .bind(extra.unit_price.to_string())
    .bind(&extra.currency)
    .bind(extra.vat_included)
    .bind(&extra.note)
    .execute(db)
    .await?;
    Ok(said(&json!({"line": estimate::extra_line(&extra)})))
}

async fn save(app: &AppState, account: &str, args: &Value) -> anyhow::Result<Value> {
    let live = match live(app, account).await? {
        Ok(live) => live,
        Err(why) => return Ok(refused(why)),
    };
    let summed = estimate::estimate(&live.lines, GroupBy::Shop);
    let mut body = json!({
        "lines": live.lines,
        "totals": summed["totals"],
        "by_currency": summed["by_currency"],
        "complete": summed["complete"],
        "from": live.from,
    });
    if !live.gone.is_empty() {
        body["gone"] = json!(live.gone);
    }
    if let Some(note) = &live.note {
        body["note"] = json!(note);
    }
    let given = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(120).collect::<String>());
    let id = secret::id();
    let kept: Option<(String, i64)> = sqlx::query_as(
        "insert into estimates (id, account_id, project_id, project_name, name, body)
         values ($1, $2, $3, $4, coalesce($5, to_char(now(), 'YYYY-MM-DD HH24:MI:SS')), $6)
         on conflict (project_id, lower(name)) do nothing
         returning name, extract(epoch from created_at)::bigint",
    )
    .bind(&id)
    .bind(account)
    .bind(&live.project.0)
    .bind(&live.project.1)
    .bind(&given)
    .bind(Json(&body))
    .fetch_optional(&app.db)
    .await?;
    let Some((name, at)) = kept else {
        let named = given.map_or_else(|| "of this second".to_owned(), |n| format!("named {n}"));
        return Ok(refused(format!(
            "this project already has a saved estimate {named}: a saved estimate never changes — choose another name, or remove_saved that one first"
        )));
    };
    let mut answer = json!({
        "saved": {"id": id, "name": name, "saved": at, "project": live.project.0},
        "by_currency": summed["by_currency"],
        "complete": summed["complete"],
    });
    if let Some(note) = live.note {
        answer["note"] = json!(note);
    }
    Ok(said(&answer))
}

async fn remove_saved(db: &PgPool, account: &str, args: &Value) -> anyhow::Result<Value> {
    let Some(wanted) = args["saved"].as_str() else {
        return Ok(refused(
            "edit_estimate action=remove_saved: saved is the saved estimate's name or id",
        ));
    };
    let Some((id, name, project, _, at, _)) = find_saved(db, account, wanted).await? else {
        return Ok(refused(format!(
            "no saved estimate {wanted} (estimate lists the project's saved ones under saved)"
        )));
    };
    sqlx::query("delete from estimates where id = $1 and account_id = $2")
        .bind(&id)
        .bind(account)
        .execute(db)
        .await?;
    Ok(said(&json!({
        "removed": true,
        "saved": {"id": id, "name": name, "saved": at, "project": project},
    })))
}
