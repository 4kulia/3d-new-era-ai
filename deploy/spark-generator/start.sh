#!/bin/sh
set -eu
cd "$(dirname "$0")"
test -s "$HOME/ia3d/generator.env"
if [ "${BUILD_BASE:-0}" = 1 ] || ! docker image inspect ia3d-hunyuan3d:base >/dev/null 2>&1; then
  DOCKER_BUILDKIT=0 nice -n 19 docker build --cpu-period 100000 --cpu-quota 200000 --cpu-shares 2 -q -t ia3d-hunyuan3d:base base
fi
docker build -q -t ia3d-generator:latest .
docker rm -f ia3d-generator >/dev/null 2>&1 || true
docker run -d --name ia3d-generator \
  --restart unless-stopped \
  --gpus all --cpus=2 --cpu-shares=2 \
  --network host \
  -v "$HOME/ia3d:/ia3d" \
  --env-file "$HOME/ia3d/generator.env" \
  -e GENERATOR_BIND=100.76.21.14 -e GENERATOR_PORT=7980 \
  ia3d-generator:latest
