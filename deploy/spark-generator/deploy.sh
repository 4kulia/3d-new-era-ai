#!/bin/sh
set -eu
cd "$(dirname "$0")"
host="${SPARK:-kulia@192.168.178.33}"
ssh -o BatchMode=yes "$host" 'mkdir -p ~/ia3d/generator-src/base'
scp -q Dockerfile board.py cudnnfix.py fetch.py pipeline.py server.py weights.py start.sh "$host:ia3d/generator-src/"
scp -q base/Dockerfile base/requirements.txt base/constraints.txt "$host:ia3d/generator-src/base/"
ssh -o BatchMode=yes "$host" "BUILD_BASE=${BUILD_BASE:-0} sh ~/ia3d/generator-src/start.sh"
