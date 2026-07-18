#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT

mkdir -p "$test_dir/bin"

cat > "$test_dir/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$PROFILE_TEST_LOG"
STUB

cat > "$test_dir/bin/samply" <<'STUB'
#!/usr/bin/env bash
printf 'samply %s\n' "$*" >> "$PROFILE_TEST_LOG"
touch profile/profile.json.gz
STUB

cat > "$test_dir/bin/python3" <<'STUB'
#!/usr/bin/env bash
touch "$3" "$4"
STUB

chmod +x "$test_dir/bin/cargo" "$test_dir/bin/samply" "$test_dir/bin/python3"

export PROFILE_TEST_LOG="$test_dir/commands.log"
PATH="$test_dir/bin:$PATH" bash "$repo_root/scripts/profile.sh" >/dev/null

cpu_build="$(head -n 1 "$PROFILE_TEST_LOG")"
if [[ "$cpu_build" != "build --release" ]]; then
  echo "expected CPU profiling to build an uninstrumented release binary" >&2
  echo "actual cargo command: $cpu_build" >&2
  exit 1
fi

if ! grep -q -- '--profile-warmups 3 --profile-iterations 250' "$PROFILE_TEST_LOG"; then
  echo "expected CPU profiling to use a warm repeated-search workload" >&2
  cat "$PROFILE_TEST_LOG" >&2
  exit 1
fi

: > "$PROFILE_TEST_LOG"
PATH="$test_dir/bin:$PATH" bash "$repo_root/scripts/profile-heap.sh" >/dev/null

heap_run="$(head -n 1 "$PROFILE_TEST_LOG")"
if [[ "$heap_run" != "run --release --features profiling --bin sqlserver-mcp -- search test query --profile-warmups 1 --profile-iterations 5" ]]; then
  echo "expected heap profiling to be isolated behind the profiling feature" >&2
  echo "actual cargo command: $heap_run" >&2
  exit 1
fi

echo "profile script separation test passed"
