//! Fetching through `AnySite`'s HTTP API, against a stand-in on a local port
//! that answers with the recorded responses: which endpoint is called, with
//! what, when the render is paid for, and what each failure becomes.

use std::sync::{Arc, Mutex};

use newera_shop::anysite::{AnySite, Endpoint, FetchError};
use newera_shop::identify;
use newera_shop::import::{Fetch, fetch};
use newera_shop::product::Kind;
use serde_json::{Value, json};

fn record(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("JSON")
}

/// What the stand-in was asked: path, key, body.
type Asked = Arc<Mutex<Vec<(String, String, Value)>>>;
/// What the stand-in answers each endpoint with, and what it was asked.
type Stand = (Arc<Vec<(String, Value)>>, Asked);

/// `AnySite` on a local port: each endpoint answers with the record named
/// for it, as it was recorded — status, `x-credits-charged`, `x-error`,
/// body.
async fn stand_in(answers: &[(&'static str, &'static str)]) -> (AnySite, Asked) {
    let records = answers
        .iter()
        .map(|(endpoint, name)| ((*endpoint).to_owned(), record(name)))
        .collect();
    stand_in_of(records).await
}

/// A record of a call in the shape of the recorded ones, made up for a case
/// no recording holds: a status, `x-credits-charged`, a page's HTML.
fn made_up(status: u16, credits: &str, body: &Value) -> Value {
    json!({"status": status, "headers": {"x-credits-charged": credits}, "body": body})
}

/// A `200` whose one item is a page with this HTML.
fn page_of(html: &str, credits: &str) -> Value {
    made_up(
        200,
        credits,
        &json!([{"@type": "@web_parser_result", "cleaned_html": html}]),
    )
}

async fn stand_in_of(answers: Vec<(String, Value)>) -> (AnySite, Asked) {
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;

    let asked: Asked = Arc::default();
    let state = (Arc::new(answers), asked.clone());
    let app = axum::Router::new()
        .route(
            "/api/{*endpoint}",
            axum::routing::post(
                |State((answers, asked)): State<Stand>,
                 Path(endpoint): Path<String>,
                 headers: HeaderMap,
                 axum::Json(body): axum::Json<Value>| async move {
                    let key = headers
                        .get("access-token")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    asked.lock().unwrap().push((endpoint.clone(), key, body));
                    let Some((_, rec)) = answers.iter().find(|(e, _)| *e == endpoint) else {
                        return (StatusCode::NOT_FOUND, "no such endpoint").into_response();
                    };
                    let mut response = (
                        StatusCode::from_u16(
                            u16::try_from(rec["status"].as_u64().unwrap()).unwrap(),
                        )
                        .unwrap(),
                        axum::Json(rec["body"].clone()),
                    )
                        .into_response();
                    for (name, value) in rec["headers"].as_object().unwrap() {
                        response.headers_mut().insert(
                            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                            value.as_str().unwrap().parse().unwrap(),
                        );
                    }
                    response
                },
            ),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (AnySite::new(&base, "test-key"), asked)
}

fn log(endpoint: &'static str, outcome: &str, credits: Option<u32>) -> Fetch {
    Fetch {
        endpoint,
        outcome: outcome.to_owned(),
        credits,
    }
}

const HORNBACH: &str =
    "https://www.hornbach.nl/p/wandtegel-elegance-wit-mat-30-x-60-cm-gerectificeerd/10656129/";

/// IKEA goes to its adapter, with the link and the key, and nothing else
/// is called.
#[tokio::test]
async fn ikea_comes_from_its_adapter() {
    let (anysite, asked) = stand_in(&[("ikea/products", "ikea_billy_00263850.json")]).await;
    let identity = identify("https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/").unwrap();
    let got = fetch(&anysite, &identity, None).await.unwrap();
    assert_eq!(got.kind, Kind::Furniture);
    assert_eq!(got.fields["price"].value, "49.99");
    assert_eq!(
        got.fields["vat_included"],
        newera_shop::Field {
            value: json!(true),
            evidence: newera_shop::Evidence::Market,
            from: "Dutch consumer prices include VAT".to_owned(),
        },
        "the adapter does not say; the Dutch market's rule does"
    );
    assert_eq!(got.fetches, [log("ikea/products", "ok", Some(5))]);
    assert_eq!(
        *asked.lock().unwrap(),
        [(
            "ikea/products".to_owned(),
            "test-key".to_owned(),
            json!({"product": "https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/"})
        )]
    );
}

/// The Dutch rule is not another market's: an IKEA link outside the
/// Netherlands has its price and no word on VAT.
#[tokio::test]
async fn ikea_outside_the_netherlands_says_nothing_of_vat() {
    let (anysite, _) = stand_in(&[("ikea/products", "ikea_billy_00263850.json")]).await;
    let identity =
        identify("https://www.ikea.com/de/de/p/billy-buecherregal-weiss-00263850/").unwrap();
    let got = fetch(&anysite, &identity, None).await.unwrap();
    assert_eq!(got.fields["price"].value, "49.99");
    assert!(!got.fields.contains_key("vat_included"));
}

/// A page silent on VAT takes the market's rule in the Netherlands and
/// nothing elsewhere: a German shop's page is not said to include Dutch VAT.
#[tokio::test]
async fn a_page_silent_on_vat_takes_the_rule_of_the_dutch_market_only() {
    let silent = page_of(
        "<h1>Kast Alaska</h1><div>135,20</div><dl><dt>Afmetingen (cm)</dt><dd>101x107x35 (hxbxd)</dd></dl>",
        "1",
    );
    for (link, vat) in [
        (
            "https://www.leenbakker.nl/kasten/kast-alaska",
            Some(newera_shop::Field {
                value: json!(true),
                evidence: newera_shop::Evidence::Market,
                from: "Dutch consumer prices include VAT".to_owned(),
            }),
        ),
        ("https://www.moebel.de/schraenke/kast-alaska", None),
    ] {
        let (anysite, asked) =
            stand_in_of(vec![("webparser/parse".to_owned(), silent.clone())]).await;
        let got = fetch(&anysite, &identify(link).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(got.fields["price"].value, "135.20", "{link}");
        assert_eq!(got.fields.get("vat_included").cloned(), vat, "{link}");
        assert_eq!(asked.lock().unwrap().len(), 1, "no render: {link}");
    }
}

/// An IKEA item the adapter gives no price is said nothing of VAT: the
/// market's rule is about a price.
#[tokio::test]
async fn an_ikea_item_without_a_price_says_nothing_of_vat() {
    let mut item = record("ikea_billy_00263850.json");
    for key in ["price", "previous_price"] {
        item["body"][0][key] = Value::Null;
    }
    let (anysite, _) = stand_in_of(vec![("ikea/products".to_owned(), item)]).await;
    let identity = identify("https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/").unwrap();
    let got = fetch(&anysite, &identity, None).await.unwrap();
    assert!(!got.fields.contains_key("price"));
    assert!(!got.fields.contains_key("vat_included"));
}

/// A page the parse reads in full costs one credit and no render.
#[tokio::test]
async fn a_complete_parse_is_not_rendered() {
    let (anysite, asked) = stand_in(&[
        ("webparser/parse", "leenbakker_kast_alaska.parse.json"),
        ("webparser/render", "hornbach_tile_10656129.render.json"),
    ])
    .await;
    let identity =
        identify("https://www.leenbakker.nl/kasten/dressoirs/kast-alaska-wit-101x107x35cm")
            .unwrap();
    let got = fetch(&anysite, &identity, None).await.unwrap();
    assert_eq!(got.kind, Kind::Furniture);
    assert_eq!(got.fields["width_cm"].value, 107.0);
    assert_eq!(got.fetches, [log("webparser/parse", "ok", Some(1))]);
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].2,
        json!({"url": "https://www.leenbakker.nl/kasten/dressoirs/kast-alaska-wit-101x107x35cm", "return_full_html": true})
    );
}

/// A bot check on the parse sends the same request to the render, whose
/// page fills the product.
#[tokio::test]
async fn a_challenged_parse_is_rendered() {
    let (anysite, asked) = stand_in(&[
        ("webparser/parse", "hornbach_tile_10656129.parse.json"),
        ("webparser/render", "hornbach_tile_10656129.render.json"),
    ])
    .await;
    let got = fetch(&anysite, &identify(HORNBACH).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(got.kind, Kind::Finish);
    assert_eq!(got.fields["price"].value, "18.13");
    assert_eq!(got.fields["pack_m2"].value, 1.62);
    assert_eq!(
        got.fetches,
        [
            log("webparser/parse", "ok", Some(1)),
            log("webparser/render", "ok", Some(10))
        ]
    );
    let asked = asked.lock().unwrap();
    assert_eq!(
        asked[0].2, asked[1].2,
        "the render is asked what the parse was"
    );
}

/// The render fills only what the parse lacked: the parse's own values
/// stay, even where the render reads another. The laminate page asked for
/// as furniture has no box, so the render is paid for; the stand-in
/// renders the Hornbach tile, whose price and article differ.
#[tokio::test]
async fn the_render_fills_gaps_only() {
    let (anysite, _) = stand_in(&[
        ("webparser/parse", "praxis_laminate_10043467.parse.json"),
        ("webparser/render", "hornbach_tile_10656129.render.json"),
    ])
    .await;
    let identity = identify("https://www.praxis.nl/x/10043467").unwrap();
    let got = fetch(&anysite, &identity, Some(Kind::Furniture))
        .await
        .unwrap();
    assert_eq!(got.kind, Kind::Furniture);
    assert_eq!(got.fetches.len(), 2);
    assert_eq!(
        got.fields["price"].value, "24.54",
        "the parse's price stays"
    );
    assert_eq!(
        got.fields["article"].value, "10656129",
        "a gap the render filled"
    );
}

/// Tegeldepot: the parse is a challenge, the render is refused as not
/// accessible and still charged; the product has no fields, both calls
/// are logged with their cost, and nothing fails.
#[tokio::test]
async fn an_unreachable_page_is_a_product_without_fields() {
    let (anysite, _) = stand_in(&[
        ("webparser/parse", "tegeldepot_search.parse.json"),
        ("webparser/render", "tegeldepot_search.render.json"),
    ])
    .await;
    let identity = identify("https://www.tegeldepot.nl/wandtegels/x.html").unwrap();
    let got = fetch(&anysite, &identity, Some(Kind::Finish))
        .await
        .unwrap();
    assert_eq!(got.kind, Kind::Finish);
    assert!(got.fields.is_empty());
    assert_eq!(
        got.fetches,
        [
            log("webparser/parse", "ok", Some(1)),
            log(
                "webparser/render",
                "AnySite could not reach the page: Failed to fetch URL. Website is not accessible",
                Some(10)
            ),
        ]
    );
}

/// A refused key stops the import: no other call would do better.
#[tokio::test]
async fn a_refused_key_stops_the_import() {
    let (anysite, asked) = stand_in(&[
        ("webparser/parse", "error_401.json"),
        ("webparser/render", "hornbach_tile_10656129.render.json"),
    ])
    .await;
    let got = fetch(&anysite, &identify(HORNBACH).unwrap(), None).await;
    assert_eq!(got, Err(FetchError::Refused));
    assert_eq!(
        asked.lock().unwrap().len(),
        1,
        "no render after a refused key"
    );
}

/// What each status `AnySite` answers with becomes.
#[tokio::test]
async fn every_status_says_what_it_was() {
    let (anysite, _) = stand_in(&[
        ("webparser/parse", "error_422.json"),
        ("webparser/render", "tegeldepot_search.render.json"),
        ("ikea/products", "error_401.json"),
    ])
    .await;
    let body = json!({"url": HORNBACH});
    let invalid = anysite.call(Endpoint::Parse, &body).await.unwrap_err();
    assert_eq!(
        invalid.error,
        FetchError::Invalid(
            r#"[{"input":{},"loc":["body","url"],"msg":"Field required","type":"missing"}]"#
                .to_owned()
        )
    );
    assert_eq!(invalid.credits, Some(0));
    let gone = anysite.call(Endpoint::Render, &body).await.unwrap_err();
    assert_eq!(
        (gone.error, gone.credits),
        (
            FetchError::NotAccessible("Failed to fetch URL. Website is not accessible".to_owned()),
            Some(10)
        )
    );
    let refused = anysite
        .call(Endpoint::IkeaProduct, &body)
        .await
        .unwrap_err();
    assert_eq!(
        (refused.error, refused.credits),
        (FetchError::Refused, None)
    );
    let nowhere = AnySite::new("http://127.0.0.1:9", "k")
        .call(Endpoint::Parse, &body)
        .await
        .unwrap_err();
    assert!(
        matches!(nowhere.error, FetchError::Network(_)),
        "{nowhere:?}"
    );
}

/// A parse that read a bare price under a heading takes the page for a
/// piece of furniture; the render shows a tile sold by the pack. The
/// product is the render's kind, with the render's unit and nothing of the
/// furniture default left, and the agent is asked for what a covering needs.
#[tokio::test]
async fn a_render_that_changes_the_kind_changes_what_the_kind_implies() {
    let parse = page_of("<h1>Wandtegel Elegance</h1><div>18,13</div>", "1");
    let by_pack = page_of(
        "<h1>Wandtegel Elegance</h1><div>Komt overeen met 18,13 € per Pak</div><dl><dt>Inhoud per pak voor</dt><dd>1,62 m²</dd></dl>",
        "10",
    );
    let bare = page_of(
        "<h1>Wandtegel Elegance</h1><div>18,13</div><dl><dt>Inhoud per pak voor</dt><dd>1,62 m²</dd></dl>",
        "10",
    );
    let identity = identify("https://www.shop.nl/p/wandtegel/10656129").unwrap();
    for (render, unit, from) in [
        (by_pack, "pack", "a price per pack"),
        (bare, "pack", "a finish priced as one item"),
    ] {
        let (anysite, _) = stand_in_of(vec![
            ("webparser/parse".to_owned(), parse.clone()),
            ("webparser/render".to_owned(), render),
        ])
        .await;
        let got = fetch(&anysite, &identity, None).await.unwrap();
        assert_eq!(got.kind, Kind::Finish);
        assert_eq!(got.fields["sale_unit"].value, unit);
        assert_eq!(got.fields["sale_unit"].from, from);
        assert_eq!(got.fields["finish_type"].value, "covering");
        assert_eq!(got.fields["price"].value, "18.13");
        assert_eq!(
            newera_shop::product::needed(got.kind, &got.fields),
            ["tile_width_cm", "tile_length_cm"],
            "the heading says no size"
        );
    }
}

/// `AnySite` failing on the parse — busy, down, too slow — is an error of
/// the import: no render is bought, nothing is stored as an empty page.
#[tokio::test]
async fn a_parse_that_anysite_failed_buys_no_render() {
    let render = page_of("<h1>Tegel</h1><div>18,13</div>", "10");
    for (status, error) in [
        (429, FetchError::Busy),
        (500, FetchError::Status(500)),
        (503, FetchError::Status(503)),
    ] {
        let (anysite, asked) = stand_in_of(vec![
            (
                "webparser/parse".to_owned(),
                made_up(status, "0", &json!({"detail": "no"})),
            ),
            ("webparser/render".to_owned(), render.clone()),
        ])
        .await;
        let got = fetch(&anysite, &identify(HORNBACH).unwrap(), None).await;
        assert_eq!(got, Err(error), "{status}");
        assert_eq!(asked.lock().unwrap().len(), 1, "{status}: no render");
    }
    let nowhere = AnySite::new("http://127.0.0.1:9", "k");
    let got = fetch(&nowhere, &identify(HORNBACH).unwrap(), None).await;
    assert!(matches!(got, Err(FetchError::Network(_))), "{got:?}");
}

/// A page `AnySite` could not reach, or found empty, is a page for the
/// render to try; a render that then fails leaves the parse's product.
#[tokio::test]
async fn an_empty_parse_is_rendered_and_a_failed_render_keeps_the_parse() {
    let render = page_of("<h1>Tegel</h1><div>18,13</div>", "10");
    for parse in [made_up(412, "1", &json!([])), made_up(200, "1", &json!([]))] {
        let (anysite, asked) = stand_in_of(vec![
            ("webparser/parse".to_owned(), parse),
            ("webparser/render".to_owned(), render.clone()),
        ])
        .await;
        let got = fetch(&anysite, &identify(HORNBACH).unwrap(), Some(Kind::Finish))
            .await
            .unwrap();
        assert_eq!(asked.lock().unwrap().len(), 2);
        assert_eq!(got.fields["price"].value, "18.13");
    }
    let (anysite, _) = stand_in_of(vec![
        (
            "webparser/parse".to_owned(),
            page_of("<h1>Tegel</h1><div>18,13</div>", "1"),
        ),
        (
            "webparser/render".to_owned(),
            made_up(500, "0", &json!({"detail": "down"})),
        ),
    ])
    .await;
    let got = fetch(&anysite, &identify(HORNBACH).unwrap(), Some(Kind::Finish))
        .await
        .unwrap();
    assert_eq!(got.fields["price"].value, "18.13");
    assert_eq!(
        got.fetches,
        [
            log("webparser/parse", "ok", Some(1)),
            log("webparser/render", "AnySite answered 500", Some(0))
        ]
    );
}

/// A variant link is fetched as the variant: its query goes to `AnySite`,
/// the tracking in it does not.
#[tokio::test]
async fn a_variant_is_fetched_with_its_query() {
    let (anysite, asked) =
        stand_in(&[("webparser/parse", "leenbakker_kast_alaska.parse.json")]).await;
    let identity = identify(
        "https://www.leenbakker.nl/kasten/dressoirs/kast-alaska-wit-101x107x35cm?colour=1&utm_source=x",
    )
    .unwrap();
    fetch(&anysite, &identity, None).await.unwrap();
    assert_eq!(
        asked.lock().unwrap()[0].2["url"],
        "https://www.leenbakker.nl/kasten/dressoirs/kast-alaska-wit-101x107x35cm?colour=1"
    );
}
