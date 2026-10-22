import importlib
import math
import sys

import bpy
import numpy as np

SIDE = 8
POWER = 100.0
SPOT_HALF_DEG = 30.0


def white_plane() -> None:
    bpy.ops.mesh.primitive_plane_add(size=20)
    material = bpy.data.materials.new("white")
    material.use_nodes = True
    bsdf = material.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (1, 1, 1, 1)
    bsdf.inputs["Roughness"].default_value = 1.0
    bsdf.inputs["Specular IOR Level"].default_value = 0.0
    bpy.context.active_object.data.materials.append(material)


def lit_by(scene: bpy.types.Scene, kind: str) -> None:
    world = bpy.data.worlds.new("world")
    scene.world = world
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (1, 1, 1, 1) if kind == "sky" else (0, 0, 0, 1)
    if kind == "sky":
        return
    light = bpy.data.lights.new("light", "POINT" if kind == "gauss" else kind.upper())
    light.energy = 1.0 if kind == "sun" else POWER
    if kind == "sun":
        light.angle = 0.0
    elif kind == "area":
        light.size = 0.1
    else:
        light.shadow_soft_size = 0.0
    if kind == "spot":
        light.spot_size = math.radians(90)
        light.spot_blend = 0.0
    if kind == "gauss":
        sys.path.insert(0, "/srv/renderer")
        importlib.import_module("scene").gaussian_spot(light, SPOT_HALF_DEG)
    obj = bpy.data.objects.new("light", light)
    scene.collection.objects.link(obj)
    obj.location = (0, 0, 1)


def main() -> None:
    asked = sys.argv[sys.argv.index("--") + 1 :]
    kind, off_axis = asked[0], float(asked[1]) if len(asked) > 1 else 0.0
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    white_plane()
    lit_by(scene, kind)
    camera = bpy.data.cameras.new("camera")
    camera.type = "ORTHO"
    camera.ortho_scale = 0.02
    obj = bpy.data.objects.new("camera", camera)
    scene.collection.objects.link(obj)
    scene.camera = obj
    obj.location = (math.tan(math.radians(off_axis)), 0, 0.5)
    scene.render.engine = "CYCLES"
    scene.cycles.use_denoising = False
    scene.cycles.samples = 256
    scene.cycles.max_bounces = 1 if kind == "sky" else 0
    scene.render.resolution_x = scene.render.resolution_y = SIDE
    scene.view_settings.view_transform = "Standard"
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.filepath = f"/tmp/units_{kind}_{off_axis:g}.exr"
    bpy.ops.render.render(write_still=True)
    image = bpy.data.images.load(scene.render.filepath)
    pixels = np.empty(SIDE * SIDE * 4, dtype=np.float32)
    image.pixels.foreach_get(pixels)
    seen = float(pixels.reshape(-1, 4)[:, 0].mean())
    intensity = seen / math.cos(math.radians(off_axis)) ** 3
    print("UNIT", kind, off_axis, seen, "intensity_ratio_basis", intensity)


main()
