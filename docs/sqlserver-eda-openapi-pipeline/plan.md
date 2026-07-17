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

## Verification performed

1. Brought up each of the 4 containers in turn (`scripts/up.sh <version>`), ran the full extraction (`scripts/extract.sh <version>`) across `master`/`msdb`/`sandbox`, generated OpenAPI (`tools/generate_openapi.py <version> <db>`), and validated every output file with `openapi-spec-validator`. All 12 files (3 databases × 4 versions) validate cleanly.
2. Spot-checked real generated content: `sp_who`'s actual result-set columns, `sys.dm_os_sys_info`'s real DMV shape, `sp_add_job`/`sp_add_schedule` correctly classified as `no_result_set`.
3. Confirmed object counts increase monotonically with version (2017: 223 master operations → 2019: 237 → 2022: 246 → 2025: 263), consistent with each release only adding to the curated surface.
4. Confirmed security schemes differ correctly by version (`azureADAuth` present only in 2022/2025 output).
