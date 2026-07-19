# sqlserver-mcp

An MCP (Model Context Protocol) server exposing SQL Server's curated system
stored procedures, functions, DMVs/DMFs, and catalog views (master/msdb/
sandbox databases) — scaffolded by [mcpify](https://github.com/guercheLE/mcpify)
from a synthetic OpenAPI representation (see `docs/sqlserver-eda-openapi-pipeline/`),
then hand-wired to a real TDS connection (via [`tiberius`](https://github.com/prisma/tiberius))
instead of mcpify's default HTTP client, since these operations describe SQL
objects, not HTTP endpoints.

[![Sponsor](https://img.shields.io/github/sponsors/guerchele?label=Sponsor&logo=github&color=EA4AAA)](https://github.com/sponsors/guerchele)

Building and maintaining this took real ideation, time, design effort, and compute (including LLM usage) to get right. If it's useful to you, consider [sponsoring its development](https://github.com/sponsors/guerchele) — any amount helps keep it going. 💛

Exposes exactly 3 tools — `search`, `get`, `call` — backed by an embedded
semantic database (`mcp_store.db`, one per supported SQL Server version —
2017, 2019, 2022, and 2025; run `versions` or see
[`docs/SCHEMA_VERSIONS.md`](docs/SCHEMA_VERSIONS.md) for the full list),
so an LLM never needs the full catalog surface in context.

## Install

```bash
cargo build --release
```

## Setup

```bash
cargo run -- setup
```

Interactively collects the SQL Server host and the credentials your chosen auth method needs, then lets you persist them as a `.env` file, a `config.json` file, or a ready-to-run CLI invocation.

## Configuration

| Env var | Purpose |
|---|---|
| `SQLSERVER_URL` | SQL Server host, or `host:port`. |
| `SQLSERVER_AUTH_METHOD` | One of `sql_server` / `windows` / `azure_ad` — see `docs/sqlserver-eda-openapi-pipeline/README.md`'s `securitySchemes` documentation for what each instance/version supports. |
| `SQLSERVER_USERNAME` / `..._PASSWORD` | `sql_server`/`windows` auth — checked before the OS keychain/encrypted-file fallback. |
| `SQLSERVER_CLIENT_ID` / `..._CLIENT_SECRET` / `..._TENANT_ID` | `azure_ad` auth (client-credentials grant against `https://database.windows.net/.default`). |
| `SQLSERVER_SQL_PORT` | TDS port when `URL` has no `:port` suffix (default `1433`). |
| `SQLSERVER_TRUST_SERVER_CERT` | Trust the server's TLS cert without CA verification (default `true` — a local/dev instance's self-signed cert; set `false` for a production CA-signed cert). |
| `SQLSERVER_POOL_MAX_SIZE` | Max pooled SQL Server connections (default `10`). |
| `SQLSERVER_LOG_LEVEL` | Log verbosity (`trace`/`debug`/`info`/`warn`/`error`). |

See `.env.example` for the full list of supported variables.

Windows Authentication (`windows`) only works when this server itself runs on
Windows — `tiberius`'s NTLM/SSPI binding is a native-Windows feature; on
Linux/macOS (this pipeline's primary target, see `docs/sqlserver-eda-openapi-pipeline/docker-compose.yml`)
it fails with a clear error at connection time. Use `sql_server` or
`azure_ad` there instead.

## Usage

### Terminal Client (default)

```bash
sqlserver-mcp search "create an issue"
sqlserver-mcp get <operationId>
sqlserver-mcp call <operationId> --args '{"key":"value"}'
```

### Harness Server

```bash
sqlserver-mcp start                              # stdio transport (default)
sqlserver-mcp http --host 127.0.0.1 --port 3000  # HTTP transport
```

## Docker

```bash
# Stdio: the MCP client launches this one-off process and owns its stdin/stdout pipes
docker compose run --rm -T sqlserver-mcp

# HTTP: a long-running network endpoint published on http://localhost:3000
docker compose up sqlserver-mcp-http
```

Run these commands from the repository root. Docker Compose automatically discovers `docker-compose.yml`; `sqlserver-mcp` and `sqlserver-mcp-http` are service names inside that file, not filenames. Writing `docker compose -f docker-compose.yml ...` is equivalent, but `-f` is only needed when the file has another name or location, or when combining multiple Compose files.

Both services read configuration from a local `.env` file (copy `.env.example`) and persist credentials and configuration under `~/.sqlserver-mcp` on the host. For stdio, `-T` disables pseudo-TTY allocation so MCP JSON-RPC stays on raw stdin/stdout, and `--rm` removes the one-off container when the client exits.

Stdio is a process transport, not a listening service: the MCP client must start the server and communicate through that exact child process's stdin/stdout. This is useful when an MCP client is configured to launch `docker compose run --rm -T sqlserver-mcp`, in local scripts or CI that directly exchange MCP messages with the process, or in a custom image where your application launches the generated server's `start` subcommand as a child process. Merely putting the application and server in the same image—or starting the stdio container separately with `docker compose up`—does not connect their streams. One stdio server process normally serves one client. Use HTTP when independently started applications, multiple clients, another container, or a remote machine need to connect over the network.

## Testing

```bash
cargo test
```

## Coverage

```bash
bash scripts/coverage.sh   # writes target/coverage/html/index.html (requires cargo-llvm-cov)
```

## Profiling

```bash
bash scripts/profile.sh        # clean CPU profiling via samply
bash scripts/profile-heap.sh   # separate heap profiling via dhat-rs
```

Both scripts profile a repeated in-process `search "test query"` workload so
the one-time embedding-model startup cost is amortized. `profile.sh` uses 3
warmups and 250 measured iterations; `profile-heap.sh` uses 1 warmup and 5
measured iterations. Override these defaults with `PROFILE_QUERY`,
`PROFILE_WARMUPS`, `PROFILE_ITERATIONS`, `PROFILE_HEAP_WARMUPS`, and
`PROFILE_HEAP_ITERATIONS`.

The scripts default `SQLSERVER_URL=localhost` and
`SQLSERVER_AUTH_METHOD=sql_server` when those variables are unset. This is
safe for the profiling workload because `search` reads only the embedded
catalog and does not connect to SQL Server.

`profile/bottleneck-report.md` combines the largest coverage gaps with the
hottest CPU functions in one small text file. CPU profiling deliberately uses
an ordinary release build; DHAT allocator instrumentation is enabled only by
`profile-heap.sh`, so it cannot distort Samply's CPU samples. Requires
[samply](https://github.com/mstange/samply) (`cargo install samply`).

---

Generated by mcpify — do not hand-edit generated files; re-run mcpify against an updated OpenAPI spec instead.
