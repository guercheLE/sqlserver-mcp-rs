#!/usr/bin/env bash
# Regenerates the sqlserver-mcp-rs project's OpenAPI specs and mcpify-managed
# scaffolding from the raw EDA JSON dumps already sitting in data/<version>/
# (no Docker/live SQL Server instance required for this script itself --
# see scripts/extract.sh for the step that actually populates data/).
#
# Runs, in order, for every version (2017/2019/2022/2025):
#   1. tools/generate_openapi.py <version> {master,msdb,sandbox}
#      -- per-database synthetic OpenAPI specs.
#   2. tools/merge_openapi.py <version>
#      -- merges the three into openapi/<version>/combined.yaml (see that
#         script's own module docstring for why the merge is needed: the
#         three per-database specs reuse identical paths/operationIds, so
#         master/msdb/sandbox couldn't otherwise coexist as one mcpify
#         project version).
#   3. openapi-spec-validator against the merged result.
# Then, once, from the repo root:
#   4. `mcpify sync --manifest mcpify.yaml`
#      -- regenerates the Rust project's mcpify-managed scaffolding
#         (mcp_store*.db, src/validation/generated_schemas*.json.zst, and
#         the marker-delimited "version-aware" regions in a handful of
#         source files) from the four merged specs.
#   5. the `resize-embeddings` binary
#      -- mcpify's generator hard-codes `semantic_endpoints` as
#         `FLOAT[768]` (see src/services/embedding_service.rs's doc
#         comment) -- every `mcpify sync` re-creates that 768-dim column,
#         so it has to be patched back to this project's actual model
#         dimension (384) before embeddings are (re)computed.
#   6. the `populate-embeddings` binary (`--all`)
#      -- refills `semantic_endpoints` for every version (steps 4-5 both
#         leave it empty), or `search` returns nothing.
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
  echo "== $version: generating per-database specs =="
  for db in master msdb sandbox; do
    python3 tools/generate_openapi.py "$version" "$db"
  done

  echo "== $version: merging into combined.yaml =="
  python3 tools/merge_openapi.py "$version"
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
mcpify sync may have just reverted src/services/embedding_service.rs (the
resize/populate steps below depend on it declaring the *current* model --
running them against a reverted embedding_service.rs would silently
re-populate 768-dim vectors into a table this script is about to size for
384-dim, or worse, a dimension mismatch error) and any other hand-edited
file (see this script's header comment for the full list). Re-apply those,
`cargo build --all-targets && cargo test` to confirm, THEN run:

  cargo build --release --bin sql-server-2025-master-msdb-sandbox-combined-catalog-resize-embeddings
  ./target/release/sql-server-2025-master-msdb-sandbox-combined-catalog-resize-embeddings
  cargo build --release --bin sql-server-2025-master-msdb-sandbox-combined-catalog-populate-embeddings
  ./target/release/sql-server-2025-master-msdb-sandbox-combined-catalog-populate-embeddings --all
EOF
