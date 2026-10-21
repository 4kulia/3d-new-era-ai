//! Paid plans, supported through Buy Me a Coffee.
//!
//! Buy Me a Coffee sells the membership on a page of its own and pays out
//! through the Stripe account connected to it. This service does two things
//! around that:
//!
//! - the support page (`/billing/checkout`) says what the plan gives and
//!   sends the person to the Buy Me a Coffee membership page;
//! - the webhook (`/billing/buymeacoffee`) hears each membership event and
//!   makes the account's plan follow it.
//!
//! What the person pays with is an address, not an account id: Buy Me a
//! Coffee's page carries nothing of ours through the checkout, so the
//! `supporter_email` of the event is the only thing tying a payment to an
//! account. The support page asks for the account's own address, and an
//! address that has no account yet gets one, the same one the sign-in link
//! would make.
//!
//! Two things Paddle did and Buy Me a Coffee cannot: there is no customer
//! portal to open, and no way for us to cancel a membership — the member
//! cancels it on Buy Me a Coffee. The account page says so.
//!
//! The tools never ask "does this account pay?": they read the plan's
//! limits, and this is what changes the plan. Nothing here is shown inside an
//! AI chat: the directories forbid selling there.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;

use crate::config::BuyMeACoffee;
use crate::login;
use crate::pages::{Lang, escape, page};
use crate::{AppState, accounts, secret};

/// The one paid plan; every membership of this page gives it.
pub const PAID_PLAN: &str = "supporter";

/// The events that say something about a plan: a membership, and the
/// "monthly support" that has no level behind it. Both carry the same
/// subscription fields.
const SUBSCRIPTION_EVENTS: &[&str] = &[
    "membership.started",
    "membership.updated",
    "membership.cancelled",
    "membership.paused",
    "recurring_donation.started",
    "recurring_donation.updated",
    "recurring_donation.cancelled",
    // Not in Buy Me a Coffee's own list of event types, but the dashboard
    // offers "Monthly support paused" and the webhook is subscribed to it.
    "recurring_donation.paused",
];

/// HMAC-SHA256, by the book (RFC 2104), on the hash the crate already uses.
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner_pad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let outer_pad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new()
        .chain_update(&inner_pad)
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(&outer_pad)
        .chain_update(inner)
        .finalize()
        .into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// The `x-signature-sha256` a delivery of `body` carries: HMAC-SHA256 of the
/// body as it arrived, under the webhook's signing secret, in hex. Here for
/// tests and for trying the endpoint by hand.
pub fn signature(secret: &str, body: &[u8]) -> String {
    hex(&hmac(secret.as_bytes(), body))
}

/// Whether a delivery is Buy Me a Coffee's.
///
/// Buy Me a Coffee signs the body and nothing else — no timestamp, so a
/// delivery cannot be told from a replay of itself. What guards against a
/// replay is the event's own `created`: an event older than the one already
/// written leaves the row alone.
pub fn verified(secret: &str, header: &str, body: &[u8]) -> bool {
    secret::same(header.trim(), &signature(secret, body))
}

/// Buy Me a Coffee's webhook endpoint.
pub async fn webhook(State(app): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(bmc) = app.config.buymeacoffee.as_ref() else {
        return StatusCode::NOT_FOUND;
    };
    let header = headers
        .get("x-signature-sha256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !verified(&bmc.webhook_secret, header, &body) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(event) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    let kind = event["type"].as_str().unwrap_or_default();
    // A test event from the dashboard arrives signed like any other. Only a
    // service told to take them lets one change a plan.
    if !event["live_mode"].as_bool().unwrap_or(true) && !bmc.test_events {
        tracing::info!("buymeacoffee {kind}: a test event, ignored");
        return StatusCode::ACCEPTED;
    }
    if !SUBSCRIPTION_EVENTS.contains(&kind) {
        // One-off coffees, extras, commissions: thank you, but no plan.
        return StatusCode::ACCEPTED;
    }
    // Every envelope carries when it was made, and that is the only thing
    // ordering one delivery against another. A delivery without it would sit
    // at the epoch, where it could neither be told from a replay of itself
    // nor be overtaken in the right direction.
    let Some(created) = event["created"].as_i64().filter(|at| *at > 0) else {
        tracing::warn!("buymeacoffee {kind}: no created timestamp, ignored");
        return StatusCode::ACCEPTED;
    };
    match subscription(&app, bmc, kind, &event["data"], created).await {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            tracing::error!("buymeacoffee {kind}: {err:#}");
            // Buy Me a Coffee retries a failure four more times and gives up
            // on a destination that keeps failing, so a shape this does not
            // know is accepted; only the database's own failures ask again.
            if err.downcast_ref::<sqlx::Error>().is_some() {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::ACCEPTED
            }
        }
    }
}

/// What the plan should be while this subscription is in this state.
///
/// A cancelled membership that is still inside the period the member paid
/// for stays `active` until `current_period_end` passes, which is what the
/// support page promises. `plan_of` reads both, so nothing has to run at the
/// end of the period for the plan to lapse.
fn status_of(kind: &str, data: &Value) -> &'static str {
    let flag = |name: &str| match &data[name] {
        Value::Bool(yes) => *yes,
        Value::String(text) => text == "true",
        _ => false,
    };
    if kind.ends_with(".paused") || flag("paused") {
        return "paused";
    }
    if kind.ends_with(".cancelled") || flag("canceled") {
        // Cancelled for the end of the period: paid for until then.
        if flag("cancel_at_period_end") {
            return "active";
        }
        return "canceled";
    }
    match data["status"].as_str() {
        Some("canceled") => "canceled",
        Some("paused") => "paused",
        _ => "active",
    }
}

/// Makes the account's plan what the membership says.
async fn subscription(
    app: &AppState,
    bmc: &BuyMeACoffee,
    kind: &str,
    data: &Value,
    created: i64,
) -> anyhow::Result<()> {
    // A page that sells more than this plan: only the levels named here pay
    // for it. Naming any of them narrows the plan to memberships at those
    // levels — a monthly support with no level behind it is not one of them,
    // and would otherwise buy the plan for whatever the payer chose.
    if !bmc.levels.is_empty() {
        let level = data["membership_level_id"].as_i64();
        if !level.is_some_and(|l| bmc.levels.contains(&l)) {
            tracing::info!("buymeacoffee {kind}: {level:?} is not a level of the paid plan");
            // A membership that used to be one of them and was moved down is
            // not simply none of our business: its row is still here saying
            // the account pays. Leaving it be would keep the plan running on a
            // level that no longer buys it — and every later event for that
            // membership, its cancellation included, would be dropped by this
            // same test. So the row follows the membership down.
            if let Some(id) = data["id"].as_i64().map(|id| id.to_string()) {
                sqlx::query(
                    "update subscriptions set status = 'canceled',
                         provider_event_at = to_timestamp($2::double precision), updated_at = now()
                     where provider = 'buymeacoffee' and provider_subscription = $1
                       and (provider_event_at is null
                            or provider_event_at <= to_timestamp($2::double precision))",
                )
                .bind(&id)
                .bind(created)
                .execute(&app.db)
                .await?;
            }
            return Ok(());
        }
    }
    let email = data["supporter_email"]
        .as_str()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .ok_or_else(|| anyhow::anyhow!("a membership with no supporter_email"))?;
    // An address whose only account here was closed. Making a new one would
    // undo a deletion the person asked for — and hand the paid plan to an
    // account they never opened — so the event is dropped. They still hold
    // the membership, and cancelling it is theirs to do.
    if accounts::only_closed(&app.db, email).await? {
        tracing::warn!("buymeacoffee {kind}: the address's account was closed; no plan given");
        return Ok(());
    }
    // An installation that is someone's own opens no account for a payer it
    // does not let in.
    let Some(account) =
        accounts::find_or_create(&app.db, &app.config.allowed_emails, email).await?
    else {
        tracing::warn!("buymeacoffee {kind}: the address is not let in here; no plan given");
        return Ok(());
    };
    let status = status_of(kind, data);
    let period_end = data["current_period_end"].as_i64();
    let supporter = data["supporter_id"].as_i64().map(|id| id.to_string());
    let subscription = data["id"].as_i64().map(|id| id.to_string());
    // An event older than the one already written is a replay, or a delivery
    // that arrived out of order: either way it must not undo the newer one.
    let applied = sqlx::query(
        "insert into subscriptions (account_id, plan, status, provider, provider_customer,
             provider_subscription, current_period_end, provider_event_at, updated_at)
         values ($1, $2, $3, 'buymeacoffee', $4, $5,
             to_timestamp($6::double precision), to_timestamp($7::double precision), now())
         on conflict (account_id) do update set
             plan = excluded.plan, status = excluded.status, provider = excluded.provider,
             provider_customer = excluded.provider_customer,
             provider_subscription = excluded.provider_subscription,
             current_period_end = excluded.current_period_end,
             provider_event_at = excluded.provider_event_at, updated_at = now()
         where subscriptions.provider_event_at is null
            or subscriptions.provider_event_at <= excluded.provider_event_at",
    )
    .bind(&account.id)
    .bind(PAID_PLAN)
    .bind(status)
    .bind(supporter)
    .bind(subscription)
    .bind(period_end)
    .bind(created)
    .execute(&app.db)
    .await?
    .rows_affected();
    if applied == 0 {
        tracing::info!("buymeacoffee {kind}: an older event than the one already written");
    } else {
        tracing::info!("plan {PAID_PLAN} ({status}) for an account");
    }
    Ok(())
}

/// What the account pays with, if it ever paid: `(provider, status)`.
///
/// Not only Buy Me a Coffee's. Accounts that subscribed through Paddle still
/// have their row, and still get charged by Paddle — whoever asks "is this
/// account being charged?" has to hear about those too, or someone closes an
/// account believing that stopped it.
async fn membership(app: &AppState, account: &str) -> sqlx::Result<Option<(String, String)>> {
    sqlx::query_as("select provider, status from subscriptions where account_id = $1")
        .bind(account)
        .fetch_optional(&app.db)
        .await
}

/// Where the membership is bought and managed.
fn membership_url(bmc: &BuyMeACoffee) -> String {
    format!("https://buymeacoffee.com/{}/membership", bmc.page)
}

/// What the account page offers about paying: support, or where to manage
/// the membership the account has. Empty when the paid plan is off.
pub async fn account_links(app: &AppState, account: &accounts::Account, lang: Lang) -> String {
    let Some(bmc) = app.config.buymeacoffee.as_ref() else {
        return String::new();
    };
    let current = membership(app, &account.id).await.ok().flatten();
    match current {
        // An account still on Paddle: there is no page of ours for it and no
        // key left to call, so the receipt is the only way in.
        Some((provider, status)) if provider != "buymeacoffee" && status != "canceled" => format!(
            r#"<p class="muted">{}</p>"#,
            lang.pick(
                "A sua assinatura ainda é a antiga, cobrada pelo Paddle. Para trocar o cartão ou cancelar, use o link do recibo que o Paddle mandou por e-mail.",
                "Your subscription is still the old one, charged by Paddle. To change the card or cancel, use the link in the receipt Paddle emailed you."
            )
        ),
        Some((_, status)) if status == "active" || status == "paused" => format!(
            r#"<p class="muted">{}</p><a class="button ghost" href="{}" target="_blank" rel="noopener">{}</a>"#,
            lang.pick(
                "A assinatura é gerenciada no Buy Me a Coffee: é lá que você troca o cartão, pausa ou cancela.",
                "The membership is managed on Buy Me a Coffee: that is where you change the card, pause or cancel."
            ),
            escape(&membership_url(bmc)),
            lang.pick("Gerenciar no Buy Me a Coffee", "Manage on Buy Me a Coffee"),
        ),
        _ => format!(
            r#"<a class="button ghost" href="/billing/checkout">{}</a>"#,
            lang.pick(
                "Apoiar com um cafezinho (plano pago)",
                "Support with a coffee (paid plan)"
            )
        ),
    }
}

/// The support page: what the plan gives, and the way to Buy Me a Coffee.
///
/// The address matters more here than anywhere else: it is all that ties the
/// payment to this account, so the page shows it and asks for it.
pub async fn checkout(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let lang = Lang::of(&headers);
    let Some(bmc) = app.config.buymeacoffee.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(account) = login::signed_in(&app, &headers).await else {
        return Redirect::to("/login?return_to=/billing/checkout").into_response();
    };
    let body = format!(
        r#"<h1>{title}</h1>
<p>{intro}</p>
<ul class="muted">
<li>{photos}</li>
<li>{projects}</li>
<li>{drafts}</li>
</ul>
<p><b>{email_label}</b><br><code>{email}</code></p>
<p class="muted">{email_text}</p>
<a class="button" href="{url}" target="_blank" rel="noopener">{go}</a>
<p class="muted" style="margin-top:14px">{fine}</p>
<a class="button ghost" href="/account">{back}</a>"#,
        title = lang.pick("Plano Supporter", "Supporter plan"),
        intro = lang.pick(
            "O plano gratuito continua valendo. O Supporter ajuda a manter o servidor e amplia o que custa processamento:",
            "The free plan stays. Supporter helps pay for the server and raises what takes server time:"
        ),
        photos = lang.pick(
            "Fotos em alta qualidade (100 por mês)",
            "High-quality photos (100 a month)"
        ),
        projects = lang.pick(
            "100 projetos e 2 GB na nuvem",
            "100 projects and 2 GB in the cloud"
        ),
        drafts = lang.pick("200 fotos rascunho por dia", "200 draft photos a day"),
        email_label = lang.pick("Use este e-mail no pagamento:", "Use this email when you pay:"),
        email = escape(&account.email),
        email_text = lang.pick(
            "É por ele que o plano encontra esta conta. Pagando com outro endereço, o plano vai para a conta daquele endereço.",
            "It is how the plan finds this account. Paying with another address puts the plan on that address's account."
        ),
        url = escape(&membership_url(bmc)),
        go = lang.pick("Continuar no Buy Me a Coffee", "Continue on Buy Me a Coffee"),
        fine = lang.pick(
            "O pagamento é feito no Buy Me a Coffee, que cuida da cobrança e do recibo. O plano vale enquanto a assinatura estiver ativa; cancelando, ele vale até o fim do período já pago. Pausar ou cancelar é feito lá.",
            "Payment happens on Buy Me a Coffee, which takes care of the charge and the receipt. The plan lasts while the membership is active; cancel and it lasts until the end of the period already paid for. Pausing and cancelling are done there."
        ),
        back = lang.pick("Voltar para a conta", "Back to the account"),
    );
    page(lang, lang.pick("Plano Supporter", "Supporter plan"), &body).into_response()
}

/// What an account being closed should be told about a membership it still
/// has. Buy Me a Coffee gives a seller no way to cancel one, so the only
/// honest thing to do is say where it is cancelled — and stop the plan here,
/// so a closed account leaves nothing behind that still counts as paid.
pub async fn cancel_for_closed(app: &AppState, account: &str) {
    let Ok(Some((provider, status))) = membership(app, account).await else {
        return;
    };
    if status == "canceled" {
        return;
    }
    if let Err(err) = sqlx::query(
        "update subscriptions set status = 'canceled', updated_at = now() where account_id = $1",
    )
    .bind(account)
    .execute(&app.db)
    .await
    {
        tracing::error!("closing an account that was paying: {err:#}");
    }
    tracing::warn!("an account was closed while {provider} was still charging it");
}

/// Whether something is still charging the account — what the closing page
/// has to warn about, because nothing here can stop it any more. Paddle
/// counts: those subscriptions outlived the move and used to be cancelled
/// through an API this no longer holds a key for.
pub async fn still_charging(app: &AppState, account: &str) -> bool {
    matches!(
        membership(app, account).await,
        Ok(Some((_, status))) if status == "active" || status == "paused"
    )
}

#[derive(Debug, Deserialize)]
pub struct Paid {
    paid: Option<String>,
}

/// The note the account page shows on the way back from Buy Me a Coffee: the
/// webhook may take a few seconds to change the plan.
pub fn thanks(lang: Lang, query: &Query<Paid>) -> String {
    if query.paid.is_none() {
        return String::new();
    }
    format!(
        r#"<p class="muted"><b>{}</b> {}</p>"#,
        lang.pick("Obrigado pelo apoio!", "Thank you for the support!"),
        lang.pick(
            "O plano muda em alguns segundos; recarregue a página se ainda não mudou.",
            "The plan changes within seconds; reload the page if it has not yet."
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// RFC 4231 test case 2.
    #[test]
    fn hmac_follows_the_rfc() {
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn only_buy_me_a_coffees_deliveries_pass() {
        // Made up here: nothing real is written in the code, and a secret
        // scanner has nothing to flag.
        let secret = &format!("whsec_{}", "test-key-not-a-secret");
        let body = br#"{"type":"membership.started"}"#;
        let header = signature(secret, body);
        assert!(verified(secret, &header, body));
        assert!(
            verified(secret, &format!(" {header} "), body),
            "spacing around the header"
        );
        assert!(
            !verified(secret, &header, br#"{"type":"membership.cancelled"}"#),
            "the body is signed"
        );
        assert!(!verified("whsec_another", &header, body), "another secret");
        assert!(!verified(secret, "", body), "no header");
        assert!(!verified(secret, "not hex", body), "nonsense");
    }

    #[test]
    fn a_cancelled_membership_lasts_until_the_period_ends() {
        let ending = json!({ "status": "active", "cancel_at_period_end": "true" });
        assert_eq!(status_of("membership.cancelled", &ending), "active");
        let over = json!({ "status": "canceled", "cancel_at_period_end": "false" });
        assert_eq!(status_of("membership.cancelled", &over), "canceled");
        let paused = json!({ "status": "paused" });
        assert_eq!(status_of("membership.paused", &paused), "paused");
        let started = json!({ "status": "active" });
        assert_eq!(status_of("membership.started", &started), "active");
        // The flags arrive as strings of "true"/"false", and booleans read
        // the same way in case that ever changes.
        let boolean = json!({ "status": "active", "canceled": true });
        assert_eq!(status_of("membership.updated", &boolean), "canceled");
    }
}
