//! Pictures and files: the plan, the 3D view, a photo, and the exports.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;

use newera_core::{Document, Point2};
use newera_draw::{RenderOptions, SceneOptions, SvgOptions, plan_scene, render_png, to_svg};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::NewEraMcp;
use super::reply::{Raw, forward, invalid, take_action};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct RenderParams {
    /// Width px (default 640, max 2048).
    pub(crate) w: Option<u32>,
    /// Height px (default 480, max 2048).
    pub(crate) h: Option<u32>,
    /// Plan region `[[minx,miny],[maxx,maxy]]`; default fits the drawing.
    /// The region is fitted to the image's aspect and grown on the short
    /// side — never cropped — so everything asked for is in the picture.
    pub(crate) region: Option<[Point2; 2]>,
    /// Instead of `region`: a room id or name to frame.
    pub(crate) room: Option<String>,
    /// With `room`: margin around it, cm (default 30).
    pub(crate) pad: Option<f64>,
    /// Draw the grid (default true).
    pub(crate) grid: Option<bool>,
    /// Background image opacity for this render (e.g. 0.5 to compare the
    /// drawing with the scanned reference; 0 hides it).
    pub(crate) bg: Option<f64>,
}
/// What `export` takes; each kind of file goes on to its own parameters.
#[derive(JsonSchema)]
#[allow(dead_code)] // a schema, never built
pub(crate) struct ExportSchema {
    /// The file to write; its extension picks the format.
    path: String,
    /// Default `plan`: the drawing or the 3D model. `cut_list`: the boards of the joinery.
    #[schemars(extend("enum" = ["plan", "cut_list"]))]
    what: Option<String>,
    /// For `plan`, `.png` only: width px (default 1600).
    w: Option<u32>,
    /// For `plan`, `.png` only: height px (default 1200).
    h: Option<u32>,
    /// For `plan`, `.pdf`: scale denominator (50 → 1:50); omitted fits the sheet.
    scale: Option<f64>,
    /// For `cut_list`: builds or drawn groups (default every one on this storey).
    ids: Option<Vec<String>>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExportParams {
    /// Output file: `.pdf`, `.svg`, `.png`, `.glb` or `.obj`.
    path: String,
    /// `.png` only: width px (default 1600).
    w: Option<u32>,
    /// `.png` only: height px (default 1200).
    h: Option<u32>,
    /// PDF scale denominator (50 → 1:50); omitted fits the sheet.
    scale: Option<f64>,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct PhotoParams {
    /// Default `aerial`.
    #[schemars(extend("enum" = ["aerial", "visitor"]))]
    view: Option<String>,
    /// Stored point of view index.
    cam: Option<usize>,
    /// Aerial turn, degrees (default 60).
    yaw: Option<f32>,
    /// Aerial height angle, degrees (default 40).
    pitch: Option<f32>,
    /// Default `draft`.
    #[schemars(extend("enum" = ["draft", "good", "best"]))]
    quality: Option<String>,
    /// Local solar hour, 0–24.
    hour: Option<f64>,
    /// Width px (default 480, 64..1600).
    w: Option<u32>,
    /// Height px (default 360, 64..1200).
    h: Option<u32>,
    /// Threads it may use, under the budget (half the cores, at most 4).
    threads: Option<usize>,
    /// Seconds it may take; past them it stops, and nothing is returned.
    max_s: Option<f64>,
    /// Answer the budget and what the photo would take, without rendering.
    #[serde(default)]
    estimate: bool,
    /// Stop the photo being rendered now (`sessions` shows it).
    #[serde(default)]
    cancel: bool,
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct Render3dParams {
    /// Default `aerial`; `front` to `top` are elevations.
    #[schemars(extend("enum" = ["aerial", "visitor", "front", "back", "left", "right", "top"]))]
    view: Option<String>,
    /// Stored point of view index (see cameras).
    cam: Option<usize>,
    /// Aerial turn, degrees.
    yaw: Option<f32>,
    /// Aerial height angle, degrees.
    pitch: Option<f32>,
    /// Aerial distance factor: 1 frames the building, 2 twice as far.
    zoom: Option<f32>,
    /// Elevations: section plane in plan cm (y for front/back, x for left/right,
    /// height for top); what lies between the viewer and it is cut away — walls
    /// and pieces alike. front views from large y, back from y=0.
    cut: Option<f64>,
    /// `up` (default), `cutaway` or `down`.
    walls: Option<String>,
    /// Storeys drawn: an id like `lv3` draws it and those below, `all`
    /// every one (default: as the editor shows them).
    level: Option<String>,
    /// Width px (default 480, 64..1600).
    w: Option<u32>,
    /// Height px (default 360, 64..1200).
    h: Option<u32>,
    /// Only this piece, framed from its own side.
    piece: Option<String>,
    /// With piece: frame this part of its model (`model` lists them).
    part: Option<String>,
    /// With piece: its reference photo at this index, beside the render, from the photo's side and part.
    #[serde(rename = "ref")]
    reference: Option<usize>,
}
/// Stops a photo past its `max_s`, passing progress and the caller's own
/// cancellation on.
struct Deadline {
    inner: Option<Arc<dyn newera_core::progress::Watcher>>,
    until: web_time::Instant,
    /// What stopped the render first: [`RUNNING`], [`LATE`] or [`STOPPED`].
    stop: std::sync::atomic::AtomicU8,
}

const RUNNING: u8 = 0;
/// `max_s` ran out.
const LATE: u8 = 1;
/// Cancelled from outside (cancel=true, the client gone).
const STOPPED: u8 = 2;

impl Deadline {
    fn new(
        inner: Option<Arc<dyn newera_core::progress::Watcher>>,
        until: web_time::Instant,
    ) -> Self {
        Self {
            inner,
            until,
            stop: std::sync::atomic::AtomicU8::new(RUNNING),
        }
    }

    /// Why the render stopped, decided once, at the first check that saw it.
    fn stopped(&self) -> u8 {
        use std::sync::atomic::Ordering;
        let now = if self.inner.as_ref().is_some_and(|w| w.cancelled()) {
            STOPPED
        } else if web_time::Instant::now() >= self.until {
            LATE
        } else {
            RUNNING
        };
        if now != RUNNING {
            let _ = self
                .stop
                .compare_exchange(RUNNING, now, Ordering::SeqCst, Ordering::SeqCst);
        }
        self.stop.load(Ordering::SeqCst)
    }
}

impl newera_core::progress::Watcher for Deadline {
    fn step(&self, what: &str, done: u64, of: u64) {
        if let Some(inner) = &self.inner {
            inner.step(what, done, of);
        }
    }
    fn cancelled(&self) -> bool {
        self.stopped() != RUNNING
    }
}

/// Plan options as the user sees them: backgrounds, and top views for
/// imported models.
#[cfg(not(target_arch = "wasm32"))]
fn scene_options_for(doc: &Document) -> SceneOptions {
    let views = newera_render::TopViews::new(
        newera_core::cache_dir().join("topviews"),
        doc.asset_dir(),
        false,
    );
    SceneOptions {
        show_background: true,
        piece_images: Some(newera_draw::PieceImages(std::sync::Arc::new(
            move |piece| views.image_for(piece),
        ))),
        ..SceneOptions::default()
    }
}

/// Match the browser canvas, which uses symbols instead of disk-backed top
/// views. Asking for the native cache directory calls `std::env::temp_dir`,
/// which panics on wasm32 and aborts the whole editor, even for an empty plan.
#[cfg(target_arch = "wasm32")]
fn scene_options_for(_doc: &Document) -> SceneOptions {
    SceneOptions {
        show_background: true,
        ..SceneOptions::default()
    }
}
impl NewEraMcp {
    fn render(
        &self,
        w: u32,
        h: u32,
        region: Option<[Point2; 2]>,
        grid: bool,
    ) -> Result<Vec<u8>, ErrorData> {
        self.render_with(w, h, region, grid, None)
    }

    fn render_with(
        &self,
        w: u32,
        h: u32,
        region: Option<[Point2; 2]>,
        grid: bool,
        bg: Option<f64>,
    ) -> Result<Vec<u8>, ErrorData> {
        let (w, h) = (w.clamp(64, 2048), h.clamp(64, 2048));
        let doc = self.document.read();
        let mut view = doc.home().level_view(doc.home().current_level());
        if let Some(opacity) = bg {
            let opacity = opacity.clamp(0.0, 1.0);
            let level = view.current_level();
            let target = match level.and_then(|id| view.levels.iter_mut().find(|l| l.id == id)) {
                Some(level) if level.background.is_some() => level.background.as_mut(),
                _ => view.background.as_mut(),
            };
            if let Some(background) = target {
                background.opacity = opacity;
                background.visible = opacity > 0.0;
            }
        }
        let scene = plan_scene(&view, &scene_options_for(&doc));
        let project = doc.asset_dir();
        drop(doc);
        let options = RenderOptions {
            width: w,
            height: h,
            grid,
            region: region.map(|[a, b]| (a, b)),
            ..RenderOptions::default()
        };
        let load = |path: &str| {
            // Through the file layer every other asset read uses, so a
            // server's jail applies here too.
            newera_core::vfs::read(&newera_core::resolve_asset(project.as_deref(), path))
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|img| img.to_rgba8())
        };
        render_png(&scene, &options, &load)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
}

#[tool_router(router = render_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        description = "PNG of the floor plan, exactly as the user sees it. region=[[minx,miny],[maxx,maxy]] frames a place — it is fitted to the image's aspect by growing the short side, never by cropping, so everything asked for is in the picture — or room=<id|name> with pad cm (default 30) frames a room without working the rectangle out. bg=0..1 overlays the background image to compare with the reference. Keep w/h small to save tokens."
    )]
    pub(crate) fn render_plan(
        &self,
        Parameters(p): Parameters<RenderParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let region = match &p.room {
            Some(raw) => {
                let doc = self.document.read();
                let home = doc.home().level_view(doc.home().current_level());
                let room = home
                    .rooms
                    .iter()
                    .find(|r| r.id.to_string() == *raw || r.name.eq_ignore_ascii_case(raw))
                    .ok_or_else(|| invalid(format!("no room `{raw}` on this storey")))?;
                let (min, max) = newera_core::element_bounds(&home, room.id.into())
                    .ok_or_else(|| invalid("that room has no outline"))?;
                let pad = p.pad.unwrap_or(30.0);
                Some([
                    Point2::new(min.x - pad, min.y - pad),
                    Point2::new(max.x + pad, max.y + pad),
                ])
            }
            None => p.region,
        };
        let png = self.render_with(
            p.w.unwrap_or(640),
            p.h.unwrap_or(480),
            region,
            p.grid.unwrap_or(true),
            p.bg,
        )?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            data,
            "image/png",
        )]))
    }
    #[tool(
        description = "Show the plan to the user as an interactive viewer (pan, zoom, 3D view, open in the editor) where the chat client can display one. Reply: a one-line summary for you; the drawing goes to the viewer. To look at the plan yourself use render_plan."
    )]
    pub(crate) fn show_plan(&self) -> CallToolResult {
        let doc = self.document.read();
        let home = doc.home().level_view(doc.home().current_level());
        let scene = plan_scene(
            &home,
            &SceneOptions {
                show_background: false,
                ..SceneOptions::default()
            },
        );
        let svg = to_svg(&scene, &SvgOptions::default());
        #[allow(clippy::cast_precision_loss)]
        // An empty sum is -0.0, and "-0 m²" reads as a bug: + 0.0 makes it 0.
        let area = (home.rooms.iter().map(newera_core::Room::area).sum::<f64>() / 1000.0).round()
            / 10.0
            + 0.0;
        let summary = serde_json::json!({
            "rooms": home.rooms.len(),
            "area": area,
            "walls": home.walls.len(),
            "pieces": home.furniture.len(),
        });
        let text = format!(
            "shown: {} rooms, {area} m², {} walls, {} pieces",
            home.rooms.len(),
            home.walls.len(),
            home.furniture.len()
        );
        let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
        result.structured_content = Some(serde_json::json!({
            "name": home.name,
            "svg": svg,
            "summary": summary,
            "editor": crate::output::EDITOR_URL,
        }));
        result
    }
    #[tool(
        description = "PNG of the home in 3D (software render with outlines, no GPU needed). view: front|back|left|right|top orthographic elevations — front looks from the plan's bottom edge (large y) toward y=0, back from y=0 toward large y, left from x=0, right from large x; cut=cm makes a section keeping only what is beyond that plane from the viewer (front cut=200 keeps y<200, so the wall at y=0 stays as the backdrop; to remove it look from back), aerial (default; frames the whole building; yaw degrees: 0 from east/+x, 90 from south/plan bottom (default 60); pitch down; zoom >1 farther), visitor (current visitor camera) or cam=i (stored point of view). walls=cutaway drops the walls between the eye and a room to 40 cm, walls=down drops them all; either hides ceilings, roofs and the doors, windows and wall pieces of lowered walls, to see the furniture from the side. piece=<id> draws it alone, seen from its own side (view; aerial is three-quarter), framed the same wherever it stands; part=<name> frames one part of its model; ref=i puts its reference photo beside it, from the photo's side. Keep w/h small."
    )]
    pub(crate) fn render_3d(
        &self,
        Parameters(p): Parameters<Render3dParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (w, h) = (
            p.w.unwrap_or(480).clamp(64, 1600),
            p.h.unwrap_or(360).clamp(64, 1200),
        );
        #[allow(clippy::cast_precision_loss)]
        let aspect = w as f32 / h as f32;
        let doc = self.document.read();
        // The storeys drawn are this call's, not the editor's.
        let mut home = doc.home().clone();
        match p.level.as_deref() {
            None => {}
            Some("all") => home.environment.all_levels_visible = true,
            Some(raw) => {
                let id: newera_core::LevelId = raw.parse().map_err(|_| {
                    invalid(format!("level: a storey id like lv3, or all (not {raw})"))
                })?;
                if home.level(id).is_none() {
                    return Err(invalid(format!("no storey {raw} (`levels` lists them)")));
                }
                home.selected_level = Some(id);
                home.environment.all_levels_visible = false;
            }
        }
        let assets = doc.asset_dir();
        drop(doc);
        let isolated;
        let mut photo: Option<String> = None;
        let (home, framed) = match &p.piece {
            // The piece frames its own shot, alone with no walls: a camera, an
            // angle, a section or lowered walls would be dropped without a word.
            Some(_)
                if p.cam.is_some()
                    || p.cut.is_some()
                    || p.yaw.is_some()
                    || p.pitch.is_some()
                    || matches!(p.walls.as_deref(), Some("cutaway" | "down")) =>
            {
                return Err(invalid(
                    "piece draws it alone, framed on its own: leave cam, cut, yaw, pitch and walls out",
                ));
            }
            Some(raw) => {
                let id: newera_core::FurnitureId =
                    raw.parse().map_err(|e| invalid(format!("piece: {e}")))?;
                let piece = home
                    .find_piece(id)
                    .ok_or_else(|| invalid(format!("no piece {raw}")))?;
                let reference = p
                    .reference
                    .map(|i| {
                        piece.references.get(i).cloned().ok_or_else(|| {
                            invalid(format!(
                                "{raw} has {} reference photos (`model id={raw}` lists them)",
                                piece.references.len()
                            ))
                        })
                    })
                    .transpose()?;
                let part = p
                    .part
                    .clone()
                    .or_else(|| reference.as_ref().and_then(|r| r.part.clone()));
                let (alone, bounds) =
                    newera_render::isolated(&home, piece, part.as_deref(), assets.as_deref())
                        .map_err(invalid)?;
                let wanted = p
                    .view
                    .clone()
                    .or_else(|| reference.as_ref().and_then(|r| r.view.clone()));
                photo = reference.map(|r| r.file);
                let side = match wanted.as_deref() {
                    None | Some("aerial") => None,
                    Some("front") => Some(newera_render::Side::Front),
                    Some("back") => Some(newera_render::Side::Back),
                    Some("left") => Some(newera_render::Side::Left),
                    Some("right") => Some(newera_render::Side::Right),
                    Some("top") => Some(newera_render::Side::Top),
                    Some(other) => {
                        return Err(invalid(format!(
                            "view `{other}` with piece: front, back, left, right, top or aerial"
                        )));
                    }
                };
                let view =
                    newera_render::View::product(&home, piece, bounds, side, p.zoom.unwrap_or(1.0));
                isolated = alone;
                (&isolated, Some(view))
            }
            None if p.part.is_some() || p.reference.is_some() => {
                return Err(invalid("part and ref: give the piece they belong to"));
            }
            None => (&home, None),
        };
        let view = match (framed, p.cam, p.view.as_deref()) {
            (Some(view), ..) => view,
            (None, cam, view) => match (cam, view) {
                (Some(i), _) => {
                    let camera = home
                        .cameras
                        .stored
                        .get(i)
                        .ok_or_else(|| invalid(format!("no stored camera {i}")))?;
                    newera_render::View::from_camera(camera, aspect)
                }
                (None, Some("visitor")) => {
                    newera_render::View::from_camera(&home.cameras.observer, aspect)
                }
                (None, None | Some("aerial")) => newera_render::View::aerial_zoom(
                    home,
                    p.yaw.unwrap_or(60.0),
                    p.pitch.unwrap_or(40.0),
                    p.zoom.unwrap_or(1.0),
                ),
                (None, Some(side @ ("front" | "back" | "left" | "right" | "top"))) => {
                    let side = match side {
                        "front" => newera_render::Side::Front,
                        "back" => newera_render::Side::Back,
                        "left" => newera_render::Side::Left,
                        "right" => newera_render::Side::Right,
                        _ => newera_render::Side::Top,
                    };
                    newera_render::View::orthographic(home, side, aspect, p.cut)
                }
                (None, Some(other)) => return Err(invalid(format!("unknown view `{other}`"))),
            },
        };
        let cutaway = match p.walls.as_deref() {
            None | Some("up") => None,
            Some("cutaway") => {
                let toward = view.target - view.eye;
                Some(newera_render::Cutaway::facing(
                    home,
                    Point2::new(f64::from(toward.x), f64::from(toward.z)),
                ))
            }
            Some("down") => Some(newera_render::Cutaway::all(home)),
            Some(other) => return Err(invalid(format!("unknown walls `{other}`"))),
        };
        let mut image =
            newera_render::render_home_cut(home, &view, cutaway.as_ref(), w, h, assets.as_deref());
        // The product photo on the left, the model as drawn on the right.
        if let Some(file) = photo {
            let path = newera_core::resolve_asset(assets.as_deref(), &file);
            let reference = newera_core::vfs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| newera_core::images::decode(&b).map_err(|e| e.to_string()))
                .map_err(|e| invalid(format!("{file}: {e}")))?
                .to_rgba8();
            image = side_by_side(&reference, &image);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| invalid(e.to_string()))?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            data,
            "image/png",
        )]))
    }
    #[tool(
        description = "Realistic photo (path traced: sun from compass location and time, lamps, glass). cam=i stored view, or view=visitor/aerial (yaw,pitch). quality draft (~10 s) | good | best; hour = local solar time (e.g. 9, 15.5, 20); w/h small. Returns PNG. One runs at a time, on at most half the cores (4): threads lowers that, max_s stops it past a deadline, estimate=true answers the budget and the work (pixels, rays, triangles, memory) without rendering, cancel=true stops the one running (sessions shows it)."
    )]
    pub(crate) fn render_photo(
        &self,
        Parameters(p): Parameters<PhotoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.render_photo_since(&p, web_time::Instant::now())
    }

    /// [`Self::render_photo`] for a call that started at `started`: its
    /// `max_s` counts from then (time already spent on another renderer).
    pub(crate) fn render_photo_since(
        &self,
        p: &PhotoParams,
        started: web_time::Instant,
    ) -> Result<CallToolResult, ErrorData> {
        if p.cancel {
            #[cfg(not(target_arch = "wasm32"))]
            let stopped = super::native_job::cancel_running();
            #[cfg(target_arch = "wasm32")]
            let stopped = false;
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                if stopped {
                    "cancelled: the render running stops at its next row; nothing in the project changed"
                } else {
                    "no render is running"
                },
            )]));
        }
        let (w, h) = photo_size(p);
        let quality = photo_quality(p)?;
        let doc = self.document.read();
        let home = doc.home().clone();
        let assets = doc.asset_dir();
        drop(doc);
        let (view, time) = photo_view(&home, p, w, h)?;
        let budget = newera_render::thread_budget();
        let threads = p.threads.unwrap_or(budget).clamp(1, budget);
        if p.estimate {
            let e = newera_render::photo_estimate(&home, &view, w, h, assets.as_deref(), quality);
            let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
            #[allow(clippy::cast_precision_loss)]
            let answer = serde_json::json!({
                "budget": {"threads": threads, "of": budget, "cores": cores, "one_at_a_time": true},
                "work": {"pixels": e.pixels, "samples": e.samples, "bounces": e.bounces, "rays": e.rays,
                         "triangles": e.triangles, "images": e.images},
                "memory_mb": (e.memory_bytes / 104_858) as f64 / 10.0,
                "measured": "memory is estimated from the sizes; time, temperature and power are not measured here",
            });
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                answer.to_string(),
            )]));
        }
        if p.max_s.is_some_and(|s| !(s.is_finite() && s > 0.0)) {
            return Err(invalid("max_s: seconds above 0"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        super::native_job::THREADS.store(threads, std::sync::atomic::Ordering::Relaxed);
        let render = || {
            newera_render::with_thread_cap(threads, || {
                newera_render::photo_home(&home, &view, time, w, h, assets.as_deref(), quality)
            })
        };
        // A deadline too far to be counted is none.
        let until = p.max_s.and_then(|seconds| {
            let span = std::time::Duration::try_from_secs_f64(seconds).ok()?;
            Some((seconds, started.checked_add(span)?))
        });
        if let Some((seconds, until)) = until
            && web_time::Instant::now() >= until
        {
            return Err(invalid(photo_late(seconds)));
        }
        let image = match until {
            None => render(),
            Some((seconds, until)) => {
                let deadline = Arc::new(Deadline::new(newera_core::progress::listener(), until));
                let watcher: Arc<dyn newera_core::progress::Watcher> = deadline.clone();
                let image = newera_core::progress::watched(&watcher, render);
                // Only what the render itself saw: one that finished in time
                // is not refused by a late look at the clock.
                match deadline.stop.load(std::sync::atomic::Ordering::SeqCst) {
                    LATE => {
                        return Err(invalid(photo_late(seconds)));
                    }
                    STOPPED => return Err(invalid(newera_core::progress::CANCELLED)),
                    _ => image,
                }
            }
        };
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| invalid(e.to_string()))?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            data,
            "image/png",
        )]))
    }
    /// What [`Self::render_photo`] would render, for a renderer outside
    /// this process; `None` when the call renders nothing (`estimate`,
    /// `cancel`).
    pub(crate) fn photo_scene(&self, p: &PhotoParams) -> Result<Option<PhotoAsk>, ErrorData> {
        if p.cancel || p.estimate {
            return Ok(None);
        }
        let (width, height) = photo_size(p);
        let quality = photo_quality(p)?;
        if p.max_s.is_some_and(|s| !(s.is_finite() && s > 0.0)) {
            return Err(invalid("max_s: seconds above 0"));
        }
        let doc = self.document.read();
        let home = doc.home().clone();
        let assets = doc.asset_dir();
        drop(doc);
        let (view, time) = photo_view(&home, p, width, height)?;
        Ok(Some(PhotoAsk {
            scene: newera_render::photo_scene(&home, &view, time, assets.as_deref()),
            width,
            height,
            quality,
            max_s: p.max_s,
        }))
    }
    #[tool(
        description = "Export to a file, by its extension. what=plan (default): the plan as .pdf (A3; scale=50/100 or fit), .svg (true scale) or .png, or the 3D model as .glb or .obj. what=cut_list: the joinery's cut list (and that of groups drawn by hand) as .csv (spreadsheet), .dxf (boards laid out on sheets, for CNC) or .svg (sheets to view); `cut_list` reads it."
    )]
    pub(crate) fn export(
        &self,
        Parameters(Raw(mut args, _)): Parameters<Raw<ExportSchema>>,
    ) -> Result<String, ErrorData> {
        match take_action(&mut args, "what", &["plan", "cut_list"], Some("plan"))?.as_str() {
            "cut_list" => self.export_cut_list(Parameters(forward(args)?)),
            _ => self.export_plan(Parameters(forward(args)?)),
        }
    }
    /// Writes the plan, or the 3D model, to a file.
    pub(crate) fn export_plan(
        &self,
        Parameters(p): Parameters<ExportParams>,
    ) -> Result<String, ErrorData> {
        let path = PathBuf::from(&p.path);
        let bytes = match path.extension().and_then(|e| e.to_str()) {
            Some("glb" | "obj") => {
                let (home, assets) = {
                    let doc = self.document.read();
                    (doc.home().clone(), doc.asset_dir())
                };
                newera_render::export_home(&home, &path, assets.as_deref())
                    .map_err(|e| invalid(e.to_string()))?;
                return Ok(format!("ok {}", path.display()));
            }
            Some("pdf") => {
                let doc = self.document.read();
                let view = doc.home().level_view(doc.home().current_level());
                let scene = plan_scene(&view, &scene_options_for(&doc));
                newera_draw::to_pdf(
                    &scene,
                    &newera_draw::PdfOptions {
                        scale: p.scale,
                        title: doc.home().name.clone(),
                        ..newera_draw::PdfOptions::default()
                    },
                )
            }
            Some("svg") => {
                let doc = self.document.read();
                let view = doc.home().level_view(doc.home().current_level());
                let scene = plan_scene(&view, &scene_options_for(&doc));
                to_svg(&scene, &SvgOptions::default()).into_bytes()
            }
            Some("png") => self.render(p.w.unwrap_or(1600), p.h.unwrap_or(1200), None, false)?,
            _ => return Err(invalid("path must end with .pdf, .svg, .png, .glb or .obj")),
        };
        std::fs::write(&path, bytes)
            .map_err(|e| invalid(format!("cannot write {}: {e}", path.display())))?;
        Ok(format!("ok {}", path.display()))
    }
}

/// What a photo that ran out of its `max_s` answers.
pub(crate) fn photo_late(seconds: f64) -> String {
    format!(
        "stopped after max_s={seconds} s; nothing in the project changed (estimate=true says what it takes; draft, fewer pixels or threads cost less)"
    )
}

/// A photo asked for, ready for a renderer outside this process.
#[derive(Debug, Clone)]
pub struct PhotoAsk {
    pub scene: newera_render::PhotoScene,
    pub width: u32,
    pub height: u32,
    pub quality: newera_render::PhotoQuality,
    /// Seconds it may take, when the call said.
    pub max_s: Option<f64>,
}

/// The photo's size in pixels, as asked or by default, within its bounds.
fn photo_size(p: &PhotoParams) -> (u32, u32) {
    (
        p.w.unwrap_or(480).clamp(64, 1600),
        p.h.unwrap_or(360).clamp(64, 1200),
    )
}

fn photo_quality(p: &PhotoParams) -> Result<newera_render::PhotoQuality, ErrorData> {
    match p.quality.as_deref() {
        None | Some("draft") => Ok(newera_render::PhotoQuality::Draft),
        Some("good") => Ok(newera_render::PhotoQuality::Good),
        Some("best") => Ok(newera_render::PhotoQuality::Best),
        Some(other) => Err(invalid(format!("unknown quality `{other}`"))),
    }
}

/// The photo's point of view and the moment its daylight is of.
fn photo_view(
    home: &newera_core::Home,
    p: &PhotoParams,
    w: u32,
    h: u32,
) -> Result<(newera_render::View, i64), ErrorData> {
    #[allow(clippy::cast_precision_loss)]
    let aspect = w as f32 / h as f32;
    let (view, time) = match (p.cam, p.view.as_deref()) {
        (Some(i), _) => {
            let camera = home
                .cameras
                .stored
                .get(i)
                .ok_or_else(|| invalid(format!("no stored camera {i}")))?;
            (
                newera_render::View::from_camera(camera, aspect),
                camera.time,
            )
        }
        (None, Some("visitor")) => (
            newera_render::View::from_camera(&home.cameras.observer, aspect),
            home.cameras.observer.time,
        ),
        (None, None | Some("aerial")) => (
            newera_render::View::aerial(home, p.yaw.unwrap_or(60.0), p.pitch.unwrap_or(40.0)),
            home.cameras.top.time,
        ),
        (None, Some(other)) => return Err(invalid(format!("unknown view `{other}`"))),
    };
    let time = p.hour.map_or(time, |hour| {
        newera_render::at_local_hour(time, hour, home.compass.longitude.unwrap_or(-46.63))
    });
    Ok((view, time))
}

/// The product photo at the render's height, at most 1600 px wide and in
/// its own proportions (a very wide one is shorter, centered), then the render.
fn side_by_side(photo: &image::RgbaImage, render: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = render.dimensions();
    let (pw, ph) = (photo.width().max(1), photo.height().max(1));
    let scale = (f64::from(h) / f64::from(ph)).min(1600.0 / f64::from(pw));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let size = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
    let (width, height) = (size(pw), size(ph).min(h));
    let left = image::imageops::resize(photo, width, height, image::imageops::FilterType::Triangle);
    let mut both = image::RgbaImage::from_pixel(width + w, h, image::Rgba([255; 4]));
    image::imageops::overlay(&mut both, &left, 0, i64::from((h - height) / 2));
    image::imageops::overlay(&mut both, render, i64::from(width), 0);
    both
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_render_stopped_from_outside_is_cancelled_not_late() {
        use super::{Deadline, LATE, RUNNING, STOPPED};
        use std::sync::Arc;
        struct Cancelled;
        impl newera_core::progress::Watcher for Cancelled {
            fn step(&self, _: &str, _: u64, _: u64) {}
            fn cancelled(&self) -> bool {
                true
            }
        }
        let later = web_time::Instant::now() + std::time::Duration::from_secs(3600);
        let outside = Deadline::new(Some(Arc::new(Cancelled)), later);
        assert_eq!(outside.stopped(), STOPPED);
        let late = Deadline::new(None, web_time::Instant::now());
        assert_eq!(late.stopped(), LATE);
        // Decided at the first look that saw it, not later.
        let running = Deadline::new(None, later);
        assert_eq!(
            running.stop.load(std::sync::atomic::Ordering::SeqCst),
            RUNNING
        );
    }
    #[test]
    fn a_very_wide_photo_keeps_its_proportions_beside_the_render() {
        let photo = image::RgbaImage::from_pixel(4000, 100, image::Rgba([200, 30, 30, 255]));
        let render = image::RgbaImage::from_pixel(96, 72, image::Rgba([0, 0, 0, 255]));
        let both = side_by_side(&photo, &render);
        // 1600 wide at most, so 40 tall, centered in the render's 72.
        assert_eq!(both.dimensions(), (1600 + 96, 72));
        assert_eq!(both.get_pixel(800, 2).0, [255; 4]);
        assert_eq!(both.get_pixel(800, 36).0, [200, 30, 30, 255]);
        // An ordinary one is as tall as the render.
        let photo = image::RgbaImage::from_pixel(200, 100, image::Rgba([200, 30, 30, 255]));
        assert_eq!(side_by_side(&photo, &render).dimensions(), (144 + 96, 72));
    }

    use super::*;
    use crate::edit::CreateParams;
    use crate::tools::server;

    #[test]
    fn create_then_render_returns_a_png_image() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Sala","at":[200,150]}]}"#,
        )
        .unwrap();
        assert_eq!(
            s.create(Parameters(params)).unwrap(),
            "ok rev=1 ids=w1,w2,w3,w4,r5"
        );
        let result = s
            .render_plan(Parameters(RenderParams {
                w: Some(200),
                h: Some(150),
                ..RenderParams::default()
            }))
            .unwrap();
        let ContentBlock::Image(image) = &result.content[0] else {
            panic!("expected image")
        };
        assert_eq!(image.mime_type, "image/png");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data)
            .unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
    }
    #[test]
    fn a_piece_alone_is_the_same_shot_wherever_it_stands_and_leaves_the_room_out() {
        let dir = std::env::temp_dir().join(format!("newera-render-piece-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("banco.obj"),
            "o assento\nv 0 40 0\nv 50 40 0\nv 50 40 40\nv 0 40 40\nf 1 4 3 2\n\
             o perna\nv 0 0 0\nv 5 0 0\nv 5 40 0\nv 0 40 0\nf 1 2 3 4\n"
                .replace("f 1 2 3 4", "f 5 6 7 8"),
        )
        .unwrap();
        let file = dir
            .join("banco.obj")
            .display()
            .to_string()
            .replace('\\', "/");
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[600,0],[600,400],[0,400]],"closed":true}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        s.place(Parameters(
            serde_json::from_str(&format!(
                r#"{{"items":[{{"model":"{file}","at":[150,150],"unit":"cm"}},{{"cat":"sofa-3","at":[150,260]}}]}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        let shot = |json: &str| -> Vec<u8> {
            let result = s
                .render_3d(Parameters(serde_json::from_str(json).unwrap()))
                .unwrap();
            let ContentBlock::Image(image) = &result.content[0] else {
                panic!("expected image")
            };
            base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .unwrap()
        };
        let id = {
            let doc = s.document.read();
            doc.home()
                .furniture
                .iter()
                .find(|f| f.model.is_some())
                .unwrap()
                .id
                .to_string()
        };
        let alone = shot(&format!(
            r#"{{"piece":"{id}","view":"front","w":96,"h":72}}"#
        ));
        // Moved across the room, with the sofa still behind it: the same shot.
        s.move_elements(Parameters(
            serde_json::from_str(&format!(r#"{{"ids":["{id}"],"dx":300,"dy":50}}"#)).unwrap(),
        ))
        .unwrap();
        let moved = shot(&format!(
            r#"{{"piece":"{id}","view":"front","w":96,"h":72}}"#
        ));
        let (a, b) = (
            image::load_from_memory(&alone).unwrap().to_rgba8(),
            image::load_from_memory(&moved).unwrap().to_rgba8(),
        );
        let differ = a
            .pixels()
            .zip(b.pixels())
            .filter(|(p, q)| p.0.iter().zip(q.0).any(|(x, y)| x.abs_diff(y) > 24))
            .count();
        assert!(
            differ < a.pixels().len() / 50,
            "same piece, same shot: {differ} pixels differ"
        );
        let leg = shot(&format!(
            r#"{{"piece":"{id}","part":"perna","w":96,"h":72}}"#
        ));
        assert_ne!(leg, alone);
        let wrong = s
            .render_3d(Parameters(
                serde_json::from_str(&format!(r#"{{"piece":"{id}","part":"braco"}}"#)).unwrap(),
            ))
            .unwrap_err();
        assert!(
            wrong.message.contains("assento") && wrong.message.contains("perna"),
            "{wrong:?}"
        );
        assert!(
            s.render_3d(Parameters(
                serde_json::from_str(r#"{"part":"perna"}"#).unwrap()
            ))
            .is_err()
        );
        // The piece frames its own shot: a camera or a section is refused.
        for extra in [
            r#""cam":0"#,
            r#""cut":100"#,
            r#""yaw":30"#,
            r#""pitch":10"#,
            r#""walls":"down""#,
        ] {
            let refused = s
                .render_3d(Parameters(
                    serde_json::from_str(&format!(r#"{{"piece":"{id}",{extra}}}"#)).unwrap(),
                ))
                .unwrap_err();
            assert!(refused.message.contains("leave cam, cut"), "{refused:?}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_photo_says_its_budget_and_cost_first_and_stops_at_its_deadline() {
        let s = server();
        s.create(Parameters(
            serde_json::from_str(
                r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}]}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let photo = |json: &str| s.render_photo(Parameters(serde_json::from_str(json).unwrap()));
        let text = |result: CallToolResult| {
            let ContentBlock::Text(t) = &result.content[0] else {
                panic!("expected text")
            };
            t.text.clone()
        };
        let estimate: serde_json::Value = serde_json::from_str(&text(
            photo(r#"{"estimate":true,"threads":1,"w":100,"h":80,"quality":"good"}"#).unwrap(),
        ))
        .unwrap();
        assert_eq!(estimate["budget"]["threads"], 1);
        assert_eq!(estimate["work"]["pixels"], 8000);
        assert_eq!(estimate["work"]["rays"], 8000 * 48);
        assert!(estimate["work"]["triangles"].as_u64().unwrap() > 0);
        assert!(estimate["memory_mb"].as_f64().unwrap() > 0.0);
        assert!(
            estimate["measured"]
                .as_str()
                .unwrap()
                .contains("not measured")
        );
        let late =
            photo(r#"{"quality":"best","w":400,"h":300,"threads":1,"max_s":0.05}"#).unwrap_err();
        assert!(
            late.message.contains("stopped after max_s=0.05"),
            "{late:?}"
        );
        // A deadline too far to count is no deadline, not a crash.
        photo(r#"{"quality":"draft","w":64,"h":64,"threads":1,"max_s":1e300}"#).unwrap();
        // cancel=true acts on whatever render runs in this process, so it is
        // tried where one is known to run (native_job's tests).
    }
    #[test]
    fn render_3d_returns_a_png() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Sala","at":[200,150],"floor_mat":"wood"}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let result = s
            .render_3d(Parameters(Render3dParams {
                w: Some(96),
                h: Some(72),
                ..Render3dParams::default()
            }))
            .unwrap();
        let ContentBlock::Image(image) = &result.content[0] else {
            panic!("expected image")
        };
        assert_eq!(image.mime_type, "image/png");
        assert!(
            s.render_3d(Parameters(Render3dParams {
                cam: Some(3),
                ..Render3dParams::default()
            }))
            .is_err()
        );
    }
    #[test]
    fn render_3d_draws_the_storeys_it_is_told() {
        let s = server();
        let walls = r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}]}"#;
        s.create(Parameters(serde_json::from_str(walls).unwrap()))
            .unwrap();
        let upper = {
            let mut doc = s.document.write();
            let upper = newera_core::ops::add_level(&mut doc, None, None).unwrap();
            let ground = doc.home().base_level();
            doc.select_level(ground);
            upper
        };
        // Upstairs, a tall box that shows from the air.
        s.create(Parameters(
            serde_json::from_str(r#"{"walls":[{"pts":[[100,100],[300,100]],"h":250}]}"#).unwrap(),
        ))
        .unwrap();
        {
            let mut doc = s.document.write();
            let id = doc.home().walls.last().unwrap().id;
            let mut wall = doc.home().wall(id).unwrap().clone();
            wall.level = Some(upper);
            doc.execute(newera_core::Command::update(wall)).unwrap();
        }
        let shown = s.document.read().home().current_level();
        let png = |level: Option<&str>| {
            let result = s
                .render_3d(Parameters(Render3dParams {
                    level: level.map(Into::into),
                    w: Some(96),
                    h: Some(72),
                    ..Render3dParams::default()
                }))
                .unwrap();
            let ContentBlock::Image(image) = &result.content[0] else {
                panic!("expected image")
            };
            image.data.clone()
        };
        let ground_only = png(None);
        let all = png(Some("all"));
        assert_ne!(all, ground_only, "the upper storey is drawn");
        assert_eq!(
            png(Some(&upper.to_string())),
            all,
            "the upper one and those below"
        );
        assert_eq!(
            s.document.read().home().current_level(),
            shown,
            "the view stays"
        );
        assert!(
            s.render_3d(Parameters(Render3dParams {
                level: Some("lv999".into()),
                ..Render3dParams::default()
            }))
            .is_err()
        );
    }

    #[test]
    fn render_3d_brings_the_walls_down() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}],"rooms":[{"name":"Sala","at":[200,150]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let png = |walls: &str| {
            let result = s
                .render_3d(Parameters(Render3dParams {
                    walls: Some(walls.into()),
                    w: Some(96),
                    h: Some(72),
                    ..Render3dParams::default()
                }))
                .unwrap();
            let ContentBlock::Image(image) = &result.content[0] else {
                panic!("expected image")
            };
            image.data.clone()
        };
        let up = png("up");
        assert_ne!(png("cutaway"), up, "the near walls came down");
        assert_ne!(png("down"), up);
        assert!(
            s.render_3d(Parameters(Render3dParams {
                walls: Some("sideways".into()),
                ..Render3dParams::default()
            }))
            .is_err()
        );
    }

    #[test]
    fn exports_pdf_and_3d_models() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        let dir = std::env::temp_dir().join(format!("newera-mcp-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for file in ["casa.pdf", "casa.glb", "casa.obj"] {
            let path = dir.join(file).display().to_string();
            let reply = s
                .export_plan(Parameters(ExportParams {
                    path: path.clone(),
                    w: None,
                    h: None,
                    scale: Some(50.0),
                }))
                .unwrap();
            assert!(reply.starts_with("ok"), "{reply}");
            assert!(std::fs::metadata(&path).unwrap().len() > 100, "{file}");
        }
        assert!(
            std::fs::read(dir.join("casa.pdf"))
                .unwrap()
                .starts_with(b"%PDF")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn render_photo_returns_a_png_at_any_hour() {
        let s = server();
        let params: CreateParams = serde_json::from_str(
            r#"{"walls":[{"pts":[[0,0],[300,0],[300,200],[0,200]],"closed":true}],"rooms":[{"name":"Sala","at":[150,100]}]}"#,
        )
        .unwrap();
        s.create(Parameters(params)).unwrap();
        for hour in [9.0, 22.0] {
            let result = s
                .render_photo(Parameters(PhotoParams {
                    w: Some(48),
                    h: Some(36),
                    hour: Some(hour),
                    ..PhotoParams::default()
                }))
                .unwrap();
            assert!(matches!(&result.content[0], ContentBlock::Image(_)));
        }
        assert!(
            s.render_photo(Parameters(PhotoParams {
                quality: Some("ultra".into()),
                ..PhotoParams::default()
            }))
            .is_err()
        );
    }
}
