import hmac
import json
import logging
import os
import queue
import secrets
import threading
import time
import traceback
from pathlib import Path
from typing import Annotated
from urllib.parse import urlsplit

import uvicorn
from fastapi import FastAPI, Header, Request, Response
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field

import fetch
from board import QUEUE_LIMIT, Board, Job
from pipeline import Generator

TOKEN = os.environ.get("GENERATOR_TOKEN", "").strip()
BIND = os.environ["GENERATOR_BIND"]
PORT = int(os.environ.get("GENERATOR_PORT", "7980"))
STORE = Path(os.environ.get("GENERATOR_STORE", "/ia3d/generator-jobs"))
LARGEST_CM = 2000.0
JOB_DEADLINE_S = 15 * 60
WATCH_S = 10
GLB = "model/gltf-binary"
PHOTO_RULE = "image_url is an https link to the product's photo"
SIZE_RULE = f"size_cm is three numbers, width, depth and height, each above 0 and up to {LARGEST_CM:.0f} cm"

log = logging.getLogger("generator")


class Ask(BaseModel):
    image_url: str
    size_cm: Annotated[list[Annotated[float, Field(gt=0, le=LARGEST_CM)]], Field(min_length=3, max_length=3)] | None = None


board = Board(STORE)


def run_one(generator: Generator, job: Job) -> None:
    board.start(job)
    try:
        photo = fetch.download(job.image_url)
        made = generator.run(photo, job.size_cm)
    except fetch.RefusedError as err:
        board.fail(job, str(err), permanent=err.permanent)
    except Exception as err:
        log.error("job %s failed: %s", job.id, traceback.format_exc())
        board.fail(job, f"{type(err).__name__}: {err}")
    else:
        board.done(job, made.glb, made.stats)
        log.info("job %s done: %s", job.id, json.dumps(made.stats))


def work() -> None:
    try:
        generator = Generator()
    except Exception:
        log.exception("the models did not load")
        os._exit(1)
    board.ready = True
    log.info("models loaded")
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
            run_one(generator, job)
        except Exception:
            log.exception("job %s could not be recorded", job.id)
            with board.lock:
                job.status, job.error = "failed", "the generator could not record the job"
                board.running = None
        board.forget_old()


def watch() -> None:
    while True:
        time.sleep(WATCH_S)
        if board.worker is None or not board.worker.is_alive():
            log.error("the worker thread is gone: restarting")
            os._exit(1)
        job = board.running
        if job is not None and job.started_at is not None and time.monotonic() - job.started_at > JOB_DEADLINE_S:
            log.error("job %s ran over %s s: failing it and restarting", job.id, JOB_DEADLINE_S)
            board.fail(job, f"the generator took over {JOB_DEADLINE_S // 60} minutes on it")
            os._exit(1)


app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)


@app.exception_handler(RequestValidationError)
def malformed(_: Request, exc: RequestValidationError) -> JSONResponse:
    where = {str(part) for error in exc.errors() for part in error.get("loc", ())}
    if "size_cm" in where:
        return JSONResponse({"error": SIZE_RULE, "field": "size_cm"}, status_code=422)
    return JSONResponse({"error": PHOTO_RULE, "field": "image_url"}, status_code=422)


def allowed(token: str | None) -> bool:
    return token is not None and hmac.compare_digest(token.encode(), TOKEN.encode())


def denied() -> JSONResponse:
    return JSONResponse({"error": "X-Generator-Token is missing or wrong"}, status_code=401)


@app.get("/health")
def health() -> dict[str, object]:
    running = board.running
    alive = board.worker is not None and board.worker.is_alive()
    return {
        "ok": alive,
        "ready": board.ready and alive,
        "generator": "hunyuan3d-2.0-m",
        "queued": board.queued(),
        "running": running.id if running is not None else None,
    }


@app.post("/generate", status_code=202)
def generate(ask: Ask, x_generator_token: Annotated[str | None, Header()] = None) -> Response:
    if not allowed(x_generator_token):
        return denied()
    parts = urlsplit(ask.image_url)
    if parts.scheme != "https" or not parts.hostname:
        return JSONResponse({"error": PHOTO_RULE, "field": "image_url"}, status_code=422)
    size = (ask.size_cm[0], ask.size_cm[1], ask.size_cm[2]) if ask.size_cm is not None else None
    job = Job(id=secrets.token_hex(12), image_url=ask.image_url, size_cm=size)
    if not board.add(job):
        return JSONResponse({"error": f"{QUEUE_LIMIT} jobs are waiting already"}, status_code=503)
    return JSONResponse({"job": job.id, "status": "queued"}, status_code=202)


@app.get("/jobs/{job_id}")
def job_state(job_id: str, x_generator_token: Annotated[str | None, Header()] = None) -> Response:
    if not allowed(x_generator_token):
        return denied()
    job = board.get(job_id)
    if job is None:
        return JSONResponse({"error": f"no job {job_id}"}, status_code=404)
    if job.status == "done":
        model = STORE / f"{job.id}.glb"
        if not model.exists():
            return JSONResponse({"error": f"no job {job_id}"}, status_code=404)
        return Response(model.read_bytes(), media_type=GLB, headers={"X-Generator-Stats": json.dumps(job.stats)})
    if job.status == "failed":
        return JSONResponse({"status": "failed", "error": job.error, "permanent": job.permanent})
    return JSONResponse({"status": job.status})


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    if not TOKEN:
        raise SystemExit("GENERATOR_TOKEN is empty: every request would be refused or let through")
    board.restore()
    board.worker = threading.Thread(target=work, name="generator", daemon=True)
    board.worker.start()
    threading.Thread(target=watch, name="watchdog", daemon=True).start()
    uvicorn.run(app, host=BIND, port=PORT, log_level="info", access_log=False)


if __name__ == "__main__":
    main()
