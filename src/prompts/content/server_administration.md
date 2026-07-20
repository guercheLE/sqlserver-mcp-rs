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
  for linked-server operations. Only add and list are available as named
  operations in this catalog — there is no dedicated drop operation.
  Removing a linked server needs a raw `DROP SERVER` statement executed via
  `sp_executesql` (search for it — it accepts an arbitrary T-SQL batch, so
  it's the general escape hatch whenever a named operation doesn't exist).
  **Confirm the exact statement with the user before executing anything
  through it** — it isn't limited to reads.

## Composing with other workflows

Login/role/permission changes belong to `sqlserver_workflow_security_provisioning`;
SQL Agent job/schedule management belongs to `sqlserver_workflow_sql_agent_jobs`
— fetch those prompts rather than duplicating their guidance here.
