import hmac
import json
import logging
import os
import queue
import secrets
import subprocess
import threading
import time
import traceback
from pathlib import Path
from typing import Annotated

import uvicorn
from fastapi import FastAPI, Header, Request, Response
from fastapi.responses import JSONResponse

import ask
from blender_log import cause
from board import QUEUE_LIMIT, Board, Job

TOKEN = os.environ.get("RENDERER_TOKEN", "").strip()
BIND = os.environ["RENDERER_BIND"]
PORT = int(os.environ.get("RENDERER_PORT", "7981"))
STORE = Path(os.environ.get("RENDERER_STORE", "/ia3d/renderer-jobs"))
JOB_DEADLINE_S = 10 * 60
WATCH_S = 10
LOG_TAIL = 1500
BLENDER = ["blender", "-b", "--factory-startup", "--python-exit-code", "1", "--python", "/srv/renderer/scene.py", "--"]

log = logging.getLogger("renderer")
board = Board(STORE)


def run_one(job: Job) -> None:
    board.start(job)
    work = board.work_dir(job.id)
    try:
        ran = subprocess.run([*BLENDER, str(work)], capture_output=True, text=True, timeout=JOB_DEADLINE_S, check=False)
        if ran.returncode != 0 or not (work / "photo.png").exists():
            said = ran.stdout + ran.stderr
            log.error("job %s: blender exited %s: %s", job.id, ran.returncode, said[-LOG_TAIL:])
            board.fail(job, f"blender exited {ran.returncode}: {cause(said)}")
            return
        stats = json.loads((work / "stats.json").read_text())
        board.done(job, (work / "photo.png").read_bytes(), stats)
        log.info("job %s done: %s", job.id, json.dumps(stats))
    except subprocess.TimeoutExpired:
        log.error("job %s ran over %s s", job.id, JOB_DEADLINE_S)
        board.fail(job, f"the render took over {JOB_DEADLINE_S // 60} minutes")
    except Exception as err:
        log.error("job %s failed: %s", job.id, traceback.format_exc())
        board.fail(job, f"{type(err).__name__}: {err}")


def work() -> None:
    while True:
        try:
            job_id = board.waiting.get(timeout=600)
        except queue.Empty:
            board.forget_old()
            continue
        job = board.get(job_id)
        if job is None:
            continue
        try:
            run_one(job)
        except Exception:
            log.exception("job %s could not be recorded", job.id)
            with board.lock:
                job.status, job.error = "failed", "the renderer could not record the job"
                board.running = None
        board.forget_old()


def watch() -> None:
    while True:
        time.sleep(WATCH_S)
        if board.worker is None or not board.worker.is_alive():
            log.error("the worker thread is gone: restarting")
            os._exit(1)
        job = board.running
        if job is not None and job.started_at is not None and time.monotonic() - job.started_at > JOB_DEADLINE_S + 60:
            log.error("job %s is stuck past its deadline: failing it and restarting", job.id)
            board.fail(job, f"the render took over {JOB_DEADLINE_S // 60} minutes")
            os._exit(1)


app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)


def allowed(token: str | None) -> bool:
    return token is not None and hmac.compare_digest(token.encode(), TOKEN.encode())


def denied() -> JSONResponse:
    return JSONResponse({"error": "X-Renderer-Token is missing or wrong"}, status_code=401)


@app.get("/health")
def health() -> dict[str, object]:
    running = board.running
    alive = board.worker is not None and board.worker.is_alive()
    return {
        "ok": alive,
        "ready": alive,
        "renderer": "blender-cycles-cuda",
        "queued": board.queued(),
        "running": running.id if running is not None else None,
    }


@app.post("/render", status_code=202)
async def render(request: Request, x_renderer_token: Annotated[str | None, Header()] = None) -> Response:
    if not allowed(x_renderer_token):
        return denied()
    length = request.headers.get("content-length")
    if length is None or not length.isdigit() or int(length) > ask.MAX_BODY_BYTES:
        return JSONResponse(
            {"error": f"the body has a length, at most {ask.MAX_BODY_BYTES} bytes", "field": "body"}, status_code=413
        )
    try:
        body = json.loads(await request.body())
    except ValueError:
        return JSONResponse({"error": "body: a JSON object", "field": "body"}, status_code=422)
    try:
        asked = ask.parse(body)
    except ask.RefusedError as err:
        return JSONResponse({"error": str(err), "field": err.field}, status_code=422)
    job = Job(id=secrets.token_hex(12))
    if not board.add(job, asked.glb, asked.scene):
        return JSONResponse({"error": f"{QUEUE_LIMIT} renders are waiting already"}, status_code=503)
    return JSONResponse({"job": job.id, "status": "queued"}, status_code=202)


@app.get("/jobs/{job_id}")
def job_state(job_id: str, x_renderer_token: Annotated[str | None, Header()] = None) -> Response:
    if not allowed(x_renderer_token):
        return denied()
    job = board.get(job_id)
    if job is None:
        return JSONResponse({"error": f"no job {job_id}"}, status_code=404)
    if job.status == "done":
        photo = board.photo(job.id)
        if not photo.exists():
            return JSONResponse({"error": f"no job {job_id}"}, status_code=404)
        return Response(photo.read_bytes(), media_type="image/png", headers={"X-Renderer-Stats": json.dumps(job.stats)})
    if job.status == "failed":
        return JSONResponse({"status": "failed", "error": job.error})
    return JSONResponse({"status": job.status})


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    if not TOKEN:
        raise SystemExit("RENDERER_TOKEN is empty: every request would be refused or let through")
    board.restore()
    board.worker = threading.Thread(target=work, name="renderer", daemon=True)
    board.worker.start()
    threading.Thread(target=watch, name="watchdog", daemon=True).start()
    uvicorn.run(app, host=BIND, port=PORT, log_level="info", access_log=False)


if __name__ == "__main__":
    main()
