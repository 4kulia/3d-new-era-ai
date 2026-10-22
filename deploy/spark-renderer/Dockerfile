FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends blender=4.0.2+dfsg-1ubuntu8 python3-numpy g++ curl ca-certificates bzip2 \
 && rm -rf /var/lib/apt/lists/* \
 && curl -Ls https://micro.mamba.pm/api/micromamba/linux-aarch64/2.0.5 | tar -xj -C /usr/local bin/micromamba \
 && /usr/local/bin/micromamba create -y -p /opt/py -c conda-forge \
      python=3.12 numpy=2.2 openimagedenoise=2.5.1 fastapi=0.115.12 uvicorn=0.34.2 \
 && /usr/local/bin/micromamba clean -a -y \
 && sed -i "s/#elif __CUDA_ARCH__ <= 899/#elif __CUDA_ARCH__ <= 1299/" /usr/share/blender/scripts/addons/cycles/source/kernel/device/cuda/config.h \
 && grep -q "__CUDA_ARCH__ <= 1299" /usr/share/blender/scripts/addons/cycles/source/kernel/device/cuda/config.h
COPY ask.py blender_log.py board.py fireflies.py oidn_denoise.py scene.py server.py units.py /srv/renderer/
ENV PATH=/usr/local/cuda/bin:$PATH \
    PYTHONPATH=/srv/renderer \
    PYTHONUNBUFFERED=1 \
    HOME=/ia3d/renderer-home
WORKDIR /ia3d
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD /opt/py/bin/python -c "import json, os, urllib.request; h = json.load(urllib.request.urlopen(f\"http://{os.environ['RENDERER_BIND']}:{os.environ.get('RENDERER_PORT', '7981')}/health\", timeout=4)); raise SystemExit(0 if h['ready'] else 1)"
ENTRYPOINT ["nice", "-n", "19", "/opt/py/bin/python", "/srv/renderer/server.py"]
CMD []
