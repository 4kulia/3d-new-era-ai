//! A library of imported models kept apart from any project, version by
//! version: the approved revision of a chair — its files, the photos and
//! measurements it was checked against, the unit, where it came from and
//! how faithful it is — published once and placed in any project after.
//!
//! On disk, `<library>/<name>/v<N>/` holds the model with its companion
//! files and an `asset.json`. A version is never changed once published: a
//! new revision is a new version, and going back is placing an older one.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::furniture::{FarModel, Measure, ModelMaterial, ModelPart, ModelTransform, Reference};

/// What a version says about itself, in `asset.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub version: u32,
    /// The model file, relative to the version's folder.
    pub file: String,
    /// Commercial name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Product page or catalog it reproduces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Unit of the model file's numbers, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Size it is placed at, `[w, d, h]` cm.
    pub size: [f64; 3],
    /// How far it can be trusted: what was estimated, what was measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fidelity: Option<String>,
    /// What changed in this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Photos, relative to the version's folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<Measure>,
    /// When it was published, ms since the epoch.
    #[serde(default)]
    pub published_ms: u64,
    /// The piece's own changes to the file, part of what was approved:
    /// materials (images relative to the version's folder), parts, the
    /// turn it is fitted with and its lighter file for afar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<ModelMaterial>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_parts: Vec<ModelPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_transform: Option<ModelTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_far: Option<FarModel>,
}

/// Where the library lives: `NEWERA_LIBRARY`, or `<config>/3d-new-era-ai/library`.
#[must_use]
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NEWERA_LIBRARY") {
        return PathBuf::from(dir);
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(crate::project::fallback_dir)
        .join("3d-new-era-ai")
        .join("library")
}

/// Whether `name` can name an entry: lowercase letters, digits, `-`, `_`.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// `library:<name>` or `library:<name>@<version>`, as `place` and
/// `edit_model` take it.
#[must_use]
pub fn parse_ref(text: &str) -> Option<(&str, Option<u32>)> {
    let rest = text.strip_prefix("library:")?;
    match rest.split_once('@') {
        Some((name, v)) => Some((name, Some(v.trim_start_matches('v').parse().ok()?))),
        None => Some((rest, None)),
    }
}

fn versions_of(root: &Path, name: &str) -> Vec<u32> {
    let mut out: Vec<u32> = std::fs::read_dir(root.join(name))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix('v')?.parse().ok())
        .collect();
    out.sort_unstable();
    out
}

/// A version of an entry (the latest when `version` is `None`), and the
/// path of its model file.
///
/// # Errors
/// When there is no such entry or version, or its `asset.json` is unreadable.
pub fn get(root: &Path, name: &str, version: Option<u32>) -> Result<(Entry, PathBuf), String> {
    let versions = versions_of(root, name);
    let version = match version {
        Some(v) if versions.contains(&v) => v,
        Some(v) => {
            return Err(format!(
                "library: {name} has no version {v} ({})",
                versions
                    .iter()
                    .map(|v| format!("v{v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        None => *versions
            .last()
            .ok_or_else(|| format!("library: no entry `{name}` (`library` lists them)"))?,
    };
    let folder = root.join(name).join(format!("v{version}"));
    let text = std::fs::read_to_string(folder.join("asset.json"))
        .map_err(|e| format!("library: {name} v{version}: {e}"))?;
    let mut entry: Entry =
        serde_json::from_str(&text).map_err(|e| format!("library: {name} v{version}: {e}"))?;
    let model = folder.join(&entry.file);
    for reference in &mut entry.references {
        reference.file = folder.join(&reference.file).display().to_string();
    }
    for image in entry
        .materials
        .iter_mut()
        .filter_map(|m| m.texture.as_mut()?.image.as_mut())
    {
        *image = folder.join(&*image).display().to_string();
    }
    if let Some(far) = &mut entry.model_far {
        far.file = folder.join(&far.file).display().to_string();
    }
    Ok((entry, model))
}

/// Every entry with its versions, by name.
#[must_use]
pub fn list(root: &Path) -> Vec<(String, Vec<Entry>)> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| valid_name(n))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let entries: Vec<Entry> = versions_of(root, &name)
                .into_iter()
                .filter_map(|v| get(root, &name, Some(v)).ok().map(|(e, _)| e))
                .collect();
            (!entries.is_empty()).then_some((name, entries))
        })
        .collect()
}

/// A relative path that stays inside the folder it is joined to.
fn inside(relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then(|| path.to_path_buf())
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let bytes = crate::vfs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(to, bytes).map_err(|e| format!("{}: {e}", to.display()))
}

/// Copies `model` as `into/file_name`, with the files it names beside it.
fn copy_model(model: &Path, into: &Path, file_name: &str) -> Result<(), String> {
    copy(model, &into.join(file_name))?;
    if let Some(dir) = model.parent() {
        for companion in crate::model_companions(model) {
            // A companion that would land outside the version is left out.
            if let Some(relative) = inside(&companion)
                && crate::vfs::exists(&dir.join(&relative))
            {
                copy(&dir.join(&relative), &into.join(&relative))?;
            }
        }
    }
    Ok(())
}

/// Creates the first free `v<N>` folder from `first` on and returns it: made
/// by this call alone, so two publishing at once never share (or delete)
/// one version.
fn claim(dir: &Path, first: u32) -> Result<(u32, PathBuf), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut version = first;
    loop {
        let folder = dir.join(format!("v{version}"));
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok((version, folder)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => version += 1,
            Err(e) => return Err(format!("{}: {e}", folder.display())),
        }
    }
}

/// Publishes `model` (with the files it names) and the photos of
/// `entry.references` as the next version of `entry.name`. The photos'
/// `file` are read as given and stored as copies in the version.
///
/// # Errors
/// On an invalid name, or a file that cannot be read or written; nothing
/// is left behind of a version that failed.
pub fn publish(root: &Path, model: &Path, mut entry: Entry) -> Result<Entry, String> {
    if !valid_name(&entry.name) {
        return Err(format!(
            "library: name `{}`: lowercase letters, digits, - and _",
            entry.name
        ));
    }
    let first = versions_of(root, &entry.name).last().map_or(1, |v| v + 1);
    let (version, folder) = claim(&root.join(&entry.name), first)?;
    let result = (|| {
        let file_name = model
            .file_name()
            .ok_or_else(|| format!("library: {} is not a file", model.display()))?
            .to_string_lossy()
            .into_owned();
        copy_model(model, &folder, &file_name)?;
        if let Some(far) = &mut entry.model_far {
            let from = PathBuf::from(&far.file);
            let name = format!(
                "longe/{}",
                from.file_name()
                    .map_or_else(|| "modelo".into(), |n| n.to_string_lossy().into_owned())
            );
            copy_model(&from, &folder.join("longe"), &name["longe/".len()..])?;
            far.file = name;
        }
        for (k, image) in entry
            .materials
            .iter_mut()
            .filter_map(|m| m.texture.as_mut()?.image.as_mut())
            .enumerate()
        {
            let from = PathBuf::from(&*image);
            let name = format!(
                "imagens/{k:02}-{}",
                from.file_name()
                    .map_or_else(|| "imagem".into(), |n| n.to_string_lossy().into_owned())
            );
            copy(&from, &folder.join(&name))?;
            *image = name;
        }
        for (k, reference) in entry.references.iter_mut().enumerate() {
            let from = PathBuf::from(&reference.file);
            let name = format!(
                "referencias/{k:02}-{}",
                from.file_name()
                    .map_or_else(|| "foto".into(), |n| n.to_string_lossy().into_owned())
            );
            copy(&from, &folder.join(&name))?;
            reference.file = name;
        }
        entry.file = file_name;
        entry.version = version;
        let text = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
        std::fs::write(folder.join("asset.json"), text).map_err(|e| e.to_string())?;
        Ok(entry)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&folder);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_taken_meanwhile_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("newera-library-claim-{}", std::process::id()));
        // Another publisher claimed v3 after this one counted v2 as the last.
        std::fs::create_dir_all(dir.join("v3")).unwrap();
        std::fs::write(dir.join("v3/asset.json"), "theirs").unwrap();
        let (version, folder) = claim(&dir, 3).unwrap();
        assert_eq!((version, folder), (4, dir.join("v4")));
        assert_eq!(
            std::fs::read_to_string(dir.join("v3/asset.json")).unwrap(),
            "theirs"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn versions_are_published_side_by_side_and_read_back() {
        let root = std::env::temp_dir().join(format!("newera-library-{}", std::process::id()));
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            work.join("win.obj"),
            "mtllib win.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        )
        .unwrap();
        std::fs::write(
            work.join("win.mtl"),
            "newmtl tecido\nmap_Kd tex/trama.png\n",
        )
        .unwrap();
        std::fs::create_dir_all(work.join("tex")).unwrap();
        std::fs::write(work.join("tex/trama.png"), b"png").unwrap();
        std::fs::write(work.join("frente.jpg"), b"jpg").unwrap();
        let library = root.join("library");
        let entry = |notes: &str| Entry {
            name: "win".into(),
            title: Some("Poltrona Win".into()),
            size: [80.0, 85.0, 90.0],
            notes: Some(notes.into()),
            references: vec![Reference {
                file: work.join("frente.jpg").display().to_string(),
                view: Some("front".into()),
                ..Reference::default()
            }],
            ..Entry::default()
        };
        let first = publish(&library, &work.join("win.obj"), entry("primeira")).unwrap();
        assert_eq!(first.version, 1);
        let second = publish(&library, &work.join("win.obj"), entry("braços revistos")).unwrap();
        assert_eq!(second.version, 2);
        let folder = library.join("win/v2");
        for file in [
            "win.obj",
            "win.mtl",
            "tex/trama.png",
            "referencias/00-frente.jpg",
            "asset.json",
        ] {
            assert!(folder.join(file).is_file(), "{file}");
        }
        let (latest, model) = get(&library, "win", None).unwrap();
        assert_eq!((latest.version, model), (2, folder.join("win.obj")));
        assert_eq!(
            Path::new(&latest.references[0].file),
            folder.join("referencias").join("00-frente.jpg")
        );
        let (older, _) = get(&library, "win", Some(1)).unwrap();
        assert_eq!(older.notes.as_deref(), Some("primeira"));
        assert!(
            get(&library, "win", Some(7))
                .unwrap_err()
                .contains("v1, v2")
        );
        assert!(get(&library, "sofa", None).is_err());
        let all = list(&library);
        assert_eq!((all.len(), all[0].1.len()), (1, 2));
        assert!(
            publish(
                &library,
                &work.join("win.obj"),
                Entry {
                    name: "Win!".into(),
                    ..Entry::default()
                }
            )
            .is_err()
        );
        // A version that fails leaves nothing behind.
        assert!(publish(&library, &work.join("missing.obj"), entry("x")).is_err());
        assert!(!library.join("win/v3").exists());
        assert_eq!(parse_ref("library:win@2"), Some(("win", Some(2))));
        assert_eq!(parse_ref("library:win"), Some(("win", None)));
        assert_eq!(parse_ref("win.obj"), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
