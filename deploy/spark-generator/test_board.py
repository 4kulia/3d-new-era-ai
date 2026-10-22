import json
import tempfile
import time
import unittest
from pathlib import Path

from board import Board, Job


class BoardTest(unittest.TestCase):
    def test_a_finished_job_is_written_whole_and_comes_back_after_a_restart(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = Path(directory)
            board = Board(store)
            job = Job(id="j1", image_url="https://example.com/a.jpg", size_cm=(1.0, 2.0, 3.0))
            board.add(job)
            board.done(job, b"glTF", {"faces": 1})
            self.assertEqual(sorted(p.name for p in store.iterdir()), ["j1.glb", "j1.json"])
            again = Board(store)
            again.restore()
            back = again.get("j1")
            self.assertIsNotNone(back)
            if back is not None:
                self.assertEqual((back.status, back.error, back.stats), ("done", None, {"faces": 1}))

    def test_a_torn_record_is_dropped_and_the_rest_restored(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = Path(directory)
            (store / "torn.json").write_text('{"id": "torn", "sta')
            (store / "torn.glb").write_bytes(b"glTF")
            (store / "partial.json").write_text(json.dumps({"id": "partial"}))
            (store / "kept.json").write_text(
                json.dumps(
                    {
                        "id": "kept",
                        "image_url": "https://example.com/a.jpg",
                        "status": "failed",
                        "error": "no",
                        "permanent": True,
                        "finished_at": time.time(),
                        "stats": {},
                    }
                )
            )
            (store / "left.json.tmp").write_text("{")
            (store / "orphan.glb").write_bytes(b"glTF")
            board = Board(store)
            board.restore()
            self.assertEqual(sorted(board.jobs), ["kept"])
            self.assertEqual(sorted(p.name for p in store.iterdir()), ["kept.json"])


if __name__ == "__main__":
    unittest.main()
