//! The fixed MCP address, `/mcp`: the same for everyone, and each AI client
//! acts for the account its token belongs to.
//!
//! A call goes to that account's tab — the editor at 3dneweraai.com/app,
//! signed in — through the relay. The handshake, the tool list and the
//! pages are answered here, so a client can connect before the tab is open.

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
            // The service's own tools — the projects, the product library —
            // are answered here, from the database: never by the tab, and
            // never by the editor's tools.
            let hosted = newera_mcp::surface::HOSTED_ONLY.contains(&name);
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            if hosted && let Some(why) = newera_mcp::surface::unknown_hosted_argument(name, &args) {
                return Some(result(
                    &id,
                    json!({"content": [{"type": "text", "text": why}], "isError": true}),
                ));
            }
            if crate::products::answers(name) {
                return Some(
                    match crate::products::call(app, &account.id, name, &args).await {
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
                    },
                );
            }
            // The person's own editor, when it is open and signed in; the
            // cloud project otherwise.
            if !hosted && let Some(room) = app.rooms.owned_by(&account.id) {
                // The tab may run an editor older than this service: its
                // answer gets its fields here, as the engine's do.
                let mut answer = newera_relay::answer_in(&app.rooms, &room, message).await;
                if let Some(result) = answer.as_mut().and_then(|a| a.get_mut("result")) {
                    newera_mcp::output::structure_json(result);
                }
                return answer;
            }
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
                .call(
                    &app.db,
                    &app.config.public_url,
                    &app.config.editor_url,
                    &account.id,
                    &plan,
                    name,
                    args,
                )
                .await
            {
                Ok(mut answer) => {
                    newera_mcp::output::structure_json(&mut answer);
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
