//! Plugins: external programs that extend the editor through its public
//! HTTP API, in any language.
//!
//! A plugin is a directory with a `plugin.json`:
//!
//! ```json
//! {
//!   "name": "room-areas",
//!   "title": "Room areas",
//!   "description": "Adds a text with the area of every room.",
//!   "command": ["python3", "main.py"]
//! }
//! ```
//!
//! Running it starts `command` inside that directory with
//! - `NEWERA_URL` — base URL of the API (`GET /api/home`, `POST /api/commands`, `/mcp`…)
//! - `NEWERA_TOKEN` — bearer token, when the server has one
//! - `NEWERA_SESSION` — a session id, so edits show up as the plugin's
//! - arguments as JSON on standard input.
//!
//! Standard output is returned to whoever ran it (menu, REST or MCP).
//!
//! Plugins are looked up in every directory of `NEWERA_PLUGINS` and in
//! `<config>/3d-new-era-ai/plugins`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use web_time::Instant;

use serde::{Deserialize, Serialize};

/// Manifest file inside a plugin directory.
pub const MANIFEST: &str = "plugin.json";
/// Longest time a plugin may run.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Output kept from each stream.
const MAX_OUTPUT: usize = 64 * 1024;

/// A plugin found on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plugin {
    /// Identifier: lowercase letters, digits, `-` and `_`.
    pub name: String,
    /// Name shown in menus.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Program and arguments, run inside the plugin directory.
    pub command: Vec<String>,
    /// What it may use: set for a program as heavy as a modeler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    #[serde(skip)]
    pub dir: PathBuf,
}

/// The resources a plugin may use, applied to its process — not asked of
/// it — where the system allows:
///
/// ```json
/// "limits": {"heavy": true, "threads": 2, "memory_mb": 4096, "nice": 15, "timeout_s": 900}
/// ```
///
/// On Linux `threads` pins it to that many cores (`taskset`), `memory_mb`
/// caps its address space (`prlimit`) and `nice` lowers its priority; it
/// also gets `NEWERA_THREADS` and `OMP_NUM_THREADS` (a Blender script
/// passes `--threads $NEWERA_THREADS`). A `heavy` plugin runs alone: not
/// beside a photo render or another heavy plugin. What could not be
/// applied is said in the run's answer, never assumed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub heavy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nice: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u64>,
}

impl Limits {
    /// Threads it gets: as asked, else half the cores (at most four) when heavy.
    fn threads(&self) -> Option<usize> {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        self.threads
            .or_else(|| self.heavy.then(|| (cores / 2).clamp(1, 4)))
            .map(|t| t.clamp(1, cores))
    }
}

/// A program on `PATH`.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

impl Plugin {
    /// Menu title, falling back to the name.
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.name
        } else {
            &self.title
        }
    }
}

/// Where the plugin reaches the editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    pub url: String,
    pub token: Option<String>,
    pub session: Option<String>,
}

/// What a finished run produced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunOutput {
    /// Exit code; `None` when killed (timeout) or ended by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    /// Peak resident memory, MB, where it could be measured.
    pub peak_mb: Option<f64>,
    /// Processor time used, seconds, where it could be measured.
    pub cpu_s: Option<f64>,
    /// Limits applied to the process: `threads 2 (taskset)`, …
    pub applied: Vec<String>,
    /// Limits asked for that this system could not apply, and why.
    pub not_applied: Vec<String>,
}

impl RunOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }
}

/// Directories searched for plugins, in order.
pub fn plugin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("NEWERA_PLUGINS")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    if let Some(config) = config {
        dirs.push(config.join("3d-new-era-ai").join("plugins"));
    }
    dirs
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Reads one plugin directory.
///
/// # Errors
/// When the manifest is missing or invalid.
pub fn load(dir: &Path) -> Result<Plugin, String> {
    let file = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut plugin: Plugin =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    if !valid_name(&plugin.name) {
        return Err(format!(
            "{}: invalid name `{}`",
            file.display(),
            plugin.name
        ));
    }
    if plugin.command.is_empty() {
        return Err(format!("{}: empty command", file.display()));
    }
    plugin.dir = dir.to_path_buf();
    Ok(plugin)
}

/// Every valid plugin under `dirs` (first one wins on duplicate names),
/// sorted by name. Invalid manifests are skipped.
pub fn discover(dirs: &[PathBuf]) -> Vec<Plugin> {
    let mut found: Vec<Plugin> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths.into_iter().filter(|p| p.join(MANIFEST).is_file()) {
            if let Ok(plugin) = load(&path)
                && !found.iter().any(|p| p.name == plugin.name)
            {
                found.push(plugin);
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Finds a plugin by name.
pub fn find(name: &str) -> Option<Plugin> {
    discover(&plugin_dirs())
        .into_iter()
        .find(|p| p.name == name)
}

fn read_capped(mut stream: impl Read) -> String {
    let mut buf = Vec::new();
    let _ = stream
        .by_ref()
        .take(MAX_OUTPUT as u64)
        .read_to_end(&mut buf);
    // Drain the rest so the child never blocks on a full pipe.
    let _ = std::io::copy(&mut stream, &mut std::io::sink());
    String::from_utf8_lossy(&buf).into_owned()
}

/// A helper thread of [`run`].
#[allow(
    clippy::disallowed_methods,
    reason = "only `run` calls it, after `Command::spawn` succeeded, which a browser tab never does"
)]
fn helper<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    std::thread::spawn(work)
}

/// Runs `plugin` with `args` on standard input and waits up to `timeout`.
///
/// # Errors
/// When the program can't be started.
pub fn run(
    plugin: &Plugin,
    host: &Host,
    args: &serde_json::Value,
    timeout: Duration,
) -> Result<RunOutput, String> {
    let (program, rest) = plugin.command.split_first().ok_or("empty command")?;
    // A program given relative to the plugin (`./run.sh`) is found there.
    let local = plugin.dir.join(program);
    let program = if program.contains('/') && local.exists() {
        local.into_os_string()
    } else {
        program.into()
    };
    // Limits are applied by wrapping the program in tools that set them and
    // then become it (`prlimit … nice … taskset … program`), so the plugin's
    // own process carries them.
    let limits = plugin.limits.clone().unwrap_or_default();
    let (mut applied, mut not_applied) = (Vec::new(), Vec::new());
    let mut chain: Vec<std::ffi::OsString> = Vec::new();
    let linux = cfg!(target_os = "linux");
    let mut wrap = |tool: &str, args: Vec<String>, what: String| {
        if linux && on_path(tool) {
            chain.push(tool.into());
            chain.extend(args.into_iter().map(Into::into));
            applied.push(format!("{what} ({tool})"));
        } else {
            not_applied.push(format!("{what}: no {tool} on this system"));
        }
    };
    if let Some(mb) = limits.memory_mb {
        wrap(
            "prlimit",
            vec![format!("--as={}", mb.saturating_mul(1024 * 1024))],
            format!("memory {mb} MB"),
        );
    }
    if let Some(nice) = limits.nice {
        wrap(
            "nice",
            vec!["-n".into(), nice.clamp(0, 19).to_string()],
            format!("priority nice +{}", nice.clamp(0, 19)),
        );
    }
    let threads = limits.threads();
    // Pinned to CPUs this process may use: in a container or under an
    // inherited mask they need not start at 0.
    let cpus = threads.map(|t| {
        let allowed = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
                    .map(|a| a.trim().to_owned())
            });
        (t, allowed.and_then(|a| cpu_list(&a, t)))
    });
    if let Some((t, Some(list))) = &cpus {
        wrap(
            "taskset",
            vec!["-c".into(), list.clone()],
            format!("threads {t}"),
        );
    }
    if let Some((t, None)) = cpus
        && linux
    {
        not_applied.push(format!(
            "threads {t}: the CPUs this process may use are unknown"
        ));
    }
    chain.push(program);
    let mut chain = chain.into_iter();
    let Some(first) = chain.next() else {
        return Err("empty command".into());
    };
    let mut command = Command::new(first);
    command.args(chain);
    if let Some(t) = threads {
        command
            .env("NEWERA_THREADS", t.to_string())
            .env("OMP_NUM_THREADS", t.to_string());
    }
    command
        .args(rest)
        .current_dir(&plugin.dir)
        .env("NEWERA_URL", &host.url)
        .env("NEWERA_PLUGIN_DIR", &plugin.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match &host.token {
        Some(token) => command.env("NEWERA_TOKEN", token),
        None => command.env_remove("NEWERA_TOKEN"),
    };
    match &host.session {
        Some(session) => command.env("NEWERA_SESSION", session),
        None => command.env_remove("NEWERA_SESSION"),
    };
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start `{}`: {e}", plugin.command.join(" ")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = args.to_string();
        helper(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let stdout = child.stdout.take().map(|s| helper(move || read_capped(s)));
    let stderr = child.stderr.take().map(|s| helper(move || read_capped(s)));

    let started = Instant::now();
    let mut timed_out = false;
    let pid = child.id();
    let (mut peak_mb, mut cpu_s) = (None, None);
    let status = loop {
        // Read while it runs: once it ends, its numbers are gone.
        if let Some((peak, cpu)) = usage(pid) {
            peak_mb = Some(peak);
            cpu_s = Some(cpu);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                timed_out = true;
                let _ = child.kill();
                break child.wait().ok();
            }
            Ok(None) => {
                #[allow(
                    clippy::disallowed_methods,
                    reason = "reached only after `Command::spawn` succeeded, which a browser tab never does"
                )]
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => return Err(err.to_string()),
        }
    };
    let join = |handle: Option<std::thread::JoinHandle<String>>| {
        handle.and_then(|h| h.join().ok()).unwrap_or_default()
    };
    Ok(RunOutput {
        code: if timed_out {
            None
        } else {
            status.and_then(|s| s.code())
        },
        stdout: join(stdout),
        stderr: join(stderr),
        timed_out,
        peak_mb,
        cpu_s,
        applied,
        not_applied,
    })
}

/// Peak resident memory (MB) and processor time (s) of a running process,
/// from `/proc` — Linux only; elsewhere they are not measured.
fn usage(pid: u32) -> Option<(f64, f64)> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let peak_kb: f64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the command name, which may hold spaces: utime and
    // stime are the 12th and 13th, in clock ticks (100 a second on Linux).
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    Some((peak_kb / 1024.0, ticks / 100.0))
}

/// Why a plugin could not run for a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    NotFound(String),
    /// The document has no HTTP server to call back into.
    NoServer,
    Start(String),
    /// A heavy plugin found heavy work already running.
    Busy(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "no plugin {name}"),
            Self::NoServer => {
                f.write_str("plugins need the HTTP server (start the editor or `newera serve`)")
            }
            Self::Start(err) => f.write_str(err),
            Self::Busy(holder) => write!(
                f,
                "{holder} is running, and heavy work runs one at a time; wait for it or stop it"
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// Runs plugin `name` (searched in `dirs`) against the server of `document`
/// under a session of its own, and reports its output, how many edits it
/// made and the revision it left.
///
/// The document lock is not held while the plugin runs, so it can call back.
///
/// # Errors
/// See [`RunError`].
pub fn run_for_document(
    document: &newera_core::SharedDocument,
    dirs: &[PathBuf],
    name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, RunError> {
    use newera_core::collab::now_ms;
    let plugin = discover(dirs)
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| RunError::NotFound(name.to_owned()))?;
    let (server, session) = {
        let mut doc = document.write();
        let server = doc.server().cloned().ok_or(RunError::NoServer)?;
        let session = doc.sessions_mut().join(plugin.label(), now_ms());
        (server, session.id)
    };
    let host = Host {
        url: server.url,
        token: server.token,
        session: Some(session.clone()),
    };
    let limits = plugin.limits.clone().unwrap_or_default();
    let slot = if limits.heavy {
        match newera_core::progress::heavy(&format!("plugin {}", plugin.name)) {
            Ok(slot) => Some(slot),
            Err(holder) => {
                document.write().sessions_mut().leave(&session);
                return Err(RunError::Busy(holder));
            }
        }
    } else {
        None
    };
    let timeout = limits
        .timeout_s
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs);
    let result = run(&plugin, &host, args, timeout);
    drop(slot);
    let mut doc = document.write();
    let edits = doc.sessions().get(&session).map_or(0, |s| s.edits);
    doc.sessions_mut().leave(&session);
    let output = result.map_err(RunError::Start)?;
    Ok(serde_json::json!({
        "ok": output.success(),
        "code": output.code,
        "timed_out": output.timed_out,
        "stdout": output.stdout,
        "stderr": output.stderr,
        "edits": edits,
        "revision": doc.revision(),
        "usage": {
            "peak_mb": output.peak_mb.map(|v| (v * 10.0).round() / 10.0),
            "cpu_s": output.cpu_s.map(|v| (v * 100.0).round() / 100.0),
            "measured": output.peak_mb.is_some(),
        },
        "limits": output.applied,
        "not_applied": output.not_applied,
    }))
}

/// The first `t` CPUs of an allowed list like `0-3,8-11`, as `taskset -c`
/// takes them (`0,1,2,3,8,9`); all of them when there are fewer.
fn cpu_list(allowed: &str, t: usize) -> Option<String> {
    let mut cpus = Vec::new();
    for range in allowed.split(',').map(str::trim).filter(|r| !r.is_empty()) {
        let (a, b) = range.split_once('-').unwrap_or((range, range));
        let (a, b): (usize, usize) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        cpus.extend(a..=b);
    }
    cpus.truncate(t);
    (!cpus.is_empty()).then(|| {
        cpus.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn threads_are_pinned_to_cpus_the_process_may_use() {
        use super::cpu_list;
        assert_eq!(cpu_list("2-5", 2).as_deref(), Some("2,3"));
        assert_eq!(cpu_list("0-3,8-11", 6).as_deref(), Some("0,1,2,3,8,9"));
        assert_eq!(cpu_list("7", 4).as_deref(), Some("7"));
        assert_eq!(cpu_list("", 2), None);
        assert_eq!(cpu_list("x-y", 2), None);
    }

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("newera-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_plugin(root: &Path, dir: &str, manifest: &str) -> PathBuf {
        let path = root.join(dir);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(MANIFEST), manifest).unwrap();
        path
    }

    #[test]
    fn discovers_valid_plugins_first_directory_wins() {
        let a = temp_dir("a");
        let b = temp_dir("b");
        write_plugin(
            &a,
            "one",
            r#"{"name":"one","title":"Um","command":["true"]}"#,
        );
        write_plugin(&a, "bad", r#"{"name":"Bad Name","command":["true"]}"#);
        write_plugin(&a, "empty", r#"{"name":"empty","command":[]}"#);
        write_plugin(
            &b,
            "one",
            r#"{"name":"one","title":"Outro","command":["true"]}"#,
        );
        write_plugin(&b, "two", r#"{"name":"two","command":["true"]}"#);
        let found = discover(&[a.clone(), b.clone(), a.join("missing")]);
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["one", "two"]);
        assert_eq!(found[0].label(), "Um");
        assert_eq!(found[1].label(), "two");
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    #[cfg(unix)]
    #[test]
    fn runs_with_the_host_environment_and_stdin() {
        let root = temp_dir("run");
        let dir = write_plugin(
            &root,
            "echo",
            r#"{"name":"echo","command":["sh","-c","printf '%s|%s|%s|' \"$NEWERA_URL\" \"$NEWERA_TOKEN\" \"$NEWERA_SESSION\"; cat; echo oops >&2; exit 3"]}"#,
        );
        let plugin = load(&dir).unwrap();
        let host = Host {
            url: "http://127.0.0.1:7878".into(),
            token: Some("t0k".into()),
            session: Some("s4".into()),
        };
        let out = run(
            &plugin,
            &host,
            &serde_json::json!({"n": 2}),
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert_eq!(out.stdout, r#"http://127.0.0.1:7878|t0k|s4|{"n":2}"#);
        assert_eq!(out.stderr.trim(), "oops");
        assert_eq!(out.code, Some(3));
        assert!(!out.success());

        let slow = write_plugin(&root, "slow", r#"{"name":"slow","command":["sleep","5"]}"#);
        let out = run(
            &load(&slow).unwrap(),
            &host,
            &serde_json::Value::Null,
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(out.timed_out && out.code.is_none());

        let missing = write_plugin(
            &root,
            "missing",
            r#"{"name":"missing","command":["./nope"]}"#,
        );
        assert!(
            run(
                &load(&missing).unwrap(),
                &host,
                &serde_json::Value::Null,
                DEFAULT_TIMEOUT
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_heavy_plugin_runs_inside_its_limits_and_says_what_it_used() {
        let root = temp_dir("limits");
        let dir = write_plugin(
            &root,
            "modeler",
            r#"{"name":"modeler","limits":{"heavy":true,"threads":1,"memory_mb":512,"nice":10,"timeout_s":30},
                "command":["sh","-c","echo threads=$NEWERA_THREADS; grep Cpus_allowed_list /proc/self/status; echo mem=$(ulimit -v); echo nice=$(nice); i=0; while [ $i -lt 30000 ]; do i=$((i+1)); done"]}"#,
        );
        let plugin = load(&dir).unwrap();
        let out = run(
            &plugin,
            &Host::default(),
            &serde_json::Value::Null,
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert!(out.success(), "{out:?}");
        assert!(out.stdout.contains("threads=1"), "{}", out.stdout);
        // `nice` adds to the niceness this test already runs at.
        let own: i32 = std::fs::read_to_string("/proc/self/stat")
            .unwrap()
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(16)
            .unwrap()
            .parse()
            .unwrap();
        let nice = format!("nice={}", (own + 10).min(19));
        for (tool, said) in [
            ("taskset", "Cpus_allowed_list:\t0\n"),
            ("prlimit", "mem=524288"),
            ("nice", nice.as_str()),
        ] {
            if on_path(tool) {
                assert!(out.stdout.contains(said), "{tool}: {}", out.stdout);
                assert!(
                    out.applied
                        .iter()
                        .any(|a| a.ends_with(&format!("({tool})"))),
                    "{out:?}"
                );
            } else {
                assert!(out.not_applied.iter().any(|a| a.contains(tool)), "{out:?}");
            }
        }
        assert!(out.peak_mb.is_some_and(|m| m > 0.0), "{out:?}");
        assert!(out.cpu_s.is_some(), "{out:?}");
        std::fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_heavy_plugin_waits_for_no_one_and_its_deadline_is_its_own() {
        let root = temp_dir("heavy");
        write_plugin(
            &root,
            "slow",
            r#"{"name":"slow","limits":{"heavy":true,"timeout_s":1},"command":["sleep","5"]}"#,
        );
        let document = newera_core::SharedDocument::new(newera_core::Document::default());
        document
            .write()
            .set_server(Some(newera_core::collab::ServerInfo {
                url: "http://127.0.0.1:1".into(),
                token: None,
            }));
        // A photo holds the heavy-work slot: the plugin is refused, not queued.
        let photo = newera_core::progress::heavy("render_photo").unwrap();
        let busy = run_for_document(
            &document,
            std::slice::from_ref(&root),
            "slow",
            &serde_json::Value::Null,
        )
        .unwrap_err();
        assert!(
            matches!(&busy, RunError::Busy(holder) if holder == "render_photo"),
            "{busy}"
        );
        assert!(
            document.read().sessions().list().is_empty(),
            "its session is left"
        );
        drop(photo);
        let started = Instant::now();
        let done = run_for_document(
            &document,
            std::slice::from_ref(&root),
            "slow",
            &serde_json::Value::Null,
        )
        .unwrap();
        assert_eq!(done["timed_out"], true);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "timeout_s=1, not the default"
        );
        assert!(
            newera_core::progress::heavy("next").is_ok(),
            "the slot is free again"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
