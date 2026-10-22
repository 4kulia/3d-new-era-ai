import json
import math
import os
import subprocess
import sys
import time
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector

OIDN_PYTHON = "/opt/py/bin/python"
DENOISE = "/srv/renderer/oidn_denoise.py"
PASSES = {"color": "Image", "albedo": "Denoising Albedo", "normal": "Denoising Normal"}
SUN_ANGLE_DEG = 0.53
CLAMP_INDIRECT = 10.0
AUTO_EXPOSURE_FLOOR = 1e-4
AUTO_EXPOSURE_RANGE = (0.3, 4000.0)
WHITE_BALANCE_CAP = 4.0
LUMINANCE = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)


def to_blender(v: list[float]) -> Vector:
    return Vector((v[0], -v[2], v[1]))


def split(rgb: list[float]) -> tuple[tuple[float, float, float], float]:
    peak = max(rgb)
    if peak <= 0:
        return (1.0, 1.0, 1.0), 0.0
    return (rgb[0] / peak, rgb[1] / peak, rgb[2] / peak), peak


def clear_glass(material: bpy.types.Material, tint: tuple[float, float, float]) -> None:
    tree = material.node_tree
    for node in list(tree.nodes):
        if node.type != "OUTPUT_MATERIAL":
            tree.nodes.remove(node)
    out = next(n for n in tree.nodes if n.type == "OUTPUT_MATERIAL")
    see = tree.nodes.new("ShaderNodeBsdfTransparent")
    see.inputs["Color"].default_value = (*tint, 1.0)
    shine = tree.nodes.new("ShaderNodeBsdfGlossy")
    shine.inputs["Roughness"].default_value = 0.02
    mix = tree.nodes.new("ShaderNodeMixShader")
    mix.inputs["Fac"].default_value = 0.08
    tree.links.new(see.outputs[0], mix.inputs[1])
    tree.links.new(shine.outputs[0], mix.inputs[2])
    tree.links.new(mix.outputs[0], out.inputs["Surface"])


def materials(clear_alpha: float) -> int:
    glass = 0
    for material in bpy.data.materials:
        if not material.use_nodes:
            continue
        bsdf = next((n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
        if bsdf is None or bsdf.inputs["Alpha"].is_linked:
            continue
        alpha = bsdf.inputs["Alpha"].default_value
        if alpha <= clear_alpha:
            color = bsdf.inputs["Base Color"].default_value
            clear_glass(material, (0.6 + 0.4 * color[0], 0.6 + 0.4 * color[1], 0.6 + 0.4 * color[2]))
            glass += 1
    return glass


def camera(scene: bpy.types.Scene, ask: dict) -> None:
    data = bpy.data.cameras.new("camera")
    data.sensor_fit = "VERTICAL"
    data.angle_y = math.radians(ask["fov_y_deg"])
    data.clip_start = 0.05
    data.clip_end = 1000.0
    obj = bpy.data.objects.new("camera", data)
    scene.collection.objects.link(obj)
    scene.camera = obj
    eye, target = to_blender(ask["eye"]), to_blender(ask["target"])
    obj.location = eye
    obj.rotation_euler = (target - eye).to_track_quat("-Z", "Y").to_euler()


def light(scene: bpy.types.Scene, name: str, kind: str) -> tuple[bpy.types.Light, bpy.types.Object]:
    data = bpy.data.lights.new(name, kind)
    obj = bpy.data.objects.new(name, data)
    scene.collection.objects.link(obj)
    return data, obj


def sun_and_sky(scene: bpy.types.Scene, ask: dict) -> None:
    world = bpy.data.worlds.new("sky")
    scene.world = world
    world.use_nodes = True
    color, strength = split(ask["sky"])
    background = world.node_tree.nodes["Background"]
    background.inputs["Color"].default_value = (*color, 1.0)
    background.inputs["Strength"].default_value = strength
    sun = ask.get("sun")
    if sun is None:
        return
    data, obj = light(scene, "sun", "SUN")
    data.color, data.energy = split(sun["irradiance"])
    data.angle = math.radians(SUN_ANGLE_DEG)
    toward = to_blender(sun["direction"]).normalized()
    obj.rotation_euler = (-toward).to_track_quat("-Z", "Y").to_euler()


def gaussian_spot(data: bpy.types.Light, half_angle_deg: float) -> None:
    data.use_nodes = True
    tree = data.node_tree
    nodes, links = tree.nodes, tree.links
    emission = next(n for n in nodes if n.type == "EMISSION")
    coords = nodes.new("ShaderNodeTexCoord")
    xyz = nodes.new("ShaderNodeSeparateXYZ")
    links.new(coords.outputs["Normal"], xyz.inputs["Vector"])

    def math_node(operation: str, a: object, b: object = None) -> bpy.types.NodeSocket:
        node = nodes.new("ShaderNodeMath")
        node.operation = operation
        if isinstance(a, float):
            node.inputs[0].default_value = a
        else:
            links.new(a, node.inputs[0])
        if isinstance(b, float):
            node.inputs[1].default_value = b
        elif b is not None:
            links.new(b, node.inputs[1])
        return node.outputs[0]

    down = math_node("MULTIPLY", xyz.outputs["Z"], -1.0)
    angle = math_node("ARCCOSINE", math_node("MINIMUM", math_node("MAXIMUM", down, -1.0), 1.0))
    ratio = math_node("DIVIDE", angle, math.radians(half_angle_deg))
    falloff = math_node("EXPONENT", math_node("MULTIPLY", math_node("POWER", ratio, 2.0), -math.log(2.0)))
    lit = math_node("MULTIPLY", falloff, math_node("GREATER_THAN", down, 0.0))
    links.new(lit, emission.inputs["Strength"])


def lamps(scene: bpy.types.Scene, ask: list[dict]) -> None:
    for i, lamp in enumerate(ask):
        color, peak = split(lamp["intensity"])
        if peak <= 0:
            continue
        where = to_blender(lamp["position"])
        if lamp.get("panel"):
            u, v = to_blender(lamp["panel"][0]), to_blender(lamp["panel"][1])
            data, obj = light(scene, f"lamp{i}", "AREA")
            data.shape = "RECTANGLE"
            data.size, data.size_y = 2 * u.length, 2 * v.length
            data.energy = math.pi * peak * 4 * u.length * v.length
            x, y = u.normalized(), -v.normalized()
            z = x.cross(y)
            obj.matrix_world = Matrix.Translation(where) @ Matrix((x, y, z)).transposed().to_4x4()
        elif lamp["half_angle_deg"] > 0:
            data, obj = light(scene, f"lamp{i}", "POINT")
            data.energy = 4 * math.pi * peak
            data.shadow_soft_size = lamp["radius"]
            gaussian_spot(data, lamp["half_angle_deg"])
            obj.location = where
        else:
            data, obj = light(scene, f"lamp{i}", "POINT")
            data.energy = 4 * math.pi * peak
            data.shadow_soft_size = lamp["radius"]
            obj.location = where
        data.color = color


def cycles(scene: bpy.types.Scene, ask: dict) -> str:
    scene.render.engine = "CYCLES"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    prefs.compute_device_type = "CUDA"
    prefs.refresh_devices()
    gpus = [d for d in prefs.devices if d.type == "CUDA"]
    if not gpus:
        raise RuntimeError("Cycles sees no CUDA device")
    for device in prefs.devices:
        device.use = device.type == "CUDA"
    c = scene.cycles
    c.device = "GPU"
    c.samples = ask["samples"]
    c.use_adaptive_sampling = True
    c.adaptive_threshold = 0.01
    c.use_denoising = False
    c.max_bounces = 12
    c.diffuse_bounces = 6
    c.glossy_bounces = 4
    c.transmission_bounces = 8
    c.transparent_max_bounces = 16
    c.caustics_reflective = False
    c.caustics_refractive = False
    c.sample_clamp_indirect = float(os.environ.get("CLAMP_INDIRECT", CLAMP_INDIRECT))
    scene.view_layers[0].cycles.denoising_store_passes = True
    scene.render.resolution_x, scene.render.resolution_y = ask["width"], ask["height"]
    scene.render.resolution_percentage = 100
    transforms = [i.identifier for i in scene.view_settings.bl_rna.properties["view_transform"].enum_items]
    scene.view_settings.view_transform = "AgX" if "AgX" in transforms else "Filmic"
    scene.view_settings.look = "None"
    return gpus[0].name


def passes_out(scene: bpy.types.Scene, work: Path) -> None:
    scene.use_nodes = True
    tree = scene.node_tree
    layers = tree.nodes.get("Render Layers") or tree.nodes.new("CompositorNodeRLayers")
    out = tree.nodes.new("CompositorNodeOutputFile")
    out.base_path = str(work)
    out.format.file_format = "OPEN_EXR"
    out.format.color_depth = "32"
    out.format.exr_codec = "ZIP"
    out.file_slots.clear()
    for key, socket in PASSES.items():
        out.file_slots.new(f"{key}_")
        tree.links.new(layers.outputs[socket], out.inputs[-1])


def read_pass(work: Path, key: str, w: int, h: int) -> np.ndarray:
    path = next(work.glob(f"{key}_*.exr"))
    image = bpy.data.images.load(str(path))
    pixels = np.empty(w * h * 4, dtype=np.float32)
    image.pixels.foreach_get(pixels)
    bpy.data.images.remove(image)
    path.unlink()
    return pixels.reshape(h, w, 4)[:, :, :3].copy()


def white_balance(linear: np.ndarray, strength: float) -> np.ndarray:
    luminance = linear @ LUMINANCE
    finite = np.isfinite(linear).all(axis=2)
    voting = finite & (luminance <= luminance[finite].mean() * WHITE_BALANCE_CAP)
    if strength <= 0 or not voting.any():
        return linear
    average = linear[voting].mean(axis=0)
    lum = float(average @ LUMINANCE)
    if lum <= 1e-8 or average.min() <= 1e-8:
        return linear
    gains = np.clip((lum / average) ** strength, 0.5, 2.0)
    gains *= lum / max(float((average * gains) @ LUMINANCE), 1e-8)
    return linear * gains.astype(np.float32)


def auto_exposure(linear: np.ndarray, key: float, gain: float) -> float:
    luminance = linear @ LUMINANCE
    mean = float(np.exp(np.log(luminance + AUTO_EXPOSURE_FLOOR).mean()))
    auto = min(max(key / mean, AUTO_EXPOSURE_RANGE[0]), AUTO_EXPOSURE_RANGE[1])
    return math.log2(gain * auto)


def main() -> None:
    work = Path(sys.argv[sys.argv.index("--") + 1])
    ask = json.loads((work / "scene.json").read_text())
    started = time.time()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    bpy.ops.import_scene.gltf(filepath=str(work / "scene.glb"))
    glass = materials(ask["clear_glass_alpha"])
    camera(scene, ask["camera"])
    sun_and_sky(scene, ask)
    lamps(scene, ask["lamps"])
    device = cycles(scene, ask)
    passes_out(scene, work)
    loaded = time.time()
    bpy.ops.render.render(write_still=False)
    rendered = time.time()
    w, h = ask["width"], ask["height"]
    for key in PASSES:
        np.save(work / f"{key}.npy", read_pass(work, key, w, h))
    subprocess.run([OIDN_PYTHON, DENOISE, str(work)], check=True)
    denoised = white_balance(np.load(work / "den.npy"), ask["white_balance"])
    scene.view_settings.exposure = auto_exposure(denoised, ask["auto_exposure_key"], ask["exposure"])
    rgba = np.concatenate([denoised, np.ones((h, w, 1), np.float32)], axis=2)
    image = bpy.data.images.new("photo", w, h, float_buffer=True, alpha=False)
    image.pixels.foreach_set(rgba.ravel())
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = "8"
    image.save_render(str(work / "photo.png"), scene=scene)
    if "KEEP_PASSES" not in os.environ:
        for key in [*PASSES, "den"]:
            (work / f"{key}.npy").unlink(missing_ok=True)
    stats = {
        "device": device,
        "samples": ask["samples"],
        "size": [w, h],
        "glass_materials": glass,
        "lamps": len(ask["lamps"]),
        "load_s": round(loaded - started, 2),
        "render_s": round(rendered - loaded, 2),
        "denoise_s": round(time.time() - rendered, 2),
        "exposure_stops": round(scene.view_settings.exposure, 3),
        "view": scene.view_settings.view_transform,
    }
    (work / "stats.json").write_text(json.dumps(stats))
    print("RENDER_DONE", json.dumps(stats), flush=True)


if __name__ == "__main__":
    main()
