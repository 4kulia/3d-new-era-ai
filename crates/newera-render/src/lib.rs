//! The home as a 3D scene: triangle meshes shared by the GPU view, and a
//! software renderer so servers and agents can see the home in 3D without
//! a graphics card.

mod asset_versions;
mod camera;
pub mod export;
pub mod mesh;
mod patterns;
pub mod photo;
mod raster;
pub mod text3d;
pub mod video;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use camera::{Side, View};
pub use mesh::{CUTAWAY_HEIGHT, Cutaway, IMAGE_BASE, Mesh, ModelSource, Selection, Vertex};
pub use photo::{thread_budget, with_thread_cap};
pub use raster::{RenderOptions, render};

/// Top-view images of pieces for the plan, rendered once and cached as PNG
/// files. Safe to share across threads.
#[derive(Debug, Clone)]
pub struct TopViews {
    inner: std::sync::Arc<std::sync::Mutex<TopViewCache>>,
    dir: PathBuf,
    assets: Option<PathBuf>,
    /// Also replace catalog symbols (otherwise only imported models).
    pub all: bool,
    /// Render missing images on a worker thread instead of blocking.
    background: Option<std::sync::mpsc::Sender<(u64, newera_core::Furniture)>>,
    /// Bumped whenever a background image becomes ready.
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    source_revision: &'static str,
}

#[derive(Debug, Default)]
struct TopViewCache {
    models: ModelCache,
    done: HashMap<u64, Option<String>>,
    queued: std::collections::HashSet<u64>,
}

impl TopViews {
    /// A provider that renders missing images right away (servers, exports).
    pub fn new(dir: PathBuf, assets: Option<PathBuf>, all: bool) -> Self {
        Self {
            inner: std::sync::Arc::default(),
            dir,
            assets,
            all,
            background: None,
            generation: std::sync::Arc::default(),
            source_revision: env!("NEWERA_SOURCE_REVISION"),
        }
    }

    /// A provider that renders missing images on a worker thread; they show
    /// up once [`TopViews::generation`] changes.
    pub fn in_background(dir: PathBuf, assets: Option<PathBuf>, all: bool) -> Self {
        let mut views = Self::new(dir, assets, all);
        let (sender, receiver) = std::sync::mpsc::channel::<(u64, newera_core::Furniture)>();
        let worker = views.clone();
        #[allow(
            clippy::disallowed_methods,
            reason = "the browser tab never asks for background views: `piece_images` returns early on wasm32"
        )]
        std::thread::spawn(move || {
            for (key, piece) in receiver {
                let image = worker.produce(&piece, key);
                let unchanged = worker.key(&piece) == Some(key);
                if let Ok(mut cache) = worker.inner.lock() {
                    cache.queued.remove(&key);
                    if unchanged {
                        cache.done.insert(key, image);
                    }
                }
                worker
                    .generation
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
        views.background = Some(sender);
        views
    }

    /// Changes whenever background images become ready.
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether images are still being rendered.
    pub fn busy(&self) -> bool {
        self.inner
            .lock()
            .is_ok_and(|c| c.queued.iter().any(|k| !c.done.contains_key(k)))
    }

    fn key(&self, piece: &newera_core::Furniture) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        if piece.is_opening() || piece.discipline.is_some() || (piece.model.is_none() && !self.all)
        {
            return None;
        }
        let mut copy = piece.clone();
        copy.position = newera_core::Point2::new(0.0, 0.0);
        copy.angle = 0.0;
        copy.elevation = 0.0;
        copy.level = None;
        copy.id = newera_core::FurnitureId(0);
        copy.name.clear();
        copy.info = newera_core::PieceInfo::default();
        copy.properties.clear();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Serialized, the look-defining fields hash stably.
        serde_json::to_string(&copy).ok()?.hash(&mut hasher);
        self.assets.hash(&mut hasher);
        self.source_revision.hash(&mut hasher);
        newera_catalog::GENERATOR_REVISION.hash(&mut hasher);
        let cache = self.inner.lock().ok()?;
        for part in piece.flatten() {
            let paths = part
                .model
                .iter()
                .chain(part.texture.iter().filter_map(|t| t.image.as_ref()))
                .chain(
                    part.materials
                        .iter()
                        .filter_map(|m| m.texture.as_ref()?.image.as_ref()),
                );
            for file in paths {
                let path = newera_core::resolve_asset(self.assets.as_deref(), file);
                path.hash(&mut hasher);
                cache.models.asset_version(&path).hash(&mut hasher);
            }
        }
        Some(hasher.finish())
    }

    /// The cached top view of a piece: rendered now, or queued when working
    /// in the background (then `None` until ready).
    pub fn image_for(&self, piece: &newera_core::Furniture) -> Option<String> {
        let key = self.key(piece)?;
        {
            let mut cache = self.inner.lock().ok()?;
            if let Some(done) = cache.done.get(&key) {
                return done.clone();
            }
            if cache.queued.contains(&key) {
                return None;
            }
            let path = self.dir.join(format!("{key:016x}.png"));
            if path.exists() {
                let found = Some(path.display().to_string());
                cache.done.insert(key, found.clone());
                return found;
            }
            if let Some(sender) = &self.background {
                if cache.queued.insert(key) && sender.send((key, piece.clone())).is_err() {
                    cache.queued.remove(&key);
                }
                return None;
            }
        }
        let image = self.produce(piece, key);
        if self.key(piece) != Some(key) {
            return None;
        }
        if let Ok(mut cache) = self.inner.lock() {
            cache.done.insert(key, image.clone());
        }
        image
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn produce(&self, piece: &newera_core::Furniture, key: u64) -> Option<String> {
        let path = self.dir.join(format!("{key:016x}.png"));
        let local = {
            let cache = self.inner.lock().ok()?;
            cache.models.piece_model(piece, self.assets.as_deref())
        }
        .unwrap_or_else(|| newera_catalog::piece_mesh(piece));
        let mesh = Mesh::piece_alone(piece, &local);
        let longest = piece.width.max(piece.depth).max(1.0);
        let px_per_cm = (256.0 / longest).min(2.0);
        let (w, h) = (
            ((piece.width * px_per_cm).round() as u32).max(8),
            ((piece.depth * px_per_cm).round() as u32).max(8),
        );
        let (hw, hd, top) = (
            (piece.width / 200.0) as f32,
            (piece.depth / 200.0) as f32,
            (piece.height / 100.0) as f32,
        );
        let view = glam::camera::rh::view::look_at_mat4(
            glam::Vec3::new(0.0, top + 1.0, 0.0),
            glam::Vec3::ZERO,
            glam::Vec3::NEG_Z,
        );
        let proj = glam::camera::rh::proj::directx::orthographic(-hw, hw, -hd, hd, 0.01, top + 2.0);
        let assets = self.assets.clone();
        let load = |file: &str| {
            newera_core::vfs::read(&newera_core::resolve_asset(assets.as_deref(), file))
                .ok()
                .and_then(|b| newera_core::images::decode(&b).ok())
                .map(|i| {
                    image::imageops::resize(
                        &i.to_rgba8(),
                        128,
                        128,
                        image::imageops::FilterType::Triangle,
                    )
                })
        };
        let image = render(
            &mesh,
            &RenderOptions {
                width: w,
                height: h,
                view_proj: proj * view,
                sky: [255, 255, 255],
                supersample: 2,
                load_image: &load,
                transparent: true,
                outlines: false,
                cut_color: None,
            },
        );
        // Never publish an image under a version that changed while rendering.
        if self.key(piece) != Some(key) {
            return None;
        }
        std::fs::create_dir_all(&self.dir).ok();
        publish_cache_file(&path, |file| {
            image
                .write_to(file, image::ImageFormat::Png)
                .map_err(std::io::Error::other)
        })
        .ok()
        .map(|()| path.display().to_string())
    }
}

// Readers (including other providers) must never observe a partially encoded PNG.
fn publish_cache_file(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    #[allow(
        clippy::disallowed_methods,
        reason = "only the disk-backed top views write here, which the browser tab never builds"
    )]
    let process = std::process::id();
    let temporary = path.with_extension(format!(
        "{process}.{}.tmp",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        write(&mut file)
    };
    let result = result.and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Reuses imported models until their source or companions change, then reloads.
/// Fits the cached mesh to each piece.
#[derive(Debug, Default)]
pub struct ModelCache {
    models: std::cell::RefCell<HashMap<PathBuf, CachedModel>>,
    versions: std::cell::RefCell<asset_versions::AssetVersions>,
}

#[derive(Debug)]
struct CachedModel {
    version: u64,
    mesh: Option<newera_catalog::Mesh>,
}

impl ModelCache {
    fn asset_version(&self, path: &Path) -> u64 {
        self.versions.borrow_mut().get(path)
    }
    /// Mesh of a piece's imported model, rotated and fitted to its box;
    /// `None` when it has no model or the file can't be read.
    pub fn piece_model(
        &self,
        piece: &newera_core::Furniture,
        assets: Option<&Path>,
    ) -> Option<newera_catalog::Mesh> {
        self.piece_model_seen(piece, assets, None)
    }

    /// [`Self::piece_model`] for a camera `distance` cm away: the piece's
    /// lighter file ([`newera_core::FarModel`]) when it is that far.
    pub fn piece_model_seen(
        &self,
        piece: &newera_core::Furniture,
        assets: Option<&Path>,
        distance: Option<f64>,
    ) -> Option<newera_catalog::Mesh> {
        let own = piece.model.as_deref()?;
        // A lighter file that cannot be read leaves the piece drawn in full,
        // never missing from afar.
        if let Some(far) = piece
            .model_far
            .as_ref()
            .and_then(|far| far.file_at(distance))
            && let Some(mesh) = self.fitted(piece, assets, far)
        {
            return Some(mesh);
        }
        self.fitted(piece, assets, own)
    }

    /// The model `file` fitted to the piece: turned, sized, its parts edited.
    fn fitted(
        &self,
        piece: &newera_core::Furniture,
        assets: Option<&Path>,
        file: &str,
    ) -> Option<newera_catalog::Mesh> {
        let path = newera_core::resolve_asset(assets, file);
        let version = self.asset_version(&path);
        let mut cache = self.models.borrow_mut();
        let cached = cache.entry(path.clone()).or_insert_with(|| CachedModel {
            version,
            mesh: newera_catalog::load_model(&path).ok().map(|m| m.mesh),
        });
        if cached.version != version {
            *cached = CachedModel {
                version,
                mesh: newera_catalog::load_model(&path).ok().map(|m| m.mesh),
            };
        }
        let mut mesh = cached.mesh.clone()?;
        mesh.rotate(piece.model_transform.rotation);
        mesh.fit_to(piece.width, piece.depth, piece.height);
        mesh.edit_parts(&piece.model_parts, true);
        Some(mesh)
    }
}

/// How far a piece's center is from a camera at `eye` (meters, world
/// axes), cm: what picks a lighter file for pieces far away.
#[must_use]
pub fn camera_distance(
    home: &newera_core::Home,
    piece: &newera_core::Furniture,
    eye: glam::Vec3,
) -> f64 {
    let floor = home.elevation_of(piece.level);
    let center = glam::DVec3::new(
        piece.position.x,
        floor + piece.elevation + piece.height / 2.0,
        piece.position.y,
    );
    (center - eye.as_dvec3() * 100.0).length()
}

/// `(min, max)` cm in a piece's model frame: x across, y up, z to its front.
pub type LocalBox = ([f32; 3], [f32; 3]);

/// A piece alone, as a product shot: the piece (and what is inside it)
/// on a plain ground, with no walls, rooms or other pieces, and the box
/// to frame — the whole piece, or its model's part `part` — in its model
/// frame (cm: x across, y up, z to its front).
///
/// # Errors
/// When the piece has no part of that name.
pub fn isolated(
    home: &newera_core::Home,
    piece: &newera_core::Furniture,
    part: Option<&str>,
    assets: Option<&Path>,
) -> Result<(newera_core::Home, LocalBox), String> {
    let mut alone = newera_core::Home::default();
    alone.levels.clone_from(&home.levels);
    alone.environment.all_levels_visible = true;
    alone.furniture.push(piece.clone());
    #[allow(clippy::cast_possible_truncation)]
    let whole = (
        [
            (-piece.width / 2.0) as f32,
            0.0,
            (-piece.depth / 2.0) as f32,
        ],
        [
            (piece.width / 2.0) as f32,
            piece.height as f32,
            (piece.depth / 2.0) as f32,
        ],
    );
    let Some(name) = part else {
        return Ok((alone, whole));
    };
    let mesh = ModelCache::default()
        .piece_model(piece, assets)
        .ok_or_else(|| format!("{} is not an imported model with parts", piece.id))?;
    let found = mesh.parts.iter().find(|p| p.name == name).ok_or_else(|| {
        let names: Vec<&str> = mesh.parts.iter().map(|p| p.name.as_str()).collect();
        format!("no part `{name}` in {} ({})", piece.id, names.join(", "))
    })?;
    let bounds = mesh
        .part_bounds(found)
        .ok_or_else(|| format!("part `{name}` has no triangles"))?;
    Ok((alone, bounds))
}

/// Exports the home's 3D model (`.glb` or `.obj`), with models and textures
/// resolved relative to `assets`. The ground plane is left out.
pub fn export_home(
    home: &newera_core::Home,
    path: &Path,
    assets: Option<&Path>,
) -> Result<(), export::ExportError> {
    let cache = ModelCache::default();
    let models = |piece: &newera_core::Furniture| cache.piece_model(piece, assets);
    let mut mesh = Mesh::from_home(home, &Selection::new(), &models);
    mesh.drop_ground();
    export::export_mesh(&mesh, path, &|file| export_image(assets, file))
}

/// An image a mesh's material names, as an export embeds it: PNG and JPEG
/// as they are, other formats converted to PNG.
fn export_image(assets: Option<&Path>, file: &str) -> Option<(Vec<u8>, String)> {
    let path = newera_core::resolve_asset(assets, file);
    let bytes = newera_core::vfs::read(&path).ok()?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map_or_else(|| "png".to_owned(), str::to_ascii_lowercase);
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
        Some((bytes, ext))
    } else {
        let image = newera_core::images::decode(&bytes).ok()?;
        let mut png = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .ok()?;
        Some((png, "png".to_owned()))
    }
}

/// The home as a binary glTF built in memory, with catalog furniture and no
/// image files (for environments without a file system).
pub fn glb_home(home: &newera_core::Home) -> Vec<u8> {
    let mut mesh = Mesh::from_home(home, &Selection::new(), &|_| None);
    mesh.drop_ground();
    export::glb(&mesh, &|_| None)
}

/// The same day as `time_ms` at `hour` local solar time for `longitude`.
pub fn at_local_hour(time_ms: i64, hour: f64, longitude: f64) -> i64 {
    const DAY: i64 = 86_400_000;
    #[allow(clippy::cast_possible_truncation)]
    let offset = ((hour - longitude / 15.0) * 3_600_000.0) as i64;
    time_ms.div_euclid(DAY) * DAY + offset
}

/// Unit vector towards the sun (y up, plan x/z) and its elevation in degrees
/// at `time_ms` for the compass location; `None` once it is well below the
/// horizon. Azimuth is clockwise from north and the compass says where north is.
#[allow(clippy::cast_possible_truncation)]
pub fn sun_direction(compass: &newera_core::Compass, time_ms: i64) -> Option<(glam::Vec3, f64)> {
    let (azimuth, elevation) = photo::sun_position(
        time_ms,
        compass.latitude.unwrap_or(-23.55),
        compass.longitude.unwrap_or(-46.63),
    );
    (elevation > -2.0).then(|| {
        let heading = (compass.north_degrees + azimuth).to_radians();
        let el = elevation.max(0.5).to_radians();
        let dir = glam::Vec3::new(
            (heading.sin() * el.cos()) as f32,
            el.sin() as f32,
            (-heading.cos() * el.cos()) as f32,
        )
        .normalize();
        (dir, elevation)
    })
}

/// Lux in one unit of scene irradiance of photos.
pub const LUX_PER_UNIT: f64 = 25_000.0;

/// How long a photo may take: samples per pixel and bounces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoQuality {
    Draft,
    Good,
    Best,
}

impl PhotoQuality {
    fn budget(self) -> (u32, u32) {
        match self {
            Self::Draft => (8, 2),
            Self::Good => (48, 3),
            Self::Best => (192, 4),
        }
    }
}

/// What a photo of the home would take, worked out without rendering it:
/// the work (pixels, samples, bounces, primary rays), the scene (triangles,
/// images) and memory estimated from their sizes. Time is not given — it
/// depends on a processor and its temperature, which are not measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoEstimate {
    pub pixels: u64,
    pub samples: u32,
    pub bounces: u32,
    /// Camera rays: pixels × samples (each bounces up to `bounces` times).
    pub rays: u64,
    pub triangles: u64,
    pub images: u64,
    /// Mesh, its ray-tracing tree, images (at 256×256) and the frame, bytes.
    pub memory_bytes: u64,
}

/// [`PhotoEstimate`] for [`photo_home`] with the same arguments.
#[must_use]
pub fn photo_estimate(
    home: &newera_core::Home,
    view: &View,
    width: u32,
    height: u32,
    assets: Option<&Path>,
    quality: PhotoQuality,
) -> PhotoEstimate {
    let cache = ModelCache::default();
    let models = |piece: &newera_core::Furniture| {
        cache.piece_model_seen(piece, assets, Some(camera_distance(home, piece, view.eye)))
    };
    let mesh = Mesh::from_home(home, &Selection::new(), &models);
    let (samples, bounces) = quality.budget();
    let pixels = u64::from(width) * u64::from(height);
    let triangles = ((mesh.indices.len() + mesh.transparent.len()) / 3) as u64;
    let images = mesh.images.len() as u64;
    let vertex = std::mem::size_of::<Vertex>() as u64;
    let memory_bytes = mesh.vertices.len() as u64 * vertex
        + (mesh.indices.len() + mesh.transparent.len()) as u64 * 4
        // Triangle list, flags and tree nodes of the path tracer.
        + triangles * (12 + 1 + 24)
        + images * 256 * 256 * 4
        // Color and surface per pixel.
        + pixels * 48;
    PhotoEstimate {
        pixels,
        samples,
        bounces,
        rays: pixels * u64::from(samples),
        triangles,
        images,
        memory_bytes,
    }
}

/// A photo of the home: sunlight from the compass location at the camera's
/// time (`time_ms`), light from lamps and glass-filtered daylight.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn photo_home(
    home: &newera_core::Home,
    view: &View,
    time_ms: i64,
    width: u32,
    height: u32,
    assets: Option<&Path>,
    quality: PhotoQuality,
) -> image::RgbaImage {
    let home = seen_from(home, view);
    let cache = ModelCache::default();
    let models = |piece: &newera_core::Furniture| {
        cache.piece_model_seen(piece, assets, Some(camera_distance(&home, piece, view.eye)))
    };
    let mesh = Mesh::from_home(&home, &Selection::new(), &models);
    let light = photo_light(&home, time_ms);
    let (samples, bounces) = quality.budget();
    let load = |file: &str| {
        newera_core::vfs::read(&newera_core::resolve_asset(assets, file))
            .ok()
            .and_then(|b| newera_core::images::decode(&b).ok())
            .map(|i| {
                image::imageops::resize(
                    &i.to_rgba8(),
                    256,
                    256,
                    image::imageops::FilterType::Triangle,
                )
            })
    };
    photo::render_photo(
        &mesh,
        &photo::PhotoOptions {
            width,
            height,
            view: *view,
            samples,
            bounces,
            sun: light.sun,
            sky: light.sky,
            lights: light.lamps,
            exposure: PHOTO_EXPOSURE,
            load_image: &load,
        },
    )
}

/// Exposure of photos on top of their auto exposure.
pub const PHOTO_EXPOSURE: f32 = 1.1;

/// What a photo of the home sees, for a path tracer of its own (Blender
/// Cycles on a GPU, `deploy/spark-renderer/`): the home as a binary glTF,
/// with the ground, every model in full and its textures, and the light
/// [`photo_home`] lights it with. Positions are the glTF's (meters, Y up);
/// light is in the photo's units of irradiance ([`LUX_PER_UNIT`]).
#[derive(Debug, Clone)]
pub struct PhotoScene {
    pub glb: Vec<u8>,
    pub view: View,
    pub light: PhotoLight,
}

/// The light of a photo.
#[derive(Debug, Clone)]
pub struct PhotoLight {
    /// The direction toward the sun and its irradiance per color channel,
    /// while it is up.
    pub sun: Option<(glam::Vec3, glam::Vec3)>,
    /// The sky's radiance per color channel, the same from every direction.
    pub sky: glam::Vec3,
    /// The lamps of the storeys shown.
    pub lamps: Vec<photo::PointLight>,
}

/// The scene [`photo_home`] renders from `view` at `time_ms`, for another
/// renderer ([`PhotoScene`]).
pub fn photo_scene(
    home: &newera_core::Home,
    view: &View,
    time_ms: i64,
    assets: Option<&Path>,
) -> PhotoScene {
    let home = seen_from(home, view);
    let cache = ModelCache::default();
    let models = |piece: &newera_core::Furniture| cache.piece_model(piece, assets);
    let mesh = Mesh::from_home(&home, &Selection::new(), &models);
    PhotoScene {
        glb: export::glb(&mesh, &|file| export_image(assets, file)),
        view: *view,
        light: photo_light(&home, time_ms),
    }
}

/// The home as a photo from `view` shows it. From above the walls, a
/// dollhouse: the path tracer sees both sides of every face, so room
/// ceilings would hide the inside that the 3D view (which culls back faces)
/// shows.
fn seen_from<'a>(
    home: &'a newera_core::Home,
    view: &View,
) -> std::borrow::Cow<'a, newera_core::Home> {
    let walls_top = home
        .walls
        .iter()
        .map(|w| home.elevation_of(w.level) + w.height.max(w.height_at_end.unwrap_or(0.0)))
        .fold(0.0, f64::max);
    if walls_top > 0.0 && f64::from(view.eye.y) * 100.0 > walls_top {
        let mut clone = home.clone();
        for room in &mut clone.rooms {
            room.ceiling_visible = false;
        }
        std::borrow::Cow::Owned(clone)
    } else {
        std::borrow::Cow::Borrowed(home)
    }
}

/// Sunlight from the compass location at `time_ms`, the sky, and the lamps.
#[allow(clippy::cast_possible_truncation)]
fn photo_light(home: &newera_core::Home, time_ms: i64) -> PhotoLight {
    use glam::Vec3;
    let sun = sun_direction(&home.compass, time_ms).map(|(dir, elevation)| {
        let strength = (elevation / 20.0).clamp(0.15, 1.0) as f32;
        (dir, Vec3::new(1.0, 0.95, 0.88) * 3.2 * strength)
    });
    // Sky light fades through twilight: about ten times less every 3° the
    // sun sinks below the horizon, down to a moonless night.
    let (_, elevation) = photo::sun_position(
        time_ms,
        home.compass.latitude.unwrap_or(-23.55),
        home.compass.longitude.unwrap_or(-46.63),
    );
    let daylight = match sun {
        Some((d, _)) if elevation > 0.0 => 0.35 + 0.65 * d.y,
        _ => (0.35 * 10f64.powf(elevation.min(0.0) / 3.0)).max(2e-6) as f32,
    };
    let sky_color = Vec3::from(home.environment.sky_color.map(f32::from)) / 255.0;
    // Twilight turns the sky deep blue.
    let dusk = (1.0 - daylight / 0.35).clamp(0.0, 1.0);
    let sky = sky_color.powf(2.2).lerp(Vec3::new(0.05, 0.09, 0.25), dusk) * 1.4 * daylight;
    let sun = sun.filter(|_| elevation > 0.0);
    PhotoLight {
        sun,
        sky,
        lamps: photo_lights(home),
    }
}

// Preserve each leaf fixture's optical size when its owner is a larger group.
#[allow(clippy::cast_possible_truncation)]
fn photo_lights(home: &newera_core::Home) -> Vec<photo::PointLight> {
    use glam::Vec3;
    let levels = mesh::shown_levels(home);
    // Lamps of the storeys shown, in photometric units: 1 scene unit of
    // irradiance is LUX_PER_UNIT lux (the noon sun is about 3).

    newera_core::lighting::emitters(home, &newera_catalog::light_for)
        .into_iter()
        .filter(|e| {
            let Some(owner) = home.furniture.iter().find(|f| f.id == e.piece) else {
                return false;
            };
            let Some(source) = home.find_piece(e.source) else {
                return false;
            };
            owner.visible
                && levels.contains(&home.resolve_level(owner.level))
                && home.shown_in_3d(owner.discipline, None)
                && home.shown_in_3d(
                    source.discipline,
                    newera_core::layer_in_group(owner, source),
                )
        })
        .map(|e| {
            let color = Vec3::new(e.color[0] as f32, e.color[1] as f32, e.color[2] as f32);
            let position = Vec3::new(
                e.position[0] as f32,
                e.position[2] as f32,
                e.position[1] as f32,
            ) * 0.01;
            let piece = home.find_piece(e.source);
            let size = piece.map_or(10.0, |f| f.width.max(f.depth)) as f32 * 0.01;
            match e.distribution {
                newera_core::lighting::Distribution::Area {
                    w,
                    d,
                    angle,
                    upward,
                } => {
                    let (sin, cos) = (angle as f32).to_radians().sin_cos();
                    let (hw, hd) = (w as f32 * 0.005, d as f32 * 0.005);
                    photo::PointLight {
                        position,
                        intensity: color
                            * (e.panel_luminance().unwrap_or(0.0) / LUX_PER_UNIT) as f32,
                        radius: 0.0,
                        half_angle: 0.0,
                        panel: Some((
                            Vec3::new(cos, 0.0, sin) * hw,
                            Vec3::new(-sin, 0.0, cos) * hd * if upward { -1.0 } else { 1.0 },
                        )),
                        clearance: 0.01,
                    }
                }
                newera_core::lighting::Distribution::Spot { half } => photo::PointLight {
                    position,
                    intensity: color * (e.peak() / LUX_PER_UNIT) as f32,
                    radius: (size * 0.3).clamp(0.01, 0.04),
                    half_angle: (half as f32).to_radians(),
                    panel: None,
                    clearance: size * 0.6,
                },
                newera_core::lighting::Distribution::Point => photo::PointLight {
                    position,
                    intensity: color * (e.peak() / LUX_PER_UNIT) as f32,
                    radius: 0.03,
                    half_angle: 0.0,
                    panel: None,
                    clearance: size * 0.6,
                },
            }
        })
        .collect()
}

/// Renders a home from a point of view: builds the mesh, loads models and
/// textures relative to `assets`, and rasterizes it in software.
pub fn render_home(
    home: &newera_core::Home,
    view: &View,
    width: u32,
    height: u32,
    assets: Option<&Path>,
) -> image::RgbaImage {
    render_home_cut(home, view, None, width, height, assets)
}

/// [`render_home`] with some walls brought down to show the rooms.
pub fn render_home_cut(
    home: &newera_core::Home,
    view: &View,
    cutaway: Option<&Cutaway>,
    width: u32,
    height: u32,
    assets: Option<&Path>,
) -> image::RgbaImage {
    let cache = ModelCache::default();
    let models = |piece: &newera_core::Furniture| {
        cache.piece_model_seen(piece, assets, Some(camera_distance(home, piece, view.eye)))
    };
    let mut mesh = Mesh::from_home_cut(home, &Selection::new(), &models, cutaway);
    // A section seen from above shows the walls it cuts as solid.
    if let (Some(near), Some(_)) = (view.near, view.ortho) {
        let dir = (view.eye - view.target).normalize_or_zero();
        if dir.y > 0.99 {
            mesh.add_section_caps(home, f64::from(view.eye.y - near) * 100.0);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    let load = |file: &str| {
        newera_core::vfs::read(&newera_core::resolve_asset(assets, file))
            .ok()
            .and_then(|b| newera_core::images::decode(&b).ok())
            .map(|i| {
                image::imageops::resize(
                    &i.to_rgba8(),
                    256,
                    256,
                    image::imageops::FilterType::Triangle,
                )
            })
    };
    render(
        &mesh,
        &RenderOptions {
            width,
            height,
            view_proj: view.view_proj(aspect),
            sky: home.environment.sky_color,
            supersample: 2,
            load_image: &load,
            transparent: false,
            outlines: true,
            cut_color: view
                .near
                .filter(|_| view.ortho.is_some())
                .map(|_| [30, 30, 34]),
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn photo_lights_follow_geometry_storeys_and_layer_visibility() {
        use newera_core::{
            Discipline, Furniture, FurnitureId, Home, Level, LevelId, Light, PlanLayer,
        };
        let mut home = Home::default();
        home.levels = vec![
            Level {
                id: LevelId(1),
                ..Level::default()
            },
            Level {
                id: LevelId(2),
                elevation: 300.0,
                ..Level::default()
            },
        ];
        home.selected_level = Some(LevelId(1));
        home.environment.all_levels_visible = false;
        let fixture = Furniture {
            id: FurnitureId(3),
            catalog: "downlight".into(),
            width: 10.0,
            depth: 10.0,
            height: 5.0,
            light: Some(Light::led(800.0, 3000.0, (0.5, 0.5, 0.0))),
            ..Furniture::default()
        };
        home.furniture.push(Furniture {
            id: FurnitureId(4),
            catalog: "group".into(),
            level: Some(LevelId(2)),
            children: vec![fixture],
            ..Furniture::default()
        });
        assert!(photo_lights(&home).is_empty(), "upper floor is not drawn");
        home.environment.all_levels_visible = true;
        assert_eq!(photo_lights(&home).len(), 1);
        home.levels[1].viewable = false;
        assert!(
            photo_lights(&home).is_empty(),
            "hidden floor must not light the visible scene"
        );
        home.levels[1].viewable = true;
        home.environment.all_levels_visible = false;
        home.selected_level = Some(LevelId(2));
        assert_eq!(photo_lights(&home).len(), 1);
        home.hidden_layers.push(PlanLayer::Lighting);
        assert_eq!(photo_lights(&home), [] as [crate::photo::PointLight; 0]);
        home.show_all_in_3d = true;
        assert_eq!(photo_lights(&home).len(), 1);
        home.show_all_in_3d = false;
        home.hidden_layers.clear();
        home.hidden_disciplines.push(Discipline::Electrical);
        assert_eq!(photo_lights(&home), [] as [crate::photo::PointLight; 0]);
        home.hidden_disciplines.clear();
        home.furniture[0].discipline = Some(Discipline::Plumbing);
        home.hidden_disciplines.push(Discipline::Plumbing);
        assert!(
            photo_lights(&home).is_empty(),
            "owner discipline hides its lights too"
        );
        home.show_all_in_3d = true;
        assert_eq!(photo_lights(&home).len(), 1);
        // Visibility is a render choice, not removal of the modeled fixture.
        assert_eq!(
            newera_core::lighting::emitters(&home, &newera_catalog::light_for).len(),
            1
        );
    }

    #[test]
    fn grouping_a_fixture_preserves_its_shadow_clearance_and_emission() {
        use newera_core::{Furniture, FurnitureId, Home, Light};
        for kind in ["point", "spot", "panel"] {
            let mut light = Light::led(800.0, 3000.0, (0.5, 0.5, 0.5));
            if kind == "spot" {
                light.beam = Some(60.0);
            }
            if kind == "panel" {
                light.area = Some([10.0, 10.0]);
                light.panel_upward = Some(true);
            }
            let fixture = Furniture {
                id: FurnitureId(1),
                catalog: "box".into(),
                width: 10.0,
                depth: 10.0,
                height: 5.0,
                elevation: 290.0,
                light: Some(light),
                ..Furniture::default()
            };
            let mut home = Home::default();
            home.furniture.push(fixture.clone());
            let before = photo_lights(&home);
            assert_eq!(before.len(), 1);
            assert!(
                before[0].clearance < 0.1,
                "small fixture must not ignore room-sized obstacles"
            );
            let nested = Furniture {
                id: FurnitureId(2),
                catalog: "group".into(),
                width: 400.0,
                depth: 500.0,
                children: vec![fixture],
                ..Furniture::default()
            };
            home.furniture = vec![Furniture {
                id: FurnitureId(3),
                catalog: "group".into(),
                width: 1000.0,
                depth: 1000.0,
                children: vec![nested],
                ..Furniture::default()
            }];
            let emitters = newera_core::lighting::emitters(&home, &newera_catalog::light_for);
            assert_eq!(emitters[0].piece, FurnitureId(3));
            assert_eq!(emitters[0].source, FurnitureId(1));
            assert_eq!(photo_lights(&home), before, "{kind} changed when grouped");
            home.furniture[0].children[0].children[0].visible = false;
            assert_eq!(photo_lights(&home), [] as [crate::photo::PointLight; 0]);
            home.furniture[0].children[0].children[0].visible = true;
            home.furniture[0].children[0].visible = false;
            assert_eq!(photo_lights(&home), [] as [crate::photo::PointLight; 0]);
            home.furniture[0].children[0].visible = true;
            home.furniture[0].visible = false;
            assert_eq!(photo_lights(&home), [] as [crate::photo::PointLight; 0]);
        }
    }

    use newera_core::{Command, Document, Point2, Room, Wall};

    use super::*;

    #[test]
    fn renders_walls_floor_and_sky() {
        let mut doc = Document::default();
        let pts = [(0.0, 0.0), (500.0, 0.0), (500.0, 400.0), (0.0, 400.0)];
        let mut commands: Vec<Command> = (0..4)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % 4]);
                Command::insert(Wall::new(
                    doc.new_wall_id(),
                    Point2::new(a.0, a.1),
                    Point2::new(b.0, b.1),
                ))
            })
            .collect();
        let mut room = Room::new(
            doc.new_room_id(),
            "Sala",
            pts.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        );
        room.floor_material = Some("wood".parse().unwrap());
        commands.push(Command::insert(room));
        doc.execute(Command::Batch { commands }).unwrap();

        let home = doc.home();
        let image = render_home(home, &View::aerial(home, -50.0, 40.0), 160, 120, None);
        assert_eq!(image.dimensions(), (160, 120));
        let sky = home.environment.sky_color;
        let top = image.get_pixel(2, 2).0;
        assert_eq!([top[0], top[1], top[2]], sky, "sky at the top corner");
        let center = image.get_pixel(80, 70).0;
        assert_ne!(
            [center[0], center[1], center[2]],
            sky,
            "the house fills the middle"
        );
        let distinct: std::collections::HashSet<[u8; 4]> = image.pixels().map(|p| p.0).collect();
        assert!(
            distinct.len() > 50,
            "shaded, textured pixels: {}",
            distinct.len()
        );
    }

    #[test]
    fn photos_from_above_look_into_the_rooms() {
        let mut home = newera_core::Home::default();
        let pts = [(0.0, 0.0), (500.0, 0.0), (500.0, 400.0), (0.0, 400.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            let id = home.new_wall_id();
            home.walls
                .push(Wall::new(id, Point2::new(a.0, a.1), Point2::new(b.0, b.1)));
        }
        let id = home.new_room_id();
        let mut room = Room::new(
            id,
            "Sala",
            pts.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        );
        room.floor_material = Some("#c83020".parse().unwrap());
        home.rooms.push(room);
        let image = photo_home(
            &home,
            &View::aerial(&home, -50.0, 80.0),
            1_782_043_200_000,
            48,
            36,
            None,
            PhotoQuality::Draft,
        );
        // The red floor, not the white ceiling over it.
        let [r, g, b, _] = image.get_pixel(24, 18).0;
        assert!(
            r > g.saturating_add(30) && r > b.saturating_add(30),
            "{r} {g} {b}"
        );
    }

    #[test]
    fn elevations_sections_and_outlines() {
        let mut home = newera_core::Home::default();
        // A 600 × 400 cm box of walls, 300 cm high, and a slab inside at 150 cm.
        let pts = [(0.0, 0.0), (600.0, 0.0), (600.0, 400.0), (0.0, 400.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            let id = home.new_wall_id();
            let mut wall = Wall::new(id, Point2::new(a.0, a.1), Point2::new(b.0, b.1));
            wall.height = 300.0;
            home.walls.push(wall);
        }
        let id = home.new_furniture_id();
        home.furniture.push(newera_core::Furniture {
            id,
            catalog: "box".into(),
            position: Point2::new(300.0, 200.0),
            width: 400.0,
            depth: 200.0,
            height: 20.0,
            elevation: 150.0,
            color: Some([250, 250, 250]),
            ..newera_core::Furniture::default()
        });
        let (w, h) = (200, 200);
        let pixel = |image: &image::RgbaImage, x: u32, y: u32| {
            let p = image.get_pixel(x, y).0;
            [p[0], p[1], p[2]]
        };
        // Front elevation: the wall fills a centered band 600 wide × 300 tall.
        let front = View::orthographic(&home, Side::Front, 1.0, None);
        let image = render_home(&home, &front, w, h, None);
        let sky = home.environment.sky_color;
        assert_ne!(pixel(&image, 100, 100), sky, "wall in the middle");
        assert_eq!(pixel(&image, 100, 5), sky, "sky above the building");
        // Outlines darken the building's silhouette edge.
        let columns: Vec<[u8; 3]> = (0..w).map(|x| pixel(&image, x, 100)).collect();
        let edge = columns.iter().position(|c| *c != sky).unwrap();
        let inner = columns[edge + 10];
        assert!(
            columns[edge].iter().zip(inner).all(|(a, b)| *a <= b),
            "{:?} {:?}",
            columns[edge],
            inner
        );

        // Section through the middle (y = 200): the slab's inside shows as the cut color.
        let section = View::orthographic(&home, Side::Front, 1.0, Some(200.0));
        let image = render_home(&home, &section, w, h, None);
        let cut = (0..h)
            .filter(|y| pixel(&image, 100, *y).iter().all(|c| *c < 45))
            .count();
        assert!(cut > 0, "cut faces visible");
        // Top view with the cut below the wall tops shows the floor between walls.
        let top = View::orthographic(&home, Side::Top, 1.0, Some(100.0));
        let image = render_home(&home, &top, w, h, None);
        assert_ne!(pixel(&image, 100, 100), pixel(&image, 2, 2));
    }
}

#[cfg(test)]
mod cache_tests;

/// A wall drawn with a shop tile's picture, as each renderer gets it: the
/// built-in one draws it at the tile's size, and the GLB the photo
/// renderer reads carries the picture and the same tiling.
#[cfg(test)]
mod finish_texture_tests {
    use crate::mesh::finish_texture_tests::tiled_wall;
    use crate::mesh::{IMAGE_BASE, Mesh};
    use crate::{Selection, Side, View};

    fn number(value: &serde_json::Value) -> usize {
        usize::try_from(value.as_u64().unwrap()).unwrap()
    }

    /// A JPEG of one tile in four quarters: red at its top left and bottom
    /// right, blue at the other two.
    fn swatch() -> Vec<u8> {
        let image = image::RgbImage::from_fn(64, 32, |x, y| {
            if (x < 32) == (y < 16) {
                image::Rgb([200, 30, 30])
            } else {
                image::Rgb([30, 30, 200])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn the_glb_carries_the_picture_and_its_tiling() {
        let mesh = Mesh::from_home(&tiled_wall([60.0, 30.0]), &Selection::new(), &|_| None);
        let jpeg = swatch();
        let glb = crate::export::glb(&mesh, &|file| {
            (file == "products/p1/finish.jpg").then(|| (jpeg.clone(), "jpg".to_owned()))
        });
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let bin = &glb[20 + json_len + 8..];
        assert_eq!(json["images"].as_array().unwrap().len(), 1, "{json}");
        assert_eq!(json["images"][0]["mimeType"], "image/jpeg");
        let view = &json["bufferViews"][number(&json["images"][0]["bufferView"])];
        let at = number(&view["byteOffset"]);
        let len = number(&view["byteLength"]);
        assert_eq!(&bin[at..at + len], jpeg.as_slice(), "the picture itself");
        assert_eq!(json["textures"], serde_json::json!([{"source": 0}]));
        // No sampler: glTF's default, which repeats the picture.
        assert!(json.get("samplers").is_none(), "{json}");
        let textured: Vec<&serde_json::Value> = json["materials"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["pbrMetallicRoughness"]["baseColorTexture"].is_object())
            .collect();
        assert_eq!(textured.len(), 1, "{json}");
        assert_eq!(
            textured[0]["pbrMetallicRoughness"]["baseColorTexture"],
            serde_json::json!({"index": 0})
        );
        assert_eq!(
            textured[0]["pbrMetallicRoughness"]["baseColorFactor"],
            serde_json::json!([1, 1, 1, 1]),
            "the picture untinted"
        );
        // Texture coordinates in tiles, as the editor draws them (glTF's v
        // runs down).
        let uvs = &json["accessors"][2];
        let view = &json["bufferViews"][number(&uvs["bufferView"])];
        let at = number(&view["byteOffset"]);
        let floats: Vec<f32> = bin[at..at + number(&view["byteLength"])]
            .chunks(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(floats.len(), mesh.vertices.len() * 2);
        let mut tiled = 0;
        for (v, uv) in mesh.vertices.iter().zip(floats.chunks(2)) {
            assert_eq!([uv[0], -uv[1]], v.uv);
            if v.kind == IMAGE_BASE && v.normal[2].abs() > 0.99 {
                assert!((uv[1] + v.position[1] * 100.0 / 30.0).abs() < 1e-3);
                tiled += 1;
            }
        }
        assert!(tiled >= 16, "{tiled}");
    }

    /// How often red gives way to blue along `pixels`: once per tile.
    fn red_to_blue(pixels: impl Iterator<Item = [u8; 4]>) -> usize {
        let mut last = None;
        let mut changes = 0;
        for [r, _, b, _] in pixels {
            let half = if i32::from(r) - i32::from(b) > 40 {
                Some('r')
            } else if i32::from(b) - i32::from(r) > 40 {
                Some('b')
            } else {
                None
            };
            if let Some(half) = half {
                if last == Some('r') && half == 'b' {
                    changes += 1;
                }
                last = Some(half);
            }
        }
        changes
    }

    /// The tiles the render shows across the wall, 50 cm above the floor,
    /// and up it, at its first metre.
    fn tiles_shown(tile: [f64; 2]) -> ([usize; 2], image::RgbaImage) {
        let home = tiled_wall(tile);
        let dir = std::path::PathBuf::from(format!(
            "/memory/finish-texture-render-{}-{}",
            tile[0], tile[1]
        ));
        newera_core::vfs::mount(&dir, [("products/p1/finish.jpg".to_owned(), swatch())]);
        let front = View::orthographic(&home, Side::Front, 1.5, None);
        let image = crate::render_home(&home, &front, 600, 400, Some(&dir));
        newera_core::vfs::unmount(&dir);
        let sky = home.environment.sky_color;
        let is_sky = |x: u32, y: u32| {
            let p = image.get_pixel(x, y).0;
            [p[0], p[1], p[2]] == sky
        };
        // The wall's height in the image, from its first column of wall.
        let column = 150;
        let rows: Vec<u32> = (0..image.height())
            .filter(|y| !is_sky(column, *y))
            .collect();
        let (top, bottom) = (rows[0], *rows.last().unwrap());
        // 50 cm above the floor: below the window.
        let row = bottom - (bottom - top) * 50 / 250;
        let across = red_to_blue((0..image.width()).map(|x| image.get_pixel(x, row).0));
        let up = red_to_blue((top..=bottom).map(|y| image.get_pixel(column, y).0));
        ([across, up], image)
    }

    #[test]
    fn the_built_in_renderer_draws_the_picture_at_the_tile_s_size() {
        // 600 cm of wall: ten 60 cm tiles, twenty 30 cm ones; 250 cm up it:
        // eight and a third 30 cm tiles, sixteen and two thirds 15 cm ones.
        let ([across, up], image) = tiles_shown([60.0, 30.0]);
        if let Ok(path) = std::env::var("NEWERA_TILE_RENDER_PNG") {
            image.save(path).unwrap();
        }
        assert!((9..=11).contains(&across), "{across}");
        assert!((8..=9).contains(&up), "{up}");
        let ([across, up], _) = tiles_shown([30.0, 15.0]);
        assert!((19..=21).contains(&across), "{across}");
        assert!((16..=17).contains(&up), "{up}");
    }
}
