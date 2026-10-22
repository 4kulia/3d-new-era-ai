import cudnnfix  # noqa: F401, I001

import io
import time
from dataclasses import dataclass, field

import numpy as np
import torch
import trimesh
from hy3dgen.rembg import BackgroundRemover
from hy3dgen.shapegen import DegenerateFaceRemover, FaceReducer, FloaterRemover, Hunyuan3DDiTFlowMatchingPipeline
from hy3dgen.texgen import Hunyuan3DPaintPipeline
from PIL import Image, ImageOps

NAME = "hunyuan3d-2.0-m"
WEIGHTS = "tencent/Hunyuan3D-2"
SHAPE_MODEL = "hunyuan3d-dit-v2-0"
PAINT_MODEL = "hunyuan3d-paint-v2-0-turbo"
STEPS = 50
OCTREE = 380
CHUNKS = 20000
SEED = 12345
FACES = 40000


@dataclass
class Made:
    glb: bytes
    stats: dict[str, object] = field(default_factory=dict)


def extents(meshes: list[trimesh.Trimesh]) -> np.ndarray:
    low = np.min([m.bounds[0] for m in meshes], axis=0)
    high = np.max([m.bounds[1] for m in meshes], axis=0)
    return high - low


def shape_of(mesh: trimesh.Trimesh) -> trimesh.Trimesh:
    bare = trimesh.Trimesh(vertices=mesh.vertices, faces=mesh.faces, process=False)
    bare.merge_vertices()
    return bare


def finished(raw: bytes, size_cm: tuple[float, float, float] | None) -> tuple[bytes, dict[str, object]]:
    scene = trimesh.load(io.BytesIO(raw), file_type="glb", force="scene")
    meshes = [m for m in scene.dump() if isinstance(m, trimesh.Trimesh)]
    if not meshes:
        raise ValueError("the painted model has no mesh")
    for mesh in meshes:
        mesh.visual.material.metallicFactor = 0.0
    natural = extents(meshes)
    if size_cm is not None:
        width, depth, height = size_cm
        wanted = np.array([width, height, depth]) / 100.0
        scale = np.eye(4)
        scale[:3, :3] = np.diag(wanted / natural)
        for mesh in meshes:
            mesh.apply_transform(scale)
    glb = trimesh.Scene(meshes).export(file_type="glb")
    stats: dict[str, object] = {
        "meshes": len(meshes),
        "components": sum(len(shape_of(m).split(only_watertight=False)) for m in meshes),
        "faces": sum(len(m.faces) for m in meshes),
        "natural_m": [round(float(v), 4) for v in natural],
        "extents_m": [round(float(v), 4) for v in extents(meshes)],
    }
    return glb, stats


class Generator:
    def __init__(self) -> None:
        self.shape = Hunyuan3DDiTFlowMatchingPipeline.from_pretrained(WEIGHTS, subfolder=SHAPE_MODEL)
        self.paint = Hunyuan3DPaintPipeline.from_pretrained(WEIGHTS, subfolder=PAINT_MODEL)
        self.cutout = BackgroundRemover()

    def run(self, photo: bytes, size_cm: tuple[float, float, float] | None) -> Made:
        started = time.monotonic()
        image = ImageOps.exif_transpose(Image.open(io.BytesIO(photo))).convert("RGB")
        cutout = self.cutout(image)
        cut = time.monotonic()
        mesh = self.shape(
            image=cutout,
            num_inference_steps=STEPS,
            octree_resolution=OCTREE,
            num_chunks=CHUNKS,
            generator=torch.manual_seed(SEED),
            output_type="trimesh",
        )[0]
        torch.cuda.synchronize()
        shaped = time.monotonic()
        mesh = FloaterRemover()(mesh)
        mesh = DegenerateFaceRemover()(mesh)
        mesh = FaceReducer()(mesh, max_facenum=FACES)
        reduced = time.monotonic()
        painted = self.paint(mesh, image=cutout)
        torch.cuda.synchronize()
        textured = time.monotonic()
        glb, stats = finished(painted.export(file_type="glb"), size_cm)
        torch.cuda.empty_cache()
        stats.update(
            {
                "generator": NAME,
                "cutout_s": round(cut - started, 1),
                "shape_s": round(shaped - cut, 1),
                "reduce_s": round(reduced - shaped, 1),
                "paint_s": round(textured - reduced, 1),
                "total_s": round(time.monotonic() - started, 1),
                "glb_bytes": len(glb),
            }
        )
        return Made(glb=glb, stats=stats)
