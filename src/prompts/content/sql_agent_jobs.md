# Guided workflow: SQL Agent job scheduling

This sub-workflow is designed to be run as an isolated sub-task where
possible — if you were delegated here from `sqlserver_workflow`'s routing, or
your environment otherwise supports running this as its own sub-task,
everything you need is in this prompt's own text plus the parameters already
listed above; report back only a short summary when done rather than the full
step-by-step trace.

**Agnostic phrasing rule**: SQL Server object availability differs across the
supported engine versions (2017/2019/2022/2025). Never call a hardcoded
`operationId` from this prompt — always `search` for the capability you need
(e.g. "search for how to create a SQL Agent job"), then read the schema `get`
currently returns before relying on any parameter or result-column name.

## Step 0 — gather required parameters

Check the "Context already provided" header above first; only ask the user
for what's still listed as missing. You need at minimum: a job name, and the
command each step should run (and its target database, if not `msdb`). Don't
proceed to Step 1 until the job name and at least one step's command are
known — ask if they aren't.

## Step 1 — create the job

Search for how to add a new SQL Agent job, then call it with the job name
(and an owner/category if the user cares). **Gate**: before moving on,
search for how to look up a job's details and confirm the job now actually
exists — don't rely on the create call simply not erroring.

## Step 2 — add one or more job steps

Gated on Step 1's job existing. Search for how to add a step to a SQL Agent
job, then call it once per step the user wants, in the order they should run.

If the user is setting up more than one *independent* job in this session,
their step-creation calls don't depend on each other — call this out as
parallelizable, and as a candidate for delegation: if your environment
supports running an isolated sub-task, delegate each independent job's
step-creation and have it return only a short confirmation, rather than
pulling every create-call's full request/response body into this
conversation. If no such mechanism is available, just do the calls directly,
one after another.

**Gate**: confirm via a job-step lookup that every step was actually added
before moving on.

## Step 3 — attach a schedule

Gated on the job and at least one step existing. Ask the user whether the job
should run on a schedule or stay on-demand-only; if scheduled, search for how
to create and attach a schedule to a job, and confirm via a schedule lookup
that it actually attached.

## Step 4 — start the job (optional)

Only if the user wants an immediate run, gated on Steps 1–3 being confirmed.
Search for how to start a job, then **verify success via the job's run
history** (search for a job-history lookup) rather than assuming success just
because the start call didn't error — a job can start successfully and still
fail on its first step.

## Step 5 — summarize and offer cleanup

Summarize what was created/started and confirm with the user. If this was a
test run, offer to stop and/or delete the job (search for the matching
operations) rather than leaving it scheduled.

## Composing with other workflows

If a job step's command references a specific database/table and you're not
sure it exists, fetch `sqlserver_workflow_schema_exploration` for how to
verify that rather than duplicating its guidance here.
