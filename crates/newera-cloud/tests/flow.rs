//! The hosted service end to end, against a real Postgres: an AI client
//! registers, a person signs in with an emailed link and allows it, the
//! client trades the code for tokens, and its calls reach the person's tab.
//!
//! Needs `TEST_DATABASE_URL` (a Postgres the tests may create databases on);
//! without it they say so and pass, so a machine without Postgres still runs
//! the rest of the suite. CI sets it.

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use newera_cloud::config::{AllowedEmails, Google, Mail};
use newera_cloud::{AppState, Config, secret};
use serde_json::{Value, json};
use sha2::Digest as _;

const SITE: &str = "http://127.0.0.1:8790";
/// A webhook signing secret made up for the tests. Nothing real is written
/// in the code.
const BMC_SECRET: &str = concat!("whsec_", "test-key-not-a-secret");
/// The Buy Me a Coffee page the tests sell from.
const BMC_PAGE: &str = "newera-test";

/// A fresh database for one test, migrated, and the service on a free port.
async fn service() -> Option<(String, AppState)> {
    service_for(vec![SITE.to_owned()]).await
}

/// The same, with the editor served from `sites`.
async fn service_for(sites: Vec<String>) -> Option<(String, AppState)> {
    service_with(sites, AllowedEmails::default(), None).await
}

/// The same, letting in only `allowed`, and signing in with `google` too.
async fn service_with(
    sites: Vec<String>,
    allowed: AllowedEmails,
    google: Option<Google>,
) -> Option<(String, AppState)> {
    service_full(sites, allowed, google, Mail::Log, None).await
}

/// The same, with the way sign-in links are sent, and the HTTP client the
/// service sends them with, chosen.
async fn service_full(
    sites: Vec<String>,
    allowed: AllowedEmails,
    google: Option<Google>,
    mail: Mail,
    http: Option<reqwest::Client>,
) -> Option<(String, AppState)> {
    service_configured(sites, allowed, google, mail, http, None).await
}

/// The same, reading shop products through `anysite`.
async fn service_configured(
    sites: Vec<String>,
    allowed: AllowedEmails,
    google: Option<Google>,
    mail: Mail,
    http: Option<reqwest::Client>,
    anysite: Option<newera_shop::AnySite>,
) -> Option<(String, AppState)> {
    let Ok(admin) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL not set: skipping");
        return None;
    };
    let name = format!("newera_test_{}", secret::id());
    let pool = sqlx::PgPool::connect(&admin)
        .await
        .expect("the test database");
    // The name is our own hex, never input: safe to put in the statement.
    sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
        .execute(&pool)
        .await
        .expect("a database of its own");
    let url = match admin.rsplit_once('/') {
        Some((server, _)) => format!("{server}/{name}"),
        None => panic!("TEST_DATABASE_URL has no database name"),
    };
    let db = newera_cloud::database(&url).await.expect("migrations run");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        public_url: base.clone(),
        database_url: url,
        site_origins: sites,
        mail,
        google,
        editor_url: format!("{SITE}/editor/"),
        buymeacoffee: Some(newera_cloud::config::BuyMeACoffee {
            page: BMC_PAGE.to_owned(),
            webhook_secret: BMC_SECRET.to_owned(),
            levels: vec![7],
            test_events: false,
        }),
        allowed_emails: allowed,
        port: 0,
        anysite,
    };
    let mut state = AppState::start(config, db).await.expect("startup");
    if let Some(http) = http {
        state.http = http;
    }
    let app = newera_cloud::router(&state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Some((base, state))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn location(response: &reqwest::Response) -> String {
    response.headers()["location"].to_str().unwrap().to_owned()
}

fn set_cookie(response: &reqwest::Response) -> String {
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn param(url: &str, key: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// A hidden field's value in a page.
fn field(html: &str, name: &str) -> String {
    let marker = format!(r#"name="{name}" value=""#);
    let start = html.find(&marker).expect(name) + marker.len();
    html[start..]
        .split('"')
        .next()
        .unwrap()
        .replace("&amp;", "&")
}

/// Signs in through an emailed link: the link is made the way `/login`
/// makes it, and the person presses the button.
async fn sign_in(base: &str, state: &AppState, email: &str, return_to: &str) -> (String, String) {
    let http = client();
    let asked = http
        .post(format!("{base}/login"))
        .form(&[("email", email), ("return_to", return_to)])
        .send()
        .await
        .unwrap();
    assert_eq!(asked.status(), 200, "the link is sent");
    // The link is stored after the answer: wait for the row.
    let mut stored = 0_i64;
    for _ in 0..50 {
        stored = sqlx::query_scalar(
            "select count(*) from login_links where email = $1 and used_at is null",
        )
        .bind(email)
        .fetch_one(&state.db)
        .await
        .unwrap();
        if stored > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(stored > 0, "no login link stored for {email} within 5 s");
    // The log carries the link; the test reads the row instead and swaps in a
    // token it knows (only the hash is stored).
    let token = secret::token();
    sqlx::query("update login_links set token_hash = $1 where email = $2 and used_at is null")
        .bind(secret::hash(&token))
        .bind(email)
        .execute(&state.db)
        .await
        .unwrap();
    let page = http
        .get(format!("{base}/login/link?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200, "the link opens a page, not a sign-in");
    let signed = http
        .post(format!("{base}/login/link"))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(signed.status(), 303);
    let cookie = set_cookie(&signed);
    let again = http
        .post(format!("{base}/login/link"))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 410, "a link works once");
    (cookie, location(&signed))
}

async fn mcp(base: &str, token: &str, body: Value) -> (u16, Value) {
    let response = client()
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn an_ai_client_is_allowed_in_and_reaches_the_tab() {
    let Some((base, state)) = service().await else {
        return;
    };
    let http = client();

    // Discovery: a call without a token points at the metadata.
    let bare = http
        .post(format!("{base}/mcp"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), 401);
    let challenge = bare.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        challenge.contains("/.well-known/oauth-protected-resource/mcp"),
        "{challenge}"
    );
    let resource: Value = http
        .get(format!("{base}/.well-known/oauth-protected-resource/mcp"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resource["resource"], format!("{base}/mcp"));
    let server: Value = http
        .get(format!("{base}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(server["code_challenge_methods_supported"], json!(["S256"]));

    // Registration.
    let redirect = "http://127.0.0.1/callback";
    let refused = http
        .post(format!("{base}/oauth/register"))
        .json(&json!({"redirect_uris": ["http://evil.example/cb"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        400,
        "plain http off this machine is refused"
    );
    let registered: Value = http
        .post(format!("{base}/oauth/register"))
        .json(&json!({"redirect_uris": [redirect], "client_name": "Test <b>client</b>"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = registered["client_id"].as_str().unwrap().to_owned();

    // The authorization request, with PKCE and a loopback port of its own.
    let verifier = secret::token();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let callback = "http://127.0.0.1:51234/callback";
    let authorize = format!(
        "{base}/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=xyz&scope=newera&resource={}",
        newera_cloud::login::url_encode(callback),
        newera_cloud::login::url_encode(&format!("{base}/mcp")),
    );
    let first = http.get(&authorize).send().await.unwrap();
    assert_eq!(first.status(), 303, "not signed in yet");
    let to_login = location(&first);
    assert!(
        to_login.starts_with("/login?return_to=%2Foauth%2Fauthorize"),
        "{to_login}"
    );

    // Signing in brings the person back to the request.
    let return_to = param(&format!("{base}{to_login}"), "return_to").unwrap();
    let (cookie, back) = sign_in(&base, &state, "Ana@Example.com", &return_to).await;
    assert!(back.starts_with("/oauth/authorize?"), "{back}");

    // The consent screen names the client (escaped) and the account.
    let consent = http
        .get(format!("{base}{back}"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(consent.contains("Test &lt;b&gt;client&lt;/b&gt;"));
    assert!(consent.contains("Ana@Example.com"));
    let csrf = field(&consent, "csrf");

    // A forged form is refused; the real one gives a code.
    let forged = http
        .post(format!("{base}/oauth/authorize"))
        .header("cookie", &cookie)
        .form(&[
            ("csrf", "nope"),
            ("decision", "allow"),
            ("client_id", &client_id),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), 400);
    let form = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", callback),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", "xyz"),
        ("scope", "newera"),
        ("csrf", csrf.as_str()),
        ("decision", "allow"),
    ];
    let allowed = http
        .post(format!("{base}/oauth/authorize"))
        .header("cookie", &cookie)
        .form(&form)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), 303);
    let redirected = location(&allowed);
    assert!(redirected.starts_with(callback));
    assert_eq!(param(&redirected, "state").as_deref(), Some("xyz"));
    assert_eq!(param(&redirected, "iss").as_deref(), Some(base.as_str()));
    let code = param(&redirected, "code").unwrap();

    // The code, with the wrong verifier, then the right one; never twice.
    let exchange = |verifier: String| {
        let http = http.clone();
        let (base, code, client_id) = (base.clone(), code.clone(), client_id.clone());
        async move {
            http.post(format!("{base}/oauth/token"))
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("code", code.as_str()),
                    ("client_id", client_id.as_str()),
                    ("redirect_uri", callback),
                    ("code_verifier", verifier.as_str()),
                ])
                .send()
                .await
                .unwrap()
        }
    };
    let wrong = exchange(secret::token()).await;
    assert_eq!(
        wrong.status(),
        400,
        "a wrong verifier spends the code for nothing"
    );
    // The code was spent by the wrong attempt: a fresh consent for the right one.
    let allowed = http
        .post(format!("{base}/oauth/authorize"))
        .header("cookie", &cookie)
        .form(&form)
        .send()
        .await
        .unwrap();
    let code = param(&location(&allowed), "code").unwrap();
    let tokens: Value = http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("redirect_uri", callback),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = tokens["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let replay = http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 400, "a code works once");
    let (status, _) = mcp(
        &base,
        &access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    )
    .await;
    assert_eq!(status, 401, "a replayed code revokes what it bought");

    // So the person allows again, and this time the client behaves.
    let allowed = http
        .post(format!("{base}/oauth/authorize"))
        .header("cookie", &cookie)
        .form(&form)
        .send()
        .await
        .unwrap();
    let code = param(&location(&allowed), "code").unwrap();
    let tokens: Value = http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("redirect_uri", callback),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh_now = tokens["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(refresh_now, refresh);

    // MCP: the handshake and the list, before any tab is open.
    let (status, hello) = mcp(
        &base,
        &access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(hello["result"]["protocolVersion"], "2025-06-18");
    let (_, list) = mcp(
        &base,
        &access,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"show_plan") && names.contains(&"create"));
    assert!(
        !names.contains(&"feedback") && !names.contains(&"run_plugin"),
        "desktop only"
    );
    let (_, page) = mcp(
        &base,
        &access,
        json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "ui://newera/plan-viewer.html"}}),
    )
    .await;
    assert_eq!(
        page["result"]["contents"][0]["mimeType"],
        "text/html;profile=mcp-app"
    );
    // With no tab open, the calls work on a project in the cloud.
    let call = |id: u64, name: &str, arguments: Value| {
        let (base, access) = (base.clone(), access.clone());
        let name = name.to_owned();
        async move {
            let (_, answer) = mcp(
                &base,
                &access,
                json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}}),
            )
            .await;
            answer["result"].clone()
        }
    };
    let drawn = call(
        4,
        "create",
        json!({"walls": [{"pts": [[0, 0], [400, 0], [400, 300], [0, 300]], "closed": true}]}),
    )
    .await;
    assert_eq!(drawn["isError"], false, "{drawn}");
    let home = call(40, "home", json!({"detail": "summary"})).await;
    assert!(
        home["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("walls"),
        "{home}"
    );
    let listed = call(41, "projects", json!({})).await;
    let listed: Value =
        serde_json::from_str(listed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(listed["rows"][0][0], "Projeto 1");
    let size: i32 = sqlx::query_scalar("select size from projects")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert!(size > 100, "the change was kept");

    // A draft photo counts; a good one is not in the free plan.
    let photo = call(42, "render_photo", json!({"w": 64, "h": 48})).await;
    assert_eq!(photo["isError"], false, "{photo}");
    let used: i32 = sqlx::query_scalar("select count from usage where kind = 'draft_render'")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(used, 1);
    let good = call(
        43,
        "render_photo",
        json!({"w": 64, "h": 48, "quality": "good"}),
    )
    .await;
    assert_eq!(good["isError"], true);
    assert!(
        good["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Free plan")
    );

    // An export comes back as a link, and the link is the file.
    let exported = call(44, "export", json!({"path": "/etc/casa.pdf"})).await;
    let text = exported["content"][0]["text"].as_str().unwrap().to_owned();
    let link = text.split_whitespace().nth(1).expect("a link").to_owned();
    assert!(link.starts_with(&format!("{base}/files/")), "{text}");
    let file = http.get(&link).send().await.unwrap();
    assert_eq!(file.status(), 200);
    assert!(
        file.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("casa.pdf")
    );
    assert!(file.bytes().await.unwrap().starts_with(b"%PDF"));
    assert!(
        !std::path::Path::new("/etc/casa.pdf").exists(),
        "never where the path said"
    );

    // Projects by name: a new one, back to the first, renamed on save.
    let made = call(
        45,
        "file",
        json!({"action": "new", "name": "Casa da praia"}),
    )
    .await;
    assert_eq!(made["isError"], false, "{made}");
    let back = call(47, "file", json!({"action": "open", "path": "Projeto 1"})).await;
    assert_eq!(back["isError"], false, "{back}");
    let renamed = call(
        48,
        "file",
        json!({"action": "save", "path": "Apartamento.newera"}),
    )
    .await;
    assert_eq!(renamed["content"][0]["text"], "ok saved Apartamento");
    // "Open in the editor" is this project, which the site may fetch with the
    // person's cookie — and nobody else may.
    let shown = call(50, "show_plan", json!({})).await;
    let editor = shown["structuredContent"]["editor"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        editor.starts_with(&format!("{SITE}/editor/?project=")),
        "{editor}"
    );
    let project_url = param(
        &format!("http://x/{}", editor.split('/').next_back().unwrap()),
        "project",
    )
    .unwrap();
    let fetched = http
        .get(&project_url)
        .header("cookie", &cookie)
        .header("origin", SITE)
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), 200);
    assert_eq!(
        fetched.headers()["access-control-allow-credentials"],
        "true"
    );
    assert!(
        fetched.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("Apartamento.newera")
    );
    assert!(fetched.bytes().await.unwrap().starts_with(b"PK"));
    let stranger = http.get(&project_url).send().await.unwrap();
    assert_ne!(stranger.status(), 200, "only its owner downloads a project");
    let refused = call(
        49,
        "edit_background",
        json!({"action": "set", "path": "/etc/passwd"}),
    )
    .await;
    assert_eq!(refused["isError"], true);

    // A tab opens a room on the relay, signs in, and claims it.
    let opened: Value = http
        .post(format!("{base}/rooms"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let room = opened["room"].as_str().unwrap().to_owned();
    let tab_key = opened["tab_key"].as_str().unwrap().to_owned();
    let tab_url = format!(
        "ws://{}{}",
        base.trim_start_matches("http://"),
        opened["tab_path"].as_str().unwrap()
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(&tab_url).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::text(
            json!({"type": "hello", "tools": [], "resources": []}).to_string(),
        ))
        .await
        .unwrap();
    let not_from_site = http
        .post(format!("{base}/account/claim"))
        .header("cookie", &cookie)
        .header("origin", "https://evil.example")
        .json(&json!({"room": room, "tab_key": tab_key}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        not_from_site.status(),
        403,
        "only the site may claim with the cookie"
    );
    let claimed = http
        .post(format!("{base}/account/claim"))
        .header("cookie", &cookie)
        .header("origin", SITE)
        .json(&json!({"room": room, "tab_key": tab_key}))
        .send()
        .await
        .unwrap();
    assert_eq!(claimed.status(), 200);
    let me: Value = http
        .get(format!("{base}/account/me"))
        .header("cookie", &cookie)
        .header("origin", SITE)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["email"], "Ana@Example.com");
    assert_eq!(me["plan"]["code"], "free");

    // The service's own tools — the projects, the product library — are
    // answered by the server while a tab holds the room: the tab is not
    // asked.
    let quiet = std::time::Duration::from_millis(400);
    while tokio::time::timeout(quiet, socket.next()).await.is_ok() {}
    for (id, name) in [(60, "projects"), (61, "products")] {
        let answered = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            call(id, name, json!({})),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} was sent to the tab, which does not answer it"));
        assert_eq!(answered["isError"], false, "{name}: {answered}");
        let listed: Value =
            serde_json::from_str(answered["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(listed["rows"].is_array(), "{name}: {listed}");
    }
    assert!(
        tokio::time::timeout(quiet, socket.next()).await.is_err(),
        "the tab received a call for a tool the server answers"
    );

    // The call goes down to the tab and its answer comes back.
    let calling = tokio::spawn({
        let (base, access) = (base.clone(), access.clone());
        async move {
            mcp(
                &base,
                &access,
                json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "home", "arguments": {}}}),
            )
            .await
        }
    });
    let work = loop {
        let message = socket.next().await.unwrap().unwrap();
        let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        if value["type"] == "call" {
            break value;
        }
    };
    assert_eq!(work["name"], "home");
    socket
        .send(tokio_tungstenite::tungstenite::Message::text(
            json!({"type": "result", "id": work["id"], "ok": true, "result": {"content": [{"type": "text", "text": "w1"}]}}).to_string(),
        ))
        .await
        .unwrap();
    let (_, answered) = calling.await.unwrap();
    assert_eq!(answered["result"]["content"][0]["text"], "w1");

    // Refresh rotates; the old refresh token used again revokes the family.
    let rotated: Value = http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_now.as_str()),
            ("client_id", client_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access2 = rotated["access_token"]
        .as_str()
        .expect("rotated")
        .to_owned();
    let (status, _) = mcp(
        &base,
        &access2,
        json!({"jsonrpc": "2.0", "id": 6, "method": "ping"}),
    )
    .await;
    assert_eq!(status, 200);
    let stolen = http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_now.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(stolen.status(), 400);
    let (status, _) = mcp(
        &base,
        &access2,
        json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}),
    )
    .await;
    assert_eq!(status, 401, "reuse of a refresh token revokes its family");

    // Closing the account signs everything out.
    let page = http
        .get(format!("{base}/account"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let csrf = field(&page, "csrf");
    let closed = http
        .post(format!("{base}/account/close"))
        .header("cookie", &cookie)
        .form(&[("csrf", csrf.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), 200);
    let me = http
        .get(format!("{base}/account/me"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 401);
}

/// A deployment that serves the editor from its own origin lets that page
/// hold a tab in the relay, and still refuses any other page.
#[tokio::test]
async fn the_editor_of_this_deployment_reaches_the_relay() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let own = "https://editor.example";
    let Some((base, _state)) = service_for(vec![own.to_owned()]).await else {
        return;
    };
    let opened: Value = client()
        .post(format!("{base}/rooms"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tab = format!(
        "ws://{}{}",
        base.trim_start_matches("http://"),
        opened["tab_path"].as_str().unwrap()
    );
    let from = |origin: &str| {
        let mut request = tab.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        request
    };
    match tokio_tungstenite::connect_async(from("https://elsewhere.example")).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status().as_u16(), 403, "another page is refused");
        }
        other => panic!("another page should get 403, got {other:?}"),
    }
    let (socket, response) = tokio_tungstenite::connect_async(from(own))
        .await
        .expect("this deployment's editor takes the tab");
    assert_eq!(response.status().as_u16(), 101);
    drop(socket);
}

#[tokio::test]
async fn sign_in_never_redirects_elsewhere() {
    let Some((base, state)) = service().await else {
        return;
    };
    for (asked, expected) in [
        ("https://evil.example/", "/account"),
        ("//evil.example/x", "/account"),
        ("/\t/evil.example", "/account"),
        ("/\n/evil.example", "/account"),
        ("/\r/evil.example", "/account"),
        ("/ /evil.example", "/account"),
        ("/\\evil", "/account"),
        ("/oauth/authorize?x=1", "/oauth/authorize?x=1"),
        ("http://127.0.0.1:8790/app/", "http://127.0.0.1:8790/app/"),
    ] {
        let (_, back) = sign_in(
            &base,
            &state,
            &format!("{}@example.com", secret::id()),
            asked,
        )
        .await;
        assert_eq!(back, expected, "{asked}");
    }
}

/// What the proxy's session check answers for `cookie`: the status, the
/// cache header and the body.
async fn check(base: &str, cookie: Option<&str>) -> (u16, String, String) {
    let mut asked = client().get(format!("{base}/login/check"));
    if let Some(cookie) = cookie {
        asked = asked.header("cookie", cookie);
    }
    let answer = asked.send().await.unwrap();
    let status = answer.status().as_u16();
    let cache = answer.headers()["cache-control"]
        .to_str()
        .unwrap()
        .to_owned();
    (status, cache, answer.text().await.unwrap())
}

#[tokio::test]
async fn the_proxy_lets_in_only_a_signed_in_browser() {
    let Some((base, state)) = service().await else {
        return;
    };
    let nobody = (401, "no-store".to_owned(), String::new());
    let welcome = (204, "no-store".to_owned(), String::new());
    assert_eq!(check(&base, None).await, nobody, "no session");
    assert_eq!(
        check(&base, Some("newera_session=made-up")).await,
        nobody,
        "a session nobody opened"
    );
    let (stranger, ..) = credentials(&base, &state, "stranger@example.com").await;
    let (owner, ..) = credentials(&base, &state, "owner@example.com").await;
    assert_eq!(check(&base, Some(&stranger)).await, welcome, "no list");

    let (base, _) = restart(&state, list_of("owner@example.com")).await;
    assert_eq!(check(&base, Some(&owner)).await, welcome, "listed");
    assert_eq!(
        check(&base, Some(&stranger)).await,
        nobody,
        "unlisted, once a list is set"
    );
}

/// The sign-in page, asked for with a return target, never brings the person
/// anywhere but a page of this service or the site.
#[tokio::test]
async fn the_sign_in_page_refuses_a_return_target_a_browser_would_read_as_another_host() {
    let Some((base, _)) = service().await else {
        return;
    };
    for target in [
        "%2F%09%2Fevil.example",
        "%2F%0A%2Fevil.example",
        "%2F%5Cevil",
    ] {
        let page = client()
            .get(format!("{base}/login?return_to={target}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(field(&page, "return_to"), "/account", "{target}");
    }
    let asked = client()
        .get(format!("{base}/login/required"))
        .header("x-original-uri", "/\t/evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(location(&asked), "/login?return_to=%2Faccount");
}

#[tokio::test]
async fn the_proxy_sends_a_turned_away_browser_to_sign_in_and_back() {
    let Some((base, _)) = service().await else {
        return;
    };
    for (original, expected) in [
        (
            Some("/app/?a=1&b=2"),
            "/login?return_to=%2Fapp%2F%3Fa%3D1%26b%3D2",
        ),
        (
            Some("/app/?project=http%3A%2F%2Fx%2Fp&relay=https://h"),
            "/login?return_to=%2Fapp%2F%3Fproject%3Dhttp%253A%252F%252Fx%252Fp%26relay%3Dhttps%3A%2F%2Fh",
        ),
        (
            Some("https://evil.example/app/"),
            "/login?return_to=%2Faccount",
        ),
        (Some("//evil.example/app/"), "/login?return_to=%2Faccount"),
        (None, "/login?return_to=%2Faccount"),
    ] {
        let mut asked = client().get(format!("{base}/login/required"));
        if let Some(original) = original {
            asked = asked.header("x-original-uri", original);
        }
        let answer = asked.send().await.unwrap();
        assert_eq!(answer.status(), 302, "{original:?}");
        assert_eq!(location(&answer), expected, "{original:?}");
        assert_eq!(answer.headers()["cache-control"], "no-store");
        let back = param(&format!("{base}{expected}"), "return_to").unwrap();
        assert_eq!(
            back,
            original
                .filter(|o| o.starts_with("/app/"))
                .unwrap_or("/account"),
            "the sign-in page reads back what was asked"
        );
    }
    // The proxy keeps the method of the request it guards.
    for method in [
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::DELETE,
    ] {
        let answer = client()
            .request(method.clone(), format!("{base}/login/required"))
            .header("x-original-uri", "/app/?a=1")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 302, "{method}");
        assert_eq!(location(&answer), "/login?return_to=%2Fapp%2F%3Fa%3D1");
    }
}

/// `POST /rooms` with `cookie` (if any): the status.
async fn open_room(base: &str, cookie: Option<&str>) -> u16 {
    let mut asked = client()
        .post(format!("{base}/rooms"))
        .header("content-type", "application/json")
        .body("{}");
    if let Some(cookie) = cookie {
        asked = asked.header("cookie", cookie);
    }
    asked.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn with_a_list_only_a_listed_session_opens_a_room() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (stranger, ..) = credentials(&base, &state, "stranger@example.com").await;
    let (owner, ..) = credentials(&base, &state, "owner@example.com").await;
    assert_eq!(open_room(&base, None).await, 200, "no list: anyone");
    assert_eq!(open_room(&base, Some(&stranger)).await, 200, "no list");

    let (base, _) = restart(&state, list_of("owner@example.com")).await;
    assert_eq!(open_room(&base, None).await, 401, "no session");
    assert_eq!(
        open_room(&base, Some("newera_session=made-up")).await,
        401,
        "a session nobody opened"
    );
    assert_eq!(open_room(&base, Some(&stranger)).await, 401, "unlisted");
    assert_eq!(open_room(&base, Some(&owner)).await, 200, "listed");
}

/// A Buy Me a Coffee delivery of `kind` for `data`, signed as it signs.
async fn bmc(base: &str, kind: &str, data: Value, live: bool, sign: bool) -> u16 {
    bmc_at(base, kind, data, live, sign, 1_800_000_000).await
}

/// The same, with the `created` the envelope carries: what tells a replay
/// from news.
async fn bmc_at(base: &str, kind: &str, data: Value, live: bool, sign: bool, at: i64) -> u16 {
    let body = json!({
        "event_id": 1234,
        "type": kind,
        "live_mode": live,
        "created": at,
        "attempt": 1,
        "data": data,
    })
    .to_string();
    let mut request = client().post(format!("{base}/billing/buymeacoffee"));
    if sign {
        request = request.header(
            "x-signature-sha256",
            newera_cloud::billing::signature(BMC_SECRET, body.as_bytes()),
        );
    }
    request.body(body).send().await.unwrap().status().as_u16()
}

/// A membership as Buy Me a Coffee sends it. The flags really do arrive as
/// strings of `"true"`/`"false"`.
fn membership(id: i64, email: &str, status: &str, ending: bool, level: i64) -> Value {
    json!({
        "object": "membership",
        "id": id,
        "psp_id": format!("sub_{id}"),
        "duration_type": "month",
        "status": status,
        "canceled": if status == "canceled" { "true" } else { "false" },
        "cancel_at_period_end": if ending { "true" } else { "false" },
        "paused": if status == "paused" { "true" } else { "false" },
        "supporter_id": 42,
        "supporter_name": "Ana",
        "supporter_email": email,
        "amount": 5.0,
        "currency": "USD",
        "membership_level_id": level,
        "membership_level_name": "Supporter",
        "started_at": 1_800_000_000_i64,
        "current_period_start": 1_800_000_000_i64,
        // Far enough ahead that the plan is live while the test runs.
        "current_period_end": 4_070_908_800_i64,
    })
}

#[tokio::test]
async fn a_membership_raises_the_plan_and_its_end_lowers_it() {
    let Some((base, state)) = service().await else {
        return;
    };
    let plan = |email: &'static str| {
        let db = state.db.clone();
        async move {
            let account =
                newera_cloud::accounts::find_or_create(&db, &AllowedEmails::default(), email)
                    .await
                    .unwrap()
                    .expect("anyone is let in");
            newera_cloud::accounts::plan_of(&db, &account.id)
                .await
                .unwrap()
                .code
        }
    };
    assert_eq!(plan("ana@example.com").await, "free");

    let started = membership(1, "ana@example.com", "active", false, 7);
    assert_eq!(
        bmc(&base, "membership.started", started.clone(), true, false).await,
        401,
        "unsigned"
    );
    assert_eq!(
        bmc(&base, "membership.started", started, true, true).await,
        200
    );
    assert_eq!(plan("ana@example.com").await, "supporter");

    // Cancelled for the end of the period: paid for until then.
    assert_eq!(
        bmc_at(
            &base,
            "membership.cancelled",
            membership(1, "ana@example.com", "active", true, 7),
            true,
            true,
            1_800_000_100,
        )
        .await,
        200
    );
    assert_eq!(
        plan("ana@example.com").await,
        "supporter",
        "the period already paid for runs out on its own"
    );

    // Paused: not paying, so not on the plan.
    assert_eq!(
        bmc_at(
            &base,
            "membership.paused",
            membership(1, "ana@example.com", "paused", false, 7),
            true,
            true,
            1_800_000_200,
        )
        .await,
        200
    );
    assert_eq!(plan("ana@example.com").await, "free");

    // A retry of the event before it must not undo the newer one.
    assert_eq!(
        bmc_at(
            &base,
            "membership.started",
            membership(1, "ana@example.com", "active", false, 7),
            true,
            true,
            1_800_000_000,
        )
        .await,
        200
    );
    assert_eq!(
        plan("ana@example.com").await,
        "free",
        "an older event leaves the row alone"
    );

    // Gone for good.
    assert_eq!(
        bmc_at(
            &base,
            "membership.cancelled",
            membership(1, "ana@example.com", "canceled", false, 7),
            true,
            true,
            1_800_000_300,
        )
        .await,
        200
    );
    assert_eq!(plan("ana@example.com").await, "free");

    // An address nobody signed in with yet gets the account the sign-in link
    // would have made.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(2, "BIA@example.com", "active", false, 7),
            true,
            true
        )
        .await,
        200
    );
    assert_eq!(plan("bia@example.com").await, "supporter", "same address");

    // Monthly support carries no level. While the page names the levels that
    // pay for the plan, that is not one of them — otherwise any amount at all
    // would buy the plan the levels were meant to fence off.
    let mut monthly = membership(3, "caio@example.com", "active", false, 7);
    monthly["object"] = json!("recurring_donation");
    monthly
        .as_object_mut()
        .unwrap()
        .remove("membership_level_id");
    assert_eq!(
        bmc(&base, "recurring_donation.started", monthly, true, true).await,
        200
    );
    assert_eq!(plan("caio@example.com").await, "free");

    // A level this page does not sell the plan for changes nothing.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(4, "dan@example.com", "active", false, 99),
            true,
            true
        )
        .await,
        200
    );
    assert_eq!(plan("dan@example.com").await, "free");

    // Moved down to a level that does not pay for the plan: the plan goes
    // with it. Left alone, the row would keep saying the account pays, and
    // every later event for that membership — the cancellation included —
    // would be dropped for having the wrong level.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(5, "edu@example.com", "active", false, 7),
            true,
            true
        )
        .await,
        200
    );
    assert_eq!(plan("edu@example.com").await, "supporter");
    assert_eq!(
        bmc_at(
            &base,
            "membership.updated",
            membership(5, "edu@example.com", "active", false, 99),
            true,
            true,
            1_800_000_400,
        )
        .await,
        200
    );
    assert_eq!(
        plan("edu@example.com").await,
        "free",
        "the plan does not outlive the level that bought it"
    );

    // A test event from the dashboard never gives the plan away.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(5, "eva@example.com", "active", false, 7),
            false,
            true
        )
        .await,
        202
    );
    assert_eq!(plan("eva@example.com").await, "free");

    // A one-off coffee is thanked for and changes no plan.
    assert_eq!(
        bmc(
            &base,
            "donation.created",
            json!({"supporter_email": "fab@example.com", "amount": 5.0}),
            true,
            true
        )
        .await,
        202
    );
    assert_eq!(plan("fab@example.com").await, "free");

    // A membership with nobody's address is accepted and dropped: retrying
    // it would never go any better.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            json!({"id": 6, "status": "active", "membership_level_id": 7}),
            true,
            true
        )
        .await,
        202
    );
}

#[tokio::test]
async fn the_account_page_sends_people_to_buy_me_a_coffee_and_back() {
    let Some((base, state)) = service().await else {
        return;
    };
    let http = client();
    // The bare address sends a browser to the site.
    let home = http.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(home.status(), 308);
    assert_eq!(location(&home), format!("{SITE}/"));

    // Signed out, the support page asks for the account first.
    let away = http
        .get(format!("{base}/billing/checkout"))
        .send()
        .await
        .unwrap();
    assert_eq!(away.status(), 303);
    assert_eq!(location(&away), "/login?return_to=/billing/checkout");

    let (cookie, _) = sign_in(&base, &state, "cid@example.com", "/account").await;
    let account = |path: &'static str| {
        let (http, cookie, base) = (http.clone(), cookie.clone(), base.clone());
        async move {
            http.get(format!("{base}{path}"))
                .header("cookie", &cookie)
                .send()
                .await
                .unwrap()
        }
    };
    let page = account("/account").await.text().await.unwrap();
    assert!(
        page.contains(r#"href="/billing/checkout""#),
        "free: offers the plan"
    );

    let checkout = account("/billing/checkout").await.text().await.unwrap();
    let membership_url = format!("https://buymeacoffee.com/{BMC_PAGE}/membership");
    assert!(checkout.contains(&membership_url), "the way to pay");
    assert!(
        checkout.contains("cid@example.com"),
        "the address the plan comes back to"
    );

    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(9, "cid@example.com", "active", false, 7),
            true,
            true
        )
        .await,
        200
    );
    let page = account("/account?paid=1").await.text().await.unwrap();
    assert!(page.contains("Thank you") || page.contains("Obrigado"));
    assert!(
        page.contains(&membership_url),
        "paying: managed on Buy Me a Coffee"
    );
    assert!(
        !page.contains(r#"href="/billing/manage""#),
        "there is no portal of ours to open"
    );
    assert!(
        page.contains("Cancel the subscription first, where it is charged")
            || page.contains("Cancele a assinatura antes, onde ela é cobrada"),
        "closing the account cannot stop the charge"
    );

    // Closing the account stops the plan here; the charge is theirs to stop.
    let csrf = field(&page, "csrf");
    let closed = http
        .post(format!("{base}/account/close"))
        .header("cookie", &cookie)
        .form(&[("csrf", csrf.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), 200);
    let done = closed.text().await.unwrap();
    assert!(
        done.contains("still active") || done.contains("continua ativa"),
        "said again on the way out"
    );
    let status: Option<String> = sqlx::query_scalar(
        "select status from subscriptions where provider = 'buymeacoffee'
         and provider_subscription = '9'",
    )
    .fetch_optional(&state.db)
    .await
    .unwrap();
    assert_eq!(status.as_deref(), Some("canceled"), "no longer paid here");

    // Buy Me a Coffee goes on charging a membership nobody here can cancel,
    // so another event for the closed address arrives sooner or later. It
    // must not undo the deletion by making the account again.
    assert_eq!(
        bmc(
            &base,
            "membership.updated",
            membership(9, "cid@example.com", "active", false, 7),
            true,
            true
        )
        .await,
        200
    );
    let live: i64 = sqlx::query_scalar(
        "select count(*) from accounts where lower(email) = lower($1) and deleted_at is null",
    )
    .bind("cid@example.com")
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(live, 0, "a closed account is not reopened by a payment");
}

/// An installation that is someone's own, letting in the two addresses its
/// list names, written the way a person types them.
async fn owned_service() -> Option<(String, AppState)> {
    let allowed = AllowedEmails::parse(Some(" Owner@Example.com , partner@example.com,"))
        .expect("a list of addresses");
    service_with(vec![SITE.to_owned()], allowed, Some(google().await)).await
}

/// Google, stood in for on a local port: the code a sign-in brings back is
/// the address Google vouches for, verified.
async fn google() -> Google {
    use axum::routing::{get, post};
    let stand_in = axum::Router::new()
        .route(
            "/token",
            post(
                |axum::Form(form): axum::Form<std::collections::HashMap<String, String>>| async move {
                    assert_eq!(form["grant_type"], "authorization_code");
                    assert_eq!(form["client_id"], "test-client");
                    axum::Json(json!({"access_token": form["code"]}))
                },
            ),
        )
        .route(
            "/userinfo",
            get(|headers: axum::http::HeaderMap| async move {
                let email = headers["authorization"]
                    .to_str()
                    .unwrap()
                    .strip_prefix("Bearer ")
                    .unwrap()
                    .to_owned();
                axum::Json(json!({"email": email, "email_verified": true}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, stand_in).await.unwrap() });
    Google {
        client_id: "test-client".to_owned(),
        client_secret: "test-secret".to_owned(),
        token_url: format!("{at}/token"),
        userinfo_url: format!("{at}/userinfo"),
    }
}

/// Signs in with Google as `email`: off to Google, and back with its code.
/// The answer to the return trip, and the session cookie it set, if any.
async fn google_sign_in(base: &str, email: &str) -> (u16, Option<String>) {
    let http = client();
    let off = http
        .get(format!("{base}/login/google?return_to=/account"))
        .send()
        .await
        .unwrap();
    assert_eq!(off.status(), 303);
    let nonce = param(&location(&off), "state").expect("a state");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", email)
        .append_pair("state", &nonce)
        .finish();
    let back = http
        .get(format!("{base}/login/google/callback?{query}"))
        .header("cookie", set_cookie(&off))
        .send()
        .await
        .unwrap();
    let session = back
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("newera_session="))
        .map(|v| v.split(';').next().unwrap().to_owned());
    if back.status() == 303 {
        assert_eq!(location(&back), "/account");
    }
    (back.status().as_u16(), session)
}

/// The plan of an installation's owners.
fn owner_plan() -> Value {
    json!({
        "code": "owner",
        "name": "Owner",
        "limits": {
            "draft_renders_per_day": 1_000_000,
            "hq_renders_per_month": 1_000_000,
            "projects": 1_000_000,
            "storage_mb": 1_000_000,
        },
    })
}

/// The plan of everyone else.
fn free_plan() -> Value {
    json!({
        "code": "free",
        "name": "Free",
        "limits": {
            "draft_renders_per_day": 20,
            "hq_renders_per_month": 0,
            "projects": 3,
            "storage_mb": 50,
        },
    })
}

/// What `/account/me` says about the browser holding `cookie`.
async fn me(base: &str, cookie: &str) -> Value {
    client()
        .get(format!("{base}/account/me"))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn count(state: &AppState, sql: &'static str, email: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(email)
        .fetch_one(&state.db)
        .await
        .unwrap()
}

/// A sign-in link for `email` that nobody asked `/login` for, such as one
/// sent before the list was set.
async fn planted_link(state: &AppState, email: &str) -> String {
    let token = secret::token();
    sqlx::query(
        "insert into login_links (token_hash, email, return_to, expires_at)
         values ($1, $2, '/account', now() + interval '15 minutes')",
    )
    .bind(secret::hash(&token))
    .bind(email)
    .execute(&state.db)
    .await
    .unwrap();
    token
}

#[tokio::test]
async fn a_listed_address_signs_in_on_the_owner_plan() {
    let Some((base, state)) = owned_service().await else {
        return;
    };
    let owner_plan = owner_plan();
    // Made on its first sign-in, in whatever case the person types it.
    let (cookie, _) = sign_in(&base, &state, "OWNER@example.com", "/account").await;
    assert_eq!(
        me(&base, &cookie).await,
        json!({
            "signed_in": true,
            "email": "OWNER@example.com",
            "plan": owner_plan,
            "mcp_url": format!("{base}/mcp"),
        })
    );
    // An account that was here before the list, on the free plan, is moved
    // onto the owner plan when it signs in.
    sqlx::query("insert into accounts (id, email) values ($1, 'partner@example.com')")
        .bind(secret::id())
        .execute(&state.db)
        .await
        .unwrap();
    let (cookie, _) = sign_in(&base, &state, "partner@example.com", "/account").await;
    assert_eq!(me(&base, &cookie).await["plan"], owner_plan);
    // A membership the owner also pays for does not lower the plan.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(6, "partner@example.com", "active", false, 7),
            true,
            true
        )
        .await,
        200
    );
    assert_eq!(me(&base, &cookie).await["plan"], owner_plan);
    assert_eq!(
        count(
            &state,
            "select count(*) from accounts where lower(email) = $1",
            "partner@example.com"
        )
        .await,
        1,
        "the account it had, not a second one"
    );
}

#[tokio::test]
async fn an_unlisted_address_gets_no_link_and_no_account() {
    let Some((base, state)) = owned_service().await else {
        return;
    };
    let http = client();
    let ask = |email: &'static str| {
        let (http, base) = (http.clone(), base.clone());
        async move {
            let response = http
                .post(format!("{base}/login"))
                .form(&[("email", email), ("return_to", "/account")])
                .send()
                .await
                .unwrap();
            (response.status().as_u16(), response.text().await.unwrap())
        }
    };
    // The page says the same as for a listed address: nothing tells which
    // addresses are let in.
    let (status, page) = ask("stranger@example.com").await;
    let (listed_status, listed_page) = ask("owner@example.com").await;
    assert_eq!((status, listed_status), (200, 200));
    assert_eq!(
        page,
        listed_page.replace("owner@example.com", "stranger@example.com")
    );
    let links = "select count(*) from login_links where email = $1";
    // The listed address's link is stored after the answer: once its row is
    // there, the unlisted one's insert (the same task, asked just before)
    // has had its turn too.
    let mut owner = 0;
    for _ in 0..50 {
        owner = count(&state, links, "owner@example.com").await;
        if owner > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        owner, 1,
        "no login link stored for the listed address within 5 s"
    );
    assert_eq!(
        count(&state, links, "stranger@example.com").await,
        0,
        "no link"
    );

    // A proven address that is not listed still opens nothing.
    let token = planted_link(&state, "stranger@example.com").await;
    let refused = http
        .post(format!("{base}/login/link"))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 403);
    assert!(refused.headers().get("set-cookie").is_none(), "no session");

    // Nor does a payment for one.
    assert_eq!(
        bmc(
            &base,
            "membership.started",
            membership(5, "stranger@example.com", "active", false, 7),
            true,
            true
        )
        .await,
        200
    );
    assert_eq!(
        count(
            &state,
            "select count(*) from accounts where lower(email) = $1",
            "stranger@example.com"
        )
        .await,
        0,
        "no account by any route"
    );
}

#[tokio::test]
async fn without_a_list_anyone_signs_in_on_the_free_plan() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (cookie, _) = sign_in(&base, &state, "stranger@example.com", "/account").await;
    assert_eq!(me(&base, &cookie).await["plan"], free_plan());
}

#[tokio::test]
async fn a_listed_address_signs_in_with_google_on_the_owner_plan() {
    let Some((base, state)) = owned_service().await else {
        return;
    };
    let (status, cookie) = google_sign_in(&base, "OWNER@example.com").await;
    assert_eq!(status, 303);
    assert_eq!(
        me(&base, &cookie.expect("a session")).await,
        json!({
            "signed_in": true,
            "email": "OWNER@example.com",
            "plan": owner_plan(),
            "mcp_url": format!("{base}/mcp"),
        })
    );
    // An account that was here before the list is moved onto the owner plan.
    sqlx::query("insert into accounts (id, email) values ($1, 'partner@example.com')")
        .bind(secret::id())
        .execute(&state.db)
        .await
        .unwrap();
    let (status, cookie) = google_sign_in(&base, "partner@example.com").await;
    assert_eq!(status, 303);
    assert_eq!(
        me(&base, &cookie.expect("a session")).await["plan"],
        owner_plan()
    );
    assert_eq!(
        count(
            &state,
            "select count(*) from accounts where lower(email) = $1",
            "partner@example.com"
        )
        .await,
        1,
        "the account it had, not a second one"
    );
}

#[tokio::test]
async fn google_vouching_for_an_unlisted_address_opens_nothing() {
    let Some((base, state)) = owned_service().await else {
        return;
    };
    let (status, cookie) = google_sign_in(&base, "stranger@example.com").await;
    assert_eq!((status, cookie), (403, None), "refused, no session");
    assert_eq!(
        count(
            &state,
            "select count(*) from accounts where lower(email) = $1",
            "stranger@example.com"
        )
        .await,
        0,
        "no account"
    );
}

#[tokio::test]
async fn without_a_list_anyone_signs_in_with_google_on_the_free_plan() {
    let Some((base, _state)) = service_with(
        vec![SITE.to_owned()],
        AllowedEmails::default(),
        Some(google().await),
    )
    .await
    else {
        return;
    };
    let (status, cookie) = google_sign_in(&base, "stranger@example.com").await;
    assert_eq!(status, 303);
    assert_eq!(
        me(&base, &cookie.expect("a session")).await["plan"],
        free_plan()
    );
}

/// The service started again over the same database, with another list.
async fn restart(state: &AppState, allowed: AllowedEmails) -> (String, AppState) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = (*state.config).clone();
    config.public_url = base.clone();
    config.allowed_emails = allowed;
    let again = AppState::start(config, state.db.clone())
        .await
        .expect("startup");
    let app = newera_cloud::router(&again);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, again)
}

fn list_of(addresses: &str) -> AllowedEmails {
    AllowedEmails::parse(Some(addresses)).expect("a list of addresses")
}

async fn plan_code(state: &AppState, email: &str) -> String {
    let id: String = sqlx::query_scalar("select id from accounts where lower(email) = $1")
        .bind(email)
        .fetch_one(&state.db)
        .await
        .unwrap();
    newera_cloud::accounts::plan_of(&state.db, &id)
        .await
        .unwrap()
        .code
}

async fn account_with(state: &AppState, email: &str) {
    sqlx::query("insert into accounts (id, email) values ($1, $2)")
        .bind(secret::id())
        .bind(email)
        .execute(&state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn startup_gives_the_owner_plan_to_the_listed_and_takes_it_from_the_rest() {
    let Some((_, state)) = service().await else {
        return;
    };
    account_with(&state, "owner@example.com").await;
    account_with(&state, "stranger@example.com").await;
    assert_eq!(plan_code(&state, "owner@example.com").await, "free");

    // Started with a list: the listed account is an owner without signing in.
    let (_, state) = restart(&state, list_of("Owner@Example.com")).await;
    assert_eq!(plan_code(&state, "owner@example.com").await, "owner");
    assert_eq!(plan_code(&state, "stranger@example.com").await, "free");

    // Started with the address gone from the list: no longer an owner.
    let (_, state) = restart(&state, list_of("partner@example.com")).await;
    assert_eq!(plan_code(&state, "owner@example.com").await, "free");

    // Listed again, the address holds it.
    let (_, state) = restart(&state, list_of("owner@example.com")).await;
    assert_eq!(plan_code(&state, "owner@example.com").await, "owner");

    // And with no list at all, nobody holds it.
    let (_, state) = restart(&state, AllowedEmails::default()).await;
    assert_eq!(plan_code(&state, "owner@example.com").await, "free");
}

/// What an account that has been signed in holds: a browser cookie, an MCP
/// access token and a refresh token.
async fn credentials(base: &str, state: &AppState, email: &str) -> (String, String, String) {
    let (cookie, _) = sign_in(base, state, email, "/account").await;
    let id: String = sqlx::query_scalar("select id from accounts where lower(email) = lower($1)")
        .bind(email)
        .fetch_one(&state.db)
        .await
        .unwrap();
    let (access, refresh) = (secret::token(), secret::token());
    for (token, kind, days) in [(&access, "access", 1), (&refresh, "refresh", 30)] {
        sqlx::query(
            "insert into oauth_tokens (token_hash, kind, client_id, account_id, scope, family, expires_at)
             values ($1, $2, 'test-client', $3, 'newera', $4, now() + make_interval(days => $5))",
        )
        .bind(secret::hash(token))
        .bind(kind)
        .bind(&id)
        .bind(format!("family-{kind}-{email}"))
        .bind(days)
        .execute(&state.db)
        .await
        .unwrap();
    }
    (cookie, access, refresh)
}

async fn ping(base: &str, access: &str) -> u16 {
    mcp(
        base,
        access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    )
    .await
    .0
}

async fn refresh_status(base: &str, refresh: &str) -> u16 {
    client()
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", "test-client"),
        ])
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn an_unlisted_account_loses_its_browser_session_when_a_list_is_set() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (stranger, ..) = credentials(&base, &state, "stranger@example.com").await;
    let (owner, ..) = credentials(&base, &state, "owner@example.com").await;
    assert_eq!(me(&base, &stranger).await["signed_in"], true);

    let (base, _) = restart(&state, list_of("owner@example.com")).await;
    assert_eq!(me(&base, &stranger).await["signed_in"], false);
    assert_eq!(me(&base, &owner).await["signed_in"], true);
}

#[tokio::test]
async fn an_unlisted_account_loses_its_access_token_when_a_list_is_set() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (_, stranger, _) = credentials(&base, &state, "stranger@example.com").await;
    let (_, owner, _) = credentials(&base, &state, "owner@example.com").await;
    assert_eq!(ping(&base, &stranger).await, 200);

    let (base, _) = restart(&state, list_of("owner@example.com")).await;
    assert_eq!(ping(&base, &stranger).await, 401);
    assert_eq!(ping(&base, &owner).await, 200);
}

#[tokio::test]
async fn an_unlisted_account_loses_its_refresh_token_when_a_list_is_set() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (.., stranger) = credentials(&base, &state, "stranger@example.com").await;
    let (.., owner) = credentials(&base, &state, "owner@example.com").await;

    let (base, _) = restart(&state, list_of("owner@example.com")).await;
    assert_eq!(refresh_status(&base, &stranger).await, 400);
    assert_eq!(refresh_status(&base, &owner).await, 200);
}

async fn ask_for_a_link(base: &str, email: &str) -> (u16, String) {
    let response = client()
        .post(format!("{base}/login"))
        .form(&[("email", email), ("return_to", "/account")])
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

#[tokio::test]
async fn with_mail_off_every_address_gets_the_same_answer() {
    let Some((base, state)) = service_full(
        vec![SITE.to_owned()],
        list_of("owner@example.com"),
        None,
        Mail::Off,
        None,
    )
    .await
    else {
        return;
    };
    let (listed_status, listed) = ask_for_a_link(&base, "owner@example.com").await;
    let (status, unlisted) = ask_for_a_link(&base, "stranger@example.com").await;
    assert_eq!((listed_status, status), (200, 200));
    assert_eq!(listed, unlisted);
    assert!(listed.contains("not available"), "{listed}");
    let links: i64 = sqlx::query_scalar("select count(*) from login_links")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(links, 0);
}

#[tokio::test]
async fn a_failed_send_looks_the_same_for_listed_and_unlisted_addresses() {
    // Sending goes through a proxy nobody listens on: every send fails, and
    // nothing leaves the machine.
    let dead = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
        .build()
        .unwrap();
    let Some((base, state)) = service_full(
        vec![SITE.to_owned()],
        list_of("owner@example.com"),
        None,
        Mail::Resend {
            api_key: "test-key-not-a-secret".to_owned(),
            from: "test <test@example.com>".to_owned(),
        },
        Some(dead),
    )
    .await
    else {
        return;
    };
    let (listed_status, listed) = ask_for_a_link(&base, "owner@example.com").await;
    let (status, unlisted) = ask_for_a_link(&base, "stranger@example.com").await;
    assert_eq!((listed_status, status), (200, 200));
    assert_eq!(
        listed,
        unlisted.replace("stranger@example.com", "owner@example.com")
    );
    let mut links: i64 = 0;
    for _ in 0..50 {
        links = sqlx::query_scalar("select count(*) from login_links")
            .fetch_one(&state.db)
            .await
            .unwrap();
        if links > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(links, 1, "only the listed address was given a link");
}

#[tokio::test]
async fn a_slow_send_does_not_slow_the_answer_for_a_listed_address() {
    // A proxy that takes the connection and never answers: a send through it
    // waits as long as the client's timeout.
    let stall = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", stall.local_addr().unwrap());
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = stall.accept().await {
            held.push(socket);
        }
    });
    let slow = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).unwrap())
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let Some((base, _)) = service_full(
        vec![SITE.to_owned()],
        list_of("owner@example.com"),
        None,
        Mail::Resend {
            api_key: "test-key-not-a-secret".to_owned(),
            from: "test <test@example.com>".to_owned(),
        },
        Some(slow),
    )
    .await
    else {
        holder.abort();
        return;
    };
    let mut took = Vec::new();
    for email in ["owner@example.com", "stranger@example.com"] {
        let started = std::time::Instant::now();
        let (status, _) = ask_for_a_link(&base, email).await;
        assert_eq!(status, 200);
        took.push(started.elapsed());
    }
    holder.abort();
    assert!(
        took.iter()
            .all(|t| *t < std::time::Duration::from_millis(500)),
        "answers took {took:?}"
    );
}

/// An account with one cloud project the engine has open: four walls drawn
/// through the AI, kept once. Answers the cookie, the access token and the
/// project's id.
async fn drawn_project(base: &str, state: &AppState, email: &str) -> (String, String, String) {
    let (cookie, access, _) = credentials(base, state, email).await;
    let drawn = tool(
        base,
        &access,
        "create",
        json!({"walls": [{"pts": [[0, 0], [400, 0], [400, 300], [0, 300]], "closed": true}]}),
    )
    .await;
    assert_eq!(drawn["isError"], false, "{drawn}");
    let id: String = sqlx::query_scalar(
        "select p.id from projects p join accounts a on a.id = p.account_id
         where lower(a.email) = lower($1)",
    )
    .bind(email)
    .fetch_one(&state.db)
    .await
    .unwrap();
    (cookie, access, id)
}

async fn tool(base: &str, access: &str, name: &str, arguments: Value) -> Value {
    let (_, answer) = mcp(
        base,
        access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}}),
    )
    .await;
    answer["result"].clone()
}

/// What the engine's copy of the active project holds, by count.
async fn counts(base: &str, access: &str) -> Value {
    let home = tool(base, access, "home", json!({"detail": "summary"})).await;
    let home: Value = serde_json::from_str(home["content"][0]["text"].as_str().unwrap()).unwrap();
    home["counts"].clone()
}

fn counted(walls: u64) -> Value {
    json!({"walls": walls, "rooms": 0, "dims": 0, "labels": 0, "furniture": 0})
}

async fn stored(state: &AppState, id: &str) -> (i64, Vec<u8>) {
    sqlx::query_as("select revision, data from projects where id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .unwrap()
}

fn empty_project() -> Vec<u8> {
    newera_core::to_project_bytes(&newera_core::Document::default())
}

async fn put(
    base: &str,
    id: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    revision: Option<&str>,
    body: Vec<u8>,
) -> reqwest::Response {
    let mut request = client()
        .put(format!("{base}/account/projects/{id}"))
        .body(body);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if let Some(revision) = revision {
        request = request.header("x-project-revision", revision);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn a_project_comes_with_its_revision_and_a_put_on_it_saves_the_next() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (cookie, _, id) = drawn_project(&base, &state, "rev@example.com").await;
    assert_eq!(stored(&state, &id).await.0, 1, "the engine's one save");

    let fetched = client()
        .get(format!("{base}/account/projects/{id}"))
        .header("cookie", &cookie)
        .header("origin", SITE)
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.headers()["x-project-revision"], "1");
    let exposed = fetched.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(exposed.contains("x-project-revision"), "{exposed}");

    // The browser asks first whether the site may PUT with the header.
    let preflight = client()
        .request(
            reqwest::Method::OPTIONS,
            format!("{base}/account/projects/{id}"),
        )
        .header("origin", SITE)
        .header("access-control-request-method", "PUT")
        .header("access-control-request-headers", "x-project-revision")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 200);
    assert_eq!(preflight.headers()["access-control-allow-origin"], SITE);
    assert_eq!(
        preflight.headers()["access-control-allow-credentials"],
        "true"
    );
    assert!(
        preflight.headers()["access-control-allow-methods"]
            .to_str()
            .unwrap()
            .contains("PUT")
    );
    assert!(
        preflight.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("x-project-revision")
    );

    let body = empty_project();
    let saved = put(
        &base,
        &id,
        Some(&cookie),
        Some(SITE),
        Some("1"),
        body.clone(),
    )
    .await;
    assert_eq!(saved.status(), 200);
    assert_eq!(saved.headers()["x-project-revision"], "2");
    assert_eq!(saved.headers()["access-control-allow-credentials"], "true");
    assert_eq!(saved.json::<Value>().await.unwrap(), json!({"revision": 2}));
    assert_eq!(stored(&state, &id).await, (2, body));
}

#[tokio::test]
async fn a_put_on_a_stale_revision_is_refused_and_changes_nothing() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (cookie, _, id) = drawn_project(&base, &state, "stale@example.com").await;
    let before = stored(&state, &id).await;
    for stale in ["0", "2"] {
        let refused = put(
            &base,
            &id,
            Some(&cookie),
            Some(SITE),
            Some(stale),
            empty_project(),
        )
        .await;
        assert_eq!(refused.status(), 409, "revision {stale}");
        assert_eq!(
            refused.json::<Value>().await.unwrap(),
            json!({"error": "the project changed since this tab loaded it", "revision": 1})
        );
        assert_eq!(stored(&state, &id).await, before);
    }
    let missing = put(&base, &id, Some(&cookie), Some(SITE), None, empty_project()).await;
    assert_eq!(missing.status(), 428);
    let garbled = put(
        &base,
        &id,
        Some(&cookie),
        Some(SITE),
        Some("one"),
        empty_project(),
    )
    .await;
    assert_eq!(garbled.status(), 428);
    assert_eq!(stored(&state, &id).await, before);
}

#[tokio::test]
async fn a_put_is_what_the_next_headless_call_works_on() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (cookie, access, id) = drawn_project(&base, &state, "evict@example.com").await;
    assert_eq!(counts(&base, &access).await, counted(4));
    let saved = put(
        &base,
        &id,
        Some(&cookie),
        Some(SITE),
        Some("1"),
        empty_project(),
    )
    .await;
    assert_eq!(saved.status(), 200);
    assert_eq!(counts(&base, &access).await, counted(0));
    // And the engine's own next save follows on from the PUT's revision.
    let drawn = tool(
        &base,
        &access,
        "create",
        json!({"walls": [{"pts": [[0, 0], [100, 0]]}]}),
    )
    .await;
    assert_eq!(drawn["isError"], false, "{drawn}");
    assert_eq!(stored(&state, &id).await.0, 3);
}

#[tokio::test]
async fn the_engine_does_not_save_over_a_project_that_changed_under_it() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (_, access, id) = drawn_project(&base, &state, "cas@example.com").await;
    assert_eq!(counts(&base, &access).await, counted(4));
    // Another writer (another process of the service) keeps an empty project.
    let theirs = empty_project();
    sqlx::query("update projects set data = $1, revision = revision + 1 where id = $2")
        .bind(&theirs)
        .bind(&id)
        .execute(&state.db)
        .await
        .unwrap();
    let refused = tool(
        &base,
        &access,
        "create",
        json!({"walls": [{"pts": [[0, 0], [100, 0]]}]}),
    )
    .await;
    assert_eq!(
        refused,
        json!({"content": [{"type": "text", "text": "the project changed, call again"}], "isError": true})
    );
    assert_eq!(stored(&state, &id).await, (2, theirs));
    // The engine reloaded: it now works on the other writer's copy.
    assert_eq!(counts(&base, &access).await, counted(0));
    let drawn = tool(
        &base,
        &access,
        "create",
        json!({"walls": [{"pts": [[0, 0], [100, 0]]}]}),
    )
    .await;
    assert_eq!(drawn["isError"], false, "{drawn}");
    assert_eq!(stored(&state, &id).await.0, 3);
    assert_eq!(counts(&base, &access).await, counted(1));
    // Its next save follows on from the one it just made.
    let again = tool(
        &base,
        &access,
        "create",
        json!({"walls": [{"pts": [[0, 100], [100, 100]]}]}),
    )
    .await;
    assert_eq!(again["isError"], false, "{again}");
    assert_eq!(stored(&state, &id).await.0, 4);
}

#[tokio::test]
async fn a_put_counts_against_the_plan_storage() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (cookie, _, id) = drawn_project(&base, &state, "full@example.com").await;
    let before = stored(&state, &id).await;
    sqlx::query(
        r#"insert into plans (code, name, limits) values ('tiny', 'Tiny', '{"projects": 3, "storage_mb": 0, "draft_renders_per_day": 0, "hq_renders_per_month": 0}')"#,
    )
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query("update accounts set given_plan = 'tiny' where email = 'full@example.com'")
        .execute(&state.db)
        .await
        .unwrap();
    // Bigger than all the plan stores: not even read.
    let refused = put(
        &base,
        &id,
        Some(&cookie),
        Some(SITE),
        Some("1"),
        empty_project(),
    )
    .await;
    assert_eq!(refused.status(), 413);
    assert_eq!(
        refused.json::<Value>().await.unwrap(),
        json!({"error": "the Tiny plan stores 0 MB of projects and this one is bigger than that"})
    );
    assert_eq!(stored(&state, &id).await, before);
    // Room for it alone, not beside the account's other project.
    sqlx::query(
        "update plans set limits = jsonb_set(limits, '{storage_mb}', '1') where code = 'tiny'",
    )
    .execute(&state.db)
    .await
    .unwrap();
    // A body bigger than all the plan stores is refused once that much has
    // arrived, without waiting for the rest: here the rest never comes.
    let address = base.trim_start_matches("http://").to_owned();
    let head = format!(
        "PUT /account/projects/{id} HTTP/1.1\r\nhost: x\r\ncookie: {cookie}\r\norigin: {SITE}\r\nx-project-revision: 1\r\ncontent-length: 10485760\r\n\r\nPK"
    );
    let answer = tokio::task::spawn_blocking(move || {
        use std::io::{Read as _, Write as _};
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        socket.write_all(head.as_bytes()).unwrap();
        // More than the megabyte the plan stores, of the ten announced.
        socket.write_all(&vec![0_u8; 1024 * 1024 + 1]).unwrap();
        let mut answer = vec![0_u8; 12];
        socket
            .read_exact(&mut answer)
            .expect("an answer before the body ends");
        answer
    })
    .await
    .unwrap();
    assert_eq!(answer, b"HTTP/1.1 413");
    assert_eq!(stored(&state, &id).await, before);
    sqlx::query(
        "insert into projects (id, account_id, name, data, size)
         select 'other', account_id, 'Other', '', 1048576 from projects where id = $1",
    )
    .bind(&id)
    .execute(&state.db)
    .await
    .unwrap();
    let refused = put(
        &base,
        &id,
        Some(&cookie),
        Some(SITE),
        Some("1"),
        empty_project(),
    )
    .await;
    assert_eq!(refused.status(), 413);
    assert_eq!(
        refused.json::<Value>().await.unwrap(),
        json!({"error": "the Tiny plan stores 1 MB and this would take 1 MB"})
    );
    assert_eq!(stored(&state, &id).await, before);
    // With the other one gone, it fits.
    sqlx::query("delete from projects where id = 'other'")
        .execute(&state.db)
        .await
        .unwrap();
    let saved = put(
        &base,
        &id,
        Some(&cookie),
        Some(SITE),
        Some("1"),
        empty_project(),
    )
    .await;
    assert_eq!(saved.status(), 200);
    assert_eq!(stored(&state, &id).await, (2, empty_project()));
}

#[tokio::test]
async fn only_the_owner_puts_a_project_and_only_from_the_site() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (cookie, _, id) = drawn_project(&base, &state, "owner1@example.com").await;
    let (other, _, _) = credentials(&base, &state, "other@example.com").await;
    let before = stored(&state, &id).await;
    let anonymous = put(&base, &id, None, Some(SITE), Some("1"), empty_project()).await;
    assert_eq!(anonymous.status(), 401);
    let stranger = put(
        &base,
        &id,
        Some(&other),
        Some(SITE),
        Some("1"),
        empty_project(),
    )
    .await;
    assert_eq!(stranger.status(), 404);
    let elsewhere = put(
        &base,
        &id,
        Some(&cookie),
        Some("https://evil.example"),
        Some("1"),
        empty_project(),
    )
    .await;
    assert_eq!(elsewhere.status(), 403);
    let no_origin = put(&base, &id, Some(&cookie), None, Some("1"), empty_project()).await;
    assert_eq!(no_origin.status(), 403);
    assert_eq!(stored(&state, &id).await, before);
}

/// What the stored project holds, by walls, as the engine reads it back.
async fn walls_in_db(base: &str, access: &str, id: &str) -> u64 {
    let opened = tool(base, access, "file", json!({"action": "open", "path": id})).await;
    assert_eq!(opened["isError"], false, "{opened}");
    counts(base, access).await["walls"].as_u64().unwrap()
}

async fn add_wall(base: &str, access: &str, y: i64) -> Value {
    tool(
        base,
        access,
        "create",
        json!({"walls": [{"pts": [[0, y], [100, y]]}]}),
    )
    .await
}

#[tokio::test]
async fn parallel_engine_calls_are_all_saved() {
    let Some((base, state)) = service().await else {
        return;
    };
    for round in 0..5 {
        let (_, access, id) =
            drawn_project(&base, &state, &format!("par{round}@example.com")).await;
        let calls: Vec<_> = (0..8)
            .map(|i| {
                let (base, access) = (base.clone(), access.clone());
                tokio::spawn(async move { add_wall(&base, &access, 500 + i * 10).await })
            })
            .collect();
        for call in calls {
            let answer = call.await.unwrap();
            assert_eq!(answer["isError"], false, "round {round}: {answer}");
        }
        // Each took the revision the one before it left: none was stale.
        assert_eq!(stored(&state, &id).await.0, 9, "round {round}");
        assert_eq!(walls_in_db(&base, &access, &id).await, 12, "round {round}");
    }
}

#[tokio::test]
async fn calls_arriving_together_open_one_copy() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "single@example.com").await;
    let calls: Vec<_> = (0..8)
        .map(|i| {
            let (base, access) = (base.clone(), access.clone());
            tokio::spawn(async move { add_wall(&base, &access, 500 + i * 10).await })
        })
        .collect();
    for call in calls {
        let answer = call.await.unwrap();
        assert_eq!(answer["isError"], false, "{answer}");
    }
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "select p.id, p.revision from projects p join accounts a on a.id = p.account_id
         where a.email = 'single@example.com'",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, 8);
    assert_eq!(walls_in_db(&base, &access, &rows[0].0).await, 8);
}

#[tokio::test]
async fn a_put_racing_engine_calls_loses_nothing() {
    let Some((base, state)) = service().await else {
        return;
    };
    for round in 0..30 {
        let (cookie, access, id) =
            drawn_project(&base, &state, &format!("race{round}@example.com")).await;
        let calls: Vec<_> = (0..6)
            .map(|i| {
                let (base, access) = (base.clone(), access.clone());
                tokio::spawn(async move { add_wall(&base, &access, 500 + i * 10).await })
            })
            .collect();
        let put = put(
            &base,
            &id,
            Some(&cookie),
            Some(SITE),
            Some("1"),
            empty_project(),
        )
        .await;
        let status = put.status().as_u16();
        let mut ok = 0_u64;
        for call in calls {
            let answer = call.await.unwrap();
            if answer["isError"] == false {
                ok += 1;
            } else {
                assert_eq!(
                    answer["content"][0]["text"], "the project changed, call again",
                    "round {round}: {answer}"
                );
            }
        }
        assert!(status == 200 || status == 409, "round {round}: {status}");
        let (revision, _) = stored(&state, &id).await;
        let engine_walls = counts(&base, &access).await["walls"].as_u64().unwrap();
        let stored_walls = walls_in_db(&base, &access, &id).await;
        let put_kept = u64::from(status == 200);
        assert_eq!(revision.cast_unsigned(), 1 + ok + put_kept, "round {round}");
        assert_eq!(engine_walls, stored_walls, "round {round}: engine diverged");
        // A kept PUT is the project, plus only what was saved after it.
        let before_calls = if status == 200 { 0 } else { 4 };
        assert_eq!(
            stored_walls,
            before_calls + ok,
            "round {round}: status {status}"
        );
    }
}

#[tokio::test]
async fn old_and_new_copies_saving_at_once_do_not_cross() {
    let Some((base, state)) = service().await else {
        return;
    };
    for round in 0..30 {
        let (cookie, access, id) =
            drawn_project(&base, &state, &format!("two{round}@example.com")).await;
        let mut calls: Vec<_> = (0..6)
            .map(|i| {
                let (base, access) = (base.clone(), access.clone());
                tokio::spawn(async move { add_wall(&base, &access, 500 + i * 10).await })
            })
            .collect();
        let status = put(
            &base,
            &id,
            Some(&cookie),
            Some(SITE),
            Some("1"),
            empty_project(),
        )
        .await
        .status()
        .as_u16();
        // Calls made after the PUT answered: copies of the old and the new
        // revision are both saving now.
        calls.extend((0..6).map(|i| {
            let (base, access) = (base.clone(), access.clone());
            tokio::spawn(async move { add_wall(&base, &access, 700 + i * 10).await })
        }));
        let mut ok = 0_u64;
        for call in calls {
            if call.await.unwrap()["isError"] == false {
                ok += 1;
            }
        }
        let (revision, _) = stored(&state, &id).await;
        let engine_walls = counts(&base, &access).await["walls"].as_u64().unwrap();
        let stored_walls = walls_in_db(&base, &access, &id).await;
        let put_kept = u64::from(status == 200);
        assert_eq!(revision.cast_unsigned(), 1 + ok + put_kept, "round {round}");
        assert_eq!(engine_walls, stored_walls, "round {round}: engine diverged");
        let before_calls = if status == 200 { 0 } else { 4 };
        assert_eq!(
            stored_walls,
            before_calls + ok,
            "round {round}: status {status}"
        );
    }
}

// ---------------------------------------------------------------- products

/// `AnySite`, stood in for on a local port with the responses recorded from
/// the real one (`newera-shop/tests/fixtures/`): each answer is chosen by
/// endpoint and by a word of the link asked for, and every call is counted.
async fn anysite(
    answers: &[(&'static str, &'static str, &'static str)],
) -> (
    newera_shop::AnySite,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../newera-shop/tests/fixtures");
    let records = answers
        .iter()
        .map(|(endpoint, word, name)| {
            let record: Value =
                serde_json::from_str(&std::fs::read_to_string(fixtures.join(name)).unwrap())
                    .unwrap();
            ((*endpoint).to_owned(), (*word).to_owned(), record)
        })
        .collect();
    anysite_of(records, std::time::Duration::ZERO).await
}

/// The same from records, each answer coming `delay` after the call.
async fn anysite_of(
    answers: Vec<(String, String, Value)>,
    delay: std::time::Duration,
) -> (
    newera_shop::AnySite,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use axum::extract::{Path, State};
    use axum::response::IntoResponse;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let answers = Arc::new(answers);
    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new()
        .route(
            "/api/{*endpoint}",
            axum::routing::post(
                move |State((answers, calls)): State<StandIn>,
                      Path(endpoint): Path<String>,
                      headers: axum::http::HeaderMap,
                      axum::Json(body): axum::Json<Value>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(delay).await;
                    assert_eq!(headers["access-token"], "anysite-test-key");
                    let link = body["url"]
                        .as_str()
                        .or(body["product"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let (_, _, record) = answers
                        .iter()
                        .find(|(e, word, _)| *e == endpoint && link.contains(word.as_str()))
                        .unwrap_or_else(|| panic!("no recorded answer for {endpoint} {link}"));
                    let mut response = (
                        axum::http::StatusCode::from_u16(
                            u16::try_from(record["status"].as_u64().unwrap()).unwrap(),
                        )
                        .unwrap(),
                        axum::Json(record["body"].clone()),
                    )
                        .into_response();
                    for (name, value) in record["headers"].as_object().unwrap() {
                        response.headers_mut().insert(
                            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                            value.as_str().unwrap().parse().unwrap(),
                        );
                    }
                    response
                },
            ),
        )
        .with_state((answers, calls.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (newera_shop::AnySite::new(&at, "anysite-test-key"), calls)
}

/// The stand-in's recorded answers by endpoint and word, and its call count.
type StandIn = (
    std::sync::Arc<Vec<(String, String, Value)>>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
);

/// The recorded shops, as the stand-in answers them.
const SHOPS: &[(&str, &str, &str)] = &[
    ("ikea/products", "ikea.com", "ikea_billy_00263850.json"),
    (
        "webparser/parse",
        "hornbach.nl",
        "hornbach_tile_10656129.parse.json",
    ),
    (
        "webparser/render",
        "hornbach.nl",
        "hornbach_tile_10656129.render.json",
    ),
    (
        "webparser/parse",
        "tegeldepot.nl",
        "tegeldepot_search.parse.json",
    ),
    (
        "webparser/render",
        "tegeldepot.nl",
        "tegeldepot_search.render.json",
    ),
];

async fn shop_service() -> Option<(
    String,
    AppState,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
)> {
    let (anysite, calls) = anysite(SHOPS).await;
    shop_service_on(anysite, calls).await
}

async fn shop_service_on(
    anysite: newera_shop::AnySite,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> Option<(
    String,
    AppState,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
)> {
    let (base, state) = service_configured(
        vec![SITE.to_owned()],
        AllowedEmails::default(),
        None,
        Mail::Log,
        None,
        Some(anysite),
    )
    .await?;
    Some((base, state, calls))
}

/// A tool's answer, as the fields a client reads.
fn fields_of(answer: &Value) -> Value {
    assert_eq!(answer["isError"], false, "{answer}");
    let text: Value = serde_json::from_str(answer["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(answer["structuredContent"], text, "the fields are the text");
    text
}

fn calls_made(calls: &std::sync::atomic::AtomicUsize) -> usize {
    calls.load(std::sync::atomic::Ordering::SeqCst)
}

/// An IKEA link is read through the adapter once; the same article under
/// another link is the product already kept, with no second fetch; the
/// library lists it, finds it by a word, and shows it whole.
#[tokio::test]
async fn an_imported_product_is_read_once_and_kept() {
    let Some((base, state, calls)) = shop_service().await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let link = "https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/";
    let first = fields_of(&tool(&base, &access, "import_product", json!({"url": link})).await);
    let id = first["id"].as_str().unwrap().to_owned();
    assert_eq!(
        first,
        json!({
            "id": id,
            "shop": "ikea.com",
            "market": "nl",
            "sku": "00263850",
            "url": link,
            "kind": "furniture",
            "title": "BILLY Boekenkast, wit, 80x28x202 cm",
            "fields": first["fields"],
            "missing": [],
            "fetched_at": first["fetched_at"],
            "prices": [["49.99", "EUR", "piece", "adapter", first["prices"][0][4]]],
            "stored": false,
            "fetched": [["ikea/products", "ok", 5]],
        })
    );
    assert_eq!(
        first["fields"]["price"],
        json!({"value": "49.99", "evidence": "adapter", "from": "previous_price (price is the IKEA Family price)"})
    );
    assert_eq!(first["fields"]["width_cm"]["value"], 80.0);
    assert_eq!(calls_made(&calls), 1);

    let again = fields_of(
        &tool(
            &base,
            &access,
            "import_product",
            json!({"url": format!("{link}?utm_source=mail#reviews")}),
        )
        .await,
    );
    assert_eq!(again["id"], first["id"]);
    assert_eq!(
        (&again["stored"], &again["fetched"]),
        (&json!(true), &json!([]))
    );
    assert_eq!(
        calls_made(&calls),
        1,
        "a product already kept is not fetched again"
    );

    let listed = fields_of(&tool(&base, &access, "products", json!({})).await);
    assert_eq!(
        listed,
        json!({
            "rows": [[id, "furniture", "ikea.com", "BILLY Boekenkast, wit, 80x28x202 cm", "49.99", "EUR", "piece", []]],
            "columns": ["id", "kind", "shop", "title", "price", "currency", "sale_unit", "missing"],
        })
    );
    let found = fields_of(&tool(&base, &access, "products", json!({"q": "billy"})).await);
    assert_eq!(found["rows"].as_array().unwrap().len(), 1);
    let none = fields_of(&tool(&base, &access, "products", json!({"q": "sofa"})).await);
    assert_eq!(none["rows"], json!([]));
    let whole = fields_of(&tool(&base, &access, "products", json!({"id": id})).await);
    let mut expected = first.clone();
    expected.as_object_mut().unwrap().remove("stored");
    expected.as_object_mut().unwrap().remove("fetched");
    assert_eq!(whole, expected);

    // Another account has a library of its own.
    let (_, other, _) = credentials(&base, &state, "neighbour@example.com").await;
    let theirs = fields_of(&tool(&base, &other, "products", json!({})).await);
    assert_eq!(theirs["rows"], json!([]));
    let unseen = tool(&base, &other, "products", json!({"id": id})).await;
    assert_eq!(unseen["isError"], true);
}

/// A shop behind a bot check is parsed, then rendered; what neither shows
/// is missing, and the agent fills it. A refresh reads the shop again and
/// keeps the agent's values where the shop still shows nothing.
#[tokio::test]
async fn the_agent_fills_what_the_shop_did_not_show() {
    let Some((base, state, calls)) = shop_service().await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let link = "https://www.tegeldepot.nl/wandtegels/wandtegel-wit-30x60.html";
    let imported = fields_of(
        &tool(
            &base,
            &access,
            "import_product",
            json!({"url": link, "kind": "finish"}),
        )
        .await,
    );
    let id = imported["id"].as_str().unwrap().to_owned();
    assert_eq!(imported["kind"], "finish");
    assert_eq!(imported["fields"], json!({}));
    assert!(
        imported.get("title").is_none(),
        "an unknown title is left out, not null: the output schema says string"
    );
    assert_eq!(
        imported["missing"],
        json!(["title", "price", "finish_type"])
    );
    assert_eq!(
        imported["fetched"],
        json!([
            ["webparser/parse", "ok", 1],
            [
                "webparser/render",
                "AnySite could not reach the page: Failed to fetch URL. Website is not accessible",
                10
            ]
        ])
    );
    assert_eq!(calls_made(&calls), 2);

    let set = fields_of(
        &tool(
            &base,
            &access,
            "edit_products",
            json!({"id": id, "action": "set", "fields": {"title": "Wandtegel wit 30x60", "price": "31,95", "finish_type": "covering"}}),
        )
        .await,
    );
    assert_eq!(
        set["missing"],
        json!(["pack_m2"]),
        "a covering needs its m² per pack"
    );
    assert_eq!(
        set["fields"]["price"],
        json!({"value": "31.95", "evidence": "agent", "from": "edit_products"})
    );
    let done = fields_of(
        &tool(
            &base,
            &access,
            "edit_products",
            json!({"id": id, "action": "set", "fields": {"pack_m2": 1.44}}),
        )
        .await,
    );
    assert_eq!(done["missing"], json!([]));
    assert_eq!(done["prices"].as_array().unwrap().len(), 1);
    assert_eq!(
        &done["prices"][0].as_array().unwrap()[..4],
        [json!("31.95"), json!("EUR"), Value::Null, json!("agent")]
    );

    for (args, says) in [
        (
            json!({"id": id, "action": "set", "fields": {"price": "free"}}),
            "price takes an amount above zero",
        ),
        (
            json!({"id": id, "action": "set", "fields": {"colour_code": "x"}}),
            "no field colour_code",
        ),
        (json!({"id": id, "action": "set"}), "fields is an object"),
        (
            json!({"id": id, "action": "refresh", "fields": {}}),
            "`fields` is for `set`, not `refresh`",
        ),
        (json!({"id": "nope", "action": "remove"}), "no product nope"),
        (
            json!({"url": link, "link": link}),
            "unknown argument `link`",
        ),
    ] {
        let name = if args.get("url").is_some() {
            "import_product"
        } else {
            "edit_products"
        };
        let answer = tool(&base, &access, name, args.clone()).await;
        assert_eq!(answer["isError"], true, "{args}");
        let text = answer["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(says), "{args}: {text}");
    }

    let before: (i64, Value) = sqlx::query_as(
        "select extract(epoch from fetched_at)::bigint, fetches from products where id = $1",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    let refreshed = fields_of(
        &tool(
            &base,
            &access,
            "edit_products",
            json!({"id": id, "action": "refresh"}),
        )
        .await,
    );
    assert_eq!(calls_made(&calls), 4, "a refresh reads the shop again");
    assert_eq!(
        refreshed["fields"]["price"]["evidence"], "agent",
        "the shop still shows no price"
    );
    assert_eq!(refreshed["missing"], json!([]));
    assert_eq!(
        refreshed["fetched_at"], before.0,
        "a read that brought nothing is not a reading of the shop"
    );
    let after: Value = sqlx::query_scalar("select fetches from products where id = $1")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    let mut both = before.1.as_array().unwrap().clone();
    both.extend(refreshed["fetched"].as_array().unwrap().clone());
    assert_eq!(
        after,
        json!(both),
        "the attempt is added to the earlier fetches"
    );
    assert_eq!(after.as_array().unwrap().len(), 4);

    let removed = fields_of(
        &tool(
            &base,
            &access,
            "edit_products",
            json!({"id": id, "action": "remove"}),
        )
        .await,
    );
    assert_eq!(removed, json!({"id": id, "removed": true}));
    let left = fields_of(&tool(&base, &access, "products", json!({})).await);
    assert_eq!(left["rows"], json!([]));
    let history: i64 =
        sqlx::query_scalar("select count(*) from product_prices where product_id = $1")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(history, 0, "the history goes with the product");
}

/// A refresh of a page the shop does show replaces the price and what goes
/// with it, adds the day's price to the history, and leaves the rest of the
/// product — the agent's corrections — as it was.
#[tokio::test]
async fn a_refresh_adds_the_days_price() {
    let Some((base, state, calls)) = shop_service().await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let link =
        "https://www.hornbach.nl/p/wandtegel-elegance-wit-mat-30-x-60-cm-gerectificeerd/10656129/";
    let imported = fields_of(&tool(&base, &access, "import_product", json!({"url": link})).await);
    let id = imported["id"].as_str().unwrap().to_owned();
    assert_eq!(imported["kind"], "finish");
    assert_eq!(imported["missing"], json!([]));
    assert_eq!(imported["fields"]["pack_m2"]["value"], 1.62);
    assert_eq!(
        imported["fetched"],
        json!([["webparser/parse", "ok", 1], ["webparser/render", "ok", 10]])
    );
    // The agent's price, from before the shop showed one, is replaced by
    // what the shop shows now; its corrections of the title and the pack
    // are not the day's price and stay.
    let set = tool(
        &base,
        &access,
        "edit_products",
        json!({"id": id, "action": "set", "fields": {"price": "17.00", "title": "Mijn tegel", "pack_m2": 1.5}}),
    )
    .await;
    assert_eq!(fields_of(&set)["fields"]["price"]["evidence"], "agent");
    let refreshed = fields_of(
        &tool(
            &base,
            &access,
            "edit_products",
            json!({"id": id, "action": "refresh"}),
        )
        .await,
    );
    assert_eq!(calls_made(&calls), 4);
    assert_eq!(
        refreshed["fields"]["price"],
        json!({"value": "18.13", "evidence": "text", "from": "Komt overeen met 18,13 € per Pak"})
    );
    assert_eq!(refreshed["title"], "Mijn tegel");
    assert_eq!(
        refreshed["fields"]["pack_m2"],
        json!({"value": 1.5, "evidence": "agent", "from": "edit_products"})
    );
    let prices: Vec<Value> = refreshed["prices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| json!(p.as_array().unwrap()[..4]))
        .collect();
    assert_eq!(
        prices,
        [
            json!(["18.13", "EUR", "pack", "text"]),
            json!(["17.00", "EUR", "pack", "agent"]),
            json!(["18.13", "EUR", "pack", "text"])
        ]
    );
}

/// Without an `AnySite` key the server says product import is not set up,
/// and the library still answers.
#[tokio::test]
async fn without_an_anysite_key_import_says_so() {
    let Some((base, state)) = service().await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let answer = tool(
        &base,
        &access,
        "import_product",
        json!({"url": "https://www.praxis.nl/a/10043467"}),
    )
    .await;
    assert_eq!(
        answer,
        json!({"content": [{"type": "text", "text": newera_cloud::products::NOT_CONFIGURED}], "isError": true})
    );
    let listed = fields_of(&tool(&base, &access, "products", json!({})).await);
    assert_eq!(listed["rows"], json!([]));
    let listing = mcp(
        &base,
        &access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await
    .1;
    let names: Vec<&str> = listing["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in ["import_product", "products", "edit_products", "projects"] {
        assert!(names.contains(&name), "{name}");
    }
}

/// A client that gives up on a slow import does not lose what was paid
/// for: the fetch goes on and keeps the product, and the call that comes
/// next finds it — waiting for the fetch if it is still running — without
/// a second fetch.
#[tokio::test]
async fn an_import_the_client_gave_up_on_is_kept() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../newera-shop/tests/fixtures");
    let record = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(fixtures.join(name)).unwrap()).unwrap()
    };
    let (anysite, calls) = anysite_of(
        vec![
            (
                "webparser/parse".to_owned(),
                "hornbach.nl".to_owned(),
                record("hornbach_tile_10656129.parse.json"),
            ),
            (
                "webparser/render".to_owned(),
                "hornbach.nl".to_owned(),
                record("hornbach_tile_10656129.render.json"),
            ),
        ],
        std::time::Duration::from_millis(700),
    )
    .await;
    let Some((base, state, calls)) = shop_service_on(anysite, calls).await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let link =
        "https://www.hornbach.nl/p/wandtegel-elegance-wit-mat-30-x-60-cm-gerectificeerd/10656129/";
    let args = json!({"url": link});
    let gave_up = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        tool(&base, &access, "import_product", args.clone()),
    )
    .await;
    assert!(
        gave_up.is_err(),
        "the import should still have been running"
    );
    // Asked again while the first fetch runs: the same fetch, not a second.
    let again = fields_of(&tool(&base, &access, "import_product", args.clone()).await);
    assert_eq!(
        (&again["stored"], &again["fetched"]),
        (&json!(true), &json!([]))
    );
    assert_eq!(again["missing"], json!([]));
    assert_eq!(calls_made(&calls), 2, "parse and render, once");
    let kept = fields_of(&tool(&base, &access, "products", json!({})).await);
    assert_eq!(kept["rows"].as_array().unwrap().len(), 1);
    // Asked together, two calls make one fetch.
    let link = "https://www.hornbach.nl/p/andere-tegel/10656129/?variant=2";
    let (a, b) = tokio::join!(
        tool(&base, &access, "import_product", json!({"url": link})),
        tool(&base, &access, "import_product", json!({"url": link})),
    );
    let (a, b) = (fields_of(&a), fields_of(&b));
    assert_eq!(a["id"], b["id"]);
    assert_eq!(
        [
            a["stored"].as_bool().unwrap(),
            b["stored"].as_bool().unwrap()
        ]
        .iter()
        .filter(|stored| **stored)
        .count(),
        1,
        "one call fetched, the other waited"
    );
    assert_eq!(calls_made(&calls), 4);
    assert_eq!(
        a["sku"], "10656129?variant=2",
        "the variant is another article"
    );
}

/// `AnySite` failing on the parse is an error of the import: no render is
/// bought, nothing is kept, and the answer says to call again.
#[tokio::test]
async fn an_import_anysite_failed_on_buys_nothing_more() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../newera-shop/tests/fixtures");
    let render: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures.join("hornbach_tile_10656129.render.json")).unwrap(),
    )
    .unwrap();
    let busy = json!({"status": 429, "headers": {}, "body": {"detail": "Too many requests"}});
    let (anysite, calls) = anysite_of(
        vec![
            ("webparser/parse".to_owned(), "busy.nl".to_owned(), busy),
            ("webparser/render".to_owned(), "busy.nl".to_owned(), render),
        ],
        std::time::Duration::ZERO,
    )
    .await;
    let Some((base, state, calls)) = shop_service_on(anysite, calls).await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let answer = tool(
        &base,
        &access,
        "import_product",
        json!({"url": "https://www.busy.nl/p/tegel/12345678"}),
    )
    .await;
    assert_eq!(answer["isError"], true, "{answer}");
    let text = answer["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("limiting calls") && text.contains("call again"),
        "{text}"
    );
    assert_eq!(
        calls_made(&calls),
        1,
        "no render after a parse AnySite failed"
    );
    let kept = fields_of(&tool(&base, &access, "products", json!({})).await);
    assert_eq!(kept["rows"], json!([]), "an empty page is not kept");
}

/// A refresh that reads a price replaces the whole commercial set: the
/// IKEA Family offer that ended is no longer the product's member price.
#[tokio::test]
async fn a_refresh_that_reads_a_price_drops_the_offer_the_shop_no_longer_shows() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../newera-shop/tests/fixtures");
    let family: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures.join("ikea_billy_00263850.json")).unwrap(),
    )
    .unwrap();
    let mut plain = family.clone();
    plain["body"][0]["price"] = json!(49.99);
    plain["body"][0]["price_tag"] = Value::Null;
    let (anysite, calls) = anysite_of(
        vec![
            ("ikea/products".to_owned(), "boekenkast".to_owned(), family),
            ("ikea/products".to_owned(), "nofamily".to_owned(), plain),
        ],
        std::time::Duration::ZERO,
    )
    .await;
    let Some((base, state, _)) = shop_service_on(anysite, calls).await else {
        return;
    };
    let (_, access, _) = credentials(&base, &state, "buyer@example.com").await;
    let link = "https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/";
    let first = fields_of(&tool(&base, &access, "import_product", json!({"url": link})).await);
    let id = first["id"].as_str().unwrap().to_owned();
    assert_eq!(first["fields"]["member_price"]["value"], "42.49");
    // The agent's own entry outside the commercial set, and a unit the
    // shop no longer sells in.
    let set = tool(
        &base,
        &access,
        "edit_products",
        json!({"id": id, "action": "set", "fields": {"colour": "mijn wit", "sale_unit": "pack"}}),
    )
    .await;
    fields_of(&set);
    let history = |state: &AppState| {
        let db = state.db.clone();
        let id = id.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "select count(*) from product_prices where product_id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let before = history(&state).await;
    // The shop's page, as it reads once the offer has ended.
    sqlx::query("update products set url = $2 where id = $1")
        .bind(&id)
        .bind("https://www.ikea.com/nl/nl/p/billy-nofamily-00263850/")
        .execute(&state.db)
        .await
        .unwrap();
    let refreshed = fields_of(
        &tool(
            &base,
            &access,
            "edit_products",
            json!({"id": id, "action": "refresh"}),
        )
        .await,
    );
    assert_eq!(refreshed["fields"]["price"]["value"], "49.99");
    assert!(
        refreshed["fields"].get("member_price").is_none(),
        "{}",
        refreshed["fields"]
    );
    assert_eq!(refreshed["fields"]["width_cm"]["value"], 80.0);
    assert_eq!(
        refreshed["fields"]["colour"],
        json!({"value": "mijn wit", "evidence": "agent", "from": "edit_products"})
    );
    assert_eq!(
        refreshed["fields"]["sale_unit"]["value"], "piece",
        "the unit goes with the price it qualifies"
    );
    assert_eq!(history(&state).await, before + 1, "one price seen");
}
