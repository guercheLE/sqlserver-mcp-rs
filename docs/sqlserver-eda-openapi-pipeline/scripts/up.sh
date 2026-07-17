#!/usr/bin/env bash
# Bring up one SQL Server version container and wait for it to become healthy.
# Usage: scripts/up.sh <2017|2019|2022|2025>
set -euo pipefail

version="${1:?usage: up.sh <2017|2019|2022|2025>}"
service="mssql${version}"

cd "$(dirname "$0")/.."

case "$version" in
  2017|2019|2022|2025) ;;
  *) echo "unknown version: $version (expected 2017, 2019, 2022, or 2025)" >&2; exit 1 ;;
esac

docker compose up -d "$service"

echo "waiting for $service to become healthy (2017/2019 run under amd64 emulation and can take several minutes)..."
until [ "$(docker inspect -f '{{.State.Health.Status}}' "$service" 2>/dev/null)" = "healthy" ]; do
  status="$(docker inspect -f '{{.State.Health.Status}}' "$service" 2>/dev/null || echo "starting")"
  if [ "$status" = "unhealthy" ]; then
    echo "$service reported unhealthy; recent logs:" >&2
    docker logs --tail 50 "$service" >&2
    exit 1
  fi
  sleep 5
done

port=$(docker compose port "$service" 1433 | cut -d: -f2)
echo "$service is healthy, listening on localhost:$port"
