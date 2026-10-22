//! The editor with no window: for an account whose AI calls while no tab is
//! open, the tools run here, on the account's active project.
//!
//! A project lives in the database as the same `.newera` the desktop writes.
//! To work on it, it is unpacked into its own directory under the engine's
//! root, opened like the desktop opens it, and written back after every call
//! that changed it. The root is the only place the process reads files from
//! (`newera_core::vfs::jail`), so no project can name another's files, or the
//! server's.
//!
//! What a server cannot do is refused with a reason, and what means
//! something else here is translated: `file` (new, open, save) works
//! on the account's projects by name, and exports come back as a link.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use newera_core::{Document, SharedDocument};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::accounts::Plan;
use crate::config::Renderer;
use crate::secret;

/// Tools that run for minutes of CPU: one at a time per account, and no more
/// at once than the machine has cores to spare.
const HEAVY: [&str; 4] = ["render_photo", "render_3d", "render_plan", "export"];

/// How long an export's link works.
const FILE_HOURS: i32 = 24;

/// A project open for an account.
#[derive(Debug)]
struct Loaded {
    project: String,
    doc: SharedDocument,
    /// This copy's own directory: no other copy of the project, old or new,
    /// reads or writes in it. It goes when the copy is dropped.
    dir: PathBuf,
    file: PathBuf,
    touched: Mutex<Instant>,
    /// The stored revision this copy was loaded at, or last saved as. Held
    /// for the whole of a save, so two calls saving at once follow on from
    /// each other instead of both claiming the same revision.
    revision: tokio::sync::Mutex<i64>,
    /// The same revision, readable without waiting for a save to end.
    known: AtomicI64,
}

impl Drop for Loaded {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// How a save went.
#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    /// Kept, as this revision.
    Saved(i64),
    /// Somebody saved since the revision the save started from: nothing kept.
    Stale,
    /// The plan has no room for it; this says why. Nothing kept.
    Full(String),
}

/// Keeps a project's bytes if the plan has room for them and the stored
/// revision is still `from`, as `from + 1`. Every save goes through here:
/// the engine's, and the editor tab's.
///
/// # Errors
///
/// When the database fails.
pub async fn store(
    db: &PgPool,
    account: &str,
    plan: &Plan,
    project: &str,
    from: i64,
    bytes: &[u8],
) -> sqlx::Result<Stored> {
    let others = kept_bytes(db, account, Some(project), None).await?;
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if let Some(why) = no_room(plan, others.saturating_add(size)) {
        return Ok(Stored::Full(why));
    }
    let saved: Option<i64> = sqlx::query_scalar(
        "update projects set data = $1, size = $2, updated_at = now(), revision = revision + 1
         where id = $3 and account_id = $4 and revision = $5 returning revision",
    )
    .bind(bytes)
    .bind(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
    .bind(project)
    .bind(account)
    .bind(from)
    .fetch_optional(db)
    .await?;
    Ok(saved.map_or(Stored::Stale, Stored::Saved))
}

/// The bytes an account keeps, which its plan's storage holds: its
/// projects but `except_project`, and its products' files but
/// `except_file` — a product's file of a kind (`model`, `texture`) about
/// to be replaced.
///
/// # Errors
///
/// When the database fails.
pub async fn kept_bytes(
    db: &PgPool,
    account: &str,
    except_project: Option<&str>,
    except_file: Option<(&str, &str)>,
) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "select ((select coalesce(sum(size), 0) from projects
                 where account_id = $1 and id is distinct from $2)
              + (select coalesce(sum(a.size), 0) from product_assets a
                 join products p on p.id = a.product_id
                 where p.account_id = $1
                   and not (a.product_id is not distinct from $3
                            and a.kind is not distinct from $4)))::bigint",
    )
    .bind(account)
    .bind(except_project)
    .bind(except_file.map(|(product, _)| product))
    .bind(except_file.map(|(_, kind)| kind))
    .fetch_one(db)
    .await
}

/// How many bytes of projects the plan stores.
#[must_use]
pub fn storage_limit(plan: &Plan) -> i64 {
    plan.limits["storage_mb"]
        .as_i64()
        .unwrap_or(0)
        .saturating_mul(1024 * 1024)
}

/// What to say when an account's projects would take `total` bytes and the
/// plan stores less; `None` when it has room.
#[must_use]
pub fn no_room(plan: &Plan, total: i64) -> Option<String> {
    let limit = storage_limit(plan);
    // Rounded up to the tenth: a size under a MB is never said as none.
    #[allow(clippy::cast_precision_loss)] // bytes far below 2^52
    let tenths = (total as f64 / (1024.0 * 1024.0) * 10.0).ceil() / 10.0;
    (total > limit).then(|| {
        format!(
            "the {} plan stores {} MB and this would take {tenths:.1} MB",
            plan.name,
            limit / 1024 / 1024,
        )
    })
}

/// One of an account's projects, as its lists show it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Listed {
    pub id: String,
    pub name: String,
    /// Bytes stored.
    pub size: i32,
    /// When it last changed, in seconds since the epoch.
    pub updated_at: i64,
    /// Whether it is the project the account's AI works on with no tab open.
    pub active: bool,
}

/// The account's projects, the last changed first.
///
/// # Errors
///
/// When the database fails.
pub async fn projects(db: &PgPool, account: &str) -> sqlx::Result<Vec<Listed>> {
    let rows: Vec<(String, String, i32, i64, bool)> = sqlx::query_as(
        "select p.id, p.name, p.size, extract(epoch from p.updated_at)::bigint,
                coalesce(a.project_id = p.id, false)
         from projects p left join active_projects a on a.account_id = p.account_id
         where p.account_id = $1 order by p.updated_at desc, p.id",
    )
    .bind(account)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, size, updated_at, active)| Listed {
            id,
            name,
            size,
            updated_at,
            active,
        })
        .collect())
}

/// How making a project went.
#[derive(Debug, PartialEq, Eq)]
pub enum Made {
    /// Made, empty, at revision 0, under this id and name.
    New { id: String, name: String },
    /// The plan keeps this many projects, and the account has them all.
    TooMany(i64),
    /// The plan has no room for even an empty one; this says why.
    Full(String),
    /// The account has a project of this name already.
    Taken(String),
}

/// Makes an empty project for the account, named `name` as cleaned (a
/// numbered default when that leaves nothing), if the plan keeps one more
/// and has room for it. Every new project goes through here: the AI's
/// `file action=new`, and the editor's.
///
/// # Errors
///
/// When the database fails.
pub async fn make(
    db: &PgPool,
    account: &str,
    plan: &Plan,
    name: Option<&str>,
) -> sqlx::Result<Made> {
    let count: i64 = sqlx::query_scalar("select count(*) from projects where account_id = $1")
        .bind(account)
        .fetch_one(db)
        .await?;
    let used = kept_bytes(db, account, None, None).await?;
    let most = plan.limits["projects"].as_i64().unwrap_or(0);
    if count >= most {
        return Ok(Made::TooMany(most));
    }
    let data = newera_core::to_project_bytes(&Document::default());
    let size = i64::try_from(data.len()).unwrap_or(i64::MAX);
    if let Some(why) = no_room(plan, used.saturating_add(size)) {
        return Ok(Made::Full(why));
    }
    let name = match name.map(clean_name).filter(|n| !n.is_empty()) {
        Some(name) => name,
        None => format!("Projeto {}", count + 1),
    };
    let id = secret::id();
    let made = sqlx::query(
        "insert into projects (id, account_id, name, data, size) values ($1, $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(account)
    .bind(&name)
    .bind(&data)
    .bind(i32::try_from(data.len()).unwrap_or(i32::MAX))
    .execute(db)
    .await;
    match made {
        Ok(_) => Ok(Made::New { id, name }),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(Made::Taken(name)),
        Err(e) => Err(e),
    }
}

/// Where a cloud project opens in the editor: the editor at `editor_url`,
/// given the project's address at the service `public_url` and the service
/// itself, so the tab saves it back there.
#[must_use]
pub fn editor_link(editor_url: &str, public_url: &str, project: &str) -> String {
    let file = format!("{public_url}/account/projects/{project}");
    format!(
        "{editor_url}?project={}&relay={}",
        crate::login::url_encode(&file),
        crate::login::url_encode(public_url)
    )
}

/// The engine: the projects open now, and the gate for heavy work.
#[derive(Debug)]
pub struct Engine {
    root: PathBuf,
    loaded: Mutex<HashMap<String, Arc<Loaded>>>,
    /// One per account, held while its project is being opened, so two calls
    /// arriving together open one copy and not two.
    opening: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    heavy: Arc<tokio::sync::Semaphore>,
    busy: Mutex<HashSet<String>>,
    reads: HashSet<String>,
    /// Where photos are made when it is reached: Cycles on a GPU
    /// (`crate::renderer`), and the client that reaches it.
    gpu: Option<(Renderer, reqwest::Client)>,
}

/// A tool's answer, as the protocol has it (`CallToolResult` as JSON).
fn said(text: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}], "isError": false})
}

fn refused(text: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}], "isError": true})
}

impl Engine {
    /// An engine working in the asset cache (`$XDG_CACHE_HOME/3d-new-era-ai`),
    /// which becomes the only place this process reads files from: projects
    /// are unpacked there, each with its files in its own copy's directory,
    /// and so are the top views the editor caches. `renderer`, when given,
    /// makes `render_photo`'s photos while it is reached.
    ///
    /// # Errors
    ///
    /// When the directory cannot be made.
    pub fn new(renderer: Option<Renderer>) -> anyhow::Result<Self> {
        let cache = newera_core::cache_dir();
        std::fs::create_dir_all(cache.join("cloud-projects"))?;
        std::fs::create_dir_all(cache.join("cloud-exports"))?;
        let root = cache.canonicalize()?;
        newera_core::vfs::jail(&root);
        let reads = newera_mcp::tools()
            .into_iter()
            .filter(|t| t.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true))
            .map(|t| t.name.to_string())
            .collect();
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        Ok(Self {
            root,
            loaded: Mutex::new(HashMap::new()),
            opening: Mutex::new(HashMap::new()),
            heavy: Arc::new(tokio::sync::Semaphore::new(cores.saturating_sub(1).max(1))),
            busy: Mutex::new(HashSet::new()),
            reads,
            gpu: renderer.map(|r| (r, crate::renderer::client())),
        })
    }

    /// The engine's root: the one place this process reads files from on
    /// disk (`newera_core::vfs::jail`).
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Forgets projects nobody touched for a while; they are saved already.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the open projects.
    pub fn sweep(&self, idle: Duration) {
        let mut loaded = self.loaded.lock().expect("loaded");
        loaded.retain(|_, l| l.touched.lock().expect("touched").elapsed() < idle);
        self.opening
            .lock()
            .expect("opening")
            .retain(|_, gate| Arc::strong_count(gate) > 1);
    }

    /// Runs one tool for an account, on its active project.
    ///
    /// # Errors
    ///
    /// When the database fails; a tool that refuses answers `Ok` with
    /// `isError`, as the protocol wants.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the engine's state.
    #[allow(clippy::too_many_arguments, reason = "each is one fact about the call")]
    pub async fn call(
        &self,
        db: &PgPool,
        public_url: &str,
        editor_url: &str,
        account: &str,
        plan: &Plan,
        name: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        self.call_with(db, public_url, editor_url, account, plan, name, args, &[])
            .await
    }

    /// [`Engine::call`], with `files` — (path inside the project, bytes) —
    /// put among the project's files first, in the copy the call runs on: a
    /// shop product's model for the `place` that uses it
    /// (`crate::staging`). The next save carries them in the `.newera`.
    ///
    /// # Errors
    ///
    /// As [`Engine::call`], and when a file cannot be written.
    ///
    /// # Panics
    ///
    /// As [`Engine::call`].
    #[allow(clippy::too_many_arguments, reason = "each is one fact about the call")]
    pub async fn call_with(
        &self,
        db: &PgPool,
        public_url: &str,
        editor_url: &str,
        account: &str,
        plan: &Plan,
        name: &str,
        args: Value,
        files: &[(String, Vec<u8>)],
    ) -> anyhow::Result<Value> {
        match name {
            "projects" => return self.list(db, account, plan).await,
            "file" if args["action"] == "new" => {
                return self.create(db, account, plan, args["name"].as_str()).await;
            }
            "file" if args["action"] == "open" => {
                let Some(wanted) = args["path"].as_str() else {
                    return Ok(refused(
                        "file action=open: path is the name of one of your projects (projects lists them)",
                    ));
                };
                return self.open(db, account, wanted).await;
            }
            "file" if args["action"] == "save" => {
                return self.keep(db, account, plan, args["path"].as_str()).await;
            }
            "file" => return Ok(refused("file: action is new, open or save")),
            _ => {}
        }
        // A photo counts against the plan by the quality asked for.
        let photo = (name == "render_photo").then(|| {
            if args["quality"].as_str().unwrap_or("draft") == "draft" {
                "draft_render"
            } else {
                "hq_render"
            }
        });
        if let Some(kind) = photo
            && let Some(why) = self.over_quota(db, account, plan, kind).await?
        {
            return Ok(refused(why));
        }

        let loaded = self.loaded(db, account, plan).await?;
        *loaded.touched.lock().expect("touched") = Instant::now();
        put_files(&loaded, files).await?;

        // Files go where the engine says, and come back as a link.
        let mut args = args;
        let export = match name {
            "export" => {
                let asked = args["path"].as_str().unwrap_or("export");
                let ext = Path::new(asked)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map_or_else(
                        || {
                            if args["what"] == "cut_list" {
                                "csv".into()
                            } else {
                                "pdf".into()
                            }
                        },
                        str::to_ascii_lowercase,
                    );
                let file_name = Path::new(asked)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("export")
                    .to_owned();
                let target = self
                    .root
                    .join("cloud-exports")
                    .join(format!("{}.{ext}", secret::id()));
                args["path"] = json!(target.to_string_lossy());
                Some((target, file_name, ext))
            }
            _ => None,
        };

        let heavy = HEAVY.contains(&name);
        if heavy && !self.busy.lock().expect("busy").insert(account.to_owned()) {
            return Ok(refused(
                "another render is running for this account: wait for it to finish, then ask again",
            ));
        }
        let before = loaded.doc.read().revision();
        let ran = self.run(&loaded, name, args, heavy).await;
        if heavy {
            self.busy.lock().expect("busy").remove(account);
        }
        let result = ran?;

        if let Some(kind) = photo
            && result["isError"] != true
        {
            count(db, account, kind).await?;
        }
        if !self.reads.contains(name)
            && loaded.doc.read().revision() != before
            && let Some(why) = self.save(db, account, plan, &loaded).await?
        {
            return Ok(refused(why));
        }
        // "Open in the editor" opens this very project there.
        if name == "show_plan" && result["isError"] != true {
            let mut result = result;
            result["structuredContent"]["editor"] =
                json!(editor_link(editor_url, public_url, &loaded.project));
            return Ok(result);
        }
        if let Some((target, file_name, ext)) = export
            && result["isError"] != true
        {
            let bytes = tokio::fs::read(&target).await.unwrap_or_default();
            let _ = tokio::fs::remove_file(&target).await;
            if bytes.is_empty() {
                return Ok(refused("the export wrote nothing"));
            }
            let url = keep_file(db, public_url, account, &file_name, &ext, bytes).await?;
            return Ok(said(format!(
                "ok {url} (the link works for {FILE_HOURS} hours)"
            )));
        }
        Ok(result)
    }

    /// Runs the tool on the copy: a photo on the GPU renderer when one is
    /// configured, falling back to the built-in renderer — within what is
    /// left of the call's `max_s` — with a note when it is not made there;
    /// everything else, and that fallback, here, heavy work under the gate
    /// of the machine's cores.
    async fn run(
        &self,
        loaded: &Loaded,
        name: &str,
        args: Value,
        heavy: bool,
    ) -> anyhow::Result<Value> {
        let started = Instant::now();
        let mut note = None;
        if name == "render_photo"
            && let Some((renderer, client)) = &self.gpu
        {
            let asked = {
                let _permit = self.heavy.clone().acquire_owned().await?;
                let (doc, args) = (loaded.doc.clone(), args.clone());
                tokio::task::spawn_blocking(move || newera_mcp::photo_scene(doc, args)).await?
            };
            match asked {
                Err(why) => return Ok(refused(why)),
                Ok(None) => {}
                Ok(Some(ask)) => {
                    match crate::renderer::render(renderer, client, &ask, started).await {
                        crate::renderer::Made::Photo(png) => {
                            let dark = newera_mcp::photo_dark_note(&png);
                            let mut answer = json!({
                                "content": [{
                                    "type": "image",
                                    "data": base64::engine::general_purpose::STANDARD.encode(png),
                                    "mimeType": "image/png",
                                }],
                                "isError": false,
                            });
                            newera_mcp::output::structure_json(&mut answer);
                            // Judged as the built-in renderer's photo is.
                            if let Some(note) = dark {
                                newera_mcp::output::add_note(&mut answer, &note);
                            }
                            return Ok(answer);
                        }
                        crate::renderer::Made::Missed(why) => {
                            tracing::warn!("GPU renderer: {why}");
                            note = Some(format!(
                                "The GPU renderer did not make this photo ({why}): it is from the built-in renderer."
                            ));
                        }
                    }
                }
            }
        }
        let permit = if heavy {
            Some(self.heavy.clone().acquire_owned().await?)
        } else {
            None
        };
        let doc = loaded.doc.clone();
        let tool = name.to_owned();
        let ran = tokio::task::spawn_blocking(move || {
            if note.is_some() {
                newera_mcp::render_photo_since(doc, args, started)
            } else {
                newera_mcp::call(doc, &tool, args)
            }
            .map(|answer| (answer, note))
        })
        .await;
        drop(permit);
        Ok(match ran? {
            Ok((answer, note)) => {
                let mut result = serde_json::to_value(answer)?;
                // A refusal is an `Err`: only a photo gets here.
                if let Some(note) = note {
                    newera_mcp::output::add_note(&mut result, &note);
                }
                result
            }
            Err(why) => refused(why),
        })
    }

    /// Whether a photo of this kind would go over the plan, and what to say
    /// if so: drafts are counted by the day, the rest by the month.
    async fn over_quota(
        &self,
        db: &PgPool,
        account: &str,
        plan: &Plan,
        kind: &str,
    ) -> anyhow::Result<Option<String>> {
        let draft = kind == "draft_render";
        let limit = plan.limits[if draft {
            "draft_renders_per_day"
        } else {
            "hq_renders_per_month"
        }]
        .as_i64()
        .unwrap_or(0);
        let used: i64 = sqlx::query_scalar(
            "select coalesce(sum(count), 0)::bigint from usage
             where account_id = $1 and kind = $2
               and day >= case when $3 then current_date else date_trunc('month', now())::date end",
        )
        .bind(account)
        .bind(kind)
        .bind(draft)
        .fetch_one(db)
        .await?;
        Ok((used >= limit).then(|| match (draft, limit) {
            (false, 0) => format!(
                "render_photo at quality good or best is not in the {} plan; quality draft is",
                plan.name
            ),
            (true, _) => format!(
                "the {} plan's {limit} draft photos for today are used up",
                plan.name
            ),
            (false, _) => format!(
                "the {} plan's {limit} good or best photos for this month are used up",
                plan.name
            ),
        }))
    }

    /// The account's projects, and how much of the plan they use.
    async fn list(&self, db: &PgPool, account: &str, plan: &Plan) -> anyhow::Result<Value> {
        let rows = projects(db, account).await?;
        let used: i64 = rows.iter().map(|r| i64::from(r.size)).sum();
        let answer = json!({
            "rows": rows.iter().map(|r| json!([r.name, r.id, r.size / 1024, r.updated_at, r.active])).collect::<Vec<_>>(),
            "columns": ["name", "id", "kb", "updated", "active"],
            "used_kb": used / 1024,
            "limits": plan.limits,
        });
        Ok(said(answer.to_string()))
    }

    /// A new, empty project, made active.
    async fn create(
        &self,
        db: &PgPool,
        account: &str,
        plan: &Plan,
        name: Option<&str>,
    ) -> anyhow::Result<Value> {
        let (id, name) = match make(db, account, plan, name).await? {
            Made::New { id, name } => (id, name),
            Made::TooMany(most) => {
                return Ok(refused(format!(
                    "the {} plan keeps {most} projects; open one with file action=open, or delete one from the account page",
                    plan.name
                )));
            }
            Made::Full(why) => return Ok(refused(why)),
            Made::Taken(name) => {
                return Ok(refused(format!("there is a project named {name} already")));
            }
        };
        self.activate(db, account, &id).await?;
        Ok(said(format!("ok new project {name}")))
    }

    /// Saves the active project now, renamed when a name is given.
    async fn keep(
        &self,
        db: &PgPool,
        account: &str,
        plan: &Plan,
        name: Option<&str>,
    ) -> anyhow::Result<Value> {
        let loaded = self.loaded(db, account, plan).await?;
        if let Some(name) = name.map(clean_name).filter(|n| !n.is_empty()) {
            let renamed =
                sqlx::query("update projects set name = $1 where id = $2 and account_id = $3")
                    .bind(&name)
                    .bind(&loaded.project)
                    .bind(account)
                    .execute(db)
                    .await;
            if renamed.is_err() {
                return Ok(refused(format!("there is a project named {name} already")));
            }
        }
        if let Some(why) = self.save(db, account, plan, &loaded).await? {
            return Ok(refused(why));
        }
        let name: String = sqlx::query_scalar("select name from projects where id = $1")
            .bind(&loaded.project)
            .fetch_one(db)
            .await?;
        Ok(said(format!("ok saved {name}")))
    }

    /// Makes a project the active one, by name or id.
    async fn open(&self, db: &PgPool, account: &str, wanted: &str) -> anyhow::Result<Value> {
        let wanted = wanted.trim().trim_end_matches(".newera");
        let found: Option<(String, String)> = sqlx::query_as(
            "select id, name from projects where account_id = $1 and (id = $2 or lower(name) = lower($2))",
        )
        .bind(account)
        .bind(wanted)
        .fetch_optional(db)
        .await?;
        let Some((id, name)) = found else {
            return Ok(refused(format!(
                "no project named {wanted}; projects lists them"
            )));
        };
        self.activate(db, account, &id).await?;
        Ok(said(format!("ok opened {name}")))
    }

    /// Makes `project` the account's active one, if it is the account's:
    /// what its AI works on when no tab holds the room, and what renders are
    /// of. The copy open of another project is dropped, so the next call
    /// opens this one. Every switch goes through here: the AI's
    /// `file action=new` and `open`, and a tab showing one of the account's
    /// projects. A copy of the same project stays: it is the one wanted.
    /// False when the account has no such project.
    ///
    /// # Errors
    ///
    /// When the database fails.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the open projects.
    pub async fn activate(&self, db: &PgPool, account: &str, project: &str) -> sqlx::Result<bool> {
        let made: Option<String> = sqlx::query_scalar(
            "insert into active_projects (account_id, project_id)
             select account_id, id from projects where account_id = $1 and id = $2
             on conflict (account_id) do update set project_id = excluded.project_id
             returning project_id",
        )
        .bind(account)
        .bind(project)
        .fetch_optional(db)
        .await?;
        if made.is_none() {
            return Ok(false);
        }
        let mut loaded = self.loaded.lock().expect("loaded");
        if loaded.get(account).is_some_and(|l| l.project != project) {
            loaded.remove(account);
        }
        Ok(true)
    }

    /// The account's active project, open — made first if it has none.
    async fn loaded(&self, db: &PgPool, account: &str, plan: &Plan) -> anyhow::Result<Arc<Loaded>> {
        let gate = self
            .opening
            .lock()
            .expect("opening")
            .entry(account.to_owned())
            .or_default()
            .clone();
        let _opening = gate.lock().await;
        let active: Option<String> =
            sqlx::query_scalar("select project_id from active_projects where account_id = $1")
                .bind(account)
                .fetch_optional(db)
                .await?;
        if let Some(current) = self.loaded.lock().expect("loaded").get(account)
            && Some(&current.project) == active.as_ref()
        {
            return Ok(current.clone());
        }
        let project = if let Some(project) = active {
            project
        } else {
            self.create(db, account, plan, None).await?;
            sqlx::query_scalar("select project_id from active_projects where account_id = $1")
                .bind(account)
                .fetch_one(db)
                .await?
        };
        let (data, revision): (Vec<u8>, i64) =
            sqlx::query_as("select data, revision from projects where id = $1 and account_id = $2")
                .bind(&project)
                .bind(account)
                .fetch_one(db)
                .await?;
        // Opened with its products' current files, at the stored revision.
        let data = crate::current::current(db, account, &data, &self.root.join("cloud-projects"))
            .await?
            .unwrap_or(data);
        let dir = self.root.join("cloud-projects").join(secret::id());
        tokio::fs::create_dir_all(&dir).await?;
        let file = dir.join("project.newera");
        // From here the copy owns the directory, and drops it with itself.
        let mut copy = Loaded {
            project,
            doc: SharedDocument::new(Document::default()),
            dir,
            file,
            touched: Mutex::new(Instant::now()),
            revision: tokio::sync::Mutex::new(revision),
            known: AtomicI64::new(revision),
        };
        tokio::fs::write(&copy.file, &data).await?;
        let opened = {
            let (dir, file) = (copy.dir.clone(), copy.file.clone());
            tokio::task::spawn_blocking(move || open_copy(&dir, &file)).await?
        };
        let doc = opened.map_err(|why| anyhow::anyhow!("the project would not open: {why}"))?;
        copy.doc = SharedDocument::new(doc);
        let loaded = Arc::new(copy);
        self.loaded
            .lock()
            .expect("loaded")
            .insert(account.to_owned(), loaded.clone());
        Ok(loaded)
    }

    /// Puts `bytes` at `path` among the files of the account's copy open
    /// now, when that copy draws something with the file, and saves the
    /// project with it: a product's model attached after it was placed with
    /// a placeholder, or a finish's new texture — with `tile`, the size the
    /// surfaces drawn with it take, in every variant (`newera_core::finishes::retile`). False when no copy is open, none of it uses the file,
    /// or the project changed meanwhile (it then takes the file the next
    /// time it is opened, `crate::current`).
    ///
    /// # Errors
    ///
    /// When the file cannot be written or the database fails.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the open projects.
    pub async fn swap_in(
        &self,
        db: &PgPool,
        account: &str,
        plan: &Plan,
        path: &str,
        bytes: &[u8],
        tile: Option<[f64; 2]>,
    ) -> anyhow::Result<bool> {
        let Some(loaded) = self.loaded.lock().expect("loaded").get(account).cloned() else {
            return Ok(false);
        };
        let uses = loaded
            .doc
            .read()
            .variants()
            .any(|v| v.home().asset_paths().iter().any(|p| p == path));
        if !uses {
            return Ok(false);
        }
        put_files(&loaded, &[(path.to_owned(), bytes.to_vec())]).await?;
        if let Some(tile) = tile {
            newera_core::finishes::retile(&mut loaded.doc.write(), path, tile)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Ok(self.save(db, account, plan, &loaded).await?.is_none())
    }

    /// Forgets the account's copy of `project` if it is older than
    /// `revision`, which was just kept: the next call loads the kept one. A
    /// copy at `revision` or later is the kept one already. Waits for a copy
    /// being opened right now, so one read from before the save does not
    /// outlive it.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the open projects.
    pub async fn forget(&self, account: &str, project: &str, revision: i64) {
        let gate = self
            .opening
            .lock()
            .expect("opening")
            .entry(account.to_owned())
            .or_default()
            .clone();
        let _opening = gate.lock().await;
        let mut loaded = self.loaded.lock().expect("loaded");
        if loaded
            .get(account)
            .is_some_and(|l| l.project == project && l.known.load(Ordering::SeqCst) < revision)
        {
            loaded.remove(account);
        }
    }

    /// Writes the project back. `Some(why)` when it was not kept: the plan
    /// has no room for it, or the project was saved from elsewhere since this
    /// copy was loaded — then the copy is dropped and the saved one loaded,
    /// and the call that changed it is to be made again.
    async fn save(
        &self,
        db: &PgPool,
        account: &str,
        plan: &Plan,
        loaded: &Arc<Loaded>,
    ) -> anyhow::Result<Option<String>> {
        let mut revision = loaded.revision.lock().await;
        let bytes = {
            let (doc, file) = (loaded.doc.clone(), loaded.file.clone());
            tokio::task::spawn_blocking(move || {
                newera_core::save_project(&doc.read(), &file)?;
                std::fs::read(&file).map_err(newera_core::ProjectError::from)
            })
            .await??
        };
        match store(db, account, plan, &loaded.project, *revision, &bytes).await? {
            Stored::Saved(saved) => {
                *revision = saved;
                loaded.known.store(saved, Ordering::SeqCst);
                // Another copy of this project, opened at an earlier
                // revision, is out of date now: reads must not see it.
                self.forget(account, &loaded.project, saved).await;
                Ok(None)
            }
            Stored::Full(why) => Ok(Some(format!(
                "the change is on screen but was not kept: {why}"
            ))),
            Stored::Stale => {
                drop(revision);
                {
                    let mut open = self.loaded.lock().expect("loaded");
                    if open.get(account).is_some_and(|l| Arc::ptr_eq(l, loaded)) {
                        open.remove(account);
                    }
                }
                self.loaded(db, account, plan).await?;
                Ok(Some("the project changed, call again".to_owned()))
            }
        }
    }
}

/// Writes `files` where the copy's relative paths resolve
/// (`Document::asset_dir`) — its unpacked bundle, or its own directory for a
/// project that had no files, which then becomes where they resolve.
async fn put_files(loaded: &Loaded, files: &[(String, Vec<u8>)]) -> anyhow::Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let dir = {
        let mut doc = loaded.doc.write();
        if let Some(dir) = doc.asset_dir() {
            dir
        } else {
            doc.set_asset_dir(Some(loaded.dir.clone()));
            loaded.dir.clone()
        }
    };
    for (path, bytes) in files {
        if !Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            anyhow::bail!("{path} is not a path inside the project");
        }
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&target, bytes).await?;
    }
    Ok(())
}

/// Opens the project written at `file` the way the desktop opens it, but
/// unpacks its files inside the copy's own directory `dir` rather than the
/// shared bundle cache: whatever the load unpacked goes with the copy.
fn open_copy(dir: &Path, file: &Path) -> Result<Document, String> {
    let (project, assets) =
        newera_core::open_project(file, &dir.join("bundle")).map_err(|e| e.to_string())?;
    let mut doc = Document::default();
    newera_sh3d::Loaded::Project {
        project,
        path: file.to_owned(),
        assets,
    }
    .into_document(&mut doc);
    Ok(doc)
}

async fn count(db: &PgPool, account: &str, kind: &str) -> sqlx::Result<()> {
    sqlx::query(
        "insert into usage (account_id, day, kind, count) values ($1, current_date, $2, 1)
         on conflict (account_id, day, kind) do update set count = usage.count + 1",
    )
    .bind(account)
    .bind(kind)
    .execute(db)
    .await?;
    Ok(())
}

/// A project name as people type it, without what would make it a path.
fn clean_name(raw: &str) -> String {
    raw.trim()
        .trim_end_matches(".newera")
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .take(80)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Keeps an export for a day and answers its link: the engine's exports,
/// and the estimate's sheet.
pub(crate) async fn keep_file(
    db: &PgPool,
    public_url: &str,
    account: &str,
    name: &str,
    ext: &str,
    bytes: Vec<u8>,
) -> sqlx::Result<String> {
    let content_type = match ext {
        "pdf" => "application/pdf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "glb" => "model/gltf-binary",
        "obj" => "text/plain",
        "csv" => "text/csv",
        "dxf" => "application/dxf",
        _ => "application/octet-stream",
    };
    let token = secret::token();
    sqlx::query(
        "insert into files (token_hash, account_id, name, content_type, data, expires_at)
         values ($1, $2, $3, $4, $5, now() + make_interval(hours => $6))",
    )
    .bind(secret::hash(&token))
    .bind(account)
    .bind(clean_name(name))
    .bind(content_type)
    .bind(bytes)
    .bind(FILE_HOURS)
    .execute(db)
    .await?;
    Ok(format!("{public_url}/files/{token}"))
}

#[cfg(test)]
mod tests {
    /// What a refusal says it would take is never rounded down to nothing:
    /// a tenth of a MB, rounded up.
    #[test]
    fn a_full_plan_says_what_it_would_take_to_the_tenth() {
        let plan = |mb: i64| crate::accounts::Plan {
            code: "tiny".into(),
            name: "Tiny".into(),
            limits: serde_json::json!({"storage_mb": mb}),
        };
        let mb = 1024 * 1024;
        assert_eq!(
            super::no_room(&plan(0), 1),
            Some("the Tiny plan stores 0 MB and this would take 0.1 MB".to_owned())
        );
        assert_eq!(
            super::no_room(&plan(1), mb + 1),
            Some("the Tiny plan stores 1 MB and this would take 1.1 MB".to_owned())
        );
        assert_eq!(
            super::no_room(&plan(1), mb * 3 / 2),
            Some("the Tiny plan stores 1 MB and this would take 1.5 MB".to_owned())
        );
        assert_eq!(super::no_room(&plan(1), mb), None);
    }

    #[test]
    fn a_copy_takes_its_directory_with_it() {
        let dir = std::env::temp_dir().join(format!("engine-drop-{}", super::secret::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("project.newera");
        std::fs::write(&file, b"x").unwrap();
        let copy = super::Loaded {
            project: "p".into(),
            doc: super::SharedDocument::new(super::Document::default()),
            dir: dir.clone(),
            file,
            touched: std::sync::Mutex::new(std::time::Instant::now()),
            revision: tokio::sync::Mutex::new(1),
            known: super::AtomicI64::new(1),
        };
        assert!(dir.exists());
        drop(copy);
        assert!(!dir.exists());
    }

    /// A project with a model in it, as the bytes the database keeps.
    fn project_with_a_model(scratch: &std::path::Path) -> Vec<u8> {
        let assets = scratch.join("assets");
        std::fs::create_dir_all(assets.join("models")).unwrap();
        std::fs::write(assets.join("models/chair.obj"), "v 0 0 0\n").unwrap();
        let mut home = newera_core::Home::default();
        let mut piece = newera_core::Furniture {
            model: Some("models/chair.obj".into()),
            ..newera_core::Furniture::default()
        };
        piece.id = home.new_furniture_id();
        home.furniture.push(piece);
        let mut doc = super::Document::new(home);
        doc.set_asset_dir(Some(assets));
        let file = scratch.join("made.newera");
        newera_core::save_project(&doc, &file).unwrap();
        std::fs::read(file).unwrap()
    }

    #[test]
    fn a_dropped_copy_leaves_no_unpacked_bundle() {
        let scratch = std::env::temp_dir().join(format!("engine-bundle-{}", super::secret::id()));
        let bytes = project_with_a_model(&scratch);
        let dir = scratch.join("copy");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("project.newera");
        std::fs::write(&file, &bytes).unwrap();
        let doc = super::open_copy(&dir, &file).unwrap();
        let assets = doc.asset_dir().expect("the bundle's files are unpacked");
        assert!(
            assets.join("models/chair.obj").exists(),
            "the model is where the copy reads it"
        );
        let copy = super::Loaded {
            project: "p".into(),
            doc: super::SharedDocument::new(doc),
            dir: dir.clone(),
            file,
            touched: std::sync::Mutex::new(std::time::Instant::now()),
            revision: tokio::sync::Mutex::new(1),
            known: super::AtomicI64::new(1),
        };
        drop(copy);
        let left = assets.exists();
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(!left, "{} outlived the copy", assets.display());
    }

    #[test]
    fn a_name_is_never_a_path() {
        assert_eq!(
            super::clean_name("  ../../etc/passwd.newera "),
            "....etcpasswd"
        );
        assert_eq!(super::clean_name("Casa da praia.newera"), "Casa da praia");
    }
}
