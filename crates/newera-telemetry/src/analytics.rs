//! How many people open the app, and on what.
//!
//! One event per run — `app_open`, with the version, the operating system, the
//! architecture and the mode the app was started in. It rides the switch the
//! rest of this crate rides: off means nothing leaves.
//!
//! What is sent: an id drawn at random the first time and kept in the config
//! directory, the app version, the OS family, the architecture and the mode.
//! What is not: the project, its path, the plan, the MCP token, the machine's
//! name, the user's name. The id names an installation so two runs can be told
//! apart from two machines; it is not a person and it is thrown away with the
//! settings directory.
//!
//! The ping goes to the project's own endpoint on the site, which holds the
//! Google Analytics secret and forwards the count. That is why it goes there
//! rather than to Google directly: a secret baked into a binary only reaches
//! the builds CI makes, and then everyone who compiles the app themselves
//! counts for nothing. The installers post the same shape from the same
//! installation id, so an install and the first run are the same installation
//! in the count.
//!
//! A build to work on, and CI, stay out of the count.

use std::time::Duration;

/// The project's endpoint. What holds the Google Analytics secret is the site,
/// not this binary.
pub(crate) const PING: &str = "https://3dneweraai.com/ping";

/// Where a ping goes, when one goes at all.
///
/// `NEWERA_PING_URL` points it somewhere else — a local `wrangler pages dev`,
/// while working on the endpoint — and asking for that is reason enough to
/// count a build that would otherwise stay quiet.
fn endpoint() -> Option<String> {
    if let Some(url) = std::env::var("NEWERA_PING_URL")
        .ok()
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
    {
        return Some(url);
    }
    super::a_keyless_build_may_report().then(|| PING.to_owned())
}

/// The id of this installation, drawn once and kept beside the settings. Not
/// a person: reinstalling draws another one.
fn client_id() -> std::io::Result<String> {
    client_id_in(&super::config_dir())
}

/// The same, in a named directory — which is what the test uses, so it never
/// touches the settings of the machine it runs on. The installers write this
/// same file before the app has ever run.
fn client_id_in(dir: &std::path::Path) -> std::io::Result<String> {
    let path = dir.join("install-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_owned();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    // Enough randomness to not collide, from what the standard library has:
    // the clock and the address of a fresh allocation.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let here = std::ptr::from_ref::<String>(&String::new()) as usize;
    let id = format!("{:x}{here:x}", now.as_nanos());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, &id)?;
    Ok(id)
}

/// What one run of the app looks like to the count. The endpoint accepts these
/// names and no others.
fn payload(client: &str, mode: &str) -> serde_json::Value {
    serde_json::json!({
        "event": "app_open",
        "client_id": client,
        "app_version": env!("CARGO_PKG_VERSION"),
        "operating_system": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "mode": mode,
    })
}

/// Reports that the app was opened, in the background: a run never waits on
/// the network, and a refusal from it is not worth a word on screen.
///
/// `mode` is how it was started — `gui`, `serve`, `mcp` — so a headless
/// server is not counted as somebody sitting in front of a window.
pub fn opened(mode: &str) {
    if !super::enabled() {
        return;
    }
    let Some(url) = endpoint() else {
        return;
    };
    let Ok(client) = client_id() else {
        return;
    };
    let body = payload(&client, mode);
    std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(4)))
            .build()
            .into();
        // Whatever comes back is dropped: the call is a ping, and a failed
        // ping is not news.
        let _ = agent.post(&url).send_json(&body);
    });
}

#[cfg(test)]
mod tests {
    /// The names the endpoint expects, and nothing else in the body: a field it
    /// does not know is dropped there, so a typo here would count nothing.
    #[test]
    fn the_ping_says_what_was_opened() {
        let body = super::payload("abc123", "gui");
        assert_eq!(body["event"], "app_open");
        assert_eq!(body["client_id"], "abc123");
        assert_eq!(body["mode"], "gui");
        assert_eq!(body["app_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["operating_system"], std::env::consts::OS);
        assert_eq!(body["architecture"], std::env::consts::ARCH);
        let serde_json::Value::Object(fields) = body else {
            panic!("an object")
        };
        assert_eq!(fields.len(), 6, "an unknown field would be dropped anyway");
    }

    /// A build for working on, and CI, stay out of the count: what is counted
    /// is the people who installed the app.
    #[test]
    fn a_build_to_work_on_counts_nothing() {
        // Pointed at a local endpoint on purpose, or an optimised build off
        // CI — which is what reports, and not what this is about.
        if std::env::var_os("NEWERA_PING_URL").is_some() || crate::a_keyless_build_may_report() {
            return;
        }
        assert!(super::endpoint().is_none());
        // Nowhere to send, so this returns without a thread and without a
        // request.
        super::opened("test");
    }

    /// The id is drawn once and then answers the same for this installation.
    #[test]
    fn the_installation_id_is_kept() {
        let dir = std::env::temp_dir().join(format!("newera-ga-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = super::client_id_in(&dir).expect("an id");
        let again = super::client_id_in(&dir).expect("the same id");
        assert_eq!(first, again);
        assert_ne!(first, "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
