# Photo renderer on the Spark

Makes `render_photo`'s photos on the hosted service with Blender Cycles on the GPU, for
`newera-cloud` (`crates/newera-cloud/src/renderer.rs`). It runs on the operator's DGX Spark
(`spark-81fc`, aarch64, GB10) as the container `ia3d-renderer`, reachable only on the tailnet
address `100.76.21.14:7981`.

## What it renders

The scene the built-in renderer would draw (`newera_render::photo_scene`): the home as a binary
glTF — walls, floors, ceilings, openings, the ground, every placed model in full with its
textures — and, beside it, the camera, the sun, the sky and the lamps that `photo_home` lights
it with. Positions are the glTF's (meters, Y up); light is in the photo's units of irradiance.
`scene.py` maps them to Cycles:

- materials as Blender's glTF importer reads them (base colour, its texture, alpha; the engine
  exports roughness 0.9 and metallic 0 for every surface, as its mesh carries no other); a
  surface at most `clear_glass_alpha` opaque (window glass) becomes clear glass: a transparent
  BSDF with 8 % gloss, which lets the sun through;
- the sun as a sun light (strength = its irradiance, 0.53°), the sky as a uniform world colour;
- lamps: a panel as a rectangular area light (power = π · radiance · area); a spot as a point
  light (power = 4π · peak intensity) whose emission a node tree shapes into the engine's profile,
  straight down, exp(−ln 2 · (θ / half angle)²) of the peak and nothing past 90° — so its peak,
  its half-intensity angle and its flux are the engine's (a Cycles spot light cannot: its
  smoothstep falloff gives a flux of 2π(1 − cos half) per unit of peak whatever the blend, about
  1/1.44 of the Gaussian's for small angles); anything else as a point light (power = 4π ·
  intensity); each with its radius;
- Cycles on the GPU with up to 12 bounces (6 diffuse), indirect samples clamped at 10;
- the image denoised with OIDN 2.5.1 (colour, albedo, normal), the engine's gray-world white
  balance and auto exposure applied (`white_balance`, `auto_exposure_key`, `exposure`), and
  Blender's Filmic view transform (this Blender has no AgX).

These conversions rest on Cycles' units, measured with `units.py` (a white diffuse plane 1 m
under the light, no bounces, mean pixel; the off-axis angle moves the sampled spot): sun strength
1 → 0.3183 (= 1/π, so strength is irradiance); a 100 W point light → 2.5328 (irradiance 7.957 =
100/4π); a 0.1 m area light of 100 W → 10.097 (radiance = power / (π · area)); world colour 1 →
1.0; the Gaussian spot of 100 W, half angle 30°, on its axis 2.5323 (the point light's peak), at
30° an intensity of 1.2666 (0.500 of the peak) and at 60° 0.1583 (0.0625 = 1/16, the Gaussian's).
Reproduce on the Spark:
`for a in "sun 0" "point 0" "area 0" "sky 0" "gauss 0" "gauss 30" "gauss 60"; do set -- $a; docker run --rm --cpus=2 --entrypoint blender ia3d-renderer:latest -b --factory-startup --python /srv/renderer/units.py -- $1 $2 | grep ^UNIT; done`
(the last column is the intensity: the pixel over cos³ of the angle).

What Cycles does not get: the engine lets a lamp's own shade and reflector (geometry within its
`clearance`) not block it, Cycles does not, so a lamp closed inside an opaque fixture lights less;
no light portals are placed at windows.

Faces that lie on each other render black in Cycles: a ray leaving one finds the other at once.
The catalog's openings have none: a window's posts stand between its sill and head and each pane
of glass ends on the mullions beside it, door jambs stop under the head, a sliding leaf is its
metal stile and then its glass, side by side, and a garage door's panels end on its jambs
(`newera_catalog::models::frame_tests`).

## API

All but `/health` need the header `X-Renderer-Token`; a missing or wrong one is `401`.

- `POST /render` — JSON: `glb` (base64 binary glTF, at most 256 MB), `camera` {`eye`,
  `target` (3 numbers each, not equal), `fov_y_deg` 1–179}, `sun` {`direction`, `irradiance`} or
  null, `sky` (3 numbers ≥ 0), `lamps` (at most 2000: `position`, `intensity`, `radius` 0–10,
  `half_angle_deg` 0–180, `panel` two half extents or null), `exposure` (>0–100, default 1),
  `auto_exposure_key` (>0–1, default 0.12), `clear_glass_alpha` (0–1, default 0.5),
  `white_balance` (0–1, default 0), `width`/`height` (16–3840, at most 3840 × 2160 pixels),
  `samples` (1–1024). `202 {"job", "status": "queued"}`; a field out of bounds or unknown is
  `422 {"error", "field"}`; a body over its length bound `413`; eight renders waiting `503`.
- `GET /jobs/<id>` — `{"status": "queued" | "running"}`, `{"status": "failed", "error"}`, or the
  PNG (`image/png`, its stats in `X-Renderer-Stats`); an unknown job `404`.
- `GET /health` — `{ok, ready, renderer, queued, running}`.

One render runs at a time. Blender runs per job (`blender -b --python-exit-code 1 --python
scene.py`), killed and the job failed past 10 minutes; a job Blender fails says the first Python
exception of its output (`blender_log.py`, e.g. `blender exited 1: struct.error: unpack_from
requires a buffer of at least 12 bytes …` for a GLB cut short); a watchdog restarts the process when its
worker thread is gone or a job outlives that by a minute. Each job's scene is written whole to
`~/ia3d/renderer-jobs/work/<id>/` and removed when it ends; its record and PNG are written whole
(a temporary file renamed into place) to `~/ia3d/renderer-jobs/`, kept 24 hours across restarts;
a record that cannot be read is dropped at start, and so is a job still waiting (the cloud falls
back on the `404`).

## Running it

`deploy.sh` (run from this directory on the Mac) copies the files to `~/ia3d/renderer-src/` on
the Spark and runs `start.sh` there, which builds `ia3d-renderer:latest` from `Dockerfile` at two
CPUs and low priority (154 s the first time) and starts:

```sh
docker run -d --name ia3d-renderer --restart unless-stopped \
  --gpus all --cpus=2 --cpu-shares=2 --network host \
  -v "$HOME/ia3d:/ia3d" -v /usr/local/cuda-13.0:/usr/local/cuda:ro \
  --env-file "$HOME/ia3d/renderer.env" \
  -e RENDERER_BIND=100.76.21.14 -e RENDERER_PORT=7981 ia3d-renderer:latest
```

The image is Ubuntu 24.04 with its Blender `4.0.2+dfsg-1ubuntu8` (arm64), Cycles' CUDA config
patched so the GB10's sm_121 takes the sm_8x kernel settings (`__CUDA_ARCH__ <= 899` →
`<= 1299`), and a micromamba (2.0.5) environment `/opt/py` with Python 3.12, numpy 2.2, OIDN
2.5.1, FastAPI 0.115.12 and uvicorn 0.34.2, in which the server runs under `nice -n 19`. The
host's CUDA 13.0 is mounted for Cycles to compile its kernel on first use; the kernel is cached
under `~/ia3d/renderer-home/.cache/cycles/kernels` (the container's `HOME`), so only the first
render after a change of Blender compiles it — 176 s on the spike, longer than the cloud waits,
so that one photo comes from the built-in renderer. Host networking with the server bound to the
tailnet address alone: until `tailscale0` has its address the bind fails, the process exits and
Docker restarts it. `~/ia3d/renderer.env` (0600) holds `RENDERER_TOKEN`; the same value is
`NEWERA_RENDERER_TOKEN` in `/opt/interioragent/app.env` on the server, beside
`NEWERA_RENDERER_URL=http://100.76.21.14:7981`.

`test_ask.py`, `test_board.py` and `test_blender_log.py` need only the standard library:
`python3 -B -m unittest test_ask test_board test_blender_log` from this directory.

## Measured

On 2026-10-22, the Spark's CPU busy with the simulations (load average 13–17 on 20 cores), the
container at `--cpus=2`, through the API with the cloud's own client
(`newera_cloud::renderer::render`): the room of the spike — 4 × 5 m, 2.6 m high, a 150 × 140 cm
window, SÖDERHAMN and BILLY as generated (variant M, `~/ia3d/out3/{soderhamn,billy}_M.glb` on
the Spark), a stored point of view in a corner — at 1600 × 900, 15:00:

| photo | samples | call | Cycles | denoise | scene sent |
|---|---|---|---|---|---|
| draft | 32 | 8.0 s | 3.7 s | 2.2 s | 12.8 MB glTF, 17.1 MB request |
| good | 128 | 17.2 s | 11.4 s | 2.2 s | same |
| best | 256 | 25.8 s | 21.4 s | 2.2 s | same |
| good, 21:00, four lamps | 128 | 15.0 s | 10.6 s | 2.2 s | 12.9 MB glTF |

The call counts the upload over an SSH tunnel from the Mac, the queue and the polling; Cycles and
denoise are the job's own stats. Blender's GPU memory peaked at 1167 MiB (`nvidia-smi
--query-compute-apps` every second on the host during the runs). The built-in renderer took
91.9 s for the same good photo on the operator's Mac (138.4 s for the night one). Reproduce
(the GLBs copied from the Spark into `<dir>`, a tunnel `ssh -f -N -L 17981:100.76.21.14:7981
kulia@192.168.178.33`):
`cargo run --release -p newera-cloud --example photo_compare -- <dir> <out> http://127.0.0.1:17981 <token> 15 good,draft,best`
and `… <token> 21 night good`.

Samples: the good photo at 512 samples took 41.3 s in Cycles against 11.3 s at 128, and its
denoised image differs from the 128-sample one by 1.7 % (RMS over the mean). Fireflies: the
night photo has 15 outlier pixels (brighter than 4 × the median of their 5 × 5 neighbourhood)
before denoising and 13 after, the same with the indirect clamp at 10 or at 1 (no indirect
sample of it reaches 1: clamped at 0.0001 its mean falls from 0.00159 to 0.00061); the day photo
2 and 0. Reproduce: `… 15 good builtin=no dump` and `… 21 night good builtin=no dump` write the
job directories `<out>/good` and `<out>/good-night`; copied to `~/ia3d/m/` on the Spark,
`docker run --rm --gpus all --cpus=2 --entrypoint blender -e KEEP_PASSES=1 [-e CLAMP_INDIRECT=1] -v ~/ia3d:/ia3d -v /usr/local/cuda-13.0:/usr/local/cuda:ro ia3d-renderer:latest -b --factory-startup --python-exit-code 1 --python /srv/renderer/scene.py -- /ia3d/m/good-night`
then `docker run --rm --entrypoint /opt/py/bin/python -v ~/ia3d:/ia3d ia3d-renderer:latest /srv/renderer/fireflies.py /ia3d/m/good-night`
(for 512 samples, the same with `"samples": 512` in its `scene.json`).

The refusals were checked with `scp measure.sh kulia@192.168.178.33:ia3d/ && ssh
kulia@192.168.178.33 sh ia3d/measure.sh`: no token and a wrong one `401`, a width of 4000, 3840 ×
3840 pixels, 2000 samples and a body that is not a glTF `422` naming the field, a
`Content-Length` past the bound `413`, an unknown job `404`.
