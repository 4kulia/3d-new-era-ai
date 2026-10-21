use std::path::{Path, PathBuf};
use std::time::Duration;

use web_time::Instant;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText, Stroke};
use egui_phosphor::regular as icon;
use newera_core::{
    Command, CoreResult, Document, Element, ElementId, Home, LengthUnit, Point2, SharedDocument,
    ops,
};
use newera_draw::{
    Palette, RenderOptions, SceneOptions, SvgOptions, plan_scene, render_png, to_svg,
};

use crate::dialogs::{Dialog, DialogOutcome};
use crate::panels;
use crate::view::plan::{PlanEvent, PlanInput, PlanView, Selection, Tool};
use crate::view::scene::SceneView;

/// How often to look for changes made by other clients (MCP, HTTP) while idle.
const EXTERNAL_CHANGES_POLL: Duration = Duration::from_millis(200);
const PASTE_OFFSET: f64 = 30.0;

/// What to do once unsaved changes have been dealt with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Pending {
    New,
    Open(Option<PathBuf>),
    Quit,
}

/// Preferences kept between runs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct Settings {
    unit: LengthUnit,
    recent: Vec<PathBuf>,
    /// How furniture looks on the plan.
    #[serde(default)]
    furniture_look: FurnitureLook,
    /// Day, night, or whatever the system is set to.
    #[serde(default)]
    theme: crate::theme::Mode,
    /// Interface language; missing means a settings file from before the
    /// window spoke more than two languages.
    #[serde(default)]
    lang: Option<crate::i18n::Lang>,
    /// Interface in English instead of Portuguese, as it was written before
    /// `lang`: read once, on the first run after the update.
    #[serde(default)]
    english: bool,
}

/// Plan drawing of furniture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum FurnitureLook {
    /// Symbols for catalog pieces, top views for imported models.
    #[default]
    Auto,
    /// Architectural symbols only.
    Symbols,
    /// Top views of every piece.
    TopViews,
}

const SETTINGS_KEY: &str = "newera-settings";
/// Where the support button goes. The membership is charged in US dollars,
/// wherever the app is being used from.
const SUPPORT_URL: &str = "https://buymeacoffee.com/leandrodaf/membership";

/// On a screen too narrow for three columns, one of these is on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Pane {
    Catalog,
    #[default]
    Plan,
    Scene,
}

/// Narrower than this (in points) the window shows one pane at a time: a
/// phone held upright, or a window squeezed to the side of a screen.
pub(crate) const NARROW: f32 = 720.0;

pub(crate) struct NewEraApp {
    pub(crate) document: SharedDocument,
    pub(crate) mcp_url: Option<String>,
    pub(crate) tool: Tool,
    pub(crate) selection: Selection,
    pub(crate) plan: PlanView,
    pub(crate) scene: SceneView,
    palette: Palette,
    /// Which theme the plan is drawn for, to notice when the system flips it.
    dark: bool,
    settings: Settings,
    pub(crate) dialog: Option<Dialog>,
    status: Option<(String, Instant)>,
    clipboard: Vec<Element>,
    pending: Option<Pending>,
    allow_close: bool,
    title: String,
    plan_rect: egui::Rect,
    pub(crate) catalog_query: String,
    /// AI clients already announced in the status bar, so a connection is
    /// said once and not on every frame.
    pub(crate) announced_agents: usize,
    /// Tool calls already named in the status bar.
    pub(crate) announced_calls: u64,
    /// A registration started from the AI panel, running in its own thread.
    pub(crate) ai_job: Option<crate::ai::RegisterSlot>,
    /// Width the window had last frame, to notice a screen that changed
    /// shape — a phone turned, a tab that finally got its real size.
    #[cfg(target_arch = "wasm32")]
    last_width: f32,
    /// What a narrow screen is showing, since it cannot show everything.
    pub(crate) pane: Pane,
    /// In a browser: whether this tab is reachable by an AI, and where.
    #[cfg(target_arch = "wasm32")]
    pub(crate) ai_link: crate::ai_web::Shared,
    /// Tab being renamed in place: `(variant index, draft name)`.
    pub(crate) renaming_variant: Option<(usize, String)>,
    /// Visitor camera last taken from the document, to follow changes made
    /// elsewhere (MCP) without fighting the user's own navigation.
    applied_observer: Option<(bool, newera_core::Camera)>,
    /// The "Criar foto" window, while open.
    pub(crate) photo: Option<crate::photo::PhotoWindow>,
    pub(crate) video: Option<crate::video::VideoWindow>,
    /// The "Ergonomia" window, while open.
    pub(crate) ergonomics: Option<crate::ergonomics::ErgonomicsWindow>,
    /// The "Armários na parede" window, while open.
    pub(crate) cabinets: Option<crate::cabinets::CabinetsWindow>,
    /// Top-view provider for the plan: `(look, asset dir, provider)`.
    top_views: Option<(
        FurnitureLook,
        Option<PathBuf>,
        newera_render::TopViews,
        newera_draw::PieceImages,
    )>,
    /// Background top views finished when the plan was last rebuilt.
    top_view_generation: u64,
    /// A plugin running in the background: `(title, outcome slot)`.
    #[cfg(not(target_arch = "wasm32"))]
    plugin_job: Option<(String, PluginSlot)>,
    /// Long work in the background, with its progress window.
    pub(crate) job: Option<crate::jobs::Job>,
    /// What to do once the save that is running finishes.
    after_save: Option<Pending>,
    /// A browser file picker to open on the next frame.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pick_request: Option<crate::files::PickKind>,
    /// Frames drawn before the browser canvas got its real size.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    waited_frames: u32,
    /// Where the background image picked in the browser is mounted, so the
    /// next one can let it go.
    #[cfg(target_arch = "wasm32")]
    background_dir: Option<std::path::PathBuf>,
    /// Whether the project on screen came back from the browser's storage,
    /// in which case the page should not open the demo over it.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) restored: bool,
    /// The last autosave attempt revision and time; not confirmation of persistence.
    #[cfg(target_arch = "wasm32")]
    mirrored: (u64, Instant, bool),
}

impl std::fmt::Debug for NewEraApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewEraApp")
            .field("tool", &self.tool)
            .finish_non_exhaustive()
    }
}

impl NewEraApp {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        document: SharedDocument,
        mcp_url: Option<String>,
    ) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        crate::theme::apply(&cc.egui_ctx);

        let stored = cc
            .storage
            .and_then(|s| eframe::get_value::<Settings>(s, SETTINGS_KEY));
        // Nothing saved yet: the language the system asks for. A settings
        // file already there keeps the language it was left in — its owner
        // chose it, whatever the system says. Under test the window speaks
        // the language the code is written in, whatever machine it runs on:
        // the tests read the labels as they appear in the source.
        let first_run = stored.is_none() && !cfg!(test);
        let mut settings = stored.unwrap_or_default();
        let lang = settings.lang.unwrap_or_else(|| {
            if settings.english {
                crate::i18n::Lang::En
            } else if first_run {
                crate::i18n::Lang::from_system().unwrap_or_default()
            } else {
                crate::i18n::Lang::Pt
            }
        });
        settings.lang = Some(lang);
        crate::i18n::set(lang);
        crate::theme::set_mode(&cc.egui_ctx, settings.theme);
        let dark = cc.egui_ctx.theme() == egui::Theme::Dark;
        #[cfg(target_arch = "wasm32")]
        let restored = crate::recovery_web::load();
        #[cfg(not(target_arch = "wasm32"))]
        let restored: Option<String> = None;
        #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
        let mut app = Self {
            document,
            mcp_url,
            tool: Tool::default(),
            selection: Selection::new(),
            plan: PlanView::new(),
            scene: SceneView::new(),
            palette: crate::theme::plan_palette(dark),
            dark,
            settings,
            dialog: None,
            status: None,
            clipboard: Vec::new(),
            pending: None,
            allow_close: false,
            title: String::new(),
            plan_rect: egui::Rect::NOTHING,
            applied_observer: None,
            photo: None,
            video: None,
            ergonomics: None,
            cabinets: None,
            top_views: None,
            top_view_generation: 0,
            #[cfg(not(target_arch = "wasm32"))]
            plugin_job: None,
            job: None,
            after_save: None,
            pick_request: None,
            waited_frames: 0,
            #[cfg(target_arch = "wasm32")]
            background_dir: None,
            restored: false,
            #[cfg(target_arch = "wasm32")]
            mirrored: (0, Instant::now(), false),
            catalog_query: String::new(),
            announced_agents: 0,
            announced_calls: 0,
            #[cfg(target_arch = "wasm32")]
            last_width: 0.0,
            pane: Pane::default(),
            ai_job: None,
            #[cfg(target_arch = "wasm32")]
            ai_link: crate::ai_web::Shared::default(),
            renaming_variant: None,
        };
        // The work of the last visit comes back before anything else opens
        // over it. A browser tab closes without asking, and nobody expects a
        // drawing to survive that unless it does.
        #[cfg(not(target_arch = "wasm32"))]
        let _ = restored;
        #[cfg(target_arch = "wasm32")]
        match restored {
            Ok(Some(snapshot)) => {
                use base64::Engine as _;
                match base64::engine::general_purpose::STANDARD.decode(&snapshot.data) {
                    Ok(bytes) if app.open_bytes(&snapshot.name, &bytes) => {
                        app.restored = true;
                        crate::recovery_web::restored(&snapshot, app.document.read().revision());
                        app.set_status(crate::i18n::tr("Projeto recuperado desta sessão."));
                    }
                    _ => crate::recovery_web::failed("saved project could not be restored", None),
                }
            }
            Err(reason) => crate::recovery_web::failed(&reason, None),
            Ok(None) => {}
        }
        // In a browser the MCP comes up on its own: this editor is for driving
        // with an AI, and a switch somebody has to find first is a step in the
        // way. The room is the one this tab held before, so the address does
        // not change from visit to visit.
        #[cfg(target_arch = "wasm32")]
        crate::ai_web::start(&app.document.clone(), &cc.egui_ctx.clone(), &app.ai_link);

        // Nobody arrives knowing that an editor can be driven by their AI, and
        // the connection is made in another program's settings: the first time
        // the window opens, it says so itself. Not in a browser — there the
        // panel would open to say what this page cannot do; the chip and the
        // menu are there for whoever goes looking.
        #[cfg(not(target_arch = "wasm32"))]
        if first_run {
            app.dialog = Some(Dialog::ConnectAi { client: 0 });
        }
        app
    }

    pub(crate) fn unit(&self) -> LengthUnit {
        self.settings.unit
    }

    pub(crate) fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    /// Runs an edit on the document, reporting errors in the status bar.
    pub(crate) fn run(&mut self, action: impl FnOnce(&mut Document) -> CoreResult<()>) {
        let result = action(&mut self.document.write());
        if let Err(err) = result {
            self.set_status(format!("⚠ {err}"));
        }
    }

    /// Writes the project into the browser's own storage, so closing the tab
    /// is not the same as throwing the drawing away. eframe saves on a timer
    /// and on a page event that never fires, which is no use to someone who
    /// closes a tab a second after the last wall: this runs from the frame,
    /// a breath after the document changes.
    #[cfg(target_arch = "wasm32")]
    fn mirror(&mut self) {
        let doc = self.document.read();
        let revision = doc.revision();
        if revision == self.mirrored.0
            && (!self.mirrored.2 || self.mirrored.1.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        let result = crate::recovery_web::save(&doc);
        drop(doc);
        // Track attempts only to avoid retrying a full snapshot every frame.
        // Confirmed persistence is tracked separately by recovery_web.
        self.mirrored = (revision, Instant::now(), result.is_err());
        if let Err(reason) = result {
            self.set_status(crate::i18n::fill(
                "⚠ Não foi possível salvar: {}",
                &[&reason],
            ));
        }
    }

    /// Saves the cloud project this tab opened back to the service, and
    /// opens the service's copy in its place when somebody saved it first.
    #[cfg(target_arch = "wasm32")]
    fn autosave(&mut self, ctx: &egui::Context) {
        match crate::ai_web::autosave(&self.document, ctx, &self.ai_link) {
            Some(crate::ai_web::Said::Reload {
                name,
                bytes,
                revision,
            }) => {
                if self.open_bytes(&name, &bytes) {
                    let now = self.document.read().revision();
                    crate::ai_web::bind(&self.ai_link, &name, revision, now);
                    self.set_status(crate::i18n::tr(
                        "O projeto mudou na nuvem: esta aba baixou uma cópia do seu e abriu a versão da nuvem.",
                    ));
                }
            }
            Some(crate::ai_web::Said::Note(text)) => self.set_status(text),
            None => {}
        }
    }

    /// The plan is drawn on the paper of the theme in use; when the system
    /// flips from day to night under the window, the sheet flips with it.
    fn follow_theme(&mut self, ctx: &egui::Context) {
        let dark = ctx.theme() == egui::Theme::Dark;
        if dark != self.dark {
            self.dark = dark;
            self.palette = crate::theme::plan_palette(dark);
            self.plan.invalidate_scene();
        }
    }

    pub(crate) fn set_tool(&mut self, tool: Tool) {
        if self.tool != tool {
            self.plan.cancel();
            self.tool = tool;
        }
    }

    // --- Files -------------------------------------------------------------

    fn is_modified(&self) -> bool {
        self.document.read().is_modified()
    }

    fn request(&mut self, action: Pending) {
        if self.is_modified() {
            self.dialog = Some(Dialog::ConfirmDiscard(action));
        } else {
            self.perform(action);
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(clippy::needless_pass_by_value))]
    pub(crate) fn perform(&mut self, action: Pending) {
        // Nothing starts on top of work already running: the document it would
        // replace is the one being read or written.
        if crate::jobs::busy(self) {
            return;
        }
        match action {
            Pending::New => {
                let mut doc = self.document.write();
                doc.load(Home::default());
                doc.set_path(None);
                doc.set_asset_dir(None);
                drop(doc);
                self.after_load();
            }
            #[cfg(target_arch = "wasm32")]
            Pending::Open(_) => self.pick_request = Some(crate::files::PickKind::Project),
            #[cfg(not(target_arch = "wasm32"))]
            Pending::Open(path) => {
                let path = path.or_else(|| {
                    rfd::FileDialog::new()
                        .add_filter(
                            crate::i18n::tr("Projetos (3D New Era AI, Sweet Home 3D)"),
                            &[newera_core::PROJECT_EXTENSION, "sh3d"],
                        )
                        .add_filter("3D New Era AI", &[newera_core::PROJECT_EXTENSION])
                        .add_filter(crate::i18n::tr("Sweet Home 3D"), &["sh3d"])
                        .pick_file()
                });
                if let Some(path) = path {
                    self.open_path(&path);
                }
            }
            Pending::Quit => {
                self.allow_close = true;
                self.pending = Some(Pending::Quit);
            }
        }
    }

    /// Opens a project or imports a home, reading it in the background: a home
    /// with a hundred textures takes seconds, and the window has to answer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn open_path(&mut self, path: &Path) {
        if crate::jobs::busy(self) {
            return;
        }
        let path = path.to_path_buf();
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        crate::jobs::start(self, crate::i18n::tr("Abrindo projeto"), name, move || {
            let read = newera_sh3d::read_file(&path);
            Box::new(move |app: &mut Self| match read {
                Ok(loaded) => {
                    let opened = loaded.into_document(&mut app.document.write());
                    app.remember(&path);
                    app.after_load();
                    let mut status = if opened.imported {
                        crate::i18n::fill(
                            "Importado de {} — salve como projeto para manter tudo num arquivo",
                            &[&path.display()],
                        )
                    } else {
                        crate::i18n::fill("Aberto: {}", &[&path.display()])
                    };
                    if !opened.warnings.is_empty() {
                        status.push_str(&crate::i18n::fill(
                            " · {} aviso(s)",
                            &[&opened.warnings.len()],
                        ));
                        for warning in &opened.warnings {
                            tracing::warn!("import: {warning}");
                        }
                    }
                    app.set_status(status);
                }
                Err(err) => app.set_status(crate::i18n::fill(
                    "⚠ Não foi possível abrir {}: {}",
                    &[&path.display(), &err],
                )),
            })
        });
    }

    /// The plan's top-view provider for the current look and project.
    fn piece_images(&mut self) -> Option<newera_draw::PieceImages> {
        let look = self.settings.furniture_look;
        // No worker threads or disk cache in the browser.
        if look == FurnitureLook::Symbols || cfg!(target_arch = "wasm32") {
            return None;
        }
        let assets = self.document.read().asset_dir();
        if self
            .top_views
            .as_ref()
            .is_none_or(|(l, a, _, _)| *l != look || *a != assets)
        {
            let views = newera_render::TopViews::in_background(
                newera_core::cache_dir().join("topviews"),
                assets.clone(),
                look == FurnitureLook::TopViews,
            );
            let worker = views.clone();
            let provider =
                newera_draw::PieceImages(std::sync::Arc::new(move |piece| worker.image_for(piece)));
            self.top_views = Some((look, assets, views, provider));
            self.plan.invalidate_scene();
        }
        let (_, _, views, provider) = self.top_views.as_ref()?;
        // Redraw as background images arrive.
        let generation = views.generation();
        if generation != self.top_view_generation {
            self.top_view_generation = generation;
            self.plan.invalidate_scene();
        }
        Some(provider.clone())
    }

    /// Derived plan annotations: engineering dimensions and room references.
    fn annotations_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.document.read().home().annotations;
        let mut next = current;
        ui.menu_button(
            format!("{} {}", icon::ARMCHAIR, crate::i18n::tr("Móveis na planta")),
            |ui| {
                for (look, name) in [
                    (
                        FurnitureLook::Auto,
                        crate::i18n::tr("Automático (símbolos e vista de cima dos modelos)"),
                    ),
                    (
                        FurnitureLook::Symbols,
                        crate::i18n::tr("Símbolos arquitetônicos"),
                    ),
                    (
                        FurnitureLook::TopViews,
                        crate::i18n::tr("Vista de cima de todos"),
                    ),
                ] {
                    ui.radio_value(&mut self.settings.furniture_look, look, name);
                }
            },
        );
        ui.checkbox(
            &mut next.auto_dimensions,
            format!(
                "{} {}",
                icon::RULER,
                crate::i18n::tr("Cotas automáticas (engenharia)")
            ),
        );
        ui.checkbox(
            &mut next.references,
            format!(
                "{} {}",
                icon::LIST_BULLETS,
                crate::i18n::tr("Referências dos cômodos")
            ),
        );
        ui.add_enabled(
            next.references,
            egui::Checkbox::new(
                &mut next.reference_details,
                crate::i18n::tr("Detalhes: marca, modelo e link"),
            ),
        );
        ui.checkbox(
            &mut next.legend,
            format!(
                "{} {}",
                icon::LIST_DASHES,
                crate::i18n::tr(crate::i18n::tr(
                    "Legenda de símbolos (elétrica e hidráulica)"
                ))
            ),
        );
        if next != current {
            self.run(|doc| doc.execute(Command::SetAnnotations { annotations: next }));
        }
    }

    /// Aerial/visitor switch and stored points of view.
    fn viewpoints_menu(&mut self, ui: &mut egui::Ui) {
        let cameras = self.document.read().home().cameras.clone();
        let aerial = self.scene.visitor.is_none();
        if ui
            .radio(
                aerial,
                format!("{} {}", icon::GLOBE, crate::i18n::tr("Visão aérea")),
            )
            .clicked()
        {
            self.scene.visitor = None;
            ui.close();
        }
        if ui
            .radio(
                !aerial,
                format!(
                    "{} {}",
                    icon::PERSON_SIMPLE_WALK,
                    crate::i18n::tr("Visitante")
                ),
            )
            .clicked()
        {
            self.scene.visitor = Some(crate::view::scene::Visitor {
                camera: cameras.observer.clone(),
            });
            ui.close();
        }
        ui.menu_button(
            format!(
                "{} {}",
                icon::CAMERA,
                crate::i18n::fill("Pontos de vista ({})", &[&cameras.stored.len()])
            ),
            |ui| {
                if cameras.stored.is_empty() {
                    ui.weak(crate::i18n::tr("Nenhum ponto de vista salvo"));
                }
                for (i, camera) in cameras.stored.iter().enumerate() {
                    let name = camera
                        .name
                        .clone()
                        .unwrap_or_else(|| crate::i18n::fill("Ponto de vista {}", &[&(i + 1)]));
                    if ui.button(name).clicked() {
                        self.scene.visitor = Some(crate::view::scene::Visitor {
                            camera: camera.clone(),
                        });
                        ui.close();
                    }
                }
            },
        );
        if ui
            .add_enabled(
                self.scene.visitor.is_some(),
                egui::Button::new(format!(
                    "{} {}",
                    icon::FLOPPY_DISK,
                    crate::i18n::tr("Salvar ponto de vista")
                )),
            )
            .clicked()
            && let Some(visitor) = &self.scene.visitor
        {
            let mut camera = visitor.camera.clone();
            let mut next = cameras.clone();
            camera.name = Some(crate::i18n::fill(
                "Ponto de vista {}",
                &[&(next.stored.len() + 1)],
            ));
            next.observer = visitor.camera.clone();
            next.stored.push(camera);
            self.run(|doc| doc.execute(Command::SetCameras { cameras: next }));
            ui.close();
        }
    }

    /// Switches the 3D view when the document's active visitor camera changes
    /// (for example when an agent picks a point of view).
    fn follow_document_camera(&mut self) {
        let current = {
            let doc = self.document.read();
            let cameras = &doc.home().cameras;
            (cameras.observer_active, cameras.observer.clone())
        };
        if self.applied_observer.as_ref() == Some(&current) {
            return;
        }
        let first = self.applied_observer.is_none();
        self.applied_observer = Some(current.clone());
        if first && !current.0 {
            return;
        }
        self.scene.visitor = current
            .0
            .then_some(crate::view::scene::Visitor { camera: current.1 });
    }

    fn after_load(&mut self) {
        // Whatever was opened is not the cloud project any more; reopening
        // the cloud copy binds it again.
        #[cfg(target_arch = "wasm32")]
        crate::ai_web::unbind(&self.ai_link);
        self.selection.clear();
        self.plan.cancel();
        self.plan.request_fit();
        self.scene.request_frame();
    }

    /// Saves to the current path, or asks for one. Returns whether saving
    /// started — it finishes in the background, and `then` runs after it.
    pub(crate) fn save_then(&mut self, save_as: bool, then: Option<Pending>) -> bool {
        if crate::jobs::busy(self) {
            return false;
        }
        self.after_save = then;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = save_as;
            let saved = self.save_web();
            if saved && let Some(action) = self.after_save.take() {
                self.perform(action);
            }
            saved
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.save_native(save_as)
    }

    /// Saves with nothing to do afterwards.
    pub(crate) fn save(&mut self, save_as: bool) -> bool {
        self.save_then(save_as, None)
    }

    #[cfg(target_arch = "wasm32")]
    fn save_web(&mut self) -> bool {
        let doc = self.document.read();
        let name = format!("{}.{}", doc.home().name, newera_core::PROJECT_EXTENSION);
        let bytes = newera_core::to_project_bytes(&doc);
        drop(doc);
        match crate::files::save_bytes(
            "3D New Era AI",
            newera_core::PROJECT_EXTENSION,
            &name,
            || Ok(bytes),
        ) {
            Ok(_) => {
                self.document.write().mark_saved(&name);
                self.set_status(crate::i18n::fill("Salvo em {}", &[&name]));
                true
            }
            Err(err) => {
                self.set_status(crate::i18n::fill("⚠ Não foi possível salvar: {}", &[&err]));
                false
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_native(&mut self, save_as: bool) -> bool {
        let current = self.document.read().path().map(Path::to_path_buf);
        let path = match current {
            Some(path) if !save_as => Some(path),
            _ => {
                let name = self.document.read().home().name.clone();
                rfd::FileDialog::new()
                    .add_filter("3D New Era AI", &[newera_core::PROJECT_EXTENSION])
                    .set_file_name(format!("{name}.{}", newera_core::PROJECT_EXTENSION))
                    .save_file()
            }
        };
        let Some(mut path) = path else {
            self.after_save = None;
            return false;
        };
        if path.extension().is_none() {
            path.set_extension(newera_core::PROJECT_EXTENSION);
        }
        // The document is only read while it is written out, so the window
        // keeps drawing from it; the progress window keeps it from being
        // edited half way through the save.
        let document = self.document.clone();
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        crate::jobs::start(self, crate::i18n::tr("Salvando projeto"), name, move || {
            let saved = newera_core::save_project(&document.read(), &path);
            Box::new(move |app: &mut Self| match saved {
                Ok(()) => {
                    app.document.write().mark_saved(&path);
                    app.remember(&path);
                    app.set_status(crate::i18n::fill("Salvo em {}", &[&path.display()]));
                    if let Some(action) = app.after_save.take() {
                        app.perform(action);
                    }
                }
                Err(err) => {
                    app.after_save = None;
                    app.set_status(crate::i18n::fill("⚠ Não foi possível salvar: {}", &[&err]));
                }
            })
        });
        true
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn remember(&mut self, path: &Path) {
        self.settings.recent.retain(|p| p != path);
        self.settings.recent.insert(0, path.to_path_buf());
        self.settings.recent.truncate(8);
    }

    /// Exports the 3D model of the storeys shown in the 3D view.
    fn export_3d(&mut self, ext: &str) {
        if crate::jobs::busy(self) {
            return;
        }
        let name = self.document.read().home().name.clone();
        let (home, assets) = {
            let doc = self.document.read();
            (doc.home().clone(), doc.asset_dir())
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) =
                crate::files::pick_save(&ext.to_uppercase(), ext, &format!("{name}.{ext}"))
            else {
                return;
            };
            let file = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            crate::jobs::start(self, crate::i18n::tr("Exportando"), file, move || {
                newera_core::progress::step("Montando o modelo 3D", 0, 0);
                let done = newera_render::export_home(&home, &path, assets.as_deref());
                Box::new(move |app: &mut Self| match done {
                    Ok(()) => {
                        app.set_status(crate::i18n::fill(
                            "Modelo 3D exportado para {}",
                            &[&path.display()],
                        ));
                    }
                    Err(err) => {
                        app.set_status(crate::i18n::fill("⚠ Falha ao exportar: {}", &[&err]));
                    }
                })
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = assets;
            if ext != "glb" {
                return self.set_status(crate::i18n::tr("No navegador, exporte em .glb."));
            }
            let saved = crate::files::save_bytes("GLB", "glb", &format!("{name}.glb"), || {
                Ok(newera_render::glb_home(&home))
            });
            match saved {
                Ok(_) => self.set_status(crate::i18n::fill(
                    "Modelo 3D exportado para {}",
                    &[&format!("{name}.glb")],
                )),
                Err(err) => self.set_status(crate::i18n::fill("⚠ Falha ao exportar: {}", &[&err])),
            }
        }
    }

    fn export(&mut self, format: &str) {
        if crate::jobs::busy(self) {
            return;
        }
        let name = self.document.read().home().name.clone();
        let svg = format == "svg";
        let pdf_scale = match format {
            "pdf50" => Some(Some(50.0)),
            "pdf100" => Some(Some(100.0)),
            "pdf" => Some(None),
            _ => None,
        };
        let ext = if pdf_scale.is_some() { "pdf" } else { format };
        let doc = self.document.read();
        let scene = plan_scene(
            &doc.home().level_view(doc.home().current_level()),
            &SceneOptions {
                unit: self.settings.unit,
                show_background: true,
                ..SceneOptions::default()
            },
        );
        let assets = doc.asset_dir();
        drop(doc);
        let title = name.clone();
        let make = move || {
            if let Some(scale) = pdf_scale {
                Ok(newera_draw::to_pdf(
                    &scene,
                    &newera_draw::PdfOptions {
                        scale,
                        title: title.clone(),
                        ..newera_draw::PdfOptions::default()
                    },
                ))
            } else if svg {
                Ok(to_svg(&scene, &SvgOptions::default()).into_bytes())
            } else {
                let load = |p: &str| {
                    newera_core::vfs::read(&newera_core::resolve_asset(assets.as_deref(), p))
                        .ok()
                        .and_then(|b| newera_core::images::decode(&b).ok())
                        .map(|i| i.to_rgba8())
                };
                let options = RenderOptions {
                    width: 2400,
                    height: 1800,
                    grid: false,
                    ..RenderOptions::default()
                };
                render_png(&scene, &options, &load).map_err(|e| e.to_string())
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) =
                crate::files::pick_save(&ext.to_uppercase(), ext, &format!("{name}.{ext}"))
            else {
                return;
            };
            let file = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            crate::jobs::start(self, crate::i18n::tr("Exportando"), file, move || {
                newera_core::progress::step("Desenhando a planta", 0, 0);
                let done = make()
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
                Box::new(move |app: &mut Self| match done {
                    Ok(()) => app.set_status(crate::i18n::fill(
                        "Planta exportada para {}",
                        &[&path.display()],
                    )),
                    Err(err) => {
                        app.set_status(crate::i18n::fill("⚠ Falha ao exportar: {}", &[&err]));
                    }
                })
            });
        }
        #[cfg(target_arch = "wasm32")]
        match crate::files::save_bytes(&ext.to_uppercase(), ext, &format!("{name}.{ext}"), make) {
            Ok(Some(path)) => {
                self.set_status(crate::i18n::fill("Planta exportada para {}", &[&path]));
            }
            Ok(None) => {}
            Err(err) => self.set_status(crate::i18n::fill("⚠ Falha ao exportar: {}", &[&err])),
        }
    }

    /// Imports an OBJ/glTF file as a piece at its natural size, placed at
    /// the middle of the plan view.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn import_model(&mut self) {
        self.set_status(crate::i18n::tr("Disponível no aplicativo para desktop."));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn import_model(&mut self) {
        if crate::jobs::busy(self) {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter(crate::i18n::tr("Modelos 3D"), &["obj", "gltf", "glb"])
            .pick_file()
        else {
            return;
        };
        let at = self.plan.view_center();
        let file = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        crate::jobs::start(
            self,
            crate::i18n::tr("Importando modelo"),
            file,
            move || {
                newera_core::progress::step("Lendo o arquivo", 0, 0);
                let loaded = newera_catalog::load_model(&path);
                Box::new(move |app: &mut Self| app.place_model(&path, at, loaded))
            },
        );
    }

    /// Puts an imported model in the plan, at its natural size.
    #[cfg(not(target_arch = "wasm32"))]
    fn place_model(
        &mut self,
        path: &Path,
        at: newera_core::Point2,
        loaded: Result<newera_catalog::ImportedModel, newera_catalog::ImportError>,
    ) {
        match loaded {
            Ok(model) => {
                let name = path.file_stem().map_or_else(
                    || crate::i18n::tr("Modelo").to_owned(),
                    |s| s.to_string_lossy().into_owned(),
                );
                let mut placed = None;
                self.run(|doc| {
                    let piece = newera_core::Furniture {
                        id: doc.new_furniture_id(),
                        catalog: "imported".to_owned(),
                        name,
                        position: at,
                        elevation: 0.0,
                        angle: 0.0,
                        width: model.size[0],
                        depth: model.size[1],
                        height: model.size[2],
                        mirrored: false,
                        color: None,
                        opening: None,
                        model: Some(path.display().to_string()),
                        visible: true,
                        level: None,
                        ..Default::default()
                    };
                    placed = Some(piece.id);
                    doc.execute(Command::insert(piece))
                });
                if let Some(id) = placed {
                    self.selection = std::iter::once(ElementId::from(id)).collect();
                    self.set_tool(Tool::Select);
                    let mut status = crate::i18n::tr(
                        "Modelo importado. Ajuste medidas com Enter ou pelas alças.",
                    )
                    .to_owned();
                    // What of the file is not drawn: "it loaded" is not
                    // "it looks as it should".
                    if !model.report.warnings.is_empty() {
                        status.push_str(" ⚠ ");
                        status.push_str(&model.report.warnings.join("; "));
                    }
                    self.set_status(status);
                }
            }
            Err(err) => self.set_status(format!("⚠ {err}")),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn import_background(&mut self) {
        // The picker answers a few frames later, through `web_files`.
        self.pick_request = Some(crate::files::PickKind::Background);
    }

    /// Puts a picked image behind the plan. In a browser there is no path to
    /// point at, so the bytes are mounted in memory under a name of their
    /// own and the background points there.
    #[cfg(target_arch = "wasm32")]
    fn place_background(&mut self, name: &str, bytes: &[u8]) {
        let size = match newera_core::images::decode(bytes) {
            Ok(image) => [image.width(), image.height()],
            Err(err) => {
                return self.set_status(crate::i18n::fill("⚠ Imagem inválida: {}", &[&err]));
            }
        };
        // A directory per import, so a second image does not have to share a
        // name with the first — and the one before it is let go.
        let revision = self.document.read().revision();
        let dir = std::path::PathBuf::from(format!("/memory/background/{revision}"));
        if let Some(previous) = self.background_dir.replace(dir.clone()) {
            newera_core::vfs::unmount(&previous);
        }
        newera_core::vfs::mount(&dir, [(name.to_owned(), bytes.to_vec())]);
        let path = dir.join(name).display().to_string();
        // Start at a plausible scale: fit the image width to ~15 m.
        let cm_per_px = 1500.0 / f64::from(size[0]);
        self.run(|doc| {
            doc.execute(Command::SetBackground {
                background: Some(newera_core::BackgroundImage {
                    path,
                    size_px: size,
                    cm_per_px,
                    offset: Point2::new(0.0, 0.0),
                    opacity: 0.5,
                    visible: true,
                    ..Default::default()
                }),
            })
        });
        self.plan.request_fit();
        self.set_tool(Tool::Calibrate);
        self.set_status(crate::i18n::tr(
            "Imagem importada. Clique em dois pontos de medida conhecida para calibrar; arraste para posicionar.",
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn import_background(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter(
                crate::i18n::tr("Imagens"),
                &["png", "jpg", "jpeg", "webp", "bmp"],
            )
            .pick_file()
        else {
            return;
        };
        let size = match image::image_dimensions(&path) {
            Ok((w, h)) => [w, h],
            Err(err) => {
                return self.set_status(crate::i18n::fill("⚠ Imagem inválida: {}", &[&err]));
            }
        };
        // Start at a plausible scale: fit the image width to ~15 m.
        let cm_per_px = 1500.0 / f64::from(size[0]);
        self.run(|doc| {
            doc.execute(Command::SetBackground {
                background: Some(newera_core::BackgroundImage {
                    path: path.display().to_string(),
                    size_px: size,
                    cm_per_px,
                    offset: Point2::new(0.0, 0.0),
                    opacity: 0.5,
                    visible: true,
                    ..Default::default()
                }),
            })
        });
        self.plan.request_fit();
        self.set_tool(Tool::Calibrate);
        self.set_status(
            crate::i18n::tr("Imagem importada. Clique em dois pontos de medida conhecida para calibrar; arraste para posicionar."),
        );
    }

    // --- Selection & clipboard -------------------------------------------------

    fn selected_elements(&self) -> Vec<Element> {
        let doc = self.document.read();
        self.selection
            .iter()
            .filter_map(|id| doc.home().element(*id))
            .collect()
    }

    fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let ids: Vec<ElementId> = std::mem::take(&mut self.selection).into_iter().collect();
        self.run(|doc| {
            doc.execute(Command::Batch {
                commands: ids.into_iter().map(Command::remove).collect(),
            })
        });
    }

    fn copy(&mut self) {
        self.clipboard = self.selected_elements();
    }

    fn paste(&mut self, elements: Vec<Element>) {
        if elements.is_empty() {
            return;
        }
        let mut new_ids = Vec::new();
        self.run(|doc| {
            let commands = elements
                .into_iter()
                .map(|element| {
                    let moved = shift_with_new_id(doc, element, PASTE_OFFSET);
                    new_ids.push(moved.id());
                    Command::insert(moved)
                })
                .collect();
            doc.execute(Command::Batch { commands })
        });
        self.selection = new_ids.into_iter().collect();
    }

    fn select_all(&mut self) {
        let doc = self.document.read();
        let view = doc.home().level_view(doc.home().current_level());
        self.selection = view
            .elements()
            .filter(|e| !matches!(e, Element::Level(_)))
            .map(|e| e.id())
            .collect();
    }

    fn nudge(&mut self, dx: f64, dy: f64) {
        let ids: Vec<ElementId> = self.selection.iter().copied().collect();
        if !ids.is_empty() {
            self.run(|doc| ops::translate(doc, &ids, dx, dy, true));
        }
    }

    pub(crate) fn open_modify(&mut self, ids: &[ElementId]) {
        let elements: Vec<Element> = {
            let doc = self.document.read();
            ids.iter()
                .filter_map(|id| doc.home().element(*id))
                .collect()
        };
        self.dialog = Dialog::modify(&elements);
    }

    pub(crate) fn set_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
    }

    /// Selection and in-progress drawing belong to the variant that was active.
    pub(crate) fn after_variant_change(&mut self) {
        self.selection.clear();
        self.plan.cancel();
    }

    pub(crate) fn open_compare(&mut self) {
        let doc = self.document.read();
        let rows = doc
            .variants()
            .enumerate()
            .map(|(i, v)| crate::tabs::stats(&v.name, i == doc.active_variant(), v.home()))
            .collect();
        drop(doc);
        self.dialog = Some(Dialog::Compare(rows));
    }

    fn cycle_variant(&mut self) {
        self.run(|doc| {
            let next = (doc.active_variant() + 1) % doc.variant_count();
            doc.switch_variant(next)
        });
        self.after_variant_change();
    }

    fn open_home_settings(&mut self) {
        let home = self.document.read().home().clone();
        self.dialog = Some(Dialog::HomeSettings {
            name: home.name,
            compass: home.compass,
        });
    }

    // --- Input -----------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        let pressed = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };

        // On a narrow screen the three panes are one keystroke apart, which is
        // also how a check drives them without hunting for a button.
        for (key, pane) in [
            (Key::Num1, Pane::Catalog),
            (Key::Num2, Pane::Plan),
            (Key::Num3, Pane::Scene),
        ] {
            if pressed(cmd, key) {
                self.pane = pane;
            }
        }
        // Ctrl+Shift+M: in a browser, make this tab reachable by an AI, or
        // close it again. On the desktop the server is already up, so the
        // same keys open the panel that says how to reach it.
        if pressed(cmd_shift, Key::M) {
            #[cfg(target_arch = "wasm32")]
            {
                let on = matches!(self.ai_link.borrow().link, crate::ai_web::Link::On { .. });
                if on {
                    crate::ai_web::disconnect(&self.ai_link);
                    self.set_status(crate::i18n::tr("MCP desligado"));
                } else {
                    crate::ai_web::connect(
                        self.document.clone(),
                        ctx.clone(),
                        self.ai_link.clone(),
                    );
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.dialog = Some(Dialog::ConnectAi { client: 0 });
            }
        }
        if pressed(cmd, Key::T) {
            self.run(|doc| {
                doc.add_variant(None, true);
                Ok(())
            });
            self.after_variant_change();
        }
        if pressed(cmd, Key::Tab) {
            self.cycle_variant();
        }
        if pressed(cmd, Key::N) {
            self.request(Pending::New);
        }
        if pressed(cmd, Key::O) {
            self.request(Pending::Open(None));
        }
        if pressed(cmd_shift, Key::S) {
            self.save(true);
        } else if pressed(cmd, Key::S) {
            self.save(false);
        }
        if ctx.egui_wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        if pressed(cmd_shift, Key::Z) || pressed(cmd, Key::Y) {
            self.run(Document::redo);
        } else if pressed(cmd, Key::Z) {
            self.run(Document::undo);
        }
        if pressed(cmd, Key::A) {
            self.select_all();
        }
        if pressed(cmd, Key::C) {
            self.copy();
        }
        if pressed(cmd, Key::X) {
            self.copy();
            self.delete_selection();
        }
        if pressed(cmd, Key::V) {
            self.paste(self.clipboard.clone());
        }
        if pressed(cmd, Key::D) {
            let selected = self.selected_elements();
            self.paste(selected);
        }
        if pressed(cmd, Key::Equals) || pressed(cmd, Key::Plus) {
            self.plan.zoom_by(self.plan_rect, 1.25);
        }
        if pressed(cmd, Key::Minus) {
            self.plan.zoom_by(self.plan_rect, 0.8);
        }
        if pressed(cmd, Key::Num0) {
            self.plan.request_fit();
        }

        let drawing_walls = self.plan.accepts_length_input();
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.plan.is_drawing() {
                self.plan.cancel();
            } else if self.tool != Tool::Select {
                self.set_tool(Tool::Select);
            } else {
                self.selection.clear();
            }
        }
        if !drawing_walls {
            for (key, tool) in [
                (Key::V, Tool::Select),
                (Key::H, Tool::Pan),
                (Key::W, Tool::Walls),
                (Key::R, Tool::Rooms),
                (Key::D, Tool::Dimensions),
                (Key::T, Tool::Labels),
                (Key::L, Tool::Lines),
            ] {
                if ctx.input(|i| i.key_pressed(key) && i.modifiers.is_none()) {
                    self.set_tool(tool);
                }
            }
            if ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                self.delete_selection();
            }
        }
        // Consume the Enter so the dialog it opens doesn't read it as "OK".
        if self.tool == Tool::Select
            && !self.selection.is_empty()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        {
            self.open_modify(&self.selection.iter().copied().collect::<Vec<_>>());
        }
        let step = if ctx.input(|i| i.modifiers.shift) {
            10.0
        } else {
            1.0
        };
        for (key, dx, dy) in [
            (Key::ArrowLeft, -step, 0.0),
            (Key::ArrowRight, step, 0.0),
            (Key::ArrowUp, 0.0, -step),
            (Key::ArrowDown, 0.0, step),
        ] {
            if ctx.input(|i| i.key_pressed(key)) && !self.selection.is_empty() {
                self.nudge(dx, dy);
            }
        }
    }

    // --- Layout ----------------------------------------------------------------

    /// Buy Me a Coffee's button, at the right end of the tool bar, in the same
    /// yellow it has on the site. It is the one thing in this bar that is not
    /// about the drawing, so it sits apart from the tools rather than among
    /// them: the app is free, and this is how it stays paid for.
    ///
    /// Where there is no room for words it becomes one more button the size of
    /// the others, with the cup and nothing else — the name still reaches a
    /// screen reader and a hover.
    fn support_button(ui: &mut egui::Ui, narrow: bool) {
        // The brand's yellow and its near-black, both fixed: the button reads
        // the same in the light theme and the dark one, as it does on the site.
        let yellow = egui::Color32::from_rgb(0xff, 0xdd, 0x00);
        let ink = egui::Color32::from_rgb(0x0d, 0x0c, 0x0c);
        let words = crate::i18n::tr("Me paga um café");
        let label = if narrow {
            // The size the tools beside it are drawn at.
            RichText::new(icon::COFFEE).size(17.0)
        } else {
            RichText::new(format!("{} {}", icon::COFFEE, words))
        };
        let button = egui::Button::new(label.color(ink))
            .fill(yellow)
            .corner_radius(egui::CornerRadius::same(7));
        let response = ui.add(button);
        // Narrowed to the cup, the button's own text is one glyph of an icon
        // font, which is what a screen reader would otherwise be handed. The
        // hover text is a picture; this is the name.
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, words));
        if response.on_hover_text(words).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(SUPPORT_URL));
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(crate::i18n::tr("Arquivo"), |ui| {
                if menu_item(ui, icon::FILE_PLUS, crate::i18n::tr("Novo"), "Ctrl+N", true) {
                    self.request(Pending::New);
                }
                if menu_item(
                    ui,
                    icon::FOLDER_OPEN,
                    crate::i18n::tr("Abrir…"),
                    "Ctrl+O",
                    true,
                ) {
                    self.request(Pending::Open(None));
                }
                let recent = self.settings.recent.clone();
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button(
                        format!(
                            "{} {}",
                            icon::CLOCK_COUNTER_CLOCKWISE,
                            crate::i18n::tr("Recentes")
                        ),
                        |ui| {
                            for path in recent {
                                let label = path.file_name().map_or_else(
                                    || path.display().to_string(),
                                    |n| n.to_string_lossy().into_owned(),
                                );
                                if ui
                                    .button(label)
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    self.request(Pending::Open(Some(path)));
                                }
                            }
                        },
                    );
                });
                ui.separator();
                if menu_item(
                    ui,
                    icon::FLOPPY_DISK,
                    crate::i18n::tr("Salvar"),
                    "Ctrl+S",
                    true,
                ) {
                    self.save(false);
                }
                if menu_item(
                    ui,
                    icon::FLOPPY_DISK_BACK,
                    crate::i18n::tr("Salvar como…"),
                    "Ctrl+Shift+S",
                    true,
                ) {
                    self.save(true);
                }
                ui.separator();
                ui.menu_button(
                    format!("{} {}", icon::EXPORT, crate::i18n::tr("Exportar planta")),
                    |ui| {
                        if ui
                            .button(crate::i18n::tr("PDF (A3, ajustado à folha)…"))
                            .clicked()
                        {
                            self.export("pdf");
                        }
                        if ui.button(crate::i18n::tr("PDF 1:50…")).clicked() {
                            self.export("pdf50");
                        }
                        if ui.button(crate::i18n::tr("PDF 1:100…")).clicked() {
                            self.export("pdf100");
                        }
                        if ui.button(crate::i18n::tr("SVG em escala real…")).clicked() {
                            self.export("svg");
                        }
                        if ui.button(crate::i18n::tr("PNG…")).clicked() {
                            self.export("png");
                        }
                    },
                );
                ui.menu_button(
                    format!("{} {}", icon::CUBE, crate::i18n::tr("Exportar 3D")),
                    |ui| {
                        if ui.button(crate::i18n::tr("glTF binário (.glb)…")).clicked() {
                            self.export_3d("glb");
                        }
                        if ui.button(crate::i18n::tr("OBJ + MTL…")).clicked() {
                            self.export_3d("obj");
                        }
                    },
                );
                ui.separator();
                if menu_item(ui, icon::SIGN_OUT, crate::i18n::tr("Sair"), "", true) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(crate::i18n::tr("Editar"), |ui| {
                let (can_undo, can_redo) = {
                    let doc = self.document.read();
                    (doc.can_undo(), doc.can_redo())
                };
                let has_selection = !self.selection.is_empty();
                if menu_item(
                    ui,
                    icon::ARROW_COUNTER_CLOCKWISE,
                    crate::i18n::tr("Desfazer"),
                    "Ctrl+Z",
                    can_undo,
                ) {
                    self.run(Document::undo);
                }
                if menu_item(
                    ui,
                    icon::ARROW_CLOCKWISE,
                    crate::i18n::tr("Refazer"),
                    "Ctrl+Shift+Z",
                    can_redo,
                ) {
                    self.run(Document::redo);
                }
                ui.separator();
                if menu_item(
                    ui,
                    icon::SCISSORS,
                    crate::i18n::tr("Recortar"),
                    "Ctrl+X",
                    has_selection,
                ) {
                    self.copy();
                    self.delete_selection();
                }
                if menu_item(
                    ui,
                    icon::COPY,
                    crate::i18n::tr("Copiar"),
                    "Ctrl+C",
                    has_selection,
                ) {
                    self.copy();
                }
                if menu_item(
                    ui,
                    icon::CLIPBOARD,
                    crate::i18n::tr("Colar"),
                    "Ctrl+V",
                    !self.clipboard.is_empty(),
                ) {
                    self.paste(self.clipboard.clone());
                }
                if menu_item(
                    ui,
                    icon::COPY_SIMPLE,
                    crate::i18n::tr("Duplicar"),
                    "Ctrl+D",
                    has_selection,
                ) {
                    let selected = self.selected_elements();
                    self.paste(selected);
                }
                if menu_item(
                    ui,
                    icon::TRASH,
                    crate::i18n::tr("Excluir"),
                    "Del",
                    has_selection,
                ) {
                    self.delete_selection();
                }
                ui.separator();
                if menu_item(
                    ui,
                    icon::SELECTION_ALL,
                    crate::i18n::tr("Selecionar tudo"),
                    "Ctrl+A",
                    true,
                ) {
                    self.select_all();
                }
                if menu_item(
                    ui,
                    icon::PENCIL_SIMPLE,
                    crate::i18n::tr("Modificar…"),
                    "Enter",
                    has_selection,
                ) {
                    self.open_modify(&self.selection.iter().copied().collect::<Vec<_>>());
                }
            });
            ui.menu_button(crate::i18n::tr("Planta"), |ui| {
                for (tool, glyph, label, key) in TOOLS {
                    if ui
                        .add(
                            egui::Button::selectable(
                                self.tool == tool,
                                format!("{glyph} {}", crate::i18n::tr(label)),
                            )
                                .shortcut_text(key),
                        )
                        .clicked()
                    {
                        self.set_tool(tool);
                    }
                }
                ui.separator();
                let single_wall = match self.selection.iter().collect::<Vec<_>>().as_slice() {
                    [ElementId::Wall(id)] => Some(*id),
                    _ => None,
                };
                // Two pieces selected, one of them joinery: embed the other in it.
                let pair = {
                    let doc = self.document.read();
                    let pieces: Vec<newera_core::Furniture> = self
                        .selection
                        .iter()
                        .filter_map(|id| match id {
                            ElementId::Furniture(f) => {
                                doc.home().furniture.iter().find(|p| p.id == *f).cloned()
                            }
                            _ => None,
                        })
                        .collect();
                    match pieces.as_slice() {
                        [a, b] => {
                            let host = |p: &newera_core::Furniture| {
                                p.properties.contains_key(newera_joinery::PARAMS_KEY)
                            };
                            match (host(a), host(b)) {
                                (true, false) => Some((b.clone(), a.id)),
                                (false, true) => Some((a.clone(), b.id)),
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                };
                if menu_item(
                    ui,
                    icon::ARROW_SQUARE_IN,
                    crate::i18n::tr("Embutir peça no móvel selecionado"),
                    "",
                    pair.is_some(),
                ) && let Some((item, host)) = pair
                {
                    let request = newera_joinery::EmbedRequest {
                        item,
                        existing: true,
                        host,
                        at: None,
                        z: None,
                        dry: false,
                    };
                    let result = newera_joinery::embed(&mut self.document.write(), &request);
                    match result {
                        Ok(reply) => self.set_status(format!(
                            "{} {}",
                            crate::i18n::tr("Embutido:"),
                            reply["notes"]
                                .as_array()
                                .and_then(|n| n.first())
                                .and_then(|n| n.as_str())
                                .unwrap_or(reply["kind"].as_str().unwrap_or_default())
                        )),
                        Err(err) => self.set_status(format!("⚠ {err}")),
                    }
                    ui.close();
                }
                let fit_ids: Vec<ElementId> = self
                    .selection
                    .iter()
                    .filter(|id| matches!(id, ElementId::Wall(_) | ElementId::Furniture(_)))
                    .copied()
                    .collect();
                if menu_item(
                    ui,
                    icon::TRIANGLE,
                    crate::i18n::tr("Ajustar ao telhado"),
                    "",
                    !fit_ids.is_empty(),
                ) {
                    self.run(|doc| {
                        newera_core::fit_to_roof(doc, &fit_ids, newera_core::ROOF_FIT_ABOVE)
                            .map(|_| ())
                    });
                    ui.close();
                }
                if menu_item(
                    ui,
                    icon::SQUARES_FOUR,
                    crate::i18n::tr("Armários na parede…"),
                    "",
                    single_wall.is_some(),
                ) && let Some(id) = single_wall
                {
                    self.cabinets = Some(crate::cabinets::CabinetsWindow::new(id));
                    ui.close();
                }
                if menu_item(
                    ui,
                    icon::SCISSORS,
                    crate::i18n::tr("Dividir parede ao meio"),
                    "",
                    single_wall.is_some(),
                ) && let Some(id) = single_wall
                {
                    self.run(|doc| ops::split_wall(doc, id, 0.5).map(|_| ()));
                }
                let walls: Vec<_> = self
                    .selection
                    .iter()
                    .filter_map(|id| match id {
                        ElementId::Wall(w) => Some(*w),
                        _ => None,
                    })
                    .collect();
                if menu_item(
                    ui,
                    icon::ARROWS_IN_LINE_HORIZONTAL,
                    crate::i18n::tr("Unir paredes selecionadas"),
                    "",
                    walls.len() > 1,
                ) {
                    let ids = walls.clone();
                    self.run(|doc| ops::merge_walls(doc, &ids).map(|_| ()));
                }
                if menu_item(
                    ui,
                    icon::RULER,
                    crate::i18n::tr("Cotar paredes selecionadas"),
                    "",
                    !walls.is_empty(),
                ) {
                    self.run(|doc| {
                        let mut commands = Vec::new();
                        for id in &walls {
                            commands.push(Command::insert(ops::wall_dimension(doc, *id, 40.0)?));
                        }
                        doc.execute(Command::Batch { commands })
                    });
                }
                ui.separator();
                ui.menu_button(
                    format!("{} {}", icon::IMAGE, crate::i18n::tr("Imagem de fundo")),
                    |ui| {
                        if ui.button(crate::i18n::tr("Importar…")).clicked() {
                            self.import_background();
                        }
                        let background = self.document.read().home().background.clone();
                        if ui
                            .add_enabled(
                                background.is_some(),
                                egui::Button::new(crate::i18n::tr("Calibrar e posicionar")),
                            )
                            .clicked()
                        {
                            self.set_tool(Tool::Calibrate);
                        }
                        if ui
                            .add_enabled(
                                background.is_some(),
                                egui::Button::new(crate::i18n::tr("Ajustes…")),
                            )
                            .clicked()
                        {
                            self.dialog = background.map(Dialog::Background);
                        }
                    },
                );
                if menu_item(
                    ui,
                    icon::CUBE,
                    crate::i18n::tr("Importar modelo 3D…"),
                    "",
                    true,
                ) {
                    self.import_model();
                }
                if menu_item(
                    ui,
                    icon::COMPASS,
                    crate::i18n::tr("Casa e bússola…"),
                    "",
                    true,
                ) {
                    self.open_home_settings();
                }
                if menu_item(
                    ui,
                    icon::PERSON_ARMS_SPREAD,
                    crate::i18n::tr("Ergonomia…"),
                    "",
                    true,
                ) {
                    self.ergonomics.get_or_insert_with(|| {
                        let home = self.document.read();
                        crate::ergonomics::ErgonomicsWindow {
                            profile: newera_ergonomics::Profile {
                                // The building code lives on the compass.
                                city: home.home().compass.city.clone(),
                                ..newera_ergonomics::Profile::of(home.home())
                            },
                            ..Default::default()
                        }
                    });
                    ui.close();
                }
            });
            ui.menu_button(crate::i18n::tr("Ver"), |ui| {
                if menu_item(
                    ui,
                    icon::CORNERS_OUT,
                    crate::i18n::tr("Enquadrar planta"),
                    "Ctrl+0",
                    true,
                ) {
                    self.plan.request_fit();
                }
                if menu_item(ui, icon::CUBE, crate::i18n::tr("Enquadrar 3D"), "", true) {
                    self.scene.request_frame();
                }
                if menu_item(ui, icon::CAMERA, crate::i18n::tr("Criar foto…"), "", true) {
                    self.photo.get_or_insert_with(Default::default);
                    ui.close();
                }
                if menu_item(
                    ui,
                    icon::FILM_STRIP,
                    crate::i18n::tr("Criar vídeo…"),
                    "",
                    true,
                ) {
                    self.video.get_or_insert_with(Default::default);
                    ui.close();
                }
                ui.separator();
                self.annotations_menu(ui);
                ui.separator();
                self.viewpoints_menu(ui);
                ui.separator();
                ui.label(format!("{} {}", icon::WALL, crate::i18n::tr("Paredes")));
                for walls in crate::view::scene::Walls::ALL {
                    if ui
                        .radio(self.scene.walls == walls, walls.label())
                        .on_hover_text(walls.hint())
                        .clicked()
                    {
                        self.scene.walls = walls;
                        ui.close();
                    }
                }
                ui.separator();
                let mut sun = self.scene.sun_hour.is_some();
                if ui
                    .checkbox(
                        &mut sun,
                        format!(
                            "{} {}",
                            icon::SUN,
                            crate::i18n::tr("Sol pela bússola e hora")
                        ),
                    )
                    .changed()
                {
                    self.scene.sun_hour = sun.then_some(10.0);
                }
                if let Some(hour) = &mut self.scene.sun_hour {
                    ui.add(
                        egui::Slider::new(hour, 0.0..=24.0)
                            .step_by(0.25)
                            .suffix(" h"),
                    );
                }
                ui.separator();
                ui.label(crate::i18n::tr("Tema"));
                for mode in crate::theme::Mode::ALL {
                    if ui
                        .radio(self.settings.theme == mode, mode.label())
                        .clicked()
                    {
                        self.settings.theme = mode;
                        crate::theme::set_mode(ui.ctx(), mode);
                    }
                }
                ui.separator();
                ui.label(crate::i18n::tr("Unidade"));
                for unit in LengthUnit::ALL {
                    ui.radio_value(&mut self.settings.unit, unit, unit.label());
                }
            });
            #[cfg(not(target_arch = "wasm32"))]
            self.plugins_menu(ui);
            ui.menu_button(crate::i18n::tr("Ajuda"), |ui| {
                ui.menu_button(format!("{} Idioma / Language", icon::TRANSLATE), |ui| {
                    let current = crate::i18n::lang();
                    for lang in crate::i18n::Lang::ALL {
                        if ui.radio(lang == current, lang.label()).clicked() {
                            self.settings.lang = Some(lang);
                            crate::i18n::set(lang);
                            self.plan.invalidate_scene();
                        }
                    }
                });
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // On by default; one click turns crash reports and the
                    // notes agents leave off, now and for the next runs.
                    let mut on = newera_telemetry::enabled();
                    let label = format!(
                        "{} {}",
                        icon::HEARTBEAT,
                        crate::i18n::tr("Enviar relatórios de erro")
                    );
                    if ui
                        .checkbox(&mut on, label)
                        .on_hover_text(crate::i18n::tr(
                            "Falhas e notas de uso vão para os desenvolvedores, sem o seu projeto, sem IP e sem nome da máquina.",
                        ))
                        .changed()
                        && let Err(e) = newera_telemetry::set_enabled(on)
                    {
                        tracing::warn!("telemetry setting not saved: {e}");
                    }
                }
                if menu_item(
                    ui,
                    icon::KEYBOARD,
                    crate::i18n::tr("Atalhos e ferramentas"),
                    "",
                    true,
                ) {
                    self.dialog = Some(Dialog::Help);
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let narrow = ui.ctx().content_rect().width() < NARROW;
        ui.horizontal(|ui| {
            // The bar is laid out from its right end, so the support button
            // keeps that end and the tools take whatever is left. On a narrow
            // window they scroll underneath it rather than pushing it off the
            // edge, which is the one thing that must not happen to it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Self::support_button(ui, narrow);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // Narrow enough and the tools do not fit on one line: they
                    // scroll sideways rather than falling off the edge.
                    if narrow {
                        egui::ScrollArea::horizontal()
                            .id_salt("toolbar_scroll")
                            .show(ui, |ui| self.toolbar_row(ui));
                    } else {
                        self.toolbar_row(ui);
                    }
                });
            });
        });
    }

    fn toolbar_row(&mut self, ui: &mut egui::Ui) {
        let t = crate::theme::of(ui.visuals());
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let big = |text: &str| RichText::new(text).size(17.0);
            keys(ui, |ui| {
                if ui
                    .button(big(icon::FILE_PLUS))
                    .on_hover_text(crate::i18n::tr("Novo (Ctrl+N)"))
                    .clicked()
                {
                    self.request(Pending::New);
                }
                if ui
                    .button(big(icon::FOLDER_OPEN))
                    .on_hover_text(crate::i18n::tr("Abrir (Ctrl+O)"))
                    .clicked()
                {
                    self.request(Pending::Open(None));
                }
                if ui
                    .button(big(icon::FLOPPY_DISK))
                    .on_hover_text(crate::i18n::tr("Salvar (Ctrl+S)"))
                    .clicked()
                {
                    self.save(false);
                }
            });
            let (can_undo, can_redo) = {
                let doc = self.document.read();
                (doc.can_undo(), doc.can_redo())
            };
            keys(ui, |ui| {
                if ui
                    .add_enabled(
                        can_undo,
                        egui::Button::new(big(icon::ARROW_COUNTER_CLOCKWISE)),
                    )
                    .on_hover_text(crate::i18n::tr("Desfazer (Ctrl+Z)"))
                    .clicked()
                {
                    self.run(Document::undo);
                }
                if ui
                    .add_enabled(can_redo, egui::Button::new(big(icon::ARROW_CLOCKWISE)))
                    .on_hover_text(crate::i18n::tr("Refazer (Ctrl+Shift+Z)"))
                    .clicked()
                {
                    self.run(Document::redo);
                }
            });
            // The tools are the row that matters: the one in use wears the
            // accent, the others stay out of the way.
            keys(ui, |ui| {
                for (tool, glyph, label, key) in TOOLS {
                    let chosen = self.tool == tool;
                    let response = ui.add(egui::Button::selectable(chosen, big(glyph)));
                    if chosen {
                        ring(ui, &response, t.accent);
                    }
                    if response
                        .on_hover_text(format!("{} ({key})", crate::i18n::tr(label)))
                        .clicked()
                    {
                        self.set_tool(tool);
                    }
                }
            });
            keys(ui, |ui| {
                if ui
                    .button(big(icon::IMAGE))
                    .on_hover_text(crate::i18n::tr("Importar imagem de fundo"))
                    .clicked()
                {
                    self.import_background();
                }
                if ui
                    .button(big(icon::COMPASS))
                    .on_hover_text(crate::i18n::tr("Casa e bússola"))
                    .clicked()
                {
                    self.open_home_settings();
                }
            });
            keys(ui, |ui| {
                if ui
                    .button(big(icon::MAGNIFYING_GLASS_PLUS))
                    .on_hover_text(crate::i18n::tr("Aproximar (Ctrl +)"))
                    .clicked()
                {
                    self.plan.zoom_by(self.plan_rect, 1.25);
                }
                if ui
                    .button(big(icon::MAGNIFYING_GLASS_MINUS))
                    .on_hover_text(crate::i18n::tr("Afastar (Ctrl -)"))
                    .clicked()
                {
                    self.plan.zoom_by(self.plan_rect, 0.8);
                }
                if ui
                    .button(big(icon::CORNERS_OUT))
                    .on_hover_text(crate::i18n::tr("Enquadrar (Ctrl+0)"))
                    .clicked()
                {
                    self.plan.request_fit();
                }
            });
            // How tall the walls stand in 3D. Each button is laid out like its
            // neighbours around an invisible glyph, then drawn over with a
            // little room showing that mode.
            keys(ui, |ui| {
                for walls in crate::view::scene::Walls::ALL {
                    let chosen = self.scene.walls == walls;
                    let button = egui::Button::selectable(
                        chosen,
                        big(icon::WALL).color(egui::Color32::TRANSPARENT),
                    );
                    let response = ui.add(button);
                    // Named after the mode, not the glyph it is laid out around.
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            true,
                            chosen,
                            walls.label(),
                        )
                    });
                    let response = response.on_hover_text(format!(
                        "{}: {}\n{}",
                        crate::i18n::tr("Paredes"),
                        walls.label(),
                        walls.hint()
                    ));
                    let look = ui.style().interact_selectable(&response, chosen);
                    let behind = if chosen {
                        t.inset.blend(ui.visuals().selection.bg_fill)
                    } else {
                        t.inset.blend(look.weak_bg_fill)
                    };
                    walls.paint_icon(ui.painter(), response.rect, look.fg_stroke.color, behind);
                    if chosen {
                        ring(ui, &response, t.accent);
                    }
                    if response.clicked() {
                        self.scene.walls = walls;
                    }
                }
            });
            keys(ui, |ui| crate::ai::button(self, ui));
        });
        ui.add_space(2.0);
    }

    /// A phone hands the editor a canvas measured in a handful of points — a
    /// 390 px screen at three device pixels each arrives as 130 × 281 — and a
    /// window laid out for a desk is unusable at that size: three buttons and
    /// a title that overlaps its own count.
    ///
    /// So the scale is set to what the screen can hold: enough points for the
    /// chrome to lay out honestly, at a size a person can still read. Above
    /// that width nothing is touched — whoever set their own zoom keeps it.
    #[cfg(target_arch = "wasm32")]
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        /// Points of width the narrow layout needs to breathe.
        const WANT: f32 = 440.0;

        let have = ctx.content_rect().width();
        // A window that changed shape was framed for the old one.
        if have > 1.0 && (have - self.last_width).abs() > self.last_width.max(1.0) * 0.1 {
            self.last_width = have;
            self.plan.request_fit();
        }
        if have <= 1.0 || have >= WANT {
            return;
        }
        let zoom = (ctx.zoom_factor() * have / WANT).clamp(0.2, 1.0);
        if (zoom - ctx.zoom_factor()).abs() > 0.02 {
            ctx.set_zoom_factor(zoom);
            // The plan was framed for the old size; frame it again for this one.
            self.plan.request_fit();
        }
    }

    /// The plan, and what drawing in it asks of the rest of the window.
    fn plan_ui(&mut self, ui: &mut egui::Ui) {
        self.plan_rect = ui.available_rect_before_wrap();
        let piece_images = self.piece_images();
        let events = self.plan.ui(
            ui,
            PlanInput {
                document: &self.document,
                selection: &mut self.selection,
                tool: self.tool,
                unit: self.settings.unit,
                palette: &self.palette,
                piece_images,
            },
        );
        for event in events {
            match event {
                PlanEvent::Modify(ids) => self.open_modify(&ids),
                PlanEvent::NewLabel(at) => {
                    self.dialog = Some(Dialog::NewLabel {
                        at,
                        text: String::new(),
                    });
                }
                PlanEvent::Calibrate { a, b } => {
                    self.dialog = Some(Dialog::Calibrate {
                        a,
                        b,
                        distance: 100.0,
                    });
                }
                PlanEvent::Placed(_) => self.tool = Tool::Select,
                PlanEvent::Status(text) => self.set_status(text),
            }
        }
    }

    /// The 3D view of the same home.
    fn scene_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (home, revision, project) = {
            let doc = self.document.read();
            (doc.home().clone(), doc.revision(), doc.asset_dir())
        };
        self.scene.ui(
            ui,
            frame.wgpu_render_state(),
            &home,
            revision,
            &self.selection,
            project.as_deref(),
        );
    }

    /// On a narrow screen: which of the three is on show. Big enough for a
    /// thumb, at the bottom, where the thumb already is.
    fn pane_bar(&mut self, ui: &mut egui::Ui) {
        let t = crate::theme::of(ui.visuals());
        ui.horizontal(|ui| {
            let each = (ui.available_width() - 16.0) / 3.0;
            for (pane, glyph, label) in [
                (Pane::Catalog, icon::ARMCHAIR, crate::i18n::tr("Catálogo")),
                (Pane::Plan, icon::SQUARES_FOUR, crate::i18n::tr("Planta")),
                (Pane::Scene, icon::CUBE, crate::i18n::tr("3D")),
            ] {
                let on = self.pane == pane;
                let button = egui::Button::new(
                    RichText::new(format!("{glyph}  {label}"))
                        .size(15.0)
                        .color(if on { t.accent } else { t.ink }),
                )
                .min_size(egui::vec2(each, 38.0))
                .fill(if on { t.accent_soft } else { t.raised });
                if ui.add(button).clicked() {
                    self.pane = pane;
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = crate::theme::of(ui.visuals());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let people: Vec<(String, [u8; 3])> = self
                .document
                .read()
                .sessions()
                .list()
                .iter()
                .map(|s| (s.name.clone(), s.color))
                .collect();
            if !people.is_empty() {
                ui.separator();
                ui.label(format!("{} {}", icon::USERS, people.len()))
                    .on_hover_ui(|ui| {
                        for (name, [r, g, b]) in &people {
                            ui.colored_label(egui::Color32::from_rgb(*r, *g, *b), name);
                        }
                    });
            }
            let narrow = ui.ctx().content_rect().width() < NARROW;
            if narrow {
                // A phone's status bar holds the lamp, the zoom and whatever
                // just happened — the pointer's coordinates are for a mouse.
                ui.separator();
                ui.label(crate::theme::fig(
                    ui.visuals(),
                    &format!("{:.0}%", self.plan.zoom_percent()),
                ));
                if let Some((text, at)) = &self.status {
                    if at.elapsed() < Duration::from_secs(8) {
                        ui.separator();
                        ui.label(RichText::new(text).color(ui.visuals().text_color()));
                    } else {
                        self.status = None;
                    }
                }
                return;
            }
            ui.separator();
            if let Some(p) = self.plan.cursor() {
                let unit = self.settings.unit;
                ui.label(
                    crate::theme::fig(
                        ui.visuals(),
                        &format!(
                            "x {}   y {}",
                            unit.format_length(p.x),
                            unit.format_length(p.y)
                        ),
                    )
                    .color(t.ink_dim),
                );
                ui.separator();
            }
            ui.label(crate::theme::fig(
                ui.visuals(),
                &format!("{:.0}%", self.plan.zoom_percent()),
            ));
            ui.separator();
            ui.label(RichText::new(tool_hint(self.tool)).color(t.ink_faint));
            if let Some((text, at)) = &self.status {
                if at.elapsed() < Duration::from_secs(8) {
                    ui.separator();
                    let color = if text.starts_with('⚠') {
                        ui.visuals().warn_fg_color
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.colored_label(color, text);
                } else {
                    self.status = None;
                }
            }
        });
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let doc = self.document.read();
        let file = doc.path().and_then(Path::file_name).map_or_else(
            || doc.home().name.clone(),
            |n| n.to_string_lossy().into_owned(),
        );
        let title = format!(
            "{}{} — 3D New Era AI",
            if doc.is_modified() { "● " } else { "" },
            file
        );
        drop(doc);
        if title != self.title {
            // The browser tab title is the page's; windows get a viewport command.
            #[cfg(target_arch = "wasm32")]
            if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                document.set_title(&title);
            }
            #[cfg(not(target_arch = "wasm32"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            #[cfg(target_arch = "wasm32")]
            let _ = ctx;
            self.title = title;
        }
    }
}

pub(crate) const TOOLS: [(Tool, &str, &str, &str); 7] = [
    (Tool::Select, icon::CURSOR, "Selecionar", "V"),
    (Tool::Pan, icon::HAND, "Mover vista", "H"),
    (Tool::Walls, icon::WALL, "Criar paredes", "W"),
    (Tool::Rooms, icon::POLYGON, "Criar cômodos", "R"),
    (Tool::Dimensions, icon::RULER, "Criar cotas", "D"),
    (Tool::Labels, icon::TEXT_T, "Adicionar texto", "T"),
    (Tool::Lines, icon::LINE_SEGMENTS, "Desenhar linhas", "L"),
];

fn tool_hint(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => crate::i18n::tr(
            "Clique seleciona · Ctrl+clique soma · arraste move · alças editam · duplo clique modifica",
        ),
        Tool::Pan => crate::i18n::tr("Arraste para mover a vista"),
        Tool::Walls => crate::i18n::tr(
            "Clique encadeia paredes · digite o comprimento + Enter · Shift desliga o ímã · duplo clique encerra",
        ),
        Tool::Rooms => crate::i18n::tr(
            "Clique os cantos e duplo clique fecha · duplo clique dentro de paredes detecta o cômodo",
        ),
        Tool::Dimensions => crate::i18n::tr(
            "Clique início e fim, mova para afastar e clique · duplo clique numa parede cota a parede",
        ),
        Tool::Labels => crate::i18n::tr("Clique onde o texto deve ficar"),
        Tool::Lines => crate::i18n::tr(
            "Clique os pontos e duplo clique encerra · na Elétrica/Hidráulica a linha vira eletroduto/tubulação",
        ),
        Tool::Calibrate => crate::i18n::tr(
            "Clique dois pontos de medida conhecida · arraste para posicionar a imagem",
        ),
        Tool::Place(_) => crate::i18n::tr(
            "Clique para posicionar · portas e janelas encaixam na parede mais próxima · Esc cancela",
        ),
    }
}

/// Rings the button in use with the accent. Drawn over it rather than set as
/// its stroke: egui sizes a button for the theme's stroke, which is none at
/// rest, so an extra one made the chosen button grow whenever the pointer
/// left it and nudged the whole row.
fn ring(ui: &egui::Ui, response: &egui::Response, color: egui::Color32) {
    let radius = ui.style().interact_selectable(response, true).corner_radius;
    ui.painter().rect_stroke(
        response.rect,
        radius,
        Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

/// A cluster of keys: buttons that belong together, sunk into the chrome as
/// one block, the way a keyboard groups its rows.
fn keys<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = crate::theme::of(ui.visuals());
    egui::Frame::new()
        .fill(t.inset)
        .stroke(Stroke::new(1.0, t.rule))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(3, 2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| add(ui)).inner
        })
        .inner
}

/// A lit indicator: the dot with a halo the site uses to say something is on.
pub(crate) fn lamp(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    let center = rect.center();
    ui.painter()
        .circle_filled(center, 5.0, color.gamma_multiply(0.22));
    ui.painter().circle_filled(center, 2.5, color);
}

pub(crate) fn menu_item(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    shortcut: &str,
    enabled: bool,
) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(format!("{glyph}  {label}")).shortcut_text(shortcut),
    )
    .clicked()
}

/// A copy of `element` with a fresh id, moved by `offset` cm on both axes.
/// The copy lands on the level being edited, like any new element.
fn shift_with_new_id(doc: &mut Document, mut element: Element, offset: f64) -> Element {
    let shift = |p: Point2| Point2::new(p.x + offset, p.y + offset);
    element.set_level(None);
    match element {
        Element::Level(mut level) => {
            level.id = doc.new_level_id();
            level.name = format!("{} {}", level.name, crate::i18n::tr("(cópia)"));
            Element::Level(level)
        }
        Element::Wall(mut w) => {
            w.id = doc.new_wall_id();
            w.start = shift(w.start);
            w.end = shift(w.end);
            Element::Wall(w)
        }
        Element::Room(mut r) => {
            r.id = doc.new_room_id();
            r.points = r.points.into_iter().map(shift).collect();
            Element::Room(r)
        }
        Element::Dimension(mut d) => {
            d.id = doc.new_dimension_id();
            d.start = shift(d.start);
            d.end = shift(d.end);
            Element::Dimension(d)
        }
        Element::Label(mut l) => {
            l.id = doc.new_label_id();
            l.position = shift(l.position);
            Element::Label(l)
        }
        Element::Furniture(mut f) => {
            f.id = doc.new_furniture_id();
            f.translate(offset, offset);
            Element::Furniture(f)
        }
        Element::Polyline(mut p) => {
            p.id = doc.new_polyline_id();
            p.points = p.points.into_iter().map(shift).collect();
            Element::Polyline(p)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
type PluginSlot = std::sync::Arc<std::sync::Mutex<Option<Result<serde_json::Value, String>>>>;

#[cfg(not(target_arch = "wasm32"))]
impl NewEraApp {
    /// Plugins found on disk; running one edits the home through the API.
    fn plugins_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(crate::i18n::tr("Plugins"), |ui| {
            let plugins = newera_plugins::discover(&newera_plugins::plugin_dirs());
            if plugins.is_empty() {
                ui.weak(crate::i18n::tr("Nenhum plugin instalado"));
            }
            let running = self.plugin_job.is_some();
            for plugin in plugins {
                let button =
                    egui::Button::new(format!("{} {}", icon::PUZZLE_PIECE, plugin.label()));
                let response = ui.add_enabled(!running, button);
                let response = if plugin.description.is_empty() {
                    response
                } else {
                    response.on_hover_text(&plugin.description)
                };
                if response.clicked() {
                    self.run_plugin(&plugin);
                    ui.close();
                }
            }
            ui.separator();
            ui.weak(format!(
                "{}: {}",
                crate::i18n::tr("Pastas"),
                newera_plugins::plugin_dirs()
                    .iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        });
    }

    pub(crate) fn run_plugin(&mut self, plugin: &newera_plugins::Plugin) {
        let slot: PluginSlot = std::sync::Arc::default();
        let (out, document, name) = (slot.clone(), self.document.clone(), plugin.name.clone());
        std::thread::spawn(move || {
            let result = newera_plugins::run_for_document(
                &document,
                &newera_plugins::plugin_dirs(),
                &name,
                &serde_json::Value::Null,
            )
            .map_err(|e| e.to_string());
            if let Ok(mut guard) = out.lock() {
                *guard = Some(result);
            }
        });
        self.set_status(format!(
            "{} {}…",
            crate::i18n::tr("Rodando"),
            plugin.label()
        ));
        self.plugin_job = Some((plugin.label().to_owned(), slot));
    }

    /// Reports a finished plugin in the status bar.
    fn collect_plugin(&mut self) {
        let Some((title, slot)) = &self.plugin_job else {
            return;
        };
        let Some(result) = slot.lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        let title = title.clone();
        self.plugin_job = None;
        let status = match result {
            Ok(out) if out["ok"] == true => {
                let first = out["stdout"]
                    .as_str()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                format!(
                    "{title}: {}",
                    if first.is_empty() {
                        crate::i18n::tr("concluído").to_owned()
                    } else {
                        first
                    }
                )
            }
            Ok(out) => {
                let err = out["stderr"]
                    .as_str()
                    .unwrap_or("")
                    .lines()
                    .last()
                    .unwrap_or("")
                    .to_owned();
                format!("⚠ {title}: {err}")
            }
            Err(err) => format!("⚠ {title}: {err}"),
        };
        self.set_status(status);
    }
}

#[cfg(target_arch = "wasm32")]
impl NewEraApp {
    /// Opens requested pickers and loads the files the browser handed over.
    /// Opens the browser's file picker for whatever asked for one this
    /// frame. Called at the *end* of the frame, so a button pressed in this
    /// one opens the picker in this one: waiting for the next frame means
    /// waiting for a repaint that an idle window has no reason to draw, and
    /// the browser may have dropped the click's user activation by then —
    /// which is a picker that never opens.
    fn web_pick(&mut self, ctx: &egui::Context) {
        if let Some(kind) = self.pick_request.take() {
            crate::files::pick(kind, ctx);
        }
    }

    fn web_files(&mut self, ctx: &egui::Context) {
        let _ = ctx;
        for picked in crate::files::take_picked() {
            match picked.kind {
                crate::files::PickKind::Project => {
                    self.open_bytes(&picked.name, &picked.bytes);
                }
                crate::files::PickKind::Background => {
                    self.place_background(&picked.name, &picked.bytes);
                }
            }
        }
    }
}

impl NewEraApp {
    /// Loads a project from memory: a `.newera` (JSON or bundle, whose models
    /// and textures are mounted in memory) or a Sweet Home 3D `.sh3d`.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> bool {
        let assets = std::path::PathBuf::from("/memory").join(name);
        let previous_assets = self.document.read().asset_dir();
        let sh3d = name.to_ascii_lowercase().ends_with(".sh3d");
        if sh3d {
            let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
            return match newera_sh3d::import_bytes(bytes, stem, &assets) {
                Ok((imported, _files)) => {
                    if let Some(previous) = &previous_assets
                        && *previous != assets
                    {
                        newera_core::vfs::unmount(previous);
                    }
                    let mut doc = self.document.write();
                    doc.load(imported.home);
                    doc.set_path(None);
                    doc.set_asset_dir(Some(assets));
                    drop(doc);
                    self.after_load();
                    let mut status = crate::i18n::fill(
                        "Importado de {} — salve como projeto para manter tudo num arquivo",
                        &[&name],
                    );
                    if !imported.warnings.is_empty() {
                        status.push_str(&crate::i18n::fill(
                            " · {} aviso(s)",
                            &[&imported.warnings.len()],
                        ));
                    }
                    self.set_status(status);
                    true
                }
                Err(err) => {
                    self.set_status(crate::i18n::fill(
                        "⚠ Não foi possível abrir {}: {}",
                        &[&name, &err],
                    ));
                    false
                }
            };
        }
        match newera_core::project_from_bytes(bytes) {
            Ok((project, files)) => {
                if let Some(previous) = &previous_assets {
                    newera_core::vfs::unmount(previous);
                }
                let bundled = !files.is_empty();
                newera_core::vfs::mount(&assets, files);
                let mut doc = self.document.write();
                project.load_into(&mut doc);
                doc.mark_saved(name);
                doc.set_asset_dir(bundled.then_some(assets));
                drop(doc);
                self.after_load();
                self.set_status(crate::i18n::fill("Aberto: {}", &[&name]));
                true
            }
            Err(err) => {
                self.set_status(crate::i18n::fill(
                    "⚠ Não foi possível abrir {}: {}",
                    &[&name, &err],
                ));
                false
            }
        }
    }
}

impl eframe::App for NewEraApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // A browser canvas starts at 300×150 until the page lays it out; panel
        // sizes chosen then would stick, so wait for the real size.
        #[cfg(target_arch = "wasm32")]
        if ctx.content_rect().height() < 300.0 && self.waited_frames < 120 {
            self.waited_frames += 1;
            ctx.request_repaint();
            return;
        }

        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.is_modified()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmDiscard(Pending::Quit));
        }
        if self.pending == Some(Pending::Quit) {
            self.pending = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(target_arch = "wasm32")]
        self.web_files(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.collect_plugin();
        self.follow_document_camera();
        crate::ai::announce(self);
        if self
            .top_views
            .as_ref()
            .is_some_and(|(_, _, views, _)| views.busy())
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // Long work runs behind a window that says so; while it does, the
        // rest of the interface is there to read but not to touch.
        crate::jobs::show(self, &ctx);
        #[cfg(target_arch = "wasm32")]
        crate::render_web::show(&ctx);
        crate::photo::show(self, &ctx);
        crate::video::show(self, &ctx);
        crate::ergonomics::show(self, &ctx);
        crate::cabinets::show(self, &ctx);

        self.follow_theme(&ctx);
        self.shortcuts(&ctx);
        self.update_title(&ctx);

        // Three heights of chrome: the command bar lifts off the panels, the
        // panels hold the work, the status bar sinks into the desk.
        let t = crate::theme::of(&ctx.style_of(ctx.theme()).visuals);
        egui::Panel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(t.raised)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                self.menu_bar(ui);
                self.toolbar(ui);
            });
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(t.deep)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| self.status_bar(ui));
        // A phone cannot hold catalogue, plan and 3D side by side, and pretending
        // otherwise gives all three at a size nobody can use. Below this width
        // the window shows one at a time, and a bar within thumb's reach says
        // which — everything else about the editor stays the same.
        #[cfg(target_arch = "wasm32")]
        self.fit_to_screen(&ctx);
        let narrow = ctx.content_rect().width() < NARROW;
        if narrow {
            egui::Panel::bottom("panes")
                .frame(
                    egui::Frame::new()
                        .fill(t.raised)
                        .inner_margin(egui::Margin::symmetric(8, 6)),
                )
                .show(ui, |ui| self.pane_bar(ui));
        } else {
            egui::Panel::left("left")
                .resizable(true)
                .default_size(270.0)
                .show(ui, |ui| panels::left(self, ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                egui::Panel::top("variant_tabs")
                    .frame(
                        egui::Frame::new()
                            .fill(t.surface)
                            .inner_margin(egui::Margin::symmetric(6, 3)),
                    )
                    .show(ui, |ui| crate::tabs::bar(self, ui));
                if narrow {
                    // One thing at a time, the whole screen for it.
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(
                        ui,
                        |ui| match self.pane {
                            Pane::Catalog => panels::left(self, ui),
                            Pane::Plan => self.plan_ui(ui),
                            Pane::Scene => self.scene_ui(ui, frame),
                        },
                    );
                    return;
                }
                egui::Panel::top("plan")
                    .resizable(true)
                    .default_size(ui.available_height() / 2.0)
                    // Never collapse, e.g. when a browser canvas starts tiny.
                    .min_size(120.0)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.plan_ui(ui));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.scene_ui(ui, frame));
            });

        // Selection may point to elements removed by undo or by an agent.
        {
            let doc = self.document.read();
            self.selection.retain(|id| doc.home().contains(*id));
        }

        if let Some(dialog) = self.dialog.take() {
            match crate::dialogs::show(self, &ctx, dialog) {
                DialogOutcome::Keep(dialog) => {
                    if self.dialog.is_none() {
                        self.dialog = Some(dialog);
                    }
                }
                DialogOutcome::Close => {}
            }
        }

        ctx.request_repaint_after(EXTERNAL_CHANGES_POLL);

        // Last, so a button pressed this frame gets its picker this frame.
        #[cfg(target_arch = "wasm32")]
        self.web_pick(&ctx);

        // A browser tab is closed without a question: what was drawn goes to
        // the browser's storage a moment after it changes, not half a minute.
        #[cfg(target_arch = "wasm32")]
        if self.mirrored.1.elapsed() > Duration::from_millis(1500) {
            self.mirror();
        }
        // A cloud project goes back to the service a moment after it changes.
        #[cfg(target_arch = "wasm32")]
        self.autosave(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        #[cfg(target_arch = "wasm32")]
        self.mirror();
    }
}

#[cfg(test)]
mod tests {
    //! Whole-app tests through `egui_kittest`: keyboard shortcuts, dialogs and
    //! their effect on the document.

    use eframe::egui::{Key, Modifiers};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use newera_core::{Point2, Wall, WallId};

    use super::*;

    fn app_with_wall() -> Harness<'static, NewEraApp> {
        let mut doc = Document::default();
        let wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
        );
        doc.execute(Command::insert(wall)).unwrap();
        let document = SharedDocument::new(doc);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |cc| NewEraApp::new(cc, document, None));
        harness.run_steps(5);
        harness
    }

    fn walls(h: &Harness<'_, NewEraApp>) -> Vec<Wall> {
        h.state().document.read().home().walls.clone()
    }

    #[test]
    fn the_load_schedule_opens_from_the_discipline_menu() {
        let mut h = app_with_wall();
        {
            let document = h.state().document.clone();
            let mut doc = document.write();
            doc.set_active_discipline(Some(newera_core::Discipline::Electrical));
            for (catalog, x, circuit) in
                [("light-ceiling", 100.0, "C1"), ("outlet-low", 200.0, "C2")]
            {
                let id = doc.new_furniture_id();
                let mut point = newera_catalog::find(catalog)
                    .unwrap()
                    .instantiate(id, Point2::new(x, 100.0));
                point
                    .properties
                    .insert(newera_core::electrical::CIRCUIT_KEY.into(), circuit.into());
                doc.execute(Command::insert(point)).unwrap();
            }
        }
        h.run_steps(3);
        // The discipline combo shows the project being edited as its value.
        h.get_by(|node| {
            node.role() == egui::accesskit::Role::ComboBox
                && node.value().is_some_and(|v| v.contains("Elétrica"))
        })
        .click();
        h.run_steps(3);
        h.get_by_label_contains("Quadro de cargas e NBR 5410")
            .click();
        h.run_steps(4);
        h.get_by_label("C2");
        h.get_by_label_contains("Total instalado:");
    }

    /// A whole line opens a band, not just the little arrow at its start:
    /// nobody aims at a triangle.
    #[test]
    fn clicking_the_name_of_a_band_opens_it() {
        let mut h = app_with_wall();
        assert!(
            h.query_by_label_contains("Sofá 3 lugares").is_none(),
            "the band starts shut"
        );
        h.get_by_label("Sala de estar").click();
        h.run_steps(3);
        h.get_by_label_contains("Sofá 3 lugares");
        h.get_by_label("Sala de estar").click();
        h.run_steps(3);
        assert!(
            h.query_by_label_contains("Sofá 3 lugares").is_none(),
            "and the same click shuts it again"
        );
    }

    /// What is picked out on the plan has to light up in the panel, and the
    /// panel has to bring it into view: a selection nobody can see is no
    /// better than no selection.
    #[test]
    fn picking_on_the_plan_lights_the_row_up_in_the_panel() {
        use egui::accesskit::Toggled;
        use egui_kittest::kittest::NodeT as _;

        let mut h = app_with_wall();
        let quiet = h.get_by_label("w1");
        assert_eq!(quiet.accesskit_node().toggled(), Some(Toggled::False));

        let id = h.state().document.read().home().walls[0].id;
        h.state_mut().selection.insert(id.into());
        h.run_steps(3);

        let lit = h.get_by_label("w1");
        assert_eq!(lit.accesskit_node().toggled(), Some(Toggled::True));
    }

    #[test]
    fn choosing_architecture_hides_the_electrical_project() {
        let mut h = app_with_wall();
        h.state()
            .document
            .write()
            .choose_view(Some(newera_core::Discipline::Electrical));
        h.run_steps(3);
        h.get_by(|node| {
            node.role() == egui::accesskit::Role::ComboBox
                && node.value().is_some_and(|v| v.contains("Elétrica"))
        })
        .click();
        h.run_steps(3);
        h.get_by_label_contains("Arquitetura").click();
        h.run_steps(3);
        let doc = h.state().document.read();
        assert_eq!(doc.home().active_discipline, None);
        assert!(
            doc.home()
                .hidden_disciplines
                .contains(&newera_core::Discipline::Electrical)
        );
    }

    #[test]
    fn the_layers_menu_hides_the_whole_electrical_project_lamps_included() {
        let mut h = app_with_wall();
        {
            let document = h.state().document.clone();
            let mut doc = document.write();
            for (cat, at) in [("pendant", (200.0, 150.0)), ("outlet-low", (100.0, 5.0))] {
                let id = doc.new_furniture_id();
                let piece = newera_catalog::find(cat)
                    .unwrap()
                    .instantiate(id, Point2::new(at.0, at.1));
                doc.execute(Command::insert(piece)).unwrap();
            }
        }
        h.run_steps(3);
        h.get_by_label_contains("Camadas").click();
        h.run_steps(3);
        h.get_by_label_contains("Elétrica (tomadas, luz, cabos)")
            .click();
        h.run_steps(3);
        let doc = h.state().document.read();
        let home = doc.home();
        assert!(
            home.hidden_disciplines
                .contains(&newera_core::Discipline::Electrical)
        );
        assert!(
            home.layer_hidden(newera_core::PlanLayer::Lighting),
            "the lamps go with it"
        );
        assert!(!home.shown_in_3d(Some(newera_core::Discipline::Electrical), None));
        assert!(!home.shown_in_3d(None, Some(newera_core::PlanLayer::Lighting)));
        assert!(!home.layer_hidden(newera_core::PlanLayer::Joinery));
    }

    #[test]
    fn the_layers_menu_hides_lighting_from_the_plan() {
        let mut h = app_with_wall();
        {
            let document = h.state().document.clone();
            let mut doc = document.write();
            let id = doc.new_furniture_id();
            let pendant = newera_catalog::find("pendant")
                .unwrap()
                .instantiate(id, Point2::new(200.0, 150.0));
            doc.execute(Command::insert(pendant)).unwrap();
        }
        h.run_steps(3);
        h.get_by_label_contains("Camadas").click();
        h.run_steps(3);
        h.get_by_label_contains("Iluminação · 1").click();
        h.run_steps(3);
        {
            let doc = h.state().document.read();
            assert!(
                doc.home()
                    .hidden_layers
                    .contains(&newera_core::PlanLayer::Lighting)
            );
            assert_eq!(doc.home().furniture.len(), 1, "the piece is still there");
            assert!(
                !doc.home()
                    .shown_in_3d(None, Some(newera_core::PlanLayer::Lighting)),
                "and gone from the 3D too"
            );
        }
        if h.query_by_label_contains("Mostrar tudo no 3D").is_none() {
            h.get_by_label_contains("Camadas").click();
            h.run_steps(3);
        }
        h.get_by_label_contains("Mostrar tudo no 3D").click();
        h.run_steps(3);
        let doc = h.state().document.read();
        assert!(doc.home().show_all_in_3d);
        assert!(
            doc.home()
                .shown_in_3d(None, Some(newera_core::PlanLayer::Lighting)),
            "shown all, it is back in 3D"
        );
    }

    #[test]
    fn telemetry_is_on_by_default_and_the_help_menu_turns_it_off() {
        let dir = std::env::temp_dir().join(format!("newera-app-telemetry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Never the switch of the machine running the tests.
        newera_telemetry::use_config_dir(dir.clone());
        let mut h = app_with_wall();
        assert!(
            newera_telemetry::enabled(),
            "on until someone says otherwise"
        );

        let toggle = |h: &mut Harness<'_, NewEraApp>| {
            h.get_by_label("Ajuda").click();
            h.run_steps(3);
            h.get_by_label_contains("Enviar relatórios de erro").click();
            h.run_steps(3);
            h.key_press(Key::Escape);
            h.run_steps(2);
        };
        toggle(&mut h);
        assert!(!newera_telemetry::enabled(), "one click turns it off");
        let saved = std::fs::read_to_string(dir.join("telemetry.json")).unwrap();
        assert!(saved.contains("false"), "and it is kept: {saved}");

        toggle(&mut h);
        assert!(newera_telemetry::enabled(), "and back on");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Whoever opens this window has to be able to see that their agent is
    /// really talking to it — and find out how, when it is not.
    #[test]
    fn the_window_shows_the_ai_connecting_and_how_to_connect_it() {
        let mut h = app_with_wall();
        h.state_mut().mcp_url = Some("http://127.0.0.1:7878/mcp".to_owned());
        h.run_steps(3);
        h.get_by_label_contains("IA · esperando");

        // The icon in its own toolbar group follows Frame and opens the panel.
        let frame = h.get_by_label(icon::CORNERS_OUT).rect();
        let robot = h.get_by_label_contains("IA · esperando").rect();
        assert!(robot.left() > frame.right());
        assert!((robot.center().y - frame.center().y).abs() < 2.0);
        h.get_by_label_contains("IA · esperando").click();
        h.run_steps(3);
        // The command is idempotent on purpose: `claude mcp add` refuses a name
        // that already exists, and somebody who installed the app has one.
        h.get_by_label_contains(
            "claude mcp remove newera 2>/dev/null; claude mcp add --transport http newera http://127.0.0.1:7878/mcp",
        );
        h.get_by_label_contains("No ar, esperando a sua IA");
        h.key_press(Key::Escape);
        h.run_steps(3);

        // An agent says hello and calls a tool: both show up without a click.
        {
            let mut doc = h.state().document.write();
            let now = newera_core::collab::now_ms();
            doc.agents_mut()
                .hello(Some("s1"), "claude-code", Some("2.0.0"), now);
            doc.agents_mut().called(Some("s1"), "place", now);
        }
        h.run_steps(3);
        h.get_by_label_contains("claude-code conectado · 1 chamadas");
        // And it says so once, where messages go — then names each tool as
        // it lands, so the work is visible without opening anything.
        h.get_by_label_contains("claude-code conectou");
        {
            let mut doc = h.state().document.write();
            doc.agents_mut()
                .called(Some("s1"), "render_photo", newera_core::collab::now_ms());
        }
        h.run_steps(3);
        h.get_by_label_contains("render_photo · claude-code");
    }

    /// The same icon remains available in the horizontally scrolling toolbar.
    #[test]
    fn the_ai_button_remains_in_the_narrow_toolbar() {
        let document = SharedDocument::new(Document::default());
        let mut narrow = Harness::builder()
            .with_size(egui::vec2(520.0, 780.0))
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |cc| NewEraApp::new(cc, document, None));
        narrow.run_steps(5);
        narrow.get_by_label_contains("Conectar sua IA");
    }

    #[test]
    fn collaborators_show_up_and_plugins_report_back() {
        let mut h = app_with_wall();
        {
            let mut doc = h.state().document.write();
            let ana = doc
                .sessions_mut()
                .join("Ana", newera_core::collab::now_ms());
            doc.sessions_mut().update(
                &ana.id,
                newera_core::collab::Presence {
                    cursor: Some(Point2::new(100.0, 0.0)),
                    ..Default::default()
                },
                newera_core::collab::now_ms(),
            );
        }
        h.run_steps(3);
        h.get_by_label_contains(&format!("{} 1", icon::USERS));

        // No HTTP server in this test: the plugin can't run and says why.
        let root = std::env::temp_dir().join(format!("newera-app-plugin-{}", std::process::id()));
        std::fs::create_dir_all(root.join("p")).unwrap();
        std::fs::write(
            root.join("p/plugin.json"),
            r#"{"name":"p","title":"P","command":["true"]}"#,
        )
        .unwrap();
        let plugin = newera_plugins::load(&root.join("p")).unwrap();
        h.state_mut().run_plugin(&plugin);
        for _ in 0..200 {
            h.run_steps(1);
            if h.state().plugin_job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let status = h
            .state()
            .status
            .as_ref()
            .map(|(s, _)| s.clone())
            .unwrap_or_default();
        assert!(status.starts_with("⚠ P:"), "{status}");
        std::fs::remove_dir_all(root).ok();
    }

    /// Work that takes seconds has to keep the window alive, say what it is
    /// doing and land its result back here when it ends.
    #[test]
    fn long_work_reports_itself_instead_of_freezing_the_window() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut h = app_with_wall();
        let finish = Arc::new(AtomicBool::new(false));
        let told = Arc::clone(&finish);
        crate::jobs::start(
            h.state_mut(),
            "Abrindo projeto",
            "casa.newera".to_owned(),
            move || {
                newera_core::progress::step("Extraindo imagens e modelos", 3, 10);
                while !told.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Box::new(|app: &mut NewEraApp| app.set_status("Aberto: casa.newera"))
            },
        );
        // The work reports from a thread of its own, which may not have run a
        // single line by the time the first frames are drawn: the window is
        // read once what the work says has reached it, not a fixed three
        // frames later.
        for _ in 0..400 {
            h.run_steps(1);
            if h.query_by_label_contains("30%").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // The window says what is happening, to what, and how far along.
        h.get_by_label_contains("Abrindo projeto");
        h.get_by_label_contains("casa.newera");
        h.get_by_label_contains("Extraindo imagens e modelos");
        h.get_by_label_contains("30%");
        // And nothing else can be started meanwhile.
        assert!(crate::jobs::busy(h.state()));

        finish.store(true, Ordering::Relaxed);
        for _ in 0..400 {
            h.run_steps(1);
            if h.state().job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(h.state().job.is_none(), "the job ended");
        assert_eq!(
            h.state().status.as_ref().map(|(s, _)| s.as_str()),
            Some("Aberto: casa.newera"),
            "its result ran on the window's thread"
        );
    }

    /// Work that does not stop when asked must not hold the window hostage:
    /// after a moment there is a way out, and it leaves the work behind.
    #[test]
    fn work_that_will_not_stop_can_be_left_behind() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut h = app_with_wall();
        let stop = Arc::new(AtomicBool::new(false));
        let told = Arc::clone(&stop);
        crate::jobs::start(
            h.state_mut(),
            "Salvando projeto",
            String::new(),
            move || {
                // Deaf to being cancelled, like a write already under way.
                while !told.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Box::new(|app: &mut NewEraApp| app.set_status("tarde demais"))
            },
        );
        h.run_steps(3);
        h.get_by_label_contains("Cancelar").click();
        h.run_steps(2);
        h.get_by_label_contains("Parando…");

        // The work ignores it, so the way out appears a couple of seconds on.
        let waited = std::time::Instant::now();
        while waited.elapsed() < std::time::Duration::from_millis(2500) {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.get_by_label_contains("Forçar").click();
        h.run_steps(2);
        assert!(h.state().job.is_none(), "the window stopped waiting");
        assert!(
            h.state()
                .status
                .as_ref()
                .is_some_and(|(s, _)| s.contains("segundo plano")),
            "and says the task is still out there"
        );
        stop.store(true, Ordering::Relaxed);
    }

    /// The real thing, with a file on disk: opening and saving are jobs of
    /// their own — the window they show is exercised above — and their result
    /// lands back on this thread with the project in hand. A file this small
    /// is read before the window can even appear, which is the point: the
    /// progress window is for the ones that are not.
    #[test]
    fn opening_and_saving_a_file_run_behind_the_progress_window() {
        let dir = std::env::temp_dir().join(format!("newera-app-job-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("casa.newera");
        {
            let mut other = Document::default();
            for x in [0.0, 500.0] {
                let wall = Wall::new(
                    other.new_wall_id(),
                    Point2::new(x, 0.0),
                    Point2::new(x, 300.0),
                );
                other.execute(Command::insert(wall)).unwrap();
            }
            newera_core::save_project(&other, &path).unwrap();
        }

        let mut h = app_with_wall();
        h.state_mut().open_path(&path);
        assert!(
            crate::jobs::busy(h.state()),
            "reading it is a job of its own"
        );
        for _ in 0..400 {
            h.run_steps(1);
            if h.state().job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(walls(&h).len(), 2, "the file is open");
        assert!(!h.state().is_modified());

        // Saving goes the same way, straight to the path it came from.
        h.state_mut().run(|doc| {
            let wall = Wall::new(
                doc.new_wall_id(),
                Point2::new(0.0, 300.0),
                Point2::new(500.0, 300.0),
            );
            doc.execute(Command::insert(wall))
        });
        assert!(h.state().is_modified());
        let before = std::fs::metadata(&path).unwrap().len();
        assert!(h.state_mut().save(false), "saving started");
        for _ in 0..400 {
            h.run_steps(1);
            if h.state().job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!h.state().is_modified(), "saved");
        assert_ne!(std::fs::metadata(&path).unwrap().len(), before);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opens_projects_from_memory_like_the_browser_editor() {
        let mut h = app_with_wall();
        let mut other = Document::default();
        for x in [0.0, 500.0] {
            let wall = Wall::new(
                other.new_wall_id(),
                Point2::new(x, 0.0),
                Point2::new(x, 300.0),
            );
            other.execute(Command::insert(wall)).unwrap();
        }
        let json = newera_core::to_project_json(&other);
        assert!(h.state_mut().open_bytes("outra.newera", json.as_bytes()));
        h.run_steps(2);
        assert_eq!(walls(&h).len(), 2);
        assert!(!h.state().is_modified());
        let assets = std::path::PathBuf::from("/memory/recovery-keeps-assets");
        newera_core::vfs::mount(&assets, [("model.bin".into(), vec![1, 2, 3])]);
        h.state_mut()
            .document
            .write()
            .set_asset_dir(Some(assets.clone()));
        assert!(!h.state_mut().open_bytes("ruim.newera", b"not a project"));
        assert_eq!(
            newera_core::vfs::read(&assets.join("model.bin")).unwrap(),
            vec![1, 2, 3]
        );
        newera_core::vfs::unmount(&assets);
        assert_eq!(walls(&h).len(), 2, "a bad file leaves the project alone");
        assert!(
            h.state()
                .status
                .as_ref()
                .is_some_and(|(s, _)| s.starts_with('⚠'))
        );
    }

    #[test]
    fn delete_undo_and_duplicate_shortcuts() {
        let mut h = app_with_wall();
        h.state_mut().selection.insert(WallId(1).into());
        h.key_press(Key::Delete);
        h.run_steps(3);
        assert!(walls(&h).is_empty(), "Delete removes the selection");

        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        assert_eq!(walls(&h).len(), 1, "Ctrl+Z restores it");

        h.state_mut().selection.insert(WallId(1).into());
        h.key_press_modifiers(Modifiers::COMMAND, Key::D);
        h.run_steps(3);
        let all = walls(&h);
        assert_eq!(all.len(), 2, "Ctrl+D duplicates");
        assert_eq!(all[1].start, Point2::new(PASTE_OFFSET, PASTE_OFFSET));
        assert_eq!(
            h.state().selection.len(),
            1,
            "the copy becomes the selection"
        );
    }

    #[test]
    fn tool_keys_switch_tools() {
        let mut h = app_with_wall();
        for (key, tool) in [
            (Key::W, Tool::Walls),
            (Key::R, Tool::Rooms),
            (Key::D, Tool::Dimensions),
            (Key::V, Tool::Select),
        ] {
            h.key_press(key);
            h.run_steps(2);
            assert_eq!(h.state().tool, tool);
        }
    }

    #[test]
    fn enter_opens_the_modify_dialog_and_ok_applies_it() {
        let mut h = app_with_wall();
        h.state_mut().selection.insert(WallId(1).into());
        h.key_press(Key::Enter);
        h.run_steps(3);
        let Some(Dialog::ModifyWalls { ids, .. }) = h.state().dialog.clone() else {
            panic!("modify dialog should be open");
        };
        assert_eq!(ids, vec![WallId(1)]);

        // Change the draft as the drag values would, then press OK.
        if let Some(Dialog::ModifyWalls { thickness, arc, .. }) = &mut h.state_mut().dialog {
            *thickness = 30.0;
            *arc = 45.0;
        }
        h.run_steps(2);
        h.get_by_label_contains("OK").click();
        h.run_steps(5);
        let wall = walls(&h).remove(0);
        assert!((wall.thickness - 30.0).abs() < 1e-9);
        assert_eq!(wall.arc_extent, Some(45.0));
        assert!(h.state().dialog.is_none(), "dialog closes");
    }

    #[test]
    fn wall_dialog_picks_a_type_and_a_side_finish_by_clicking() {
        let mut h = app_with_wall();
        h.state_mut().selection.insert(WallId(1).into());
        h.key_press(Key::Enter);
        h.run_steps(3);
        h.get_by_value("Personalizada").click();
        h.run_steps(3);
        h.get_by_label_contains("Drywall 95 mm").click();
        h.run_steps(3);
        // Two finish combos (left, right); the first is the left side.
        h.get_all_by_value("Sem acabamento")
            .next()
            .expect("left finish")
            .click();
        h.run_steps(3);
        h.get_by_label("Tijolo aparente").click();
        h.run_steps(3);
        h.get_by_label_contains("OK").click();
        h.run_steps(5);
        let wall = walls(&h).remove(0);
        assert_eq!(wall.wall_type.as_deref(), Some("drywall-95"));
        assert!((wall.thickness - 9.5).abs() < 1e-9, "{}", wall.thickness);
        assert_eq!(
            wall.left_side,
            Some(newera_core::Material::pattern(newera_core::Pattern::Brick))
        );
        assert!(wall.right_side.is_none());
    }

    #[test]
    fn modify_room_name_by_typing() {
        let mut h = app_with_wall();
        let room_id = {
            let mut doc = h.state().document.write();
            let room = newera_core::Room::new(
                doc.new_room_id(),
                "",
                vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(300.0, 0.0),
                    Point2::new(300.0, 300.0),
                ],
            );
            let id = room.id;
            doc.execute(Command::insert(room)).unwrap();
            id
        };
        h.run_steps(2);
        h.state_mut().open_modify(&[room_id.into()]);
        h.run_steps(3);
        // The catalog search is also a text input; the dialog's comes last.
        h.get_all_by_role(egui::accesskit::Role::TextInput)
            .last()
            .expect("room name field")
            .type_text("Suíte");
        h.run_steps(3);
        h.get_by_label_contains("OK").click();
        h.run_steps(5);
        assert_eq!(
            h.state().document.read().home().room(room_id).unwrap().name,
            "Suíte"
        );
    }
}

#[cfg(test)]
mod screenshots {
    //! Renders the real UI headlessly for visual review:
    //! `cargo test -p newera-app screenshots -- --ignored`, images land in
    //! `target/screenshots/`.

    use egui_kittest::Harness;
    use newera_core::{Point2, Room, Wall};

    use super::*;

    fn render(name: &str, setup: impl FnOnce(&mut NewEraApp)) {
        let mut doc = Document::default();
        let pts = [(0.0, 0.0), (600.0, 0.0), (600.0, 400.0), (0.0, 400.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            let wall = Wall::new(
                doc.new_wall_id(),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            );
            doc.execute(Command::insert(wall)).unwrap();
        }
        let room = Room::new(
            doc.new_room_id(),
            "Sala",
            pts.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        );
        doc.execute(Command::insert(room)).unwrap();
        let document = SharedDocument::new(doc);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_eframe(move |cc| {
                NewEraApp::new(cc, document, Some("http://127.0.0.1:7878/mcp".into()))
            });
        harness.run_steps(5);
        setup(harness.state_mut());
        harness.run_steps(10);
        let image = harness.render().expect("render");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join(format!("{name}.png"))).unwrap();
    }

    #[test]
    #[ignore = "visual review; needs a GPU"]
    fn dialogs() {
        render("modify-wall", |app| {
            app.selection.insert(newera_core::WallId(1).into());
            app.open_modify(&[newera_core::WallId(1).into()]);
        });
        render("home-settings", NewEraApp::open_home_settings);
        render("help", |app| app.dialog = Some(Dialog::Help));
        render("confirm-discard", |app| {
            app.dialog = Some(Dialog::ConfirmDiscard(Pending::New));
        });
        render("calibrate", |app| {
            app.dialog = Some(Dialog::Calibrate {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(600.0, 0.0),
                distance: 500.0,
            });
        });
    }
}

#[cfg(test)]
mod furniture_screenshots {
    use egui_kittest::Harness;
    use newera_core::{Point2, Room, Wall, align_to_wall};

    use super::*;

    fn furnished() -> Document {
        let mut doc = Document::default();
        let pts = [(0.0, 0.0), (700.0, 0.0), (700.0, 500.0), (0.0, 500.0)];
        let mut walls = Vec::new();
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            walls.push(Wall {
                thickness: 20.0,
                ..Wall::new(
                    doc.new_wall_id(),
                    Point2::new(a.0, a.1),
                    Point2::new(b.0, b.1),
                )
            });
        }
        let room = Room::new(
            doc.new_room_id(),
            "Sala",
            pts.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        );
        let mut commands: Vec<Command> = walls.iter().cloned().map(Command::insert).collect();
        commands.push(Command::insert(room));
        let mut add = |doc: &mut Document,
                       id: &str,
                       at: (f64, f64),
                       angle: f64,
                       wall: Option<(usize, f64)>| {
            let mut piece = newera_catalog::find(id)
                .unwrap()
                .instantiate(doc.new_furniture_id(), Point2::new(at.0, at.1));
            piece.angle = angle;
            if let Some((w, along)) = wall {
                align_to_wall(&mut piece, &walls[w], along);
            }
            commands.push(Command::insert(piece));
        };
        add(&mut doc, "door", (0.0, 0.0), 0.0, Some((2, 150.0)));
        add(&mut doc, "window", (0.0, 0.0), 0.0, Some((0, 350.0)));
        add(&mut doc, "sofa-3", (350.0, 400.0), 180.0, None);
        add(&mut doc, "coffee-table", (350.0, 280.0), 0.0, None);
        add(&mut doc, "armchair", (150.0, 250.0), 90.0, None);
        add(&mut doc, "plant", (620.0, 80.0), 0.0, None);
        add(&mut doc, "bookcase", (60.0, 30.0), 0.0, None);
        doc.execute(Command::Batch { commands }).unwrap();
        doc
    }

    fn render(name: &str, setup: impl FnOnce(&mut NewEraApp)) {
        let document = SharedDocument::new(furnished());
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_eframe(move |cc| NewEraApp::new(cc, document, None));
        harness.run_steps(5);
        setup(harness.state_mut());
        harness.run_steps(12);
        let image = harness.render().expect("render");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join(format!("{name}.png"))).unwrap();
    }

    #[test]
    #[ignore = "visual review; needs a GPU"]
    fn furniture() {
        render("furniture-selected", |app| {
            app.selection.insert(newera_core::FurnitureId(7).into());
        });
        render("furniture-dialog", |app| {
            app.open_modify(&[newera_core::FurnitureId(7).into()]);
        });
        render("catalog-search", |app| app.catalog_query = "cama".into());
        render("catalog-place", |app| {
            app.set_tool(Tool::Place("bed-double"));
        });
    }
}

#[cfg(test)]
mod variant_tests {
    use eframe::egui::{Key, Modifiers};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use newera_core::{Point2, Wall};

    use super::*;

    fn app() -> Harness<'static, NewEraApp> {
        let mut doc = Document::default();
        let wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
        );
        doc.execute(Command::insert(wall)).unwrap();
        let document = SharedDocument::new(doc);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |cc| NewEraApp::new(cc, document, None));
        h.run_steps(5);
        h
    }

    #[test]
    fn duplicate_edit_and_switch_tabs() {
        let mut h = app();
        h.key_press_modifiers(Modifiers::COMMAND, Key::T);
        h.run_steps(4);
        assert_eq!(h.state().document.read().variant_count(), 2);
        assert_eq!(h.state().document.read().active_variant(), 1);

        // Edit the copy.
        h.state()
            .document
            .write()
            .execute(Command::remove(newera_core::WallId(1)))
            .unwrap();
        h.run_steps(3);

        // Click the first tab: the original still has its wall.
        // Exact label: "Versão 1 (cópia)" also contains "Versão 1".
        h.get_by_label(&format!("{} Versão 1", icon::FILE_TEXT))
            .click();
        h.run_steps(4);
        let doc = h.state().document.read();
        assert_eq!(doc.active_variant(), 0);
        assert_eq!(doc.home().walls.len(), 1);
        drop(doc);

        h.key_press_modifiers(Modifiers::COMMAND, Key::Tab);
        h.run_steps(3);
        assert_eq!(h.state().document.read().active_variant(), 1);
        assert_eq!(
            h.state().document.read().home().walls,
            [] as [newera_core::Wall; 0]
        );
    }

    #[test]
    fn add_level_button_creates_a_storey_and_scopes_select_all() {
        let mut h = app();
        h.get_by_label(&format!("{} Andar", icon::PLUS)).click();
        h.run_steps(4);
        {
            let doc = h.state().document.read();
            assert_eq!(doc.home().levels.len(), 2, "ground + first floor");
            assert_ne!(
                doc.home().current_level(),
                Some(doc.home().base_level().unwrap())
            );
        }
        // The upper storey is empty: select-all picks nothing from below.
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(2);
        assert!(h.state().selection.is_empty(), "{:?}", h.state().selection);

        let ground = h.state().document.read().home().base_level().unwrap();
        h.state().document.write().select_level(Some(ground));
        h.run_steps(3);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(2);
        assert_eq!(
            h.state().selection,
            [ElementId::Wall(newera_core::WallId(1))].into()
        );
    }

    #[test]
    fn polyline_dialog_edits_style_and_app_follows_document_camera() {
        let mut h = app();
        let id = {
            let mut doc = h.state().document.write();
            let line = newera_core::Polyline::new(
                doc.new_polyline_id(),
                vec![Point2::new(0.0, 0.0), Point2::new(200.0, 50.0)],
            );
            let id = line.id;
            doc.execute(Command::insert(line)).unwrap();
            id
        };
        h.run_steps(2);
        h.state_mut().open_modify(&[id.into()]);
        h.run_steps(3);
        if let Some(Dialog::ModifyPolyline(line)) = &mut h.state_mut().dialog {
            line.dash = newera_core::DashStyle::Dash;
            line.thickness = 3.0;
        } else {
            panic!("polyline dialog should be open");
        }
        h.run_steps(2);
        h.get_by_label_contains("OK").click();
        h.run_steps(4);
        let line = h
            .state()
            .document
            .read()
            .home()
            .polyline(id)
            .cloned()
            .unwrap();
        assert_eq!(
            (line.dash, line.thickness),
            (newera_core::DashStyle::Dash, 3.0)
        );

        // An agent activates a stored point of view: the 3D view follows.
        assert!(h.state().scene.visitor.is_none());
        {
            let mut doc = h.state().document.write();
            let mut cameras = doc.home().cameras.clone();
            cameras.observer.x = 123.0;
            cameras.observer_active = true;
            doc.execute(Command::SetCameras { cameras }).unwrap();
        }
        h.run_steps(3);
        let visitor = h.state().scene.visitor.clone().expect("visitor view");
        assert!((visitor.camera.x - 123.0).abs() < 1e-9);
    }

    #[test]
    fn toolbar_buttons_keep_their_size_under_the_pointer() {
        let mut h = app();
        h.run_steps(3);
        // The tools show twice, in the toolbar and in the side panel: the
        // toolbar's is the top one.
        let toolbar = |h: &Harness<'static, NewEraApp>, label: &str| {
            h.get_all_by_label(label)
                .map(|n| n.rect())
                .min_by(|a, b| a.top().total_cmp(&b.top()))
                .unwrap()
        };
        let sizes = |h: &Harness<'static, NewEraApp>| {
            let mut sizes: Vec<egui::Vec2> = TOOLS.iter().map(|t| toolbar(h, t.1).size()).collect();
            sizes.push(h.get_by_label_contains("IA").rect().size());
            sizes
        };
        let at_rest = sizes(&h);
        let tools = &at_rest[..TOOLS.len()];
        assert!(tools.iter().all(|s| *s == tools[0]), "{at_rest:?}");

        // The pointer on the tool in use, then on the AI button: nothing
        // grows or shrinks.
        h.hover_at(toolbar(&h, TOOLS[0].1).center());
        h.run_steps(2);
        assert_eq!(sizes(&h), at_rest);
        h.hover_at(h.get_by_label_contains("IA").rect().center());
        h.run_steps(2);
        assert_eq!(sizes(&h), at_rest);
    }

    #[test]
    fn compare_dialog_lists_every_version() {
        let mut h = app();
        h.key_press_modifiers(Modifiers::COMMAND, Key::T);
        h.run_steps(3);
        h.get_by_label_contains("Comparar").click();
        h.run_steps(4);
        match &h.state().dialog {
            Some(Dialog::Compare(rows)) => {
                assert_eq!(rows.len(), 2);
                assert!(rows[1].active);
                assert_eq!(rows[0].walls, 1);
            }
            other => panic!("compare dialog should be open, got {other:?}"),
        }
    }

    #[test]
    fn closing_a_version_asks_first() {
        let mut h = app();
        h.key_press_modifiers(Modifiers::COMMAND, Key::T);
        h.run_steps(3);
        h.state_mut().set_dialog(Dialog::ConfirmCloseVariant {
            index: 1,
            name: "x".into(),
        });
        h.run_steps(3);
        h.get_by_label_contains("Fechar versão").click();
        h.run_steps(4);
        assert_eq!(h.state().document.read().variant_count(), 1);
    }

    #[test]
    fn the_wall_buttons_switch_the_3d_and_keep_their_size() {
        let mut h = app();
        h.run_steps(3);
        let widths = |h: &Harness<'static, NewEraApp>| {
            crate::view::scene::Walls::ALL
                .iter()
                .map(|w| h.get_by_label(w.label()).rect().width())
                .collect::<Vec<f32>>()
        };
        let at_rest = widths(&h);
        assert!(
            at_rest.iter().all(|w| (w - at_rest[0]).abs() < 0.01),
            "{at_rest:?}"
        );

        h.get_by_label("Recortadas").click();
        h.run_steps(3);
        assert_eq!(h.state().scene.walls, crate::view::scene::Walls::Cutaway);
        // The one in use keeps its size, with the pointer on it or away.
        assert_eq!(widths(&h), at_rest);
        h.hover_at(egui::pos2(640.0, 400.0));
        h.run_steps(2);
        assert_eq!(widths(&h), at_rest);
    }
}
