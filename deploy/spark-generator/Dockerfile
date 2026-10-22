ARG BASE=ia3d-hunyuan3d:base
FROM ${BASE}
COPY board.py cudnnfix.py fetch.py pipeline.py server.py weights.py /srv/generator/
ENV PYTHONPATH=/srv/generator:/opt/Hunyuan3D-2 \
    HF_HOME=/ia3d/hf \
    HF_HUB_OFFLINE=1 \
    HY3DGEN_MODELS=/ia3d/models \
    PYTHONUNBUFFERED=1
WORKDIR /ia3d
HEALTHCHECK --interval=30s --timeout=5s --start-period=300s --retries=3 \
  CMD python -c "import json, os, urllib.request; h = json.load(urllib.request.urlopen(f\"http://{os.environ['GENERATOR_BIND']}:{os.environ.get('GENERATOR_PORT', '7980')}/health\", timeout=4)); raise SystemExit(0 if h['ready'] else 1)"
ENTRYPOINT ["nice", "-n", "19", "python", "/srv/generator/server.py"]
CMD []
