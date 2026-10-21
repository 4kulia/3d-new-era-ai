//! Native project file (`.newera`): a ZIP bundle holding versioned JSON
//! (`project.json`) plus the files the project uses (models, textures,
//! images), so it travels as one small file. Asset paths inside are relative
//! to the bundle root.
//!
//! The JSON is compact and deflated, and large JPEG textures are recompressed
//! on the way in: a furnished apartment with a dozen scanned finishes went
//! from 12 MB to under 3 MB. Plain JSON files (older saves) still open.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::{Document, FIRST_VARIANT_NAME};
use crate::home::Home;

pub const PROJECT_EXTENSION: &str = "newera";
const FORMAT: &str = "3d-new-era-ai";
/// 1: a single `home`. 2: `variants` (versions of the plan) and `active`.
const VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("not a 3D New Era AI project")]
    WrongFormat,
    #[error("project version {0} is newer than this app supports ({VERSION})")]
    TooNew(u32),
    #[error("invalid project file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid project bundle: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{}", crate::progress::CANCELLED)]
    Cancelled,
}

const BUNDLE_JSON: &str = "project.json";

#[derive(Serialize)]
struct VariantOut<'a> {
    name: &'a str,
    home: &'a Home,
}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'a str,
    version: u32,
    active: usize,
    variants: Vec<VariantOut<'a>>,
}

#[derive(Deserialize)]
struct VariantIn {
    name: String,
    home: Home,
}

#[derive(Deserialize)]
struct FileIn {
    format: Option<String>,
    version: Option<u32>,
    #[serde(default)]
    active: usize,
    #[serde(default)]
    variants: Vec<VariantIn>,
    /// Version 1 files.
    home: Option<Home>,
}

/// A project read from disk: its variants and which one is active.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub variants: Vec<(String, Home)>,
    pub active: usize,
}

impl Project {
    /// Replaces a document's content with this project.
    pub fn load_into(self, doc: &mut Document) {
        doc.load_variants(self.variants, self.active);
    }
}

/// Serializes every variant of a document as a project file.
///
/// # Panics
///
/// Never in practice: every [`Home`] is representable as JSON.
pub fn to_project_json(doc: &Document) -> String {
    let file = FileOut {
        format: FORMAT,
        version: VERSION,
        active: doc.active_variant(),
        variants: doc
            .variants()
            .map(|v| VariantOut {
                name: &v.name,
                home: v.home(),
            })
            .collect(),
    };
    serde_json::to_string(&file).expect("home is always serializable")
}

pub fn from_project_json(json: &str) -> Result<Project, ProjectError> {
    let file: FileIn = serde_json::from_str(json)?;
    if file.format.as_deref() != Some(FORMAT) {
        return Err(ProjectError::WrongFormat);
    }
    let version = file.version.unwrap_or(0);
    if version > VERSION {
        return Err(ProjectError::TooNew(version));
    }
    let variants: Vec<(String, Home)> = if file.variants.is_empty() {
        vec![(FIRST_VARIANT_NAME.to_owned(), file.home.unwrap_or_default())]
    } else {
        file.variants
            .into_iter()
            .map(|v| (v.name, v.home))
            .collect()
    };
    let active = file.active.min(variants.len() - 1);
    Ok(Project { variants, active })
}

/// Per-user cache directory for unpacked bundles and imports.
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("3d-new-era-ai")
}

/// Resolves a stored asset path against the asset directory.
pub fn resolve_asset(dir: Option<&Path>, stored: &str) -> PathBuf {
    let stored = Path::new(stored);
    match dir {
        Some(dir) if stored.is_relative() => dir.join(stored),
        _ => stored.to_path_buf(),
    }
}

/// Files a model needs next to it: an OBJ's material libraries and the
/// textures they name, a glTF's buffers and images. Names are relative to
/// the model's folder.
pub fn model_companions(model: &Path) -> Vec<String> {
    model_companions_with(model, |p| crate::vfs::read(p).ok())
}

pub(crate) fn model_companions_with(
    model: &Path,
    read: impl Fn(&Path) -> Option<Vec<u8>>,
) -> Vec<String> {
    let ext = model
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Some(dir) = model.parent() else {
        return Vec::new();
    };
    let text = |p: &Path| read(p).map(|b| String::from_utf8_lossy(&b).into_owned());
    let mut out = Vec::new();
    match ext.as_str() {
        "obj" => {
            let Some(obj) = text(model) else { return out };
            for lib in obj.lines().filter_map(|l| l.trim().strip_prefix("mtllib ")) {
                let lib = lib.trim().to_owned();
                if let Some(mtl) = text(&dir.join(&lib)) {
                    for line in mtl.lines() {
                        let line = line.trim();
                        let is_map = ["map_", "bump", "norm", "disp", "refl"]
                            .iter()
                            .any(|k| line.starts_with(k));
                        // The file name is the last token (options come first).
                        if is_map && let Some(file) = line.split_whitespace().last() {
                            out.push(file.to_owned());
                            // The importer also accepts full texture names
                            // containing spaces. Keep that reference as well.
                            if let Some((_, rest)) = line.split_once(char::is_whitespace) {
                                out.push(rest.trim().to_owned());
                            }
                        }
                    }
                }
                out.push(lib);
            }
        }
        "gltf" | "glb" => {
            let Some(bytes) = read(model) else { return out };
            let json = if ext == "glb" {
                // GLB 2: twelve-byte header, then the mandatory JSON chunk.
                if bytes.get(..4) != Some(b"glTF")
                    || bytes.get(4..8) != Some(&2u32.to_le_bytes())
                    || bytes.get(16..20) != Some(b"JSON")
                {
                    return out;
                }
                let len =
                    u32::from_le_bytes(bytes[12..16].try_into().expect("GLB header")) as usize;
                let Some(end) = 20usize.checked_add(len) else {
                    return out;
                };
                let Some(json) = bytes.get(20..end) else {
                    return out;
                };
                json
            } else {
                &bytes
            };
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) {
                for key in ["buffers", "images"] {
                    for item in value[key].as_array().into_iter().flatten() {
                        if let Some(uri) = item["uri"].as_str().filter(|u| !u.starts_with("data:"))
                        {
                            out.push(uri.to_owned());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Files referenced by every variant, resolved on disk, with the name each
/// gets inside a bundle. Models bring the material libraries, textures and
/// buffers they reference, kept at the same place next to them.
fn bundle_files(doc: &Document) -> Vec<(PathBuf, String)> {
    let dir = doc.asset_dir();
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut push = |source: PathBuf, name: String| {
        if !out.iter().any(|(_, n)| *n == name) {
            out.push((source, name));
        }
    };
    for variant in doc.variants() {
        for stored in variant.home().asset_paths() {
            let source = resolve_asset(dir.as_deref(), &stored);
            let name = bundle_name(&stored);
            let folder = Path::new(&name)
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            for companion in model_companions(&source) {
                let relative = Path::new(&companion);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    continue;
                }
                let inside = if folder.is_empty() {
                    companion.replace('\\', "/")
                } else {
                    format!("{folder}/{}", companion.replace('\\', "/"))
                };
                if let Some(source_dir) = source.parent() {
                    push(source_dir.join(&companion), inside);
                }
            }
            push(source, name);
        }
    }
    out
}

/// Name of an asset inside a bundle: relative paths stay (normalized to
/// `/`); absolute or escaping paths go to `external/<hash>/<file>`.
fn bundle_name(stored: &str) -> String {
    use std::hash::{Hash, Hasher};
    let path = Path::new(stored);
    let escapes = path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    if !escapes {
        return path
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.parent().hash(&mut hasher);
    let file = path
        .file_name()
        .map_or_else(|| "asset".to_owned(), |f| f.to_string_lossy().into_owned());
    format!("external/{:016x}/{file}", hasher.finish())
}

/// JPEG files at least this big are worth trying to recompress.
const RECOMPRESS_FROM: usize = 256 * 1024;
/// Longest side kept for a bundled JPEG: plenty for a texture tile or a scan.
const MAX_IMAGE_SIDE: u32 = 2048;
/// Quality of recompressed JPEGs: no visible change on finishes and scans.
const JPEG_QUALITY: u8 = 85;

/// Bytes stored for an asset: big JPEGs recompressed (and capped in size)
/// when that saves at least a quarter, everything else as it is. A file
/// already recompressed doesn't shrink enough to be touched again, so saving
/// repeatedly doesn't wear the image down.
fn packed_asset(name: &str, bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() < RECOMPRESS_FROM || !has_extension(name, &["jpg", "jpeg"]) {
        return bytes;
    }
    let Ok(image) = crate::images::decode(&bytes) else {
        return bytes;
    };
    let image = if image.width().max(image.height()) > MAX_IMAGE_SIDE {
        image.resize(
            MAX_IMAGE_SIDE,
            MAX_IMAGE_SIDE,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    if image.to_rgb8().write_with_encoder(encoder).is_err() || out.len() * 4 > bytes.len() * 3 {
        return bytes;
    }
    out
}

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Formats that are already compressed: deflating them again only costs time.
fn already_compressed(name: &str) -> bool {
    has_extension(
        name,
        &["jpg", "jpeg", "png", "webp", "glb", "zip", "mp4", "avi"],
    )
}

/// A project as the bytes of a `.newera` file with no bundled files: for
/// saving where there is no file system (the browser).
///
/// # Panics
///
/// Never in practice: writing a ZIP into memory cannot fail.
pub fn to_project_bytes(doc: &Document) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(BUNDLE_JSON, deflated())
        .expect("zip in memory");
    std::io::Write::write_all(&mut zip, to_project_json(doc).as_bytes()).expect("zip in memory");
    zip.finish().expect("zip in memory").into_inner()
}

fn deflated() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9))
}

/// Saves a project as a bundle with every referenced file.
///
/// The document's asset paths are left untouched; the bundle stores them
/// renamed where needed.
pub fn save_project(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    let files = bundle_files(doc);
    // The bundle's own JSON counts as one part, then a part per file.
    let parts = files.len() as u64 + 1;
    crate::progress::step("Gravando o projeto", 0, parts);
    // Rewrite escaping paths to their bundle names in a copy.
    let mut copy = Document::default();
    let variants: Vec<(String, Home)> = doc
        .variants()
        .map(|v| {
            let mut home = v.home().clone();
            home.for_each_asset_mut(&mut |p| *p = bundle_name(p));
            (v.name.clone(), home)
        })
        .collect();
    copy.load_variants(variants, doc.active_variant());

    let tmp = path.with_extension("newera.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        zip.start_file(BUNDLE_JSON, deflated())?;
        std::io::Write::write_all(&mut zip, to_project_json(&copy).as_bytes())?;
        for (done, (source, name)) in files.into_iter().enumerate() {
            crate::progress::step("Guardando imagens e modelos", done as u64 + 1, parts);
            if crate::progress::cancelled() {
                // Nothing is renamed into place, so the saved file is
                // whatever it was before this started.
                drop(zip);
                let _ = std::fs::remove_file(&tmp);
                return Err(ProjectError::Cancelled);
            }
            match crate::vfs::read(&source) {
                Ok(bytes) => {
                    let options = if already_compressed(&name) {
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored)
                    } else {
                        deflated()
                    };
                    let bytes = packed_asset(&name, bytes);
                    zip.start_file(name, options)?;
                    std::io::Write::write_all(&mut zip, &bytes)?;
                }
                // A missing file is kept as a dangling reference rather than
                // losing the whole save.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        zip.finish()?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Files packed in a bundle, by name.
pub type BundledFiles = Vec<(String, Vec<u8>)>;

/// Reads a project from memory (JSON or bundle). Bundled asset files are
/// returned by name, for environments without a file system.
pub fn project_from_bytes(bytes: &[u8]) -> Result<(Project, BundledFiles), ProjectError> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return Ok((
            from_project_json(&String::from_utf8_lossy(bytes))?,
            Vec::new(),
        ));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut json = String::new();
    std::io::Read::read_to_string(&mut archive.by_name(BUNDLE_JSON)?, &mut json)?;
    let project = from_project_json(&json)?;
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name()?.into_owned();
        if file.is_dir() || name == BUNDLE_JSON {
            continue;
        }
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut data)?;
        files.push((name, data));
    }
    Ok((project, files))
}

/// Opens a project file (JSON or bundle). Bundles are unpacked under
/// `cache`; the returned directory is where their assets live.
pub fn open_project(path: &Path, cache: &Path) -> Result<(Project, Option<PathBuf>), ProjectError> {
    use std::hash::{Hash, Hasher};
    crate::progress::step("Lendo o arquivo", 0, 0);
    let bytes = std::fs::read(path)?;
    if !bytes.starts_with(b"PK\x03\x04") {
        let json = String::from_utf8_lossy(&bytes);
        return Ok((from_project_json(&json)?, None));
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    bytes.len().hash(&mut hasher);
    bytes.hash(&mut hasher);
    let stem = path.file_stem().map_or_else(
        || "project".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    let dir = cache.join(format!("{stem}-{:016x}", hasher.finish()));

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut json = String::new();
    std::io::Read::read_to_string(&mut archive.by_name(BUNDLE_JSON)?, &mut json)?;
    let project = from_project_json(&json)?;
    let parts = archive.len() as u64;
    for i in 0..archive.len() {
        crate::progress::step("Abrindo imagens e modelos", i as u64, parts);
        if crate::progress::cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let mut file = archive.by_index(i)?;
        let Some(name) = file.enclosed_name() else {
            continue;
        };
        if file.is_dir() || name == Path::new(BUNDLE_JSON) {
            continue;
        }
        let target = dir.join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(target)?;
        std::io::copy(&mut file, &mut out)?;
    }
    Ok((project, Some(dir)))
}

/// Resolves a path stored in a project relative to the project file.
pub fn resolve_project_path(project: Option<&Path>, stored: &str) -> PathBuf {
    let stored = Path::new(stored);
    match project.and_then(Path::parent) {
        Some(dir) if stored.is_relative() => dir.join(stored),
        _ => stored.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Point2, Wall};

    #[test]
    fn round_trips_all_variants_and_keeps_id_counters() {
        let mut doc = Document::default();
        let wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(300.0, 0.0),
        );
        doc.execute(Command::insert(wall)).unwrap();
        doc.add_variant(Some("Sem parede".into()), false);
        let json = to_project_json(&doc);
        assert!(json.contains("\"format\":\"3d-new-era-ai\""));
        assert!(json.contains("\"version\":2"));

        let project = from_project_json(&json).unwrap();
        assert_eq!(project.active, 1);
        assert_eq!(project.variants.len(), 2);
        assert_eq!(project.variants[1].0, "Sem parede");
        let mut loaded = Document::default();
        project.load_into(&mut loaded);
        loaded.switch_variant(0).unwrap();
        assert_eq!(loaded.home().walls.len(), 1);
        assert_eq!(loaded.new_wall_id().0, 2, "counter survives saving");
    }

    #[test]
    fn reads_version_1_files_as_a_single_variant() {
        let v1 = r#"{"format":"3d-new-era-ai","version":1,"home":{"name":"Antiga"}}"#;
        let project = from_project_json(v1).unwrap();
        assert_eq!(project.variants.len(), 1);
        assert_eq!(project.variants[0].0, FIRST_VARIANT_NAME);
        assert_eq!(project.variants[0].1.name, "Antiga");
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert!(matches!(
            from_project_json(r#"{"a":1}"#),
            Err(ProjectError::WrongFormat)
        ));
        let future = r#"{"format":"3d-new-era-ai","version":99,"home":{"name":"x"}}"#;
        assert!(matches!(
            from_project_json(future),
            Err(ProjectError::TooNew(99))
        ));
    }

    #[test]
    fn resolves_relative_paths_next_to_the_project() {
        let p = resolve_project_path(Some(Path::new("/tmp/casa/projeto.newera")), "planta.png");
        assert_eq!(p, PathBuf::from("/tmp/casa/planta.png"));
        let abs = resolve_project_path(Some(Path::new("/tmp/casa/projeto.newera")), "/img/a.png");
        assert_eq!(abs, PathBuf::from("/img/a.png"));
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn projects_with_files_save_as_bundles_and_reopen() {
        let root = std::env::temp_dir().join(format!("newera-bundle-{}", std::process::id()));
        let assets = root.join("assets");
        std::fs::create_dir_all(assets.join("models/chair")).unwrap();
        std::fs::write(
            assets.join("models/chair/chair.obj"),
            "mtllib chair.mtl\nv 0 0 0\n",
        )
        .unwrap();
        std::fs::write(
            assets.join("models/chair/chair.mtl"),
            "newmtl a\nmap_Kd -s 1 1 1 wood.png\n",
        )
        .unwrap();
        std::fs::write(
            assets.join("models/chair/wood.png"),
            [0x89, b'P', b'N', b'G'],
        )
        .unwrap();
        std::fs::write(
            assets.join("models/chair/unrelated.txt"),
            "not part of the model",
        )
        .unwrap();
        std::fs::write(root.join("neighbor.jpg"), [0xFF, 0xD8]).unwrap();
        let outside = root.join("outside.png");
        std::fs::write(&outside, [0x89, b'P', b'N', b'G']).unwrap();

        let mut home = Home::default();
        let mut piece = crate::Furniture {
            model: Some("models/chair/chair.obj".into()),
            ..crate::Furniture::default()
        };
        piece.id = home.new_furniture_id();
        home.furniture.push(piece);
        let mut room = crate::Room::new(
            home.new_room_id(),
            "Sala",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(100.0, 0.0),
                Point2::new(0.0, 100.0),
            ],
        );
        room.floor_material = Some(crate::Material {
            image: Some(outside.display().to_string()),
            ..crate::Material::default()
        });
        home.rooms.push(room);
        let mut doc = Document::new(home);
        doc.set_asset_dir(Some(assets.clone()));

        let file = root.join("casa.newera");
        save_project(&doc, &file).unwrap();
        let (project, dir) = open_project(&file, &root.join("cache")).unwrap();
        let dir = dir.expect("bundle unpacks");
        let home = &project.variants[0].1;
        assert!(
            dir.join("models/chair/chair.mtl").exists()
                && dir.join("models/chair/wood.png").exists(),
            "the model's materials and textures travel"
        );
        assert!(
            !dir.join("models/chair/unrelated.txt").exists(),
            "unrelated files stay out"
        );
        let external: Vec<_> = walk(&dir.join("external"));
        assert_eq!(
            external.len(),
            1,
            "only the referenced image, not its folder: {external:?}"
        );
        let floor = home.rooms[0]
            .floor_material
            .as_ref()
            .unwrap()
            .image
            .clone()
            .unwrap();
        assert!(floor.starts_with("external/"), "{floor}");
        assert!(dir.join(&floor).exists());

        // No files: still a small bundle, and older plain JSON files open.
        let empty = root.join("empty.newera");
        save_project(&Document::default(), &empty).unwrap();
        assert!(std::fs::read(&empty).unwrap().starts_with(b"PK\x03\x04"));
        assert!(open_project(&empty, &root.join("cache")).is_ok());
        let plain = root.join("plain.newera");
        std::fs::write(&plain, to_project_json(&Document::default())).unwrap();
        assert!(open_project(&plain, &root).unwrap().1.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn noisy_jpeg(w: u32, h: u32, quality: u8) -> Vec<u8> {
        // Pseudo-random texture: the kind of detail scans have.
        let mut seed = 12_345_u32;
        let image = image::RgbImage::from_fn(w, h, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let n = seed.to_be_bytes()[0];
            let (x, y) = (x.to_le_bytes()[0], y.to_le_bytes()[0]);
            image::Rgb([n, x / 2 + n / 2, y / 2 + n / 2])
        });
        let mut out = Vec::new();
        image
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out, quality,
            ))
            .unwrap();
        out
    }

    #[test]
    fn big_jpegs_are_recompressed_once_and_the_rest_kept_as_is() {
        let heavy = noisy_jpeg(1024, 1024, 100);
        assert!(heavy.len() >= RECOMPRESS_FROM, "{}", heavy.len());
        let packed = packed_asset("tex/Wood.JPG", heavy.clone());
        assert!(
            packed.len() * 4 <= heavy.len() * 3,
            "{} -> {}",
            heavy.len(),
            packed.len()
        );
        let decoded = image::load_from_memory(&packed).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 1024));
        // Saving again leaves it alone: no generation loss.
        assert_eq!(packed_asset("tex/Wood.JPG", packed.clone()), packed);

        let huge = noisy_jpeg(4096, 2048, 95);
        let decoded = image::load_from_memory(&packed_asset("scan.jpeg", huge)).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (MAX_IMAGE_SIDE, MAX_IMAGE_SIDE / 2)
        );

        assert_eq!(
            packed_asset("model.png", heavy.clone()),
            heavy,
            "PNGs stay lossless"
        );
        let broken = vec![0xFF_u8; RECOMPRESS_FROM + 1];
        assert_eq!(packed_asset("broken.jpg", broken.clone()), broken);
    }

    #[test]
    fn project_bytes_open_back() {
        let mut doc = Document::default();
        let wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(300.0, 0.0),
        );
        doc.execute(Command::insert(wall)).unwrap();
        let bytes = to_project_bytes(&doc);
        assert!(bytes.len() < to_project_json(&doc).len());
        let (project, files) = project_from_bytes(&bytes).unwrap();
        assert_eq!(files, [] as [(std::string::String, std::vec::Vec<u8>); 0]);
        assert_eq!(project.variants[0].1.walls.len(), 1);
    }
}
