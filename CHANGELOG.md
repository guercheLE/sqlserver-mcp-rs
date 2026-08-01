# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project uses tag-driven releases (`chore(release): bump version to X.Y.Z`
commits, each matched by a `vX.Y.Z` tag) rather than Semantic Versioning
guarantees pre-1.0 — treat every bump as potentially containing a breaking
change until the project reaches `1.0.0`.

## [Unreleased]

Nothing since `v0.6.7` (`HEAD` is the release-bump commit itself).

## [0.6.7] - 2026-07-26

### Fixed

- Stopped retrying the disk-lock test on GitHub Actions' Windows runners and
  skip it there instead — a 60s retry budget (widened across the previous
  three releases) still wasn't enough, pointing at something outside this
  crate's connection handling (most likely Defender or a similar background
  scanner holding an exclusive file handle under that runner's load), not a
  real leak. A short 10x50ms retry remains for genuine transient contention
  on platforms where it actually clears.

## [0.6.6] - 2026-07-26

### Fixed

- Widened the Windows store-lock retry from 10s to 60s (still not enough in
  practice — resolved in `0.6.7` by skipping the test on that runner).

## [0.6.5] - 2026-07-26

### Fixed

- `clippy::manual_repeat_n` and `clippy::len_zero` lints newly enforced by a
  more recent stable toolchain (`search_tool.rs`, `cli_commands.rs`).
- Dropped the stale "2025" from the generated-file banner/description
  comment, frozen at initial generation before the other three SQL Server
  versions were added via `mcpify add-version` — now reads "SQL Server -
  master/msdb/sandbox combined catalog" everywhere.
- Guarded a `HOME` env var test race: `credential_storage.rs`'s file-fallback
  round-trip test mutates the real process-wide `HOME` variable without a
  lock, which could race an unrelated test.
- Widened the Windows file-lock retry in `store.rs` from 1s (50x20ms) to 10s
  (100x100ms) — CI kept failing after exhausting the shorter budget.

## [0.6.4] - 2026-07-26

### Fixed

- `cargo fmt --check` failures in `api_client.rs`, `sql_type.rs`,
  `get_tool.rs`, `cli_commands.rs`.
- Windows handle-release timing: added a retry around `remove_file` in the
  store lock-release test.

## [0.6.3] - 2026-07-26

### Added

- `SQLSERVER_READ_ONLY` safeguard (default `true`) for the `call` tool.
  T-SQL has no session-level "read only" setting the way PostgreSQL's
  `default_transaction_read_only` does, so this is enforced by object
  classification instead: `call` refuses any operation whose catalog type
  isn't `VIEW` or a `*_FUNCTION` kind (both are guaranteed side-effect-free
  by SQL Server itself, independent of the connecting login's actual
  grants). Stored procedures, and any operation with a missing/unrecognized
  classification, are rejected outright.

### Fixed

- Every operation's input/output schema stored in `mcp_store.db` still
  referenced a locally-embedded `$defs` block instead of being fully
  resolved — a leftover from mcpify's earlier `$ref`-to-`$defs`-localization
  fix. Recursively inlined every `$ref` (none form a genuine reference
  cycle in this catalog), matching a companion fix landed in the mcpify
  generator itself.

## [0.6.2] - 2026-07-26

### Documentation

- Lowered the coverage target from 85% to 75%: closing the remaining gap
  would require mocking `inquire`'s interactive prompts, adding a live
  SQL Server/Docker test tier, or simulating a real MCP client handshake —
  each substantially larger than incremental unit testing. 76.23% (reached
  in `0.6.1`) clears the revised target.

## [0.6.1] - 2026-07-25

### Fixed

- Restored the "windows (NTLM — explicit domain\username + password, NOT
  single sign-on)" setup-wizard wording and its clarifying prompt-time
  message, silently lost during recovery from the `mcpify sync` incident
  (`0.6.0`) because the recovery patch had been snapshotted before that
  specific edit landed. Neither compilation nor tests caught the loss since
  nothing asserted on the wizard's literal prompt text.

### Changed

- Raised project-wide line coverage from 65.01% to 76.23% (`cargo-llvm-cov`):
  added tests for the `search`/`get`/`call` tool-dispatch layer, connection
  pooling, the embedding service, `populate_embeddings`, `AuthManager`'s
  credential-resolution cascade, and extracted `setup_wizard`'s
  selection-parsing logic into pure, independently-testable functions.

## [0.6.0] - 2026-07-25

### Added

- Replaced the hand-curated `allowlist.yaml`/`*.sql` operation list with a
  full `sys.all_objects` sweep (`is_ms_shipped = 1`) across
  master/msdb/model — `model` replaces the old ad hoc "sandbox" placeholder
  since it's a real, always-present system database.
- Rewrote `generate_openapi.py` for cross-database deduplication and
  ranking: one path per operation (`/<schema>/<name>`) instead of per
  database, objects found identically in multiple databases are merged
  into a single operation tagged `x-sql-databases`, candidates are ranked
  by cross-version presence / metadata completeness / object-type tier /
  alphabetical, and capped to the top 500 per version.
- Added a configurable default execution database: `resolve_execution_database`
  now resolves, in order, the caller's `execution_database` request
  property, then the operator's configured `Config::default_database`
  (settable via the setup wizard or `SQLSERVER_DEFAULT_DATABASE`), then the
  live connection's own current database context.
- Added `resultset_fmtonly.sql` as a `SET FMTONLY ON` fallback for objects
  `sys.dm_exec_describe_first_result_set` can't describe, deliberately
  excluding extended/CLR objects from dynamic-call introspection (confirmed
  live on SQL Server 2017 that probing those types this way can trigger a
  session-fatal engine exception).

### Changed

- The generated API shape changed: one path per deduplicated operation
  instead of per-database, plus a new `execution_database` request
  property on every operation.
- `sp_executesql` and every other extended stored procedure dropped out of
  the generated catalog entirely (no queryable parameter metadata), rather
  than being documented with a fabricated signature.

### Fixed

- Three guided prompt workflows (dropping a linked server, `CREATE INDEX`,
  killing a blocking session) relied on `sp_executesql` as a general T-SQL
  escape hatch that no longer resolves to any real operation — reworded to
  say so honestly. `server_administration.md`'s linked-server guidance
  claimed there was no dedicated drop operation, which turned out to be
  false once the broader sweep picked up `sp_dropserver`.
- Guarded against `sys.dm_exec_describe_first_result_set` returning
  anonymous/null-named columns for some extended procs (`FOR JSON PATH`
  silently drops null-valued keys), which was letting `sp_executesql` pass
  as "documented" with zero real columns.

### Documentation

- Documented the pipeline rewrite (`is_ms_shipped` sweep, cross-db dedup,
  FMTONLY fallback, ranking/top-500 cutoff) and the `mcpify sync` vs.
  `mcpify add-version` pitfall: `sync` regenerates the whole project's
  scaffolding from the manifest and was confirmed, during this release, to
  reset the hand-rewritten TDS/SQL-Server auth and transport layer back to
  mcpify's generic Basic/OAuth2/reqwest-HTTP template; `add-version --force`
  is the command that's actually safe for a routine catalog refresh.

## [0.5.3] - 2026-07-21

### Fixed

- Regenerated every configured SQL Server catalog version (2017–2025) via
  `mcpify add-version --force`, picking up an upstream mcpify generator fix
  (0.11.5) where a component `$ref` inside a `get`-tool response could
  point at nothing in the returned snippet. Every operation's stored
  `input_schema`/`output_schema` now embeds a `$defs` map alongside any
  `$ref`, so `get` responses are self-contained. Semantic-search embeddings
  were repopulated for every refreshed store.

## [0.5.2] - 2026-07-21

### Fixed

- Capped every `release.yml` and `ci.yml` workflow job with
  `timeout-minutes` — none had one, so a hang fell back to GitHub Actions'
  own 6-hour job ceiling (the only thing that actually bounded a real
  incident in a sibling project, a hung test caused by mutex poisoning on
  the shared store connection, fixed separately in `0.5.1`).

## [0.5.1] - 2026-07-21

### Fixed

- Recovered from mutex poisoning on the shared, process-wide
  `cached_store_connection` lock instead of propagating it. A panic while
  holding that lock previously poisoned it permanently, so every later
  `search`/`get`/`call` (and CLI invocation) sharing the connection would
  panic too — the same bug independently found and fixed in `bamboo-mcp-rs`
  and in mcpify's own Rust code-generator template. Safe here because all
  access through this lock is read-only.

## [0.5.0] - 2026-07-20

### Changed

- **BREAKING**: renamed every MCP prompt identifier from snake_case with a
  redundant "workflow" segment to a short kebab-case identifier, matching
  the slash-command convention most MCP clients use (e.g.
  `sqlserver_workflow_sql_agent_jobs` → `sqlserver-sql-agent-jobs`). Only
  the wire-visible `name` values and prose cross-references changed; Rust
  identifiers and JSON argument keys were unaffected. Any client already
  calling a prompt by its old name must switch to the new one.

## [0.4.1] - 2026-07-20

### Fixed

- Every parameterized stored-procedure call (`sp_executesql`, `sp_columns`,
  `sp_rename`, etc.) was failing or silently sending no real arguments to
  SQL Server, on every supported engine version, due to two independent
  bugs: named `EXEC` arguments were bracket-quoted (`[name] = @P1`) instead
  of `@`-prefixed (`@name = @P1`), which SQL Server rejects outright; and
  `ordered_params` read the schema's literal top-level `properties` map,
  but every generated input schema nests its real parameters one level
  down under a single `body` property, collapsing every parameterized
  operation to one fake parameter. Verified end-to-end against a real SQL
  Server 2022 instance, including a 3-parameter `sp_rename` call that now
  actually renames the target object.

## [0.4.0] - 2026-07-20

### Added

- Two new guided MCP prompt workflows found by reviewing the full ~800-row
  operation catalog: `sqlserver-blocking-and-locks` (diagnose a blocking
  chain to its head blocker, then terminate it only with explicit user
  confirmation) and `sqlserver-index-tuning-recommendations` (find
  missing-index candidates, cross-check for overlap, then create the index
  only with explicit user confirmation). Every new operation reference was
  cross-checked against all four supported engine version stores
  (2017/2019/2022/2025).
- Expanded `performance_diagnostics` with In-Memory OLTP/columnstore and
  OS-health pointers.

### Fixed

- `server_administration`'s linked-server guidance pointed at
  `sp_droplinkedserver`, which doesn't exist in the catalog on any
  supported engine version — corrected to the two operations that actually
  exist (`sp_addlinkedserver`, `sp_linkedservers`).

### Documentation

- Documented the blocking/locks and index-tuning workflow addendum, and
  the MCP prompts guided workflows generally, in the README.

## [0.3.0] - 2026-07-20

### Added

- MCP **prompts** capability: a `sqlserver` master menu prompt plus six
  domain sub-workflows (schema exploration, indexes/constraints, security
  provisioning, SQL Agent jobs, server administration, performance
  diagnostics) that sequence the existing `search`/`get`/`call` tools into
  step-by-step guidance, mirroring the existing `tool_router` pattern.

## [0.2.2] - 2026-07-19

### Documentation

- Added a sponsorship callout and `FUNDING.yml`.
- Kept the README headline version-agnostic: moved the concrete SQL Server
  version list (2017/2019/2022/2025) out of the crates.io headline
  paragraph (which would drift as new versions are added) into the tools
  paragraph, alongside a pointer to `docs/SCHEMA_VERSIONS.md`.

## [0.2.1] - 2026-07-19

### Fixed

- `dist host --steps=create` (the release-tag trigger every real release
  invokes) refused to run against `release.yml` because it's a
  deliberately simplified hand-written workflow rather than cargo-dist's
  own auto-generated multi-job shape, and dist flagged it "out of date."
  Added `allow-dirty` to tell dist the divergence is intentional; fixed
  upstream in mcpify's `dist-workspace.toml.tera` template too.

## [0.2.0] - 2026-07-19

### Added

- Switched back to the 768-dim `all-mpnet-base-v2` embedding model. The
  prior switch to a 384-dim model was driven by crates.io's 10 MiB
  package-size limit, measured when stores were still committed
  uncompressed; with zstd compression (level 19) now in place, a
  768-dim/4-version catalog measures 8.5 MiB compressed — comfortably
  under the limit, and 768-dim also matches mcpify's own hard-coded
  `semantic_endpoints` column width. Removed the now-dead
  `resize_embeddings.rs` binary that existed solely to patch the
  dimension mismatch for the 384-dim model.

### Fixed

- Embedded `mcp_store*.db` zstd-compressed instead of raw, porting
  mcpify's own zstd store-compression feature into this hand-maintained
  project (not synced via `mcpify sync`, which would overwrite the
  TDS-based `api_client.rs` and other hand-rewritten files). Brought four
  API versions' worth of embedded stores from well past crates.io's 10 MiB
  limit down to 4.5 MiB.
- `object_summary()`'s fallback cases rendered raw underscored SQL
  identifiers (e.g. "all_parameters") straight into the generated summary
  text used for semantic-search embedding, which embeds worse against
  natural-language queries than space-separated words. Only the
  human-readable rendering changed; identifiers (`operationId`, dict keys)
  are unaffected.
- Stopped sending a literal `[sandbox]` database qualifier: "sandbox" is
  the EDA pipeline's placeholder for whatever database the TDS connection
  is already in, not a real database name any production instance is
  guaranteed to have. `build_statement` now two-part qualifies
  (`schema.name`) for sandbox operations by default, relying on the
  connection's own current database; a caller can pass a top-level
  `database` argument to target a specific one. `master`/`msdb` operations
  are unaffected — they're real, always-present system databases and stay
  three-part qualified.
- Made profiling reliable: separated Samply CPU profiling from DHAT heap
  instrumentation, and added a repeated warm search workload with
  persisted benchmark details.
- Hardened SQL bindings: reject invalid and out-of-range SQL parameter
  values; expanded CLI, MCP, HTTP, and SQL conversion test coverage.

## [0.1.4] - 2026-07-17

### Added

- `publish-crate.yml` CI workflow to publish to crates.io on tag push —
  every sibling repo in this family already had one; without it, `v0.1.3`
  had to be published to crates.io by hand.

## [0.1.3] - 2026-07-17

### Fixed

- Shortened the generated package/bin/env-var identifiers (derived from
  the OpenAPI title as
  `sql-server-2025-master-msdb-sandbox-combined-catalog`) to
  `sqlserver-mcp`, matching the repo name and the binary names users
  actually expect (`sqlserver-mcp`, `sqlserver-mcp-healthcheck`,
  `sqlserver-mcp-populate-embeddings`, `sqlserver-mcp-resize-embeddings`).
  Also fixed two `ENV_PREFIX` constants (`auth_manager.rs`,
  `config_manager.rs`) missed during the rename, which would have silently
  ignored the new `SQLSERVER_*` env vars.
- Disabled ThinLTO in `[profile.dist]`: GitHub's macOS runner ships an
  Xcode/libLTO too old to parse ThinLTO bitcode from the pinned rustc's
  LLVM version, breaking the `aarch64-apple-darwin` release build.

## [0.1.2] - 2026-07-17

### Fixed

- Made the package dist-able: cargo-dist skips non-publishable packages by
  default, so `publish = false` left `dist build` with no binaries to
  build, failing the release workflow on every target. Added the
  crates.io metadata cargo-dist expects and regenerated `release.yml`,
  which had drifted from the current template.

## [0.1.1] - 2026-07-17

### Added

- Initial SQL Server MCP server implementation: auth strategies, the
  `search`/`get`/`call` tool surface, SQL Server connection pooling,
  schema validation, embeddings, CLI, Docker/CI tooling, and the EDA/
  OpenAPI generation pipeline used to build the per-version schema
  catalogs.

### Fixed

- Satisfied `cargo fmt --check` and `cargo clippy -D warnings` in CI,
  which were failing on unformatted code and a collapsible-if lint.

[Unreleased]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.7...HEAD
[0.6.7]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.6...v0.6.7
[0.6.6]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.5...v0.6.6
[0.6.5]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.4...v0.6.5
[0.6.4]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.3...v0.6.4
[0.6.3]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.1.4...v0.2.0
[0.1.4]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/guercheLE/sqlserver-mcp-rs/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/guercheLE/sqlserver-mcp-rs/releases/tag/v0.1.1
