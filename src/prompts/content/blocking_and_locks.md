# Guided workflow: diagnosing and resolving blocking

This sub-workflow is designed to be run as an isolated sub-task where
possible — if you were delegated here from `sqlserver`'s routing, or
your environment otherwise supports running this as its own sub-task,
everything you need is in this prompt's own text; report back only a short
summary when done rather than the full step-by-step trace.

**Agnostic phrasing rule**: never call a hardcoded `operationId`. Search for
the capability you need, then read the schema `get` currently returns before
relying on any parameter or result-column name — object availability can
differ across the four supported engine versions (2017/2019/2022/2025). Every
operation this workflow needs (active-requests/sessions/connections
listings, a session's last-statement lookup, and lock listings) is present
in all four versions.

## Step 1 — find the blocking chain

Search for how to list currently executing requests, and look for sessions
whose blocking-session column is non-zero — that column names the session
they're waiting on. If nothing is blocked, stop here and report that.

## Step 2 — walk the chain to the head blocker

A blocked session can itself be blocking another session. Follow the chain
(each session's blocker → that session's own blocker, and so on) until you
reach a session that isn't waiting on anyone else — that's the head blocker.
If several independent chains exist, they're safe to investigate in
parallel; if your environment supports running an isolated sub-task, delegate
each chain's investigation and have it return only a short summary (head
blocker's session ID and last statement) rather than the full request/lock
listings.

## Step 3 — inspect the head blocker's last statement

Search for how to look up a session's last-submitted statement (its "input
buffer") using the head blocker's session ID, so the user can see what it's
actually running. Also check lock information (search for a lock listing)
to see what resource is being held.

## Step 4 — summarize for the user, then gate on explicit confirmation

Report the chain, the head blocker's session ID, its last statement, and how
long it's been blocking. **Do not terminate anything without the user
explicitly confirming** — ending a session is destructive (it rolls back any
open transaction and disconnects whatever is using that session) and cannot
be undone. Ask directly: "should I terminate session `<id>`?"

## Step 5 — terminate only after confirmation, then verify

This catalog has no general "run arbitrary T-SQL" operation — extended
stored procedures like `sp_executesql` have no queryable parameter metadata
and aren't part of the generated catalog (see
docs/sqlserver-eda-openapi-pipeline/README.md's "Known limitations"), and
`KILL` has no dedicated wrapper procedure in stock SQL Server either. So
even after the user confirms, this server cannot terminate the session for
them through an MCP tool call — tell them that directly, and give them the
exact statement to run themselves: `KILL <session_id>`. Offer to verify
afterward: repeat Step 1 once they confirm they've run it, to confirm the
chain actually cleared.

## Composing with other workflows

For a lighter-weight "who's connected right now" overview without a full
blocking investigation, `sqlserver-performance-diagnostics` points
at `sp_who`/`sp_who2`. If the underlying cause looks like a missing index
making a query hold locks longer than necessary, fetch
`sqlserver-index-tuning-recommendations`.
