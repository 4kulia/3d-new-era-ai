//! Exports, served at their secret link for a day.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::{AppState, secret};

/// The file behind a link, while the link works.
pub async fn get(State(app): State<AppState>, Path(token): Path<String>) -> Response {
    let found: Option<(String, String, Vec<u8>)> = sqlx::query_as(
        "select name, content_type, data from files where token_hash = $1 and expires_at > now()",
    )
    .bind(secret::hash(&token))
    .fetch_optional(&app.db)
    .await
    .ok()
    .flatten();
    let Some((name, content_type, data)) = found else {
        return (StatusCode::NOT_FOUND, "this link expired").into_response();
    };
    download(&name, &content_type, data)
}

/// A file to save, under its own name: as written (RFC 5987
/// `filename*`), and in plain ASCII for whatever reads only `filename`.
pub fn download(name: &str, content_type: &str, data: Vec<u8>) -> Response {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    (
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{safe}\"; filename*=UTF-8''{}",
                    ext_value(name)
                ),
            ),
            (header::CACHE_CONTROL, "private, no-store".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        Body::from(data),
    )
        .into_response()
}

/// `name` as an RFC 5987 `ext-value`: UTF-8, every byte but its
/// `attr-char`s percent-encoded.
fn ext_value(name: &str) -> String {
    use std::fmt::Write as _;
    name.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
        out
    })
}
