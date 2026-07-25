# SQL Server Multi-Version EDA → OpenAPI Documentation Pipeline

## Context

`sqlserver-mcp-rs` is a brand-new, empty repository — presumably the future home of a Rust MCP server that exposes SQL Server functionality to an LLM. Before building that server, the goal here is to produce a reliable, versioned reference of SQL Server's built-in surface (system stored procedures, system functions, DMVs/DMFs, catalog views) across the four still-relevant engine versions (2017, 2019, 2022, 2025), captured by actually running each version in Docker and querying its own metadata — not just copying docs. The output is a set of OpenAPI YAML files per database/version that can later drive MCP tool schemas. This entire pipeline (Docker infra, SQL scripts, generator, generated output) is reference material feeding that future Rust server, not the server itself — hence it lives under `docs/` rather than at the repo root.

## Decisions made before implementation started

- **Scope**: curated common set — `sp_*` admin procs (sp_who, sp_help, sp_executesql, sp_configure, sp_helpdb, sp_rename, sp_columns, etc.), common DMVs/DMFs (`sys.dm_exec_*`, `sys.dm_os_*`, `sys.dm_db_*`, `sys.dm_tran_*`), `INFORMATION_SCHEMA.*` views, and `sys.*` catalog views/functions. Not a full exhaustive dump of every internal/undocumented object.
- **Platform**: emulate all 4 versions via `--platform linux/amd64` on Apple Silicon (2017/2019 have no arm64 image; 2022 has partial arm64 support; 2025 confirmed available at `2025-latest`). Accept slower container startup.
- **OpenAPI mapping**: each proc/function becomes a synthetic `POST /<schema>/<name>` operation. Parameters (from `sys.parameters`/`sys.all_parameters`/`sys.system_parameters`) become the request body schema; result-set columns (from `sys.dm_exec_describe_first_result_set`, or `INFORMATION_SCHEMA.ROUTINES`/`ROUTINE_COLUMNS` for scalar/table functions) become the response schema, when introspectable. One OpenAPI file per database per version (e.g. `2022/master.yaml`, `2022/msdb.yaml`).

## Approach (as built)

### 1. Docker infrastructure
`docker-compose.yml` with 4 services (`mssql2017`, `mssql2019`, `mssql2022`, `mssql2025`), each on `platform: linux/amd64`, `ACCEPT_EULA=Y`, `MSSQL_PID=Developer`, distinct host ports (14330–14333), distinct named volumes, and a healthcheck. `scripts/up.sh <version>` / `scripts/down.sh [<version>]` bring individual versions up/down and wait for healthy.

### 2. EDA extraction (per version, per database)
SQL scripts under `sql/eda/` run via `sqlcmd` *inside* each container (`scripts/extract.sh`, using `docker exec`/`docker cp` — no host-side `sqlcmd` needed):
- `allowlist.yaml` — human-readable curated list of object names/patterns (source of truth); `allowlist_names.sql`/`allowlist_patterns.sql` are the `sqlcmd`-loadable copies, kept in sync by hand.
- `objects.sql` — which allowlisted objects exist in the current database.
- `params.sql` — parameter metadata (name/type/direction/default) via `sys.all_parameters`.
- `resultset.sql` — best-effort result-set column introspection via `sys.dm_exec_describe_first_result_set`, with `unknown`/`no_result_set` status for objects that can't be described.
- `version_diff.sql` + `scripts/diff_versions.sh` — plain-text object list per version, diffable across versions.
- Output: raw JSON dumps to `data/<version>/<db>.<script>.json` (gitignored).

### 3. OpenAPI generation
`tools/generate_openapi.py` reads the JSON dumps and emits OpenAPI 3.1 YAML to `openapi/<version>/<database>.yaml`, one `POST` operation per object, with `components.schemas` for request/response bodies.

### 4. Known limitations
Documented in `README.md`: extended stored procedures (`xp_*`) excluded (no queryable metadata), conditional/dynamic result sets only describe the default invocation, 2017/2019 emulation is slow, `allowlist.yaml` and its `.sql` copies must be kept in sync by hand.

## Additions made after the initial implementation and verification

These were requested in follow-up conversation, after the pipeline above was built and first verified end-to-end against SQL Server 2022:

1. **`x-sql-type` annotations** — every property in every generated schema (request params, output params, response columns) carries an `x-sql-type` extension field with the exact SQL Server type text (e.g. `nvarchar(256)`, `decimal(18,2)`, `datetime2(7)`), computed in `format_sql_type()`/`sql_type_to_schema()` in `tools/generate_openapi.py`. Motivation: OpenAPI's `type`/`format` is too coarse to round-trip SQL Server types on its own (every string-like type maps to `type: string`), so this is the field to read for exact JSON↔SQL Server type mapping.

2. **Full end-to-end verification across all 4 versions**, not just 2022. This surfaced and fixed several real bugs:
   - `sqlcmd`'s `:r` file includes resolve relative to its own cwd, not the including script's directory → run via `docker exec -w /tmp/eda`.
   - `docker cp` writes as root but `sqlcmd` runs as the unprivileged `mssql` user → cleanup/chmod needs `docker exec --user root`.
   - The allowlist temp table's primary key collided under default case-insensitive collation (`tables` vs `TABLES`, etc.) → dropped the PK constraint.
   - `sqlcmd` truncates `nvarchar(max)` output at 256 chars and wraps at an 80-column screen width by default → added `-y 0 -Y 0 -w 65535`.
   - SQL Server's `FOR JSON` legitimately splits output over 2,033 characters across multiple result-set rows; `sqlcmd` writes each as its own line, so `load_json_dump()` must strip the newlines between them before parsing (a bare newline is never valid inside `FOR JSON`'s escaped string output).
   - `FOR JSON AUTO` auto-nests output based on join structure (`params.sql` joins 3 tables) → switched all EDA scripts to `FOR JSON PATH` for guaranteed flat rows.
   - The 2017 image ships the older `mssql-tools` package (`/opt/mssql-tools/bin/sqlcmd`), not `mssql-tools18` like 2019+ → healthcheck and extraction scripts now probe for the newer path first and fall back to the older one.

3. **Version-aware authentication scheme documentation** — each generated file's top-level `security` + `components.securitySchemes` document the TDS-protocol authentication modes that SQL Server *version* accepts, mapped to the closest-fitting OpenAPI shape (`build_security()` in `tools/generate_openapi.py`):
   - `sqlAuth` (`http`/`basic`) — SQL Server Authentication, all 4 versions.
   - `windowsAuth` (`http`/`negotiate`) — Windows/Integrated Authentication, all 4 versions.
   - `azureADAuth` (`oauth2`) — Azure AD/Microsoft Entra ID authentication, **2022 and 2025 only** — this was a new on-premises/Linux engine feature introduced in SQL Server 2022; 2017/2019 cannot authenticate this way at all.

4. **Explicit `x-sql-schema` / `x-sql-database` fields** on every operation, alongside the existing path/operationId encoding, so tooling doesn't need to parse schema/database back out of the path string. Same rationale as `x-sql-type`: an explicit field is more robust than re-deriving it.

5. **This reorganization** — moved the entire pipeline (`docker-compose.yml`, `.env.example`, `README.md`, `scripts/`, `sql/`, `tools/`, `data/`, `openapi/`, and this plan) into `docs/sqlserver-eda-openapi-pipeline/`, so the repo root stays clean for the eventual Rust MCP server and this reference-generation pipeline reads as a single self-contained unit under `docs/`.

6. **Explicit active-database context** — each `sql/eda/*.sql` script now opens with `USE $(db);`, driven by a required `sqlcmd` scripting variable (`-v db=<name>`, passed by `scripts/extract.sh`/`scripts/diff_versions.sh`), instead of relying solely on `sqlcmd`'s `-d` connection flag. Motivation: `-d` sets the database invisibly on the command line — if one of these `.sql` files were opened directly (SSMS, copy-paste elsewhere) the reader would have no way to tell which database it's meant to run against, and for `resultset.sql` specifically, an unqualified `EXEC`/`SELECT` built inside `sys.dm_exec_describe_first_result_set(@sql, ...)` resolves against whatever database is silently current — a wrong context there fails silently against the wrong object, not loudly. Verified by re-running the `sandbox` extraction and confirming `DB_NAME()` in every returned row is `sandbox`, not `master`/`msdb`.

7. **Error responses (`400`/`403`/`500`)** — every operation previously only documented `200`; there was no way to tell a spec reader what happens on `RAISERROR`/`THROW`/constraint violations/permission failures. Added `build_error_responses()` and a shared `components.schemas.SqlServerError` schema (`number`/`severity`/`state`/`procedure`/`line`/`message`, matching `ERROR_NUMBER()`/`ERROR_SEVERITY()`/etc.) in `tools/generate_openapi.py`. The status-code mapping follows SQL Server's own severity levels rather than being invented: severity 11–16 → `400`, severity-14 permission-denied errors specifically → `403` (split out from the rest of severity 14), severity 17–25 → `500`. Caught a real bug while implementing this: the shared `$ref` dict for the error schema was reused across the 400/403/500 entries within one `build_error_responses()` call, and even after deep-copying per operation, `copy.deepcopy` preserves internal object-identity aliasing — so PyYAML emitted `&id001`/`*id001` anchor/alias syntax across the generated files. Fixed by generating an independent `$ref` dict per status code; verified by grepping all 12 regenerated files for `&id`/`*id` (none found) and re-validating with `openapi-spec-validator`.

8. **Hand-curated parameters for 7 core engine procs** — the user noticed most paths document only `responses`, no request parameters, and asked whether `sp_help` could recover the missing ones for `sp_executesql`. Investigation (against a live SQL Server 2022 container) found: 183/246 `master` objects are `VIEW`s (genuinely parameterless — expected, not a bug); but 10 are `EXTENDED_STORED_PROCEDURE`s with zero rows in `sys.all_parameters`, 7 of which are important, well-documented system procs (`sp_executesql`, `sp_prepare`, `sp_execute`, `sp_unprepare`, `sp_describe_first_result_set`, `sp_describe_undeclared_parameters`, `sp_set_session_context`). Tested `sp_help` directly against the same container: it shows a parameter section for a regular proc like `sp_who` but only the bare header (no parameters) for `sp_executesql`/`xp_cmdshell` — confirming `sp_help` pulls from the exact same catalog we already query, so it can't recover anything sys.parameters doesn't have. Fetched all 7 signatures from Microsoft Learn (URLs in `tools/generate_openapi.py`'s `CURATED_PARAMETERS` comment) and hand-encoded them, including resolving two internal doc inconsistencies (`sp_prepare`'s prose mislabels `params` as OUTPUT, contradicted by its own example; `sp_execute`'s syntax box shows `handle OUTPUT`, contradicted by its argument description and example) by following the working examples. Every curated schema is tagged `x-sql-params-source: hand-curated` so it's distinguishable from live-introspected ones; variadic parameters (`sp_executesql`'s `@param1..N`, `sp_execute`'s `bound_param`) aren't statically enumerable and are called out in the schema description instead. Caught and fixed a second bug during this: curated default values were stored as strings (`"0"`) but the declared JSON schema `type` is `integer`/`boolean`, which `openapi-spec-validator` correctly rejected (`'0' is not of type 'integer'`) — fixed by using properly-typed Python values (`0`, `False`).

## Verification performed (initial implementation, superseded by the rewrite below)

1. Brought up each of the 4 containers in turn (`scripts/up.sh <version>`), ran the full extraction (`scripts/extract.sh <version>`) across `master`/`msdb`/`sandbox`, generated OpenAPI (`tools/generate_openapi.py <version> <db>`), and validated every output file with `openapi-spec-validator`. All 12 files (3 databases × 4 versions) validate cleanly.
2. Spot-checked real generated content: `sp_who`'s actual result-set columns, `sys.dm_os_sys_info`'s real DMV shape, `sp_add_job`/`sp_add_schedule` correctly classified as `no_result_set`.
3. Confirmed object counts increase monotonically with version (2017: 223 master operations → 2019: 237 → 2022: 246 → 2025: 263), consistent with each release only adding to the curated surface.
4. Confirmed security schemes differ correctly by version (`azureADAuth` present only in 2022/2025 output).

## Rewrite (2026-07-25): operations extraction rebuilt from scratch

Requested from scratch, based on three prior research conversations (T-SQL
`SET FMTONLY ON`/`sp_describe_first_result_set`/`SET NOEXEC`; a cross-database
common/exclusive object query for `master`/`msdb`/`model` via
`sys.all_objects`; and extended-stored-procedure parameter discovery,
including `is_ms_shipped`'s meaning). Scope and ranking were confirmed with
the user before implementation (broad `is_ms_shipped` sweep over the curated
allowlist; metadata-completeness-first ranking; live re-extraction across all
four versions) — see the session transcript for the full exchange. Changes
from the original design above:

1. **`sandbox` → `model`.** `model` is a real, always-present SQL Server
   system database (the template new user databases are created from) — the
   closest thing to a generic "non-system user database" guaranteed to exist
   on every instance, and requires no `CREATE DATABASE` step unlike the old
   ad hoc `sandbox` placeholder.

2. **Curated allowlist dropped for a broad `is_ms_shipped = 1` sweep.**
   `sql/eda/allowlist.yaml`/`allowlist_names.sql`/`allowlist_patterns.sql`
   are gone. `objects.sql`/`params.sql`/`resultset.sql`/`version_diff.sql`
   now match every system-shipped procedure/function/view directly, across a
   fixed object-type set (`P`, `PC`, `X`, `FN`, `IF`, `TF`, `FS`, `FT`, `V`).
   This turns up far more candidates than are useful (SQL Server ships
   thousands of internal system objects) — hence ranking + a top-500 cutoff,
   see below.

3. **`SET FMTONLY ON` fallback (`sql/eda/resultset_fmtonly.sql`, new).**
   `resultset.sql`'s primary method (`sys.dm_exec_describe_first_result_set`,
   pure static analysis) still fails for some objects (e.g. result sets built
   from temp tables). For those, a second pass attempts `SET FMTONLY ON`,
   which genuinely compiles/partially runs the statement without altering
   data (Microsoft's own documented guarantee) — deliberately excluded for
   extended stored procedures (type `X`) since that guarantee doesn't cover
   what a compiled DLL's code might actually do. There is no server-side way
   to capture FMTONLY's column metadata (it's a wire-protocol response to the
   connecting client, not queryable from within the T-SQL session), so this
   script leans on sqlcmd's own text rendering of the (empty, zero-row)
   result set's header line, split on whitespace for column names (SQL
   Server identifiers can never contain spaces) — names only, no types,
   tagged `x-sql-columns-source: fmtonly` downstream. Verified live against a
   2025 container before scaling out: discovered mid-implementation that the
   `-y 0 -Y 0` flags used elsewhere in the pipeline (to avoid truncating the
   giant FOR JSON strings) unexpectedly suppress sqlcmd's header/dashes
   rendering entirely for this script's plain-text output, so
   `resultset_fmtonly.sql`'s `sqlcmd` invocation deliberately omits them.

4. **Extended stored procedures with no introspectable data are dropped
   entirely**, not hand-curated. The previous `CURATED_PARAMETERS` dict
   (Microsoft-Learn-sourced signatures for `sp_executesql`/`sp_prepare`/
   `sp_execute`/`sp_unprepare`/`sp_describe_first_result_set`/
   `sp_describe_undeclared_parameters`/`sp_set_session_context`) is removed.
   If neither `sys.dm_exec_describe_first_result_set` nor FMTONLY recovered
   any real columns for an `EXTENDED_STORED_PROCEDURE`-typed object with zero
   `sys.all_parameters` rows, `tools/generate_openapi.py`'s
   `build_operations()` leaves it out of the generated spec rather than
   emitting it parameterless/columnless.

5. **One path per operation, not one per database.** The old design
   database-prefixed every path/operationId (`/master/sys/sp_who`,
   `/msdb/sys/sp_who`, ...) specifically so three per-database specs could
   merge without colliding. `tools/generate_openapi.py` now loads
   `master`/`msdb`/`model` together and deduplicates objects that appear
   identically in more than one (by schema+name+type) into a single `/
   <schema>/<name>` operation, recording which database(s) it was found in
   on `x-sql-databases` (a list). `tools/merge_openapi.py` is deleted —
   there's nothing left to merge.

6. **Ranked, capped to the top 500 operations per version.** Primary sort:
   cross-version presence — added mid-session at the user's request ("keep
   the top 500 as similar as possible on all versions so that workflows work
   on all versions"), via `compute_presence()` scanning every extracted
   version's `data/<version>/*.objects.json` for the same schema+name+type
   identity, so a prompt/workflow built against one version's tool set keeps
   working when switched to another. Secondary: metadata completeness (real
   params + real columns > columns-only > params-only > neither). Tertiary:
   object-type tier (procs > functions > views). Quaternary: alphabetical.
   This only works as intended when every version has already been extracted
   before generation runs for any of them —
   `scripts/regenerate_mcp_server.sh` was reordered/commented accordingly.

7. **Synthetic `execution_database` request parameter on every operation.**
   Deduplication means one operation can now represent the same object in
   more than one database, so which one to actually hit against has to be
   resolved per call rather than baked into the operation at generation
   time. Every generated operation gets an optional `execution_database`
   request-body property (`tools/generate_openapi.py`'s
   `build_request_schema()`) — real request-schema property, not a
   `x-sql-...` vendor extension, because mcpify's generated store doesn't
   carry vendor extensions through into the schemas it validates calls
   against. Resolution order at call time
   (`services::api_client::resolve_execution_database`): the caller's
   `execution_database` body value, else the operator's configured
   `Config::default_database` (new, hand-added field — optional, settable at
   `sqlserver-mcp setup` via a new wizard prompt, or via
   `SQLSERVER_DEFAULT_DATABASE`/`default_database` config), else the live
   connection's own current database context (`DB_NAME()`/`DB_ID()`) — no
   qualification at all, two-part `schema.name`. This subsumes and
   generalizes the old design's `sandbox`-only `database` override argument
   (which was an undocumented sibling of `body`, not a real schema property,
   and only applied to `sandbox`-tagged operations — `master`/`msdb`
   operations always went out three-part-qualified with a literal database
   name before). `endpoint.path` accordingly shrank from
   `/<db>/<schema>/<name>` to `/<schema>/<name>`
   (`services::api_client::parse_path`), and `build_statement` no longer
   takes a `db` argument at all — every operation now behaves the way only
   `sandbox`-tagged ones used to.

## Verification performed (rewrite)

1. Smoke-tested the FMTONLY text-capture approach live against a 2025
   container before writing `resultset_fmtonly.sql`'s Python parser —
   confirmed sqlcmd does print a header + dashes line for a zero-row FMTONLY
   result set, but only without `-y 0`/`-Y 0` (see item 3 above).
2. `cargo check --all-targets` and `cargo test --lib services::api_client`
   (23/23 passing) after the `parse_path`/`build_statement`/
   `resolve_execution_database` rewrite, before re-running the live
   extraction+generation+`mcpify sync` pipeline end-to-end across all four
   versions.
3. Full live extraction across all four versions (`master`/`msdb`/`model`),
   using the fixed scripts. Candidates after dedup, before the top-500 cut:
   2017: 2,647 (186 extended procs dropped); 2019: 2,713 (208 dropped);
   2022: 2,815 (298 dropped); 2025: 2,885 (358 dropped) — consistent with
   each release only adding to the system-shipped surface, same monotonic
   pattern as the original implementation's curated-allowlist numbers.
4. Cross-version presence ranking achieved its goal: of each version's 500
   generated operations, 495 are identical across all four versions (only 5
   version-specific slots each) -- verified by loading all four
   `combined.yaml` files and diffing their path sets.
5. All four `openapi/<version>/combined.yaml` files validate cleanly with
   `openapi-spec-validator`.
6. **`mcpify sync --manifest mcpify.yaml` is the wrong command for this kind
   of update and must not be used for routine catalog refreshes** -- unlike
   `mcpify add-version --force` (used successfully in the July 21 commit
   above, touching only 6 files: `.mcpify/versions.json`, the 4 `mcp_store*
   .db.zst` binaries, and a 10-line `src/data/store.rs` diff), `sync`
   regenerates the *entire* project's scaffolding from the manifest as if
   from a blank slate, discovered live this session: it reset
   `AuthMethod`/`Config`/the setup wizard/`api_client.rs` to mcpify's
   generic Basic/OAuth2/reqwest-HTTP template (losing the hand-rewritten
   TDS/SQL-Server-specific auth and transport layer entirely) and created
   several brand-new generic scaffolding files that don't belong in this
   project (`src/auth/strategies/basic.rs`, `src/core/api_url_builder.rs`,
   `src/http/auth_extractor.rs`, an `examples/` directory, ...) -- mcpify's
   auth-scheme classifier has *never* been able to derive Rust variant names
   like `SqlServer`/`Windows`/`AzureAd` from spec scheme keys like `sqlAuth`
   (confirmed by reading mcpify's own source,
   `src/targets/rust/context.rs`'s `auth_method_variant_name` — it's a
   closed match over 5 generic literals, not name-derived at all), so this
   project's entire custom auth naming has always been a hand-edit
   reapplied after generation, but `sync` goes much further than just that
   by also regenerating dozens of other hand-tailored files back to generic
   HTTP-client scaffolding. Recovered by reverting every touched file to
   `HEAD` (git had zero uncommitted changes before this session started),
   deleting the new generic files, and reapplying this session's actual
   intended hand-edits from a saved patch, then using `mcpify add-version
   --project . --version <v> --input openapi/<v>/combined.yaml --force`
   (`--set-default` for 2025) per version instead -- confirmed to touch only
   the expected files (version ledger, that version's compiled store, that
   version's schema bundle, plus the pre-existing `// mcpify:versions:...`
   marker regions). `scripts/regenerate_mcp_server.sh` still documents
   `sync`; it should be corrected to use `add-version --force` per version
   in a follow-up, since as written it would reproduce this exact regression
   for anyone who actually runs it.
7. `cargo build --all-targets` and `cargo test` (118 tests: 112 lib unit
   tests + 1 main.rs test + 5 CLI integration tests + 1 explicitly-run
   `--ignored` embeddings test) all pass after recovery.
   `./target/release/sqlserver-mcp-populate-embeddings --all` repopulated
   `semantic_endpoints` for all 4 versions (500 rows each, matching
   `endpoints`).
8. `scripts/coverage.sh` (`cargo llvm-cov`): **65.01% total line coverage**,
   short of the project's 85% target. This is a pre-existing gap, not one
   this rewrite introduced: every 0%-covered file (`tools/call_tool.rs`,
   `tools/get_tool.rs`, `tools/search_tool.rs`, `services/sql_pool.rs`,
   `services/embedding_service.rs`, `cli/setup_wizard.rs`,
   `bin/populate_embeddings.rs`, `core/otel.rs`, ...) is either untouched by
   this session's changes or is interactive/live-connection code this
   project has never had unit tests for. The one file this rewrite changed
   most heavily, `services/api_client.rs`, sits at 77.46% line coverage on
   its own. Closing the project-wide gap to 85% is a separate, substantially
   larger testing-infrastructure effort (mocking or live-instance testing
   for the tool-dispatch/connection-pooling/embedding-service/CLI-wizard
   layers) than this rewrite's scope.
