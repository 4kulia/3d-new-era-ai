#!/bin/sh
set -eu
cd "$(dirname "$0")"
test -s "$HOME/ia3d/renderer.env"
mkdir -p "$HOME/ia3d/renderer-jobs" "$HOME/ia3d/renderer-home"
DOCKER_BUILDKIT=0 nice -n 19 docker build --cpu-period 100000 --cpu-quota 200000 --cpu-shares 2 -q -t ia3d-renderer:latest .
docker rm -f ia3d-renderer >/dev/null 2>&1 || true
docker run -d --name ia3d-renderer \
  --restart unless-stopped \
  --gpus all --cpus=2 --cpu-shares=2 \
  --network host \
  -v "$HOME/ia3d:/ia3d" \
  -v /usr/local/cuda-13.0:/usr/local/cuda:ro \
  --env-file "$HOME/ia3d/renderer.env" \
  -e RENDERER_BIND=100.76.21.14 -e RENDERER_PORT=7981 \
  ia3d-renderer:latest
