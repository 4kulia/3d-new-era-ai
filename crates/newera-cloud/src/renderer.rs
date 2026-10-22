//! Photos made by Blender Cycles on a GPU: the service in
//! `deploy/spark-renderer/`, on the operator's Spark, over the tailnet.
//!
//! `render_photo` on the hosted service sends it the scene the built-in
//! renderer would draw ([`newera_render::photo_scene`]) and waits for the
//! picture, within [`Renderer::wait`] and the call's `max_s`. A renderer
//! that cannot be reached, fails, or is not done in time leaves the photo to
//! the built-in renderer, and the agent is told why ([`Made::Missed`]).

use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::config::Renderer;

/// How long one request to the renderer may take: the scene's upload, a
/// few megabytes over the tailnet, or one question after it. A chosen
/// bound; the wait for the photo as a whole is [`Renderer::wait`].
const REQUEST_WAIT: Duration = Duration::from_secs(30);

/// What became of a photo sent to the renderer.
#[derive(Debug, PartialEq)]
pub enum Made {
    /// The PNG.
    Photo(Vec<u8>),
    /// Why there is none: the built-in renderer makes it instead.
    Missed(String),
}

/// The HTTP client photos go through (`crate::config::spark_client`).
#[must_use]
pub fn client() -> reqwest::Client {
    crate::config::spark_client(REQUEST_WAIT)
}

/// Cycles samples per pixel for a photo quality. Measured on the Spark
/// (`deploy/spark-renderer/README.md`): a good photo at 512 samples differs
/// from one at 128 by 1.7 % once denoised and takes 3.6 times as long.
#[must_use]
pub fn samples(quality: newera_render::PhotoQuality) -> u32 {
    match quality {
        newera_render::PhotoQuality::Draft => 32,
        newera_render::PhotoQuality::Good => 128,
        newera_render::PhotoQuality::Best => 256,
    }
}

/// The request the renderer takes for a photo (`POST /render`).
#[must_use]
pub fn request(ask: &newera_mcp::PhotoAsk) -> Value {
    // Six decimals: micrometres and millionths, and no f32 noise.
    fn n(x: f32) -> f64 {
        (f64::from(x) * 1e6).round() / 1e6
    }
    fn v(v: impl Into<[f32; 3]>) -> Value {
        json!(v.into().map(n))
    }
    let scene = &ask.scene;
    let lamps: Vec<Value> = scene
        .light
        .lamps
        .iter()
        .map(|lamp| {
            json!({
                "position": v(lamp.position),
                "intensity": v(lamp.intensity),
                "radius": n(lamp.radius),
                "half_angle_deg": n(lamp.half_angle.to_degrees()),
                "panel": lamp.panel.map(|(u, w)| json!([v(u), v(w)])),
            })
        })
        .collect();
    json!({
        "glb": base64::engine::general_purpose::STANDARD.encode(&scene.glb),
        "camera": {
            "eye": v(scene.view.eye),
            "target": v(scene.view.target),
            "fov_y_deg": n(scene.view.fov_y.to_degrees()),
        },
        "sun": scene.light.sun.map(|(direction, irradiance)| json!({
            "direction": v(direction),
            "irradiance": v(irradiance),
        })),
        "sky": v(scene.light.sky),
        "lamps": lamps,
        "exposure": n(newera_render::PHOTO_EXPOSURE),
        "auto_exposure_key": n(newera_render::photo::AUTO_EXPOSURE_KEY),
        "clear_glass_alpha": n(newera_render::photo::CLEAR_GLASS_ALPHA),
        "white_balance": n(newera_render::photo::WHITE_BALANCE),
        "width": ask.width,
        "height": ask.height,
        "samples": samples(ask.quality),
    })
}

/// Has the renderer make the photo, for a call that started at `started`:
/// waiting for it until [`Renderer::wait`] has passed since then, or the
/// call's `max_s` when that is sooner.
pub async fn render(
    renderer: &Renderer,
    client: &reqwest::Client,
    ask: &newera_mcp::PhotoAsk,
    started: Instant,
) -> Made {
    let bound = ask
        .max_s
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .map_or(renderer.wait, |max| max.min(renderer.wait));
    let until = started + bound;
    let out_of_time = || {
        Made::Missed(format!(
            "it had not finished after {} s",
            (bound.as_secs_f64() * 10.0).round() / 10.0
        ))
    };
    let left = |until: Instant| until.saturating_duration_since(Instant::now());
    let sent = client
        .post(format!("{}/render", renderer.url))
        .header("X-Renderer-Token", &renderer.token)
        .timeout(left(until).min(REQUEST_WAIT))
        .json(&request(ask))
        .send()
        .await;
    let response = match sent {
        Ok(response) => response,
        Err(err) if err.is_timeout() && left(until).is_zero() => return out_of_time(),
        Err(err) => return Made::Missed(format!("it is unreachable: {err}")),
    };
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let Some(job) = body["job"]
        .as_str()
        .filter(|_| status == reqwest::StatusCode::ACCEPTED)
        .map(str::to_owned)
    else {
        let said = body["error"]
            .as_str()
            .map_or_else(|| body.to_string(), str::to_owned);
        return Made::Missed(format!("it answered {status}: {said}"));
    };
    loop {
        let wait = left(until);
        if wait.is_zero() {
            return out_of_time();
        }
        tokio::time::sleep(renderer.poll.min(wait)).await;
        match ask_after(renderer, client, &job, left(until)).await {
            Heard::Photo(png) => return Made::Photo(png),
            Heard::Failed(why) => return Made::Missed(why),
            Heard::Waiting => {}
        }
    }
}

/// What the renderer said of a photo it was asked after.
enum Heard {
    Photo(Vec<u8>),
    Waiting,
    Failed(String),
}

async fn ask_after(
    renderer: &Renderer,
    client: &reqwest::Client,
    job: &str,
    left: Duration,
) -> Heard {
    let asked = client
        .get(format!("{}/jobs/{job}", renderer.url))
        .header("X-Renderer-Token", &renderer.token)
        .timeout(left.clamp(Duration::from_millis(1), REQUEST_WAIT))
        .send()
        .await;
    // A question that does not get through is asked again: the photo is
    // still being made over there.
    let Ok(response) = asked else {
        return Heard::Waiting;
    };
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Heard::Failed("it lost the photo".to_owned());
    }
    let png = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .is_some_and(|t| t.as_bytes().starts_with(b"image/png"));
    if !status.is_success() {
        let said = response.text().await.unwrap_or_default();
        return Heard::Failed(format!("it answered {status}: {said}"));
    }
    if png {
        return match response.bytes().await {
            Ok(bytes) => Heard::Photo(bytes.to_vec()),
            Err(_) => Heard::Waiting,
        };
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    match body["status"].as_str() {
        Some("queued" | "running") => Heard::Waiting,
        Some("failed") => Heard::Failed(format!(
            "it failed: {}",
            body["error"].as_str().unwrap_or("without saying why")
        )),
        _ => Heard::Failed(format!("it answered {body}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn ask(quality: newera_render::PhotoQuality) -> newera_mcp::PhotoAsk {
        newera_mcp::PhotoAsk {
            scene: newera_render::PhotoScene {
                glb: b"glTF-scene".to_vec(),
                view: newera_render::View {
                    eye: Vec3::new(0.3, 1.6, 4.7),
                    target: Vec3::new(2.6, 0.9, 1.2),
                    fov_y: 0.5,
                    ortho: None,
                    near: None,
                },
                light: newera_render::PhotoLight {
                    sun: Some((Vec3::new(-0.6, 0.8, 0.0), Vec3::new(3.2, 3.04, 2.816))),
                    sky: Vec3::new(0.72, 0.93, 1.15),
                    lamps: vec![
                        newera_render::photo::PointLight {
                            position: Vec3::new(2.0, 2.3, 2.5),
                            intensity: Vec3::new(0.1, 0.09, 0.07),
                            radius: 0.03,
                            half_angle: 0.0,
                            panel: None,
                            clearance: 0.1,
                        },
                        newera_render::photo::PointLight {
                            position: Vec3::new(1.0, 2.6, 1.0),
                            intensity: Vec3::new(0.5, 0.5, 0.5),
                            radius: 0.02,
                            half_angle: 0.25,
                            panel: None,
                            clearance: 0.05,
                        },
                        newera_render::photo::PointLight {
                            position: Vec3::new(3.0, 2.59, 1.2),
                            intensity: Vec3::new(0.2, 0.2, 0.2),
                            radius: 0.0,
                            half_angle: 0.0,
                            panel: Some((Vec3::new(0.3, 0.0, 0.0), Vec3::new(0.0, 0.0, -0.15))),
                            clearance: 0.01,
                        },
                    ],
                },
            },
            width: 1600,
            height: 900,
            quality,
            max_s: Some(30.0),
        }
    }

    #[test]
    fn the_request_carries_the_scene_as_the_renderer_reads_it() {
        assert_eq!(
            request(&ask(newera_render::PhotoQuality::Good)),
            json!({
                "glb": base64::engine::general_purpose::STANDARD.encode(b"glTF-scene"),
                "camera": {"eye": [0.3, 1.6, 4.7], "target": [2.6, 0.9, 1.2], "fov_y_deg": 28.647_89},
                "sun": {"direction": [-0.6, 0.8, 0.0], "irradiance": [3.2, 3.04, 2.816]},
                "sky": [0.72, 0.93, 1.15],
                "lamps": [
                    {"position": [2.0, 2.3, 2.5], "intensity": [0.1, 0.09, 0.07], "radius": 0.03,
                     "half_angle_deg": 0.0, "panel": null},
                    {"position": [1.0, 2.6, 1.0], "intensity": [0.5, 0.5, 0.5], "radius": 0.02,
                     "half_angle_deg": 14.323_945, "panel": null},
                    {"position": [3.0, 2.59, 1.2], "intensity": [0.2, 0.2, 0.2], "radius": 0.0,
                     "half_angle_deg": 0.0, "panel": [[0.3, 0.0, 0.0], [0.0, 0.0, -0.15]]},
                ],
                "exposure": 1.1,
                "auto_exposure_key": 0.12,
                "clear_glass_alpha": 0.5,
                "white_balance": 0.5,
                "width": 1600,
                "height": 900,
                "samples": 128,
            })
        );
    }

    #[test]
    fn night_sends_no_sun_and_each_quality_its_samples() {
        let mut night = ask(newera_render::PhotoQuality::Draft);
        night.scene.light.sun = None;
        let sent = request(&night);
        assert_eq!(
            (sent["sun"].clone(), sent["samples"].clone()),
            (Value::Null, json!(32))
        );
        assert_eq!(
            request(&ask(newera_render::PhotoQuality::Best))["samples"],
            256
        );
    }
}
