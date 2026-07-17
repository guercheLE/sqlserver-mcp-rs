#!/usr/bin/env bash
# Run the full EDA extraction (objects, params, resultset) against one running
# SQL Server container, across master/msdb/sandbox, dumping JSON into data/<version>/.
#
# Runs sqlcmd *inside* the container (via docker exec) rather than requiring
# sqlcmd on the host -- the official SQL Server images already ship
# /opt/mssql-tools18/bin/sqlcmd. The sql/eda directory is copied into the
# container with `docker cp` so sqlcmd's `:r` file includes (used to share
# the allowlist between objects.sql/params.sql/resultset.sql) resolve against
# a real filesystem path.
#
# Usage: scripts/extract.sh <2017|2019|2022|2025>
set -euo pipefail

version="${1:?usage: extract.sh <2017|2019|2022|2025>}"
cd "$(dirname "$0")/.."

case "$version" in
  2017|2019|2022|2025) ;;
  *) echo "unknown version: $version" >&2; exit 1 ;;
esac

: "${MSSQL_SA_PASSWORD:?set MSSQL_SA_PASSWORD (e.g. source .env)}"

container="mssql${version}"
remote_dir="/tmp/eda"

# The 2017 image ships the older `mssql-tools` package; 2019+ ship
# `mssql-tools18` at a different path. Detect which one this container has.
if docker exec "$container" test -x /opt/mssql-tools18/bin/sqlcmd; then
  sqlcmd_bin="/opt/mssql-tools18/bin/sqlcmd"
else
  sqlcmd_bin="/opt/mssql-tools/bin/sqlcmd"
fi

outdir="data/${version}"
mkdir -p "$outdir"

# `docker cp` writes as root inside the container, but sqlcmd runs as the
# unprivileged `mssql` user -- so any cleanup of a previous copy must run as
# root too, or `rm` gets a permission denied on root-owned files.
docker exec --user root "$container" rm -rf "$remote_dir"
docker cp sql/eda "$container:$remote_dir"
docker exec --user root "$container" chmod -R a+rX "$remote_dir"

run_sql() {
  # sqlcmd resolves `:r` includes relative to its own current working
  # directory, not the including script's directory -- so cd into
  # remote_dir before invoking it (otherwise `:r allowlist_names.sql`
  # fails with "Invalid filename").
  # -y 0 -Y 0 disable sqlcmd's default 256-char truncation of (n)varchar(max)
  # columns, and -w 65535 disables line-wrapping at the default 80-char
  # screen width -- without both, the single giant FOR JSON string gets
  # truncated and/or has raw newlines spliced into it, corrupting the JSON.
  # -d "$db" sets the connection's initial database; -v db="$db" additionally
  # makes it explicit *inside* the script text via `USE $(db);` (see the
  # header comment in each sql/eda/*.sql file) -- so the active database is
  # never just an invisible command-line flag.
  local db="$1" script="$2" out_local="$3"
  docker exec -w "$remote_dir" "$container" "$sqlcmd_bin" -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -C \
    -y 0 -Y 0 -w 65535 -d "$db" -v db="$db" -i "${script}" -o "/tmp/out.json"
  docker cp "$container:/tmp/out.json" "$out_local"
}

docker exec "$container" "$sqlcmd_bin" -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -C \
  -Q "IF DB_ID('sandbox') IS NULL CREATE DATABASE sandbox;"

for db in master msdb sandbox; do
  for script in objects params resultset; do
    echo "extracting ${script} from ${db} (SQL Server ${version})..."
    run_sql "$db" "${script}.sql" "${outdir}/${db}.${script}.json"
  done
done

echo "done: ${outdir}/*.json"
