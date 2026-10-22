#!/bin/sh
set -eu
cd "$(dirname "$0")"
host="${SPARK:-kulia@192.168.178.33}"
ssh -o BatchMode=yes "$host" 'mkdir -p ~/ia3d/renderer-src'
scp -q Dockerfile ask.py blender_log.py board.py fireflies.py oidn_denoise.py scene.py server.py start.sh units.py "$host:ia3d/renderer-src/"
ssh -o BatchMode=yes "$host" "sh ~/ia3d/renderer-src/start.sh"
