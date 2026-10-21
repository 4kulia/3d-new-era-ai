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

use newera_core::{Document, SharedDocument};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::accounts::Plan;
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
    let others: i64 = sqlx::query_scalar(
        "select coalesce(sum(size), 0)::bigint from projects where account_id = $1 and id <> $2",
    )
    .bind(account)
    .bind(project)
    .fetch_one(db)
    .await?;
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
    (total > limit).then(|| {
        format!(
            "the {} plan stores {} MB and this would take {} MB",
            plan.name,
            limit / 1024 / 1024,
            total / 1024 / 1024
        )
    })
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
    /// are unpacked there, and so are the bundles and top views the editor
    /// caches.
    ///
    /// # Errors
    ///
    /// When the directory cannot be made.
    pub fn new() -> anyhow::Result<Self> {
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
        })
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
        let permit = if heavy {
            Some(self.heavy.clone().acquire_owned().await?)
        } else {
            None
        };
        let before = loaded.doc.read().revision();
        let doc = loaded.doc.clone();
        let tool = name.to_owned();
        let ran = tokio::task::spawn_blocking(move || newera_mcp::call(doc, &tool, args)).await;
        drop(permit);
        if heavy {
            self.busy.lock().expect("busy").remove(account);
        }
        let result = match ran? {
            Ok(result) => serde_json::to_value(result)?,
            Err(why) => refused(why),
        };

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
            let file = format!("{public_url}/account/projects/{}", loaded.project);
            result["structuredContent"]["editor"] = json!(format!(
                "{editor_url}?project={}&relay={}",
                crate::login::url_encode(&file),
                crate::login::url_encode(public_url)
            ));
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
        let rows: Vec<(String, String, i32, i64, bool)> = sqlx::query_as(
            "select p.id, p.name, p.size, extract(epoch from p.updated_at)::bigint,
                    coalesce(a.project_id = p.id, false)
             from projects p left join active_projects a on a.account_id = p.account_id
             where p.account_id = $1 order by p.updated_at desc",
        )
        .bind(account)
        .fetch_all(db)
        .await?;
        let used: i64 = rows.iter().map(|r| i64::from(r.2)).sum();
        let answer = json!({
            "rows": rows.iter().map(|(id, name, size, at, active)| json!([name, id, size / 1024, at, active])).collect::<Vec<_>>(),
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
        let count: i64 = sqlx::query_scalar("select count(*) from projects where account_id = $1")
            .bind(account)
            .fetch_one(db)
            .await?;
        let most = plan.limits["projects"].as_i64().unwrap_or(0);
        if count >= most {
            return Ok(refused(format!(
                "the {} plan keeps {most} projects; open one with file action=open, or delete one from the account page",
                plan.name
            )));
        }
        let name = match name.map(clean_name).filter(|n| !n.is_empty()) {
            Some(name) => name,
            None => format!("Projeto {}", count + 1),
        };
        let data = newera_core::to_project_bytes(&Document::default());
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
        if made.is_err() {
            return Ok(refused(format!("there is a project named {name} already")));
        }
        activate(db, account, &id).await?;
        self.loaded.lock().expect("loaded").remove(account);
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
        activate(db, account, &id).await?;
        self.loaded.lock().expect("loaded").remove(account);
        Ok(said(format!("ok opened {name}")))
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
            let file = copy.file.clone();
            tokio::task::spawn_blocking(move || {
                let mut doc = Document::default();
                newera_sh3d::open_file(&mut doc, &file).map(|_| doc)
            })
            .await?
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

async fn activate(db: &PgPool, account: &str, project: &str) -> sqlx::Result<()> {
    sqlx::query(
        "insert into active_projects (account_id, project_id) values ($1, $2)
         on conflict (account_id) do update set project_id = $2",
    )
    .bind(account)
    .bind(project)
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

/// Keeps an export for a day and answers its link.
async fn keep_file(
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

    #[test]
    fn a_name_is_never_a_path() {
        assert_eq!(
            super::clean_name("  ../../etc/passwd.newera "),
            "....etcpasswd"
        );
        assert_eq!(super::clean_name("Casa da praia.newera"), "Casa da praia");
    }
}
