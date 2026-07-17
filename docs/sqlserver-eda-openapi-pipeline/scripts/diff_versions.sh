#!/usr/bin/env bash
# Run version_diff.sql (inside each container, via docker exec) against every
# currently-healthy container and print pairwise diffs of the matched object
# list for a given database.
# Usage: scripts/diff_versions.sh <master|msdb|sandbox>
set -euo pipefail

db="${1:?usage: diff_versions.sh <master|msdb|sandbox>}"
cd "$(dirname "$0")/.."
: "${MSSQL_SA_PASSWORD:?set MSSQL_SA_PASSWORD (e.g. source .env)}"

remote_dir="/tmp/eda"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

versions=()
for version in 2017 2019 2022 2025; do
  container="mssql${version}"
  if docker inspect -f '{{.State.Health.Status}}' "$container" 2>/dev/null | grep -q healthy; then
    # The 2017 image ships the older `mssql-tools` package at a different
    # path than 2019+'s `mssql-tools18`; detect which one this container has.
    if docker exec "$container" test -x /opt/mssql-tools18/bin/sqlcmd; then
      sqlcmd_bin="/opt/mssql-tools18/bin/sqlcmd"
    else
      sqlcmd_bin="/opt/mssql-tools/bin/sqlcmd"
    fi
    # docker cp writes as root; rm/chmod on a prior copy must run as root too.
    docker exec --user root "$container" rm -rf "$remote_dir"
    docker cp sql/eda "$container:$remote_dir"
    docker exec --user root "$container" chmod -R a+rX "$remote_dir"
    # cd into remote_dir first: sqlcmd resolves `:r` includes relative to its
    # own cwd, not the including script's directory. -v db="$db" makes the
    # active database explicit via `USE $(db);` in version_diff.sql itself,
    # not just via the -d connection flag -- see sql/eda/objects.sql.
    docker exec -w "$remote_dir" "$container" "$sqlcmd_bin" -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -C \
      -y 0 -Y 0 -w 65535 -d "$db" -v db="$db" -i "version_diff.sql" -o "/tmp/diff_out.txt" -h -1 -W
    docker cp "$container:/tmp/diff_out.txt" "${tmpdir}/${version}.txt"
    versions+=("$version")
  else
    echo "skipping ${version}: ${container} not healthy/running" >&2
  fi
done

for i in "${!versions[@]}"; do
  for j in "${!versions[@]}"; do
    if [ "$i" -lt "$j" ]; then
      a="${versions[$i]}"; b="${versions[$j]}"
      echo "=== ${a} vs ${b} (${db}) ==="
      diff "${tmpdir}/${a}.txt" "${tmpdir}/${b}.txt" || true
      echo
    fi
  done
done
