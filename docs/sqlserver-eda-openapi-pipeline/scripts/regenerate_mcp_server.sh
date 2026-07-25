#!/usr/bin/env bash
# Regenerates the sqlserver-mcp-rs project's OpenAPI specs and mcpify-managed
# scaffolding from the raw EDA extraction output already sitting in
# data/<version>/ (no Docker/live SQL Server instance required for this
# script itself -- see scripts/extract.sh for the step that actually
# populates data/).
#
# IMPORTANT: this script always extracts+generates ALL FOUR versions before
# touching mcpify -- never call tools/generate_openapi.py for a single
# version in isolation as part of a real regeneration. Its ranking prefers
# operations that exist on every extracted version (see
# tools/generate_openapi.py's compute_presence()) precisely so the top-500
# cut lands on nearly the same operation set on 2017/2019/2022/2025 alike;
# that only works when data/<version>/ already has all four versions'
# EDA output on disk *before* generation runs for any of them.
#
# Runs, in order, for every version (2017/2019/2022/2025):
#   1. tools/generate_openapi.py <version>
#      -- loads master/msdb/model EDA output together, deduplicates objects
#         found in more than one of those databases into a single operation
#         each, ranks (cross-version presence, then metadata completeness),
#         caps to the top 500, and writes openapi/<version>/combined.yaml
#         directly (no separate merge step -- unlike the old per-database +
#         merge design, deduplicated operations don't need database-prefixed
#         paths to stay disjoint).
#   2. openapi-spec-validator against the generated result.
# Then, once, from the repo root:
#   3. `mcpify sync --manifest mcpify.yaml`
#      -- regenerates the Rust project's mcpify-managed scaffolding
#         (mcp_store*.db.zst, src/validation/generated_schemas*.json.zst,
#         and the marker-delimited "version-aware" regions in a handful of
#         source files) from the four merged specs.
#   5. the `populate-embeddings` binary (`--all`)
#      -- refills `semantic_endpoints` for every version (mcpify's
#         generator leaves it empty). No resize step needed first: mcpify
#         hard-codes that table as `FLOAT[768]`, which matches this
#         project's own model (see src/services/embedding_service.rs's doc
#         comment) -- the two haven't diverged since the 768-dim package
#         size was re-measured post-zstd-compression and found to fit
#         crates.io's limit with headroom.
#
# IMPORTANT: `mcpify sync` fully regenerates several files from its Tera
# templates (observed directly: it overwrote hand-edited
# src/services/mod.rs and src/validation/validator.rs on every run during
# this project's initial build-out) -- it is NOT limited to the
# marker-delimited regions its own documentation describes. Any hand-edit
# you've made to a mcpify-templated file (most of src/auth/, src/services/,
# src/core/, src/tools/, src/cli/, src/http/, src/data/store.rs,
# src/validation/validator.rs) will need to be redone after running this
# script. Since this project's transport/auth were hand-rewritten from
# mcpify's original HTTP-client scaffolding to a real TDS connection (see
# src/services/api_client.rs's module doc), *committing your work before
# running this script* (`git status` clean, or stashed) is the only way to
# safely recover hand-edits afterward -- there is no other undo.
#
# Usage: scripts/regenerate_mcp_server.sh
# (always operates on all four versions; there is no single-version mode,
# since mcpify.yaml's `mcpify sync` always syncs every version it lists.)

set -euo pipefail

PIPELINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$PIPELINE_DIR/../.." && pwd)"
VERSIONS=(2017 2019 2022 2025)

cd "$PIPELINE_DIR"
if [ ! -d .venv ]; then
  echo "error: $PIPELINE_DIR/.venv not found -- see README.md's Setup section" >&2
  exit 1
fi
# shellcheck disable=SC1091
source .venv/bin/activate

for version in "${VERSIONS[@]}"; do
  echo "== $version: generating combined.yaml (master/msdb/model, deduplicated) =="
  python3 tools/generate_openapi.py "$version"
  openapi-spec-validator "openapi/$version/combined.yaml"
done

echo "== syncing mcpify project at $REPO_ROOT =="
cd "$REPO_ROOT"
if ! command -v mcpify >/dev/null 2>&1; then
  echo "error: mcpify not found on PATH (cargo install --path <mcpify checkout>, or see its README)" >&2
  exit 1
fi
mcpify sync --manifest mcpify.yaml

cat <<'EOF'

== mcpify sync done -- STOP AND RE-APPLY HAND-EDITS BEFORE CONTINUING ==
mcpify sync may have just reverted src/services/embedding_service.rs and
any other hand-edited file (see this script's header comment for the full
list). Re-apply those, `cargo build --all-targets && cargo test` to
confirm, THEN run:

  cargo build --release --bin sqlserver-mcp-populate-embeddings
  ./target/release/sqlserver-mcp-populate-embeddings --all
EOF
