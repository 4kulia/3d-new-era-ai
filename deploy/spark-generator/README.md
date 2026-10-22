# 3D generator on the Spark

Makes a furniture product's GLB from its shop photo, for `newera-cloud`'s 3D jobs
(`crates/newera-cloud/src/generation.rs`). It runs on the operator's DGX Spark
(`spark-81fc`, aarch64, GB10) as the container `ia3d-generator`, reachable only on the
tailnet address `100.76.21.14:7980`.

## What it makes

Variant M of the operator's comparison: Hunyuan3D-2.0 (`tencent/Hunyuan3D-2`,
`hunyuan3d-dit-v2-0`) at its defaults — background removed with `rembg`, 50 steps, octree
380, seed 12345, floaters and degenerate faces removed, reduced to 40 000 faces — painted with
`hunyuan3d-paint-v2-0-turbo`, then every material's `metallicFactor` set to 0 (the paint
pipeline's `SimpleMaterial` reads as fully metallic in glTF). With `size_cm` the model is
scaled per axis to the shop's size: width along x, height along y, depth along z, in metres —
the editor's import reads a GLB y-up, width x, depth z, height y
(`crates/newera-catalog/src/import.rs`).

`cudnnfix.py` is imported before anything else: cuDNN's half-precision convolutions give
wrong results on the GB10.

## API

Every route but `/health` needs the header `X-Generator-Token`; a missing or wrong one is
`401 {"error"}`. The service does not start with an empty token.

- `POST /generate {image_url, size_cm?: [w, d, h]}` → `202 {"job", "status": "queued"}`.
  A link that is not https, or a `size_cm` that is not three numbers in (0, 2000], is
  `422 {"error", "field": "image_url" | "size_cm"}`. More than 100 jobs waiting →
  `503 {"error"}`. The cloud always sends `size_cm`.
- `GET /jobs/<id>` → `200 {"status": "queued" | "running"}`,
  `200 {"status": "failed", "error", "permanent"}`, or, when done, `200` with the GLB bytes
  (`Content-Type: model/gltf-binary`) and `X-Generator-Stats` (JSON: meshes, components,
  faces, natural_m, extents_m, timings, glb_bytes). Unknown id → `404 {"error"}`.
  `permanent` is true for a photo that will not come however often it is asked: not https,
  off the public internet, a 4xx answer other than 408 and 429, over 20 MB, slower than 60 s, more than 3
  redirects. The cloud fails such a job at once and tries any other failure again.
- `GET /health` → `{"ok", "ready", "generator": "hunyuan3d-2.0-m", "queued", "running"}`;
  `ready` turns true once the models are loaded (about a minute after start) and false
  if the worker thread is gone.

One job runs at a time, the rest wait in order. The photo is fetched over https only, from an
address that resolves entirely to the public internet — no loopback, private, link-local,
shared (tailnet) or reserved address, nor an IPv6 form carrying one (mapped, IPv4-compatible,
NAT64, 6to4, Teredo) — connected to the address checked, at most 3 redirects each checked
again, at most 20 MB, within 60 s in all: one timer per download shuts the socket at the deadline in whatever
phase it is — connecting, TLS, the status line and headers, a redirect, the body — and such a
photo is a permanent failure. A job that runs over 15 minutes is failed and the
process exits, so Docker restarts it with the GPU freed; so does a worker thread that dies.
Docker's health check reads `/health` every 30 s. Each job's record and model are written
whole (a temporary file renamed into place) to `~/ia3d/generator-jobs/`, kept 24 hours across
restarts; a record that cannot be read is dropped at start. A job still waiting when the
container stops is lost, and the cloud gives it again on the `404`.

## Running it

`deploy.sh` (run from this directory on the Mac) copies the files to `~/ia3d/generator-src/`
on the Spark and runs `start.sh` there. That builds the base image `ia3d-hunyuan3d:base` from
`base/` when it is missing or `BUILD_BASE=1` is set, then `ia3d-generator:latest` on it, and
starts:

```sh
docker run -d --name ia3d-generator --restart unless-stopped \
  --gpus all --cpus=2 --cpu-shares=2 --network host \
  -v "$HOME/ia3d:/ia3d" --env-file "$HOME/ia3d/generator.env" \
  -e GENERATOR_BIND=100.76.21.14 -e GENERATOR_PORT=7980 ia3d-generator:latest
```

The base image (`base/Dockerfile`) is NVIDIA's `nvcr.io/nvidia/vllm:26.01-py3` (torch for the
GB10), the runtime libraries the paint pipeline loads, the Python packages in
`base/requirements.txt` under `base/constraints.txt` (the base's torch, numpy 2.1.0,
transformers 4.57.1), Hunyuan3D-2 at `f8db63096c8282cb27354314d896feba5ba6ff8a` in
`/opt/Hunyuan3D-2`, its `custom_rasterizer` (CUDA, built for sm_121 with the CUDA 12
cuSOLVER/cuSPARSE headers the base lacks) and `mesh_processor` built in place, and rembg's
`u2net` model. It built in 232 s on the Spark at two CPUs and low priority.

The weights (22 GB) live outside the image, in `~/ia3d/hf` (`HF_HOME`, read offline) with
`~/ia3d/models/tencent/Hunyuan3D-2` (`HY3DGEN_MODELS`) linked to their snapshot. On a fresh
host they are fetched once with
`docker run --rm -v "$HOME/ia3d:/ia3d" -e HF_HOME=/ia3d/hf -e HY3DGEN_MODELS=/ia3d/models -e HF_HUB_OFFLINE=0 --entrypoint python ia3d-generator:latest /srv/generator/weights.py`.

The process runs under `nice -n 19` (the image's entrypoint). Host networking with the server
bound to the tailnet address alone: until `tailscale0` has its address the bind fails, the
process exits and Docker restarts it. `~/ia3d/generator.env` (0600) holds `GENERATOR_TOKEN`;
the same value is `NEWERA_GENERATOR_TOKEN` in `/opt/interioragent/app.env` on the server,
beside `NEWERA_GENERATOR_URL=http://100.76.21.14:7980`.

`test_fetch.py` and `test_board.py` need only the standard library: `python3 -B -m unittest test_fetch test_board`
from this directory, on the Mac or in `ia3d-hunyuan3d:base`.

## Measured

Through the API on 2026-10-22, on the image built from `base/`, the Spark's CPU busy with the
simulations (load average 45 on 20 cores), the container at `--cpus=2`:

| product | photo | size_cm | generation | GLB | meshes | components | metallic | extents (m) |
|---|---|---|---|---|---|---|---|---|
| BILLY | `billy-boekenkast-wit__1590272_pe1038911_s5.jpg` | 80, 28, 202 | 99.4 s (cutout 4.1, shape 67.5, reduce 7.6, paint 19.1) | 2.1 MB | 1 | 1 | 0.0 | 0.80, 2.02, 0.28 |
| SÖDERHAMN 3-zitsbank | `soderhamn-3-zitsbank-kelinge-beige__1499662_pe1006784_s5.jpg` | 198, 99, 83 | 109.0 s (cutout 3.9, shape 66.8, reduce 9.3, paint 27.5) | 3.3 MB | 1 | 1 | 0.0 | 1.98, 0.83, 0.99 |

SÖDERHAMN waited behind BILLY: 211 s from its submission to its model. Components are counted on the mesh with its vertices merged by position:
the painted mesh's UV seams split it into dozens of pieces otherwise. Read through the
editor's import (`newera_cloud::models::check_glb`), the files are 80.0 × 28.0 × 202.0 cm and
198.0 × 99.0 × 83.0 cm (w × d × h).

Unscaled, the models came out 0.82 × 1.99 × 0.27 m and 1.99 × 0.85 × 0.95 m (x, y, z): the
generator fits the largest side to about 2 m, so every model needs the per-axis scale.
Reproduce: `scp measure.sh kulia@192.168.178.33:ia3d/ && ssh kulia@192.168.178.33 sh ia3d/measure.sh`
(it posts both photos, polls, saves the GLBs in `~/ia3d/measure/` and reads them back with
trimesh). The same run checks the refusals: no
token `401`, an `http://` photo `422`, a short `size_cm` `422`, an unknown job `404`, photos at
`192.168.178.1`, `localhost` and the tailnet's `100.98.121.122` fail as "not on the public
internet".
