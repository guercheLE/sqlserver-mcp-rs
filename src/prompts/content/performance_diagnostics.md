# Guided workflow: performance and diagnostics

This is a thin pointer to the right read-only signal, not a multi-step flow —
match the symptom to a search, read the result, and report back; don't
hardcode an `operationId`, since availability differs across the four
supported engine versions (2017/2019/2022/2025). For **blocking or a stuck
query**, fetch the dedicated `sqlserver-blocking-and-locks` prompt
rather than improvising here — it has the full diagnose-then-confirm-before-
killing flow. For **high CPU or general slowness**, search for
wait-statistics and scheduler/CPU DMVs (`dm_os_*`) to see what the engine is
spending time on. For **a specific slow query**, search for
query-stats/execution-plan DMVs (`dm_exec_*`) to find its plan and resource
cost — if the plan points at a missing index, fetch
`sqlserver-index-tuning-recommendations` for the guided path from
there to an actual fix. For **active transactions**, search for transaction
DMVs (`dm_tran_*`). For **I/O bottlenecks**, search for I/O statistics DMVs
(`dm_io_*`). For **resource governor limits**, search for
`dm_resource_governor_*`. For **In-Memory OLTP (memory-optimized tables) or
columnstore-index health**, search for the `dm_db_xtp_*` / `dm_db_column_store_*`
DMV families — these only return rows if the database actually uses those
features. For **general server/OS-level pressure** (memory, scheduler,
recent out-of-memory events), search for `dm_os_sys_info`, `dm_os_ring_buffers`
(a rolling log of internal diagnostic events, including OOM conditions), and,
**on newer engine versions only**, a dedicated memory-health-history view — if
searching for it comes up empty, fall back to `dm_os_ring_buffers`. A quick
session/connection overview is also available via a `sp_who`/`sp_who2`-style
lookup if a DMV feels like overkill for a fast "who's connected right now"
check.
