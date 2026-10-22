import json
import logging
import os
import queue
import shutil
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

KEEP_S = 24 * 3600
QUEUE_LIMIT = 8

log = logging.getLogger("renderer")


@dataclass
class Job:
    id: str
    status: str = "queued"
    error: str | None = None
    started_at: float | None = None
    finished_at: float | None = None
    stats: dict[str, object] = field(default_factory=dict)


def write_whole(path: Path, data: bytes) -> None:
    part = path.with_name(path.name + ".tmp")
    with part.open("wb") as out:
        out.write(data)
        out.flush()
        os.fsync(out.fileno())
    part.replace(path)


class Board:
    def __init__(self, store: Path) -> None:
        self.store = store
        self.lock = threading.Lock()
        self.jobs: dict[str, Job] = {}
        self.waiting: queue.Queue[str] = queue.Queue()
        self.running: Job | None = None
        self.worker: threading.Thread | None = None

    def work_dir(self, job_id: str) -> Path:
        return self.store / "work" / job_id

    def photo(self, job_id: str) -> Path:
        return self.store / f"{job_id}.png"

    def queued(self) -> int:
        with self.lock:
            return sum(1 for j in self.jobs.values() if j.status == "queued")

    def add(self, job: Job, glb: bytes, scene: dict[str, object]) -> bool:
        with self.lock:
            if sum(1 for j in self.jobs.values() if j.status == "queued") >= QUEUE_LIMIT:
                return False
            work = self.work_dir(job.id)
            work.mkdir(parents=True)
            write_whole(work / "scene.glb", glb)
            write_whole(work / "scene.json", json.dumps(scene).encode())
            self.jobs[job.id] = job
        self.waiting.put(job.id)
        return True

    def get(self, job_id: str) -> Job | None:
        with self.lock:
            return self.jobs.get(job_id)

    def start(self, job: Job) -> None:
        with self.lock:
            job.status = "running"
            job.started_at = time.monotonic()
            self.running = job

    def restore(self) -> None:
        self.store.mkdir(parents=True, exist_ok=True)
        shutil.rmtree(self.store / "work", ignore_errors=True)
        for part in self.store.glob("*.tmp"):
            part.unlink(missing_ok=True)
        for photo in self.store.glob("*.png"):
            if not photo.with_suffix(".json").exists():
                photo.unlink(missing_ok=True)
        now = time.time()
        for meta in self.store.glob("*.json"):
            photo = meta.with_suffix(".png")
            try:
                record = json.loads(meta.read_text())
                job = Job(
                    id=str(record["id"]),
                    status=str(record["status"]),
                    error=record.get("error"),
                    finished_at=float(record["finished_at"]),
                    stats=dict(record.get("stats", {})),
                )
            except (OSError, ValueError, KeyError, TypeError) as err:
                log.warning("dropping the unreadable job record %s: %s", meta.name, err)
                meta.unlink(missing_ok=True)
                photo.unlink(missing_ok=True)
                continue
            if job.finished_at is None or now - job.finished_at > KEEP_S or (job.status == "done" and not photo.exists()):
                meta.unlink(missing_ok=True)
                photo.unlink(missing_ok=True)
                continue
            self.jobs[job.id] = job

    def forget_old(self) -> None:
        now = time.time()
        with self.lock:
            old = [j.id for j in self.jobs.values() if j.finished_at is not None and now - j.finished_at > KEEP_S]
            for job_id in old:
                del self.jobs[job_id]
        for job_id in old:
            (self.store / f"{job_id}.json").unlink(missing_ok=True)
            self.photo(job_id).unlink(missing_ok=True)

    def done(self, job: Job, png: bytes, stats: dict[str, object]) -> None:
        write_whole(self.photo(job.id), png)
        self.record(job, "done", None, stats)

    def fail(self, job: Job, error: str) -> None:
        self.record(job, "failed", error, {})

    def record(self, job: Job, status: str, error: str | None, stats: dict[str, object]) -> None:
        finished_at = time.time()
        entry = {"id": job.id, "status": status, "error": error, "finished_at": finished_at, "stats": stats}
        write_whole(self.store / f"{job.id}.json", json.dumps(entry).encode())
        shutil.rmtree(self.work_dir(job.id), ignore_errors=True)
        with self.lock:
            job.status, job.error, job.finished_at, job.stats = status, error, finished_at, stats
            if self.running is job:
                self.running = None
