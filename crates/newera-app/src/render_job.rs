//! Resource ownership and progress shared by photo and video jobs.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use web_time::Instant;

pub(crate) type ByteOutcome = Arc<Mutex<Option<Result<Vec<u8>, String>>>>;

/// Conservative workload caps, not an estimate of free system memory.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BrowserBudget {
    pub pixels: u64,
    pub scene_bytes: usize,
    pub asset_bytes: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
impl BrowserBudget {
    pub(crate) fn for_device(memory_gb: Option<f64>, cores: Option<f64>) -> Self {
        let valid = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0);
        let (memory_gb, cores) = (valid(memory_gb), valid(cores));
        let (pixels, mib) =
            if memory_gb.is_some_and(|v| v <= 2.0) || cores.is_some_and(|v| v <= 2.0) {
                (640 * 480, 8)
            } else if memory_gb.is_some_and(|v| v >= 8.0) && cores.is_some_and(|v| v >= 8.0) {
                (1280 * 960, 32)
            } else {
                // Missing/rounded capability hints never grant the largest job.
                (960 * 720, 16)
            };
        Self {
            pixels,
            scene_bytes: mib * 1024 * 1024 / 2,
            asset_bytes: mib * 1024 * 1024,
        }
    }

    pub(crate) fn restricted_by(self, other: Self) -> Self {
        Self {
            pixels: self.pixels.min(other.pixels),
            scene_bytes: self.scene_bytes.min(other.scene_bytes),
            asset_bytes: self.asset_bytes.min(other.asset_bytes),
        }
    }
}

/// A render size the device budget cut down: what was asked, what is drawn.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reduced {
    pub requested: (u64, u64),
    pub rendered: (u64, u64),
    pub limit: u64,
}

#[cfg(any(target_arch = "wasm32", test))]
impl Reduced {
    /// The sentence the tool answer carries.
    pub(crate) fn note(&self) -> String {
        format!(
            "Image size reduced: requested {}x{}, rendered {}x{} (this device's limit is {} pixels per image).",
            self.requested.0, self.requested.1, self.rendered.0, self.rendered.1, self.limit
        )
    }
}

/// Adds the reduction note to a tool answer as every note is added
/// ([`newera_mcp::output::add_note`]): a last text item in `content`, and
/// in `structuredContent`'s `note`, beside a note already there. An answer
/// that was not reduced, or has no `content` array, is left alone.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn append_note(result: &mut serde_json::Value, reduced: Option<&Reduced>) {
    if let Some(reduced) = reduced {
        newera_mcp::output::add_note(result, &reduced.note());
    }
}

/// The largest size within `limit` pixels that keeps the aspect ratio of
/// `w`x`h`, never above the request and never below one pixel a side.
#[cfg(any(target_arch = "wasm32", test))]
fn fit_pixels(w: u64, h: u64, limit: u64) -> (u64, u64) {
    if w.saturating_mul(h) <= limit {
        return (w, h);
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let scale = (limit as f64 / (w as f64 * h as f64)).sqrt();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let (mut nw, mut nh) = (
        (((w as f64 * scale).floor()) as u64).clamp(1, w),
        (((h as f64 * scale).floor()) as u64).clamp(1, h),
    );
    // A side held up to one pixel can push the product past the limit.
    if nw.saturating_mul(nh) > limit {
        if nw == 1 {
            nh = limit.clamp(1, h);
        } else {
            nw = (limit / nh).clamp(1, w);
        }
    }
    // Flooring both sides can leave room: grow the side whose ratio fell
    // furthest while the product still fits.
    loop {
        let (gw, gh) = if u128::from(nw) * u128::from(h) <= u128::from(nh) * u128::from(w) {
            (nw + 1, nh)
        } else {
            (nw, nh + 1)
        };
        if gw > w || gh > h || gw.saturating_mul(gh) > limit {
            break;
        }
        (nw, nh) = (gw, gh);
    }
    (nw, nh)
}

/// Brings the size in a render request within the budget, in place: the MCP
/// kind through its `w`/`h` arguments, the other kinds through `size`. A
/// size over the budget is cut down keeping its aspect ratio and reported;
/// a zero side is refused.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn clamp_request(
    args: &mut serde_json::Value,
    budget: BrowserBudget,
) -> Result<Option<Reduced>, String> {
    let mcp = args["kind"] == "mcp";
    let (w, h) = if mcp {
        let defaults = if args["name"] == "render_plan" {
            [640, 480]
        } else {
            [480, 360]
        };
        let params = args
            .get_mut("args")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("invalid render arguments")?;
        // Preserve each tool's defaults, which also fit small devices.
        let w = params
            .entry("w")
            .or_insert(serde_json::json!(defaults[0]))
            .as_u64()
            .ok_or("invalid width")?;
        let h = params
            .entry("h")
            .or_insert(serde_json::json!(defaults[1]))
            .as_u64()
            .ok_or("invalid height")?;
        (w, h)
    } else {
        (
            args["size"][0].as_u64().ok_or("invalid width")?,
            args["size"][1].as_u64().ok_or("invalid height")?,
        )
    };
    if w == 0 || h == 0 {
        return Err("Image width and height must be at least 1 pixel.".into());
    }
    if w.saturating_mul(h) <= budget.pixels {
        return Ok(None);
    }
    let (nw, nh) = fit_pixels(w, h, budget.pixels);
    if mcp {
        args["args"]["w"] = serde_json::json!(nw);
        args["args"]["h"] = serde_json::json!(nh);
    } else {
        args["size"] = serde_json::json!([nw, nh]);
    }
    Ok(Some(Reduced {
        requested: (w, h),
        rendered: (nw, nh),
        limit: budget.pixels,
    }))
}

thread_local! {
    static BUSY: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
}

pub(crate) struct Permit(Arc<AtomicBool>);
impl Permit {
    pub(crate) fn acquire() -> Result<Self, String> {
        BUSY.with(|busy| {
            busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .map(|_| Self(busy.clone()))
                .map_err(|_| "Uma renderização já está em andamento. Aguarde ou cancele.".into())
        })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct Report {
    state: Mutex<(String, u64, u64, Instant)>,
    pub(crate) cancelled: AtomicBool,
}
impl Default for Report {
    fn default() -> Self {
        Self {
            state: Mutex::new(("Preparando cena".into(), 0, 0, Instant::now())),
            cancelled: AtomicBool::new(false),
        }
    }
}
impl newera_core::progress::Watcher for Report {
    fn step(&self, what: &str, done: u64, total: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.0 == what {
                state.1 = state.1.max(done);
                state.2 = total;
            } else {
                *state = (what.into(), done, total, Instant::now());
            }
        }
    }
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}
impl Report {
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn ui(&self, ui: &mut egui::Ui) -> bool {
        let (what, done, total, started) = self.state.lock().expect("render progress").clone();
        let elapsed = started.elapsed().as_secs_f64();
        let text = if done > 0 && total > done && elapsed >= 1.0 {
            format!(
                "{done}/{total} · ~{:.0} s restantes",
                elapsed / done as f64 * (total - done) as f64
            )
        } else if total > 0 && done < total {
            format!("{done}/{total} · estimando tempo…")
        } else {
            what.clone()
        };
        ui.add(
            egui::ProgressBar::new(if total == 0 {
                0.0
            } else {
                (done as f32 / total as f32).min(0.99)
            })
            .animate(true)
            .text(text)
            .desired_width(280.0),
        )
        .on_hover_text(what);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
        ui.button(crate::i18n::tr("Cancelar")).clicked()
    }
}
use eframe::egui;

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;
    fn reduced() -> Reduced {
        Reduced {
            requested: (1920, 1080),
            rendered: (960, 540),
            limit: 691_200,
        }
    }
    fn answer() -> serde_json::Value {
        serde_json::json!({
            "content":[{"type":"image", "data":"AAAA", "mimeType":"image/png"}],
            "structuredContent":{"mimeType":"image/png"}
        })
    }
    #[test]
    fn the_note_is_the_last_text_item_after_the_image_and_in_the_structured_answer() {
        let mut result = answer();
        append_note(&mut result, Some(&reduced()));
        let content = result["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0], answer()["content"][0]);
        assert_eq!(
            content[1],
            serde_json::json!({"type":"text", "text":reduced().note()})
        );
        assert_eq!(
            result["structuredContent"],
            serde_json::json!({"mimeType":"image/png", "note":reduced().note()})
        );
    }
    /// A photo's own note (it came out dark) stays beside the reduction's,
    /// in the fields as in the text.
    #[test]
    fn the_reduction_note_joins_the_answers_own() {
        let mut result = serde_json::json!({
            "content":[{"type":"image", "data":"AAAA", "mimeType":"image/png"}, {"type":"text", "text":"dark"}],
            "structuredContent":{"mimeType":"image/png", "note":"dark"}
        });
        append_note(&mut result, Some(&reduced()));
        assert_eq!(
            result,
            serde_json::json!({
                "content":[
                    {"type":"image", "data":"AAAA", "mimeType":"image/png"},
                    {"type":"text", "text":"dark"},
                    {"type":"text", "text":reduced().note()},
                ],
                "structuredContent":{"mimeType":"image/png", "note":format!("dark\n{}", reduced().note())}
            })
        );
    }
    #[test]
    fn an_answer_that_was_not_reduced_or_has_no_content_is_left_alone() {
        let mut result = answer();
        append_note(&mut result, None);
        assert_eq!(result, answer());
        let mut bare = serde_json::json!({"structuredContent":{"mimeType":"image/png"}});
        append_note(&mut bare, Some(&reduced()));
        assert_eq!(
            bare,
            serde_json::json!({"structuredContent":{"mimeType":"image/png"}})
        );
        let mut no_structure = serde_json::json!({"content":[]});
        append_note(&mut no_structure, Some(&reduced()));
        assert_eq!(no_structure["content"].as_array().unwrap().len(), 1);
        assert!(no_structure.get("structuredContent").is_none());
    }
    fn budget(pixels: u64) -> BrowserBudget {
        BrowserBudget {
            pixels,
            scene_bytes: 1,
            asset_bytes: 1,
        }
    }
    fn mcp(name: &str, w: u64, h: u64) -> serde_json::Value {
        serde_json::json!({"kind":"mcp", "name":name, "args":{"w":w, "h":h}})
    }
    #[test]
    fn an_mcp_size_over_the_budget_is_cut_down_keeping_its_aspect_and_reported() {
        let mut args = mcp("render_3d", 1920, 1080);
        let reduced = clamp_request(&mut args, budget(960 * 720))
            .unwrap()
            .unwrap();
        let (w, h) = (
            args["args"]["w"].as_u64().unwrap(),
            args["args"]["h"].as_u64().unwrap(),
        );
        assert_eq!((w, h), reduced.rendered);
        assert_eq!(reduced.requested, (1920, 1080));
        assert!(w * h <= 960 * 720);
        assert!(
            (w as f64 / h as f64 - 1920.0 / 1080.0).abs() < 0.02,
            "{w}x{h}"
        );
        assert!(w * h > 960 * 720 * 98 / 100, "uses the budget: {w}x{h}");
        assert!(reduced.note().contains("requested 1920x1080"));
        assert!(reduced.note().contains("691200"));
    }
    #[test]
    fn a_ui_size_over_the_budget_is_cut_down_in_size() {
        let mut args = serde_json::json!({"kind":"photo", "size":[3000, 1000]});
        let reduced = clamp_request(&mut args, budget(640 * 480))
            .unwrap()
            .unwrap();
        let size = args["size"].as_array().unwrap();
        let (w, h) = (size[0].as_u64().unwrap(), size[1].as_u64().unwrap());
        assert_eq!((w, h), reduced.rendered);
        assert!(w * h <= 640 * 480 && w <= 3000 && h <= 1000);
        assert!((w as f64 / h as f64 - 3.0).abs() < 0.05, "{w}x{h}");
    }
    #[test]
    fn a_size_within_the_budget_is_untouched_and_never_enlarged() {
        let mut args = mcp("render_plan", 100, 50);
        assert_eq!(clamp_request(&mut args, budget(960 * 720)).unwrap(), None);
        assert_eq!(args, mcp("render_plan", 100, 50));
        let mut exact = mcp("render_plan", 960, 720);
        assert_eq!(clamp_request(&mut exact, budget(960 * 720)).unwrap(), None);
        let mut missing = serde_json::json!({"kind":"mcp", "name":"render_plan", "args":{}});
        assert_eq!(
            clamp_request(&mut missing, budget(1))
                .unwrap()
                .unwrap()
                .rendered,
            (1, 1)
        );
    }
    #[test]
    fn extreme_shapes_never_exceed_the_budget_or_the_request() {
        for (w, h) in [
            (1, 100_000),
            (100_000, 1),
            (7, 9999),
            (1280, 960),
            (4096, 4096),
        ] {
            for limit in [1, 2, 640 * 480, 960 * 720, 1280 * 960] {
                let mut args = mcp("render_3d", w, h);
                clamp_request(&mut args, budget(limit)).unwrap();
                let (nw, nh) = (
                    args["args"]["w"].as_u64().unwrap(),
                    args["args"]["h"].as_u64().unwrap(),
                );
                assert!(
                    nw >= 1 && nh >= 1 && nw <= w && nh <= h,
                    "{w}x{h} -> {nw}x{nh}"
                );
                assert!(nw * nh <= limit.max(1), "{w}x{h}@{limit} -> {nw}x{nh}");
            }
        }
    }
    #[test]
    fn a_zero_side_is_still_refused_for_both_kinds() {
        assert!(clamp_request(&mut mcp("render_3d", 0, 10), budget(100)).is_err());
        assert!(clamp_request(&mut mcp("render_3d", 10, 0), budget(100)).is_err());
        let mut ui = serde_json::json!({"kind":"video", "size":[0, 10]});
        assert!(clamp_request(&mut ui, budget(100)).is_err());
    }
    #[test]
    fn browser_budget_uses_the_limiting_hint_and_cannot_be_expanded_by_a_request() {
        let small = BrowserBudget::for_device(Some(2.0), Some(32.0));
        assert_eq!(small.pixels, 640 * 480);
        assert_eq!(
            BrowserBudget::for_device(Some(32.0), Some(2.0)).pixels,
            small.pixels
        );
        let unknown = BrowserBudget::for_device(None, None);
        assert_eq!(unknown.pixels, 960 * 720);
        assert_eq!(
            BrowserBudget::for_device(Some(f64::NAN), Some(-1.0)).pixels,
            unknown.pixels
        );
        let large = BrowserBudget::for_device(Some(8.0), Some(8.0));
        assert_eq!(large.pixels, 1280 * 960);
        let bounded = small.restricted_by(large);
        assert_eq!(bounded.pixels, small.pixels);
        assert_eq!(bounded.asset_bytes, 8 * 1024 * 1024);
        assert_eq!(bounded.scene_bytes, 4 * 1024 * 1024);
        assert_eq!(large.restricted_by(small).pixels, small.pixels);
    }
    #[test]
    fn only_one_render_runs_and_cancellation_releases_its_budget() {
        let permit = Permit::acquire().unwrap();
        assert!(Permit::acquire().is_err());
        drop(permit);
        assert!(Permit::acquire().is_ok());
    }
}
