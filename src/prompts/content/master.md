# SQL Server operational workflows — start here

This server exposes exactly 3 generic tools — `search`, `get`, `call` — over a
curated SQL Server catalog (system stored procedures, DMVs/DMFs, and catalog
views). The prompts below sequence those tools into guided, multi-step
workflows for common operational tasks, so you don't have to re-derive the
right order of calls, the gotchas, and the verification steps from scratch.

## Menu

| Prompt | Use when the goal is... |
|---|---|
| `sqlserver_workflow_schema_exploration` | Discovering what databases/schemas/tables/views/columns exist |
| `sqlserver_workflow_indexes_constraints` | Inspecting a table's indexes, foreign keys, or check constraints |
| `sqlserver_workflow_security_provisioning` | Creating a login, granting database access, or managing role membership |
| `sqlserver_workflow_sql_agent_jobs` | Scheduling or managing a SQL Agent job |
| `sqlserver_workflow_server_administration` | Server/database config, renaming objects, disk usage, linked servers |
| `sqlserver_workflow_performance_diagnostics` | Diagnosing high CPU, slow queries, I/O, or resource limits |
| `sqlserver_workflow_blocking_and_locks` | Diagnosing a blocked/stuck query, and — only with explicit confirmation — killing the session causing it |
| `sqlserver_workflow_index_tuning_recommendations` | Finding and, with explicit confirmation, creating a missing index |

## Routing

1. Match the user's goal (or `goal`, if already supplied above) to exactly one
   row in the menu. Ask a clarifying question if more than one plausibly
   applies, or none do.
2. **Delegate the whole matched sub-workflow if your environment supports
   running an isolated sub-task** (an agent/task tool): hand the sub-task the
   matched prompt's name plus any parameters already known, let it fetch that
   prompt itself and carry out every step — including all of its own
   `search`/`get`/`call` traffic — in its own context, and have it report
   back only a short summary (what was accomplished/confirmed, and anything
   still needed from the user). This is what keeps a workflow's full tool-call
   trace out of this conversation.
3. If no such delegation mechanism is available, fetch the matched prompt
   yourself and carry out its steps directly here.

Every sub-workflow prompt is self-contained: don't read this menu's
description as the full instructions — fetch the matched prompt for the real
steps, gates, and forks.
