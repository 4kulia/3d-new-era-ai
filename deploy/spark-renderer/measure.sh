#!/bin/sh
set -eu
. "$HOME/ia3d/renderer.env"
api=http://100.76.21.14:7981
glb=$(printf 'glTF\002\000\000\000' | base64)
ask() {
  curl -sS -m 10 -w ' %{http_code}' -H "X-Renderer-Token: $RENDERER_TOKEN" -H 'Content-Type: application/json' -d "$1" "$api/render"
  echo
}
scene() {
  echo "{\"glb\":\"$glb\",\"camera\":{\"eye\":[0,1.6,0],\"target\":[1,1,1],\"fov_y_deg\":50},\"sun\":null,\"sky\":[0.5,0.6,0.7],\"lamps\":[],\"width\":$1,\"height\":$2,\"samples\":$3}"
}
echo "health: $(curl -sS -m 5 "$api/health")"
echo "no token: $(curl -sS -m 10 -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "$(scene 64 64 8)" "$api/render")"
echo "wrong token: $(curl -sS -m 10 -o /dev/null -w '%{http_code}' -H 'X-Renderer-Token: nope' "$api/jobs/x")"
echo "too wide: $(ask "$(scene 4000 64 8)")"
echo "too many pixels: $(ask "$(scene 3840 3840 8)")"
echo "too many samples: $(ask "$(scene 64 64 2000)")"
echo "not a glb: $(ask '{"glb":"aGVsbG8=","camera":{"eye":[0,1,0],"target":[1,1,1],"fov_y_deg":50},"sky":[1,1,1],"width":64,"height":64,"samples":8}')"
echo "body too long: $(curl -sS -m 10 -o /dev/null -w '%{http_code}' -H "X-Renderer-Token: $RENDERER_TOKEN" -H 'Content-Type: application/json' -H 'Content-Length: 999999999999' --data-binary '{}' "$api/render")"
echo "unknown job: $(curl -sS -m 10 -w ' %{http_code}' -H "X-Renderer-Token: $RENDERER_TOKEN" "$api/jobs/nope")"
