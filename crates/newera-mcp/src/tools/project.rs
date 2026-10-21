//! The project as an object: open it, save it, name it, version it, undo it,
//! and see who else is editing.

use std::path::PathBuf;

use newera_core::{Command, Compass, Home, Point2};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::{Raw, core, forward, invalid, ok, take_action, write_action};
use crate::compact;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct SetHomeParams {
    /// The project's name.
    name: Option<String>,
    /// Clockwise degrees from plan up to north.
    north: Option<f64>,
    /// Where the compass sits on the plan, `[x,y]` cm.
    compass_at: Option<Point2>,
    /// Compass diameter cm.
    compass_d: Option<f64>,
    /// Show or hide the compass.
    compass_visible: Option<bool>,
    /// City whose building code applies, e.g. `sao-paulo`; `""` clears it.
    /// Kept with the project, so `ergonomics`, `layout` and every dry
    /// run weigh the same municipal rules.
    city: Option<String>,
    /// Country whose standards apply, ISO 3166-1 alpha-2: `br`, `us`, `de`;
    /// `""` clears it. It decides which sources oblige and which only inform —
    /// a Brazilian standard is a reference in Florida, and the IRC is a
    /// reference here. A city we hold implies its country, so this is for
    /// saying where a project is when no code was picked.
    country: Option<String>,
    /// State or province whose code applies: `sp`, `fl`; `""` clears it. In the
    /// United States it is the state that adopts a building code, so it
    /// matters as much as the country there.
    region: Option<String>,
    /// Project properties to set, `{key: "value"}`, or remove, `{key: null}`
    /// — what an import leaves behind (window sizes, panel dividers, ids of
    /// the program it came from). With `level`, that storey's instead.
    properties: Option<serde_json::Map<String, serde_json::Value>>,
    /// Storey whose properties `properties` changes, e.g. `lv2`.
    level: Option<String>,
    /// Who lives there — occupants, children, elderly, wheelchair, stature
    /// cm, scope — kept with the project: ergonomics and every dry run then
    /// score for them. What is left out stays as it was.
    people: Option<super::check::People>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct PluginsParams {
    /// `list` (default) or `run`.
    action: Option<String>,
    /// Plugin to run, by name (the plugins tool lists them).
    name: Option<String>,
    /// Arguments passed to the plugin as JSON.
    args: Option<serde_json::Value>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct VariantsParams {
    /// `duplicate` copies the active version, `new` starts an empty one.
    #[schemars(extend("enum" = ["duplicate", "new", "switch", "rename", "delete"]))]
    action: Option<String>,
    /// Variant index for switch/rename/delete.
    i: Option<usize>,
    /// Name for duplicate/new/rename.
    name: Option<String>,
}
/// A point in the work, by name.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct CheckpointParams {
    /// Default `checkpoint`, remembering here; `revert` goes back.
    #[schemars(extend("enum" = ["checkpoint", "revert"]))]
    action: Option<String>,
    /// The checkpoint's name; required by both actions.
    label: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct RunPluginParams {
    /// Plugin to run, by name (the plugins tool lists them).
    name: String,
    /// Arguments passed to the plugin as JSON.
    args: Option<serde_json::Value>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct PathParams {
    /// Project file (`.newera`). Optional when the project was saved once.
    path: Option<String>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct OpenParams {
    /// Project (`.newera`) or Sweet Home 3D file (`.sh3d`).
    path: String,
}
/// What `file` takes; each action goes on to its own parameters.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct FileSchema {
    /// What to do with the project.
    #[schemars(extend("enum" = ["new", "open", "save"]))]
    action: String,
    /// For `open` and `save`: the file, `.newera` (or `.sh3d` to import).
    path: Option<String>,
}
/// What `edit_history` takes.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct HistorySchema {
    /// What to do.
    #[schemars(extend("enum" = ["undo", "redo", "checkpoint", "revert"]))]
    action: String,
    /// For `checkpoint` and `revert`: the checkpoint's name.
    label: Option<String>,
}
fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(newera_core::PROJECT_EXTENSION)
    }
}
#[tool_router(router = project_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        name = "edit_home",
        description = "Rename the project, set or remove project properties (properties {key: value|null}, or a storey's with level), set the compass (north), set the city whose building code applies (city=sao-paulo), and say who lives there (people {occupants, children, elderly, wheelchair, stature cm, scope}). City and people belong to the project: `ergonomics`, `layout` and every dry run then weigh the same rules for the same people, so a change can be tested against the score it moves."
    )]
    pub(crate) fn set_home(
        &self,
        Parameters(p): Parameters<SetHomeParams>,
    ) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        let mut commands = Vec::new();
        if let Some(name) = p.name {
            commands.push(Command::RenameHome { name });
        }
        if p.north.is_some()
            || p.compass_at.is_some()
            || p.compass_d.is_some()
            || p.compass_visible.is_some()
            || p.city.is_some()
            || p.country.is_some()
            || p.region.is_some()
        {
            let c = doc.home().compass.clone();
            // `""` clears, absent keeps: a caller setting the country must not
            // have to restate the city it already chose.
            let set = |given: Option<&str>, current: &Option<String>| match given.map(str::trim) {
                Some("") => None,
                Some(value) => Some(value.to_owned()),
                None => current.clone(),
            };
            commands.push(Command::SetCompass {
                compass: Compass {
                    center: p.compass_at.unwrap_or(c.center),
                    diameter: p.compass_d.unwrap_or(c.diameter),
                    north_degrees: p.north.unwrap_or(c.north_degrees),
                    visible: p.compass_visible.unwrap_or(c.visible),
                    city: set(p.city.as_deref(), &c.city),
                    country: set(p.country.as_deref(), &c.country),
                    region: set(p.region.as_deref(), &c.region),
                    ..c.clone()
                },
            });
        }
        if let Some(changes) = &p.properties {
            let apply = |target: &mut newera_core::Properties| -> Result<(), ErrorData> {
                for (key, value) in changes {
                    match value {
                        serde_json::Value::Null => {
                            target.remove(key);
                        }
                        serde_json::Value::String(text) => {
                            target.insert(key.clone(), text.clone());
                        }
                        other => {
                            target.insert(key.clone(), other.to_string());
                        }
                    }
                }
                Ok(())
            };
            if let Some(raw) = &p.level {
                let id: newera_core::LevelId =
                    raw.parse().map_err(|e| invalid(format!("level: {e}")))?;
                let mut level = doc
                    .home()
                    .level(id)
                    .cloned()
                    .ok_or_else(|| invalid(format!("no storey {raw}")))?;
                apply(&mut level.properties)?;
                commands.push(Command::update(level));
            } else {
                let mut properties = doc.home().properties.clone();
                apply(&mut properties)?;
                commands.push(Command::SetProperties { properties });
            }
        }
        if let Some(people) = &p.people {
            let kept = newera_ergonomics::Profile {
                city: None,
                ..people.over(doc.home())
            };
            let json = serde_json::to_string(&kept).unwrap_or_default();
            // Into the project's properties change when there is one, so the
            // whole call stays one step.
            let joined = commands.iter_mut().find_map(|c| match c {
                Command::SetProperties { properties } => Some(properties),
                _ => None,
            });
            if let Some(properties) = joined {
                properties.insert(newera_ergonomics::PEOPLE.to_owned(), json);
            } else {
                let mut properties = doc.home().properties.clone();
                properties.insert(newera_ergonomics::PEOPLE.to_owned(), json);
                commands.push(Command::SetProperties { properties });
            }
        }
        if commands.is_empty() {
            return Err(invalid("nothing to change"));
        }
        doc.execute(Command::Batch { commands }).map_err(core)?;
        Ok(ok(&doc, &[]))
    }
    /// Saves the project where it was, or at `path`.
    pub(crate) fn save_home(
        &self,
        Parameters(p): Parameters<PathParams>,
    ) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        let path = match (p.path, doc.path()) {
            (Some(path), _) => with_extension(PathBuf::from(path)),
            (None, Some(path)) => path.to_path_buf(),
            (None, None) => return Err(invalid("`path` is required for the first save")),
        };
        newera_core::save_project(&doc, &path)
            .map_err(|e| invalid(format!("cannot write {}: {e}", path.display())))?;
        doc.mark_saved(&path);
        Ok(format!("ok {}", path.display()))
    }
    /// Opens a project, or imports a Sweet Home 3D file, in place of this one.
    pub(crate) fn open_home(
        &self,
        Parameters(p): Parameters<OpenParams>,
    ) -> Result<String, ErrorData> {
        let path = PathBuf::from(p.path);
        let mut doc = self.document.write();
        let opened = newera_sh3d::open_file(&mut doc, &path).map_err(invalid)?;
        let mut reply = ok(&doc, &[]);
        if opened.imported {
            reply.push_str(" imported (unsaved)");
        }
        for warning in opened.warnings {
            reply.push_str("\nwarning: ");
            reply.push_str(&warning);
        }
        Ok(reply)
    }
    #[tool(
        description = "Start, open or save the project. new: an empty project in place of this one. open {path}: a project (.newera), or a Sweet Home 3D file (.sh3d) imported, in place of this one. save {path?}: writes the project (.newera); path is optional once it was saved."
    )]
    pub(crate) fn file(
        &self,
        Parameters(Raw(mut args, _)): Parameters<Raw<FileSchema>>,
    ) -> Result<String, ErrorData> {
        match take_action(&mut args, "action", &["new", "open", "save"], None)?.as_str() {
            "new" => Ok(self.new_home()),
            "open" => self.open_home(Parameters(forward(args)?)),
            _ => self.save_home(Parameters(forward(args)?)),
        }
    }
    /// Starts a new, empty project.
    pub(crate) fn new_home(&self) -> String {
        let mut doc = self.document.write();
        doc.load(Home::default());
        doc.set_path(None);
        doc.set_asset_dir(None);
        ok(&doc, &[])
    }
    #[tool(
        name = "plugins",
        description = "Plugins (external programs editing through the HTTP API): rows [name,title,description]. Run one with run_plugin."
    )]
    pub(crate) fn list_plugins(
        &self,
        Parameters(_): Parameters<super::Nothing>,
    ) -> Result<String, ErrorData> {
        self.plugins(Parameters(PluginsParams::default()))
    }
    #[tool(
        description = "Run a plugin (an external program editing through the HTTP API) by name, with args as JSON: {ok,code,stdout,stderr,edits,revision,usage {peak_mb,cpu_s,measured},limits,not_applied}. A plugin as heavy as a modeler gets the limits its plugin.json declares (cores, memory, priority, time) that this system can apply, not_applied naming the rest, and runs alone, never beside a photo render. The plugins tool lists them."
    )]
    pub(crate) fn run_plugin(
        &self,
        Parameters(p): Parameters<RunPluginParams>,
    ) -> Result<String, ErrorData> {
        self.plugins(Parameters(PluginsParams {
            action: Some("run".to_owned()),
            name: Some(p.name),
            args: p.args,
        }))
    }
    /// Plugins: the list, and running one.
    pub(crate) fn plugins(
        &self,
        Parameters(p): Parameters<PluginsParams>,
    ) -> Result<String, ErrorData> {
        let dirs = newera_plugins::plugin_dirs();
        match p.action.as_deref().unwrap_or("list") {
            "list" => {
                let rows: Vec<serde_json::Value> = newera_plugins::discover(&dirs)
                    .iter()
                    .map(|p| serde_json::json!([p.name, p.title, p.description]))
                    .collect();
                Ok(serde_json::json!({ "rows": rows }).to_string())
            }
            "run" => {
                let name = p
                    .name
                    .as_deref()
                    .ok_or_else(|| invalid("`name` is required"))?;
                let args = p.args.unwrap_or(serde_json::Value::Null);
                newera_plugins::run_for_document(&self.document, &dirs, name, &args)
                    .map(|v| v.to_string())
                    .map_err(|e| invalid(e.to_string()))
            }
            other => Err(invalid(format!("unknown action `{other}`"))),
        }
    }
    #[tool(
        description = "People and agents on this project now: rows [id,name,cursor,selection,edits]; render, while one runs: {tool, seconds, phase, done, total, threads} (render_photo cancel=true stops it)."
    )]
    pub(crate) fn sessions(&self) -> String {
        let mut doc = self.document.write();
        doc.sessions_mut().expire(newera_core::collab::now_ms());
        let rows: Vec<serde_json::Value> = doc
            .sessions()
            .list()
            .iter()
            .map(|s| {
                serde_json::json!([
                    s.id,
                    s.name,
                    s.cursor.map(|c| [compact::num(c.x), compact::num(c.y)]),
                    s.selection
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    s.edits
                ])
            })
            .collect();
        let out = serde_json::json!({ "rev": doc.revision(), "rows": rows });
        // A render running here, on the desktop (a tab renders on its own).
        #[cfg(not(target_arch = "wasm32"))]
        let out = {
            let mut out = out;
            if let Some(render) = super::native_job::status() {
                out["render"] = render;
            }
            out
        };
        out.to_string()
    }
    #[tool(
        name = "variants",
        description = "Plan versions (tabs): rows [i,name,active,walls,rooms,m2,furniture,issues]. Edits apply to the active version; change them with edit_variants."
    )]
    pub(crate) fn list_variants(
        &self,
        Parameters(_): Parameters<super::Nothing>,
    ) -> Result<String, ErrorData> {
        self.variants(Parameters(VariantsParams::default()))
    }
    #[tool(
        description = "Change the plan versions (tabs): duplicate {name?} copies the active one, new {name?} starts an empty one — both switch to it; switch {i}; rename {i,name}; delete {i}. Edits apply to the active version. The list is the variants tool."
    )]
    pub(crate) fn edit_variants(
        &self,
        Parameters(p): Parameters<VariantsParams>,
    ) -> Result<String, ErrorData> {
        write_action(
            p.action.as_deref(),
            &["duplicate", "new", "switch", "rename", "delete"],
            "variants",
        )?;
        self.variants(Parameters(p))
    }
    /// Plan versions: the list, and every change to them.
    pub(crate) fn variants(
        &self,
        Parameters(p): Parameters<VariantsParams>,
    ) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        let need = |i: Option<usize>| i.ok_or_else(|| invalid("`i` is required"));
        match p.action.as_deref().unwrap_or("list") {
            "list" => Ok(compact::variants(&doc).to_string()),
            "duplicate" | "new" => {
                let index = doc.add_variant(p.name, p.action.as_deref() == Some("duplicate"));
                Ok(format!("ok rev={} v={index} i={index}", doc.revision()))
            }
            "switch" => {
                doc.switch_variant(need(p.i)?).map_err(core)?;
                Ok(ok(&doc, &[]))
            }
            "rename" => {
                let name = p.name.ok_or_else(|| invalid("`name` is required"))?;
                doc.rename_variant(need(p.i)?, name).map_err(core)?;
                Ok(ok(&doc, &[]))
            }
            "delete" => {
                doc.remove_variant(need(p.i)?).map_err(core)?;
                Ok(ok(&doc, &[]))
            }
            other => Err(invalid(format!("unknown action `{other}`"))),
        }
    }
    #[tool(
        name = "history",
        description = "The checkpoints `edit_history` remembered: rows [label, changes ago]. A checkpoint lives with the project and survives saving."
    )]
    pub(crate) fn list_checkpoints(
        &self,
        Parameters(_): Parameters<super::Nothing>,
    ) -> Result<String, ErrorData> {
        self.checkpoint(Parameters(CheckpointParams {
            action: Some("list".to_owned()),
            label: None,
        }))
    }
    #[tool(
        description = "Undo, redo, or name where the plan is now and come back to it. undo and redo step through changes, whoever made them, and the reply names what changed. checkpoint {label} remembers this point; revert {label} undoes back down to it, keeping every id — which duplicating a version cannot do, since a copy renumbers. A checkpoint lives with the project and survives saving; `history` lists them."
    )]
    pub(crate) fn edit_history(
        &self,
        Parameters(Raw(mut args, _)): Parameters<Raw<HistorySchema>>,
    ) -> Result<String, ErrorData> {
        let action = take_action(
            &mut args,
            "action",
            &["undo", "redo", "checkpoint", "revert"],
            None,
        )?;
        match action.as_str() {
            "undo" => self.undo(),
            "redo" => self.redo(),
            _ => {
                let mut p: CheckpointParams = forward(args)?;
                p.action = Some(action);
                self.set_checkpoint(Parameters(p))
            }
        }
    }
    /// Remembers a checkpoint, or goes back to one.
    pub(crate) fn set_checkpoint(
        &self,
        Parameters(p): Parameters<CheckpointParams>,
    ) -> Result<String, ErrorData> {
        let action = write_action(
            Some(p.action.as_deref().unwrap_or("checkpoint")),
            &["checkpoint", "set", "revert"],
            "history",
        )?
        .to_owned();
        self.checkpoint(Parameters(CheckpointParams {
            action: Some(action),
            label: p.label,
        }))
    }
    /// Checkpoints: the list, remembering one, and going back to one.
    pub(crate) fn checkpoint(
        &self,
        Parameters(p): Parameters<CheckpointParams>,
    ) -> Result<String, ErrorData> {
        const KEY: &str = "checkpoint:";
        let action = p.action.as_deref().unwrap_or("list");
        let mut doc = self.document.write();
        let depth = doc.undo_depth();
        let mut properties = doc.home().properties.clone();
        match action {
            "list" => {
                let rows: Vec<serde_json::Value> = properties
                    .iter()
                    .filter_map(|(key, value)| {
                        let label = key.strip_prefix(KEY)?;
                        let at: usize = value.parse().ok()?;
                        Some(serde_json::json!([label, depth.saturating_sub(at)]))
                    })
                    .collect();
                Ok(serde_json::json!({ "checkpoints": rows }).to_string())
            }
            "checkpoint" | "set" => {
                let label = p.label.ok_or_else(|| invalid("`label` is required"))?;
                // Writing the checkpoint is itself a change: the point to
                // come back to is the one just after it, so coming back does
                // not undo the checkpoint along with the work.
                properties.insert(format!("{KEY}{label}"), (depth + 1).to_string());
                doc.execute(Command::SetProperties { properties })
                    .map_err(core)?;
                Ok(ok(&doc, &[]))
            }
            "revert" => {
                let label = p.label.ok_or_else(|| invalid("`label` is required"))?;
                let at: usize = properties
                    .get(&format!("{KEY}{label}"))
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| invalid(format!("no checkpoint `{label}`")))?;
                if at > depth {
                    return Err(invalid(format!(
                        "`{label}` is ahead of where the plan is: nothing to undo"
                    )));
                }
                for _ in 0..(depth - at) {
                    doc.undo().map_err(core)?;
                }
                Ok(ok(&doc, &[]))
            }
            other => Err(invalid(format!("unknown action `{other}`"))),
        }
    }
    /// Undoes the last change, whoever made it.
    pub(crate) fn undo(&self) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        let before = doc.home().clone();
        doc.undo().map_err(core)?;
        // What was undone, named: an undo that takes the plan's reference
        // numbers with it is otherwise found only in a render.
        Ok(super::reply::applied(&doc, &before))
    }
    /// Redoes the last undone change.
    pub(crate) fn redo(&self) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        let before = doc.home().clone();
        doc.redo().map_err(core)?;
        Ok(super::reply::applied(&doc, &before))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::CreateParams;
    use crate::tools::server;

    #[test]
    fn an_import_is_cleaned_by_rule_not_one_element_at_a_time() {
        let s = server();
        {
            let mut doc = s.document.write();
            let part = |id: u64, name: &str| newera_core::Furniture {
                id: newera_core::FurnitureId(id),
                catalog: "box".into(),
                name: name.to_owned(),
                width: 30.0,
                depth: 30.0,
                height: 30.0,
                ..newera_core::Furniture::default()
            };
            let mut tower = part(1, "12 — Torre quente");
            tower.children = vec![
                part(2, "48 — Gabinete do tanque 80,5 cm"),
                part(3, "puxador"),
            ];
            doc.execute(Command::Batch {
                commands: vec![
                    Command::insert(tower),
                    Command::insert(part(4, "Mesa Dover")),
                    Command::SetProperties {
                        properties: [
                            (
                                "com.eteks.sweethome3d.SweetHome3D.FrameX".to_owned(),
                                "40".to_owned(),
                            ),
                            ("keep".to_owned(), "yes".to_owned()),
                        ]
                        .into(),
                    },
                ],
            })
            .unwrap();
            for text in ["[09]", "[10]", "Vidro canelado"] {
                let label = newera_core::Label {
                    id: doc.new_label_id(),
                    text: text.to_owned(),
                    ..newera_core::Label::default()
                };
                doc.execute(Command::insert(label)).unwrap();
            }
        }
        // The index codes, found by pattern.
        let found: serde_json::Value = serde_json::from_str(
            &s.annotations(Parameters(
                serde_json::from_str(r#"{"q":"re:^\\[\\d+\\]$"}"#).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(found["labels"].as_array().unwrap().len(), 2, "{found}");

        // The numbered prefixes go in one call, parts of groups included.
        let rename = |json: &str| s.update(Parameters(serde_json::from_str(json).unwrap()));
        let dry: serde_json::Value = serde_json::from_str(
            &rename(r#"{"rename":{"pattern":"^\\d+ — ","to":""},"dry":true}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(dry["changed"].as_array().unwrap().len(), 2, "{dry}");
        rename(r#"{"rename":{"pattern":"^\\d+ — ","to":""}}"#).unwrap();
        let names: Vec<String> = {
            let doc = s.document.read();
            doc.home()
                .furniture
                .iter()
                .flat_map(newera_core::Furniture::flatten)
                .map(|f| f.name.clone())
                .collect()
        };
        assert_eq!(
            names,
            [
                "Torre quente",
                "Gabinete do tanque 80,5 cm",
                "puxador",
                "Mesa Dover"
            ]
        );
        let nothing = rename(r#"{"rename":{"pattern":"^\\d+ — ","to":""}}"#).unwrap_err();
        assert!(nothing.message.contains("nothing matches"), "{nothing:?}");

        // The properties an import left, removed by name.
        s.set_home(Parameters(
            serde_json::from_str(
                r#"{"properties":{"com.eteks.sweethome3d.SweetHome3D.FrameX":null,"source":"limpo"}}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let doc = s.document.read();
        let props = &doc.home().properties;
        assert!(
            !props.contains_key("com.eteks.sweethome3d.SweetHome3D.FrameX"),
            "{props:?}"
        );
        assert_eq!(props.get("keep").map(String::as_str), Some("yes"));
        assert_eq!(props.get("source").map(String::as_str), Some("limpo"));
    }

    #[test]
    fn an_undo_that_switches_the_plans_annotations_off_says_so() {
        let s = server();
        s.annotations(Parameters(
            serde_json::from_str(r#"{"refs":true}"#).unwrap(),
        ))
        .unwrap();
        let reply = s.undo().unwrap();
        let diff: serde_json::Value =
            serde_json::from_str(&reply[reply.find('{').expect(&reply)..]).unwrap();
        assert_eq!(diff["annotations"]["from"]["refs"], true, "{reply}");
        assert_eq!(diff["annotations"]["to"]["refs"], false, "{reply}");
        let reply = s.redo().unwrap();
        assert!(
            reply.contains(r#""to":{"details":false,"dims":false,"legend":false,"refs":true}"#),
            "{reply}"
        );
    }

    #[test]
    fn variants_duplicate_switch_and_list() {
        let s = server();
        let params: CreateParams =
            serde_json::from_str(r#"{"walls":[{"pts":[[0,0],[300,0]]}]}"#).unwrap();
        s.create(Parameters(params)).unwrap();
        let reply = s
            .variants(Parameters(VariantsParams {
                action: Some("duplicate".into()),
                name: Some("B".into()),
                i: None,
            }))
            .unwrap();
        assert!(reply.ends_with("i=1"), "{reply}");
        let params: CreateParams =
            serde_json::from_str(r#"{"walls":[{"pts":[[0,100],[300,100]]}]}"#).unwrap();
        s.create(Parameters(params)).unwrap();
        let list = s.variants(Parameters(VariantsParams::default())).unwrap();
        assert_eq!(
            list,
            r#"[[0,"Versão 1",false,1,0,0.0,0,0],[1,"B",true,2,0,0.0,0,0]]"#
        );
        s.variants(Parameters(VariantsParams {
            action: Some("switch".into()),
            i: Some(0),
            name: None,
        }))
        .unwrap();
        assert_eq!(s.document.read().home().walls.len(), 1);
        assert!(
            s.variants(Parameters(VariantsParams {
                action: Some("switch".into()),
                i: Some(9),
                name: None
            }))
            .is_err()
        );
    }
    #[test]
    fn save_open_round_trip() {
        let s = server();
        let dir = std::env::temp_dir().join(format!("newera-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("casa");
        let params: CreateParams =
            serde_json::from_str(r#"{"labels":[{"text":"Oi","at":[1,2]}]}"#).unwrap();
        s.create(Parameters(params)).unwrap();
        let reply = s
            .save_home(Parameters(PathParams {
                path: Some(path.display().to_string()),
            }))
            .unwrap();
        assert!(reply.ends_with("casa.newera"), "{reply}");
        s.new_home();
        assert_eq!(
            s.document.read().home().labels,
            [] as [newera_core::Label; 0]
        );
        s.open_home(Parameters(OpenParams {
            path: dir.join("casa.newera").display().to_string(),
        }))
        .unwrap();
        assert_eq!(s.document.read().home().labels[0].text, "Oi");
        assert!(!s.document.read().is_modified());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn plugins_and_sessions_tools() {
        let s = server();
        let list: serde_json::Value =
            serde_json::from_str(&s.plugins(Parameters(PluginsParams::default())).unwrap())
                .unwrap();
        assert!(list["rows"].is_array());
        // Without an HTTP server there is nothing for plugins to call back.
        assert!(
            s.plugins(Parameters(PluginsParams {
                action: Some("run".into()),
                ..PluginsParams::default()
            }))
            .is_err()
        );
        s.document
            .write()
            .sessions_mut()
            .join("Ana", newera_core::collab::now_ms());
        let rows: serde_json::Value = serde_json::from_str(&s.sessions()).unwrap();
        assert_eq!(rows["rows"][0][1], "Ana");
    }
    #[test]
    fn writes_name_the_active_variant_once_there_are_several() {
        let s = server();
        let wall = || -> CreateParams {
            serde_json::from_str(r#"{"walls":[{"pts":[[0,0],[100,0]]}]}"#).unwrap()
        };
        assert!(!s.create(Parameters(wall())).unwrap().contains(" v="));
        let reply = s
            .variants(Parameters(VariantsParams {
                action: Some("new".into()),
                ..VariantsParams::default()
            }))
            .unwrap();
        assert!(reply.contains(" v=1 i=1"), "{reply}");
        let reply = s.create(Parameters(wall())).unwrap();
        assert!(reply.contains(" v=1 ids="), "{reply}");
    }
    #[test]
    fn writes_can_name_their_version() {
        let s = server();
        s.variants(Parameters(VariantsParams {
            action: Some("new".into()),
            ..VariantsParams::default()
        }))
        .unwrap();
        // Someone switches back to the first tab meanwhile.
        s.document.write().switch_variant(0).unwrap();
        let params: CreateParams =
            serde_json::from_str(r#"{"v":1,"walls":[{"pts":[[0,0],[100,0]]}]}"#).unwrap();
        let reply = s.create(Parameters(params)).unwrap();
        assert!(reply.contains(" v=1 "), "{reply}");
        let doc = s.document.read();
        assert_eq!(doc.active_variant(), 1);
        assert_eq!(doc.home().walls.len(), 1);
        drop(doc);
        let bad: CreateParams =
            serde_json::from_str(r#"{"v":9,"walls":[{"pts":[[0,0],[100,0]]}]}"#).unwrap();
        assert!(s.create(Parameters(bad)).is_err());
    }

    /// Somewhere to come back to that does not renumber the plan: the reason
    /// a real session edited the user's own drawing instead of a copy.
    #[test]
    fn a_checkpoint_comes_back_without_renumbering_anything() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true,"t":15}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let ids = |s: &NewEraMcp| -> Vec<String> {
            s.document
                .read()
                .home()
                .walls
                .iter()
                .map(|w| w.id.to_string())
                .collect()
        };
        let before = ids(&s);
        let checkpoint = |json: &str| -> serde_json::Value {
            serde_json::from_str(
                &s.checkpoint(Parameters(serde_json::from_str(json).unwrap()))
                    .unwrap(),
            )
            .unwrap_or_default()
        };
        checkpoint(r#"{"action":"checkpoint","label":"banheiros"}"#);

        s.create(Parameters(
            serde_json::from_str(r#"{"walls":[{"pts":[[200,0],[200,300]],"t":10}]}"#).unwrap(),
        ))
        .unwrap();
        s.create(Parameters(
            serde_json::from_str(r#"{"walls":[{"pts":[[0,150],[200,150]],"t":10}]}"#).unwrap(),
        ))
        .unwrap();
        assert_eq!(s.document.read().home().walls.len(), 6);
        let listed = checkpoint(r#"{"action":"list"}"#);
        assert_eq!(listed["checkpoints"][0][0], "banheiros", "{listed}");
        assert_eq!(listed["checkpoints"][0][1], 2, "two changes ago: {listed}");

        checkpoint(r#"{"action":"revert","label":"banheiros"}"#);
        assert_eq!(ids(&s), before, "the same walls, with the same ids");
        // And the checkpoint is still there to come back to again.
        let listed = checkpoint(r#"{"action":"list"}"#);
        assert_eq!(listed["checkpoints"][0][0], "banheiros", "{listed}");
    }
}
