//! Accounts, the browsers signed into them, and the plan each one is on.

use serde::Serialize;
use sqlx::PgPool;

use crate::config::AllowedEmails;
use crate::secret;

/// How long a browser stays signed in.
pub const SESSION_DAYS: i32 = 30;

/// Someone who signed in, by the address they proved they read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Account {
    pub id: String,
    pub email: String,
}

/// The plan an account is on, and what it allows.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub code: String,
    pub name: String,
    pub limits: serde_json::Value,
}

/// The plan of the addresses an installation lets in, when it lets in only
/// those it lists.
pub const OWNER_PLAN: &str = "owner";

/// The account for an address, made the first time the address signs in.
/// `None` when the address may not hold one here; an address `allowed`
/// lists is put on the owner plan.
///
/// Every account is made here, so this is what keeps an installation that
/// is someone's own to its owners, whichever way an address arrives.
///
/// # Errors
///
/// When the database fails.
pub async fn find_or_create(
    db: &PgPool,
    allowed: &AllowedEmails,
    email: &str,
) -> sqlx::Result<Option<Account>> {
    let email = email.trim();
    if !allowed.admits(email) {
        return Ok(None);
    }
    let account = find_or_insert(db, email).await?;
    if allowed.lists(email) {
        sqlx::query(
            "update accounts set given_plan = $2
             where id = $1 and given_plan is distinct from $2",
        )
        .bind(&account.id)
        .bind(OWNER_PLAN)
        .execute(db)
        .await?;
    }
    Ok(Some(account))
}

/// Brings the plans given by the installation in line with its list: the
/// accounts it lists hold the owner plan, and an account holding it that is
/// no longer listed (or when there is no list) does not.
///
/// # Errors
///
/// When the database fails.
pub async fn sync_given_plan(db: &PgPool, allowed: &AllowedEmails) -> sqlx::Result<()> {
    let listed = allowed.list().unwrap_or_default().to_vec();
    let mut tx = db.begin().await?;
    sqlx::query(
        "update accounts set given_plan = $2
         where deleted_at is null and lower(email) = any($1)
           and given_plan is distinct from $2",
    )
    .bind(&listed)
    .bind(OWNER_PLAN)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update accounts set given_plan = null
         where given_plan = $2 and not (lower(email) = any($1))",
    )
    .bind(&listed)
    .bind(OWNER_PLAN)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn find_or_insert(db: &PgPool, email: &str) -> sqlx::Result<Account> {
    if let Some(found) = sqlx::query_as::<_, Account>(
        "select id, email from accounts where lower(email) = lower($1) and deleted_at is null",
    )
    .bind(email)
    .fetch_optional(db)
    .await?
    {
        return Ok(found);
    }
    // Two first sign-ins at once: the unique index lets one win, and the other
    // reads what it made.
    sqlx::query(
        "insert into accounts (id, email) values ($1, $2)
         on conflict (lower(email)) where deleted_at is null do nothing",
    )
    .bind(secret::id())
    .bind(email)
    .execute(db)
    .await?;
    sqlx::query_as::<_, Account>(
        "select id, email from accounts where lower(email) = lower($1) and deleted_at is null",
    )
    .bind(email)
    .fetch_one(db)
    .await
}

/// Whether this address had an account here and closed it, and has not
/// opened another since.
///
/// What it guards: an address is free again the moment its account closes, so
/// anything that would make an account for an address — a payment arriving
/// for one, above all — has to ask first, or a deletion someone asked for
/// quietly comes back as a new account.
///
/// # Errors
///
/// When the database fails.
pub async fn only_closed(db: &PgPool, email: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select not exists (
             select 1 from accounts where lower(email) = lower($1) and deleted_at is null
         ) and exists (
             select 1 from accounts where lower(email) = lower($1) and deleted_at is not null
         )",
    )
    .bind(email.trim())
    .fetch_one(db)
    .await
}

/// A live account by id.
///
/// # Errors
///
/// When the database fails.
pub async fn get(db: &PgPool, id: &str) -> sqlx::Result<Option<Account>> {
    sqlx::query_as::<_, Account>(
        "select id, email from accounts where id = $1 and deleted_at is null",
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

/// The plan that applies now: the one the installation gave the account,
/// else an active subscription's, else the free one.
///
/// # Errors
///
/// When the database fails.
pub async fn plan_of(db: &PgPool, account_id: &str) -> sqlx::Result<Plan> {
    let row: (String, String, serde_json::Value) = sqlx::query_as(
        "select p.code, p.name, p.limits from plans p
         where p.code = coalesce((
             select a.given_plan from accounts a where a.id = $1
         ), (
             select s.plan from subscriptions s
             where s.account_id = $1
               and s.status in ('active', 'trialing')
               and (s.current_period_end is null or s.current_period_end > now())
         ), 'free')",
    )
    .bind(account_id)
    .fetch_one(db)
    .await?;
    Ok(Plan {
        code: row.0,
        name: row.1,
        limits: row.2,
    })
}

/// Signs a browser in: the cookie's value, which only the browser keeps.
///
/// # Errors
///
/// When the database fails.
pub async fn open_session(db: &PgPool, account_id: &str) -> sqlx::Result<String> {
    let token = secret::token();
    sqlx::query(
        "insert into sessions (token_hash, account_id, expires_at)
         values ($1, $2, now() + make_interval(days => $3))",
    )
    .bind(secret::hash(&token))
    .bind(account_id)
    .bind(SESSION_DAYS)
    .execute(db)
    .await?;
    Ok(token)
}

/// The account a browser's cookie belongs to, if it is still good and the
/// installation still lets its address in.
///
/// # Errors
///
/// When the database fails.
pub async fn session(
    db: &PgPool,
    allowed: &AllowedEmails,
    token: &str,
) -> sqlx::Result<Option<Account>> {
    sqlx::query_as::<_, Account>(
        "select a.id, a.email from sessions s join accounts a on a.id = s.account_id
         where s.token_hash = $1 and s.expires_at > now() and a.deleted_at is null
           and ($2::text[] is null or lower(a.email) = any($2))",
    )
    .bind(secret::hash(token))
    .bind(allowed.list())
    .fetch_optional(db)
    .await
}

/// Signs a browser out.
///
/// # Errors
///
/// When the database fails.
pub async fn close_session(db: &PgPool, token: &str) -> sqlx::Result<()> {
    sqlx::query("delete from sessions where token_hash = $1")
        .bind(secret::hash(token))
        .execute(db)
        .await?;
    Ok(())
}

/// Closes an account: it stops signing in at once, every browser and AI
/// client it had is let go, and the address is free for a new account.
/// The row and what hangs on it are purged by `purge_closed` after 30 days,
/// as the privacy policy says.
///
/// # Errors
///
/// When the database fails.
pub async fn close(db: &PgPool, account_id: &str) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("update accounts set deleted_at = now() where id = $1")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from sessions where account_id = $1")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "update oauth_tokens set revoked_at = now() where account_id = $1 and revoked_at is null",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Removes accounts closed more than 30 days ago, with everything that
/// references them, and secrets that expired.
///
/// # Errors
///
/// When the database fails.
pub async fn purge_closed(db: &PgPool) -> sqlx::Result<u64> {
    let gone = sqlx::query("delete from accounts where deleted_at < now() - interval '30 days'")
        .execute(db)
        .await?
        .rows_affected();
    sqlx::query("delete from sessions where expires_at < now()")
        .execute(db)
        .await?;
    sqlx::query("delete from login_links where expires_at < now() - interval '1 day'")
        .execute(db)
        .await?;
    sqlx::query("delete from oauth_codes where expires_at < now() - interval '1 day'")
        .execute(db)
        .await?;
    sqlx::query("delete from oauth_tokens where expires_at < now() - interval '1 day'")
        .execute(db)
        .await?;
    sqlx::query("delete from files where expires_at < now()")
        .execute(db)
        .await?;
    Ok(gone)
}
