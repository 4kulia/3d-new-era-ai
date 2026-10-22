//! Crash reports and usage notes, sent to Sentry.
//!
//! On by default, off with one switch that is kept between runs and honoured
//! at once — no restart. `NEWERA_TELEMETRY=0` turns it off for a run, for
//! machines that cannot keep a setting.
//!
//! The DSN is baked in at build time from `NEWERA_SENTRY_DSN` (a CI secret;
//! locally, a git-ignored `.env.local` the Makefile reads). A DSN inside a
//! shipped binary can be read out of it, as with every client-side Sentry key:
//! it lets its holder *send* events, never read them, and Sentry's rate limits
//! and inbound filters bound what that costs.
//!
//! A build made without one — anybody who compiles the app themselves, and the
//! installers when there is no release for that machine — reports through
//! `3dneweraai.com/ping/sentry`, the project's own endpoint, which holds the
//! DSN (see `tunnel.rs`). So a crash on a machine that built its own copy is
//! seen too. A build to work on and CI stay out of it either way.
//!
//! What is sent: panics with their backtrace, `error!` logs as events, and the
//! `warn!`/`info!` before them as breadcrumbs; the OS and app version. What is
//! not: IP addresses and hostnames (`send_default_pii` off), the MCP token,
//! and the plan itself.
//!
//! [`analytics`] rides the same switch and counts one thing: that the app was
//! opened, and on what. It goes through the same endpoint, for the same
//! reason.

#[cfg(not(target_arch = "wasm32"))]
pub mod analytics;
#[cfg(not(target_arch = "wasm32"))]
mod tunnel;

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether reports go out, read once from the settings file and then kept in
/// memory so a switch in the menu applies to the very next event.
static ENABLED: AtomicBool = AtomicBool::new(true);
static LOADED: std::sync::Once = std::sync::Once::new();

/// A settings directory set by a test, so it never flips the switch of the
/// machine it runs on.
static TEST_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Keeps settings and notes in `dir` for the rest of this process. For tests;
/// call it before anything reads the switch.
#[doc(hidden)]
pub fn use_config_dir(dir: PathBuf) {
    if let Ok(mut slot) = TEST_DIR.lock() {
        *slot = Some(dir);
    }
}

/// Where this app keeps its settings.
fn config_dir() -> PathBuf {
    if let Some(dir) = TEST_DIR.lock().ok().and_then(|d| d.clone()) {
        return dir;
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                let home = PathBuf::from(home);
                if cfg!(target_os = "macos") {
                    home.join("Library").join("Application Support")
                } else {
                    home.join(".config")
                }
            })
        })
        .unwrap_or_else(fallback_dir);
    base.join("3d-new-era-ai")
}

/// Where settings go when no home is known. A browser tab has no temporary
/// directory (asking std for it aborts the editor), so there it is the empty
/// path, relative and never written.
fn fallback_dir() -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    {
        PathBuf::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::temp_dir()
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Settings {
    /// `None` until someone chooses: the default is on.
    #[serde(default)]
    telemetry: Option<bool>,
}

fn settings_path() -> PathBuf {
    config_dir().join("telemetry.json")
}

fn off_by_environment() -> bool {
    std::env::var("NEWERA_TELEMETRY").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        )
    })
}

fn load() {
    LOADED.call_once(|| {
        let chosen = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .and_then(|s| s.telemetry);
        ENABLED.store(
            chosen.unwrap_or(true) && !off_by_environment(),
            Ordering::Relaxed,
        );
    });
}

/// Whether telemetry is on: the person's choice, on when never made.
pub fn enabled() -> bool {
    load();
    ENABLED.load(Ordering::Relaxed)
}

/// Turns telemetry on or off, now and for the next runs.
///
/// # Errors
/// When the choice cannot be written; it still applies to this run.
pub fn set_enabled(on: bool) -> std::io::Result<()> {
    load();
    ENABLED.store(on, Ordering::Relaxed);
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(&Settings {
        telemetry: Some(on),
    })
    .map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

/// The DSN this build reports to, if it has one.
fn dsn() -> Option<String> {
    let given = |d: &String| !d.trim().is_empty();
    std::env::var("NEWERA_SENTRY_DSN")
        .ok()
        .filter(given)
        .or_else(|| option_env!("NEWERA_SENTRY_DSN").map(str::to_owned))
        .filter(given)
}

/// Where a build without a DSN of its own sends its crash reports: the
/// project's endpoint, which holds the DSN and signs the envelope.
#[cfg(not(target_arch = "wasm32"))]
const TUNNEL: &str = "https://3dneweraai.com/ping/sentry";

/// A DSN standing in for the one such a build does not have. Sentry's client
/// needs one to start at all, and the endpoint replaces it with the real one;
/// nothing is ever sent to this host.
#[cfg(not(target_arch = "wasm32"))]
const TUNNEL_DSN: &str = "https://tunnel@3dneweraai.com/0";

/// Whether a build made without the project's keys may report through the
/// endpoint that holds them.
///
/// Yes for a release built from source — somebody installed the app and their
/// crashes are worth seeing. No for a build to work on and no in CI: those
/// crashes are already in front of whoever caused them, and counting them
/// would drown the number this is for.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn a_keyless_build_may_report() -> bool {
    !cfg!(debug_assertions) && !in_ci()
}

/// Whether this is a machine running the project's own checks rather than
/// somebody's copy of the app.
#[cfg(not(target_arch = "wasm32"))]
fn in_ci() -> bool {
    ["CI", "GITHUB_ACTIONS"].into_iter().any(|name| {
        std::env::var(name).is_ok_and(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && value != "0" && value != "false"
        })
    })
}

/// The endpoint a build without a DSN reports through, when it may.
/// `NEWERA_SENTRY_TUNNEL` points it somewhere else, for working on that
/// endpoint, and asking for it is reason enough to report from a build that
/// would otherwise stay quiet.
#[cfg(not(target_arch = "wasm32"))]
fn tunnel_url() -> Option<String> {
    if let Some(url) = std::env::var("NEWERA_SENTRY_TUNNEL")
        .ok()
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
    {
        return Some(url);
    }
    a_keyless_build_may_report().then(|| TUNNEL.to_owned())
}

/// Whether this build can report at all: it was built with a DSN, or it may
/// use the endpoint that has one.
#[cfg(not(target_arch = "wasm32"))]
pub fn available() -> bool {
    dsn().is_some() || tunnel_url().is_some()
}

/// Whether this build can report at all (it was built with a DSN).
#[cfg(target_arch = "wasm32")]
pub fn available() -> bool {
    dsn().is_some()
}

/// Keeps the reporter alive: dropping it flushes what is pending.
pub struct Guard {
    #[cfg(not(target_arch = "wasm32"))]
    _client: Option<sentry::ClientInitGuard>,
}

impl std::fmt::Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard").finish_non_exhaustive()
    }
}

/// Starts reporting, when this build has a DSN. Call first thing in `main`
/// and keep the guard until the program ends. `mode` names how the program
/// runs (`gui`, `serve`, `mcp`) so reports can be told apart.
#[cfg(not(target_arch = "wasm32"))]
pub fn init(mode: &str) -> Guard {
    load();
    // A build with a DSN posts straight to Sentry, as it always has; one
    // without goes through the endpoint that holds ours.
    let (dsn, through) = match dsn() {
        Some(dsn) => (dsn, None),
        None => match tunnel_url() {
            Some(url) => (TUNNEL_DSN.to_owned(), Some(url)),
            None => return Guard { _client: None },
        },
    };
    let secret = std::env::var("NEWERA_TOKEN").ok().filter(|t| !t.is_empty());
    let mut options = sentry::ClientOptions::default();
    options.release = Some(concat!("newera@", env!("CARGO_PKG_VERSION")).into());
    options.environment = Some(
        if cfg!(debug_assertions) {
            "development"
        } else {
            "production"
        }
        .into(),
    );
    // No IP addresses, no hostnames: nobody's machine is named.
    options.send_default_pii = false;
    options.server_name = None;
    options.attach_stacktrace = true;
    options.before_send = Some(std::sync::Arc::new(move |mut event| {
        if !ENABLED.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(secret) = &secret {
            scrub(&mut event, secret);
        }
        Some(event)
    }));
    options.before_breadcrumb = Some(std::sync::Arc::new(|crumb| {
        ENABLED.load(Ordering::Relaxed).then_some(crumb)
    }));
    if let Some(url) = through {
        options.transport = Some(std::sync::Arc::new(tunnel::Factory { url }));
    }
    let client = sentry::init((dsn, options));
    sentry::configure_scope(|scope| scope.set_tag("mode", mode));
    Guard {
        _client: Some(client),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn init(_mode: &str) -> Guard {
    Guard {}
}

/// Sends one event that says it is a test, and returns its id.
#[cfg(not(target_arch = "wasm32"))]
pub fn test_event() -> String {
    let id = sentry::capture_message("telemetry test", sentry::Level::Info);
    if let Some(client) = sentry::Hub::current().client() {
        client.flush(Some(std::time::Duration::from_secs(10)));
    }
    id.simple().to_string()
}

#[cfg(target_arch = "wasm32")]
pub fn test_event() -> String {
    String::new()
}

/// Takes the MCP token out of anything an event says.
#[cfg(not(target_arch = "wasm32"))]
fn scrub(event: &mut sentry::protocol::Event<'static>, secret: &str) {
    let clean = |text: &mut String| {
        if text.contains(secret) {
            *text = text.replace(secret, "[token]");
        }
    };
    if let Some(message) = event.message.as_mut() {
        clean(message);
    }
    if let Some(entry) = event.logentry.as_mut() {
        clean(&mut entry.message);
    }
    for exception in &mut event.exception.values {
        if let Some(value) = exception.value.as_mut() {
            clean(value);
        }
    }
    for crumb in &mut event.breadcrumbs.values {
        if let Some(message) = crumb.message.as_mut() {
            clean(message);
        }
    }
}

/// A `tracing` layer that turns `error!` into reports and earlier `warn!` and
/// `info!` into their breadcrumbs. Inert while there is no reporter.
#[cfg(not(target_arch = "wasm32"))]
pub fn layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    sentry::integrations::tracing::layer().event_filter(|meta| filter(*meta.level(), meta.target()))
}

/// Where a log line goes: `error!` is a report, `warn!` and `info!` are
/// breadcrumbs. The clipboard logs as an error a paste that finds nothing it
/// can read (an empty clipboard, an image where text was asked), which is the
/// user's clipboard and not a fault, so it stays a breadcrumb.
#[cfg(not(target_arch = "wasm32"))]
fn filter(level: tracing::Level, target: &str) -> sentry::integrations::tracing::EventFilter {
    use sentry::integrations::tracing::EventFilter;
    match level {
        tracing::Level::ERROR if target.starts_with("egui_winit::clipboard") => {
            EventFilter::Breadcrumb
        }
        tracing::Level::ERROR => EventFilter::Event,
        tracing::Level::WARN | tracing::Level::INFO => EventFilter::Breadcrumb,
        _ => EventFilter::Ignore,
    }
}

/// A note about what could work better, from whoever is using the program —
/// an agent through MCP, typically.
///
/// Shaped like a written report of a friction, because a one-line complaint
/// gets fixed in a way that breaks something else: the fix needs the case
/// that produced it, the literal answer, what was true, and what already
/// works and has to stay working.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    /// `friction` (it worked, at a cost), `bug` (it answered wrong), `idea`.
    pub kind: String,
    /// The tool it is about, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// What the task was, and the situation in the plan.
    pub goal: String,
    /// The call made, with its arguments.
    pub tried: String,
    /// What came back, literally.
    pub got: String,
    /// What was true instead, and how that was found out.
    #[serde(default)]
    pub expected: String,
    /// What it cost: the detour, the extra calls, the wrong conclusion.
    #[serde(default)]
    pub cost: String,
    /// The change that would have shortened the way.
    pub would_help: String,
    /// What works today and must keep working with that change.
    #[serde(default)]
    pub must_keep: String,
}

impl Note {
    /// The report as text, one titled paragraph per part.
    pub fn text(&self) -> String {
        [
            ("Objetivo", &self.goal),
            ("Chamada", &self.tried),
            ("Resposta", &self.got),
            ("O que era verdade", &self.expected),
            ("Custo", &self.cost),
            ("Encurtaria", &self.would_help),
            ("Não pode piorar", &self.must_keep),
        ]
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| format!("{k}: {}", v.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
    }
}

/// Where a note ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Kept on this machine and sent to the project.
    Sent,
    /// Kept on this machine only: telemetry is off, or this build cannot send.
    Kept,
}

/// The file every note is appended to, sent or not.
pub fn notes_path() -> PathBuf {
    config_dir().join("notes.jsonl")
}

/// Keeps a note on this machine and, with telemetry on, sends it.
///
/// # Errors
/// When the note cannot be written locally; sending is best effort.
pub fn note(note: &Note) -> std::io::Result<Delivery> {
    let path = notes_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(note).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")?;
    if !enabled() {
        return Ok(Delivery::Kept);
    }
    Ok(send(note))
}

#[cfg(not(target_arch = "wasm32"))]
fn send(note: &Note) -> Delivery {
    let Some(client) = sentry::Hub::current().client() else {
        return Delivery::Kept;
    };
    if !client.is_enabled() {
        return Delivery::Kept;
    }
    let first_line = note.would_help.lines().next().unwrap_or_default();
    let title: String = first_line.chars().take(120).collect();
    sentry::with_scope(
        |scope| {
            scope.set_tag("note.kind", &note.kind);
            if let Some(tool) = &note.tool {
                scope.set_tag("note.tool", tool);
            }
            for (key, value) in [
                ("goal", &note.goal),
                ("tried", &note.tried),
                ("got", &note.got),
                ("expected", &note.expected),
                ("cost", &note.cost),
                ("would_help", &note.would_help),
                ("must_keep", &note.must_keep),
            ] {
                scope.set_extra(key, value.clone().into());
            }
            scope.set_fingerprint(Some(&[
                "note",
                &note.kind,
                note.tool.as_deref().unwrap_or("-"),
                &title,
            ]));
        },
        || {
            sentry::capture_message(
                &format!("[{}] {title}\n\n{}", note.kind, note.text()),
                sentry::Level::Info,
            )
        },
    );
    Delivery::Sent
}

#[cfg(target_arch = "wasm32")]
fn send(_note: &Note) -> Delivery {
    Delivery::Kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings and notes go to a directory of the test's own.
    fn isolated() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("newera-telemetry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        use_config_dir(dir.join("3d-new-era-ai"));
        dir
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_empty_clipboard_is_a_breadcrumb_not_a_report() {
        use sentry::integrations::tracing::EventFilter;
        use tracing::Level;
        let goes = |level, target| filter(level, target).bits();
        assert_eq!(
            goes(Level::ERROR, "egui_winit::clipboard"),
            EventFilter::Breadcrumb.bits()
        );
        assert_eq!(
            goes(Level::ERROR, "eframe::native::run"),
            EventFilter::Event.bits()
        );
        assert_eq!(goes(Level::DEBUG, "newera_app"), EventFilter::Ignore.bits());
    }

    #[test]
    fn on_by_default_off_with_one_switch_kept_between_runs_and_notes_kept() {
        let dir = isolated();
        assert!(enabled(), "never chosen: on");

        set_enabled(false).unwrap();
        assert!(!enabled(), "applies at once");
        let saved = std::fs::read_to_string(dir.join("3d-new-era-ai/telemetry.json")).unwrap();
        assert!(saved.contains("false"), "{saved}");

        // Off, a note is kept here and not sent.
        let delivery = note(&Note {
            kind: "friction".into(),
            tool: Some("measure".into()),
            goal: "medir a passagem da lavanderia".into(),
            tried: "measure(axis=x, at=450)".into(),
            got: "[613,650,null] — livre".into(),
            would_help: "a sonda ler as chapas do vassoureiro".into(),
            ..Note::default()
        })
        .unwrap();
        assert_eq!(delivery, Delivery::Kept);
        let kept = std::fs::read_to_string(notes_path()).unwrap();
        assert!(
            kept.contains("vassoureiro") && kept.contains("\"tool\":\"measure\""),
            "{kept}"
        );

        set_enabled(true).unwrap();
        assert!(enabled());
        // Without a reporter running nothing can be sent, and saying so is
        // the honest answer.
        let delivery = note(&Note {
            kind: "idea".into(),
            goal: "repetir o arremate".into(),
            tried: "place(model=…)".into(),
            got: "arquivo não existe".into(),
            would_help: "copiar peça".into(),
            ..Note::default()
        })
        .unwrap();
        assert_eq!(delivery, Delivery::Kept);
        let _ = std::fs::remove_dir_all(dir);
    }
}
