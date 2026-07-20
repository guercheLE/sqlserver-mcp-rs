# Guided workflow: login/user/role provisioning

This sub-workflow is designed to be run as an isolated sub-task where
possible — if you were delegated here from `sqlserver`'s routing, or
your environment otherwise supports running this as its own sub-task,
everything you need is in this prompt's own text plus the parameters already
listed above; report back only a short summary when done rather than the
full step-by-step trace.

**Agnostic phrasing rule**: never call a hardcoded `operationId`. Search for
the capability you need (e.g. "search for how to grant a login access to a
database"), then read the schema `get` currently returns before relying on
any parameter or result-column name — object availability can differ across
the four supported engine versions (2017/2019/2022/2025).

## Step 0 — gather required parameters

Check the "Context already provided" header above first. You need at
minimum: the login name, and the target database. Don't proceed to Step 1
until both are known — ask if they aren't.

## Step 1 — does the login already exist?

Search for how to list server logins/principals and check whether
`login_name` already exists.

- **If it doesn't exist**: search for how to create a SQL login, then call
  it. **Gate**: confirm via the same lookup that the login now actually
  exists before moving on — don't rely on the create call not erroring.
- **If it already exists**: skip straight to Step 2 — don't recreate it.

## Step 2 — grant database access

Gated on Step 1's login existing. Search for how to grant a login access to
a database (creating the corresponding database user), then call it with
`database`. **Gate**: confirm via a database-principals lookup that the user
now actually exists in that database.

## Step 3 — is a built-in role sufficient, or does this need a custom one?

This is a genuine fork, not a default — creating an unnecessary custom role
is a common but avoidable mistake. Ask the user: "should this login use an
existing fixed/built-in database role (e.g. `db_datareader`,
`db_datawriter`), or does it need a new custom role?"

- **Built-in role is sufficient**: use `role_name` as given (or ask which
  built-in role, if not yet supplied) and skip to Step 4.
- **Needs a custom role**: search for how to list existing database roles
  first — confirm one matching the user's intent doesn't already exist —
  then search for how to create a new database role, call it, and use that
  as `role_name` in Step 4.

## Step 4 — add the login to the role

Gated on Steps 2–3. Search for how to add a database user to a role, then
call it with the database user (from Step 2) and `role_name` (from Step 3).

## Step 5 — verify actual role membership

Don't consider this done just because the grant call didn't error. Search
for a database-principals/role-membership lookup and confirm the login is
actually listed as a member of the target role in the target database.

## Composing with other workflows

If you need to confirm the database itself exists before granting access to
it, fetch `sqlserver-schema-exploration` rather than duplicating its
guidance here.
