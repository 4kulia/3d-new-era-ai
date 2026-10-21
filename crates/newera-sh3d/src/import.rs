//! Maps a Sweet Home 3D home onto the New Era model, extracting the models,
//! textures and images it embeds.
//!
//! Every value the source stores is carried over: geometry and looks map to
//! first-class fields, and anything purely informational (original ids,
//! unsupported settings) goes into element `properties` under `sh3d:` keys.

// Source values are Java floats and ints mapped to our types on purpose.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::single_match_else
)]

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use newera_core::{
    ArrowStyle, BackgroundImage, Baseboard, Camera, Cameras, Compass, DashStyle, Dimension,
    DrawingMode, Environment, Furniture, Home, Label, Level, LevelId, Light, LightSource, LineCap,
    LineJoin, Material, ModelMaterial, ModelTransform, Opening, OpeningKind, PaperOrientation,
    PhotoSettings, PieceInfo, PieceLocks, Point2, Polyline, PrintSettings, Properties, Room, Sash,
    TextAlign, TextStyle, VideoSettings, Wall, WallCutOut,
};

use crate::javaser::{self, Entry, Graph, Object, Value};

const MODEL: &str = "com.eteks.sweethome3d.model.";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("not a Sweet Home 3D file: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("the file has no `Home` entry")]
    NoHome,
    #[error("invalid home data: {0}")]
    Stream(#[from] javaser::Error),
    #[error("the home entry does not hold a home")]
    NotAHome,
    #[error("{}", newera_core::progress::CANCELLED)]
    Cancelled,
}

/// Files extracted in memory, by name relative to the assets directory.
pub type BundledFiles = Vec<(String, Vec<u8>)>;

/// Result of an import.
#[derive(Debug)]
pub struct Imported {
    pub home: Home,
    /// What could not be carried over exactly.
    pub warnings: Vec<String>,
}

/// Imports `path`, extracting embedded files under `assets` (created if
/// needed). Asset paths in the returned home are relative to `assets`.
pub fn import_file(path: &Path, assets: &Path) -> Result<Imported, ImportError> {
    let bytes = std::fs::read(path).map_err(|source| ImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned());
    let (imported, _) = import(&bytes, name, assets, false)?;
    Ok(imported)
}

/// Imports a `.sh3d` file held in memory, without touching the disk: embedded
/// files are mounted in [`newera_core::vfs`] under `assets` (any virtual
/// directory) and also returned by name, e.g. to save a bundle later.
pub fn import_bytes(
    bytes: &[u8],
    name: &str,
    assets: &Path,
) -> Result<(Imported, BundledFiles), ImportError> {
    import(bytes, Some(name.to_owned()), assets, true)
}

fn import(
    bytes: &[u8],
    name: Option<String>,
    assets: &Path,
    in_memory: bool,
) -> Result<(Imported, BundledFiles), ImportError> {
    newera_core::progress::step("Lendo o arquivo", 0, 0);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec()))?;
    let mut data = Vec::new();
    archive
        .by_name("Home")
        .map_err(|_| ImportError::NoHome)?
        .read_to_end(&mut data)
        .map_err(|source| ImportError::Io {
            path: PathBuf::from("Home"),
            source,
        })?;
    newera_core::progress::step("Lendo o projeto", 0, 0);
    let graph = Graph::parse(&data)?;
    let root = graph
        .roots
        .iter()
        .find(|v| graph.is_instance(v, &format!("{MODEL}Home")))
        .ok_or(ImportError::NotAHome)?;
    let mut importer = Importer {
        graph: &graph,
        archive,
        assets: assets.to_owned(),
        memory: in_memory.then(Vec::new),
        extracted: HashMap::new(),
        levels: HashMap::new(),
        warnings: Vec::new(),
        home: Home::default(),
    };
    importer.home(root);
    if newera_core::progress::cancelled() {
        return Err(ImportError::Cancelled);
    }
    let mut home = importer.home;
    if let Some(name) = name {
        home.name = name;
    }
    Ok((
        Imported {
            home,
            warnings: importer.warnings,
        },
        importer.memory.unwrap_or_default(),
    ))
}

/// Typed view of a deserialized object.
#[derive(Clone, Copy)]
struct Obj<'g> {
    graph: &'g Graph,
    object: &'g Object,
}

impl<'g> Obj<'g> {
    fn new(graph: &'g Graph, value: &Value) -> Option<Self> {
        graph.object(value).map(|object| Self { graph, object })
    }

    fn class(&self) -> &'g str {
        self.graph
            .class_desc(self.object.class)
            .map_or("", |c| c.name.as_str())
    }

    fn is(&self, simple: &str) -> bool {
        self.object.data.iter().any(|d| {
            self.graph
                .class_desc(d.class)
                .is_some_and(|c| c.name.strip_prefix(MODEL) == Some(simple))
        })
    }

    /// Field by name; the most derived class wins.
    fn field(&self, name: &str) -> Option<&'g Value> {
        self.object
            .data
            .iter()
            .rev()
            .find_map(|d| d.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v))
    }

    fn obj(&self, name: &str) -> Option<Self> {
        Self::new(self.graph, self.field(name)?)
    }

    /// A primitive or boxed number.
    fn num(&self, name: &str) -> Option<f64> {
        number(self.graph, self.field(name)?)
    }

    fn f(&self, name: &str) -> f64 {
        self.num(name).unwrap_or(0.0)
    }

    fn deg(&self, name: &str) -> f64 {
        self.f(name).to_degrees()
    }

    fn b(&self, name: &str) -> bool {
        match self.field(name) {
            Some(Value::Bool(b)) => *b,
            Some(v) => Self::new(self.graph, v)
                .and_then(|o| o.field("value").cloned())
                .is_some_and(|v| v == Value::Bool(true)),
            None => false,
        }
    }

    fn s(&self, name: &str) -> Option<String> {
        self.graph.string(self.field(name)?).map(str::to_owned)
    }

    fn color(&self, name: &str) -> Option<[u8; 3]> {
        #[allow(clippy::cast_possible_truncation)]
        self.num(name).map(|c| rgb(c as i64))
    }

    fn enum_name(&self, name: &str) -> Option<String> {
        match self.graph.entry(self.field(name)?)? {
            Entry::Enum { constant, .. } => Some(constant.clone()),
            Entry::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn list(&self, name: &str) -> Vec<&'g Value> {
        self.field(name)
            .map_or_else(Vec::new, |v| list(self.graph, v))
    }

    fn floats(&self, name: &str) -> Vec<f64> {
        match self.field(name).and_then(|v| self.graph.entry(v)) {
            Some(Entry::Array { values, .. }) => values
                .iter()
                .filter_map(|v| number(self.graph, v))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn points(&self, name: &str) -> Vec<Point2> {
        match self.field(name).and_then(|v| self.graph.entry(v)) {
            Some(Entry::Array { values, .. }) => values
                .iter()
                .filter_map(|row| match self.graph.entry(row)? {
                    Entry::Array { values, .. } if values.len() >= 2 => Some(Point2::new(
                        number(self.graph, &values[0])?,
                        number(self.graph, &values[1])?,
                    )),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn matrix(&self, name: &str) -> Option<[[f64; 3]; 3]> {
        let Entry::Array { values, .. } = self.graph.entry(self.field(name)?)? else {
            return None;
        };
        let mut out = [[0.0; 3]; 3];
        for (r, row) in values.iter().take(3).enumerate() {
            let Entry::Array { values, .. } = self.graph.entry(row)? else {
                return None;
            };
            for (c, v) in values.iter().take(3).enumerate() {
                out[r][c] = number(self.graph, v)?;
            }
        }
        Some(out)
    }
}

fn number(graph: &Graph, value: &Value) -> Option<f64> {
    match value {
        Value::Byte(v) => Some(f64::from(*v)),
        Value::Short(v) => Some(f64::from(*v)),
        Value::Int(v) => Some(f64::from(*v)),
        #[allow(clippy::cast_precision_loss)]
        Value::Long(v) => Some(*v as f64),
        Value::Float(v) => Some(f64::from(*v)),
        Value::Double(v) => Some(*v),
        Value::Char(v) => Some(f64::from(*v)),
        Value::Ref(_) => {
            let object = graph.object(value)?;
            let (_, inner) = object
                .data
                .iter()
                .rev()
                .find_map(|d| d.fields.iter().find(|(n, _)| n == "value"))?;
            number(graph, inner)
        }
        Value::Null | Value::Bool(_) => None,
    }
}

/// Elements of an `ArrayList`, unmodifiable list wrapper or object array.
fn list<'g>(graph: &'g Graph, value: &'g Value) -> Vec<&'g Value> {
    match graph.entry(value) {
        Some(Entry::Array { values, .. }) => values.iter().collect(),
        Some(Entry::Object(object)) => {
            let obj = Obj { graph, object };
            if let Some(inner) = obj.field("list").or_else(|| obj.field("c")) {
                return list(graph, inner);
            }
            object
                .data
                .iter()
                .flat_map(|d| javaser::annotation_values(&d.annotations))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Key/value pairs of a `HashMap` (or singleton map) of strings.
fn string_map(graph: &Graph, value: &Value) -> Vec<(String, String)> {
    let Some(obj) = Obj::new(graph, value) else {
        return Vec::new();
    };
    let text = |v: &Value| {
        graph
            .string(v)
            .map(str::to_owned)
            .or_else(|| number(graph, v).map(|n| n.to_string()))
    };
    if let (Some(k), Some(v)) = (obj.field("k"), obj.field("v")) {
        return text(k).zip(text(v)).into_iter().collect();
    }
    let values: Vec<&Value> = obj
        .object
        .data
        .iter()
        .flat_map(|d| javaser::annotation_values(&d.annotations))
        .collect();
    values
        .chunks(2)
        .filter_map(|pair| match pair {
            [k, v] => text(k).zip(text(v)),
            _ => None,
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rgb(argb: i64) -> [u8; 3] {
    [
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
    ]
}

fn text_style(style: Option<Obj<'_>>, default_size: f64) -> TextStyle {
    let Some(s) = style else {
        return TextStyle::new(default_size);
    };
    TextStyle {
        font: s.s("fontName"),
        size: s.num("fontSize").unwrap_or(default_size),
        bold: s.b("bold"),
        italic: s.b("italic"),
        align: match s.enum_name("alignment").as_deref() {
            Some("LEFT") => TextAlign::Left,
            Some("RIGHT") => TextAlign::Right,
            _ => TextAlign::Center,
        },
    }
}

/// Default sizes Sweet Home 3D uses when an element has no explicit style.
const LABEL_SIZE: f64 = 18.0;
const DIMENSION_SIZE: f64 = 18.0;
const ROOM_NAME_SIZE: f64 = 24.0;
const ROOM_AREA_SIZE: f64 = 24.0;
const PIECE_NAME_SIZE: f64 = 18.0;

struct Importer<'g> {
    graph: &'g Graph,
    archive: zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    assets: PathBuf,
    /// Files extracted in memory (browser editor) instead of on disk.
    memory: Option<Vec<(String, Vec<u8>)>>,
    /// Archive entry → extracted relative path (`None` when it failed).
    extracted: HashMap<String, Option<String>>,
    levels: HashMap<usize, LevelId>,
    warnings: Vec<String>,
    home: Home,
}

/// What an extracted entry is used for, to pick a file extension.
#[derive(Clone, Copy, PartialEq)]
enum AssetKind {
    Model,
    Image,
}

impl<'g> Importer<'g> {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn obj(&self, value: &Value) -> Option<Obj<'g>> {
        Obj::new(self.graph, value)
    }

    fn properties(&self, o: Obj<'_>) -> Properties {
        let mut props = Properties::new();
        if let Some(id) = o.s("id") {
            props.insert("sh3d:id".into(), id);
        }
        if let Some(map) = o.field("properties") {
            for (k, v) in string_map(self.graph, map) {
                props.insert(k, v);
            }
        }
        props
    }

    fn level_of(&self, o: Obj<'_>) -> Option<LevelId> {
        match o.field("level")? {
            Value::Ref(h) => self.levels.get(h).copied(),
            _ => None,
        }
    }

    // --- Assets ---------------------------------------------------------------

    /// Archive entry named by a content object (`URLContent` and friends).
    fn content_entry(&mut self, content: Obj<'_>) -> Option<String> {
        let url = content.obj("url")?;
        let file = url.s("file").unwrap_or_default();
        let protocol = url.s("protocol").unwrap_or_default();
        if let Some((_, entry)) = file.split_once("!/")
            && (protocol == "jar" || file.starts_with("file:"))
        {
            return Some(entry.to_owned());
        }
        self.warn(format!(
            "external content not embedded in the file: {protocol}:{file}"
        ));
        None
    }

    /// Extracts the file (and, for models, its sibling files) behind a
    /// content object; returns its path relative to the assets directory.
    fn asset(&mut self, content: Option<Obj<'_>>, kind: AssetKind) -> Option<String> {
        let entry = self.content_entry(content?)?;
        if let Some(done) = self.extracted.get(&entry) {
            return done.clone();
        }
        // Where the time of an import goes: one image or model at a time, and
        // how many there are in total is only known once they are all out.
        newera_core::progress::step(
            "Extraindo imagens e modelos",
            self.extracted.len() as u64 + 1,
            0,
        );
        let result = self.extract(&entry, kind);
        if let Err(err) = &result {
            self.warn(format!("cannot extract `{entry}`: {err}"));
        }
        let path = result.ok();
        self.extracted.insert(entry, path.clone());
        path
    }

    /// Saves an extracted file under the assets directory (or in memory).
    fn store(&mut self, relative: &str, bytes: Vec<u8>) -> std::io::Result<()> {
        if let Some(memory) = &mut self.memory {
            newera_core::vfs::mount(&self.assets, [(relative.to_owned(), bytes.clone())]);
            memory.push((relative.to_owned(), bytes));
            return Ok(());
        }
        let target = self.assets.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)
    }

    fn extract(&mut self, entry: &str, kind: AssetKind) -> std::io::Result<String> {
        let read = |archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str| {
            let mut bytes = Vec::new();
            archive
                .by_name(name)
                .map_err(std::io::Error::other)?
                .read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        };
        match entry.rsplit_once('/') {
            // A file inside a directory: models bring their MTL and textures.
            Some((dir, _)) => {
                let names: Vec<String> = if kind == AssetKind::Model {
                    self.archive
                        .file_names()
                        .filter(|n| n.starts_with(&format!("{dir}/")) && !n.ends_with('/'))
                        .map(str::to_owned)
                        .collect()
                } else {
                    vec![entry.to_owned()]
                };
                for name in names {
                    let bytes = read(&mut self.archive, &name)?;
                    self.store(&name, bytes)?;
                }
                Ok(entry.to_owned())
            }
            // A bare entry: give it an extension so loaders know its format.
            None => {
                let bytes = read(&mut self.archive, entry)?;
                let ext = sniff_extension(&bytes).unwrap_or(match kind {
                    AssetKind::Model => "obj",
                    AssetKind::Image => "png",
                });
                if ext == "zip" {
                    return self.extract_nested_model(entry, &bytes);
                }
                let name = format!("{entry}.{ext}");
                self.store(&name, bytes)?;
                Ok(name)
            }
        }
    }

    /// Some models are stored as a ZIP inside the file; unpack it and point
    /// at its first model file.
    fn extract_nested_model(&mut self, entry: &str, bytes: &[u8]) -> std::io::Result<String> {
        let mut inner =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(std::io::Error::other)?;
        let mut model = None;
        for i in 0..inner.len() {
            let mut file = inner.by_index(i).map_err(std::io::Error::other)?;
            if file.is_dir() {
                continue;
            }
            let name = format!("{entry}.d/{}", file.name());
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            drop(file);
            self.store(&name, data)?;
            let lower = name.to_lowercase();
            if model.is_none()
                && [".obj", ".gltf", ".glb", ".dae", ".3ds"]
                    .iter()
                    .any(|e| lower.ends_with(e))
            {
                model = Some(name);
            }
        }
        model.ok_or_else(|| std::io::Error::other("no model file inside"))
    }

    // --- Home -----------------------------------------------------------------

    fn home(&mut self, root: &Value) {
        let Some(h) = self.obj(root) else { return };
        let mut home = Home::default();
        home.wall_height = h.num("wallHeight").unwrap_or(Wall::DEFAULT_HEIGHT);
        home.base_plan_locked = h.b("basePlanLocked");
        home.furniture_descending = h.b("furnitureDescendingSorted");
        home.furniture_sort = h
            .s("furnitureSortedPropertyName")
            .or_else(|| h.enum_name("furnitureSortedProperty"));
        home.properties = self.properties(h);
        if let Some(name) = h.s("name") {
            home.properties.insert("sh3d:name".into(), name);
        }
        if let Some(version) = h.num("version") {
            home.properties
                .insert("sh3d:version".into(), version.to_string());
        }
        if let Some(map) = h.field("visualProperties") {
            for (k, v) in string_map(self.graph, map) {
                home.properties.insert(format!("sh3d:visual:{k}"), v);
            }
        }
        home.furniture_columns = h
            .list("furnitureVisiblePropertyNames")
            .into_iter()
            .filter_map(|v| self.graph.string(v).map(str::to_owned))
            .collect();
        if home.furniture_columns.is_empty() {
            home.furniture_columns = h
                .list("furnitureVisibleProperties")
                .into_iter()
                .filter_map(|v| match self.graph.entry(v) {
                    Some(Entry::Enum { constant, .. }) => Some(constant.clone()),
                    _ => None,
                })
                .collect();
        }
        self.home = home;

        // Levels first: everything else refers to them.
        for value in h.list("levels") {
            let Value::Ref(handle) = value else { continue };
            let Some(l) = self.obj(value) else { continue };
            let id = self.home.new_level_id();
            self.levels.insert(*handle, id);
            let background = l.obj("backgroundImage").and_then(|b| self.background(b));
            let level = Level {
                id,
                name: l.s("name").unwrap_or_else(|| "Nível".into()),
                elevation: l.f("elevation"),
                height: l.f("height"),
                floor_thickness: l.f("floorThickness"),
                elevation_index: l.num("elevationIndex").unwrap_or(0.0) as i32,
                viewable: l.field("viewable").is_none() || l.b("viewable"),
                background,
                properties: self.properties(l),
            };
            self.home.levels.push(level);
        }
        if let Some(Value::Ref(handle)) = h.field("selectedLevel") {
            self.home.selected_level = self.levels.get(handle).copied();
        }
        self.home.background = h.obj("backgroundImage").and_then(|b| self.background(b));

        for value in h.list("walls") {
            if let Some(w) = self.obj(value) {
                let wall = self.wall(w);
                self.home.walls.push(wall);
            }
        }
        for value in h.list("rooms") {
            if let Some(r) = self.obj(value) {
                let room = self.room(r);
                self.home.rooms.push(room);
            }
        }
        for value in h.list("dimensionLines") {
            if let Some(d) = self.obj(value) {
                let dim = self.dimension(d);
                self.home.dimensions.push(dim);
            }
        }
        for value in h.list("labels") {
            if let Some(l) = self.obj(value) {
                let label = self.label(l);
                self.home.labels.push(label);
            }
        }
        for value in h.list("polylines") {
            if let Some(p) = self.obj(value) {
                let polyline = self.polyline(p);
                self.home.polylines.push(polyline);
            }
        }
        // Newer files keep groups in `furnitureWithGroups`; `furniture` is a
        // flattened copy for old versions.
        let furniture = [
            "furnitureWithGroups",
            "furnitureWithDoorsAndWindows",
            "furniture",
        ]
        .into_iter()
        .map(|name| h.list(name))
        .find(|l| !l.is_empty())
        .unwrap_or_default();
        for value in furniture {
            if let Some(p) = self.obj(value) {
                let piece = self.piece(p);
                self.home.furniture.push(piece);
            }
        }

        if let Some(c) = h.obj("compass") {
            self.home.compass = Self::compass(c);
        }
        if let Some(e) = h.obj("environment") {
            self.home.environment = self.environment(e);
        }
        let observer = h.obj("observerCamera").map(|c| camera(c));
        let top = h.obj("topCamera").map(|c| camera(c));
        let observer_active = match (h.field("camera"), h.field("observerCamera")) {
            (Some(Value::Ref(a)), Some(Value::Ref(b))) => a == b,
            _ => false,
        };
        self.home.cameras = Cameras {
            top: top.unwrap_or_default(),
            observer: observer.unwrap_or_default(),
            observer_active,
            observer_fixed_size: h.obj("observerCamera").is_none_or(|c| c.b("fixedSize")),
            stored: h
                .list("storedCameras")
                .into_iter()
                .filter_map(|v| self.obj(v).map(camera))
                .collect(),
        };
        if let Some(p) = h.obj("print") {
            self.home.print = Some(print(p, &self.levels));
        }
    }

    fn texture(
        &mut self,
        t: Option<Obj<'_>>,
        color: Option<[u8; 3]>,
        shininess: f64,
    ) -> Option<Material> {
        let Some(t) = t else {
            return color.map(|c| Material {
                shininess,
                ..Material::paint(c)
            });
        };
        let image = self.asset(t.obj("image"), AssetKind::Image);
        let scale = t.num("scale").filter(|s| *s > 0.0).unwrap_or(1.0);
        let offset = [t.f("xOffset"), t.f("yOffset")];
        Some(Material {
            color: None,
            pattern: None,
            image,
            tile: Some([t.f("width") * scale, t.f("height") * scale])
                .filter(|[w, h]| *w > 0.0 && *h > 0.0),
            angle: t.deg("angle"),
            offset: (offset != [0.0, 0.0]).then_some(offset),
            shininess,
            name: t.s("name"),
            fit: t.b("fittingArea"),
            left_to_right: t.field("leftToRightOriented").is_none() || t.b("leftToRightOriented"),
        })
    }

    fn background(&mut self, b: Obj<'_>) -> Option<BackgroundImage> {
        let path = self.asset(b.obj("image"), AssetKind::Image)?;
        let (width, height) = newera_core::vfs::read(&self.assets.join(&path))
            .ok()
            .and_then(|bytes| {
                image::ImageReader::new(std::io::Cursor::new(bytes))
                    .with_guessed_format()
                    .ok()?
                    .into_dimensions()
                    .ok()
            })
            .unwrap_or((1, 1));
        let start = Point2::new(b.f("scaleDistanceXStart"), b.f("scaleDistanceYStart"));
        let end = Point2::new(b.f("scaleDistanceXEnd"), b.f("scaleDistanceYEnd"));
        let cm_per_px =
            BackgroundImage::cm_per_px_from(start, end, b.f("scaleDistance")).unwrap_or(1.0);
        Some(BackgroundImage {
            path,
            size_px: [width, height],
            cm_per_px,
            offset: Point2::new(-b.f("xOrigin"), -b.f("yOrigin")),
            opacity: 1.0,
            visible: !b.b("invisible"),
            ..BackgroundImage::default()
        })
    }

    fn wall(&mut self, w: Obj<'_>) -> Wall {
        let side = |this: &mut Self, side: &str| {
            let texture = w.obj(&format!("{side}SideTexture"));
            let color = w.color(&format!("{side}SideColor"));
            let shininess = w.f(&format!("{side}SideShininess"));
            this.texture(texture, color, shininess)
        };
        let baseboard = |this: &mut Self, side: &str| {
            let b = w.obj(&format!("{side}SideBaseboard"))?;
            let material = this.texture(b.obj("texture"), b.color("color"), 0.0);
            Some(Baseboard {
                thickness: b.f("thickness"),
                height: b.f("height"),
                material,
            })
        };
        let left_side = side(self, "left");
        let right_side = side(self, "right");
        let left_baseboard = baseboard(self, "left");
        let right_baseboard = baseboard(self, "right");
        let mut wall = Wall::new(
            newera_core::WallId(0),
            Point2::new(w.f("xStart"), w.f("yStart")),
            Point2::new(w.f("xEnd"), w.f("yEnd")),
        );
        wall.id = self.home.new_wall_id();
        wall.thickness = w.num("thickness").unwrap_or(Wall::DEFAULT_THICKNESS);
        wall.height = w.num("height").unwrap_or(self.home.wall_height);
        wall.height_at_end = w.num("heightAtEnd");
        wall.arc_extent = w.num("arcExtent").map(f64::to_degrees);
        wall.level = self.level_of(w);
        wall.left_side = left_side;
        wall.right_side = right_side;
        wall.left_baseboard = left_baseboard;
        wall.right_baseboard = right_baseboard;
        wall.top_color = w.color("topColor");
        wall.pattern = w.s("pattern").or_else(|| w.enum_name("pattern"));
        wall.properties = self.properties(w);
        wall
    }

    fn room(&mut self, r: Obj<'_>) -> Room {
        let floor = self.texture(
            r.obj("floorTexture"),
            r.color("floorColor"),
            r.f("floorShininess"),
        );
        let ceiling = self.texture(
            r.obj("ceilingTexture"),
            r.color("ceilingColor"),
            r.f("ceilingShininess"),
        );
        let mut room = Room::new(
            self.home.new_room_id(),
            r.s("name").unwrap_or_default(),
            r.points("points"),
        );
        room.level = self.level_of(r);
        room.floor_visible = r.b("floorVisible");
        room.ceiling_visible = r.b("ceilingVisible");
        room.area_visible = r.b("areaVisible");
        room.floor_material = floor;
        room.ceiling_material = ceiling;
        room.ceiling_flat = r.field("ceilingFlat").is_none() || r.b("ceilingFlat");
        room.name_style = Some(text_style(r.obj("nameStyle"), ROOM_NAME_SIZE));
        room.area_style = Some(text_style(r.obj("areaStyle"), ROOM_AREA_SIZE));
        room.name_offset = [r.f("nameXOffset"), r.f("nameYOffset")];
        room.area_offset = [r.f("areaXOffset"), r.f("areaYOffset")];
        room.name_angle = r.deg("nameAngle");
        room.area_angle = r.deg("areaAngle");
        room.properties = self.properties(r);
        room
    }

    fn dimension(&mut self, d: Obj<'_>) -> Dimension {
        Dimension {
            id: self.home.new_dimension_id(),
            start: Point2::new(d.f("xStart"), d.f("yStart")),
            end: Point2::new(d.f("xEnd"), d.f("yEnd")),
            // Sweet Home 3D measures offsets toward the right of start → end.
            offset: -d.f("offset"),
            level: self.level_of(d),
            color: d.color("color"),
            end_mark: d.num("endMarkSize").unwrap_or(Dimension::DEFAULT_END_MARK),
            style: Some(text_style(d.obj("lengthStyle"), DIMENSION_SIZE)),
            visible_in_3d: d.b("visibleIn3D"),
            elevation: [d.f("elevationStart"), d.f("elevationEnd")],
            pitch: d.deg("pitch"),
            properties: self.properties(d),
            ..Default::default()
        }
    }

    fn label(&mut self, l: Obj<'_>) -> Label {
        let style = text_style(l.obj("style"), LABEL_SIZE);
        Label {
            id: self.home.new_label_id(),
            text: l.s("text").unwrap_or_default(),
            position: Point2::new(l.f("x"), l.f("y")),
            size: style.size,
            angle: l.deg("angle"),
            level: self.level_of(l),
            font: style.font,
            bold: style.bold,
            italic: style.italic,
            align: style.align,
            color: l.color("color"),
            outline: l.color("outlineColor"),
            elevation: l.f("elevation"),
            pitch: l.num("pitch").map(f64::to_degrees),
            properties: self.properties(l),
            ..Default::default()
        }
    }

    fn polyline(&mut self, p: Obj<'_>) -> Polyline {
        let mut line = Polyline::new(self.home.new_polyline_id(), p.points("points"));
        line.closed = p.b("closedPath");
        line.thickness = p.num("thickness").unwrap_or(1.0);
        line.color = p.color("color").unwrap_or([0, 0, 0]);
        line.cap = match p.s("capStyleName").as_deref() {
            Some("ROUND") => LineCap::Round,
            Some("SQUARE") => LineCap::Square,
            _ => LineCap::Butt,
        };
        line.join = match p.s("joinStyleName").as_deref() {
            Some("BEVEL") => LineJoin::Bevel,
            Some("ROUND") => LineJoin::Round,
            Some("CURVED") => LineJoin::Curved,
            _ => LineJoin::Miter,
        };
        line.dash = match p.s("dashStyleName").as_deref() {
            Some("DOT") => DashStyle::Dot,
            Some("DASH") => DashStyle::Dash,
            Some("DASH_DOT") => DashStyle::DashDot,
            Some("DASH_DOT_DOT") => DashStyle::DashDotDot,
            Some("CUSTOMIZED") => DashStyle::Custom,
            _ => DashStyle::Solid,
        };
        line.dash_pattern = p.floats("dashPattern");
        line.dash_offset = p.f("dashOffset");
        let arrow = |name: &str| match p.s(name).as_deref() {
            Some("DELTA") => ArrowStyle::Delta,
            Some("OPEN") => ArrowStyle::Open,
            Some("DISC") => ArrowStyle::Disc,
            _ => ArrowStyle::None,
        };
        line.start_arrow = arrow("startArrowStyleName");
        line.end_arrow = arrow("endArrowStyleName");
        line.elevation = p.num("elevation");
        line.level = self.level_of(p);
        line.properties = self.properties(p);
        line
    }

    fn piece(&mut self, p: Obj<'_>) -> Furniture {
        let known = [
            "HomePieceOfFurniture",
            "HomeDoorOrWindow",
            "HomeLight",
            "HomeFurnitureGroup",
        ];
        let class = p.class().strip_prefix(MODEL).unwrap_or(p.class());
        if !known.contains(&class) {
            self.warn(format!("`{class}` imported as a plain piece"));
        }
        let children: Vec<Furniture> = p
            .list("furniture")
            .into_iter()
            .filter_map(|v| Obj::new(self.graph, v))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|child| self.piece(child))
            .collect();
        let is_group = p.is("HomeFurnitureGroup");
        let model = if is_group {
            None
        } else {
            self.asset(p.obj("model"), AssetKind::Model)
        };
        let texture = self.texture(p.obj("texture"), None, p.f("shininess"));
        let materials = p
            .list("modelMaterials")
            .into_iter()
            .filter_map(|v| Obj::new(self.graph, v))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|m| {
                let shininess = m.num("shininess");
                ModelMaterial {
                    name: m.s("name").unwrap_or_default(),
                    key: m.s("key"),
                    color: m.color("color"),
                    texture: self.texture(m.obj("texture"), None, shininess.unwrap_or(0.0)),
                    shininess,
                    repeat: None,
                }
            })
            .collect();
        let icon = self.asset(p.obj("icon"), AssetKind::Image);
        let plan_icon = self.asset(p.obj("planIcon"), AssetKind::Image);
        let text = |name: &str| p.s(name).or_else(|| p.num(name).map(|n| n.to_string()));
        let info = PieceInfo {
            description: p.s("description"),
            information: p.s("information"),
            creator: p.s("creator"),
            license: p.s("license"),
            source_catalog_id: p.s("catalogId"),
            price: text("price"),
            currency: p.s("currency"),
            vat_percentage: text("valueAddedTaxPercentage"),
            icon,
            plan_icon,
            ..PieceInfo::default()
        };
        let opening = p.is("HomeDoorOrWindow").then(|| opening(p));
        let light = p.is("HomeLight").then(|| Light {
            power: p.num("power").unwrap_or(0.5),
            sources: p
                .list("lightSources")
                .into_iter()
                .filter_map(|v| Obj::new(self.graph, v))
                .map(|s| LightSource {
                    x: s.f("x"),
                    y: s.f("y"),
                    z: s.f("z"),
                    color: s.color("color").unwrap_or([255, 255, 255]),
                    diameter: s.num("diameter"),
                })
                .collect(),
            source_materials: p
                .list("lightSourceMaterialNames")
                .into_iter()
                .filter_map(|v| self.graph.string(v).map(str::to_owned))
                .collect(),
            lumens: None,
            watts: None,
            lamp: None,
            kelvin: None,
            beam: None,
            area: None,
            panel_upward: None,
        });
        let name_style = p
            .obj("nameStyle")
            .map(|s| text_style(Some(s), PIECE_NAME_SIZE));
        Furniture {
            id: self.home.new_furniture_id(),
            catalog: if is_group {
                "group".into()
            } else {
                "imported".into()
            },
            name: p.s("name").unwrap_or_default(),
            position: Point2::new(p.f("x"), p.f("y")),
            elevation: p.f("elevation"),
            angle: p.deg("angle"),
            width: p.f("width"),
            depth: p.f("depth"),
            height: p.f("height"),
            mirrored: p.b("modelMirrored"),
            color: p.color("color"),
            opening,
            model,
            visible: p.b("visible"),
            level: self.level_of(p),
            pitch: p.deg("pitch"),
            roll: p.deg("roll"),
            info,
            locks: PieceLocks {
                movable: p.b("movable"),
                resizable: p.b("resizable"),
                deformable: p.b("deformable"),
                texturable: p.b("texturable"),
                horizontally_rotatable: p.b("horizontallyRotatable"),
            },
            model_transform: ModelTransform {
                rotation: p
                    .matrix("modelRotation")
                    .unwrap_or(ModelTransform::default().rotation),
                centered_at_origin: p.field("modelCenteredAtOrigin").is_none()
                    || p.b("modelCenteredAtOrigin"),
                back_face_shown: p.b("backFaceShown"),
                flags: p.num("modelFlags").unwrap_or(0.0) as u32,
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                file_size: p.num("modelSize").map(|s| s as u64),
            },
            texture,
            shininess: p.num("shininess"),
            materials,
            light,
            children,
            drop_on_top: p.num("dropOnTopElevation").unwrap_or(1.0),
            staircase_cut_out: p.s("staircaseCutOutShape"),
            name_visible: p.b("nameVisible"),
            name_style,
            name_offset: [p.f("nameXOffset"), p.f("nameYOffset")],
            name_angle: p.deg("nameAngle"),
            properties: self.properties(p),
            ..Default::default()
        }
    }

    fn compass(c: Obj<'_>) -> Compass {
        let time_zone = c.obj("timeZone").and_then(|tz| tz.s("ID"));
        let mut compass = Compass {
            center: Point2::new(c.f("x"), c.f("y")),
            diameter: c.num("diameter").unwrap_or(100.0),
            north_degrees: c.deg("northDirection"),
            visible: c.b("visible"),
            latitude: c.num("latitude").map(f64::to_degrees),
            longitude: c.num("longitude").map(f64::to_degrees),
            time_zone,
            // Sweet Home 3D has no notion of the code a project answers to:
            // the place is left unsaid, and the review falls back to the home
            // jurisdiction until somebody says otherwise.
            city: None,
            country: None,
            region: None,
        };
        if compass.diameter <= 0.0 {
            compass.diameter = 100.0;
        }
        compass
    }

    fn environment(&mut self, e: Obj<'_>) -> Environment {
        let ground_texture = self.texture(e.obj("groundTexture"), None, 0.0);
        let sky_texture = self.texture(e.obj("skyTexture"), None, 0.0);
        let defaults = Environment::default();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let int = |name: &str, default: u32| e.num(name).map_or(default, |v| v as u32);
        Environment {
            ground_color: e.color("groundColor").unwrap_or(defaults.ground_color),
            ground_texture,
            sky_color: e.color("skyColor").unwrap_or(defaults.sky_color),
            sky_texture,
            light_color: e.color("lightColor").unwrap_or(defaults.light_color),
            ceiling_light_color: e
                .color("ceilingLightColor")
                .unwrap_or(defaults.ceiling_light_color),
            walls_alpha: e.f("wallsAlpha"),
            drawing_mode: match e.enum_name("drawingMode").as_deref() {
                Some("OUTLINE") => DrawingMode::Outline,
                Some("FILL_AND_OUTLINE") => DrawingMode::FillAndOutline,
                _ => DrawingMode::Fill,
            },
            all_levels_visible: e.b("allLevelsVisible"),
            background_on_ground: e.b("backgroundImageVisibleOnGround3D"),
            observer_elevation_adjusted: e.field("observerCameraElevationAdjusted").is_none()
                || e.b("observerCameraElevationAdjusted"),
            subpart_size_under_light: e.f("subpartSizeUnderLight"),
            photo: PhotoSettings {
                width: int("photoWidth", 400),
                height: int("photoHeight", 300),
                quality: u8::try_from(int("photoQuality", 0)).unwrap_or(0),
                aspect_ratio: e
                    .s("photoAspectRatioName")
                    .or_else(|| e.enum_name("photoAspectRatio")),
            },
            video: VideoSettings {
                width: int("videoWidth", 320),
                quality: u8::try_from(int("videoQuality", 0)).unwrap_or(0),
                frame_rate: int("videoFrameRate", 25),
                speed: e.num("videoSpeed").unwrap_or(defaults.video.speed),
                aspect_ratio: e
                    .s("videoAspectRatioName")
                    .or_else(|| e.enum_name("videoAspectRatio")),
            },
            camera_path: e
                .list("cameraPath")
                .into_iter()
                .filter_map(|v| Obj::new(self.graph, v))
                .map(camera)
                .collect(),
        }
    }
}

fn opening(p: Obj<'_>) -> Opening {
    let name = format!(
        "{} {}",
        p.s("name").unwrap_or_default(),
        p.s("catalogId").unwrap_or_default()
    )
    .to_lowercase();
    let sashes: Vec<Sash> = p
        .list("sashes")
        .into_iter()
        .filter_map(|v| Obj::new(p.graph, v))
        .map(|s| Sash {
            x_axis: s.f("xAxis"),
            y_axis: s.f("yAxis"),
            width: s.f("width"),
            start_angle: s.deg("startAngle"),
            end_angle: s.deg("endAngle"),
        })
        .collect();
    let window = ["window", "janela", "slider", "fechamento", "vitr"]
        .iter()
        .any(|k| name.contains(k));
    let passage = sashes.is_empty() && !window;
    let sliding = ["correr", "slid", "sliding"]
        .iter()
        .any(|k| name.contains(k));
    Opening {
        kind: if window {
            OpeningKind::Window
        } else if passage {
            OpeningKind::Passage
        } else {
            OpeningKind::Door
        },
        hinge_right: false,
        leaves: u8::try_from(sashes.len().clamp(1, 255)).unwrap_or(1),
        sliding,
        sashes,
        cut_out: Some(WallCutOut {
            wall_thickness: p.num("wallThickness").unwrap_or(1.0),
            wall_distance: p.f("wallDistance"),
            wall_width: p.num("wallWidth").unwrap_or(1.0),
            wall_left: p.f("wallLeft"),
            wall_height: p.num("wallHeight").unwrap_or(1.0),
            wall_top: p.f("wallTop"),
            shape: p.s("cutOutShape"),
            both_sides: p.b("wallCutOutOnBothSides"),
            bound_to_wall: p.b("boundToWall"),
            width_depth_deformable: p.field("widthDepthDeformable").is_none()
                || p.b("widthDepthDeformable"),
        }),
    }
}

fn camera(c: Obj<'_>) -> Camera {
    #[allow(clippy::cast_possible_truncation)]
    Camera {
        name: c.s("name"),
        x: c.f("x"),
        y: c.f("y"),
        z: c.f("z"),
        yaw: c.deg("yaw"),
        pitch: c.deg("pitch"),
        fov: c.deg("fieldOfView"),
        time: c.num("time").unwrap_or(0.0) as i64,
        lens: c.s("lensName").or_else(|| c.enum_name("lens")),
        renderer: c.s("renderer"),
    }
}

fn print(p: Obj<'_>, levels: &HashMap<usize, LevelId>) -> PrintSettings {
    let printed_levels = p.field("printedLevels").map(|v| {
        list(p.graph, v)
            .into_iter()
            .filter_map(|l| match l {
                Value::Ref(h) => levels.get(h).copied(),
                _ => None,
            })
            .collect()
    });
    PrintSettings {
        orientation: match p.enum_name("paperOrientation").as_deref() {
            Some("LANDSCAPE") => PaperOrientation::Landscape,
            Some("REVERSE_LANDSCAPE") => PaperOrientation::ReverseLandscape,
            _ => PaperOrientation::Portrait,
        },
        paper_width: p.f("paperWidth"),
        paper_height: p.f("paperHeight"),
        margins: [
            p.f("paperTopMargin"),
            p.f("paperLeftMargin"),
            p.f("paperBottomMargin"),
            p.f("paperRightMargin"),
        ],
        furniture: p.b("furniturePrinted"),
        plan: p.b("planPrinted"),
        view_3d: p.b("view3DPrinted"),
        plan_scale: p.num("planScale"),
        header: p.s("headerFormat"),
        footer: p.s("footerFormat"),
        levels: printed_levels,
    }
}

/// File extension from a file's first bytes.
fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\x89PNG") {
        Some("png")
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if starts(b"GIF8") {
        Some("gif")
    } else if starts(b"BM") {
        Some("bmp")
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if starts(b"glTF") {
        Some("glb")
    } else if starts(b"PK\x03\x04") {
        Some("zip")
    } else if starts(&[0x4D, 0x4D]) {
        Some("3ds")
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
        if head.contains("<COLLADA") {
            Some("dae")
        } else if head.trim_start().starts_with('{') && head.contains("\"asset\"") {
            Some("gltf")
        } else if head.lines().any(|l| {
            l.starts_with("v ")
                || l.starts_with("o ")
                || l.starts_with("g ")
                || l.starts_with("mtllib")
        }) || head.starts_with('#')
        {
            Some("obj")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_formats() {
        assert_eq!(sniff_extension(b"\x89PNG\r\n"), Some("png"));
        assert_eq!(sniff_extension(b"# Blender\nv 1 2 3\n"), Some("obj"));
        assert_eq!(sniff_extension(b"glTF\x02\x00"), Some("glb"));
        assert_eq!(sniff_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff_extension(b"\x00\x01"), None);
    }

    #[test]
    fn colors_drop_alpha() {
        assert_eq!(
            rgb(i64::from(0xFF12_3456_u32.cast_signed())),
            [0x12, 0x34, 0x56]
        );
        assert_eq!(rgb(0x00B8_BB5D), [0xB8, 0xBB, 0x5D]);
    }

    /// The real project saved as a bundle and reopened keeps every file its
    /// models and materials need.
    #[test]
    #[ignore = "needs NEWERA_SH3D_SAMPLE=/path/to/file.sh3d"]
    fn a_real_home_round_trips_through_a_bundle() {
        let path = std::env::var("NEWERA_SH3D_SAMPLE").expect("NEWERA_SH3D_SAMPLE");
        let root = std::env::temp_dir().join(format!("newera-sh3d-bundle-{}", std::process::id()));
        let mut doc = newera_core::Document::default();
        crate::open_file(&mut doc, Path::new(&path)).unwrap();
        let file = root.join("planta.newera");
        std::fs::create_dir_all(&root).unwrap();
        newera_core::save_project(&doc, &file).unwrap();
        let (project, dir) = newera_core::open_project(&file, &root.join("cache")).unwrap();
        let dir = dir.unwrap();
        let mut home = project.variants[0].1.clone();
        let mut missing = Vec::new();
        let mut total = 0;
        home.for_each_asset_mut(&mut |p| {
            total += 1;
            if !dir.join(p.as_str()).exists() {
                missing.push(p.clone());
            }
        });
        assert!(total > 0 && missing.is_empty(), "missing {missing:?}");
        let models: Vec<String> = home
            .furniture
            .iter()
            .flat_map(|f| f.flatten())
            .filter_map(|f| f.model.clone())
            .collect();
        let broken: Vec<&String> = models
            .iter()
            .filter(|m| newera_catalog::load_model(&dir.join(m.as_str())).is_err())
            .collect();
        assert!(broken.is_empty(), "{broken:?}");
        // Textures named by the models' material libraries came along too.
        for model in models
            .iter()
            .filter(|m| m.to_ascii_lowercase().ends_with(".obj"))
        {
            let mesh = newera_catalog::load_model(&dir.join(model.as_str()))
                .unwrap()
                .mesh;
            for texture in mesh.materials.iter().filter_map(|m| m.texture.as_ref()) {
                assert!(texture.exists(), "{model}: {}", texture.display());
            }
        }
        let size = std::fs::metadata(&file).unwrap().len();
        println!(
            "bundle {size} bytes, {total} assets, {} models",
            models.len()
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// The same real project imported in memory matches the import on disk,
    /// and its models load from the mounted files.
    #[test]
    #[ignore = "needs NEWERA_SH3D_SAMPLE=/path/to/file.sh3d"]
    fn imports_a_real_home_in_memory() {
        let path = std::env::var("NEWERA_SH3D_SAMPLE").expect("NEWERA_SH3D_SAMPLE");
        let disk_dir =
            std::env::temp_dir().join(format!("newera-sh3d-disk-{}", std::process::id()));
        let on_disk = import_file(Path::new(&path), &disk_dir).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let virtual_dir = Path::new("/virtual/sh3d-memory-test");
        let (in_memory, files) = import_bytes(&bytes, "planta", virtual_dir).unwrap();
        assert_ne!(files, [] as [(std::string::String, std::vec::Vec<u8>); 0]);
        let (mut a, mut b) = (on_disk.home.clone(), in_memory.home.clone());
        a.name.clear();
        b.name.clear();
        assert_eq!(a, b, "same home either way");
        let models: Vec<String> = in_memory
            .home
            .furniture
            .iter()
            .flat_map(|f| f.flatten())
            .filter_map(|f| f.model.clone())
            .collect();
        assert_ne!(models, [] as [std::string::String; 0]);
        let loaded = models
            .iter()
            .filter(|m| newera_catalog::load_model(&virtual_dir.join(m)).is_ok())
            .count();
        assert_eq!(loaded, models.len(), "every model loads from memory");
        std::fs::remove_dir_all(disk_dir).ok();
    }

    /// Imports the real project given by `NEWERA_SH3D_SAMPLE` and checks
    /// that every element made it across.
    #[test]
    #[ignore = "needs NEWERA_SH3D_SAMPLE=/path/to/file.sh3d"]
    fn imports_a_real_home() {
        let path = std::env::var("NEWERA_SH3D_SAMPLE").expect("NEWERA_SH3D_SAMPLE");
        let assets = std::env::temp_dir().join(format!("newera-sh3d-{}", std::process::id()));
        let imported = import_file(Path::new(&path), &assets).unwrap();
        let home = &imported.home;
        let pieces: usize = home.furniture.iter().map(|f| f.flatten().len()).sum();
        println!(
            "levels {} walls {} rooms {} dims {} labels {} polylines {} top pieces {} all pieces {} cameras {}",
            home.levels.len(),
            home.walls.len(),
            home.rooms.len(),
            home.dimensions.len(),
            home.labels.len(),
            home.polylines.len(),
            home.furniture.len(),
            pieces,
            home.cameras.stored.len()
        );
        for w in &imported.warnings {
            println!("warning: {w}");
        }
        for piece in home.furniture.iter().flat_map(Furniture::flatten) {
            if let Some(model) = &piece.model {
                assert!(assets.join(model).exists(), "{model} extracted");
            }
        }
        assert_ne!(home.walls, [] as [newera_core::Wall; 0]);
        let json = newera_core::to_project_json(&newera_core::Document::new(home.clone()));
        println!("project json: {} KB", json.len() / 1024);
        std::fs::remove_dir_all(assets).ok();
    }
}

/// Fields every Sweet Home 3D class may carry, as `Class.field`, that the
/// importer maps (or deliberately derives from other data). The coverage
/// test fails on anything else that holds a value, so no data goes unnoticed.
#[cfg(test)]
const HANDLED: &[&str] = &[
    "HomeObject.id",
    "HomeObject.properties",
    // Home
    "Home.camera",
    "Home.compass",
    "Home.dimensionLines",
    "Home.environment",
    "Home.furniture",
    "Home.furnitureVisibleProperties",
    "Home.furnitureVisiblePropertyNames",
    "Home.furnitureWithDoorsAndWindows",
    "Home.furnitureWithGroups",
    "Home.labels",
    "Home.levels",
    "Home.name",
    "Home.observerCamera",
    "Home.polylines",
    "Home.print",
    "Home.properties",
    "Home.rooms",
    "Home.selectedLevel",
    "Home.storedCameras",
    "Home.topCamera",
    "Home.version",
    "Home.wallHeight",
    "Home.walls",
    "Home.visualProperties",
    "Home.basePlanLocked",
    "Home.furnitureDescendingSorted",
    "Home.furnitureSortedProperty",
    "Home.furnitureSortedPropertyName",
    "Home.backgroundImage",
    // Legacy copies of environment values kept by Home for old versions.
    "Home.groundColor",
    "Home.lightColor",
    "Home.skyColor",
    "Home.wallsAlpha",
    "Home.groundTexture",
    // Furniture
    "HomePieceOfFurniture.angle",
    "HomePieceOfFurniture.catalogId",
    "HomePieceOfFurniture.color",
    "HomePieceOfFurniture.creator",
    "HomePieceOfFurniture.deformable",
    "HomePieceOfFurniture.depth",
    "HomePieceOfFurniture.depthInPlan",
    "HomePieceOfFurniture.description",
    "HomePieceOfFurniture.doorOrWindow",
    "HomePieceOfFurniture.dropOnTopElevation",
    "HomePieceOfFurniture.elevation",
    "HomePieceOfFurniture.height",
    "HomePieceOfFurniture.heightInPlan",
    "HomePieceOfFurniture.horizontallyRotatable",
    "HomePieceOfFurniture.icon",
    "HomePieceOfFurniture.level",
    "HomePieceOfFurniture.license",
    "HomePieceOfFurniture.model",
    "HomePieceOfFurniture.modelCenteredAtOrigin",
    "HomePieceOfFurniture.modelMaterials",
    "HomePieceOfFurniture.modelMirrored",
    "HomePieceOfFurniture.modelRotation",
    "HomePieceOfFurniture.modelSize",
    "HomePieceOfFurniture.movable",
    "HomePieceOfFurniture.name",
    "HomePieceOfFurniture.planIcon",
    "HomePieceOfFurniture.resizable",
    "HomePieceOfFurniture.shininess",
    "HomePieceOfFurniture.texturable",
    "HomePieceOfFurniture.texture",
    "HomePieceOfFurniture.visible",
    "HomePieceOfFurniture.width",
    "HomePieceOfFurniture.widthInPlan",
    "HomePieceOfFurniture.x",
    "HomePieceOfFurniture.y",
    "HomePieceOfFurniture.pitch",
    "HomePieceOfFurniture.roll",
    "HomePieceOfFurniture.backFaceShown",
    "HomePieceOfFurniture.modelFlags",
    "HomePieceOfFurniture.nameAngle",
    "HomePieceOfFurniture.nameVisible",
    "HomePieceOfFurniture.nameXOffset",
    "HomePieceOfFurniture.nameYOffset",
    "HomePieceOfFurniture.nameStyle",
    "HomePieceOfFurniture.information",
    "HomePieceOfFurniture.price",
    "HomePieceOfFurniture.currency",
    "HomePieceOfFurniture.valueAddedTaxPercentage",
    "HomePieceOfFurniture.staircaseCutOutShape",
    "HomeFurnitureGroup.furniture",
    "HomeFurnitureGroup.deformable",
    "HomeFurnitureGroup.resizable",
    "HomeFurnitureGroup.texturable",
    "HomeFurnitureGroup.dropOnTopElevation",
    "HomeDoorOrWindow.cutOutShape",
    "HomeDoorOrWindow.sashes",
    "HomeDoorOrWindow.wallCutOutOnBothSides",
    "HomeDoorOrWindow.wallDistance",
    "HomeDoorOrWindow.wallHeight",
    "HomeDoorOrWindow.wallThickness",
    "HomeDoorOrWindow.wallWidth",
    "HomeDoorOrWindow.wallLeft",
    "HomeDoorOrWindow.wallTop",
    "HomeDoorOrWindow.widthDepthDeformable",
    "HomeDoorOrWindow.boundToWall",
    "HomeLight.lightSources",
    "HomeLight.power",
    "HomeLight.lightSourceMaterialNames",
    "LightSource.color",
    "LightSource.diameter",
    "LightSource.x",
    "LightSource.y",
    "LightSource.z",
    "Sash.endAngle",
    "Sash.startAngle",
    "Sash.width",
    "Sash.xAxis",
    "Sash.yAxis",
    "HomeMaterial.color",
    "HomeMaterial.name",
    "HomeMaterial.shininess",
    "HomeMaterial.texture",
    "HomeMaterial.key",
    // Walls, rooms, annotations
    "Wall.height",
    "Wall.leftSideColor",
    "Wall.level",
    "Wall.rightSideColor",
    "Wall.thickness",
    "Wall.topColor",
    "Wall.xEnd",
    "Wall.xStart",
    "Wall.yEnd",
    "Wall.yStart",
    "Wall.arcExtent",
    "Wall.heightAtEnd",
    "Wall.leftSideBaseboard",
    "Wall.rightSideBaseboard",
    "Wall.leftSideTexture",
    "Wall.rightSideTexture",
    "Wall.leftSideShininess",
    "Wall.rightSideShininess",
    "Wall.pattern",
    // Joins are recomputed from matching end points; symmetry is derived.
    "Wall.wallAtEnd",
    "Wall.wallAtStart",
    "Wall.symmetric",
    "Baseboard.thickness",
    "Baseboard.height",
    "Baseboard.color",
    "Baseboard.texture",
    "Room.ceilingColor",
    "Room.ceilingFlat",
    "Room.ceilingVisible",
    "Room.floorColor",
    "Room.floorShininess",
    "Room.floorTexture",
    "Room.floorVisible",
    "Room.level",
    "Room.name",
    "Room.nameStyle",
    "Room.nameXOffset",
    "Room.nameYOffset",
    "Room.points",
    "Room.areaAngle",
    "Room.areaVisible",
    "Room.areaXOffset",
    "Room.areaYOffset",
    "Room.ceilingShininess",
    "Room.ceilingTexture",
    "Room.nameAngle",
    "Room.areaStyle",
    "Label.color",
    "Label.level",
    "Label.outlineColor",
    "Label.style",
    "Label.text",
    "Label.x",
    "Label.y",
    "Label.angle",
    "Label.elevation",
    "Label.pitch",
    "TextStyle.alignment",
    "TextStyle.bold",
    "TextStyle.fontSize",
    "TextStyle.italic",
    "TextStyle.fontName",
    "DimensionLine.endMarkSize",
    "DimensionLine.lengthStyle",
    "DimensionLine.level",
    "DimensionLine.offset",
    "DimensionLine.xEnd",
    "DimensionLine.xStart",
    "DimensionLine.yEnd",
    "DimensionLine.yStart",
    "DimensionLine.color",
    "DimensionLine.elevationStart",
    "DimensionLine.elevationEnd",
    "DimensionLine.pitch",
    "DimensionLine.visibleIn3D",
    "Polyline.color",
    "Polyline.level",
    "Polyline.points",
    "Polyline.thickness",
    "Polyline.closedPath",
    "Polyline.capStyleName",
    "Polyline.joinStyleName",
    "Polyline.dashStyleName",
    "Polyline.dashPattern",
    "Polyline.dashOffset",
    "Polyline.startArrowStyleName",
    "Polyline.endArrowStyleName",
    "Polyline.elevation",
    "Level.backgroundImage",
    "Level.elevation",
    "Level.elevationIndex",
    "Level.floorThickness",
    "Level.height",
    "Level.name",
    "Level.viewable",
    // Derived from `viewable` and the selected level.
    "Level.visible",
    "BackgroundImage.image",
    "BackgroundImage.invisible",
    "BackgroundImage.scaleDistance",
    "BackgroundImage.scaleDistanceXEnd",
    "BackgroundImage.scaleDistanceXStart",
    "BackgroundImage.scaleDistanceYEnd",
    "BackgroundImage.scaleDistanceYStart",
    "BackgroundImage.xOrigin",
    "BackgroundImage.yOrigin",
    "HomeTexture.height",
    "HomeTexture.image",
    "HomeTexture.leftToRightOriented",
    "HomeTexture.name",
    "HomeTexture.scale",
    "HomeTexture.width",
    "HomeTexture.angle",
    "HomeTexture.xOffset",
    "HomeTexture.yOffset",
    "HomeTexture.fittingArea",
    "URLContent.url",
    // Cameras, compass, environment, print
    "Camera.fieldOfView",
    "Camera.name",
    "Camera.pitch",
    "Camera.time",
    "Camera.x",
    "Camera.y",
    "Camera.yaw",
    "Camera.z",
    "Camera.lensName",
    "Camera.lens",
    "Camera.renderer",
    "ObserverCamera.fixedSize",
    "Compass.diameter",
    "Compass.latitude",
    "Compass.longitude",
    "Compass.northDirection",
    "Compass.timeZone",
    "Compass.x",
    "Compass.y",
    "Compass.visible",
    "HomeEnvironment.ceilingLightColor",
    "HomeEnvironment.drawingMode",
    "HomeEnvironment.groundColor",
    "HomeEnvironment.lightColor",
    "HomeEnvironment.observerCameraElevationAdjusted",
    "HomeEnvironment.photoHeight",
    "HomeEnvironment.photoWidth",
    "HomeEnvironment.skyColor",
    "HomeEnvironment.videoFrameRate",
    "HomeEnvironment.videoSpeed",
    "HomeEnvironment.videoWidth",
    "HomeEnvironment.cameraPath",
    "HomeEnvironment.allLevelsVisible",
    "HomeEnvironment.backgroundImageVisibleOnGround3D",
    "HomeEnvironment.photoQuality",
    "HomeEnvironment.subpartSizeUnderLight",
    "HomeEnvironment.videoQuality",
    "HomeEnvironment.wallsAlpha",
    "HomeEnvironment.groundTexture",
    "HomeEnvironment.skyTexture",
    "HomeEnvironment.photoAspectRatioName",
    "HomeEnvironment.videoAspectRatioName",
    "HomeEnvironment.photoAspectRatio",
    "HomeEnvironment.videoAspectRatio",
    "HomePrint.paperBottomMargin",
    "HomePrint.paperHeight",
    "HomePrint.paperLeftMargin",
    "HomePrint.paperOrientation",
    "HomePrint.paperRightMargin",
    "HomePrint.paperTopMargin",
    "HomePrint.paperWidth",
    "HomePrint.planPrinted",
    "HomePrint.planScale",
    "HomePrint.furniturePrinted",
    "HomePrint.view3DPrinted",
    "HomePrint.headerFormat",
    "HomePrint.footerFormat",
    "HomePrint.printedLevels",
];

#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// Every field holding a value in the sample is one the importer knows.
    #[test]
    #[ignore = "needs NEWERA_SH3D_SAMPLE=/path/to/file.sh3d"]
    fn every_stored_field_is_handled() {
        let path = std::env::var("NEWERA_SH3D_SAMPLE").expect("NEWERA_SH3D_SAMPLE");
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut bytes = Vec::new();
        archive
            .by_name("Home")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let graph = Graph::parse(&bytes).unwrap();
        let mut missing = std::collections::BTreeSet::new();
        for entry in &graph.entries {
            let Entry::Object(object) = entry else {
                continue;
            };
            for data in &object.data {
                let Some(class) = graph.class_desc(data.class) else {
                    continue;
                };
                let Some(simple) = class
                    .name
                    .strip_prefix(MODEL)
                    .or_else(|| class.name.strip_prefix("com.eteks.sweethome3d.tools."))
                else {
                    continue;
                };
                for (name, value) in &data.fields {
                    let empty = match value {
                        Value::Null => true,
                        Value::Bool(b) => !*b,
                        Value::Float(f) => *f == 0.0,
                        Value::Double(f) => *f == 0.0,
                        Value::Int(i) => *i == 0,
                        Value::Long(i) => *i == 0,
                        _ => false,
                    };
                    let key = format!("{simple}.{name}");
                    if !empty && !HANDLED.contains(&key.as_str()) {
                        missing.insert(key);
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "fields not handled by the importer: {missing:?}"
        );
    }
}
