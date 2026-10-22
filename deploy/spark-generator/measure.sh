#!/bin/sh
set -eu
. "$HOME/ia3d/generator.env"
api=http://100.76.21.14:7980
out="$HOME/ia3d/measure"
mkdir -p "$out"
ask() {
  curl -sS -m 10 -H "X-Generator-Token: $GENERATOR_TOKEN" -H 'Content-Type: application/json' -d "$1" "$api/generate"
}
echo "no token: $(curl -sS -m 10 -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"image_url":"https://example.com/a.jpg"}' "$api/generate")"
echo "wrong token: $(curl -sS -m 10 -o /dev/null -w '%{http_code}' -H 'X-Generator-Token: nope' "$api/jobs/x")"
echo "http link: $(curl -sS -m 10 -w ' %{http_code}' -H "X-Generator-Token: $GENERATOR_TOKEN" -H 'Content-Type: application/json' -d '{"image_url":"http://www.ikea.com/a.jpg"}' "$api/generate")"
echo "bad size: $(curl -sS -m 10 -o /dev/null -w '%{http_code}' -H "X-Generator-Token: $GENERATOR_TOKEN" -H 'Content-Type: application/json' -d '{"image_url":"https://www.ikea.com/a.jpg","size_cm":[1,2]}' "$api/generate")"
echo "unknown job: $(curl -sS -m 10 -w ' %{http_code}' -H "X-Generator-Token: $GENERATOR_TOKEN" "$api/jobs/nope")"
private=$(ask '{"image_url":"https://192.168.178.1/a.jpg"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["job"])')
loop=$(ask '{"image_url":"https://localhost/a.jpg"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["job"])')
tailnet=$(ask '{"image_url":"https://100.98.121.122/a.jpg"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["job"])')
billy=$(ask '{"image_url":"https://www.ikea.com/nl/nl/images/products/billy-boekenkast-wit__1590272_pe1038911_s5.jpg","size_cm":[80,28,202]}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["job"])')
sofa=$(ask '{"image_url":"https://www.ikea.com/nl/nl/images/products/soderhamn-3-zitsbank-kelinge-beige__1499662_pe1006784_s5.jpg","size_cm":[198,99,83]}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["job"])')
start=$(date +%s)
wait_for() {
  name=$1; id=$2
  while :; do
    code=$(curl -sS -m 30 -o "$out/$name.body" -D "$out/$name.headers" -w '%{http_code}' -H "X-Generator-Token: $GENERATOR_TOKEN" "$api/jobs/$id")
    if grep -qi '^content-type: model/gltf-binary' "$out/$name.headers"; then
      mv "$out/$name.body" "$out/$name.glb"
      echo "$name done after $(( $(date +%s) - start )) s: $(grep -i '^x-generator-stats' "$out/$name.headers" | tr -d '\r')"
      return
    fi
    if grep -q '"failed"' "$out/$name.body"; then
      echo "$name $code $(cat "$out/$name.body")"
      return
    fi
    sleep 5
  done
}
wait_for private "$private"
wait_for loopback "$loop"
wait_for tailnet "$tailnet"
wait_for billy "$billy"
wait_for soderhamn "$sofa"
echo "health: $(curl -sS -m 5 "$api/health")"
docker run --rm --entrypoint python -v "$out:/m" ia3d-hunyuan3d:base -c '
import json, struct, trimesh
for name in ["billy", "soderhamn"]:
    raw = open(f"/m/{name}.glb", "rb").read()
    length = struct.unpack_from("<I", raw, 12)[0]
    gltf = json.loads(raw[20:20 + length])
    scene = trimesh.load(f"/m/{name}.glb", force="scene")
    print(name, "bytes", len(raw), "meshes", len(gltf["meshes"]), "nodes", len(gltf["nodes"]),
          "materials", [m.get("pbrMetallicRoughness", {}) | {"baseColorTexture": "..."} for m in gltf["materials"]],
          "extents_m", [round(float(v), 4) for v in scene.extents],
          "components", sum(len(trimesh.Trimesh(g.vertices, g.faces).split(only_watertight=False)) for g in scene.geometry.values()))
'
