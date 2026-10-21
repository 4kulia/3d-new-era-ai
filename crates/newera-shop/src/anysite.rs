//! `AnySite`'s HTTP API, as this service calls it.
//!
//! What it is, from `AnySite`'s own `OpenAPI` (`https://api.anysite.io/openapi.json`)
//! and its CLI's client (`anysite-cli`, `src/anysite/api/client.py`), checked
//! with recorded calls (`tests/fixtures/`):
//!
//! - every endpoint is `POST {base}/api/<source>/<path>` with a JSON body,
//!   base `https://api.anysite.io`;
//! - the key goes in the `access-token` header;
//! - a success is `200` with a JSON array of items (one for a product or a
//!   page), and says what it cost in `x-credits-charged`;
//! - `401 {"detail":"Invalid token"}` is a refused key, `422` a body that
//!   does not fit (`FastAPI`'s `detail` list), `429` too many calls, `5xx`
//!   `AnySite`'s own failure;
//! - a page `AnySite` could not reach is `412` with an empty array and the
//!   reason in `x-error` — and it is still charged (a render: 10 credits).

use std::time::Duration;

use serde_json::Value;

/// Where `AnySite` answers.
pub const BASE_URL: &str = "https://api.anysite.io";

/// How long one call may take. A render that ended in `AnySite`'s own
/// "not accessible" took 77 s (recorded `tegeldepot_search.render.json`);
/// successful renders took 15–20 s and parses 1–3 s. Waiting past the
/// slowest answer measured lets `AnySite` say why instead of our clock.
pub const TIMEOUT: Duration = Duration::from_secs(90);

/// The endpoints this service uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// `ikea/products`: an IKEA product by link, 5 credits.
    IkeaProduct,
    /// `webparser/parse`: a page as served, 1 credit.
    Parse,
    /// `webparser/render`: a page in a browser, 10 credits.
    Render,
}

impl Endpoint {
    /// The name `AnySite`'s tools and documents use.
    pub fn name(self) -> &'static str {
        match self {
            Self::IkeaProduct => "ikea/products",
            Self::Parse => "webparser/parse",
            Self::Render => "webparser/render",
        }
    }
}

/// Why a call brought nothing back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// `401`: the key is wrong or spent.
    Refused,
    /// `412`: `AnySite` could not reach the page, with its reason.
    NotAccessible(String),
    /// `422`: the request did not fit the endpoint.
    Invalid(String),
    /// `429`: too many calls.
    Busy,
    /// Any other status, `AnySite`'s `5xx` among them.
    Status(u16),
    /// No answer: the network, or past [`TIMEOUT`].
    Network(String),
    /// `200` with no item: nothing there.
    Nothing,
}

impl FetchError {
    /// The page itself brought nothing — it could not be reached, or it was
    /// empty — which a browser render may still read. Any other failure is
    /// `AnySite`'s or ours, and buying a second call does not answer it.
    pub fn is_the_page(&self) -> bool {
        matches!(self, Self::NotAccessible(_) | Self::Nothing)
    }

    /// `AnySite` itself failed (busy, down, too slow): the same call is
    /// worth making again later.
    pub fn is_outage(&self) -> bool {
        matches!(self, Self::Busy | Self::Status(_) | Self::Network(_))
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused => f.write_str("AnySite refused this server's key"),
            Self::NotAccessible(why) => write!(f, "AnySite could not reach the page: {why}"),
            Self::Invalid(why) => write!(f, "AnySite did not accept the request: {why}"),
            Self::Busy => f.write_str("AnySite is limiting calls; try again in a minute"),
            Self::Status(code) => write!(f, "AnySite answered {code}"),
            Self::Network(why) => write!(f, "AnySite did not answer: {why}"),
            Self::Nothing => f.write_str("AnySite found nothing at that link"),
        }
    }
}

/// One item `AnySite` returned, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub item: Value,
    pub credits: Option<u32>,
}

/// A failed call, and what it cost all the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub error: FetchError,
    pub credits: Option<u32>,
}

/// `AnySite`, for one key.
#[derive(Clone)]
pub struct AnySite {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl std::fmt::Debug for AnySite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AnySite {{ base: {:?}, key: <hidden> }}", self.base)
    }
}

impl AnySite {
    /// `AnySite` at `base` (no trailing slash needed) with `key`.
    ///
    /// # Panics
    ///
    /// If the HTTP client cannot be built (no TLS backend), a build problem.
    pub fn new(base: &str, key: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("3d-new-era-ai/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("an HTTP client");
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
        }
    }

    /// Calls `endpoint` with `body`, and hands back its first item.
    pub async fn call(&self, endpoint: Endpoint, body: &Value) -> Result<Answer, Failure> {
        let url = format!("{}/api/{}", self.base, endpoint.name());
        let sent = self
            .http
            .post(url)
            .header("access-token", &self.key)
            .json(body)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(err) => {
                return Err(Failure {
                    error: FetchError::Network(err.without_url().to_string()),
                    credits: None,
                });
            }
        };
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let credits = header("x-credits-charged").and_then(|c| c.parse().ok());
        let reason = header("x-error");
        let fail = |error| Err(Failure { error, credits });
        let body: Value = response.json().await.unwrap_or(Value::Null);
        match status {
            200 => match body {
                Value::Array(mut items) if !items.is_empty() => Ok(Answer {
                    item: items.swap_remove(0),
                    credits,
                }),
                _ => fail(FetchError::Nothing),
            },
            401 | 403 => fail(FetchError::Refused),
            412 => fail(FetchError::NotAccessible(
                reason.unwrap_or_else(|| "no reason given".to_owned()),
            )),
            422 => fail(FetchError::Invalid(detail(&body))),
            429 => fail(FetchError::Busy),
            other => fail(FetchError::Status(other)),
        }
    }
}

/// `FastAPI`'s `detail`: a sentence, or the list of what did not fit.
fn detail(body: &Value) -> String {
    match &body["detail"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
