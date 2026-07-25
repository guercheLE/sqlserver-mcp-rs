# Guided workflow: index tuning recommendations

This sub-workflow is designed to be run as an isolated sub-task where
possible — if you were delegated here from `sqlserver`'s routing, or
your environment otherwise supports running this as its own sub-task,
everything you need is in this prompt's own text plus the parameters already
listed above; report back only a short summary when done rather than the
full step-by-step trace.

**Agnostic phrasing rule**: never call a hardcoded `operationId`. Search for
the capability you need, then read the schema `get` currently returns before
relying on any parameter or result-column name — object availability can
differ across the four supported engine versions (2017/2019/2022/2025). This
workflow's core operations (the missing-index DMV family and the automatic
tuning-recommendations view) are present in all four versions, with two
documented exceptions handled below.

## Step 0 — gather scope

Check the "Context already provided" header above first. `database`,
`schema`, and `table` narrow the search but aren't required — without a
table, treat this as "find the best candidate across the whole database."

## Step 1 — find missing-index candidates

Search for the missing-index DMV family and list candidates ranked by
estimated improvement, scoped to `database`/`table` if known. **Version
note**: a dedicated "ranked query" view for this exists in most versions but
is absent in SQL Server 2017 — if searching for it comes up empty, compute
the same ranking yourself from the unranked group-stats and details views
instead (`avg_total_user_cost * avg_user_impact * (user_seeks +
user_scans)` is the standard formula) rather than assuming the ranked view
exists.

SQL Server's own automatic tuning can also surface recommendations — search
for an automatic tuning-recommendations view as a second source, and
cross-check it against Step 1's results rather than picking one arbitrarily.
A deeper, more detailed automatic-tuning breakdown (recommendation impact
metrics, workflow status) may also be available, but **only on newer
engine versions** — treat it as an optional deeper dive if search finds it,
never a required step.

## Step 2 — get the exact column list for a chosen candidate

Once the user picks (or you recommend) a candidate, search for the specific
key/included-column breakdown for that missing-index group, so you have the
exact columns and order needed to define the index.

## Step 3 — check for an existing or overlapping index

Gated on Step 2. Fetch `sqlserver-indexes-constraints` (or directly
search for an index listing on the same table) and confirm no existing index
already covers this — creating a redundant or near-duplicate index wastes
write overhead for no benefit. Don't skip this just because a DMV recommends
it.

## Step 4 — build the statement and get explicit confirmation

Compose the exact `CREATE INDEX` statement (name it something descriptive
and reversible, e.g. prefixed `IX_` with the table/columns encoded) and show
it to the user verbatim. **Do not execute anything without the user
explicitly confirming this exact statement** — creating an index is a
schema change with real cost (build time, disk space, write overhead) that
isn't trivially undone on a large or busy table.

## Step 5 — execute and verify

This catalog has no general "run arbitrary T-SQL" operation — extended
stored procedures like `sp_executesql` have no queryable parameter metadata
and aren't part of the generated catalog (see
docs/sqlserver-eda-openapi-pipeline/README.md's "Known limitations"), so
there is no MCP tool call that can run the `CREATE INDEX` statement from
Step 4. Once the user has explicitly confirmed the exact statement, tell
them this server can recommend and verify the index but can't create it for
them — they'll need to run the confirmed `CREATE INDEX` statement themselves
(SSMS/sqlcmd). Offer to verify it afterward: search for an index lookup on
the table and confirm the new index now exists, and note that the
missing-index DMVs won't reflect this fix until the workload that triggered
the recommendation runs again.

## Composing with other workflows

Table/column discovery belongs to `sqlserver-schema-exploration`;
the read-only side of index inspection (without the tuning-recommendation
angle) belongs to `sqlserver-indexes-constraints` — fetch those
prompts rather than duplicating their guidance here.
