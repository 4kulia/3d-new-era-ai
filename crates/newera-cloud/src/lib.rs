//! The hosted 3D New Era AI.
//!
//! One service behind `https://mcp.3dneweraai.com`:
//!
//! - **the relay**, unchanged: a browser tab opens a room and anybody holding
//!   the room's secret URL reaches it — no account, as before;
//! - **accounts**, signed into with an emailed link (or Google), each on a
//!   plan whose limits the tools respect;
//! - **OAuth 2.1** for AI clients (Claude, `ChatGPT`, Codex…): dynamic
//!   registration and client metadata documents, PKCE, rotating refresh
//!   tokens;
//! - **one fixed MCP address**, `/mcp`, that reaches the account's own tab
//!   when one is open and signed in.
//!
//! Secrets are stored only as hashes. A project kept in the cloud is
//! stored once, at a revision: the engine and the editor tab both save it
//! back on the revision they loaded, and a save from a stale one is refused.

pub mod account;
pub mod accounts;
pub mod billing;
pub mod config;
pub mod engine;
pub mod files;
pub mod limits;
pub mod login;
pub mod mail;
pub mod mcp;
pub mod oauth;
pub mod pages;
pub mod products;
pub mod secret;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use sqlx::PgPool;

pub use config::Config;

/// What every handler shares.
#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub rooms: newera_relay::Rooms,
    pub http: reqwest::Client,
    pub limits: limits::Limiter,
    pub engine: Arc<engine::Engine>,
    /// Product fetches running apart from the requests that asked for them.
    pub flights: products::Flights,
}

impl AppState {
    /// The state at startup: the database brought in line with the list of
    /// addresses the installation lets in, then the state for it.
    ///
    /// # Errors
    ///
    /// When the database fails.
    ///
    /// # Panics
    ///
    /// As `new`.
    pub async fn start(config: Config, db: PgPool) -> sqlx::Result<Self> {
        accounts::sync_given_plan(&db, &config.allowed_emails).await?;
        Ok(Self::new(config, db))
    }

    /// The state for a configuration and a database already migrated.
    ///
    /// # Panics
    ///
    /// If the HTTP client cannot be built (no TLS backend), which is a build
    /// problem, or the engine's directory cannot be made.
    pub fn new(config: Config, db: PgPool) -> Self {
        let engine = Arc::new(engine::Engine::new().expect("the engine's directory"));
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("3d-new-era-ai/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("an HTTP client");
        // The editor this service names is let into the relay too.
        let rooms = newera_relay::Rooms::allowing(config.site_origins.clone());
        Self {
            config: Arc::new(config),
            db,
            rooms,
            http,
            limits: limits::Limiter::default(),
            engine,
            flights: products::Flights::default(),
        }
    }
}

/// Connects to the database and brings its schema up to date.
///
/// # Errors
///
/// When the database cannot be reached or a migration fails.
pub async fn database(url: &str) -> anyhow::Result<PgPool> {
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    Ok(db)
}

/// Every route of the service.
pub fn router(state: &AppState) -> Router {
    use axum::http::{HeaderValue, Method, header};
    use axum::routing::post;
    use tower_http::cors::CorsLayer;

    // The editor at 3dneweraai.com asks who is signed in and claims its room,
    // with the person's cookie: only the site's own origins may.
    let origins: Vec<HeaderValue> = state
        .config
        .site_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    let site = Router::new()
        .route("/account/me", get(account::me))
        .route("/account/claim", post(account::claim))
        .route(
            "/account/projects/{id}",
            get(account::download).put(account::upload),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_credentials(true)
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers([header::CONTENT_TYPE, account::REVISION])
                // The editor names a cloud project by the file name it comes
                // with, and saves it back on the revision it came at.
                .expose_headers([header::CONTENT_DISPOSITION, account::REVISION]),
        );
    let cloud = Router::new()
        .route("/", get(home))
        .route("/cloud/health", get(health))
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth::server_metadata),
        )
        .route(
            "/.well-known/openid-configuration",
            get(oauth::server_metadata),
        )
        .route("/oauth/register", post(oauth::register))
        .route(
            "/oauth/authorize",
            get(oauth::authorize).post(oauth::decide),
        )
        .route("/oauth/token", post(oauth::token))
        .route("/oauth/revoke", post(oauth::revoke))
        .route("/login", get(login::form).post(login::send))
        .route("/login/link", get(login::confirm).post(login::redeem))
        // Asked by the proxy in front of the editor: its auth subrequest is
        // always a GET, whatever the request it guards was.
        .route("/login/check", axum::routing::any(login::check))
        // Where the proxy sends a turned-away browser, with the original
        // request's method: any method must reach the redirect.
        .route("/login/required", axum::routing::any(login::required))
        .route("/login/google", get(login::google))
        .route("/login/google/callback", get(login::google_callback))
        .route("/logout", get(login::logout))
        .route("/account", get(account::show))
        .route("/account/close", post(account::close))
        .route("/account/done", get(account::done))
        .route("/mcp", post(mcp::post).get(mcp::get))
        .route("/files/{token}", get(files::get))
        .route("/billing/buymeacoffee", post(billing::webhook))
        .route("/billing/checkout", get(billing::checkout))
        .merge(site)
        .with_state(state.clone());
    // With a list of owners, the relay is theirs: opening a room needs a
    // listed session, so the editor cannot be driven through this server by
    // somebody else's page. Without a list the relay is open as before.
    let relay = newera_relay::router_with(state.rooms.clone()).layer(
        axum::middleware::from_fn_with_state(state.clone(), rooms_for_owners),
    );
    relay.merge(cloud)
}

/// Turns away `POST /rooms` from a browser without a listed session when
/// the installation has a list of owners.
async fn rooms_for_owners(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let opens = request.method() == axum::http::Method::POST && request.uri().path() == "/rooms";
    if opens
        && state.config.allowed_emails.list().is_some()
        && login::signed_in(&state, request.headers()).await.is_none()
    {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
        )
            .into_response();
    }
    next.run(request).await
}

/// The service's own address, opened in a browser: the site, which says
/// what this is, its prices and its terms.
async fn home(
    axum::extract::State(app): axum::extract::State<AppState>,
) -> axum::response::Redirect {
    let site = app
        .config
        .site_origins
        .first()
        .map_or("https://3dneweraai.com", String::as_str);
    axum::response::Redirect::permanent(&format!("{site}/"))
}

/// Healthy means the database answers: the deploy checks this, and rolls
/// back a version whose service cannot reach its data.
async fn health(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    let db = sqlx::query_scalar::<_, i32>("select 1")
        .fetch_one(&state.db)
        .await
        .is_ok();
    let status = if db {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        axum::Json(serde_json::json!({ "ok": db, "database": db })),
    )
}
