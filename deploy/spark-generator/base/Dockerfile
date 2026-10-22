FROM nvcr.io/nvidia/vllm:26.01-py3

ARG HUNYUAN_REV=f8db63096c8282cb27354314d896feba5ba6ff8a

RUN apt-get update \
 && apt-get install -y --no-install-recommends fontconfig-config fonts-dejavu-core libegl1 libfontconfig1 libfreetype6 libgbm1 libgl1 libgl1-mesa-dri libglu1-mesa libgraphite2-3 libharfbuzz0b libosmesa6 libpng16-16t64 libx11-6 libxext6 libxkbcommon0 libxml2 git \
 && rm -rf /var/lib/apt/lists/*

COPY constraints.txt requirements.txt /opt/ia3d/
RUN cat /opt/ia3d/constraints.txt >> /etc/pip/constraint.txt \
 && pip install --no-cache-dir -r /opt/ia3d/requirements.txt

RUN git clone https://github.com/Tencent-Hunyuan/Hunyuan3D-2.git /opt/Hunyuan3D-2 \
 && git -C /opt/Hunyuan3D-2 checkout "$HUNYUAN_REV"

RUN pip download --no-deps -d /tmp/hdr nvidia-cusolver-cu12==11.7.5.82 nvidia-cusparse-cu12==12.5.10.65 \
 && mkdir -p /opt/cuda12-hdr \
 && for wheel in /tmp/hdr/*.whl; do python -m zipfile -e "$wheel" /tmp/hdr/x; done \
 && cp /tmp/hdr/x/nvidia/cusolver/include/cusolver*.h /tmp/hdr/x/nvidia/cusparse/include/cusparse.h /opt/cuda12-hdr/ \
 && rm -rf /tmp/hdr

ENV CPATH=/usr/local/cuda/include:/opt/cuda12-hdr \
    TORCH_CUDA_ARCH_LIST="12.1+PTX" \
    MAX_JOBS=2

RUN cd /opt/Hunyuan3D-2/hy3dgen/texgen/custom_rasterizer && pip install --no-cache-dir --no-build-isolation . \
 && cd /opt/Hunyuan3D-2/hy3dgen/texgen/differentiable_renderer && pip install --no-cache-dir --no-build-isolation .

RUN python -c "from rembg import new_session; new_session()"

WORKDIR /opt/Hunyuan3D-2
ENTRYPOINT ["sleep"]
CMD ["infinity"]
