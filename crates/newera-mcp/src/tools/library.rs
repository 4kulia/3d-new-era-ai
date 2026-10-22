//! The model library: approved revisions of imported models, kept apart
//! from any project and placed in any of them as `library:<name>@<v>`.

use newera_core::library::{self, Entry};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::NewEraMcp;
use super::reply::invalid;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LibraryParams {
    /// One entry's versions instead of the list, e.g. `win`.
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditLibraryParams {
    /// `publish`: the piece's model as the next version of `name`.
    #[schemars(extend("enum" = ["publish"]))]
    action: String,
    /// publish: the piece whose model, photos, measurements and unit are kept, e.g. `f12`.
    id: String,
    /// publish: the entry, lowercase letters, digits, - and _ (e.g. `oak-armchair`).
    name: String,
    /// publish: what changed in this version.
    notes: Option<String>,
    /// publish: how far it can be trusted (what was estimated, what was measured).
    fidelity: Option<String>,
}

/// The library, when this process may read it: a server confined to its
/// projects' folder (the hosted service) keeps no library of its own.
pub(crate) fn root() -> Result<std::path::PathBuf, String> {
    #[cfg(test)]
    if let Some(dir) = TEST_ROOT.with(|root| root.borrow().clone()) {
        return Ok(dir);
    }
    let dir = library::dir();
    if !cfg!(target_arch = "wasm32") && newera_core::vfs::allowed(&dir.join("x")) {
        Ok(dir)
    } else {
        Err("the model library is on the desktop editor only".to_owned())
    }
}

/// A `library:<name>@<v>` model: the version's entry and its model file.
pub(crate) fn resolve(model: &str) -> Option<Result<(Entry, String), String>> {
    let (name, version) = library::parse_ref(model)?;
    Some(root().and_then(|root| {
        library::get(&root, name, version).map(|(entry, path)| (entry, path.display().to_string()))
    }))
}

#[tool_router(router = library_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[allow(clippy::unused_self)] // tool methods need the receiver
    #[tool(
        description = "Versions kept in the asset library: approved revisions of model files, reusable in any project with place model=library:<name> or library:<name>@<v>; edit_model replace file=library:<name>@<v> goes back to an older one. Rows [name, latest, title, brand, versions]; name=<entry> lists its versions [v, title, size, unit, source, fidelity, notes, pictures, sizes]. edit_library publishes."
    )]
    pub(crate) fn library(
        &self,
        Parameters(p): Parameters<LibraryParams>,
    ) -> Result<String, ErrorData> {
        let root = root().map_err(invalid)?;
        let all = library::list(&root);
        match p.name {
            None => Ok(json!({
                "rows": all
                    .iter()
                    .map(|(name, versions)| {
                        let last = versions.last().expect("an entry has a version");
                        json!([name, last.version, last.title, last.brand, versions.len()])
                    })
                    .collect::<Vec<_>>(),
            })
            .to_string()),
            Some(name) => {
                let versions = all
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v)
                    .ok_or_else(|| invalid(format!("no entry `{name}` in the library")))?;
                Ok(json!({
                    "name": name,
                    "versions": versions
                        .iter()
                        .map(|e| json!([e.version, e.title, e.size, e.unit, e.source, e.fidelity, e.notes, e.references.len(), e.measures.len()]))
                        .collect::<Vec<_>>(),
                })
                .to_string())
            }
        }
    }

    #[tool(
        description = "Publish to the asset library: action=publish keeps a piece's model file — with its companion files, reference pictures, measured sizes, unit, box, brand and source link — as the next version of name, with notes on what changed and fidelity (what was estimated). A version never changes after; library lists them."
    )]
    pub(crate) fn edit_library(
        &self,
        Parameters(p): Parameters<EditLibraryParams>,
    ) -> Result<String, ErrorData> {
        if p.action != "publish" {
            return Err(invalid(format!("action `{}`: publish", p.action)));
        }
        let root = root().map_err(invalid)?;
        let doc = self.document.read();
        let id: newera_core::FurnitureId = p.id.parse().map_err(|e| invalid(format!("id: {e}")))?;
        let piece = doc
            .home()
            .find_piece(id)
            .ok_or_else(|| invalid(format!("{} not found", p.id)))?;
        let model = piece.model.as_deref().ok_or_else(|| {
            invalid(format!(
                "{} is built from the catalog, not an imported model",
                p.id
            ))
        })?;
        let references = piece
            .references
            .iter()
            .map(|r| newera_core::Reference {
                file: doc.resolve_asset(&r.file).display().to_string(),
                ..r.clone()
            })
            .collect();
        let entry = Entry {
            name: p.name.clone(),
            title: piece
                .info
                .model_name
                .clone()
                .or_else(|| Some(piece.name.clone())),
            brand: piece.info.brand.clone(),
            source: piece.info.url.clone(),
            unit: piece.properties.get(crate::edit::MODEL_UNIT_KEY).cloned(),
            size: [piece.width, piece.depth, piece.height],
            fidelity: p.fidelity,
            notes: p.notes,
            references,
            measures: piece.measures.clone(),
            published_ms: newera_core::collab::now_ms(),
            // What was done to the file is part of what was approved.
            materials: piece
                .materials
                .iter()
                .map(|m| {
                    let mut m = m.clone();
                    if let Some(image) = m.texture.as_mut().and_then(|t| t.image.as_mut()) {
                        *image = doc.resolve_asset(image).display().to_string();
                    }
                    m
                })
                .collect(),
            model_parts: piece.model_parts.clone(),
            model_transform: (piece.model_transform != newera_core::ModelTransform::default())
                .then(|| piece.model_transform.clone()),
            model_far: piece.model_far.as_ref().map(|far| newera_core::FarModel {
                file: doc.resolve_asset(&far.file).display().to_string(),
                ..far.clone()
            }),
            ..Entry::default()
        };
        let published =
            library::publish(&root, &doc.resolve_asset(model), entry).map_err(invalid)?;
        Ok(format!(
            "ok published library:{}@{} ({} photos, {} measures)",
            published.name,
            published.version,
            published.references.len(),
            published.measures.len()
        ))
    }
}

#[cfg(test)]
thread_local! {
    /// A library of the test's own, so tests running side by side don't share one.
    static TEST_ROOT: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::server;

    #[test]
    fn a_published_chair_goes_into_another_project_and_back_a_version() {
        let dir = std::env::temp_dir().join(format!("newera-library-tool-{}", std::process::id()));
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        TEST_ROOT.with(|root| *root.borrow_mut() = Some(dir.join("library")));
        let obj = |w: f64| format!("v 0 0 0\nv {w} 0 0\nv {w} 90 0\nv 0 0 85\nf 1 2 3\nf 1 3 4\n");
        std::fs::write(work.join("win.obj"), obj(80.0)).unwrap();
        std::fs::write(work.join("frente.png"), b"png").unwrap();
        // Paths inside JSON: `\\` would escape, `/` works everywhere.
        let file = work
            .join("win.obj")
            .display()
            .to_string()
            .replace('\\', "/");
        let photo = work
            .join("frente.png")
            .display()
            .to_string()
            .replace('\\', "/");

        // Checked in one project, and published.
        let first = server();
        first
            .place(Parameters(
                serde_json::from_str(&format!(
                    r#"{{"items":[{{"model":"{file}","at":[0,0],"unit":"cm"}}]}}"#
                ))
                .unwrap(),
            ))
            .unwrap();
        first
            .update(Parameters(
                serde_json::from_str(r#"{"items":[{"id":"f1","brand":"Example Furniture","model_name":"Poltrona Win","url":"https://example.com/win"}]}"#)
                    .unwrap(),
            ))
            .unwrap();
        let edit =
            |json: String| first.edit_model(Parameters(serde_json::from_str(&json).unwrap()));
        edit(format!(
            r#"{{"action":"reference","ids":["f1"],"file":"{photo}","view":"front"}}"#
        ))
        .unwrap();
        edit(
            r#"{"action":"measure","ids":["f1"],"dimension":"height","cm":90,"confirmed":true}"#
                .to_owned(),
        )
        .unwrap();
        // What was done to the file: a part hidden, a weave repeated, a
        // lighter file for afar.
        std::fs::write(work.join("win-longe.obj"), obj(80.0)).unwrap();
        {
            let mut doc = first.document.write();
            let mut chair = doc
                .home()
                .find_piece("f1".parse().unwrap())
                .unwrap()
                .clone();
            chair.model_parts.push(newera_core::ModelPart {
                name: "braco".into(),
                hidden: true,
                ..newera_core::ModelPart::default()
            });
            chair.materials.push(newera_core::ModelMaterial {
                name: "tecido".into(),
                key: None,
                color: Some([90, 60, 40]),
                texture: None,
                shininess: None,
                repeat: Some(2.0),
            });
            chair.model_far = Some(newera_core::FarModel {
                file: work.join("win-longe.obj").display().to_string(),
                beyond: 500.0,
                off: false,
            });
            doc.execute(newera_core::Command::update(chair)).unwrap();
        }
        let publish = |notes: &str| {
            first
                .edit_library(Parameters(
                    serde_json::from_str(&format!(
                        r#"{{"action":"publish","id":"f1","name":"win","notes":"{notes}","fidelity":"curvas estimadas"}}"#
                    ))
                    .unwrap(),
                ))
                .unwrap()
        };
        assert_eq!(
            publish("primeira"),
            "ok published library:win@1 (1 photos, 1 measures)"
        );
        std::fs::write(work.join("win.obj"), obj(70.0)).unwrap();
        assert!(publish("braços mais finos").starts_with("ok published library:win@2"));
        // The originals can go: the library keeps its own copies.
        std::fs::remove_dir_all(&work).unwrap();

        let listed: serde_json::Value =
            serde_json::from_str(&first.library(Parameters(LibraryParams::default())).unwrap())
                .unwrap();
        assert_eq!(
            listed["rows"],
            json!([["win", 2, "Poltrona Win", "Example Furniture", 2]])
        );
        let versions: serde_json::Value = serde_json::from_str(
            &first
                .library(Parameters(LibraryParams {
                    name: Some("win".into()),
                }))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(versions["versions"][1][6], "braços mais finos");

        // Another project: the latest version, with its photos and measures.
        let second = server();
        second
            .place(Parameters(
                serde_json::from_str(r#"{"items":[{"model":"library:win","at":[200,100]}]}"#)
                    .unwrap(),
            ))
            .unwrap();
        let piece = |s: &NewEraMcp| {
            let doc = s.document.read();
            doc.home()
                .find_piece("f1".parse().unwrap())
                .unwrap()
                .clone()
        };
        let placed = piece(&second);
        assert!(
            std::path::Path::new(placed.model.as_deref().unwrap()).ends_with("win/v2/win.obj"),
            "{:?}",
            placed.model
        );
        assert_eq!(placed.info.brand.as_deref(), Some("Example Furniture"));
        assert_eq!((placed.references.len(), placed.measures.len()), (1, 1));
        assert!(std::path::Path::new(&placed.references[0].file).is_file());
        // The approved edits come along, the lighter file from the library.
        assert_eq!(
            (placed.model_parts.len(), placed.model_parts[0].hidden),
            (1, true)
        );
        assert_eq!(placed.materials[0].repeat, Some(2.0));
        let far = placed.model_far.as_ref().unwrap();
        assert!(
            std::path::Path::new(&far.file).ends_with("win/v2/longe/win-longe.obj")
                && std::path::Path::new(&far.file).is_file(),
            "{far:?}"
        );
        assert!(
            (placed.width - 80.0).abs() < 1e-6,
            "the published size: {}",
            placed.width
        );
        // Back to the first version, in one step and under the same id.
        let reply = second
            .edit_model(Parameters(
                serde_json::from_str(r#"{"action":"replace","ids":["f1"],"file":"library:win@1"}"#)
                    .unwrap(),
            ))
            .unwrap();
        assert!(
            std::path::Path::new(piece(&second).model.as_deref().unwrap())
                .ends_with("win/v1/win.obj"),
            "{reply}"
        );
        assert!(
            second
                .place(Parameters(
                    serde_json::from_str(r#"{"items":[{"model":"library:win@9","at":[0,0]}]}"#)
                        .unwrap(),
                ))
                .unwrap_err()
                .message
                .contains("v1, v2")
        );
        TEST_ROOT.with(|root| root.borrow_mut().take());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
