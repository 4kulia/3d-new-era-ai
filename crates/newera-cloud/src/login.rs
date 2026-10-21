//! Signing in: an emailed link, or Google. Either way the result is the
//! same — an account for a verified address, and a session cookie.
//!
//! The emailed link opens a page with a button rather than signing in by
//! itself: mail scanners follow links, and a one-time link they followed
//! would be spent before the person clicks it.

use std::time::Duration;

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::pages::{Lang, escape, page};
use crate::{AppState, accounts, mail, secret};

/// The session cookie's name.
pub const SESSION: &str = "newera_session";
/// Carries the Google sign-in state across the round trip.
const GOOGLE_STATE: &str = "newera_google";
/// How long an emailed link works.
const LINK_MINUTES: i32 = 15;

/// A cookie's value from the request.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// A `Set-Cookie` for this service's host only.
fn set_cookie(state: &AppState, name: &str, value: &str, max_age: i64) -> HeaderValue {
    let secure = if state.config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("cookie values are URL-safe")
}

/// The account whose browser this is, if it is signed in.
pub async fn signed_in(state: &AppState, headers: &HeaderMap) -> Option<accounts::Account> {
    let token = cookie(headers, SESSION)?;
    accounts::session(&state.db, &state.config.allowed_emails, &token)
        .await
        .ok()
        .flatten()
}

/// Where a sign-in may send the person after: a path on this service, or a
/// page of the site. Anything else — another host, `//evil`, a scheme — goes
/// to the account page instead, so a link cannot turn this into a
/// redirector.
pub fn safe_return(state: &AppState, return_to: Option<&str>) -> String {
    let fallback = "/account".to_owned();
    let Some(target) = return_to.map(str::trim).filter(|t| !t.is_empty()) else {
        return fallback;
    };
    if target.starts_with('/') && !target.starts_with("//") && !target.contains('\\') {
        return target.to_owned();
    }
    let Ok(parsed) = url::Url::parse(target) else {
        return fallback;
    };
    let origin = parsed.origin().ascii_serialization();
    if state.config.site_origins.contains(&origin) {
        parsed.to_string()
    } else {
        fallback
    }
}

#[derive(Debug, Deserialize)]
pub struct ReturnTo {
    return_to: Option<String>,
}

/// The sign-in page.
pub async fn form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ReturnTo>,
) -> Response {
    let lang = Lang::of(&headers);
    let return_to = safe_return(&state, q.return_to.as_deref());
    if signed_in(&state, &headers).await.is_some() {
        return Redirect::to(&return_to).into_response();
    }
    let google = google_button(&state, lang, &return_to);
    if matches!(state.config.mail, crate::config::Mail::Off) {
        return email_unavailable(lang, &google);
    }
    let body = format!(
        r#"<h1>{title}</h1>
<p class="muted">{lead}</p>
<form method="post" action="/login">
<label for="email">{email}</label>
<input id="email" name="email" type="email" required autocomplete="email" autofocus>
<input type="hidden" name="return_to" value="{rt}">
<button type="submit">{send}</button>
</form>
{google}"#,
        title = lang.pick("Entrar", "Sign in"),
        lead = lang.pick(
            "Enviamos um link para o seu e-mail. Sem senha.",
            "We email you a link. No password."
        ),
        email = lang.pick("E-mail", "Email"),
        rt = escape(&return_to),
        send = lang.pick("Enviar link", "Email me a link"),
    );
    page(lang, lang.pick("Entrar", "Sign in"), &body).into_response()
}

/// The "Continue with Google" button, when Google sign-in is set up.
fn google_button(state: &AppState, lang: Lang, return_to: &str) -> String {
    state.config.google.as_ref().map_or_else(String::new, |_| {
        format!(
            r#"<a class="button ghost" href="/login/google?return_to={rt}">{label}</a>"#,
            rt = escape(&url_encode(return_to)),
            label = lang.pick("Entrar com Google", "Continue with Google"),
        )
    })
}

/// The answer wherever email sign-in is asked for while it is not set up.
fn email_unavailable(lang: Lang, google: &str) -> Response {
    let body = format!(
        "<h1>{}</h1><p class=\"muted\">{}</p>{google}",
        lang.pick("Entrar", "Sign in"),
        lang.pick(
            "O acesso por e-mail ainda não está disponível.",
            "Signing in by email is not available yet."
        ),
    );
    page(lang, lang.pick("Entrar", "Sign in"), &body).into_response()
}

#[derive(Debug, Deserialize)]
pub struct EmailForm {
    email: String,
    return_to: Option<String>,
}

/// Sends the link.
pub async fn send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<EmailForm>,
) -> Response {
    let lang = Lang::of(&headers);
    // Without email sign-in every address gets this answer, ahead of any
    // decision about the address: the page never tells which are let in.
    if matches!(state.config.mail, crate::config::Mail::Off) {
        let return_to = safe_return(&state, f.return_to.as_deref());
        return email_unavailable(lang, &google_button(&state, lang, &return_to));
    }
    let email = f.email.trim().to_owned();
    let plausible = email.len() <= 254
        && email.split_once('@').is_some_and(|(user, host)| {
            !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
        })
        && !email
            .chars()
            .any(|c| c.is_whitespace() || c == '<' || c == '>');
    if !plausible {
        let body = format!(
            "<h1>{}</h1><p><a href=\"/login\">{}</a></p>",
            lang.pick(
                "Esse e-mail não parece certo",
                "That email does not look right"
            ),
            lang.pick("Tentar de novo", "Try again")
        );
        return (StatusCode::BAD_REQUEST, page(lang, "Sign in", &body)).into_response();
    }
    let hour = Duration::from_secs(3600);
    let who = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .split(',')
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_owned();
    if !state
        .limits
        .allow(&format!("mail:{}", email.to_lowercase()), 5, hour)
        || !state.limits.allow(&format!("ip:{who}"), 30, hour)
    {
        let body = format!(
            "<h1>{}</h1><p class=\"muted\">{}</p>",
            lang.pick("Muitos pedidos", "Too many requests"),
            lang.pick(
                "Espere um pouco e peça de novo.",
                "Wait a little and ask again."
            )
        );
        return (StatusCode::TOO_MANY_REQUESTS, page(lang, "Sign in", &body)).into_response();
    }
    // An address this installation does not let in gets no link, and the
    // same answer as one it does: the page never tells which are let in.
    if !state.config.allowed_emails.admits(&email) {
        return sent(lang, &email);
    }
    // The insert and the send run apart from the answer: how long they take
    // must not tell a listed address from one that is not, and whatever goes
    // wrong in them is logged, not shown.
    let return_to = safe_return(&state, f.return_to.as_deref());
    let page = sent(lang, &email);
    tokio::spawn(async move {
        let token = secret::token();
        let stored = sqlx::query(
            "insert into login_links (token_hash, email, return_to, expires_at)
             values ($1, $2, $3, now() + make_interval(mins => $4))",
        )
        .bind(secret::hash(&token))
        .bind(&email)
        .bind(&return_to)
        .bind(LINK_MINUTES)
        .execute(&state.db)
        .await;
        if let Err(err) = stored {
            tracing::error!("login link: {err}");
            return;
        }
        let link = format!("{}/login/link?token={token}", state.config.public_url);
        if let Err(err) =
            mail::send_login_link(&state.http, &state.config.mail, &email, &link, lang).await
        {
            tracing::error!("sending the sign-in link: {err:#}");
        }
    });
    page
}

/// "Check your email": the answer to a link asked for.
fn sent(lang: Lang, email: &str) -> Response {
    let body = format!(
        "<h1>{}</h1><p>{} <b>{}</b>.</p><p class=\"muted\">{}</p>",
        lang.pick("Veja seu e-mail", "Check your email"),
        lang.pick("Mandamos um link para", "We sent a link to"),
        escape(email),
        lang.pick(
            "Ele vale por 15 minutos. Pode fechar esta página.",
            "It works for 15 minutes. You can close this page."
        ),
    );
    page(
        lang,
        lang.pick("Veja seu e-mail", "Check your email"),
        &body,
    )
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct LinkQuery {
    token: String,
}

/// The page the emailed link opens: one button, which spends the link.
pub async fn confirm(headers: HeaderMap, Query(q): Query<LinkQuery>) -> Response {
    let lang = Lang::of(&headers);
    let body = format!(
        r#"<h1>{title}</h1>
<form method="post" action="/login/link">
<input type="hidden" name="token" value="{token}">
<button type="submit">{go}</button>
</form>"#,
        title = lang.pick("Entrar no 3D New Era AI", "Sign in to 3D New Era AI"),
        token = escape(&q.token),
        go = lang.pick("Entrar", "Sign in"),
    );
    page(lang, lang.pick("Entrar", "Sign in"), &body).into_response()
}

#[derive(Debug, Deserialize)]
pub struct LinkForm {
    token: String,
}

/// Spends the link and signs the browser in.
pub async fn redeem(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<LinkForm>,
) -> Response {
    let lang = Lang::of(&headers);
    let used: Option<(String, String)> = sqlx::query_as(
        "update login_links set used_at = now()
         where token_hash = $1 and used_at is null and expires_at > now()
         returning email, return_to",
    )
    .bind(secret::hash(&f.token))
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);
    let Some((email, return_to)) = used else {
        let body = format!(
            "<h1>{}</h1><p class=\"muted\">{}</p><a class=\"button\" href=\"/login\">{}</a>",
            lang.pick("Link vencido", "Link expired"),
            lang.pick(
                "Esse link já foi usado ou passou de 15 minutos.",
                "That link was used already or is older than 15 minutes."
            ),
            lang.pick("Pedir outro", "Get another"),
        );
        return (StatusCode::GONE, page(lang, "Sign in", &body)).into_response();
    };
    finish(&state, lang, &email, &return_to).await
}

/// An address is proven: its account, a session, and on to where it was going.
/// An address this installation does not let in stops here, whichever way it
/// was proven.
async fn finish(state: &AppState, lang: Lang, email: &str, return_to: &str) -> Response {
    let account =
        match accounts::find_or_create(&state.db, &state.config.allowed_emails, email).await {
            Ok(Some(account)) => account,
            Ok(None) => {
                tracing::warn!("sign-in refused: the address is not one this installation lets in");
                let body = format!(
                    "<h1>{}</h1><p class=\"muted\">{}</p>",
                    lang.pick("Acesso fechado", "Not open to you"),
                    lang.pick(
                        "Este endereço não pode entrar aqui.",
                        "This address cannot sign in here."
                    ),
                );
                return (StatusCode::FORBIDDEN, page(lang, "Sign in", &body)).into_response();
            }
            Err(err) => {
                tracing::error!("account for a sign-in: {err}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    let token = match accounts::open_session(&state.db, &account.id).await {
        Ok(token) => token,
        Err(err) => {
            tracing::error!("session: {err}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut response = Redirect::to(&safe_return(state, Some(return_to))).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            state,
            SESSION,
            &token,
            i64::from(accounts::SESSION_DAYS) * 86_400,
        ),
    );
    response
}

/// Signs this browser out.
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie(&headers, SESSION) {
        let _ = accounts::close_session(&state.db, &token).await;
    }
    let mut response = Redirect::to("/login").into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, set_cookie(&state, SESSION, "", 0));
    response
}

/// Off to Google, with a state only this browser holds.
pub async fn google(State(state): State<AppState>, Query(q): Query<ReturnTo>) -> Response {
    let Some(google) = &state.config.google else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let nonce = secret::token();
    let return_to = safe_return(&state, q.return_to.as_deref());
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email&state={}&prompt=select_account",
        url_encode(&google.client_id),
        url_encode(&format!(
            "{}/login/google/callback",
            state.config.public_url
        )),
        nonce,
    );
    let mut response = Redirect::to(&url).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            &state,
            GOOGLE_STATE,
            &format!("{nonce}.{}", url_encode(&return_to)),
            600,
        ),
    );
    response
}

#[derive(Debug, Deserialize)]
pub struct GoogleCallback {
    code: Option<String>,
    state: Option<String>,
}

/// Back from Google: the address, if Google says it is verified.
pub async fn google_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<GoogleCallback>,
) -> Response {
    let (Some(google), Some(code), Some(nonce)) = (&state.config.google, q.code, q.state) else {
        return Redirect::to("/login").into_response();
    };
    let Some((kept, return_to)) = cookie(&headers, GOOGLE_STATE)
        .as_deref()
        .and_then(|c| c.split_once('.'))
        .map(|(n, r)| (n.to_owned(), url_decode(r)))
    else {
        return Redirect::to("/login").into_response();
    };
    if !secret::same(&kept, &nonce) {
        return Redirect::to("/login").into_response();
    }
    let email = async {
        let tokens: serde_json::Value = state
            .http
            .post(&google.token_url)
            .form(&[
                ("code", code.as_str()),
                ("client_id", google.client_id.as_str()),
                ("client_secret", google.client_secret.as_str()),
                (
                    "redirect_uri",
                    &format!("{}/login/google/callback", state.config.public_url),
                ),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let access = tokens["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("no access token from Google"))?;
        let info: serde_json::Value = state
            .http
            .get(&google.userinfo_url)
            .bearer_auth(access)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if info["email_verified"] != serde_json::Value::Bool(true) {
            anyhow::bail!("Google has not verified this address");
        }
        info["email"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("no email from Google"))
    }
    .await;
    match email {
        Ok(email) => {
            let mut response = finish(&state, Lang::of(&headers), &email, &return_to).await;
            response
                .headers_mut()
                .append(header::SET_COOKIE, set_cookie(&state, GOOGLE_STATE, "", 0));
            response
        }
        Err(err) => {
            tracing::warn!("google sign-in: {err:#}");
            Redirect::to("/login").into_response()
        }
    }
}

/// Percent-encodes a query value.
pub fn url_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

fn url_decode(text: &str) -> String {
    url::form_urlencoded::parse(format!("x={text}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}
