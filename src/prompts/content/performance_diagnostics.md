# Guided workflow: performance and diagnostics

This is a thin pointer to the right read-only signal, not a multi-step flow —
match the symptom to a search, read the result, and report back; don't
hardcode an `operationId`, since availability differs across the four
supported engine versions (2017/2019/2022/2025). For **blocking or a stuck
query**, search for active-requests/sessions and lock information (the
`dm_exec_*`/`dm_tran_*` DMV family, or a session-summary proc) to find the
blocking session and what it's waiting on. For **high CPU or general
slowness**, search for wait-statistics and scheduler/CPU DMVs
(`dm_os_*`) to see what the engine is spending time on. For **a specific
slow query**, search for query-stats/execution-plan DMVs (`dm_exec_*`) to
find its plan and resource cost. For **active transactions**, search for
transaction DMVs (`dm_tran_*`). For **I/O bottlenecks**, search for I/O
statistics DMVs (`dm_io_*`). For **resource governor limits**, search for
`dm_resource_governor_*`. A quick session/connection overview is also
available via a `sp_who`/`sp_who2`-style lookup if a DMV feels like overkill
for a fast "who's connected right now" check.
