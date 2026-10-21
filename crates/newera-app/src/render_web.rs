//! Browser rendering in one disposable worker, never on the canvas thread.
use crate::render_job::{Permit, Report};
use newera_core::progress::Watcher;
use std::sync::{Arc, Mutex};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

type Outcome = crate::render_job::ByteOutcome;

pub(crate) fn budget() -> crate::render_job::BrowserBudget {
    let navigator = get(&js_sys::global(), "navigator");
    crate::render_job::BrowserBudget::for_device(
        get(&navigator, "deviceMemory").as_f64(),
        get(&navigator, "hardwareConcurrency").as_f64(),
    )
}

/// Inside a worker the size was already fitted where the request entered:
/// anything still over the budget is refused, never cut again.
fn bound_request(
    args: &mut serde_json::Value,
    budget: crate::render_job::BrowserBudget,
) -> Result<(), String> {
    let mut fitted = args.clone();
    match crate::render_job::clamp_request(&mut fitted, budget)? {
        None => {
            *args = fitted;
            Ok(())
        }
        Some(_) => Err(format!(
            "Image size exceeds this device's limit of {} pixels per image.",
            budget.pixels
        )),
    }
}

pub(crate) struct Worker {
    handle: web_sys::Worker,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    _permit: Permit,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.handle.set_onmessage(None);
        self.handle.set_onerror(None);
        self.handle.terminate();
    }
}
fn set(object: &JsValue, key: &str, value: &JsValue) -> Result<(), String> {
    js_sys::Reflect::set(object, &key.into(), value)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}
fn get(object: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(object, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

impl Worker {
    pub(crate) fn start(
        mut request: serde_json::Value,
        paths: Vec<String>,
        report: Arc<Report>,
        result: Outcome,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let permit = Permit::acquire()?;
        let budget = budget();
        bound_request(&mut request, budget)?;
        request["render_budget"] = serde_json::json!(budget);
        let dir = request["assets"].as_str().map(std::path::PathBuf::from);
        let paths = paths
            .into_iter()
            .map(|p| newera_core::resolve_asset(dir.as_deref(), &p));
        let request = request.to_string();
        if request.len() > budget.scene_bytes {
            return Err("Scene exceeds this browser's render limit.".into());
        }
        let assets = js_sys::Array::new();
        let transfers = js_sys::Array::new();
        let files = newera_core::vfs::snapshot_paths(paths, budget.asset_bytes)?;
        newera_core::images::validate_assets(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_ref())),
            (budget.asset_bytes as u64).saturating_mul(8),
        )?;
        for (path, bytes) in files {
            let bytes = js_sys::Uint8Array::from(bytes.as_ref());
            transfers.push(&bytes.buffer());
            assets.push(&js_sys::Array::of2(&path.into(), &bytes));
        }
        let parts =
            js_sys::Array::of1(&include_str!("../../../web/editor/render-worker.js").into());
        let properties = web_sys::BlobPropertyBag::new();
        properties.set_type("text/javascript");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &properties)
            .map_err(|e| format!("{e:?}"))?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
        let options = web_sys::WorkerOptions::new();
        options.set_type(web_sys::WorkerType::Module);
        let worker = web_sys::Worker::new_with_options(&url, &options);
        let _ = web_sys::Url::revoke_object_url(&url);
        let worker =
            worker.map_err(|e| format!("Não foi possível iniciar a renderização: {e:?}"))?;
        let out = result.clone();
        let repaint = ctx.clone();
        let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                let data = event.data();
                match get(&data, "type").as_string().as_deref() {
                    Some("progress") => {
                        let phase = get(&data, "phase").as_string().unwrap_or_default();
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        report.step(
                            &phase,
                            get(&data, "done").as_f64().unwrap_or(0.0) as u64,
                            get(&data, "total").as_f64().unwrap_or(0.0) as u64,
                        );
                    }
                    Some("done") => {
                        *out.lock().expect("render outcome") =
                            Some(Ok(js_sys::Uint8Array::new(&get(&data, "bytes")).to_vec()));
                    }
                    Some("error") => {
                        *out.lock().expect("render outcome") = Some(Err(get(&data, "error")
                            .as_string()
                            .unwrap_or_else(|| "Falha ao renderizar".into())));
                    }
                    _ => {}
                }
                repaint.request_repaint();
            },
        );
        let error =
            Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
                *result.lock().expect("render outcome") = Some(Err(event.message()));
                ctx.request_repaint();
            });
        worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(error.as_ref().unchecked_ref()));
        let handle = Self {
            handle: worker,
            _message: message,
            _error: error,
            _permit: permit,
        };
        let base = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.base_uri().ok().flatten())
            .ok_or("no document URL")?;
        let module = web_sys::Url::new_with_base("./pkg/newera_editor_web.js", &base)
            .map_err(|e| format!("{e:?}"))?;
        let version = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.body())
            .and_then(|b| b.get_attribute("data-render-build"));
        if let Some(version) = version {
            module.search_params().set("v", &version);
        }
        let module = module.href();
        let data = js_sys::Object::new();
        set(&data, "module", &module.into())?;
        set(&data, "request", &request.into())?;
        set(&data, "assets", &assets)?;
        handle
            .handle
            .post_message_with_transfer(&data, &transfers)
            .map_err(|e| format!("{e:?}"))?;
        Ok(handle)
    }
}

struct WorkerProgress;
impl Watcher for WorkerProgress {
    fn step(&self, what: &str, done: u64, total: u64) {
        let message =
            serde_json::json!({"type":"progress", "phase":what, "done":done, "total":total});
        if let Ok(value) = js_sys::JSON::parse(&message.to_string()) {
            let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
            let _ = scope.post_message(&value);
        }
    }
}

/// Worker-only entry: bounded photo/video rendering. Never call from a Window.
/// # Errors
/// Invalid requests, resource budgets or rendering failures.
#[allow(clippy::cast_possible_truncation)]
pub fn render(request: &str, assets: &JsValue) -> Result<Vec<u8>, String> {
    if web_sys::window().is_some() {
        return Err("Renderização exige um worker.".into());
    }
    let mut budget = budget();
    if request.len() > budget.scene_bytes {
        return Err("Scene exceeds this browser's render limit.".into());
    }
    let mut args: serde_json::Value = serde_json::from_str(request).map_err(|e| e.to_string())?;
    if let Some(requested) = args.get("render_budget") {
        budget = budget
            .restricted_by(serde_json::from_value(requested.clone()).map_err(|e| e.to_string())?);
    }
    if request.len() > budget.scene_bytes {
        return Err("Scene exceeds this device's render limit.".into());
    }
    bound_request(&mut args, budget)?;
    let home: newera_core::Home =
        serde_json::from_value(args["home"].take()).map_err(|e| e.to_string())?;
    let files = js_sys::Array::from(assets);
    let mut mounted = Vec::new();
    let mut bytes = 0usize;
    for file in files.iter() {
        let pair = js_sys::Array::from(&file);
        let data = js_sys::Uint8Array::new(&pair.get(1));
        bytes = bytes
            .checked_add(data.length() as usize)
            .ok_or("Assets exceed render budget")?;
        if bytes > budget.asset_bytes {
            return Err("Assets exceed render budget".into());
        }
        mounted.push((
            pair.get(0).as_string().ok_or("invalid asset path")?,
            data.to_vec(),
        ));
    }
    newera_core::images::validate_assets(
        mounted
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        (budget.asset_bytes as u64).saturating_mul(8),
    )?;
    newera_core::vfs::mount(std::path::Path::new(""), mounted);
    let dir = args["assets"].as_str().map(std::path::Path::new);
    if args["kind"] == "mcp" {
        let name = args["name"].as_str().ok_or("tool name")?;
        if !["render_plan", "render_3d", "render_photo"].contains(&name) {
            return Err("Not a render tool".into());
        }
        let params = &args["args"];
        let w = params["w"].as_u64().unwrap_or(800);
        let h = params["h"].as_u64().unwrap_or(600);
        if w == 0 || h == 0 || w.saturating_mul(h) > 1_228_800 {
            return Err("Render excede 1280 × 960 pixels. Reduza w/h.".into());
        }
        let mut doc = newera_core::Document::default();
        doc.load(home);
        doc.set_asset_dir(dir.map(std::path::Path::to_path_buf));
        let watcher: Arc<dyn Watcher> = Arc::new(WorkerProgress);
        return newera_core::progress::watched(&watcher, || {
            newera_core::progress::step("Renderizando imagem", 0, 0);
            let result =
                newera_mcp::call(newera_core::SharedDocument::new(doc), name, params.clone())?;
            serde_json::to_vec(&result).map_err(|e| e.to_string())
        });
    }
    let size: [u32; 2] = serde_json::from_value(args["size"].clone()).map_err(|e| e.to_string())?;
    if size.contains(&0) || u64::from(size[0]) * u64::from(size[1]) > 1_228_800 {
        return Err("No navegador, use no máximo 1280 × 960 pixels.".into());
    }
    let watcher: Arc<dyn Watcher> = Arc::new(WorkerProgress);
    newera_core::progress::watched(&watcher, || match args["kind"].as_str() {
        Some("photo") => {
            let eye: [f32; 3] =
                serde_json::from_value(args["eye"].clone()).map_err(|e| e.to_string())?;
            let target: [f32; 3] =
                serde_json::from_value(args["target"].clone()).map_err(|e| e.to_string())?;
            let view = newera_render::View {
                eye: eye.into(),
                target: target.into(),
                fov_y: args["fov"].as_f64().ok_or("fov")? as f32,
                ortho: args["ortho"].as_f64().map(|n| n as f32),
                near: args["near"].as_f64().map(|n| n as f32),
            };
            let quality = match args["quality"].as_str() {
                Some("Best") => newera_render::PhotoQuality::Best,
                Some("Good") => newera_render::PhotoQuality::Good,
                _ => newera_render::PhotoQuality::Draft,
            };
            Ok(newera_render::photo_home(
                &home,
                &view,
                args["time"].as_i64().ok_or("time")?,
                size[0],
                size[1],
                dir,
                quality,
            )
            .into_raw())
        }
        Some("video") => newera_render::video::render_video_bytes(
            &home,
            &home.environment.camera_path,
            home.environment.video.frame_rate,
            home.environment.video.speed,
            (size[0], size[1]),
            dir,
            |_, _| {},
        )
        .map(|(bytes, _)| bytes),
        _ => Err("Unknown render job".into()),
    })
}
use eframe::egui;

thread_local! {
    static MCP_REPORT: std::cell::RefCell<Option<Arc<Report>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn show(ctx: &egui::Context) {
    MCP_REPORT.with(|slot| {
        if let Some(report) = slot.borrow().as_ref() {
            egui::Window::new("Renderização da IA")
                .collapsible(false)
                .show(ctx, |ui| {
                    if report.ui(ui) {
                        report
                            .cancelled
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                });
        }
    });
}

pub(crate) async fn mcp(
    document: &newera_core::SharedDocument,
    name: &str,
    args: serde_json::Value,
    ctx: &egui::Context,
) -> Result<serde_json::Value, String> {
    let report = Arc::new(Report::default());
    let result = Arc::new(Mutex::new(None));
    let (request, paths) = {
        let doc = document.read();
        (
            serde_json::json!({"kind":"mcp", "home":doc.home(), "assets":doc.asset_dir(), "name":name, "args":args}),
            doc.home().asset_paths(),
        )
    };
    let mut request = request;
    let reduced = crate::render_job::clamp_request(&mut request, budget())?;
    let _worker = Worker::start(request, paths, report.clone(), result.clone(), ctx.clone())?;
    MCP_REPORT.with(|slot| *slot.borrow_mut() = Some(report.clone()));
    ctx.request_repaint();
    let started = web_time::Instant::now();
    let outcome = loop {
        if report.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            break Err("Renderização cancelada".into());
        }
        if let Some(done) = result.lock().ok().and_then(|mut slot| slot.take()) {
            break done.and_then(|bytes| {
                let mut answer: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                crate::render_job::append_note(&mut answer, reduced.as_ref());
                Ok(answer)
            });
        }
        if started.elapsed().as_secs() > 150 {
            break Err(
                "Renderização excedeu 150 segundos. Reduza a qualidade ou resolução.".into(),
            );
        }
        let timer = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(timer).await;
    };
    MCP_REPORT.with(|slot| *slot.borrow_mut() = None);
    ctx.request_repaint();
    outcome
}
