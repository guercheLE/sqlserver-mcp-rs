# SQL Server Multi-Version EDA → OpenAPI Pipeline

Runs SQL Server 2017, 2019, 2022, and 2025 in Docker, introspects each
instance's own system catalog for every system-shipped (`is_ms_shipped = 1`)
stored procedure, function, and view across the `master`, `msdb`, and
`model` databases, deduplicates objects that appear identically in more than
one of those databases, ranks the result, and emits one synthetic OpenAPI
3.1 YAML file per version (`openapi/<version>/combined.yaml`) describing the
survivors' inputs and outputs.

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
scripts/extract.sh 2022      # dump master/msdb/model EDA output to data/2022/
.venv/bin/python tools/generate_openapi.py 2022
.venv/bin/openapi-spec-validator openapi/2022/combined.yaml
scripts/down.sh 2022
```

Repeat for `2017`, `2019`, `2025`. **Extract every version before generating
any of them**: `tools/generate_openapi.py`'s ranking prefers operations that
exist on every extracted version (see `compute_presence()` below) so the
top-500 cut lands on nearly the same operation set across versions — that
only works once `data/<version>/` has all four versions' output on disk.
`scripts/regenerate_mcp_server.sh` already runs generation in that order;
`docker compose up -d` (no service name) brings up all four containers at
once if the host has the RAM/CPU for simultaneous emulation.

## Comparing what changed across versions

```bash
scripts/diff_versions.sh master
```

Runs `sql/eda/version_diff.sql` against every currently-healthy container and
prints pairwise diffs of the matched object list for the given database
(`master`/`msdb`/`model`), so newly-added or version-specific objects surface
directly instead of being inferred from documentation.

## Repository layout

- `docker-compose.yml` — one service per SQL Server version, Developer edition, distinct host ports (14330–14333).
- `scripts/up.sh` / `down.sh` — start/stop one version and wait for its healthcheck.
- `scripts/extract.sh` — run the EDA SQL scripts against a running container for `master`/`msdb`/`model`.
- `scripts/diff_versions.sh` — cross-version object-list diff.
- `scripts/regenerate_mcp_server.sh` — regenerates every version's OpenAPI
  spec (`tools/generate_openapi.py`), then re-syncs the generated Rust MCP
  server at the repo root (`mcpify sync`). Only needs `data/<version>/`
  (already extracted, for all four versions) — no live SQL Server/Docker
  required. See the script's own header comment for an important caveat:
  `mcpify sync` fully regenerates several hand-edited source files, so
  commit or stash first.
- `sql/eda/objects.sql` — every system-shipped stored procedure/function/view
  (`is_ms_shipped = 1`) in the current database. There is no curated
  allowlist anymore — this is a full sweep, deduplicated and ranked
  downstream by `tools/generate_openapi.py`. Every script under `sql/eda/`
  opens with `USE $(db);`, so the active database is a required `sqlcmd`
  scripting variable (`-v db=<name>`) made explicit in the script text — not
  just an invisible `-d` connection flag. `scripts/extract.sh`/
  `scripts/diff_versions.sh` already pass it; running one of these files
  directly with `sqlcmd -i` requires `-v db=<master|msdb|model|...>` too, or
  it fails fast instead of silently querying the wrong database.
- `sql/eda/params.sql` — parameter metadata (name/type/direction/default) for matched objects.
- `sql/eda/resultset.sql` — best-effort, execution-free result-set
  introspection via `sys.dm_exec_describe_first_result_set`, with every
  declared parameter passed as `NULL` so the query processor has a
  syntactically complete statement to analyze.
- `sql/eda/resultset_fmtonly.sql` — fallback column-*name* discovery via
  `SET FMTONLY ON` for objects `resultset.sql` couldn't describe. Excludes
  extended stored procedures (type `X`) entirely for safety — see the
  script's own header comment. Its output is sqlcmd's own text rendering
  (there is no server-side way to capture FMTONLY's column metadata; it's a
  wire-protocol response to the connecting client), not JSON.
- `sql/eda/version_diff.sql` — same object sweep, plain-text output for diffing across versions.
- `tools/generate_openapi.py` — `data/<version>/*.{json,txt}` → `openapi/<version>/combined.yaml`.
  Loads `master`/`msdb`/`model` together, deduplicates objects found
  identically in more than one, ranks by cross-version presence then
  metadata completeness, caps to the top 500, and adds the synthetic
  `execution_database` parameter to every operation. See the script's own
  module docstring and this file's "OpenAPI mapping convention" below.
- `data/<version>/` — raw EDA extraction output from `scripts/extract.sh`. Tracked in git (not gitignored, despite the size) so `tools/generate_openapi.py` can be re-run and cross-checked without a live SQL Server instance on hand.
- `openapi/<version>/combined.yaml` — generated output (one file per version; no more per-database files or a separate merge step).

## OpenAPI mapping convention

SQL objects aren't HTTP resources, so the mapping is intentionally synthetic:

- Each documented object becomes `POST /<schema>/<name>` — **one path per
  object, not per database**. Objects found identically in more than one of
  `master`/`msdb`/`model` (e.g. every database has its own
  `INFORMATION_SCHEMA.COLUMNS`) are deduplicated into a single operation;
  which database(s) it was actually found in is documented on
  `x-sql-databases` (a list) rather than baked into the path. This is a
  deliberate change from an earlier version of this pipeline, which
  database-prefixed every path (`/master/sys/sp_who`,
  `/msdb/sys/sp_who`, ...) specifically to keep per-database specs disjoint
  before merging — deduplication makes that unnecessary.
- Every operation carries an optional `execution_database` request-body
  property: which database to actually run it against. This is **not** a
  real SQL Server parameter of the underlying object — it never appears in
  `sys.all_parameters` for it — it's added after introspection by
  `tools/generate_openapi.py` specifically because an operation can now be
  documented once but exist in more than one database. It has to be a real
  documented request property rather than an OpenAPI vendor extension,
  because mcpify's generated store does not carry vendor extensions through
  into the schemas it validates calls against (see
  `src/services/api_client.rs`'s own doc comments). If omitted, the
  generated MCP server falls back to the operator's configured
  `default_database` (optional, set during `sqlserver-mcp setup` or via the
  `SQLSERVER_DEFAULT_DATABASE` env var / `default_database` config key) and,
  failing that, the live connection's own current database context
  (equivalent to `DB_NAME()`/`DB_ID()`).
- Input parameters (`is_output = 0`) become the JSON request body schema,
  alongside `execution_database`.
- Output parameters (`is_output = 1`) are documented separately as an
  `X-Output-Parameters-Schema` response header schema, since OpenAPI request/
  response bodies don't model SQL's OUTPUT-parameter semantics directly.
- The result set becomes the `200` response body schema (an array of row
  objects) when it could be described. Two introspection methods are tried,
  in order, and the schema records which one actually produced it:
  1. `sys.dm_exec_describe_first_result_set` (`resultset.sql`) — pure static
     analysis, never executes anything, fully typed columns.
  2. `SET FMTONLY ON` (`resultset_fmtonly.sql`) — a fallback for objects the
     static method couldn't describe (e.g. result sets built from temp
     tables). Genuinely compiles/partially runs the statement without
     altering data, per Microsoft's own documented FMTONLY behavior — so
     it's skipped entirely for extended stored procedures (type `X`, opaque
     compiled DLLs like `xp_cmdshell`), whose actual runtime behavior isn't
     something the query processor's "no data altered" guarantee covers.
     Only column *names* are recoverable this way (sqlcmd's text renderer
     carries no type information for a header row), tagged
     `x-sql-columns-source: fmtonly` on the response schema so a reader
     knows these are name-only, unlike method 1's fully-typed output.
  If neither method produced anything, `objects with EXTENDED_STORED_PROCEDURE`
  type are **dropped from the generated spec entirely** rather than emitted
  parameterless/columnless (see "Known limitations" below) — every other
  object type is still emitted, with the response schema documenting
  whichever introspection outcome it actually got (including "unknown").
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
- Every operation also carries an explicit `x-sql-databases` (plural — see
  above) and `x-sql-schema` field, duplicating what's already encoded in the
  path/`x-sql-databases`/`operationId` -- so tooling can read the
  schema/found-in-databases directly instead of having to parse it back out
  of a string.
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

## Ranking and the top-500 cutoff

The broad `is_ms_shipped` sweep this pipeline runs turns up far more
candidate objects than are useful to expose as MCP tools (SQL Server ships
thousands of internal system objects across `master`/`msdb`/`model`).
`tools/generate_openapi.py` ranks every deduplicated candidate and keeps only
the top 500 per version (`TOP_N_PER_VERSION`), in this order:

1. **Cross-version presence** (`compute_presence()`) — an operation that
   exists (by schema/name/type identity) on every version whose data has
   been extracted ranks ahead of one that exists on fewer. This is what
   keeps the generated tool set stable across versions, so a prompt/workflow
   written against one version keeps working when the operator switches to
   another.
2. **Metadata completeness** — has both real introspected parameters and
   real result-set columns > has columns only > has parameters only >
   neither.
3. **Object-type tier** — stored procedures > functions > views.
4. **Alphabetical** (schema, then name) — a stable tiebreak among otherwise
   equally-ranked operations.

Cross-version presence only works as intended when all four versions have
already been extracted before generation runs for any of them — see "Running
the pipeline for one version" above.

## Known limitations

- **Extended stored procedures (`xp_*`, and a handful of core-engine procs
  that are internally the same kind of object — `sp_executesql`,
  `sp_prepare`, `sp_execute`, `sp_unprepare`, `sp_describe_first_result_set`,
  `sp_describe_undeclared_parameters`, `sp_set_session_context`)** are
  compiled DLLs with no queryable parameter metadata (`sys.all_parameters`
  has zero rows for them) and are deliberately never called under `SET
  FMTONLY ON` (unlike regular T-SQL objects, FMTONLY's "no data altered"
  guarantee doesn't cover what a compiled extended proc's code might
  actually do — see `sql/eda/resultset_fmtonly.sql`'s header comment). If
  neither `sys.dm_exec_describe_first_result_set` nor FMTONLY recovered any
  real columns for one either, it is **dropped from the generated spec
  entirely** rather than emitted parameterless/columnless — an earlier
  version of this pipeline hand-curated signatures for the seven core-engine
  procs above from Microsoft Learn; this rewrite removed that hand-curation
  in favor of only ever documenting what was actually introspected live. If
  you need one of these procs documented, source its signature by hand
  rather than trying to introspect it live.
- **Conditional/dynamic result sets**: some procedures return different
  shapes depending on their arguments (`sp_help` is the classic example, and
  many `sp_help*` procs behave similarly). `resultset.sql`/
  `resultset_fmtonly.sql` describe the *default* (all-parameters-NULL)
  invocation only; an "unknown" result means introspection failed for that
  specific call, not necessarily that the object has no result set.
- **Permissions and session state**: `sys.dm_exec_describe_first_result_set`
  and FMTONLY both run inside the same session as the extraction script and
  can fail for objects needing elevated permissions, a specific `USE`
  context, or an active transaction. These come back as "unknown".
- **FMTONLY column names only, no types**: there is no server-side way to
  capture FMTONLY's column metadata (it's a wire-protocol response to the
  connecting client, not something a T-SQL session can query back out of
  itself), so `resultset_fmtonly.sql`'s output is sqlcmd's own text
  rendering, parsed for header lines. Column *names* only — no type
  information — tagged `x-sql-columns-source: fmtonly` in the generated spec.
- **2017/2019 emulation**: running under `linux/amd64` emulation on Apple
  Silicon is slow and occasionally flaky. If a container fails its
  healthcheck, check `docker logs mssql2017` / `mssql2019` before assuming the
  extraction itself is broken.
- **Top-500 cutoff**: the broad sweep can find well over 500 candidate
  objects; anything beyond the rank cutoff (see "Ranking and the top-500
  cutoff" above) is silently absent from the generated spec, not just
  undocumented.
