//! Desktop editor for 3D New Era AI.
//!
//! Layout follows classic home design tools: catalog and home contents on the
//! left, the 2D floor plan on top and the native 3D view below it.

mod ai;
#[cfg(target_arch = "wasm32")]
mod ai_web;
mod app;
#[cfg(any(target_arch = "wasm32", test))]
mod autosave;
mod cabinets;
mod dialogs;
mod ergonomics;
mod files;
mod i18n;
mod jobs;
mod panels;
mod photo;
#[cfg(target_arch = "wasm32")]
mod recovery_web;
mod render_job;
#[cfg(target_arch = "wasm32")]
pub mod render_web;
#[cfg(test)]
mod screens;
#[cfg(any(target_arch = "wasm32", test))]
mod server_projects;
#[cfg(any(target_arch = "wasm32", test))]
mod staged_asset;
mod tabs;
mod theme;
mod video;
mod view;

use std::path::PathBuf;

use newera_core::SharedDocument;

/// Options for [`run`].
#[derive(Debug, Clone, Default)]
pub struct AppOptions {
    /// MCP endpoint shown in the status bar, if the server is running.
    pub mcp_url: Option<String>,
    /// Project to open at startup.
    pub open: Option<PathBuf>,
}

/// Opens the editor window and blocks until it is closed.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(document: SharedDocument, options: AppOptions) -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("3D New Era AI")
        .with_inner_size([1440.0, 920.0])
        .with_min_inner_size([960.0, 620.0]);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let native = eframe::NativeOptions {
        viewport,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        "3D New Era AI",
        native,
        Box::new(move |cc| {
            let mut app = app::NewEraApp::new(cc, document, options.mcp_url);
            if let Some(path) = options.open {
                app.open_path(&path);
            }
            Ok(Box::new(app))
        }),
    )
}

/// The default wgpu setup, minus the validation of indirect draws.
///
/// wgpu builds a compute pipeline for that check when the device is created,
/// and the Metal compiler of older Intel Macs (macOS 12) fails on it, which
/// loses the device and closes the editor at launch. Nothing here draws
/// indirectly, so the check guards nothing; `WGPU_VALIDATION_INDIRECT_CALL=1`
/// still turns it back on.
#[cfg(not(target_arch = "wasm32"))]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::{egui_wgpu::WgpuSetup, wgpu::InstanceFlags};
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        let flags = &mut setup.instance_descriptor.flags;
        *flags = flags
            .difference(InstanceFlags::VALIDATION_INDIRECT_CALL)
            .with_env();
    }
    options
}

/// The app mark (`assets/icon.svg`), for the window, dock and taskbar.
#[cfg(not(target_arch = "wasm32"))]
fn app_icon() -> Option<std::sync::Arc<eframe::egui::IconData>> {
    let png = include_bytes!("../assets/icon-256.png");
    let image = image::load_from_memory(png).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(std::sync::Arc::new(eframe::egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }))
}

/// Starts the editor in a browser canvas, optionally with a project
/// (`.newera` JSON or bundle bytes) already open — one of the account's cloud
/// projects when `revision`, the service's revision of it, is given.
///
/// # Errors
/// When the canvas can't get a graphics context.
#[cfg(target_arch = "wasm32")]
pub async fn start_web(
    canvas: web_sys::HtmlCanvasElement,
    project: Option<(String, Vec<u8>)>,
    asked: bool,
    revision: Option<i64>,
) -> Result<(), wasm_bindgen::JsValue> {
    let document = SharedDocument::default();
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                if let Some(body) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.body())
                {
                    let backend = cc.wgpu_render_state.as_ref().map_or_else(
                        || "WebGL".to_owned(),
                        |state| format!("{:?}", state.adapter.get_info().backend),
                    );
                    let _ = body.set_attribute("data-editor-backend", &backend);
                }
                let mut app = app::NewEraApp::new(cc, document, None);
                // The demo home is what a first visit opens on; someone who
                // was already drawing here gets their own work back instead —
                // unless the address asked for a project (a link from their
                // AI, "open in the editor"), which is what they came for.
                // A cloud project comes with its revision: the tab saves it back
                // on that revision while the account holds its room.
                if let Some((name, bytes)) = project.filter(|_| asked || !app.restored)
                    && app.open_bytes(&name, &bytes)
                    && let Some(revision) = revision
                    && let Some(url) = ai_web::cloud_url()
                {
                    let now = app.document.read().revision();
                    ai_web::bind(&app.ai_link, &url, &name, revision, now);
                }
                Ok(Box::new(app))
            }),
        )
        .await
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use eframe::{egui_wgpu::WgpuSetup, wgpu::InstanceFlags};

    #[test]
    fn the_editor_does_not_validate_indirect_draws() {
        let WgpuSetup::CreateNew(setup) = super::wgpu_options().wgpu_setup else {
            panic!("the editor creates its own wgpu instance");
        };
        let flags = setup.instance_descriptor.flags;
        if std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none() {
            assert!(!flags.contains(InstanceFlags::VALIDATION_INDIRECT_CALL));
        }
    }
}
