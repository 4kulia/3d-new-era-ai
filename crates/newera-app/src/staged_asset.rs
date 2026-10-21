//! A file the hosted service hands the tab before a call that uses it (the
//! relay's `asset` message): a shop product's model, kept in memory where the
//! project's own files are, so the call that places it finds it and the next
//! save carries it to the service inside the project.

use std::path::{Component, Path, PathBuf};

use base64::Engine as _;

/// Keeps the file `message` carries — `{"path", "data"}`, the bytes in
/// standard base64 — under `dir`, where the document's relative paths
/// resolve (`Document::asset_dir`; none: as given). A `replace` — a model
/// attached after it was placed — is kept only when `uses` says the
/// document draws something with that path. Answers what the tab tells the
/// service: `{"path", "kept"}`.
///
/// # Errors
///
/// When the message carries no path, a path that would leave the project's
/// files, or data that is not base64.
pub(crate) fn keep(
    message: &serde_json::Value,
    dir: Option<PathBuf>,
    uses: impl Fn(&str) -> bool,
) -> Result<serde_json::Value, String> {
    let path = message["path"]
        .as_str()
        .filter(|p| !p.is_empty())
        .ok_or("the file came without a path")?;
    if !Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(format!("{path} is not a path inside the project"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(message["data"].as_str().unwrap_or_default())
        .map_err(|e| format!("the file {path} is not base64: {e}"))?;
    if message["replace"] == true && !uses(path) {
        return Ok(serde_json::json!({ "path": path, "kept": false }));
    }
    newera_core::vfs::mount(&dir.unwrap_or_default(), [(path.to_owned(), bytes)]);
    Ok(serde_json::json!({ "path": path, "kept": true }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_file_is_kept_where_the_projects_paths_resolve() {
        let dir = std::path::PathBuf::from("/staged-asset-test/assets");
        let answer = super::keep(
            &json!({"type": "asset", "id": 3, "path": "products/p1/model.glb", "data": "Z2xURgIA"}),
            Some(dir.clone()),
            |_| false,
        );
        assert_eq!(
            answer,
            Ok(json!({"path": "products/p1/model.glb", "kept": true}))
        );
        let mut doc = newera_core::Document::default();
        doc.set_asset_dir(Some(dir.clone()));
        assert_eq!(
            newera_core::vfs::read(&doc.resolve_asset("products/p1/model.glb")).unwrap(),
            b"glTF\x02\x00"
        );
        newera_core::vfs::unmount(&dir);
    }

    #[test]
    fn a_document_without_a_folder_reads_it_by_its_path() {
        let path = "products/staged-asset-test/model.glb";
        super::keep(&json!({"path": path, "data": "AQI="}), None, |_| false).unwrap();
        let doc = newera_core::Document::default();
        assert_eq!(
            newera_core::vfs::read(&doc.resolve_asset(path)).unwrap(),
            [1, 2]
        );
        newera_core::vfs::unmount(std::path::Path::new("products/staged-asset-test"));
    }

    /// A replacement — a model attached after it was placed — is kept only
    /// where the document draws something with it.
    #[test]
    fn a_replacement_is_kept_only_where_it_is_used() {
        let dir = std::path::PathBuf::from("/staged-asset-test/replace");
        let message = json!({"path": "products/p2/model.glb", "data": "AQI=", "replace": true});
        assert_eq!(
            super::keep(&message, Some(dir.clone()), |_| false),
            Ok(json!({"path": "products/p2/model.glb", "kept": false}))
        );
        assert!(newera_core::vfs::read(&dir.join("products/p2/model.glb")).is_err());
        assert_eq!(
            super::keep(&message, Some(dir.clone()), |p| p
                == "products/p2/model.glb"),
            Ok(json!({"path": "products/p2/model.glb", "kept": true}))
        );
        assert_eq!(
            newera_core::vfs::read(&dir.join("products/p2/model.glb")).unwrap(),
            [1, 2]
        );
        newera_core::vfs::unmount(&dir);
    }

    #[test]
    fn a_path_out_of_the_project_or_bad_data_is_refused() {
        for (message, why) in [
            (
                json!({"data": "AQI="}),
                "the file came without a path".to_owned(),
            ),
            (
                json!({"path": "../x.glb", "data": "AQI="}),
                "../x.glb is not a path inside the project".to_owned(),
            ),
            (
                json!({"path": "/etc/x.glb", "data": "AQI="}),
                "/etc/x.glb is not a path inside the project".to_owned(),
            ),
        ] {
            assert_eq!(super::keep(&message, None, |_| false), Err(why));
        }
        let bad =
            super::keep(&json!({"path": "a.glb", "data": "%%"}), None, |_| false).unwrap_err();
        assert!(bad.starts_with("the file a.glb is not base64: "), "{bad}");
    }
}
