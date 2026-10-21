//! The fixed MCP address, `/mcp`: the same for everyone, and each AI client
//! acts for the account its token belongs to.
//!
//! A call goes to that account's tab — the editor at 3dneweraai.com/app,
//! signed in — through the relay; renders run on the server all the same,
//! after the tab has saved. The handshake, the tool list and the pages are
//! answered here, so a client can connect before the tab is open.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::{AppState, oauth};

/// Protocol versions this endpoint speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// The 401 that sends a client to the metadata, and from there to sign-in.
fn unauthorized(app: &AppState) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(
            json!({"error": "invalid_token", "error_description": "sign in to use 3D New Era AI"}),
        ),
    )
        .into_response();
    let challenge = format!(
        r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource/mcp", scope="{}""#,
        app.config.public_url,
        oauth::SCOPE
    );
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_str(&challenge).expect("ASCII"),
    );
    response
}

/// `POST /mcp`: one JSON-RPC message, or a batch.
pub async fn post(State(app): State<AppState>, headers: HeaderMap, body: String) -> Response {
    let Some(account) = oauth::bearer(&app, &headers).await else {
        return unauthorized(&app);
    };
    if !app.limits.allow(
        &format!("mcp:{}", account.id),
        2400,
        Duration::from_secs(3600),
    ) {
        return (StatusCode::TOO_MANY_REQUESTS, "too many calls this hour").into_response();
    }
    let Ok(parsed) = serde_json::from_str::<Value>(&body) else {
        return Json(
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "not JSON"}}),
        )
        .into_response();
    };
    match parsed {
        Value::Array(batch) => {
            let mut answers = Vec::new();
            for message in &batch {
                if let Some(answer) = answer(&app, &account, message).await {
                    answers.push(answer);
                }
            }
            if answers.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(Value::Array(answers)).into_response()
            }
        }
        message => match answer(&app, &account, &message).await {
            Some(answer) => Json(answer).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

/// No server-to-client stream here; clients carry on over POST.
pub async fn get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

fn result(id: &Value, result: Value) -> Value {
    let mut answer = json!({"jsonrpc": "2.0", "id": id});
    answer["result"] = result;
    answer
}

fn failure(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The tools a client sees: the editor's, minus what is only for a desktop,
/// with the file tools saying what they do here, and the service's own —
/// the projects and the product library.
fn hosted_tools() -> Vec<Value> {
    // What the service offers and how it words it is the MCP crate's to say.
    let mut tools = newera_mcp::surface::tools(newera_mcp::Transport::Cloud);
    // Every tool here works on the account: say so per tool, the way the
    // OpenAI Apps SDK reads it, so ChatGPT signs the person in first.
    for tool in &mut tools {
        tool["securitySchemes"] = json!([{"type": "oauth2", "scopes": [oauth::SCOPE]}]);
    }
    tools
}

async fn answer(
    app: &AppState,
    account: &crate::accounts::Account,
    message: &Value,
) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let notification = message.get("id").is_none();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("");
            let version = VERSIONS
                .iter()
                .find(|v| **v == asked)
                .unwrap_or(&VERSIONS[1]);
            Some(result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {
                        "name": "3d-new-era-ai",
                        "title": "3D New Era AI",
                        "version": env!("CARGO_PKG_VERSION"),
                        "description": newera_mcp::output::DESCRIPTION,
                        "websiteUrl": newera_mcp::output::WEBSITE,
                        "icons": newera_mcp::output::icons(),
                    },
                    "instructions": newera_mcp::surface::instructions(newera_mcp::Transport::Cloud),
                }),
            ))
        }
        _ if notification => None,
        "ping" => Some(result(&id, json!({}))),
        "tools/list" => Some(result(&id, json!({ "tools": hosted_tools() }))),
        "resources/list" => Some(result(&id, newera_mcp::app::resource_list())),
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(match newera_mcp::app::read(uri) {
                Some(found) => result(&id, found),
                None => failure(&id, -32002, &format!("no resource {uri}")),
            })
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !newera_mcp::Transport::Cloud.offers(name) {
                return Some(result(
                    &id,
                    json!({"content": [{"type": "text", "text": format!("{name} runs only in the desktop app")}], "isError": true}),
                ));
            }
            // The service's own tools — the projects, the product library,
            // the estimate — are answered here, from the database: never by
            // the tab, and never by the editor's tools.
            let hosted = newera_mcp::surface::HOSTED_ONLY.contains(&name);
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            if hosted && let Some(why) = newera_mcp::surface::unknown_hosted_argument(name, &args) {
                return Some(result(
                    &id,
                    json!({"content": [{"type": "text", "text": why}], "isError": true}),
                ));
            }
            if crate::products::answers(name) || crate::estimate::answers(name) {
                let answered = if crate::estimate::answers(name) {
                    crate::estimate::call(app, &account.id, name, &args).await
                } else {
                    crate::products::call(app, &account.id, name, &args).await
                };
                return Some(match answered {
                    Ok(mut answer) => {
                        newera_mcp::output::structure_json(&mut answer);
                        result(&id, answer)
                    }
                    Err(err) => {
                        tracing::error!("{name}: {err:#}");
                        result(
                            &id,
                            json!({"content": [{"type": "text", "text": format!("{name} failed on the server: {err}")}], "isError": true}),
                        )
                    }
                });
            }
            // Shop products the call names are made ready first: checked
            // against the account's library, their files fetched, the call
            // rewritten to what the editor reads (`crate::staging`).
            let (args, files, notes) = match crate::staging::stage(
                &app.db,
                &account.id,
                name,
                &args,
            )
            .await
            {
                Ok(Ok(Some(staged))) => (staged.args, staged.files, staged.notes),
                Ok(Ok(None)) => (args, Vec::new(), Vec::new()),
                Ok(Err(why)) => {
                    return Some(result(
                        &id,
                        json!({"content": [{"type": "text", "text": why}], "isError": true}),
                    ));
                }
                Err(err) => {
                    tracing::error!("staging {name}: {err:#}");
                    return Some(result(
                        &id,
                        json!({"content": [{"type": "text", "text": format!("{name} failed on the server: {err}")}], "isError": true}),
                    ));
                }
            };
            // The person's own editor, when it is open and signed in; the
            // cloud project otherwise. A render always runs here, on the
            // server: the tab saves what it has first.
            let render = RENDERS.contains(&name);
            let room = if hosted {
                None
            } else {
                app.rooms.owned_by(&account.id)
            };
            if !render && let Some(room) = room {
                // The files first, so the call that uses them finds them.
                for (path, bytes) in &files {
                    if let Err(why) = newera_relay::stage_in(
                        &app.rooms,
                        &room,
                        path,
                        bytes,
                        crate::staging::STAGE_WAIT,
                    )
                    .await
                    {
                        return Some(result(
                            &id,
                            json!({"content": [{"type": "text", "text": format!("the editor tab did not take the file {path} ({why}): reload the tab and call again")}], "isError": true}),
                        ));
                    }
                }
                let mut sent = message.clone();
                sent["params"]["arguments"] = args;
                // The tab may run an editor older than this service: its
                // answer gets its fields here, as the engine's do.
                let mut answer = newera_relay::answer_in(&app.rooms, &room, &sent).await;
                if let Some(result) = answer.as_mut().and_then(|a| a.get_mut("result")) {
                    newera_mcp::output::structure_json(result);
                    add_notes(result, &notes);
                }
                return answer;
            }
            let note = match room {
                Some(room) if render => flush(app, &account.id, &room).await,
                _ => None,
            };
            let plan = match crate::accounts::plan_of(&app.db, &account.id).await {
                Ok(plan) => plan,
                Err(err) => {
                    tracing::error!("plan: {err}");
                    return Some(failure(
                        &id,
                        -32603,
                        "the service could not read the account's plan",
                    ));
                }
            };
            match app
                .engine
                .call_with(
                    &app.db,
                    &app.config.public_url,
                    &app.config.editor_url,
                    &account.id,
                    &plan,
                    name,
                    args,
                    &files,
                )
                .await
            {
                Ok(mut answer) => {
                    newera_mcp::output::structure_json(&mut answer);
                    add_notes(&mut answer, &notes);
                    if let Some(note) = note {
                        add_note(&mut answer, &note);
                    }
                    Some(result(&id, answer))
                }
                Err(err) => {
                    tracing::error!("engine {name}: {err:#}");
                    Some(result(
                        &id,
                        json!({"content": [{"type": "text", "text": format!("{name} failed on the server: {err}")}], "isError": true}),
                    ))
                }
            }
        }
        other => Some(failure(
            &id,
            -32601,
            &format!("this server does not do {other}"),
        )),
    }
}

/// The tools that render: always run by the engine, never by a tab.
const RENDERS: [&str; 3] = ["render_plan", "render_3d", "render_photo"];

/// How long a render or an estimate waits for the tab to save what it has: one upload of
/// the project, started the moment the tab is asked. A chosen bound, not a
/// measured upload time; past it the render goes ahead on what is stored.
pub(crate) const FLUSH_WAIT: Duration = Duration::from_secs(10);

/// Before a render, while the account's tab holds the room: the tab saves
/// its pending change and says at which revision the project is kept, so the
/// engine renders what is on the person's screen.
///
/// The render goes ahead in every case, on what is stored; what this answers
/// is the note the agent gets with it when the stored project may not be the
/// tab's:
///
/// - the tab answers with the active project and a revision: the engine's
///   copy is made at least that revision (`Engine::forget`), no note;
/// - the tab is bound to another project, or to none (a project only in the
///   browser): the render is of the active project, and the note names both
///   and what the person can do (`elsewhere`);
/// - no answer within `FLUSH_WAIT` (an editor older than the request, a tab
///   asleep, a slow upload), or the tab could not save: the stored copy is
///   rendered, and the note says the tab's latest edits may be missing.
async fn flush(app: &AppState, account: &str, room: &str) -> Option<String> {
    let answered = newera_relay::flush_in(&app.rooms, room, FLUSH_WAIT).await;
    let active: Option<String> =
        match sqlx::query_scalar("select project_id from active_projects where account_id = $1")
            .bind(account)
            .fetch_optional(&app.db)
            .await
        {
            Ok(active) => active,
            Err(err) => {
                tracing::error!("active project: {err}");
                None
            }
        };
    let saved = match answered {
        Ok(saved) => saved,
        Err(why) => {
            tracing::warn!("flush before a render: {why}");
            return Some(format!(
                "The editor tab did not confirm it saved its latest edits ({why}): this is the project as last saved, and edits made in the tab since may be missing."
            ));
        }
    };
    let Some(project) = saved["project"].as_str() else {
        return Some(
            "The editor tab shows a project that is not saved on the server: this is the active server project, not what the tab shows."
                .to_owned(),
        );
    };
    if active.as_deref() != Some(project) {
        return Some(elsewhere(app, account, active.as_deref(), project).await);
    }
    let Some(revision) = saved["revision"].as_i64() else {
        return Some(
            "The editor tab did not say which revision it saved: this is the project as last saved, and edits made in the tab since may be missing."
                .to_owned(),
        );
    };
    app.engine.forget(account, project, revision).await;
    None
}

/// The note for a render of the active project while the tab shows
/// `shown`. A tab makes the project it shows the active one when it binds
/// it and when it claims the room, so this is a tab that has not managed to
/// yet: reloading it does both again.
async fn elsewhere(app: &AppState, account: &str, active: Option<&str>, shown: &str) -> String {
    let name = |id: Option<&str>| {
        let id = id.map(str::to_owned);
        async move {
            let id = id?;
            sqlx::query_scalar::<_, String>(
                "select name from projects where id = $1 and account_id = $2",
            )
            .bind(&id)
            .bind(account)
            .fetch_optional(&app.db)
            .await
            .unwrap_or_else(|err| {
                tracing::error!("project name: {err}");
                None
            })
        }
    };
    let active = name(active)
        .await
        .unwrap_or_else(|| "the account's active project".to_owned());
    match name(Some(shown)).await {
        Some(shown) => format!(
            "This render is of the active project, {active}; the editor tab shows another project, {shown}, and its edits are not in it. Reloading the editor tab makes {shown} the active project; then ask again."
        ),
        None => format!(
            "This render is of the active project, {active}; the editor tab shows a project this account does not have ({shown}), and its edits are not in it."
        ),
    }
}

/// The notes staging left for an answer that did what was asked.
fn add_notes(answer: &mut Value, notes: &[String]) {
    if answer["isError"] != true {
        for note in notes {
            add_note(answer, note);
        }
    }
}

/// Tells the agent something about an answer it got, beside it: a line of
/// text after the content, and `note` among its fields — the shape of the
/// answer stays what it was.
fn add_note(answer: &mut Value, note: &str) {
    if let Some(content) = answer.get_mut("content").and_then(Value::as_array_mut) {
        content.push(json!({"type": "text", "text": note}));
    }
    if let Some(fields) = answer
        .get_mut("structuredContent")
        .and_then(Value::as_object_mut)
    {
        fields.insert("note".to_owned(), json!(note));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instructions name every tool the service lists, its own
    /// included: a client that defers definitions shows the model only the
    /// names and these words.
    #[test]
    fn the_instructions_name_every_hosted_tool() {
        let said = newera_mcp::surface::instructions(newera_mcp::Transport::Cloud);
        for tool in hosted_tools() {
            let name = tool["name"].as_str().unwrap();
            assert!(
                said.contains(&format!(" {name}")),
                "the instructions leave out {name}"
            );
        }
    }

    /// No hosted tool works without the account, and each one says so.
    #[test]
    fn every_hosted_tool_asks_for_the_account() {
        let tools = hosted_tools();
        for name in newera_mcp::surface::HOSTED_ONLY {
            assert!(tools.iter().any(|t| t["name"] == name), "{name}");
        }
        for tool in &tools {
            assert_eq!(
                tool["securitySchemes"],
                json!([{"type": "oauth2", "scopes": ["newera"]}]),
                "{}",
                tool["name"]
            );
        }
    }
}
