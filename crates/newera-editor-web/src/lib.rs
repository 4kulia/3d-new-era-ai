//! Browser entry point of the editor: `make web-editor` builds it into
//! `web/editor/`, where `index.html` calls [`start`].

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Sends warnings and errors from the renderer (wgpu, eframe) to the browser
/// console, where they would otherwise be lost.
#[cfg(target_arch = "wasm32")]
struct ConsoleLog;

#[cfg(target_arch = "wasm32")]
impl log::Log for ConsoleLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = JsValue::from_str(&format!("{}: {}", record.target(), record.args()));
        if record.level() == log::Level::Error {
            web_sys::console::error_1(&text);
        } else {
            web_sys::console::warn_1(&text);
        }
    }

    fn flush(&self) {}
}

/// Starts the editor in the canvas with id `canvas_id`, opening `bytes`
/// (a `.newera` project named `name`) when given; `revision` is the cloud
/// service's revision of it, for a project from the account's cloud.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start(
    canvas_id: String,
    name: Option<String>,
    bytes: Option<Vec<u8>>,
    asked: Option<bool>,
    revision: Option<String>,
) -> Result<(), JsValue> {
    use wasm_bindgen::JsCast;
    console_error_panic_hook::set_once();
    if log::set_logger(&ConsoleLog).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(&canvas_id))
        .ok_or_else(|| JsValue::from_str("canvas not found"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    let project = bytes.map(|b| (name.unwrap_or_else(|| "projeto.newera".into()), b));
    let revision = revision.and_then(|r| r.parse().ok());
    newera_app::start_web(canvas, project, asked.unwrap_or(false), revision).await
}

/// Runs a bounded render in an isolated worker, reporting progress by message.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn render_worker(request: &str, assets: &JsValue) -> Result<Vec<u8>, JsValue> {
    newera_app::render_web::render(request, assets).map_err(|e| JsValue::from_str(&e))
}
