//! Runs the hosted service.
//!
//!   `DATABASE_URL=postgres://… RESEND_API_KEY=… newera-cloud`
//!
//! See `newera_cloud::Config` for the rest of the environment.

use anyhow::Context as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `--health` is how the container checks itself: same binary, no shell.
    if std::env::args().any(|arg| arg == "--health") {
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(7979);
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        stream
            .write_all(b"GET /cloud/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .await?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await?;
        anyhow::ensure!(answer.contains("\"ok\":true"), "unhealthy: {answer}");
        println!("ok");
        return Ok(());
    }
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = newera_cloud::Config::from_env()?;
    let db = newera_cloud::database(&config.database_url)
        .await
        .context("database")?;
    let port = config.port;
    let state = newera_cloud::AppState::start(config, db.clone()).await?;

    // Furniture models generated from their photos, jobs resumed from the
    // table.
    let _generation = newera_cloud::generation::start(state.clone());

    // Projects nobody touched for half an hour leave memory; they are kept.
    let engine = state.engine.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_mins(5));
        loop {
            every.tick().await;
            engine.sweep(std::time::Duration::from_mins(30));
        }
    });

    // Closed accounts and expired secrets go, once an hour.
    tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            every.tick().await;
            match newera_cloud::accounts::purge_closed(&db).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("purged {n} closed accounts"),
                Err(err) => tracing::warn!("purge failed: {err}"),
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("newera-cloud on http://0.0.0.0:{port}");
    axum::serve(
        listener,
        newera_cloud::router(&state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
