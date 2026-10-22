import base64
import binascii
import math
from dataclasses import dataclass

MAX_SIDE = 3840
MIN_SIDE = 16
MAX_PIXELS = 3840 * 2160
MAX_SAMPLES = 1024
MAX_LAMPS = 2000
MAX_GLB_BYTES = 256 * 1024 * 1024
GLB_RULE = "the scene as a binary glTF, base64"
MAX_BODY_BYTES = MAX_GLB_BYTES * 4 // 3 + 16 * 1024 * 1024
FIELDS = {
    "glb",
    "camera",
    "sun",
    "sky",
    "lamps",
    "exposure",
    "auto_exposure_key",
    "clear_glass_alpha",
    "white_balance",
    "width",
    "height",
    "samples",
}


class RefusedError(ValueError):
    def __init__(self, field: str, rule: str) -> None:
        super().__init__(f"{field}: {rule}")
        self.field = field


@dataclass
class Ask:
    glb: bytes
    scene: dict[str, object]


def number(value: object, field: str, low: float, high: float, *, low_open: bool = False) -> float:
    rule = f"a number above {low:g} and up to {high:g}" if low_open else f"a number from {low:g} to {high:g}"
    if isinstance(value, bool) or not isinstance(value, int | float) or not math.isfinite(value):
        raise RefusedError(field, rule)
    if value < low or value > high or (low_open and value == low):
        raise RefusedError(field, rule)
    return float(value)


def whole(value: object, field: str, low: int, high: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < low or value > high:
        raise RefusedError(field, f"a whole number from {low} to {high}")
    return value


def vector(value: object, field: str, *, light: bool = False) -> list[float]:
    if not isinstance(value, list) or len(value) != 3:
        raise RefusedError(field, "three numbers")
    low = 0.0 if light else -1e6
    return [number(v, field, low, 1e6) for v in value]


def scene_glb(raw: object) -> bytes:
    if not isinstance(raw, str):
        raise RefusedError("glb", GLB_RULE)
    if len(raw) > MAX_GLB_BYTES * 4 // 3 + 4:
        raise RefusedError("glb", f"at most {MAX_GLB_BYTES // (1024 * 1024)} MB")
    try:
        glb = base64.b64decode(raw, validate=True)
    except (binascii.Error, ValueError) as err:
        raise RefusedError("glb", GLB_RULE) from err
    if glb[:4] != b"glTF":
        raise RefusedError("glb", GLB_RULE)
    return glb


def camera_of(camera: object) -> dict[str, object]:
    if not isinstance(camera, dict):
        raise RefusedError("camera", "an object: eye, target, fov_y_deg")
    eye = vector(camera.get("eye"), "camera.eye")
    target = vector(camera.get("target"), "camera.target")
    if eye == target:
        raise RefusedError("camera.target", "a point other than the eye")
    return {"eye": eye, "target": target, "fov_y_deg": number(camera.get("fov_y_deg"), "camera.fov_y_deg", 1.0, 179.0)}


def sun_of(sun: object) -> dict[str, object] | None:
    if sun is None:
        return None
    if not isinstance(sun, dict):
        raise RefusedError("sun", "an object: direction, irradiance; or null")
    direction = vector(sun.get("direction"), "sun.direction")
    if not any(direction):
        raise RefusedError("sun.direction", "a direction, not zero")
    return {"direction": direction, "irradiance": vector(sun.get("irradiance"), "sun.irradiance", light=True)}


def lamp_of(lamp: object) -> dict[str, object]:
    if not isinstance(lamp, dict):
        raise RefusedError("lamps", "objects: position, intensity, radius, half_angle_deg, panel")
    panel = lamp.get("panel")
    if panel is not None:
        if not isinstance(panel, list) or len(panel) != 2:
            raise RefusedError("lamps.panel", "two half extents, or null")
        panel = [vector(p, "lamps.panel") for p in panel]
    return {
        "position": vector(lamp.get("position"), "lamps.position"),
        "intensity": vector(lamp.get("intensity"), "lamps.intensity", light=True),
        "radius": number(lamp.get("radius", 0.0), "lamps.radius", 0.0, 10.0),
        "half_angle_deg": number(lamp.get("half_angle_deg", 0.0), "lamps.half_angle_deg", 0.0, 180.0),
        "panel": panel,
    }


def parse(body: object) -> Ask:
    if not isinstance(body, dict):
        raise RefusedError("body", "a JSON object")
    unknown = sorted(set(body) - FIELDS)
    if unknown:
        raise RefusedError(unknown[0], "not a field of a render")
    glb = scene_glb(body.get("glb"))
    lamps = body.get("lamps", [])
    if not isinstance(lamps, list) or len(lamps) > MAX_LAMPS:
        raise RefusedError("lamps", f"a list of at most {MAX_LAMPS} lamps")
    width = whole(body.get("width"), "width", MIN_SIDE, MAX_SIDE)
    height = whole(body.get("height"), "height", MIN_SIDE, MAX_SIDE)
    if width * height > MAX_PIXELS:
        raise RefusedError("height", f"width x height at most {MAX_PIXELS} pixels")
    scene = {
        "camera": camera_of(body.get("camera")),
        "sun": sun_of(body.get("sun")),
        "sky": vector(body.get("sky"), "sky", light=True),
        "lamps": [lamp_of(lamp) for lamp in lamps],
        "exposure": number(body.get("exposure", 1.0), "exposure", 0.0, 100.0, low_open=True),
        "auto_exposure_key": number(body.get("auto_exposure_key", 0.12), "auto_exposure_key", 0.0, 1.0, low_open=True),
        "clear_glass_alpha": number(body.get("clear_glass_alpha", 0.5), "clear_glass_alpha", 0.0, 1.0),
        "white_balance": number(body.get("white_balance", 0.0), "white_balance", 0.0, 1.0),
        "width": width,
        "height": height,
        "samples": whole(body.get("samples"), "samples", 1, MAX_SAMPLES),
    }
    return Ask(glb=glb, scene=scene)
