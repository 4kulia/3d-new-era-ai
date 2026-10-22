//! Fetching a product: which `AnySite` endpoint, and when a second one.
//!
//! IKEA comes from its adapter. Any other shop's page is parsed first
//! (1 credit), and rendered in a browser (10 credits) only when the parse
//! left out something the product's kind needs ([`render_worth_it`]): a
//! page behind a bot check (Hornbach, Tegeldepot) parses as a challenge
//! with no heading, so everything is missing and the render is tried. The
//! render fills only what the parse lacked. What neither found is left to
//! the calling agent.
//!
//! A call that failed for `AnySite`'s own reason — busy, down, too slow, a
//! refused key — is an error of the import, not an empty page: no second
//! call is bought on top of it. Only a page that could not be read (`412`,
//! or a `200` with nothing in it) is a product with its fields missing.

use serde_json::json;

use crate::anysite::{AnySite, Endpoint, Failure, FetchError};
use crate::extract::{dutch_vat, forget_default_unit, from_ikea, from_page, settle_sale_unit};
use crate::identity::{Identity, Source};
use crate::product::{Fields, Kind, render_worth_it};

/// One call made for a product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    pub endpoint: &'static str,
    /// `ok`, or why it brought nothing.
    pub outcome: String,
    /// What `AnySite` charged, as it said.
    pub credits: Option<u32>,
}

/// What the fetch found.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub kind: Kind,
    pub fields: Fields,
    pub fetches: Vec<Fetch>,
}

/// Fetches and reads one product. `Err` when `AnySite` itself failed on the
/// first call (the key refused, busy, down, too slow): nothing was read and
/// the import may be tried again. A page that could not be reached is a
/// product with its fields missing, for the agent to fill; so is one whose
/// render failed after a parse that answered.
pub async fn fetch(
    anysite: &AnySite,
    identity: &Identity,
    kind: Option<Kind>,
) -> Result<Fetched, FetchError> {
    let mut got = read(anysite, identity, kind).await?;
    // What the shop left unsaid of VAT, the market says: in the
    // Netherlands a consumer price includes it (IKEA's adapter never says,
    // recorded `ikea_billy_00263850.json`). Elsewhere it stays unknown.
    if identity.market == "nl" {
        dutch_vat(&mut got.fields);
    }
    Ok(got)
}

/// What the shop's adapter or page says of the product.
async fn read(
    anysite: &AnySite,
    identity: &Identity,
    kind: Option<Kind>,
) -> Result<Fetched, FetchError> {
    let mut fetches = Vec::new();
    match identity.source() {
        Source::Ikea => {
            let answer = anysite
                .call(Endpoint::IkeaProduct, &json!({"product": identity.url}))
                .await;
            let item = logged(&mut fetches, Endpoint::IkeaProduct, answer, true)?;
            let fields = item.as_ref().map(from_ikea).unwrap_or_default();
            Ok(Fetched {
                kind: kind.unwrap_or(Kind::Furniture),
                fields,
                fetches,
            })
        }
        Source::Page => {
            let body = json!({"url": identity.url, "return_full_html": true});
            let parsed = anysite.call(Endpoint::Parse, &body).await;
            let parsed = logged(&mut fetches, Endpoint::Parse, parsed, true)?;
            let (parsed_kind, mut fields) = match &parsed {
                Some(result) => from_page(result, kind),
                None => (kind.unwrap_or(Kind::Furniture), Fields::new()),
            };
            if !render_worth_it(parsed_kind, &fields) {
                return Ok(Fetched {
                    kind: parsed_kind,
                    fields,
                    fetches,
                });
            }
            let rendered = anysite.call(Endpoint::Render, &body).await;
            let rendered = logged(&mut fetches, Endpoint::Render, rendered, false)?;
            // The agent's word, or what the parse showed of a finish; a
            // page that showed nothing (a bot check) leaves it to the render.
            let given = kind.or((parsed_kind == Kind::Finish).then_some(Kind::Finish));
            let kind = match &rendered {
                Some(result) => {
                    let (kind, more) = from_page(result, given);
                    if kind != parsed_kind {
                        // What the parse took for a default of its kind is
                        // not the render's.
                        forget_default_unit(&mut fields);
                    }
                    for (name, field) in more {
                        fields.entry(name).or_insert(field);
                    }
                    settle_sale_unit(kind, &mut fields);
                    kind
                }
                None => parsed_kind,
            };
            Ok(Fetched {
                kind,
                fields,
                fetches,
            })
        }
    }
}

/// Records a call; the item it brought, `None` when the page brought
/// nothing, and `Err` when the key was refused, which no other call would
/// change, or — for a `strict` call, the one the import stands on —
/// when `AnySite` failed.
fn logged(
    fetches: &mut Vec<Fetch>,
    endpoint: Endpoint,
    answer: Result<crate::anysite::Answer, Failure>,
    strict: bool,
) -> Result<Option<serde_json::Value>, FetchError> {
    match answer {
        Ok(answer) => {
            fetches.push(Fetch {
                endpoint: endpoint.name(),
                outcome: "ok".to_owned(),
                credits: answer.credits,
            });
            Ok(Some(answer.item))
        }
        Err(failure) => {
            fetches.push(Fetch {
                endpoint: endpoint.name(),
                outcome: failure.error.to_string(),
                credits: failure.credits,
            });
            if failure.error == FetchError::Refused || (strict && !failure.error.is_the_page()) {
                Err(failure.error)
            } else {
                Ok(None)
            }
        }
    }
}
