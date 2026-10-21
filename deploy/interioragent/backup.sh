#!/usr/bin/env bash
# Nightly dump of the interioragent database, kept 14 days. Run by cron as kulia.
# The password is read inside the container from its own POSTGRES_PASSWORD.
set -euo pipefail
out=/home/kulia/interioragent_backups
mkdir -p "$out"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
part="$out/interioragent-$stamp.dump.part"
trap 'rm -f "$part"' EXIT
docker exec interioragent-postgres-1 \
  sh -c 'PGPASSWORD="$POSTGRES_PASSWORD" pg_dump -U postgres -d interioragent --format=custom' > "$part"
mv "$part" "$out/interioragent-$stamp.dump"
find "$out" -name 'interioragent-*.dump' -mtime +14 -delete
