import os
import sys
from pathlib import Path

from huggingface_hub import hf_hub_download, snapshot_download

WEIGHTS = "tencent/Hunyuan3D-2"
PATTERNS = [
    "hunyuan3d-dit-v2-0/config.yaml",
    "hunyuan3d-dit-v2-0/model.fp16.safetensors",
    "hunyuan3d-paint-v2-0-turbo/*",
    "hunyuan3d-delight-v2-0/*",
]
UNET = "hunyuan3d-paint-v2-0-turbo/unet/diffusion_pytorch_model.bin"


def main() -> None:
    models = Path(os.environ["HY3DGEN_MODELS"])
    snapshot = Path(snapshot_download(WEIGHTS, allow_patterns=PATTERNS, ignore_patterns=[UNET], max_workers=4))
    hf_hub_download(WEIGHTS, UNET)
    link = models / WEIGHTS
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.is_symlink() or link.exists():
        link.unlink()
    link.symlink_to(snapshot)
    print("weights at", snapshot, "linked from", link, file=sys.stderr)


if __name__ == "__main__":
    main()
