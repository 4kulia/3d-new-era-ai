import sys
from pathlib import Path

import numpy as np

LUMINANCE = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
WINDOW = 5
FACTOR = 4.0
FLOOR = 0.02


def outliers(linear: np.ndarray) -> tuple[int, float]:
    lum = linear @ LUMINANCE
    pad = np.pad(lum, WINDOW // 2, mode="edge")
    h, w = lum.shape
    median = np.median(np.stack([pad[i : i + h, j : j + w] for i in range(WINDOW) for j in range(WINDOW)]), axis=0)
    return int((lum > FACTOR * median + FLOOR).sum()), float(lum.mean())


def main() -> None:
    work = Path(sys.argv[1])
    for key in ("color", "den"):
        count, mean = outliers(np.load(work / f"{key}.npy"))
        print(key, "outliers", count, "mean", round(mean, 5))


main()
