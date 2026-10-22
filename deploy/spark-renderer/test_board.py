import json
import tempfile
import time
import unittest
from pathlib import Path

import board
from board import Board, Job

GLB = b"glTF\x02\x00\x00\x00scene"
SCENE = {"width": 64, "height": 64, "samples": 8}


class Lifecycle(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.store = Path(self.dir.name)
        self.board = Board(self.store)
        self.board.restore()

    def tearDown(self) -> None:
        self.dir.cleanup()

    def files(self) -> list[str]:
        return sorted(str(p.relative_to(self.store)) for p in self.store.rglob("*") if p.is_file())

    def test_a_job_put_in_has_its_scene_in_a_work_dir_of_its_own(self) -> None:
        job = Job(id="a1")
        self.assertTrue(self.board.add(job, GLB, SCENE))
        self.assertEqual(self.files(), ["work/a1/scene.glb", "work/a1/scene.json"])
        self.assertEqual((self.store / "work/a1/scene.glb").read_bytes(), GLB)
        self.assertEqual(json.loads((self.store / "work/a1/scene.json").read_text()), SCENE)
        self.assertEqual((self.board.queued(), self.board.waiting.get_nowait()), (1, "a1"))

    def test_a_done_job_keeps_its_photo_and_record_and_drops_its_work_dir(self) -> None:
        job = Job(id="a1")
        self.board.add(job, GLB, SCENE)
        self.board.start(job)
        self.assertIs(self.board.running, job)
        self.board.done(job, b"PNG", {"render_s": 1.0})
        self.assertEqual(self.files(), ["a1.json", "a1.png"])
        record = json.loads((self.store / "a1.json").read_text())
        self.assertEqual(
            {k: record[k] for k in ("id", "status", "error", "stats")},
            {"id": "a1", "status": "done", "error": None, "stats": {"render_s": 1.0}},
        )
        self.assertEqual((job.status, self.board.running), ("done", None))

    def test_a_failed_job_keeps_its_record_only(self) -> None:
        job = Job(id="a1")
        self.board.add(job, GLB, SCENE)
        self.board.start(job)
        self.board.fail(job, "blender exited 1: struct.error: short")
        self.assertEqual(self.files(), ["a1.json"])
        self.assertEqual((job.status, job.error), ("failed", "blender exited 1: struct.error: short"))

    def test_the_queue_holds_its_limit(self) -> None:
        for i in range(board.QUEUE_LIMIT):
            self.assertTrue(self.board.add(Job(id=f"q{i}"), GLB, SCENE))
        self.assertFalse(self.board.add(Job(id="over"), GLB, SCENE))
        self.assertFalse((self.store / "work/over").exists())


class Restore(unittest.TestCase):
    def test_a_restart_keeps_finished_jobs_and_drops_the_rest(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            store = Path(raw)
            first = Board(store)
            first.restore()
            done, failed = Job(id="done1"), Job(id="failed1")
            for job in (done, failed):
                first.add(job, GLB, SCENE)
                first.start(job)
            first.done(done, b"PNG", {})
            first.fail(failed, "it broke")
            first.add(Job(id="waiting1"), GLB, SCENE)
            (store / "orphan.png").write_bytes(b"PNG")
            (store / "half.json.tmp").write_bytes(b"{")
            (store / "bad.json").write_text("not json")
            lost = Job(id="lost1")
            first.add(lost, GLB, SCENE)
            first.done(lost, b"PNG", {})
            (store / "lost1.png").unlink()
            old = {"id": "old1", "status": "done", "error": None, "finished_at": time.time() - board.KEEP_S - 1, "stats": {}}
            (store / "old1.json").write_text(json.dumps(old))
            (store / "old1.png").write_bytes(b"PNG")

            again = Board(store)
            again.restore()
            self.assertEqual(sorted(again.jobs), ["done1", "failed1"])
            self.assertEqual(
                sorted(p.name for p in store.rglob("*") if p.is_file()),
                ["done1.json", "done1.png", "failed1.json"],
            )
            self.assertEqual((again.jobs["failed1"].status, again.jobs["failed1"].error), ("failed", "it broke"))
            self.assertIsNone(again.get("waiting1"))

    def test_old_jobs_are_forgotten_with_their_files(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            store = Path(raw)
            kept = Board(store)
            kept.restore()
            job = Job(id="j1")
            kept.add(job, GLB, SCENE)
            kept.done(job, b"PNG", {})
            job.finished_at = time.time() - board.KEEP_S - 1
            kept.forget_old()
            self.assertEqual((kept.get("j1"), list(store.glob("j1.*"))), (None, []))


if __name__ == "__main__":
    unittest.main()
