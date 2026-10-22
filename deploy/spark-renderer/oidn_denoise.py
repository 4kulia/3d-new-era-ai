import ctypes
import sys
import time
from pathlib import Path

import numpy as np

LIBRARY = "/opt/py/lib/libOpenImageDenoise.so"
FLOAT3 = 3
QUALITY_HIGH = 6


def main() -> None:
    work = Path(sys.argv[1])
    lib = ctypes.CDLL(LIBRARY)
    lib.oidnNewDevice.restype = ctypes.c_void_p
    lib.oidnNewFilter.restype = ctypes.c_void_p
    lib.oidnNewFilter.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.oidnCommitDevice.argtypes = [ctypes.c_void_p]
    lib.oidnSetSharedFilterImage.argtypes = [
        ctypes.c_void_p,
        ctypes.c_char_p,
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_size_t,
        ctypes.c_size_t,
        ctypes.c_size_t,
        ctypes.c_size_t,
        ctypes.c_size_t,
    ]
    lib.oidnSetFilterBool.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_bool]
    lib.oidnSetFilterInt.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    lib.oidnCommitFilter.argtypes = [ctypes.c_void_p]
    lib.oidnExecuteFilter.argtypes = [ctypes.c_void_p]
    lib.oidnGetDeviceError.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_char_p)]
    device = lib.oidnNewDevice(0)
    lib.oidnCommitDevice(device)
    images = {key: np.ascontiguousarray(np.load(work / f"{key}.npy"), dtype=np.float32) for key in ("color", "albedo", "normal")}
    out = np.zeros_like(images["color"])
    h, w, _ = out.shape
    flt = lib.oidnNewFilter(device, b"RT")
    for name, arr in (
        (b"color", images["color"]),
        (b"albedo", images["albedo"]),
        (b"normal", images["normal"]),
        (b"output", out),
    ):
        lib.oidnSetSharedFilterImage(flt, name, arr.ctypes.data, FLOAT3, w, h, 0, 0, 0)
    lib.oidnSetFilterBool(flt, b"hdr", True)
    lib.oidnSetFilterInt(flt, b"quality", QUALITY_HIGH)
    lib.oidnCommitFilter(flt)
    started = time.time()
    lib.oidnExecuteFilter(flt)
    error = ctypes.c_char_p()
    code = lib.oidnGetDeviceError(device, ctypes.byref(error))
    print("OIDN", code, error.value, round(time.time() - started, 2), flush=True)
    if code:
        raise SystemExit(1)
    np.save(work / "den.npy", out)


main()
