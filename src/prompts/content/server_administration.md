# Guided workflow: server administration

Server/database configuration, renaming objects, disk-space usage, dependency
lookups, bulk per-table/per-database operations, and linked servers are each
a single search-then-call action, not a multi-step ordered flow.

**Agnostic phrasing rule**: never call a hardcoded `operationId`. Search for
the capability you need, then read the schema `get` currently returns before
relying on any parameter or result-column name — object availability can
differ across the four supported engine versions (2017/2019/2022/2025).

## Typical questions and what to search for

- "What's this server's configuration option set to (or how do I change
  it)?" → search for a server-configuration lookup/set operation.
- "What databases exist, and their state/recovery model?" → search for a
  database-info lookup.
- "How do I rename a table/column/object?" → search for a rename operation.
  **Confirm the exact object and its dependents with the user first** —
  renaming can silently break anything that references the old name.
- "How much space is a table/database using?" → search for a space-usage
  lookup.
- "What depends on this object (or what does it depend on)?" → search for a
  dependency lookup.
- "Run this against every table/database" → search for the bulk
  per-table/per-database operations. **These run their command once per
  matched object** — always confirm the exact command and scope (which
  tables/databases match) with the user before calling, especially for
  anything destructive.
- "How do I connect to another SQL Server instance from this one?" → search
  for linked-server operations (add/list/drop). **Confirm the exact server
  name with the user before calling the drop operation** — it isn't limited
  to reads, and removing a linked server can break anything still using it.
- This catalog has no general "run arbitrary T-SQL" operation — extended
  stored procedures like `sp_executesql` have no queryable parameter
  metadata and aren't part of the generated catalog (see
  docs/sqlserver-eda-openapi-pipeline/README.md's "Known limitations"). If a
  `search` for the specific action you need (rename, drop, configure, ...)
  comes up empty, don't assume there's an escape hatch — tell the user this
  action isn't available through this MCP server and needs to be run
  directly (SSMS/sqlcmd).

## Composing with other workflows

Login/role/permission changes belong to `sqlserver-security-provisioning`;
SQL Agent job/schedule management belongs to `sqlserver-sql-agent-jobs`
— fetch those prompts rather than duplicating their guidance here.
