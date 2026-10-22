//! The editor runs in a browser tab; an AI speaks MCP over HTTP. A tab cannot
//! listen on a port, so the two cannot meet — this stands between them.
//!
//! A tab asks for a **room** and gets back two secrets: one it keeps (to hold
//! its socket) and one the person pastes into their AI client. The AI then
//! talks MCP to this service, and every tool call is handed down the socket to
//! that one tab, which does the work in the project on screen and answers.
//!
//! What this does not do is as important as what it does:
//!
//! - **It stores no project.** Messages pass through; the home, the walls, the
//!   photos live in the tab and nowhere else. Kill this service and nobody
//!   loses a drawing.
//! - **Rooms do not meet.** A room is reachable only by its own secrets, which
//!   are 128 bits of randomness each and never derived from anything. There is
//!   no listing, no enumeration, no "rooms of this user": the id is the
//!   capability, and one room's socket cannot be addressed from another.
//! - **It forgets.** A room dies with its tab (after a short grace, so a
//!   reload is not a new room) and, failing that, after going quiet.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt::Write as _;
use tokio::sync::{mpsc, oneshot};

/// How long a room outlives the tab that made it.
///
/// A refresh takes seconds, but the address is pasted into somebody's AI
/// client and losing it means setting that up again: the room waits a quarter
/// of an hour for its tab to come back. A room with nobody in it costs a few
/// hundred bytes and answers "not connected" to anyone who tries it.
const GRACE: Duration = Duration::from_mins(15);

/// A room nobody has touched for this long is gone.
const IDLE: Duration = Duration::from_mins(360);

/// Longest a tool may take before the AI is told it timed out. Renders and
/// photos are slow on purpose; an agent waiting forever is worse.
const TOOL_TIMEOUT: Duration = Duration::from_secs(180);

/// How often the tab is pinged. Cloudflare closes a WebSocket that says
/// nothing for about a hundred seconds, and a tab whose socket was closed
/// under it looks, from the outside, exactly like a tab that went away: the
/// address stops answering while somebody is still looking at the editor.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// The protocol version this speaks when a client does not name one.
const PROTOCOL: &str = "2025-06-18";

/// The largest message an AI may send. Images go in over this route (a scanned
/// plan for `set_background`), so it is generous — and finite, because a body
/// nobody bounds is a way to take the machine down with one request.
const MAX_BODY: usize = 24 * 1024 * 1024;

/// Rooms one address may open in an hour. A room costs little, but nothing
/// stops a script from asking for millions unless something does.
const ROOMS_PER_HOUR: usize = 60;

/// Rooms this process holds at once, whoever asked.
const MAX_ROOMS: usize = 5_000;

/// Calls one room may have in flight. An agent that floods its own window
/// hurts only itself, but memory here is shared by everyone.
const MAX_IN_FLIGHT: usize = 16;

/// Where the editor is served from. Anything else is refused at the browser's
/// own gate: the secrets are the real lock, this is the outer door.
const ORIGINS: [&str; 6] = [
    "https://3dneweraai.com",
    "https://www.3dneweraai.com",
    // Where the editor is served while it is being worked on and checked.
    "http://127.0.0.1:8801",
    "http://localhost:8801",
    "http://127.0.0.1:8790",
    "http://localhost:8790",
];

/// What the tab is asked to do, and what it answers.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToTab {
    /// Run a tool and answer with `id`.
    Call { id: u64, name: String, args: Value },
    /// An AI client finished the handshake — the window says so on screen.
    Client {
        name: String,
        version: Option<String>,
    },
    /// Save the cloud project now — the pending change at once, after the
    /// save already on its way — and answer with a `result` under `id`
    /// carrying `{"project", "revision"}`: the project it is
    /// bound to and the service revision that holds its latest state, or no
    /// project when it is bound to none. An editor older than this message
    /// ignores it and never answers; whoever asks waits a bounded time.
    Flush { id: u64 },
    /// Keep this file in the tab's project, at `path` relative to where its
    /// files are, and answer with a `result` under `id` carrying `{"path"}`:
    /// a shop product's model, sent by the hosted service before the call
    /// that places it. `data` is the file's bytes in standard base64. An
    /// editor older than this message ignores it and never answers; whoever
    /// sends it waits a bounded time.
    ///
    /// `replace`: the file replaces one already placed (a model attached
    /// later); the tab keeps it only when its document uses that path, and
    /// says whether it did (`{"path", "kept"}`).
    ///
    /// `tile`: the file is a shop finish's texture, drawn `[across, up]` cm:
    /// every surface whose look is that file takes that size, as its new
    /// picture may lie the other way (`{"path", "kept", "retiled"}`).
    Asset {
        id: u64,
        path: String,
        data: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        replace: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        tile: Option<[f64; 2]>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FromTab {
    /// Sent once, on connecting: the tools this window offers, and the
    /// pages (MCP Apps) some of them show in the chat, with their contents.
    Hello {
        tools: Value,
        #[serde(default)]
        resources: Value,
        /// What the server says about itself, as the tab's editor words it.
        #[serde(default)]
        instructions: Option<String>,
    },
    /// The answer to a call.
    Result {
        id: u64,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        error: Option<String>,
    },
}

/// One tab, and the calls it still owes answers for.
#[derive(Debug)]
struct Room {
    /// Secret the tab shows to take (or retake) the socket.
    tab_key: String,
    /// Secret in the URL the person pastes into their AI client.
    client_token: String,
    /// Where to send work, while a tab is connected.
    outbox: Option<mpsc::UnboundedSender<ToTab>>,
    /// The tools this tab said it has, from its `hello`.
    tools: Value,
    /// The pages this tab said it has, contents included, from its `hello`.
    resources: Value,
    /// The instructions this tab's editor gives, from its `hello`.
    instructions: Option<String>,
    /// Calls in flight, waiting on the tab.
    waiting: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
    next_call: u64,
    /// When the tab last went away, for the grace period.
    orphaned_at: Option<Instant>,
    touched: Instant,
    /// The account the person in the tab signed in with, once the tab claims
    /// the room for it: then their AI reaches it through the account, with
    /// no secret in any URL. `None` for a tab nobody signed into.
    owner: Option<String>,
}

impl Room {
    fn new(tab_key: String, client_token: String) -> Self {
        Self {
            tab_key,
            client_token,
            outbox: None,
            tools: json!([]),
            resources: json!([]),
            instructions: None,
            waiting: HashMap::new(),
            next_call: 0,
            orphaned_at: Some(Instant::now()),
            touched: Instant::now(),
            owner: None,
        }
    }

    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.touched) > IDLE
            || self
                .orphaned_at
                .is_some_and(|gone| now.duration_since(gone) > GRACE)
    }
}

/// Every room this process is holding, and who has been asking for them.
#[derive(Debug, Clone, Default)]
pub struct Rooms(Arc<Mutex<Held>>);

/// What the service keeps in memory. Nothing here outlives the process, and
/// none of it is anybody's drawing.
#[derive(Debug, Default)]
struct Held {
    rooms: HashMap<String, Room>,
    /// When each address last opened rooms, to keep one from opening all of
    /// them. Trimmed as it is read; nothing is kept for longer than an hour.
    asked: HashMap<String, Vec<Instant>>,
    /// Where else this deployment serves the editor, beside `ORIGINS`.
    origins: Vec<String>,
}

impl Rooms {
    /// No rooms yet, and the editor also served from `origins`: a deployment
    /// of its own names where its editor lives.
    #[must_use]
    pub fn allowing(origins: impl IntoIterator<Item = String>) -> Self {
        Self(Arc::new(Mutex::new(Held {
            origins: origins.into_iter().collect(),
            ..Held::default()
        })))
    }
}

/// Compares two secrets without telling anyone, through timing, how much of
/// one was right.
fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |seen, (x, y)| seen | (x ^ y)) == 0
}

/// 128 bits of randomness as hex — an id nobody guesses and nothing derives.
fn secret() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes.iter().fold(String::with_capacity(32), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

/// A room a tab is asking to keep: the one it held last time.
///
/// Every field defaults, so a body that says nothing of the sort — `{}`, or
/// somebody else's JSON — reads as "no preference" and opens a fresh room,
/// rather than being refused at the door.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Wanted {
    room: String,
    tab_key: String,
    client_token: String,
}

impl Wanted {
    /// Nothing but three of our own secrets is worth looking at.
    fn plausible(&self) -> bool {
        let hex = |s: &String| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
        hex(&self.room) && hex(&self.tab_key) && hex(&self.client_token)
    }
}

impl Rooms {
    /// Opens a room for a tab, or gives back the one it already had.
    ///
    /// An address that changes is an address somebody has to paste into their
    /// AI client again, so a tab may ask to keep its own: it presents the room
    /// and the key it was given, and gets the same room back — whether it is
    /// still here, or gone because this service restarted under it. A room
    /// that exists and does not match that key belongs to somebody else and is
    /// refused; an id nobody holds is 128 bits nobody guesses.
    ///
    /// `caller` is whoever asked, as far as the network knows — one address
    /// may not open rooms without end.
    fn open(
        &self,
        caller: &str,
        wanted: Option<Wanted>,
    ) -> Result<(String, String, String), &'static str> {
        self.sweep();
        let wanted = wanted.filter(Wanted::plausible);
        if let Some(wanted) = &wanted {
            let mut held = self.0.lock().expect("rooms");
            if let Some(room) = held.rooms.get_mut(&wanted.room) {
                if !same_secret(&room.tab_key, &wanted.tab_key) {
                    return Err("that room is not yours");
                }
                room.touched = Instant::now();
                return Ok((
                    wanted.room.clone(),
                    room.tab_key.clone(),
                    room.client_token.clone(),
                ));
            }
        }
        let (id, tab_key, client_token) = match wanted {
            Some(wanted) => (wanted.room, wanted.tab_key, wanted.client_token),
            None => (secret(), secret(), secret()),
        };
        let now = Instant::now();
        let mut held = self.0.lock().expect("rooms");
        if held.rooms.len() >= MAX_ROOMS {
            return Err("this relay is full");
        }
        let hour = Duration::from_secs(3600);
        let asked = held.asked.entry(caller.to_owned()).or_default();
        asked.retain(|at| now.duration_since(*at) < hour);
        if asked.len() >= ROOMS_PER_HOUR {
            return Err("too many rooms from here in the last hour");
        }
        asked.push(now);
        held.rooms
            .insert(id.clone(), Room::new(tab_key.clone(), client_token.clone()));
        Ok((id, tab_key, client_token))
    }

    /// Drops rooms whose tab is gone for good, rooms nobody uses, and the
    /// memory of who asked for what.
    fn sweep(&self) {
        let now = Instant::now();
        let hour = Duration::from_secs(3600);
        let mut held = self.0.lock().expect("rooms");
        held.rooms.retain(|_, room| !room.expired(now));
        held.asked.retain(|_, times| {
            times.retain(|at| now.duration_since(*at) < hour);
            !times.is_empty()
        });
    }

    /// Hands a room to an account. Only the tab that holds it can — it shows
    /// its key — and whoever calls this has already checked, by its own
    /// means, that the account is the person in that tab.
    ///
    /// An account keeps one room: a tab claiming a new one takes the account
    /// from the old, so "the tab I have open" is never ambiguous.
    ///
    /// # Errors
    ///
    /// When the room is not there or the key is not its own.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the rooms.
    pub fn claim(&self, room: &str, tab_key: &str, owner: &str) -> Result<(), &'static str> {
        let mut held = self.0.lock().expect("rooms");
        match held.rooms.get(room) {
            Some(entry) if same_secret(&entry.tab_key, tab_key) => {}
            _ => return Err("that room is not yours"),
        }
        for (id, entry) in &mut held.rooms {
            if id != room && entry.owner.as_deref() == Some(owner) {
                entry.owner = None;
            }
        }
        let entry = held.rooms.get_mut(room).expect("checked above");
        entry.owner = Some(owner.to_owned());
        entry.touched = Instant::now();
        Ok(())
    }

    /// The room an account's tab holds, while that tab is connected.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the rooms.
    pub fn owned_by(&self, owner: &str) -> Option<String> {
        self.sweep();
        let held = self.0.lock().expect("rooms");
        held.rooms
            .iter()
            .find(|(_, entry)| entry.owner.as_deref() == Some(owner) && entry.outbox.is_some())
            .map(|(id, _)| id.clone())
    }

    /// How many rooms are held right now — for `/health`, not for listing.
    fn count(&self) -> usize {
        self.sweep();
        self.0.lock().expect("rooms").rooms.len()
    }
}

/// The service: the tab's socket, the AI's MCP endpoint, and a health check.
pub fn router() -> Router {
    router_with(Rooms::default())
}

/// The same, over rooms you already hold (the tests want to look inside).
///
/// # Panics
///
/// When the rooms' lock was poisoned by a thread that panicked holding it.
pub fn router_with(rooms: Rooms) -> Router {
    // Only the editor's own pages may ask a browser to call this; everything
    // else is a script, and a script has no business opening rooms for a tab
    // that is not there. The secrets are the lock — this is the outer door.
    let extra = rooms.0.lock().expect("rooms").origins.clone();
    let origins = ORIGINS
        .iter()
        .copied()
        .chain(extra.iter().map(String::as_str))
        .filter_map(|origin| origin.parse::<axum::http::HeaderValue>().ok())
        .collect::<Vec<_>>();
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
            axum::http::HeaderName::from_static("mcp-protocol-version"),
            axum::http::HeaderName::from_static("mcp-session-id"),
        ]);
    Router::new()
        .route("/health", get(health))
        .route("/rooms", post(open_room))
        .route("/r/{room}/tab", get(tab_socket))
        .route("/r/{room}/{token}/mcp", post(mcp).get(no_stream))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .layer(cors)
        .with_state(rooms)
}

async fn health(State(rooms): State<Rooms>) -> Response {
    axum::Json(json!({"ok": true, "rooms": rooms.count()})).into_response()
}

/// A tab asking for somewhere to be reached.
#[derive(Debug, Serialize)]
struct Opened {
    room: String,
    tab_key: String,
    /// What the person pastes into their AI client — relative, because only
    /// the caller knows the name this service answers to.
    mcp_path: String,
    tab_path: String,
}

async fn open_room(
    State(rooms): State<Rooms>,
    headers: axum::http::HeaderMap,
    // Read as text and parsed here: a body that is not a room request — `{}`,
    // `null`, somebody's stray JSON — should open a room, not be turned away
    // with a complaint about its shape.
    body: String,
) -> Response {
    // This runs behind a tunnel, so the peer address is the tunnel's: the
    // forwarded header is what tells one caller from another. It is not proof
    // of anything — it only has to be steady enough to count against, and the
    // ceiling below it holds even when everyone looks the same.
    let who = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map_or_else(|| "unknown".to_owned(), str::to_owned);
    let wanted = serde_json::from_str::<Wanted>(&body).ok();
    let (room, tab_key, client_token) = match rooms.open(&who, wanted) {
        Ok(opened) => opened,
        Err(why @ "that room is not yours") => {
            return (StatusCode::FORBIDDEN, why).into_response();
        }
        Err(why) => return (StatusCode::TOO_MANY_REQUESTS, why).into_response(),
    };
    axum::Json(Opened {
        mcp_path: format!("/r/{room}/{client_token}/mcp"),
        tab_path: format!("/r/{room}/tab?key={tab_key}"),
        room,
        tab_key,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
struct TabQuery {
    key: String,
}

/// The tab takes its socket. Wrong room or wrong key looks the same from
/// outside: there is nothing to learn by trying.
async fn tab_socket(
    State(rooms): State<Rooms>,
    Path(room): Path<String>,
    axum::extract::Query(query): axum::extract::Query<TabQuery>,
    headers: axum::http::HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    {
        let mut held = rooms.0.lock().expect("rooms");
        if !from_the_editor(origin, &held.origins) {
            return (StatusCode::FORBIDDEN, "not from here").into_response();
        }
        let Some(entry) = held.rooms.get_mut(&room) else {
            return (StatusCode::NOT_FOUND, "no such room").into_response();
        };
        if !same_secret(&entry.tab_key, &query.key) {
            return (StatusCode::NOT_FOUND, "no such room").into_response();
        }
        entry.touched = Instant::now();
    }
    upgrade.on_upgrade(move |socket| hold_tab(socket, rooms, room))
}

/// Whether a socket handshake came from a page allowed to hold a tab.
///
/// A handshake is not a fetch: the browser sends it whatever CORS says, so the
/// gate has to be here. Something with no origin at all is not a browser — a
/// script, a check, somebody's own tool — and for those the key is the lock.
fn from_the_editor(origin: Option<&str>, extra: &[String]) -> bool {
    origin.is_none_or(|origin| ORIGINS.contains(&origin) || extra.iter().any(|e| e == origin))
}

/// Pumps one tab's socket: work down, answers up, until it goes away.
async fn hold_tab(socket: WebSocket, rooms: Rooms, room: String) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ToTab>();
    {
        let mut held = rooms.0.lock().expect("rooms");
        let Some(entry) = held.rooms.get_mut(&room) else {
            return;
        };
        entry.outbox = Some(tx);
        entry.orphaned_at = None;
        entry.touched = Instant::now();
    }
    let writing = tokio::spawn(async move {
        let mut beat = tokio::time::interval(KEEPALIVE);
        beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        beat.tick().await; // the first tick is immediate; the socket is new
        loop {
            tokio::select! {
                message = rx.recv() => {
                    let Some(message) = message else { break };
                    let text = serde_json::to_string(&message).unwrap_or_default();
                    if sink.send(Message::text(text)).await.is_err() {
                        break;
                    }
                }
                _ = beat.tick() => {
                    // Silence is what gets a socket closed in the middle; the
                    // browser answers this without the page knowing.
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(from) = serde_json::from_str::<FromTab>(&text) else {
            continue;
        };
        let mut held = rooms.0.lock().expect("rooms");
        let Some(entry) = held.rooms.get_mut(&room) else {
            break;
        };
        entry.touched = Instant::now();
        match from {
            FromTab::Hello {
                tools,
                resources,
                instructions,
            } => {
                entry.tools = tools;
                entry.resources = resources;
                entry.instructions = instructions;
            }
            FromTab::Result {
                id,
                ok,
                result,
                error,
            } => {
                if let Some(waiting) = entry.waiting.remove(&id) {
                    let _ = waiting.send(if ok {
                        Ok(result)
                    } else {
                        Err(error.unwrap_or_else(|| "the window did not say why".into()))
                    });
                }
            }
        }
    }

    writing.abort();
    let mut held = rooms.0.lock().expect("rooms");
    if let Some(entry) = held.rooms.get_mut(&room) {
        entry.outbox = None;
        entry.orphaned_at = Some(Instant::now());
        // Whoever was waiting is not going to be answered by this socket.
        for (_, waiting) in entry.waiting.drain() {
            let _ = waiting.send(Err("the editor's tab went away".into()));
        }
    }
}

/// MCP over HTTP: what an AI client talks to.
async fn mcp(
    State(rooms): State<Rooms>,
    Path((room, token)): Path<(String, String)>,
    body: String,
) -> Response {
    let message: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(err) => return jsonrpc_error(&Value::Null, -32700, &format!("bad JSON: {err}")),
    };
    // A batch is a list; answer each in turn, keeping only what has an id.
    let messages = match &message {
        Value::Array(list) => list.clone(),
        one => vec![one.clone()],
    };
    let mut answers = Vec::new();
    for one in messages {
        if let Some(answer) = answer(&rooms, &room, &token, &one).await {
            answers.push(answer);
        }
    }
    match answers.len() {
        // Notifications only: the protocol wants nothing back.
        0 => StatusCode::ACCEPTED.into_response(),
        1 => axum::Json(answers.remove(0)).into_response(),
        _ => axum::Json(Value::Array(answers)).into_response(),
    }
}

/// This endpoint has no server-to-client stream: the spec's own answer for
/// that is 405, and clients carry on over POST.
async fn no_stream() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

/// One JSON-RPC message in, at most one out.
async fn answer(rooms: &Rooms, room: &str, token: &str, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let notification = message.get("id").is_none();

    // Every method below needs the room, and a wrong token must not tell
    // anyone whether the room exists.
    let known = {
        let mut held = rooms.0.lock().expect("rooms");
        match held.rooms.get_mut(room) {
            Some(entry) if same_secret(&entry.client_token, token) => {
                entry.touched = Instant::now();
                true
            }
            _ => false,
        }
    };
    if !known {
        return (!notification).then(|| {
            error_body(
                &id,
                -32001,
                "this address is not open — the tab that made it is gone",
            )
        });
    }
    answer_in(rooms, room, message).await
}

/// One JSON-RPC message for a room whoever asks has already been let into —
/// by its token in the URL, or by the account that owns it.
///
/// # Panics
///
/// If another thread panicked while holding the rooms.
pub async fn answer_in(rooms: &Rooms, room: &str, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let notification = message.get("id").is_none();
    if let Some(entry) = rooms.0.lock().expect("rooms").rooms.get_mut(room) {
        entry.touched = Instant::now();
    }

    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL)
                .to_owned();
            // The window shows who just connected, by name.
            if let Some(client) = params.get("clientInfo") {
                let name = client
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP")
                    .to_owned();
                let told = ToTab::Client {
                    name,
                    version: client
                        .get("version")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                };
                let held = rooms.0.lock().expect("rooms");
                if let Some(entry) = held.rooms.get(room)
                    && let Some(outbox) = &entry.outbox
                {
                    let _ = outbox.send(told);
                }
            }
            // The tab's editor words its own; an older one sent none.
            let instructions = {
                let held = rooms.0.lock().expect("rooms");
                held.rooms
                    .get(room)
                    .and_then(|entry| entry.instructions.clone())
                    .unwrap_or_else(|| INSTRUCTIONS.to_owned())
            };
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": version,
                    "capabilities": { "tools": {}, "resources": {} },
                    "serverInfo": {
                        "name": "3d-new-era-ai",
                        "title": "3D New Era AI (browser)",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": instructions,
                }
            }))
        }
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => {
            let tools = {
                let held = rooms.0.lock().expect("rooms");
                held.rooms.get(room).map(|entry| entry.tools.clone())
            };
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": tools.unwrap_or_else(|| json!([])) }
            }))
        }
        "resources/list" | "resources/read" => {
            let resources = {
                let held = rooms.0.lock().expect("rooms");
                held.rooms
                    .get(room)
                    .map(|entry| entry.resources.clone())
                    .unwrap_or_default()
            };
            let resources = resources.as_array().cloned().unwrap_or_default();
            if method == "resources/list" {
                let listed: Vec<Value> = resources
                    .into_iter()
                    .map(|mut r| {
                        if let Some(map) = r.as_object_mut() {
                            map.remove("text");
                            map.remove("blob");
                        }
                        r
                    })
                    .collect();
                return Some(json!({"jsonrpc": "2.0", "id": id, "result": {"resources": listed}}));
            }
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(match resources.into_iter().find(|r| r["uri"] == uri) {
                Some(found) => json!({"jsonrpc": "2.0", "id": id, "result": {"contents": [found]}}),
                None => error_body(
                    &id,
                    -32002,
                    &format!("no resource {uri}; resources/list names the ones there are"),
                ),
            })
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match hand_to_tab(rooms, room, &name, args).await {
                Ok(result) => Some(json!({"jsonrpc": "2.0", "id": id, "result": result})),
                // A tool that refuses is not a protocol error: the agent is
                // told inside the result, which is what lets it try again.
                Err(why) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": why}],
                        "isError": true,
                    }
                })),
            }
        }
        _ if notification => None,
        other => Some(error_body(
            &id,
            -32601,
            &format!("this server does not do {other}"),
        )),
    }
}

/// What the browser's MCP tells an agent about itself when the tab sent no
/// words of its own (an editor older than the relay). A tab's `hello` carries
/// `newera_mcp::surface::instructions`, which name every tool it offers.
const INSTRUCTIONS: &str = "\
Home design editor running in someone's browser tab, reached through a relay. \
Units: cm. Plan axes: x right, y down. Everything you change appears on their \
screen as you do it and can be undone with Ctrl+Z. The project lives in that \
tab: it is not saved anywhere here, and closing the tab ends the session.";

/// Asks an account's tab to save its cloud project now, and waits up to
/// `within` for the answer (`ToTab::Flush`): `{"project": id or null,
/// "revision": n}` as the tab says it. `Err` when the tab is not connected,
/// refused, went away, or did not answer in time — an editor older than this
/// message never does.
///
/// # Errors
///
/// When the tab gave no answer to act on, and why.
///
/// # Panics
///
/// If another thread panicked while holding the rooms.
pub async fn flush_in(rooms: &Rooms, room: &str, within: Duration) -> Result<Value, String> {
    ask_tab(rooms, room, None, within, |id| ToTab::Flush { id }).await
}

/// Hands the account's tab a file to keep in its project before a call
/// that uses it (`ToTab::Asset`), and waits up to `within` for the tab to
/// say it has it: `{"path"}` as the tab kept it.
///
/// # Errors
///
/// When the tab gave no answer to act on, and why.
///
/// # Panics
///
/// If another thread panicked while holding the rooms.
pub async fn stage_in(
    rooms: &Rooms,
    room: &str,
    path: &str,
    bytes: &[u8],
    within: Duration,
) -> Result<Value, String> {
    send_file(rooms, room, path, bytes, within, false, None).await
}

/// Hands the account's tab a file that replaces one already placed — kept
/// only when its document uses `path` — and waits up to `within` for its
/// answer: `{"path", "kept"}`. With `tile`, a finish's texture: the
/// surfaces drawn with it take that size.
///
/// # Errors
///
/// When the tab gave no answer to act on, and why.
///
/// # Panics
///
/// If another thread panicked while holding the rooms.
pub async fn replace_in(
    rooms: &Rooms,
    room: &str,
    path: &str,
    bytes: &[u8],
    tile: Option<[f64; 2]>,
    within: Duration,
) -> Result<Value, String> {
    send_file(rooms, room, path, bytes, within, true, tile).await
}

async fn send_file(
    rooms: &Rooms,
    room: &str,
    path: &str,
    bytes: &[u8],
    within: Duration,
    replace: bool,
    tile: Option<[f64; 2]>,
) -> Result<Value, String> {
    use base64::Engine as _;
    let (path, data) = (
        path.to_owned(),
        base64::engine::general_purpose::STANDARD.encode(bytes),
    );
    ask_tab(rooms, room, None, within, move |id| ToTab::Asset {
        id,
        path,
        data,
        replace,
        tile,
    })
    .await
}

/// Sends one call down to the tab and waits for its answer.
async fn hand_to_tab(rooms: &Rooms, room: &str, name: &str, args: Value) -> Result<Value, String> {
    let call = name.to_owned();
    ask_tab(rooms, room, Some(name), TOOL_TIMEOUT, move |id| {
        ToTab::Call {
            id,
            name: call,
            args,
        }
    })
    .await
}

/// Sends the tab one request that it answers with a `result` under the id
/// given to `request`, and waits up to `within` for it. `tool` is the tool a
/// call runs, checked against what the tab offers. A request not answered in
/// time stops waiting for good: it holds no slot of the room's in-flight
/// calls after that.
async fn ask_tab(
    rooms: &Rooms,
    room: &str,
    tool: Option<&str>,
    within: Duration,
    request: impl FnOnce(u64) -> ToTab,
) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel();
    let id = {
        let mut held = rooms.0.lock().expect("rooms");
        let entry = held
            .rooms
            .get_mut(room)
            .ok_or_else(|| "this address is not open any more".to_owned())?;
        if entry.waiting.len() >= MAX_IN_FLIGHT {
            return Err(format!(
                "this window already has {MAX_IN_FLIGHT} calls in the air — wait for one to answer"
            ));
        }
        let Some(outbox) = entry.outbox.clone() else {
            return Err(
                "the editor's tab is not connected — open the project again in the browser"
                    .to_owned(),
            );
        };
        // A tab keeps the editor it loaded: one older than the service that
        // listed a tool does not have it, and says so better than `unknown`.
        if let Some(name) = tool
            && let Some(offered) = entry.tools.as_array()
            && !offered.is_empty()
            && !offered.iter().any(|tool| tool["name"] == name)
        {
            return Err(format!(
                "the editor open in the browser has no tool `{name}`: it is older than this service — reload the tab and call again"
            ));
        }
        entry.next_call += 1;
        let id = entry.next_call;
        entry.waiting.insert(id, tx);
        if outbox.send(request(id)).is_err() {
            entry.waiting.remove(&id);
            return Err("the editor's tab went away".to_owned());
        }
        id
    };
    match tokio::time::timeout(within, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("the editor's tab went away".to_owned()),
        Err(_) => {
            if let Some(entry) = rooms.0.lock().expect("rooms").rooms.get_mut(room) {
                entry.waiting.remove(&id);
            }
            Err(format!(
                "the window took longer than {} s to answer",
                within.as_secs_f64()
            ))
        }
    }
}

fn error_body(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn jsonrpc_error(id: &Value, code: i32, message: &str) -> Response {
    axum::Json(error_body(id, code, message)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_is_two_secrets_and_neither_is_the_other() {
        let rooms = Rooms::default();
        let (id, tab_key, client_token) = rooms.open("test", None).expect("a room");
        assert_eq!(id.len(), 32);
        assert_ne!(id, tab_key);
        assert_ne!(tab_key, client_token);
        assert_eq!(rooms.count(), 1);
    }

    /// A room only answers to its own token — and a wrong one learns nothing
    /// about whether the room is there.
    #[tokio::test]
    async fn a_wrong_token_is_turned_away() {
        let rooms = Rooms::default();
        let (id, _tab, token) = rooms.open("test", None).expect("a room");
        let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let good = answer(&rooms, &id, &token, &list).await.expect("an answer");
        assert!(good.get("result").is_some());

        let bad = answer(&rooms, &id, "0".repeat(32).as_str(), &list)
            .await
            .expect("an answer");
        assert_eq!(bad["error"]["code"], -32001);

        let nowhere = answer(&rooms, &"f".repeat(32), &token, &list)
            .await
            .expect("an answer");
        assert_eq!(
            nowhere["error"]["message"], bad["error"]["message"],
            "a room that is not there and a wrong token must read the same"
        );
    }

    /// With nobody on the other end, a call fails in words an agent can act
    /// on, and not by hanging.
    #[tokio::test]
    async fn a_call_without_a_tab_says_so() {
        let rooms = Rooms::default();
        let (id, _tab, token) = rooms.open("test", None).expect("a room");
        let call = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": "home", "arguments": {}}
        });
        let answer = answer(&rooms, &id, &token, &call).await.expect("an answer");
        assert_eq!(answer["result"]["isError"], true);
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not connected"), "{text}");
    }

    /// A tab running an editor older than the service that listed a tool
    /// says to reload it, instead of an unknown tool.
    #[tokio::test]
    async fn an_older_tab_asks_to_be_reloaded() {
        let rooms = Rooms::default();
        let (id, _tab, token) = rooms.open("test", None).expect("a room");
        let (outbox, _inbox) = mpsc::unbounded_channel();
        {
            let mut held = rooms.0.lock().expect("rooms");
            let entry = held.rooms.get_mut(&id).expect("the room");
            entry.outbox = Some(outbox);
            entry.tools = json!([{"name": "home"}]);
        }
        let call = json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": {"name": "file", "arguments": {"action": "save"}}
        });
        let answer = answer(&rooms, &id, &token, &call).await.expect("an answer");
        assert_eq!(answer["result"]["isError"], true);
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("reload the tab"), "{text}");
    }

    /// The handshake answers like a server, and says what it is.
    #[tokio::test]
    async fn it_shakes_hands() {
        let rooms = Rooms::default();
        let (id, _tab, token) = rooms.open("test", None).expect("a room");
        let hello = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "claude-code", "version": "2.0.0"}
            }
        });
        let answer = answer(&rooms, &id, &token, &hello)
            .await
            .expect("an answer");
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(answer["result"]["serverInfo"]["name"], "3d-new-era-ai");
        assert!(answer["result"]["capabilities"]["tools"].is_object());
    }

    /// The pages a tab offers are listed without their contents and read
    /// whole; a page nobody offered is an error that says where to look.
    #[tokio::test]
    async fn a_tab_serves_its_pages() {
        let rooms = Rooms::default();
        let (id, _tab, token) = rooms.open("test", None).expect("a room");
        {
            let mut held = rooms.0.lock().unwrap();
            held.rooms.get_mut(&id).unwrap().resources = json!([
                {"uri": "ui://newera/plan-viewer.html", "mimeType": "text/html;profile=mcp-app", "text": "<html></html>"}
            ]);
        }
        let list = json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"});
        let listed = answer(&rooms, &id, &token, &list).await.unwrap();
        let first = &listed["result"]["resources"][0];
        assert_eq!(first["uri"], "ui://newera/plan-viewer.html");
        assert!(first.get("text").is_none(), "a listing carries no contents");

        let read = json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read",
            "params": {"uri": "ui://newera/plan-viewer.html"}});
        let page = answer(&rooms, &id, &token, &read).await.unwrap();
        assert_eq!(page["result"]["contents"][0]["text"], "<html></html>");

        let missing = json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read",
            "params": {"uri": "ui://elsewhere"}});
        let none = answer(&rooms, &id, &token, &missing).await.unwrap();
        assert_eq!(none["error"]["code"], -32002);

        // And a wrong token reads nothing, as for tools.
        let other = answer(&rooms, &id, &"0".repeat(32), &read).await.unwrap();
        assert_eq!(other["error"]["code"], -32001);
    }

    /// A tab hands its room to the account signed in there; only the tab
    /// that holds the key can, an account keeps one room, and the account
    /// reaches the room only while the tab is connected.
    #[tokio::test]
    async fn an_account_reaches_the_tab_it_claimed() {
        let rooms = Rooms::default();
        let (first, first_key, _) = rooms.open("test", None).expect("a room");
        let (second, second_key, _) = rooms.open("test", None).expect("another");
        assert!(
            rooms.claim(&first, &second_key, "acct-1").is_err(),
            "another tab's key"
        );
        rooms
            .claim(&first, &first_key, "acct-1")
            .expect("its own key");
        assert_eq!(rooms.owned_by("acct-1"), None, "no tab connected yet");

        let (tx, _rx) = mpsc::unbounded_channel();
        rooms
            .0
            .lock()
            .unwrap()
            .rooms
            .get_mut(&first)
            .unwrap()
            .outbox = Some(tx.clone());
        assert_eq!(rooms.owned_by("acct-1").as_deref(), Some(first.as_str()));
        assert_eq!(rooms.owned_by("acct-2"), None);

        // The same person opens another tab: the account follows it.
        rooms
            .0
            .lock()
            .unwrap()
            .rooms
            .get_mut(&second)
            .unwrap()
            .outbox = Some(tx);
        rooms.claim(&second, &second_key, "acct-1").unwrap();
        assert_eq!(rooms.owned_by("acct-1").as_deref(), Some(second.as_str()));

        // And the account's AI is answered without any token.
        let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        let answer = answer_in(&rooms, &second, &hello).await.expect("an answer");
        assert_eq!(answer["result"]["serverInfo"]["name"], "3d-new-era-ai");
    }

    /// One address cannot take the whole relay: after the hour's worth of
    /// rooms, it is told no — and somebody else is not.
    #[test]
    fn one_caller_cannot_open_every_room() {
        let rooms = Rooms::default();
        for _ in 0..ROOMS_PER_HOUR {
            rooms
                .open("203.0.113.7", None)
                .expect("within the hour's share");
        }
        assert!(
            rooms.open("203.0.113.7", None).is_err(),
            "the share runs out"
        );
        rooms
            .open("198.51.100.4", None)
            .expect("somebody else is not affected");
    }

    /// An address must survive: the same tab asks for the room it held and
    /// gets it back — even when this service restarted and has never heard of
    /// it. A room that belongs to somebody else is refused.
    #[test]
    fn a_tab_keeps_its_own_address() {
        let rooms = Rooms::default();
        let (id, key, token) = rooms.open("test", None).expect("a room");

        let again = rooms
            .open(
                "test",
                Some(Wanted {
                    room: id.clone(),
                    tab_key: key.clone(),
                    client_token: token.clone(),
                }),
            )
            .expect("the same room, still held");
        assert_eq!(again, (id.clone(), key.clone(), token.clone()));

        // As if the service had restarted: nothing in memory, the tab asks for
        // what it had, and the address it pasted into an AI client still works.
        let empty = Rooms::default();
        let revived = empty
            .open(
                "test",
                Some(Wanted {
                    room: id.clone(),
                    tab_key: key.clone(),
                    client_token: token.clone(),
                }),
            )
            .expect("claimed again");
        assert_eq!(revived, (id.clone(), key.clone(), token.clone()));

        // Somebody else's key opens nothing.
        let stolen = empty.open(
            "test",
            Some(Wanted {
                room: id.clone(),
                tab_key: "0".repeat(32),
                client_token: "1".repeat(32),
            }),
        );
        assert_eq!(stolen.unwrap_err(), "that room is not yours");

        // And a made-up shape is ignored rather than trusted: a fresh room.
        let (other, ..) = empty
            .open(
                "test",
                Some(Wanted {
                    room: "nope".into(),
                    tab_key: "nope".into(),
                    client_token: "nope".into(),
                }),
            )
            .expect("a room all the same");
        assert_ne!(other, "nope");
    }

    /// Secrets are compared in a way that does not leak how much was right.
    #[test]
    fn secrets_are_compared_whole() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
        assert!(!same_secret("", "a"));
    }

    /// The socket is not open to any page that asks — CORS does not apply to a
    /// WebSocket handshake, so the origin is checked where it arrives.
    #[test]
    fn a_socket_from_somewhere_else_is_refused() {
        assert!(from_the_editor(Some("https://3dneweraai.com"), &[]));
        assert!(from_the_editor(Some("http://127.0.0.1:8801"), &[]));
        assert!(!from_the_editor(Some("https://evil.example"), &[]));
        assert!(
            !from_the_editor(Some("https://3dneweraai.com.evil.example"), &[]),
            "a name that merely starts the same is somewhere else"
        );
        assert!(
            from_the_editor(None, &[]),
            "not a browser: the key is the lock there"
        );
        let own = ["https://editor.example".to_owned()];
        assert!(
            from_the_editor(Some("https://editor.example"), &own),
            "a deployment's own editor is let in"
        );
        assert!(
            !from_the_editor(Some("https://editor.example.evil.example"), &own),
            "and only that origin, not a name that starts the same"
        );
    }

    /// A room with a tab on the end of `outbox`, as a connected socket has it.
    fn connected(rooms: &Rooms) -> (String, mpsc::UnboundedReceiver<ToTab>) {
        let (id, _tab, _token) = rooms.open("test", None).expect("a room");
        let (outbox, inbox) = mpsc::unbounded_channel();
        rooms.0.lock().unwrap().rooms.get_mut(&id).unwrap().outbox = Some(outbox);
        (id, inbox)
    }

    fn in_flight(rooms: &Rooms, room: &str) -> usize {
        rooms.0.lock().unwrap().rooms[room].waiting.len()
    }

    /// The tab is asked to flush in its own message, and its answer is what
    /// the asker gets back.
    #[tokio::test]
    async fn a_flush_is_asked_and_answered() {
        let rooms = Rooms::default();
        let (room, mut inbox) = connected(&rooms);
        let asking = tokio::spawn({
            let (rooms, room) = (rooms.clone(), room.clone());
            async move { flush_in(&rooms, &room, Duration::from_secs(5)).await }
        });
        let asked = inbox.recv().await.expect("the request");
        assert_eq!(
            serde_json::to_value(&asked).unwrap(),
            json!({"type": "flush", "id": 1})
        );
        let answer = rooms
            .0
            .lock()
            .unwrap()
            .rooms
            .get_mut(&room)
            .unwrap()
            .waiting
            .remove(&1)
            .expect("waiting under its id");
        answer
            .send(Ok(json!({"project": "p1", "revision": 4})))
            .unwrap();
        assert_eq!(
            asking.await.unwrap(),
            Ok(json!({"project": "p1", "revision": 4}))
        );
    }

    /// A file for the tab goes as its own message, its bytes in base64, and
    /// the tab's answer is what the sender gets back; a tab that does not
    /// answer frees its slot as a flush does.
    #[tokio::test]
    async fn a_file_is_handed_to_the_tab_and_acknowledged() {
        let rooms = Rooms::default();
        let (room, mut inbox) = connected(&rooms);
        let sending = tokio::spawn({
            let (rooms, room) = (rooms.clone(), room.clone());
            async move {
                stage_in(
                    &rooms,
                    &room,
                    "products/p1/model.glb",
                    b"glTF\x02\x00",
                    Duration::from_secs(5),
                )
                .await
            }
        });
        let sent = inbox.recv().await.expect("the file");
        assert_eq!(
            serde_json::to_value(&sent).unwrap(),
            json!({"type": "asset", "id": 1, "path": "products/p1/model.glb", "data": "Z2xURgIA"})
        );
        let answer = rooms
            .0
            .lock()
            .unwrap()
            .rooms
            .get_mut(&room)
            .unwrap()
            .waiting
            .remove(&1)
            .expect("waiting under its id");
        answer
            .send(Ok(json!({"path": "products/p1/model.glb"})))
            .unwrap();
        assert_eq!(
            sending.await.unwrap(),
            Ok(json!({"path": "products/p1/model.glb"}))
        );
        // A replacement says so, for the tab to keep only where it is used.
        let replacing = tokio::spawn({
            let (rooms, room) = (rooms.clone(), room.clone());
            async move {
                replace_in(
                    &rooms,
                    &room,
                    "products/p1/model.glb",
                    b"glTF",
                    None,
                    Duration::from_millis(50),
                )
                .await
            }
        });
        assert_eq!(
            serde_json::to_value(inbox.recv().await.expect("the replacement")).unwrap(),
            json!({"type": "asset", "id": 2, "path": "products/p1/model.glb", "data": "Z2xURg==", "replace": true})
        );
        let _ = replacing.await;
        // A finish's texture says the size its surfaces are drawn at.
        let retiling = tokio::spawn({
            let (rooms, room) = (rooms.clone(), room.clone());
            async move {
                replace_in(
                    &rooms,
                    &room,
                    "products/p1/finish.jpg",
                    b"\xff\xd8",
                    Some([30.0, 60.0]),
                    Duration::from_millis(50),
                )
                .await
            }
        });
        assert_eq!(
            serde_json::to_value(inbox.recv().await.expect("the texture")).unwrap(),
            json!({"type": "asset", "id": 3, "path": "products/p1/finish.jpg", "data": "/9g=",
                   "replace": true, "tile": [30.0, 60.0]})
        );
        let _ = retiling.await;
        let unanswered = stage_in(&rooms, &room, "x.glb", b"", Duration::from_millis(10)).await;
        assert_eq!(
            unanswered,
            Err("the window took longer than 0.01 s to answer".to_owned())
        );
        assert_eq!(in_flight(&rooms, &room), 0);
    }

    /// A tab that never answers — an editor older than the message — costs
    /// the asker the bounded wait and nothing more: no slot stays taken, so
    /// later calls are not refused as too many in the air.
    #[tokio::test]
    async fn an_unanswered_flush_times_out_and_frees_its_slot() {
        let rooms = Rooms::default();
        let (room, _inbox) = connected(&rooms);
        for _ in 0..=MAX_IN_FLIGHT {
            let answer = flush_in(&rooms, &room, Duration::from_millis(10)).await;
            assert_eq!(
                answer,
                Err("the window took longer than 0.01 s to answer".to_owned())
            );
            assert_eq!(in_flight(&rooms, &room), 0);
        }
    }

    /// Without a connected tab, a flush fails at once.
    #[tokio::test]
    async fn a_flush_without_a_tab_fails_at_once() {
        let rooms = Rooms::default();
        let (room, _tab, _token) = rooms.open("test", None).expect("a room");
        let answer = flush_in(&rooms, &room, Duration::from_secs(60)).await;
        assert_eq!(
            answer,
            Err(
                "the editor's tab is not connected — open the project again in the browser"
                    .to_owned()
            )
        );
        assert_eq!(in_flight(&rooms, &room), 0);
    }

    /// A notification is not answered at all, which is what the protocol says.
    #[tokio::test]
    async fn a_notification_gets_no_answer() {
        let rooms = Rooms::default();
        let (id, _tab, token) = rooms.open("test", None).expect("a room");
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(answer(&rooms, &id, &token, &note).await.is_none());
    }
}
