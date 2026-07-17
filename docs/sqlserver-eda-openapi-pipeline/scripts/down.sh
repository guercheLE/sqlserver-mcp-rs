#!/usr/bin/env bash
# Stop one SQL Server version container (or all, if no argument given).
# Usage: scripts/down.sh [<2017|2019|2022|2025>]
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$#" -eq 0 ]; then
  docker compose down
  exit 0
fi

version="$1"
case "$version" in
  2017|2019|2022|2025) ;;
  *) echo "unknown version: $version (expected 2017, 2019, 2022, or 2025)" >&2; exit 1 ;;
esac

docker compose stop "mssql${version}"
