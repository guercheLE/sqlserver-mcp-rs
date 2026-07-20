# Guided workflow: indexes and constraints

Inspecting a table's indexes, foreign keys, and check constraints is a
single search-then-call action per question, not a multi-step ordered flow.

Check the "Context already provided" header above first; if `database`,
`schema`, or `table` are still missing and the user's question needs them,
ask before searching — most of these lookups need a specific table to be
useful.

**Agnostic phrasing rule**: never call a hardcoded `operationId`. Search for
the capability you need (e.g. "search for how to list indexes on a table"),
then read the schema `get` currently returns before relying on any parameter
or result-column name — object availability can differ across the four
supported engine versions (2017/2019/2022/2025).

## Typical questions and what to search for

- "What indexes exist on this table, and which columns/order?" → search for
  an index listing, then an index-columns listing for the columns/key order
  of a specific index.
- "What foreign keys reference or are defined on this table?" → search for a
  foreign-key listing.
- "What check constraints exist on this table?" → search for a
  check-constraint listing.
- "How much space could this table save with compression?" → search for a
  compression-savings estimate, which needs the table (and optionally a
  specific index) named explicitly.
- A more detailed, human-readable summary of a single index's columns and
  key order is also available via a dedicated index-help lookup — search for
  it by name if the structured listing above isn't enough.

## Composing with other workflows

Whether the table itself exists, and what columns it has, is covered by
`sqlserver-schema-exploration` — fetch that prompt rather than
duplicating its guidance here.
