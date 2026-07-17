# SQL Server Multi-Version EDA → OpenAPI Pipeline

Runs SQL Server 2017, 2019, 2022, and 2025 in Docker, introspects each
instance's own system catalog for a curated set of well-known stored
procedures, functions, DMVs/DMFs, and catalog views, and emits a synthetic
OpenAPI 3.1 YAML file per version/database describing their inputs and
outputs.

This whole directory (`docs/sqlserver-eda-openapi-pipeline/`) is a
self-contained unit — all commands below assume your shell's current
directory is *this* directory, not the repo root. See `plan.md` in this same
folder for the full design rationale and history of what was requested.

## Prerequisites

- Docker Desktop (or compatible), with `docker compose` v2.
- No host-side `sqlcmd` needed — `scripts/extract.sh` and `scripts/diff_versions.sh` run
  `sqlcmd` *inside* each container (it ships in the official image) via `docker exec`/`docker cp`.
- Python 3.10+ with `pip install -r tools/requirements.txt` (a `.venv` works fine).

On Apple Silicon, SQL Server 2017 and 2019 have no native arm64 image and run
under x86-64 emulation (`platform: linux/amd64` in `docker-compose.yml`).
Expect container startup to take several minutes and CPU use to be high while
they're running.

## Setup

```bash
cp .env.example .env        # edit MSSQL_SA_PASSWORD to a strong password
set -a; source .env; set +a  # export MSSQL_SA_PASSWORD into your shell
```

## Running the pipeline for one version

```bash
scripts/up.sh 2022           # bring up the container, wait for healthy
scripts/extract.sh 2022      # dump master/msdb/sandbox EDA JSON to data/2022/
.venv/bin/python tools/generate_openapi.py 2022 master
.venv/bin/python tools/generate_openapi.py 2022 msdb
.venv/bin/python tools/generate_openapi.py 2022 sandbox
.venv/bin/openapi-spec-validator openapi/2022/master.yaml
scripts/down.sh 2022
```

Repeat for `2017`, `2019`, `2025`. Running all four containers at once is
possible if the host has the RAM/CPU for it (`docker compose up -d` with no
service name brings up everything), but the scripts are written to be run
one version at a time to keep resource use predictable under emulation.

## Comparing what changed across versions

```bash
scripts/diff_versions.sh master
```

Runs `sql/eda/version_diff.sql` against every currently-healthy container and
prints pairwise diffs of the matched object list, so newly-added or
version-specific DMVs/procs surface directly instead of being inferred from
documentation.

## Repository layout

- `docker-compose.yml` — one service per SQL Server version, Developer edition, distinct host ports (14330–14333).
- `scripts/up.sh` / `down.sh` — start/stop one version and wait for its healthcheck.
- `scripts/extract.sh` — run the EDA SQL scripts against a running container for all three target databases.
- `scripts/diff_versions.sh` — cross-version object-list diff.
- `scripts/regenerate_mcp_server.sh` — regenerates every version's per-database
  OpenAPI specs (`tools/generate_openapi.py`) and merged spec
  (`tools/merge_openapi.py`), then re-syncs the generated Rust MCP server
  at the repo root (`mcpify sync`). Only needs `data/<version>/` (already
  extracted) — no live SQL Server/Docker required. See the script's own
  header comment for an important caveat: `mcpify sync` fully regenerates
  several hand-edited source files, so commit or stash first.
- `sql/eda/allowlist.yaml` — human-readable curated list of objects/patterns in scope (source of truth).
- `sql/eda/allowlist_names.sql`, `allowlist_patterns.sql` — the same list, duplicated in SQL because `sqlcmd` has no YAML support. Keep these in sync with the YAML by hand.
- `sql/eda/objects.sql` — which allowlisted objects actually exist in the current database. Every script under `sql/eda/` opens with `USE $(db);`, so the active database is a required `sqlcmd` scripting variable (`-v db=<name>`) made explicit in the script text — not just an invisible `-d` connection flag. `scripts/extract.sh`/`scripts/diff_versions.sh` already pass it; running one of these files directly with `sqlcmd -i` requires `-v db=<master|msdb|sandbox|...>` too, or it fails fast instead of silently querying the wrong database.
- `sql/eda/params.sql` — parameter metadata (name/type/direction/default) for matched objects.
- `sql/eda/resultset.sql` — best-effort result-set column introspection via `sys.dm_exec_describe_first_result_set`.
- `sql/eda/version_diff.sql` — same object match, plain-text output for diffing across versions.
- `tools/generate_openapi.py` — JSON dumps → `openapi/<version>/<database>.yaml`.
- `tools/merge_openapi.py` — merges one version's `master`/`msdb`/`sandbox.yaml` into `openapi/<version>/combined.yaml`, prefixing `path`/`operationId` with the database name (see the script's own module docstring for why) — the spec `../../mcpify.yaml` actually feeds to `mcpify sync`.
- `data/<version>/` — raw JSON dumps from `scripts/extract.sh` (gitignored).
- `openapi/<version>/<database>.yaml` — generated output.

## OpenAPI mapping convention

SQL objects aren't HTTP resources, so the mapping is intentionally synthetic:

- Each documented object becomes `POST /<schema>/<name>`.
- Input parameters (`is_output = 0`) become the JSON request body schema.
- Output parameters (`is_output = 1`) are documented separately as an
  `X-Output-Parameters-Schema` response header schema, since OpenAPI request/
  response bodies don't model SQL's OUTPUT-parameter semantics directly.
- The result set, when `sys.dm_exec_describe_first_result_set` could describe
  it, becomes the `200` response body schema (an array of row objects).
- `operationId` is `<schema>_<name>`; `summary` comes from a hand-curated
  one-line description in `tools/generate_openapi.py` for well-known objects
  (system catalog metadata has no reliable description field to pull this
  from automatically).
- Every property in every generated schema carries an `x-sql-type` field with
  the exact SQL Server type text (e.g. `nvarchar(256)`, `decimal(18,2)`,
  `datetime2(7)`), in addition to the best-effort OpenAPI `type`/`format`.
  OpenAPI's type system is too coarse to round-trip SQL Server types on its
  own — every string-like type maps to `type: string`, every exact/approximate
  numeric type maps to `type: number` — so `x-sql-type` is the field to read
  when generating actual SQL parameter bindings or column definitions from
  the spec, rather than trying to reverse-engineer the type from `type`/`format`.
- Each file's top-level `security` + `components.securitySchemes` document the
  TDS-protocol authentication modes that engine *version* accepts for a
  connection (this isn't a real HTTP auth flow -- it's the closest-fitting
  OpenAPI shape for "what credentials can a client present"). `security` is a
  list of independent single-scheme entries, which OpenAPI resolves as
  "satisfies ANY one of these":
  - `sqlAuth` (`http`/`basic`) -- SQL Server Authentication (a SQL login,
    username + password). All four versions, but only usable when the
    instance is configured for Mixed Mode.
  - `windowsAuth` (`http`/`negotiate`) -- Windows Authentication / Integrated
    Security (Kerberos or NTLM passthrough). All four versions, though a
    Linux container needs explicit Kerberos configuration (keytab +
    `krb5.conf`) to actually honor it.
  - `azureADAuth` (`oauth2`) -- Azure Active Directory (Microsoft Entra ID)
    authentication. **Only on 2022 and 2025** -- this was a new *engine*
    feature introduced in SQL Server 2022 for on-premises/Linux instances
    (Azure SQL Database/Managed Instance had it earlier, but that's a
    different product); 2017 and 2019 cannot authenticate this way at all.
- Every operation also carries explicit `x-sql-database` and `x-sql-schema`
  fields (e.g. `master` / `sys`), duplicating what's already encoded in the
  path and `operationId` — so tooling can read the schema/database directly
  instead of having to parse it back out of a path string.
- Every operation documents `400`/`403`/`500` responses in addition to `200`,
  all sharing one `components.schemas.SqlServerError` schema (`number`,
  `severity`, `state`, `procedure`, `line`, `message` — the fields available
  from `THROW`/`RAISERROR`/`ERROR_NUMBER()`/etc.). SQL Server errors are TDS
  errors, not HTTP statuses, so this mapping is synthetic like the rest of
  the convention — but it follows the engine's own severity levels, which
  already distinguish "the caller can fix this" from "the server is broken":
  - **400** — severity 11–16: constraint violations, conversion errors,
    invalid object/column names, an explicit `RAISERROR`/`THROW` at the
    default severity 16.
  - **403** — severity-14 errors specifically about authorization (Msg
    229/230, `"... permission was denied on the object ..."`), split out
    from the rest of severity 14 (400) because it's a distinct failure mode
    a caller needs to handle differently (request access vs. fix input).
  - **500** — severity 17–25: resource exhaustion, hardware/software faults,
    fatal errors that tear down the connection — not fixable by resubmitting.
  - `THROW` with no arguments (re-raising a caught error) preserves the
    original error's number/severity/state, so it lands in whichever bucket
    the original error was already in — there's no separate "rethrow" case.

## Known limitations

- **Extended stored procedures (`xp_*`)** are compiled DLLs with no queryable
  parameter or result metadata (e.g. `xp_cmdshell`, `xp_readerrorlog`). They
  are excluded from the curated allowlist; if you need them documented,
  source their signatures from Microsoft Learn by hand rather than trying to
  introspect them live.
- **Core engine procs with no catalogued parameters**: `sp_executesql`,
  `sp_prepare`, `sp_execute`, `sp_unprepare`, `sp_describe_first_result_set`,
  `sp_describe_undeclared_parameters`, and `sp_set_session_context` are
  `EXTENDED_STORED_PROCEDURE`s internally, just like `xp_*` procs — `sys.
  all_parameters` has zero rows for them, and `sp_help` doesn't recover
  anything either (confirmed live: `sp_help` reads the same catalog). Unlike
  `xp_*`, these seven are important enough that `tools/generate_openapi.py`'s
  `CURATED_PARAMETERS` dict hand-documents their signatures instead of
  leaving them parameterless, verified against Microsoft Learn. Every
  hand-curated schema is tagged `x-sql-params-source: hand-curated` (absent
  = live-introspected) so a spec reader can tell which is which. A few of
  these procs also take variadic parameters whose names/types are declared
  dynamically at call time by another parameter's string value (`@param1..N`
  in `sp_executesql`, `bound_param` in `sp_execute`) — those aren't
  statically enumerable in OpenAPI and are called out in the schema's
  `description` instead.
- **Conditional/dynamic result sets**: some procedures return different
  shapes depending on their arguments (`sp_help` is the classic example, and
  many `sp_help*` procs behave similarly). `resultset.sql` describes the
  *default* invocation only; a `result_set_status: "unknown"` in the output
  means introspection failed for that specific call, not necessarily that the
  object has no result set — check `error_message` for why.
- **Permissions and session state**: `sys.dm_exec_describe_first_result_set`
  runs inside the same session as the extraction script and can fail for
  procs needing elevated permissions, a specific `USE` context, or an active
  transaction. These also come back as `"unknown"`.
- **2017/2019 emulation**: running under `linux/amd64` emulation on Apple
  Silicon is slow and occasionally flaky. If a container fails its
  healthcheck, check `docker logs mssql2017` / `mssql2019` before assuming the
  extraction itself is broken.
- **`allowlist.yaml` vs. the `.sql` copies**: the human-readable YAML and the
  `sqlcmd`-loadable SQL fragments are maintained by hand in parallel. If you
  add an object to one, add it to the other.
