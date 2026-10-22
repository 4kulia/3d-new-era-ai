import base64
import copy
import unittest

import ask

GLB = base64.b64encode(b"glTF\x02\x00\x00\x00rest").decode()


def body() -> dict:
    return {
        "glb": GLB,
        "camera": {"eye": [0.3, 1.6, 4.7], "target": [2.6, 0.9, 1.2], "fov_y_deg": 38.5},
        "sun": {"direction": [-0.64, 0.76, 0.09], "irradiance": [3.2, 3.04, 2.816]},
        "sky": [0.72, 0.93, 1.15],
        "lamps": [
            {"position": [2, 2.3, 2.5], "intensity": [0.1, 0.09, 0.07], "radius": 0.03, "half_angle_deg": 0, "panel": None},
            {
                "position": [3, 2.6, 1.2],
                "intensity": [0.2, 0.2, 0.2],
                "radius": 0,
                "half_angle_deg": 0,
                "panel": [[0.3, 0, 0], [0, 0, 0.3]],
            },
        ],
        "exposure": 1.1,
        "auto_exposure_key": 0.12,
        "clear_glass_alpha": 0.5,
        "white_balance": 0.5,
        "width": 1600,
        "height": 900,
        "samples": 128,
    }


def refused(changed: dict) -> tuple[str, str]:
    with unittest.TestCase().assertRaises(ask.RefusedError) as caught:
        ask.parse(changed)
    return caught.exception.field, str(caught.exception)


class Parse(unittest.TestCase):
    def test_a_whole_request_is_read_as_it_was_sent(self) -> None:
        sent = body()
        asked = ask.parse(sent)
        self.assertEqual(asked.glb, b"glTF\x02\x00\x00\x00rest")
        expected = {k: v for k, v in sent.items() if k != "glb"}
        self.assertEqual(asked.scene, expected)

    def test_defaults_fill_what_is_left_out(self) -> None:
        sent = body()
        for key in ("sun", "lamps", "exposure", "auto_exposure_key", "clear_glass_alpha", "white_balance"):
            del sent[key]
        scene = ask.parse(sent).scene
        self.assertEqual(
            {k: scene[k] for k in ("sun", "lamps", "exposure", "auto_exposure_key", "clear_glass_alpha", "white_balance")},
            {
                "sun": None,
                "lamps": [],
                "exposure": 1.0,
                "auto_exposure_key": 0.12,
                "clear_glass_alpha": 0.5,
                "white_balance": 0.0,
            },
        )

    def test_the_limits_hold_at_their_edges(self) -> None:
        for field, ok, bad in (
            ("width", 16, 15),
            ("width", 3840, 3841),
            ("height", 16, 15),
            ("samples", 1, 0),
            ("samples", 1024, 1025),
        ):
            sent = body()
            sent["width"], sent["height"] = 64, 64
            sent[field] = ok
            self.assertEqual(ask.parse(sent).scene[field], ok, field)
            sent[field] = bad
            self.assertEqual(refused(sent)[0], field, (field, bad))

    def test_numbers_take_their_upper_edge(self) -> None:
        sent = body()
        sent["camera"]["fov_y_deg"] = 179
        sent["clear_glass_alpha"] = 1
        sent["exposure"] = 100
        scene = ask.parse(sent).scene
        self.assertEqual((scene["camera"]["fov_y_deg"], scene["clear_glass_alpha"], scene["exposure"]), (179.0, 1.0, 100.0))

    def test_too_many_pixels_are_refused(self) -> None:
        sent = body()
        sent["width"], sent["height"] = 3840, 2160
        self.assertEqual(ask.parse(sent).scene["height"], 2160)
        sent["height"] = 2161
        self.assertEqual(refused(sent), ("height", "height: width x height at most 8294400 pixels"))

    def test_each_malformed_field_is_named(self) -> None:
        cases = [
            ("glb", None, "glb: the scene as a binary glTF, base64"),
            ("glb", "not base64!", "glb: the scene as a binary glTF, base64"),
            ("glb", base64.b64encode(b"PNG...").decode(), "glb: the scene as a binary glTF, base64"),
            ("camera", None, "camera: an object: eye, target, fov_y_deg"),
            ("sun", "noon", "sun: an object: direction, irradiance; or null"),
            ("lamps", {}, "lamps: a list of at most 2000 lamps"),
            ("sky", [1, 1], "sky: three numbers"),
            ("sky", [1, -1, 1], "sky: a number from 0 to 1e+06"),
            ("exposure", 0, "exposure: a number above 0 and up to 100"),
            ("auto_exposure_key", 2, "auto_exposure_key: a number above 0 and up to 1"),
            ("clear_glass_alpha", 1.5, "clear_glass_alpha: a number from 0 to 1"),
            ("white_balance", -0.1, "white_balance: a number from 0 to 1"),
            ("width", 64.5, "width: a whole number from 16 to 3840"),
            ("width", True, "width: a whole number from 16 to 3840"),
            ("samples", "128", "samples: a whole number from 1 to 1024"),
            ("samples", True, "samples: a whole number from 1 to 1024"),
            ("color", 1, "color: not a field of a render"),
        ]
        for field, value, said in cases:
            sent = body()
            sent[field] = value
            self.assertEqual(refused(sent), (said.split(":")[0], said), (field, value))

    def test_camera_and_light_fields_are_checked_inside(self) -> None:
        cases = [
            (("camera", "eye"), [0, 1], "camera.eye", "camera.eye: three numbers"),
            (("camera", "target"), [0.3, 1.6, 4.7], "camera.target", "camera.target: a point other than the eye"),
            (("camera", "fov_y_deg"), 180, "camera.fov_y_deg", "camera.fov_y_deg: a number from 1 to 179"),
            (("camera", "fov_y_deg"), float("nan"), "camera.fov_y_deg", "camera.fov_y_deg: a number from 1 to 179"),
            (("sun", "direction"), [0, 0, 0], "sun.direction", "sun.direction: a direction, not zero"),
            (("sun", "irradiance"), [1, 1, -1], "sun.irradiance", "sun.irradiance: a number from 0 to 1e+06"),
        ]
        for (outer, inner), value, field, said in cases:
            sent = body()
            sent[outer][inner] = value
            self.assertEqual(refused(sent), (field, said), (outer, inner, value))

    def test_lamps_are_checked_one_by_one(self) -> None:
        cases = [
            ("position", [1, 2], "lamps.position"),
            ("intensity", [0, -1, 0], "lamps.intensity"),
            ("radius", 11, "lamps.radius"),
            ("half_angle_deg", 181, "lamps.half_angle_deg"),
            ("panel", [[1, 0, 0]], "lamps.panel"),
        ]
        for key, value, field in cases:
            sent = body()
            sent["lamps"][1][key] = value
            self.assertEqual(refused(sent)[0], field, key)
        sent = body()
        sent["lamps"] = [copy.deepcopy(sent["lamps"][0])] * 2001
        self.assertEqual(refused(sent)[0], "lamps")
        sent["lamps"] = ["lamp"]
        self.assertEqual(refused(sent)[0], "lamps")

    def test_an_empty_sun_is_not_taken_for_none(self) -> None:
        sent = body()
        sent["sun"] = {}
        self.assertEqual(refused(sent), ("sun.direction", "sun.direction: three numbers"))
        sent["sun"] = ""
        self.assertEqual(refused(sent), ("sun", "sun: an object: direction, irradiance; or null"))

    def test_a_body_that_is_not_an_object_is_refused(self) -> None:
        self.assertEqual(refused([1, 2]), ("body", "body: a JSON object"))


if __name__ == "__main__":
    unittest.main()
