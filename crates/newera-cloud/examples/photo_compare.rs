//! The measured room of `deploy/spark-renderer/README.md`, photographed by
//! the built-in renderer and by the GPU renderer, through the same path
//! `render_photo` takes on the hosted service.
//!
//! A 4 × 5 m room with a window, SÖDERHAMN and BILLY as generated (variant
//! M), a stored point of view in a corner; with `night` four lamps too.
//!
//! ```sh
//! cargo run -p newera-cloud --example photo_compare -- \
//!   <dir with soderhamn_M.glb and billy_M.glb> <out dir> <renderer url> <token> [hour] [night] [qualities]
//! ```
//!
//! The renderer's address is its tailnet one, or a tunnel to it. `hour`
//! defaults to 15, `qualities` (comma-separated) to `good`; `builtin=no`
//! among the arguments skips the built-in photo, and `dump` writes each
//! request's scene to `<out>/<quality>/scene.glb` and `scene.json`, the job
//! directory `deploy/spark-renderer/scene.py` reads.

use std::time::{Duration, Instant};

use base64::Engine as _;
use newera_core::{Document, SharedDocument};
use serde_json::{Value, json};

fn call(doc: &SharedDocument, name: &str, args: &Value) -> Value {
    let answer = newera_mcp::call(doc.clone(), name, args.clone())
        .unwrap_or_else(|why| panic!("{name} {args}: {why}"));
    let answer = serde_json::to_value(answer).expect("an answer");
    assert_ne!(answer["isError"], true, "{name}: {answer}");
    answer
}

fn room(assets: &str, night: bool) -> SharedDocument {
    let doc = SharedDocument::new(Document::default());
    doc.write().set_asset_dir(Some(assets.into()));
    call(
        &doc,
        "create",
        &json!({"walls": [{"pts": [[0, 0], [400, 0], [400, 500], [0, 500]], "closed": true, "h": 260}],
                "rooms": [{"name": "Woonkamer", "at": [200, 250]}]}),
    );
    call(
        &doc,
        "place",
        &json!({"items": [{"cat": "window", "at": [110, 0], "w": 150, "h": 140, "elev": 80}]}),
    );
    call(
        &doc,
        "place",
        &json!({"items": [
            {"model": "soderhamn_M.glb", "unit": "m", "at": [260, 55], "w": 198, "d": 99, "h": 83, "stretch": true, "facing": "+y"},
            {"model": "billy_M.glb", "unit": "m", "at": [384, 200], "w": 80, "d": 28, "h": 202, "stretch": true, "facing": "-x"}
        ]}),
    );
    if night {
        call(
            &doc,
            "place",
            &json!({"items": [
                {"cat": "pendant", "at": [200, 250]},
                {"cat": "downlight", "at": [100, 400]},
                {"cat": "downlight", "at": [300, 400]},
                {"cat": "led-panel", "at": [300, 120]}
            ]}),
        );
    }
    call(
        &doc,
        "edit_cameras",
        &json!({"action": "store", "name": "corner", "x": 30, "y": 470, "z": 160, "look_at": [260, 120, 90], "fov": 63}),
    );
    doc
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, assets, out, url, token, rest @ ..] = args.as_slice() else {
        panic!(
            "photo_compare <assets> <out> <renderer url> <token> [hour] [night] [qualities] [builtin=no]"
        );
    };
    let hour: f64 = rest.iter().find_map(|a| a.parse().ok()).unwrap_or(15.0);
    let night = rest.iter().any(|a| a == "night");
    let builtin = !rest.iter().any(|a| a == "builtin=no");
    let dump = rest.iter().any(|a| a == "dump");
    let qualities = rest
        .iter()
        .find(|a| a.contains("good") || a.contains("draft") || a.contains("best"))
        .map_or("good", String::as_str);
    let tag = if night { "-night" } else { "" };
    let doc = room(assets, night);
    let photo = json!({"cam": 0, "w": 1600, "h": 900, "quality": "good", "hour": hour});
    if builtin {
        let started = Instant::now();
        let made = call(&doc, "render_photo", &photo);
        let png = base64::engine::general_purpose::STANDARD
            .decode(made["content"][0]["data"].as_str().expect("a PNG"))
            .expect("base64");
        std::fs::write(format!("{out}/builtin-good-1600x900{tag}.png"), png).expect("written");
        println!(
            "builtin good 1600x900: {:.1} s",
            started.elapsed().as_secs_f64()
        );
    }
    let renderer = newera_cloud::config::Renderer {
        url: url.clone(),
        token: token.clone(),
        wait: Duration::from_secs(600),
        poll: newera_cloud::config::RENDERER_POLL,
    };
    let client = newera_cloud::renderer::client();
    for quality in qualities.split(',') {
        let mut asked = photo.clone();
        asked["quality"] = json!(quality);
        let ask = newera_mcp::photo_scene(doc.clone(), asked)
            .expect("a photo")
            .expect("a render");
        let mut sent = newera_cloud::renderer::request(&ask);
        let request = serde_json::to_vec(&sent).expect("JSON");
        if dump {
            let dir = format!("{out}/{quality}{tag}");
            std::fs::create_dir_all(&dir).expect("a directory");
            std::fs::write(format!("{dir}/scene.glb"), &ask.scene.glb).expect("written");
            sent.as_object_mut().expect("an object").remove("glb");
            std::fs::write(format!("{dir}/scene.json"), sent.to_string()).expect("written");
        }
        let started = Instant::now();
        let made = newera_cloud::renderer::render(&renderer, &client, &ask, started).await;
        let seconds = started.elapsed().as_secs_f64();
        match made {
            newera_cloud::renderer::Made::Photo(png) => {
                std::fs::write(format!("{out}/cycles-{quality}-1600x900{tag}.png"), png)
                    .expect("written");
                println!(
                    "cycles {quality} 1600x900: {seconds:.1} s, glb {} B, request {} B, lamps {}",
                    ask.scene.glb.len(),
                    request.len(),
                    ask.scene.light.lamps.len()
                );
            }
            newera_cloud::renderer::Made::Missed(why) => {
                println!("cycles {quality}: {why} after {seconds:.1} s");
            }
        }
    }
}
