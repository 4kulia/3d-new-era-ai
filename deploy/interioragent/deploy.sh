#!/usr/bin/env bash
# Forced command of the interioragent-deploy SSH key. stdin is a gzipped tar
# holding images.tar (a `docker save` of interioragent-cloud:<sha> and
# interioragent-editor:<sha>) and the commit's docker-compose.yml; the caller
# controls only <sha> and the images. The compose file, this script and the
# nginx site are installed on the host by hand from deploy/interioragent/: a
# commit whose compose file differs from the installed one is refused, with
# the difference, until the new file is installed. Then: load the images,
# switch TAG, wait for both services to answer, put the previous TAG back when
# they do not.
set -euo pipefail

dir=/opt/interioragent
sha="${SSH_ORIGINAL_COMMAND:-${1:-}}"
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo "expected a 40-hex commit sha, got: $sha" >&2; exit 2; }

cd "$dir"
exec 9>"$dir/.deploy.lock"
flock -n 9 || { echo "another deploy is running" >&2; exit 3; }

incoming="$(mktemp -d)"
trap 'rm -rf "$incoming"' EXIT
tar -xzf - -C "$incoming" images.tar docker-compose.yml
if ! diff -u docker-compose.yml "$incoming/docker-compose.yml" >&2; then
  echo "this commit changes docker-compose.yml: install it on the host first" >&2
  exit 4
fi
docker load -i "$incoming/images.tar"
for image in interioragent-cloud interioragent-editor; do
  docker image inspect "$image:$sha" >/dev/null
done

previous="$(sed -n 's/^TAG=//p' .env)"
set_tag() { sed -i "s/^TAG=.*/TAG=$1/" .env; }

healthy() {
  for _ in $(seq 1 60); do
    if curl -fsS -m 5 http://127.0.0.1:7979/cloud/health | grep -q '"ok":true' \
      && curl -fsS -m 5 -o /dev/null http://127.0.0.1:7978/app/; then
      return 0
    fi
    sleep 2
  done
  return 1
}

set_tag "$sha"
if docker compose up -d --remove-orphans && healthy; then
  echo "deployed $sha"
  docker image ls --format '{{.Repository}}:{{.Tag}}' \
    | grep -E '^interioragent-(cloud|editor):' \
    | grep -v -e ":$sha$" -e ":${previous:-none}$" \
    | xargs -r docker image rm >/dev/null || true
  exit 0
fi

echo "new build $sha is unhealthy; logs follow, rolling back to ${previous:-nothing}" >&2
docker compose logs --tail 80 app editor >&2 || true
if [ -n "$previous" ]; then
  set_tag "$previous"
  docker compose up -d --remove-orphans && healthy && echo "rolled back to $previous" >&2
fi
exit 1
