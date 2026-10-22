import unittest
from pathlib import Path

from blender_log import cause

BROKEN_GLB = (Path(__file__).parent / "testdata" / "blender-broken-glb.txt").read_text()


class Cause(unittest.TestCase):
    def test_a_failed_import_names_the_importers_exception(self) -> None:
        self.assertEqual(
            cause(BROKEN_GLB),
            "struct.error: unpack_from requires a buffer of at least 12 bytes for unpacking 8 bytes at offset 4"
            " (actual buffer size is 8)",
        )

    def test_the_tail_alone_still_names_it(self) -> None:
        self.assertEqual(cause(BROKEN_GLB[-1500:]).split(":")[0], "struct.error")

    def test_an_exception_of_the_script_itself_is_named(self) -> None:
        said = (
            "Blender 4.0.2\nTraceback (most recent call last):\n  File '/srv/renderer/scene.py', line 9, in cycles\n"
            "RuntimeError: Cycles sees no CUDA device\nError: script failed, file: '/srv/renderer/scene.py', exiting.\n"
        )
        self.assertEqual(cause(said), "RuntimeError: Cycles sees no CUDA device")

    def test_the_first_exception_is_the_cause(self) -> None:
        said = (
            "KeyError: 'Alpha'\n\nDuring handling of the above exception, another exception occurred:\n\n"
            "AttributeError: 'NoneType' object has no attribute 'inputs'\n"
        )
        self.assertEqual(cause(said), "KeyError: 'Alpha'")

    def test_without_an_exception_the_last_line_is_said(self) -> None:
        self.assertEqual(cause("Blender 4.0.2\nSegmentation fault\n"), "Segmentation fault")
        self.assertEqual(cause(""), "no output")


if __name__ == "__main__":
    unittest.main()
