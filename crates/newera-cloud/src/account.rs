//! The account: its page (plan, sign out, close), and what the editor in a
//! browser tab asks of it — who is signed in, and "this tab is mine".

use axum::Json;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::json;

use crate::login::{self, SESSION};
use crate::pages::{Lang, escape, page};
use crate::{AppState, accounts, engine, secret};

fn close_token(session: &str) -> String {
    secret::hash(&format!("close:{session}"))
}

/// The account page.
pub async fn show(
    State(app): State<AppState>,
    headers: HeaderMap,
    paid: Query<crate::billing::Paid>,
) -> Response {
    let lang = Lang::of(&headers);
    let Some(account) = login::signed_in(&app, &headers).await else {
        return Redirect::to("/login?return_to=/account").into_response();
    };
    let plan = accounts::plan_of(&app.db, &account.id).await.ok();
    let session = login::cookie(&headers, SESSION).unwrap_or_default();
    let body = format!(
        r#"<h1>{title}</h1>
<p>{email}</p>
<p class="muted">{plan_label}: <b>{plan}</b></p>
{thanks}
{projects}
{support}
<a class="button" href="{editor_href}">{editor}</a>
<a class="button ghost" href="/logout">{out}</a>
<details><summary class="muted">{close}</summary>
<form method="post" action="/account/close">
<p class="muted">{close_text}</p>
{charging}
<input type="hidden" name="csrf" value="{csrf}">
<button class="ghost" type="submit">{close_button}</button>
</form>
</details>"#,
        title = lang.pick("Sua conta", "Your account"),
        email = escape(&account.email),
        plan_label = lang.pick("Plano", "Plan"),
        plan = escape(&plan.map_or_else(|| "Free".to_owned(), |p| p.name)),
        thanks = crate::billing::thanks(lang, &paid),
        projects = project_list(&app, &account.id, lang).await,
        support = crate::billing::account_links(&app, &account, lang).await,
        editor_href = escape(&format!(
            "{}?relay={}",
            app.config.editor_url,
            login::url_encode(&app.config.public_url)
        )),
        editor = lang.pick("Abrir o editor", "Open the editor"),
        out = lang.pick("Sair", "Sign out"),
        close = lang.pick("Apagar a conta", "Close the account"),
        close_text = lang.pick(
            "Encerra o acesso na hora, desconecta as IAs ligadas a ela e apaga tudo em até 30 dias.",
            "Ends access at once, disconnects the AIs linked to it and deletes everything within 30 days."
        ),
        csrf = close_token(&session),
        close_button = lang.pick("Apagar minha conta", "Close my account"),
        charging = if crate::billing::still_charging(&app, &account.id).await {
            format!(
                r#"<p class="muted"><b>{}</b></p>"#,
                lang.pick(
                    "Cancele a assinatura antes, onde ela é cobrada: apagar a conta aqui não para a cobrança.",
                    "Cancel the subscription first, where it is charged: closing the account here does not stop the charge."
                )
            )
        } else {
            String::new()
        },
    );
    page(lang, lang.pick("Sua conta", "Your account"), &body).into_response()
}

/// The account's cloud projects, each with its download and a link that
/// opens it in the editor, saving back here.
async fn project_list(app: &AppState, account: &str, lang: Lang) -> String {
    let rows = engine::projects(&app.db, account).await.unwrap_or_default();
    if rows.is_empty() {
        return String::new();
    }
    let items = rows.iter().fold(String::new(), |mut out, p| {
        use std::fmt::Write as _;
        let _ = write!(
            out,
            r#"<li><a href="/account/projects/{id}">{name}.newera</a> <span class="muted">{kb} KB</span> · <a href="{open}">{open_label}</a></li>"#,
            id = escape(&p.id),
            name = escape(&p.name),
            kb = p.size / 1024,
            open = escape(&engine::editor_link(
                &app.config.editor_url,
                &app.config.public_url,
                &p.id
            )),
            open_label = lang.pick("abrir no editor", "open in editor"),
        );
        out
    });
    format!(
        r#"<p class="muted">{title}</p><ul>{items}</ul>"#,
        title = lang.pick(
            "Projetos na nuvem (abra no editor, ou baixe e abra no app):",
            "Projects in the cloud (open in the editor, or download and open in the app):"
        ),
    )
}

/// The account's cloud projects, for the editor's "open from the server":
/// the last changed first.
pub async fn list(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(account) = login::signed_in(&app, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match engine::projects(&app.db, &account.id).await {
        Ok(projects) => Json(json!({"projects": projects})).into_response(),
        Err(err) => {
            tracing::error!("listing projects: {err}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// "This tab shows this project": the editor, on binding one of the
/// account's projects (opened by link, from the server, or made there),
/// makes it the account's active one, so its AI and its renders work on the
/// project on screen. 204; 404 when the account has no such project.
pub async fn activate(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !from_site(&app, &headers) {
        return (
            StatusCode::FORBIDDEN,
            "only the editor can switch the project",
        )
            .into_response();
    }
    let Some(account) = login::signed_in(&app, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match app.engine.activate(&app.db, &account.id, &id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("activating a project from the editor: {err}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct NewProject {
    name: Option<String>,
}

/// Makes an empty cloud project for the editor's "new project on the
/// server", within the plan as the AI's `file action=new` is. Answers its id,
/// name and revision (201); 403 when the plan keeps no more projects, 413
/// when it has no room, 409 when the name is taken.
pub async fn create(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !from_site(&app, &headers) {
        return (StatusCode::FORBIDDEN, "only the editor can make a project").into_response();
    }
    let Some(account) = login::signed_in(&app, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let asked: NewProject = if body.iter().all(u8::is_ascii_whitespace) {
        NewProject::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(asked) => asked,
            Err(err) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("the body is {{\"name\": \"…\"}}: {err}")})),
                )
                    .into_response();
            }
        }
    };
    let made = match accounts::plan_of(&app.db, &account.id).await {
        Ok(plan) => engine::make(&app.db, &account.id, &plan, asked.name.as_deref())
            .await
            .map(|made| (plan, made)),
        Err(err) => Err(err),
    };
    let (plan, made) = match made {
        Ok(made) => made,
        Err(err) => {
            tracing::error!("making a project from the editor: {err}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let (status, why) = match made {
        engine::Made::New { id, name } => {
            return (
                StatusCode::CREATED,
                [(REVISION, header::HeaderValue::from(0))],
                Json(json!({"id": id, "name": name, "revision": 0})),
            )
                .into_response();
        }
        engine::Made::TooMany(most) => (
            StatusCode::FORBIDDEN,
            format!(
                "the {} plan keeps {most} projects; delete one from the account page",
                plan.name
            ),
        ),
        engine::Made::Full(why) => (StatusCode::PAYLOAD_TOO_LARGE, why),
        engine::Made::Taken(name) => (
            StatusCode::CONFLICT,
            format!("there is a project named {name} already"),
        ),
    };
    (status, Json(json!({"error": why}))).into_response()
}

/// A cloud project as a `.newera` file, for its owner.
pub async fn download(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let Some(account) = login::signed_in(&app, &headers).await else {
        return Redirect::to("/login?return_to=/account").into_response();
    };
    let found: Option<(String, Vec<u8>, i64)> = sqlx::query_as(
        "select name, data, revision from projects where id = $1 and account_id = $2",
    )
    .bind(&id)
    .bind(&account.id)
    .fetch_optional(&app.db)
    .await
    .ok()
    .flatten();
    match found {
        Some((name, data, revision)) => {
            let mut response =
                crate::files::download(&format!("{name}.newera"), "application/zip", data);
            response
                .headers_mut()
                .insert(REVISION, header::HeaderValue::from(revision));
            response
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The header a project's revision travels in: answered with the project,
/// sent back with a save, answered again with the revision the save made.
pub const REVISION: header::HeaderName = header::HeaderName::from_static("x-project-revision");

/// Keeps a cloud project the editor tab sends back, if it is still the
/// revision the tab loaded (`X-Project-Revision`). Answers the new revision;
/// 409 when somebody saved since, 413 when the plan has no room for it.
pub async fn upload(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    body: axum::body::Body,
) -> Response {
    if !from_site(&app, &headers) {
        return (StatusCode::FORBIDDEN, "only the editor can save a project").into_response();
    }
    let Some(account) = login::signed_in(&app, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(from) = headers
        .get(&REVISION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
    else {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "X-Project-Revision: the revision this project was loaded at",
        )
            .into_response();
    };
    match save_upload(&app, &account.id, &id, from, body).await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!("saving a project from the editor: {err}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Why a body was not read to its end.
#[derive(Debug, PartialEq, Eq)]
enum Capped {
    /// More than `most` bytes came.
    TooLarge,
    /// The sender went away, or the connection failed, before the end.
    Aborted,
}

/// The body, read until it passes `most` bytes — then no further.
async fn read_capped(body: axum::body::Body, most: usize) -> Result<Vec<u8>, Capped> {
    use axum::body::HttpBody as _;
    let mut body = std::pin::pin!(body);
    let mut bytes = Vec::new();
    while let Some(frame) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
        let frame = frame.map_err(|_| Capped::Aborted)?;
        if let Ok(data) = frame.into_data() {
            if data.len() > most - bytes.len() {
                return Err(Capped::TooLarge);
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes)
}

async fn save_upload(
    app: &AppState,
    account: &str,
    id: &str,
    from: i64,
    body: axum::body::Body,
) -> anyhow::Result<Response> {
    let current: Option<i64> =
        sqlx::query_scalar("select revision from projects where id = $1 and account_id = $2")
            .bind(id)
            .bind(account)
            .fetch_optional(&app.db)
            .await?;
    let Some(current) = current else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let plan = accounts::plan_of(&app.db, account).await?;
    // Nothing bigger than all the plan stores can be kept: read no more.
    let bytes = match read_capped(
        body,
        usize::try_from(engine::storage_limit(&plan)).unwrap_or(0),
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(Capped::TooLarge) => {
            let why = format!(
                "the {} plan stores {} MB of projects and this one is bigger than that",
                plan.name,
                engine::storage_limit(&plan) / 1024 / 1024
            );
            return Ok((StatusCode::PAYLOAD_TOO_LARGE, Json(json!({"error": why}))).into_response());
        }
        Err(Capped::Aborted) => return Ok(StatusCode::BAD_REQUEST.into_response()),
    };
    match engine::store(&app.db, account, &plan, id, from, &bytes).await? {
        engine::Stored::Saved(revision) => {
            app.engine.forget(account, id, revision).await;
            Ok((
                [(REVISION, header::HeaderValue::from(revision))],
                Json(json!({"revision": revision})),
            )
                .into_response())
        }
        engine::Stored::Stale => Ok((
            StatusCode::CONFLICT,
            Json(json!({"error": "the project changed since this tab loaded it", "revision": current})),
        )
            .into_response()),
        engine::Stored::Full(why) => {
            Ok((StatusCode::PAYLOAD_TOO_LARGE, Json(json!({"error": why}))).into_response())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CloseForm {
    csrf: String,
}

/// Where signing in from the editor ends: the editor's own tab picks the
/// account up by itself, so this one only says so.
pub async fn done(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let lang = Lang::of(&headers);
    let Some(account) = login::signed_in(&app, &headers).await else {
        return Redirect::to("/login?return_to=/account/done").into_response();
    };
    let body = format!(
        "<h1>{}</h1><p>{} <b>{}</b>.</p><p class=\"muted\">{}</p>",
        lang.pick("Pronto", "All set"),
        lang.pick("Você entrou como", "You are signed in as"),
        escape(&account.email),
        lang.pick(
            "Pode fechar esta aba e voltar para o editor: ele se conecta à sua conta sozinho.",
            "You can close this tab and go back to the editor: it links to your account by itself."
        ),
    );
    page(lang, lang.pick("Pronto", "All set"), &body).into_response()
}

/// Closes the account.
pub async fn close(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<CloseForm>,
) -> Response {
    let lang = Lang::of(&headers);
    let Some(account) = login::signed_in(&app, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let session = login::cookie(&headers, SESSION).unwrap_or_default();
    if !secret::same(&f.csrf, &close_token(&session)) {
        return (StatusCode::BAD_REQUEST, "the form expired").into_response();
    }
    let charging = crate::billing::still_charging(&app, &account.id).await;
    crate::billing::cancel_for_closed(&app, &account.id).await;
    if let Err(err) = accounts::close(&app.db, &account.id).await {
        tracing::error!("closing an account: {err}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let body = format!(
        "<h1>{}</h1><p class=\"muted\">{}</p>",
        lang.pick("Conta apagada", "Account closed"),
        lang.pick(
            "Tudo o que era dela some em até 30 dias.",
            "Everything that was in it goes within 30 days."
        )
    ) + &if charging {
        format!(
            "<p class=\"muted\"><b>{}</b></p>",
            lang.pick(
                "A assinatura continua ativa onde é cobrada: cancele lá para não ser cobrado de novo.",
                "The subscription is still active where it is charged: cancel it there so you are not charged again."
            )
        )
    } else {
        String::new()
    };
    let mut response = page(lang, "Account", &body).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        header::HeaderValue::from_static(
            "newera_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        ),
    );
    response
}

/// A request from the site with the person's cookie must come from the
/// site: the browser sets `Origin` and a page cannot forge it.
fn from_site(app: &AppState, headers: &HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|origin| app.config.site_origins.iter().any(|o| o == origin))
}

/// Who is signed in, for the editor's AI panel.
pub async fn me(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(account) = login::signed_in(&app, &headers).await else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"signed_in": false}))).into_response();
    };
    let plan = accounts::plan_of(&app.db, &account.id).await.ok();
    Json(json!({
        "signed_in": true,
        "email": account.email,
        "plan": plan,
        "mcp_url": format!("{}/mcp", app.config.public_url),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct Claim {
    room: String,
    tab_key: String,
}

/// "This tab is mine": the editor, signed in, hands its relay room to the
/// account, so the account's AI clients reach it at the fixed address.
pub async fn claim(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(c): Json<Claim>,
) -> Response {
    if !from_site(&app, &headers) {
        return (StatusCode::FORBIDDEN, "only the editor can claim a room").into_response();
    }
    let Some(account) = login::signed_in(&app, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match app.rooms.claim(&c.room, &c.tab_key, &account.id) {
        Ok(()) => Json(json!({"ok": true, "mcp_url": format!("{}/mcp", app.config.public_url)}))
            .into_response(),
        Err(why) => (StatusCode::FORBIDDEN, why).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Capped, read_capped};
    use axum::body::{Body, Bytes};

    fn chunks(parts: Vec<Result<&'static [u8], &'static str>>) -> Body {
        Body::from_stream(futures_util::stream::iter(
            parts
                .into_iter()
                .map(|p| p.map(Bytes::from_static).map_err(std::io::Error::other)),
        ))
    }

    #[tokio::test]
    async fn a_body_is_read_up_to_the_cap_and_no_further() {
        let body = chunks(vec![Ok(b"abc"), Ok(b"de")]);
        assert_eq!(read_capped(body, 5).await, Ok(b"abcde".to_vec()));
        let body = chunks(vec![Ok(b"abc"), Ok(b"def")]);
        assert_eq!(read_capped(body, 5).await, Err(Capped::TooLarge));
    }

    #[tokio::test]
    async fn a_sender_that_goes_away_is_not_a_body_too_large() {
        let body = chunks(vec![Ok(b"abc"), Err("connection reset")]);
        assert_eq!(read_capped(body, 1000).await, Err(Capped::Aborted));
    }
}
