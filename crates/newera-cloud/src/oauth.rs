//! OAuth 2.1 for AI clients, as the MCP authorization spec asks.
//!
//! - Metadata: protected resource (RFC 9728) and authorization server
//!   (RFC 8414).
//! - Clients: registered here on the fly (RFC 7591), or known by the URL of
//!   their metadata document (client ID metadata documents) — fetched with
//!   care, since the URL is the caller's choice.
//! - Codes with PKCE S256 only; public clients only (no client secrets).
//! - Access tokens for an hour; refresh tokens for 30 days, each used once:
//!   one presented twice revokes its family.
//!
//! Every token is opaque and stored as its hash.

use std::net::IpAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::json;

use crate::login::{self, url_encode};
use crate::pages::{Lang, escape, page};
use crate::{AppState, accounts, secret};

/// The one scope: act in 3D New Era AI for this account.
pub const SCOPE: &str = "newera";

/// Asked for by clients that want to stay signed in (`ChatGPT` does). Every
/// grant already comes with a refresh token, so it changes nothing.
const OFFLINE: &str = "offline_access";

/// Whether a requested scope asks only for what this server grants.
fn known_scopes(scope: &str) -> bool {
    scope.split_whitespace().all(|s| s == SCOPE || s == OFFLINE)
}
const ACCESS_SECS: i64 = 3600;
const REFRESH_DAYS: i64 = 30;
const CODE_SECS: i64 = 600;
/// A client's metadata document is read again after this long.
const METADATA_TTL_SECS: i64 = 3600;

fn base(state: &AppState) -> &str {
    &state.config.public_url
}

/// The protected resource: `/mcp`, and who issues tokens for it.
pub async fn resource_metadata(State(state): State<AppState>) -> Response {
    let base = base(&state);
    cached(Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "3D New Era AI",
        "resource_documentation": "https://github.com/leandrodaf/3d-new-era-ai#connect-your-ai",
    })))
}

/// The authorization server.
pub async fn server_metadata(State(state): State<AppState>) -> Response {
    let base = base(&state);
    cached(Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [SCOPE, OFFLINE],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
        "service_documentation": "https://github.com/leandrodaf/3d-new-era-ai#connect-your-ai",
    })))
}

fn cached(body: Json<serde_json::Value>) -> Response {
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("public, max-age=300"),
    );
    response
}

/// An OAuth error, as JSON (token, registration) with its status.
fn error(status: StatusCode, code: &str, description: &str) -> Response {
    let mut response = (
        status,
        Json(json!({ "error": code, "error_description": description })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// An address a client may be sent back to: HTTPS anywhere, or plain HTTP
/// on this machine's loopback (for clients like Claude Code that listen
/// locally). Never a fragment.
fn allowed_redirect(uri: &str) -> bool {
    let Ok(parsed) = url::Url::parse(uri) else {
        return false;
    };
    if parsed.fragment().is_some() {
        return false;
    }
    match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        "http" => matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        _ => false,
    }
}

/// Whether `given` is one of `registered`. A loopback address matches
/// whatever its port, as RFC 8252 asks — local clients pick a free port each
/// time.
fn redirect_matches(registered: &[String], given: &str) -> bool {
    let Ok(given_url) = url::Url::parse(given) else {
        return false;
    };
    registered.iter().any(|r| {
        if r == given {
            return true;
        }
        let Ok(r_url) = url::Url::parse(r) else {
            return false;
        };
        let loopback = matches!(r_url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        loopback
            && r_url.scheme() == "http"
            && given_url.scheme() == "http"
            && r_url.host_str() == given_url.host_str()
            && r_url.path() == given_url.path()
            && r_url.query() == given_url.query()
    })
}

#[derive(Debug, Deserialize)]
pub struct Registration {
    #[serde(default)]
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    token_endpoint_auth_method: Option<String>,
    grant_types: Option<Vec<String>>,
}

/// Dynamic client registration: a public client with its redirect URIs.
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<Registration>,
) -> Response {
    let who = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    if !state
        .limits
        .allow(&format!("register:{who}"), 60, Duration::from_secs(3600))
    {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "slow_down",
            "too many registrations from here",
        );
    }
    if r.redirect_uris.is_empty() || r.redirect_uris.len() > 10 {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "give one to ten redirect_uris",
        );
    }
    if let Some(bad) = r.redirect_uris.iter().find(|u| !allowed_redirect(u)) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            &format!("{bad}: https, or http on 127.0.0.1/localhost"),
        );
    }
    if r.token_endpoint_auth_method
        .as_deref()
        .is_some_and(|m| m != "none")
    {
        // Public clients only: a secret in an AI client protects nothing.
        tracing::info!(
            "registration asked for {:?}; answered none",
            r.token_endpoint_auth_method
        );
    }
    if r.grant_types.as_ref().is_some_and(|g| {
        g.iter()
            .any(|t| t != "authorization_code" && t != "refresh_token")
    }) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "grant_types: authorization_code and refresh_token",
        );
    }
    let name: String = r
        .client_name
        .unwrap_or_else(|| "AI client".to_owned())
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect();
    let client_id = secret::id();
    let stored = sqlx::query(
        "insert into oauth_clients (client_id, kind, name, redirect_uris) values ($1, 'registered', $2, $3)",
    )
    .bind(&client_id)
    .bind(&name)
    .bind(&r.redirect_uris)
    .execute(&state.db)
    .await;
    if let Err(err) = stored {
        tracing::error!("register: {err}");
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "try again",
        );
    }
    let mut response = (
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "client_id_issued_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            "client_name": name,
            "redirect_uris": r.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// A client, as far as this server trusts what it says.
#[derive(Debug, Clone)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}

/// The client behind an id: registered here, or described by the document
/// at the URL that is its id.
///
/// # Errors
///
/// When there is no such client, or its document cannot be trusted; the
/// message says why.
pub async fn client(state: &AppState, client_id: &str) -> Result<Client, String> {
    if client_id.starts_with("https://") {
        return metadata_client(state, client_id).await;
    }
    let row: Option<(Option<String>, Vec<String>)> = sqlx::query_as(
        "select name, redirect_uris from oauth_clients where client_id = $1 and kind = 'registered'",
    )
    .bind(client_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let (name, redirect_uris) = row.ok_or("unknown client_id")?;
    Ok(Client {
        id: client_id.to_owned(),
        name: name.unwrap_or_else(|| "AI client".to_owned()),
        redirect_uris,
    })
}

/// A client ID metadata document: read over HTTPS from a public address,
/// small, naming itself, and kept for an hour.
async fn metadata_client(state: &AppState, url: &str) -> Result<Client, String> {
    let cached: Option<(Option<String>, Vec<String>)> = sqlx::query_as(
        "select name, redirect_uris from oauth_clients
         where client_id = $1 and kind = 'metadata' and fetched_at > now() - ($2 * interval '1 second')",
    )
    .bind(url)
    .bind(METADATA_TTL_SECS)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if let Some((name, redirect_uris)) = cached {
        return Ok(Client {
            id: url.to_owned(),
            name: name.unwrap_or_else(|| "AI client".to_owned()),
            redirect_uris,
        });
    }
    let parsed = url::Url::parse(url).map_err(|_| "client_id is not a URL")?;
    let host = parsed.host_str().ok_or("client_id has no host")?.to_owned();
    if parsed.scheme() != "https" || parsed.fragment().is_some() || parsed.path() == "/" {
        return Err("client_id must be an https URL with a path".to_owned());
    }
    // The URL is the caller's choice: it must not make this server reach
    // into its own network.
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addresses: Vec<IpAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| "client_id host does not resolve")?
        .map(|a| a.ip())
        .collect();
    if addresses.is_empty() || addresses.iter().any(|ip| !public(*ip)) {
        return Err("client_id host is not a public address".to_owned());
    }
    let response = state
        .http
        .get(url)
        .header(header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| "client metadata could not be fetched")?;
    if !response.status().is_success() {
        return Err(format!("client metadata answered {}", response.status()));
    }
    if response.content_length().is_some_and(|n| n > 64 * 1024) {
        return Err("client metadata is too large".to_owned());
    }
    let body = response
        .bytes()
        .await
        .map_err(|_| "client metadata could not be read")?;
    if body.len() > 64 * 1024 {
        return Err("client metadata is too large".to_owned());
    }
    let doc: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "client metadata is not JSON")?;
    if doc["client_id"].as_str() != Some(url) {
        return Err("client metadata names another client_id".to_owned());
    }
    let redirect_uris: Vec<String> = doc["redirect_uris"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if redirect_uris.is_empty() || !redirect_uris.iter().all(|u| allowed_redirect(u)) {
        return Err("client metadata has no usable redirect_uris".to_owned());
    }
    let name: String = doc["client_name"]
        .as_str()
        .unwrap_or(&host)
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect();
    sqlx::query(
        "insert into oauth_clients (client_id, kind, name, redirect_uris, fetched_at)
         values ($1, 'metadata', $2, $3, now())
         on conflict (client_id) do update set name = $2, redirect_uris = $3, fetched_at = now()
         where oauth_clients.kind = 'metadata'",
    )
    .bind(url)
    .bind(&name)
    .bind(&redirect_uris)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(Client {
        id: url.to_owned(),
        name,
        redirect_uris,
    })
}

/// Whether an address is on the public internet.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // carrier-grade NAT
                || o[0] >= 224)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80) // link local
        }
    }
}

/// What an authorization request carries, in the query or the consent form.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthRequest {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

/// A request checked far enough to know where errors may be sent.
struct Checked {
    client: Client,
    redirect_uri: String,
    challenge: String,
    state: Option<String>,
    resource: Option<String>,
}

/// Checks a request. `Err(Left)` is shown on a page (the redirect address
/// is not trusted yet); `Err(Right)` goes back to the client.
async fn check(
    app: &AppState,
    r: &AuthRequest,
) -> Result<Checked, Result<String, (String, String, &'static str, String)>> {
    let client_id = r
        .client_id
        .as_deref()
        .ok_or(Ok("client_id is missing".to_owned()))?;
    let client = client(app, client_id).await.map_err(Ok)?;
    let redirect_uri = match r.redirect_uri.as_deref() {
        Some(given) if redirect_matches(&client.redirect_uris, given) => given.to_owned(),
        Some(_) => {
            return Err(Ok(
                "redirect_uri is not one this client registered".to_owned()
            ));
        }
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        None => return Err(Ok("redirect_uri is missing".to_owned())),
    };
    let back = |code: &'static str, why: &str| {
        Err(Err((
            redirect_uri.clone(),
            r.state.clone().unwrap_or_default(),
            code,
            why.to_owned(),
        )))
    };
    if r.response_type.as_deref() != Some("code") {
        return back("unsupported_response_type", "response_type must be code");
    }
    let Some(challenge) = r.code_challenge.clone().filter(|c| c.len() == 43) else {
        return back("invalid_request", "code_challenge (S256) is required");
    };
    if r.code_challenge_method.as_deref() != Some("S256") {
        return back("invalid_request", "code_challenge_method must be S256");
    }
    if let Some(scope) = &r.scope
        && !known_scopes(scope)
    {
        return back("invalid_scope", "the scopes are newera and offline_access");
    }
    let resource = r.resource.clone();
    if let Some(res) = &resource {
        let ours = [format!("{}/mcp", base(app)), base(app).to_owned()];
        if !ours.iter().any(|o| o == res.trim_end_matches('/')) {
            return back("invalid_target", "resource must be this server's /mcp");
        }
    }
    Ok(Checked {
        client,
        redirect_uri,
        challenge,
        state: r.state.clone(),
        resource,
    })
}

fn with_query(uri: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(uri).expect("checked before");
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(key, value);
        }
    }
    url.to_string()
}

fn error_page(lang: Lang, why: &str) -> Response {
    let body = format!(
        "<h1>{}</h1><p class=\"muted\">{}</p>",
        lang.pick("Não foi possível conectar", "This connection cannot go on"),
        escape(why)
    );
    (StatusCode::BAD_REQUEST, page(lang, "Error", &body)).into_response()
}

fn consent_token(session: &str) -> String {
    secret::hash(&format!("consent:{session}"))
}

/// The consent screen, after signing in if need be.
pub async fn authorize(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(r): Query<AuthRequest>,
) -> Response {
    let lang = Lang::of(&headers);
    let checked = match check(&app, &r).await {
        Ok(checked) => checked,
        Err(Ok(why)) => return error_page(lang, &why),
        Err(Err((to, state, code, why))) => {
            return Redirect::to(&with_query(
                &to,
                &[
                    ("error", code),
                    ("error_description", &why),
                    ("state", &state),
                    ("iss", base(&app)),
                ],
            ))
            .into_response();
        }
    };
    let Some(account) = login::signed_in(&app, &headers).await else {
        let here = uri
            .path_and_query()
            .map_or("/oauth/authorize", |p| p.as_str());
        return Redirect::to(&format!("/login?return_to={}", url_encode(here))).into_response();
    };
    let session = login::cookie(&headers, login::SESSION).unwrap_or_default();
    let host = url::Url::parse(&checked.redirect_uri)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let hidden = |name: &str, value: Option<&str>| {
        value.map_or_else(String::new, |v| {
            format!(
                r#"<input type="hidden" name="{name}" value="{}">"#,
                escape(v)
            )
        })
    };
    let body = format!(
        r#"<h1>{title}</h1>
<p><b>{name}</b> {asks}</p>
<p class="muted">{as_who} <b>{email}</b>. {back} <code>{host}</code>.</p>
<form method="post" action="/oauth/authorize">
{fields}
<input type="hidden" name="csrf" value="{csrf}">
<div class="row">
<button class="ghost" type="submit" name="decision" value="deny">{deny}</button>
<button type="submit" name="decision" value="allow">{allow}</button>
</div>
</form>
<p class="muted"><a href="/logout">{other}</a></p>"#,
        title = lang.pick("Permitir acesso", "Allow access"),
        name = escape(&checked.client.name),
        asks = lang.pick(
            "quer desenhar e editar os seus projetos no 3D New Era AI.",
            "wants to draw and edit your projects in 3D New Era AI."
        ),
        as_who = lang.pick("Você está entrando como", "You are signed in as"),
        email = escape(&account.email),
        back = lang.pick("Depois, você volta para", "You will be sent back to"),
        host = escape(&host),
        fields = [
            hidden("response_type", r.response_type.as_deref()),
            hidden("client_id", r.client_id.as_deref()),
            hidden("redirect_uri", Some(&checked.redirect_uri)),
            hidden("code_challenge", r.code_challenge.as_deref()),
            hidden("code_challenge_method", r.code_challenge_method.as_deref()),
            hidden("state", r.state.as_deref()),
            hidden("scope", r.scope.as_deref()),
            hidden("resource", r.resource.as_deref()),
        ]
        .concat(),
        csrf = consent_token(&session),
        deny = lang.pick("Recusar", "Deny"),
        allow = lang.pick("Permitir", "Allow"),
        other = lang.pick("Usar outra conta", "Use another account"),
    );
    let mut response =
        page(lang, lang.pick("Permitir acesso", "Allow access"), &body).into_response();
    // The consent screen is not to be framed by anyone.
    response.headers_mut().insert(
        "content-security-policy",
        // Not form-action: browsers apply it to the redirect back to the
        // client after the form, and that redirect must go through.
        header::HeaderValue::from_static("frame-ancestors 'none'"),
    );
    response
}

#[derive(Debug, Deserialize)]
pub struct Decision {
    #[serde(flatten)]
    request: AuthRequest,
    csrf: String,
    #[serde(rename = "decision")]
    choice: String,
}

/// The person's answer on the consent screen.
pub async fn decide(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(d): Form<Decision>,
) -> Response {
    let lang = Lang::of(&headers);
    let Some(account) = login::signed_in(&app, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let session = login::cookie(&headers, login::SESSION).unwrap_or_default();
    if !secret::same(&d.csrf, &consent_token(&session)) {
        return error_page(lang, "the form expired; start again from your AI client");
    }
    let checked = match check(&app, &d.request).await {
        Ok(checked) => checked,
        Err(Ok(why)) => return error_page(lang, &why),
        Err(Err((to, state, code, why))) => {
            return Redirect::to(&with_query(
                &to,
                &[
                    ("error", code),
                    ("error_description", &why),
                    ("state", &state),
                    ("iss", base(&app)),
                ],
            ))
            .into_response();
        }
    };
    let state = checked.state.clone().unwrap_or_default();
    if d.choice != "allow" {
        return Redirect::to(&with_query(
            &checked.redirect_uri,
            &[
                ("error", "access_denied"),
                ("state", &state),
                ("iss", base(&app)),
            ],
        ))
        .into_response();
    }
    let code = secret::token();
    let stored = sqlx::query(
        "insert into oauth_codes (code_hash, client_id, account_id, redirect_uri, code_challenge, scope, resource, expires_at)
         values ($1, $2, $3, $4, $5, $6, $7, now() + ($8 * interval '1 second'))",
    )
    .bind(secret::hash(&code))
    .bind(&checked.client.id)
    .bind(&account.id)
    .bind(&checked.redirect_uri)
    .bind(&checked.challenge)
    .bind(SCOPE)
    .bind(&checked.resource)
    .bind(CODE_SECS)
    .execute(&app.db)
    .await;
    if let Err(err) = stored {
        tracing::error!("code: {err}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let mut query: Vec<(&str, &str)> = vec![("code", &code), ("iss", base(&app))];
    if checked.state.is_some() {
        query.push(("state", &state));
    }
    Redirect::to(&with_query(&checked.redirect_uri, &query)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

/// Issues an access and a refresh token in `family`.
async fn issue(
    app: &AppState,
    client_id: &str,
    account_id: &str,
    family: &str,
) -> Result<Response, sqlx::Error> {
    let (access, refresh) = (secret::token(), secret::token());
    let mut tx = app.db.begin().await?;
    for (token, kind, secs) in [
        (&access, "access", ACCESS_SECS),
        (&refresh, "refresh", REFRESH_DAYS * 86_400),
    ] {
        sqlx::query(
            "insert into oauth_tokens (token_hash, kind, client_id, account_id, scope, family, expires_at)
             values ($1, $2, $3, $4, $5, $6, now() + ($7 * interval '1 second'))",
        )
        .bind(secret::hash(token))
        .bind(kind)
        .bind(client_id)
        .bind(account_id)
        .bind(SCOPE)
        .bind(family)
        .bind(secs)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let mut response = Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": ACCESS_SECS,
        "refresh_token": refresh,
        "scope": SCOPE,
    }))
    .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

async fn revoke_family(app: &AppState, family: &str) {
    let _ = sqlx::query(
        "update oauth_tokens set revoked_at = now() where family = $1 and revoked_at is null",
    )
    .bind(family)
    .execute(&app.db)
    .await;
}

/// The token endpoint: a code for tokens, or a refresh token for new ones.
pub async fn token(State(app): State<AppState>, Form(t): Form<TokenRequest>) -> Response {
    match t.grant_type.as_str() {
        "authorization_code" => {
            let (Some(code), Some(verifier), Some(client_id)) = (
                t.code.as_deref(),
                t.code_verifier.as_deref(),
                t.client_id.as_deref(),
            ) else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code, code_verifier and client_id are required",
                );
            };
            let code_hash = secret::hash(code);
            // Spent in the same statement that reads it: a code works once.
            let row: Option<(String, String, String, String, bool)> = sqlx::query_as(
                "with old as (
                     select code_hash, used_at, expires_at from oauth_codes
                     where code_hash = $1 for update
                 )
                 update oauth_codes c set used_at = now() from old
                 where c.code_hash = old.code_hash
                 returning c.client_id, c.account_id, c.redirect_uri, c.code_challenge,
                           (old.used_at is null and old.expires_at > now())",
            )
            .bind(&code_hash)
            .fetch_optional(&app.db)
            .await
            .ok()
            .flatten();
            let Some((owner, account_id, redirect_uri, challenge, fresh)) = row else {
                return error(StatusCode::BAD_REQUEST, "invalid_grant", "unknown code");
            };
            if !fresh {
                // Presented twice: whatever it bought is not to be trusted.
                revoke_family(&app, &code_hash).await;
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "code expired or already used",
                );
            }
            if owner != client_id
                || t.redirect_uri.as_deref().is_some_and(|r| r != redirect_uri)
                || !secret::pkce_matches(verifier, &challenge)
            {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "code does not match this request",
                );
            }
            if accounts::get(&app.db, &account_id)
                .await
                .ok()
                .flatten()
                .is_none()
            {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "the account is closed",
                );
            }
            issue(&app, client_id, &account_id, &code_hash)
                .await
                .unwrap_or_else(|_| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "server_error",
                        "try again",
                    )
                })
        }
        "refresh_token" => {
            let Some(refresh) = t.refresh_token.as_deref() else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                );
            };
            let row: Option<(String, String, String, bool)> = sqlx::query_as(
                "with old as (
                     select token_hash, revoked_at, expires_at from oauth_tokens
                     where token_hash = $1 and kind = 'refresh' for update
                 )
                 update oauth_tokens t set revoked_at = coalesce(t.revoked_at, now()) from old
                 where t.token_hash = old.token_hash
                 returning t.client_id, t.account_id, t.family,
                           (old.revoked_at is null and old.expires_at > now())",
            )
            .bind(secret::hash(refresh))
            .fetch_optional(&app.db)
            .await
            .ok()
            .flatten();
            let Some((client_id, account_id, family, fresh)) = row else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "unknown refresh_token",
                );
            };
            if !fresh {
                revoke_family(&app, &family).await;
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "refresh_token expired or already used",
                );
            }
            if t.client_id.as_deref().is_some_and(|c| c != client_id) {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "refresh_token belongs to another client",
                );
            }
            if !accounts::get(&app.db, &account_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|account| app.config.allowed_emails.admits(&account.email))
            {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "the account is closed",
                );
            }
            issue(&app, &client_id, &account_id, &family)
                .await
                .unwrap_or_else(|_| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "server_error",
                        "try again",
                    )
                })
        }
        _ => error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "authorization_code or refresh_token",
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct Revocation {
    token: String,
}

/// Revokes a token (and, for a refresh token, what came from it). Always
/// answers 200, as RFC 7009 asks, so it tells nobody what exists.
pub async fn revoke(State(app): State<AppState>, Form(r): Form<Revocation>) -> StatusCode {
    let family: Option<String> = sqlx::query_scalar(
        "update oauth_tokens set revoked_at = coalesce(revoked_at, now())
         where token_hash = $1 returning family",
    )
    .bind(secret::hash(&r.token))
    .fetch_optional(&app.db)
    .await
    .ok()
    .flatten();
    if let Some(family) = family {
        revoke_family(&app, &family).await;
    }
    StatusCode::OK
}

/// The account an access token acts for, if it is good.
pub async fn bearer(app: &AppState, headers: &HeaderMap) -> Option<accounts::Account> {
    let token = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    sqlx::query_as::<_, accounts::Account>(
        "select a.id, a.email from oauth_tokens t join accounts a on a.id = t.account_id
         where t.token_hash = $1 and t.kind = 'access' and t.revoked_at is null
           and t.expires_at > now() and a.deleted_at is null
           and ($2::text[] is null or lower(a.email) = any($2))",
    )
    .bind(secret::hash(token))
    .bind(app.config.allowed_emails.list())
    .fetch_optional(&app.db)
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ChatGPT` asks for `offline_access` next to ours; anything else is refused.
    #[test]
    fn offline_access_is_a_known_scope() {
        assert!(known_scopes("newera"));
        assert!(known_scopes("newera offline_access"));
        assert!(known_scopes("offline_access"));
        assert!(!known_scopes("newera admin"));
    }

    #[test]
    fn only_https_or_loopback_redirects() {
        assert!(allowed_redirect("https://claude.ai/api/mcp/auth_callback"));
        assert!(allowed_redirect("http://127.0.0.1:33418/callback"));
        assert!(allowed_redirect("http://localhost/cb"));
        assert!(!allowed_redirect("http://evil.example/cb"));
        assert!(!allowed_redirect("javascript:alert(1)"));
        assert!(!allowed_redirect("https://a.example/cb#frag"));
    }

    #[test]
    fn a_loopback_redirect_matches_any_port() {
        let registered = vec![
            "http://127.0.0.1/callback".to_owned(),
            "https://chatgpt.com/connector_platform_oauth_redirect".to_owned(),
        ];
        assert!(redirect_matches(
            &registered,
            "http://127.0.0.1:51234/callback"
        ));
        assert!(!redirect_matches(
            &registered,
            "http://127.0.0.1:51234/other"
        ));
        assert!(redirect_matches(
            &registered,
            "https://chatgpt.com/connector_platform_oauth_redirect"
        ));
        assert!(!redirect_matches(
            &registered,
            "https://chatgpt.com:8443/connector_platform_oauth_redirect"
        ));
    }

    #[test]
    fn private_addresses_are_not_public() {
        for private in [
            "127.0.0.1",
            "10.0.0.8",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!public(private.parse().unwrap()), "{private}");
        }
        for open in ["1.1.1.1", "140.82.112.3", "2606:4700::1111"] {
            assert!(public(open.parse().unwrap()), "{open}");
        }
    }
}
