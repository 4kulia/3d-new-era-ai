//! A furniture product's 3D model, made from its photo by the generator on
//! the private network (`deploy/spark-generator/`, `config::Generator`).
//!
//! Importing a piece with a photo, or `edit_products action=generate`, puts
//! a job in `product_jobs` ([`enqueue`]); a product has at most one open.
//! A worker in this process ([`start`]) hands each job to the generator with
//! the photo and the shop's width, depth and height, asks after it, and
//! keeps what it makes as the product's model (`models::keep`, source
//! `generated`), which replaces the placeholder where the product is drawn
//! now (`staging::swap`). A generator that is unreachable, loses the job or
//! fails on it is tried again after a wait that doubles each time, up to
//! [`ATTEMPTS`]; the jobs live in the table, so a restart of this service
//! picks them up where they were.
//!
//! The generator is on the tailnet, not the public internet: it is called
//! with a client of its own, not through `models::download`, which reaches
//! only public addresses.

use std::sync::Arc;
use std::time::Duration;

use newera_shop::product::Fields;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::types::Json;
use tokio::sync::Notify;

use crate::config::Generator;
use crate::models::{self, Source};
use crate::{AppState, secret};

/// What the generated models are recorded as made by: Hunyuan3D-2.0 with
/// its paint, metallic off — variant M of the comparison the operator chose.
pub const GENERATOR: &str = "hunyuan3d-2.0-m";

/// How many tries a job gets before it is failed for good. A chosen bound:
/// with `config::GENERATOR_BACKOFF` the waits between them add up to about
/// a quarter of an hour.
pub const ATTEMPTS: i32 = 5;

/// How long one request to the generator may take: a status, or a model of
/// up to `models::MODEL_LIMIT` over the private network. A chosen bound.
const REQUEST_WAIT: Duration = Duration::from_secs(60);

/// Rung when a job is put in, so the worker looks at once.
#[derive(Debug, Clone, Default)]
pub struct Wake(Arc<Notify>);

impl Wake {
    pub fn ring(&self) {
        self.0.notify_one();
    }
}

/// The product's latest job as `products` shows it: `{id, status, error?,
/// attempts, updated}`.
///
/// # Errors
///
/// When the database fails.
pub async fn latest(db: &PgPool, product: &str) -> sqlx::Result<Option<Value>> {
    let row: Option<(String, String, Option<String>, i32, i64)> = sqlx::query_as(
        "select id, status, error, attempts, extract(epoch from updated_at)::bigint
         from product_jobs where product_id = $1 order by created_at desc limit 1",
    )
    .bind(product)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(id, status, error, attempts, updated)| {
        let mut job = json!({"id": id, "status": status, "attempts": attempts, "updated": updated});
        if let Some(error) = error {
            job["error"] = json!(error);
        }
        job
    }))
}

/// Puts in a job for `product` unless one is open, and answers the open
/// one's id.
///
/// # Errors
///
/// When the database fails.
pub async fn enqueue(app: &AppState, account: &str, product: &str) -> anyhow::Result<String> {
    let made: Option<String> = sqlx::query_scalar(
        "insert into product_jobs (id, account_id, product_id) values ($1, $2, $3)
         on conflict (product_id) where status in ('queued', 'running') do nothing
         returning id",
    )
    .bind(secret::id())
    .bind(account)
    .bind(product)
    .fetch_optional(&app.db)
    .await?;
    let id = match made {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "select id from product_jobs
                 where product_id = $1 and status in ('queued', 'running')",
            )
            .bind(product)
            .fetch_one(&app.db)
            .await?
        }
    };
    app.jobs.ring();
    Ok(id)
}

/// The product's width, depth and height in cm, when all three are known:
/// a model is made only at the shop's size.
#[must_use]
pub fn size(fields: &Fields) -> Option<[f64; 3]> {
    match models::dimensions(fields) {
        [Some(w), Some(d), Some(h)] => Some([w, d, h]),
        _ => None,
    }
}

/// The product's photo, the link the generator fetches it from.
#[must_use]
pub fn photo(fields: &Fields) -> Option<&str> {
    fields
        .get("image")
        .and_then(|f| f.value.as_str())
        .filter(|link| link.starts_with("https://"))
}

/// Starts the worker, when a generator is configured. It runs until the
/// process ends, or the handle is aborted.
///
/// # Panics
///
/// If the HTTP client cannot be built (no TLS backend), a build problem.
#[must_use]
pub fn start(app: AppState) -> Option<tokio::task::JoinHandle<()>> {
    let generator = app.config.generator.clone()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(REQUEST_WAIT)
        .user_agent(concat!("3d-new-era-ai/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("an HTTP client");
    Some(tokio::spawn(async move {
        loop {
            if let Err(err) = tick(&app, &generator, &client).await {
                tracing::warn!("3D jobs: {err:#}");
            }
            tokio::select! {
                () = app.jobs.0.notified() => {}
                () = tokio::time::sleep(generator.poll) => {}
            }
        }
    }))
}

#[derive(Debug, sqlx::FromRow)]
struct Open {
    id: String,
    account_id: String,
    product_id: String,
    provider_job: Option<String>,
    status: String,
    attempts: i32,
    /// Seconds since the generator was given the job or last moved it on.
    held_s: f64,
}

/// Takes every open job that is due one step further.
async fn tick(
    app: &AppState,
    generator: &Generator,
    client: &reqwest::Client,
) -> anyhow::Result<()> {
    let due: Vec<Open> = sqlx::query_as(
        "select id, account_id, product_id, provider_job, status, attempts,
                extract(epoch from now() - state_since)::float8 as held_s
         from product_jobs
         where status in ('queued', 'running') and next_at <= now()
         order by created_at",
    )
    .fetch_all(&app.db)
    .await?;
    for job in due {
        let step = match &job.provider_job {
            None => submit(app, generator, client, &job).await,
            Some(asked) => poll(app, generator, client, &job, asked).await,
        };
        if let Err(err) = step {
            tracing::warn!("3D job {}: {err:#}", job.id);
        }
    }
    Ok(())
}

/// What the generator said of a job it was asked after.
#[derive(Debug)]
enum Heard {
    Model(Vec<u8>),
    Waiting(String),
    Failed { why: String, permanent: bool },
    Lost,
    Trouble(String),
}

async fn submit(
    app: &AppState,
    generator: &Generator,
    client: &reqwest::Client,
    job: &Open,
) -> anyhow::Result<()> {
    let fields: Option<Json<Fields>> =
        sqlx::query_scalar("select fields from products where id = $1")
            .bind(&job.product_id)
            .fetch_optional(&app.db)
            .await?;
    let Some(Json(fields)) = fields else {
        return Ok(());
    };
    let Some(image) = photo(&fields) else {
        return fail(
            &app.db,
            job,
            "the product has no photo: edit_products action=set fields image (an https link), then action=generate",
        )
        .await;
    };
    let Some([w, d, h]) = size(&fields) else {
        return fail(
            &app.db,
            job,
            "the product has no width, depth and height to make its model at: edit_products action=set fields width_cm, depth_cm, height_cm, then action=generate",
        )
        .await;
    };
    let asked = json!({"image_url": image, "size_cm": [w, d, h]});
    let sent = client
        .post(format!("{}/generate", generator.url))
        .header("X-Generator-Token", &generator.token)
        .json(&asked)
        .send()
        .await;
    let response = match sent {
        Ok(response) => response,
        Err(err) => {
            return retry(
                app,
                generator,
                job,
                &format!("the generator is unreachable: {err}"),
                true,
            )
            .await;
        }
    };
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status == reqwest::StatusCode::ACCEPTED
        && let Some(provider) = body["job"].as_str()
    {
        sqlx::query(
            "update product_jobs set provider_job = $2, status = 'queued',
                 state_since = now(), updated_at = now()
             where id = $1",
        )
        .bind(&job.id)
        .bind(provider)
        .execute(&app.db)
        .await?;
        return Ok(());
    }
    let said = body["error"]
        .as_str()
        .map_or_else(|| body.to_string(), str::to_owned);
    if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        let what = if body["field"] == "size_cm" {
            format!("the size {w}×{d}×{h} cm")
        } else {
            format!("the photo {image}")
        };
        return fail(
            &app.db,
            job,
            &format!("the generator refused {what}: {said}"),
        )
        .await;
    }
    retry(
        app,
        generator,
        job,
        &format!("the generator answered {status}: {said}"),
        true,
    )
    .await
}

async fn ask_after(generator: &Generator, client: &reqwest::Client, provider: &str) -> Heard {
    let response = match client
        .get(format!("{}/jobs/{provider}", generator.url))
        .header("X-Generator-Token", &generator.token)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => return Heard::Trouble(format!("the generator is unreachable: {err}")),
    };
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Heard::Lost;
    }
    let glb = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .is_some_and(|t| t.as_bytes().starts_with(b"model/gltf-binary"));
    if !status.is_success() {
        let said = response.text().await.unwrap_or_default();
        return Heard::Trouble(format!("the generator answered {status}: {said}"));
    }
    if glb {
        return match response.bytes().await {
            Ok(bytes) => Heard::Model(bytes.to_vec()),
            Err(err) => Heard::Trouble(format!("the model's download broke off: {err}")),
        };
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    match body["status"].as_str() {
        Some(state @ ("queued" | "running")) => Heard::Waiting(state.to_owned()),
        Some("failed") => Heard::Failed {
            why: body["error"]
                .as_str()
                .unwrap_or("the generator failed without saying why")
                .to_owned(),
            permanent: body["permanent"] == true,
        },
        _ => Heard::Trouble(format!("the generator answered {body}")),
    }
}

async fn poll(
    app: &AppState,
    generator: &Generator,
    client: &reqwest::Client,
    job: &Open,
    provider: &str,
) -> anyhow::Result<()> {
    match ask_after(generator, client, provider).await {
        Heard::Model(bytes) => finish(app, job, bytes).await,
        Heard::Waiting(state) if state != job.status => {
            sqlx::query(
                "update product_jobs set status = $2, error = null, state_since = now(), updated_at = now()
                 where id = $1",
            )
            .bind(&job.id)
            .bind(state)
            .execute(&app.db)
            .await?;
            Ok(())
        }
        // Only a job it is running can be stuck: one waiting its turn
        // behind others is not, and giving it again would make it twice.
        Heard::Waiting(state)
            if state == "running" && job.held_s > generator.stall.as_secs_f64() =>
        {
            retry(
                app,
                generator,
                job,
                &format!(
                    "the generator has been running the job for over {} minutes without an answer",
                    generator.stall.as_secs() / 60
                ),
                false,
            )
            .await
        }
        Heard::Waiting(_) => {
            sqlx::query(
                "update product_jobs set error = null, updated_at = now()
                 where id = $1 and error is not null",
            )
            .bind(&job.id)
            .execute(&app.db)
            .await?;
            Ok(())
        }
        Heard::Failed {
            why,
            permanent: true,
        } => {
            fail(
                &app.db,
                job,
                &format!("the generator could not use the photo: {why}"),
            )
            .await
        }
        Heard::Failed { why, .. } => {
            retry(
                app,
                generator,
                job,
                &format!("the generator failed: {why}"),
                false,
            )
            .await
        }
        Heard::Lost => {
            retry(
                app,
                generator,
                job,
                "the generator no longer has the job (it restarted)",
                false,
            )
            .await
        }
        Heard::Trouble(why) => retry(app, generator, job, &why, true).await,
    }
}

/// The wait before the next try after `failed` tries.
#[must_use]
pub fn backoff(first: Duration, failed: i32) -> Duration {
    let doublings = u32::try_from(failed.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
    first.saturating_mul(1 << doublings)
}

/// One more try spent: the job waits and goes again — to the job the
/// generator has when `keep` (the generator was not reached), else as a
/// new one — or, with every try spent, fails.
async fn retry(
    app: &AppState,
    generator: &Generator,
    job: &Open,
    why: &str,
    keep: bool,
) -> anyhow::Result<()> {
    let attempts = job.attempts + 1;
    if attempts >= ATTEMPTS {
        sqlx::query(
            "update product_jobs set status = 'failed', attempts = $2, error = $3, updated_at = now()
             where id = $1",
        )
        .bind(&job.id)
        .bind(attempts)
        .bind(format!("{why}; gave up after {attempts} tries"))
        .execute(&app.db)
        .await?;
        return Ok(());
    }
    let wait = backoff(generator.backoff, attempts);
    sqlx::query(
        "update product_jobs set attempts = $2, error = $3, status = 'queued',
             provider_job = case when $4 then provider_job end,
             next_at = now() + make_interval(secs => $5), updated_at = now()
         where id = $1",
    )
    .bind(&job.id)
    .bind(attempts)
    .bind(format!("{why}; trying again"))
    .bind(keep)
    .bind(wait.as_secs_f64())
    .execute(&app.db)
    .await?;
    Ok(())
}

/// Fails the job for good: no try would go otherwise.
async fn fail(db: &PgPool, job: &Open, why: &str) -> anyhow::Result<()> {
    sqlx::query(
        "update product_jobs set status = 'failed', error = $2, updated_at = now() where id = $1",
    )
    .bind(&job.id)
    .bind(why)
    .execute(db)
    .await?;
    Ok(())
}

/// Keeps the model made as the product's, where its plan has room, marks
/// the job done with it in one transaction, then draws it where the product
/// is drawn now: a swap that fails, or a restart before it, leaves the job
/// done and the model kept. A model attached after the job was asked for is
/// the agent's word and stays, whatever generator it names; only the model
/// kept by this job (`product_assets.job_id`) is its own.
async fn finish(app: &AppState, job: &Open, bytes: Vec<u8>) -> anyhow::Result<()> {
    let attached: Option<String> = sqlx::query_scalar(
        "select source from product_assets
         where product_id = $1 and kind = 'model' and source <> 'placeholder'
           and job_id is distinct from $2
           and created_at > (select created_at from product_jobs where id = $2)",
    )
    .bind(&job.product_id)
    .bind(&job.id)
    .fetch_optional(&app.db)
    .await?;
    if attached.is_some() {
        return fail(
            &app.db,
            job,
            "a model was attached while it was being made, and stays: edit_products action=generate makes a new one in its place",
        )
        .await;
    }
    let plan = crate::accounts::plan_of(&app.db, &job.account_id).await?;
    let kept =
        crate::engine::kept_bytes(&app.db, &job.account_id, None, Some(&job.product_id)).await?;
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if let Some(why) = crate::engine::no_room(&plan, kept.saturating_add(size)) {
        return fail(&app.db, job, &format!("the model was not kept: {why}")).await;
    }
    let mut tx = app.db.begin().await?;
    let bytes = match models::keep(
        &mut *tx,
        &job.product_id,
        bytes,
        Source::Generated,
        Some(GENERATOR),
    )
    .await?
    {
        Ok(bytes) => bytes,
        Err(why) => {
            drop(tx);
            return fail(
                &app.db,
                job,
                &format!("the generator made no model the editor reads: {why}"),
            )
            .await;
        }
    };
    sqlx::query("update product_assets set job_id = $2 where product_id = $1 and kind = 'model'")
        .bind(&job.product_id)
        .bind(&job.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "update product_jobs set status = 'done', error = null, updated_at = now() where id = $1",
    )
    .bind(&job.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    match crate::staging::swap(app, &job.account_id, &plan, &job.product_id, &bytes).await {
        Ok(drawn) => tracing::info!("3D job {} kept for {}: {drawn}", job.id, job.product_id),
        Err(err) => tracing::warn!(
            "3D job {} kept for {}, not drawn where it is open: {err:#}",
            job.id,
            job.product_id
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn each_retry_waits_twice_as_long_as_the_last() {
        let first = Duration::from_secs(60);
        let waits: Vec<u64> = (1..=super::ATTEMPTS)
            .map(|n| super::backoff(first, n).as_secs())
            .collect();
        assert_eq!(waits, [60, 120, 240, 480, 960]);
        assert_eq!(super::backoff(first, 0).as_secs(), 60);
    }
}
