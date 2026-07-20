# Guided workflow: schema exploration

Discovering what exists in a SQL Server instance — databases, schemas,
tables, views, columns, types, triggers — is a single search-then-call
action per question, not a multi-step ordered flow, so this prompt is a
pattern reference rather than a numbered procedure.

**Agnostic phrasing rule**: never call a hardcoded `operationId`. Search for
the capability you need (e.g. "search for how to list tables in a database",
"search for how to list columns of a table"), then read the schema `get`
currently returns before relying on any parameter or result-column name —
object availability can differ across the four supported engine versions
(2017/2019/2022/2025).

## Two equivalent naming conventions

The catalog exposes the same schema metadata two ways:

- **`sys.*` catalog views** (`sys.tables`, `sys.columns`, `sys.views`,
  `sys.types`, `sys.schemas`, `sys.triggers`, ...) — SQL Server-native, more
  detail (e.g. object IDs, `is_ms_shipped`).
- **`INFORMATION_SCHEMA.*` views** (`TABLES`, `COLUMNS`, `VIEWS`, `ROUTINES`,
  ...) — ANSI-standard, portable naming, less SQL-Server-specific detail.

Search for either by name if the user has a preference; otherwise prefer
`sys.*` when the question needs SQL-Server-specific detail (object IDs,
`is_ms_shipped`, filegroup info) and `INFORMATION_SCHEMA.*` for a plain
inventory question.

## Typical questions and what to search for

- "What databases/schemas exist?" → `sys.databases` / `sys.schemas`.
- "What tables/views exist in a schema?" → `sys.tables` / `sys.views` (or
  `INFORMATION_SCHEMA.TABLES` filtered by `TABLE_TYPE`).
- "What columns does this table have, and their types?" → `sys.columns`
  joined conceptually with `sys.types`, or `INFORMATION_SCHEMA.COLUMNS`.
- "What does this stored procedure/view's definition look like?" → search
  for a proc/module-definition lookup (`sql_modules` or `sp_helptext`).
- "What triggers exist on this table?" → `sys.triggers`.

## Composing with other workflows

Indexes, foreign keys, and check constraints on a specific table are covered
in more depth by `sqlserver-indexes-constraints` — fetch that prompt
rather than duplicating its guidance here.
