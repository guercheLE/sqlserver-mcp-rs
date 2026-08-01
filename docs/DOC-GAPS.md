# Documentation gaps: lessons from this repo's history

`sqlserver-mcp-rs` does not have a full PRD/architecture doc set. Its
`docs/` folder holds three things: `SCHEMA_VERSIONS.md` (a one-page table of
the four supported catalog versions), `mcp-prompts-workflow-plan.md` (a
detailed design doc for the MCP-prompts feature, including a later
addendum), and `sqlserver-eda-openapi-pipeline/` — the SQL Server catalog
extraction pipeline, with its own `README.md` (usage, mapping conventions,
"Known limitations") and `plan.md` (a running implementation log). There is
no requirements doc, no architecture overview, and no test-strategy doc.
That narrower baseline only changes **which docs a gap is compared
against** below — it does not narrow how much of the git history was
mined. All ~62 commits across all 22 releases (`v0.1.1`–`v0.6.7`) were
walked for this analysis.

This repo is generated/scaffolded by a separate tool, **mcpify**
(`~/Documents/GitHub/mcpify`, source docs at `~/Documents/GitHub/mcpify/docs/`
— `product-brief.md`, `prd.md`, `architecture.md`, `v1`–`v10-implementation-plan.md`,
and its own `docs/DOC-GAPS.md`/`CHANGELOG.md`). But this is **not** a plain
mcpify-generated HTTP-API-wrapper project: mcpify's generic
`reqwest`/HTTP-based API-calling layer was replaced end-to-end with a
hand-built SQL Server "channel" — a real TDS connection via `tiberius`
(`api_client.rs`, `sql_type.rs`, `sql_pool.rs`, the object-classification
read-only safeguard, the sandbox/database-qualification logic), conceptually
the same kind of swap as `rabbitmq-mcp-ts`'s hand-built AMQP channel
adaptation (though that project isn't mcpify-generated at all). This
matters for how each entry below is tagged and what it's compared against:

- **`(shared mcpify-template gap)`** — the doc gap is in mcpify's own
  generic scaffold (logging, the base config cascade, CI/release workflow,
  credential storage, the embedded `mcp_store.db` store layer, schema/`$ref`
  handling, the pre-commit gate, embedding-model packaging) — code this
  repo never touched or adapted. These entries are compared against
  mcpify's own `docs/prd.md`/`docs/architecture.md`/`vN-implementation-plan.md`,
  and cross-referenced against mcpify's own `docs/DOC-GAPS.md` where a
  matching entry already exists there.
- **`(channel-adaptation-specific)`** — the doc gap is in the hand-built
  SQL Server TDS channel itself (or a bug that only exists *because* that
  hand-built channel replaced mcpify's generated HTTP client). mcpify's
  generic docs would never have specified SQL-Server-wire-protocol behavior
  — that's this repo's own hand-built adaptation on top of the generated
  scaffold. These entries are compared only against this repo's own
  docs/README, never against mcpify's docs.
- **`(repo-specific)`** — genuinely about this repo's own domain tooling
  (the EDA/OpenAPI catalog-extraction pipeline, the MCP-prompts feature —
  neither of which mcpify generates at all) or a project-management choice
  (a coverage target) that has no mcpify-side counterpart to compare
  against either way.

## Lessons for future docs

1. **Specify a pinned toolchain and a captured lint baseline once, in
   `architecture.md`, not per generated file.** This repo hit CI-failing
   `cargo fmt`/`clippy` issues three separate times (`v0.1.1`, `v0.6.3`,
   `v0.6.5` — the last from toolchain drift enforcing new lints). mcpify's
   own `docs/DOC-GAPS.md` already names the adjacent half of this problem
   ("no local pre-commit gate despite every CI pipeline enforcing one," fixed
   2026-08-01 via `.githooks/pre-commit` in mcpify itself and all 5
   generated-project templates) — worth pointing at directly rather than
   re-deriving. But a pre-commit hook only helps a *human* catch drift
   before pushing; it doesn't prevent a *freshly generated* project from
   already failing its own CI gate at generation time, which is what
   happened at `v0.1.1`. `docs/prd.md` REQ-2.5.1's "zero-placeholder"
   quality bar requires a freshly generated project's *test suite* to pass
   unedited — it says nothing about the same project's own `fmt`/`clippy`
   gate. That's a narrower, still-open sub-gap worth adding explicitly.

2. **When a fix for a shared, cross-repo bug lands, write the retrospective
   doc-gap entry in the same commit/PR — don't leave it for a later
   archaeology pass.** A striking number of this repo's `(shared
   mcpify-template gap)` entries below turned out to already be fixed
   upstream in mcpify's current templates (the CI-timeout fix, the
   `publish-crate.yml` workflow, the dist-ability `Cargo.toml` fix, the
   profiling CPU/heap split, the `768`-dim embedding restoration) — but
   **none of them have a corresponding entry in mcpify's own
   `docs/DOC-GAPS.md`**, which only tracks entries someone deliberately
   wrote down after the fact. Worse, one fix (`lto = "thin"` still
   unconditional in `[profile.dist]` of mcpify's current
   `src/targets/rust/templates/Cargo.toml.tera`, confirmed by direct
   inspection at the time of this writing) was **never fixed upstream at
   all** — this repo's `v0.1.3` patch only disabled it locally, so every
   other mcpify-generated Rust project still ships with a broken macOS
   release build waiting to happen. A fix that isn't written up as a gap is
   a fix that doesn't propagate — either to sibling repos already generated,
   or as a lesson for the next `vN-implementation-plan.md`.

3. **Concurrency/platform-specific I/O semantics for any embedded local
   datastore belong in the architecture doc, not just in Rust doc-comments.**
   Already the exact wording of mcpify's own `docs/DOC-GAPS.md` Lesson #5,
   and this repo's Windows store-lock saga (`v0.6.4`–`v0.6.7`) and mutex-
   poisoning fix (`v0.5.1`) are literally downstream copies of the same
   fixes mcpify's own history already made to its `mcp_store.db` template
   (see the dated entries below) — no new lesson needed here beyond
   pointing at mcpify's own writeup.

4. **A generator that offers both a "regenerate everything" and an
   "incrementally update" command must document, in the architecture doc
   itself, exactly what each one is allowed to clobber — before anyone
   hand-edits a single generated file.** mcpify's `docs/architecture.md` has
   a full section titled "`add-version`: a lighter, separate lifecycle"
   spelling out precisely which steps `add-version` skips and which files it
   touches. There is no equivalent section anywhere in mcpify's docs for
   `sync` (`mcpify sync --manifest ...`, present in the CLI per
   `src/cli.rs`'s own test fixtures) — no doc states that `sync` fully
   regenerates a project's scaffolding from the manifest, as if from
   scratch, with no protection for hand-edited files. This repo's costliest
   incident (`v0.6.0`–`v0.6.1`, below) is a direct consequence of that
   missing section. This asymmetry — one command's blast radius documented,
   the sibling command's not — is exactly the kind of gap a "Cross-Cutting
   Constraints" or "Command Safety" section would catch if reviewed as a
   pair rather than one command at a time.

5. **When a generated integration layer is deliberately replaced by a
   hand-built one (a "channel adaptation" — this repo's TDS channel instead
   of mcpify's generated `reqwest` HTTP client, or `rabbitmq-mcp-ts`'s AMQP
   channel), write a short contract doc for what the replacement must
   preserve from the thing it's replacing — before writing the replacement.**
   Every `(channel-adaptation-specific)` entry below (the named-`EXEC`
   parameter syntax, the input-schema `body`-property nesting the
   hand-written parser didn't know to expect, the sandbox database
   qualifier, the SQL-parameter out-of-range validation, the read-only
   object-classification safeguard) is a case where the hand-written channel
   silently diverged from an assumption mcpify's *own* generated client
   would have satisfied automatically (mcpify's generic schema-normalization
   code already wraps request parameters under `body` consistently — a
   hand-written parser has to be told that explicitly, since it isn't
   regenerated alongside it). This repo has no such contract doc anywhere —
   the README's one-line mention ("hand-wired to a real TDS connection ...
   instead of mcpify's default HTTP client") is the closest thing to one.
   A short "what the generated `input_schema`/`output_schema` shape
   guarantees, and what the replacement client must still honor" doc,
   written once when the channel was first hand-built, would have caught
   at least two of the five entries below before they shipped as bugs.

6. **Capture hard platform/registry limits (package-size caps, embedding-
   model dimensionality trade-offs) in a constraints section before they're
   discovered by a failed release.** mcpify's own `v1-implementation-plan.md`
   and `v2-implementation-plan.md` hard-code the `768`-dim `all-mpnet-base-v2`
   embedding model against the `mcp_store.db` vector schema with no
   accompanying package-size budget — this repo hit exactly that collision
   against crates.io's 10 MiB limit (`v0.2.0`, below), and mcpify's own
   Lesson #4 already names the identical pattern for schema `$ref`
   duplication ("attach a cost budget, not just a correctness claim"). The
   embedding-dimension case is the same lesson, different axis, still
   uncaptured.

7. **Pick wire-visible naming conventions before first ship, not after.**
   This repo's MCP prompt identifiers shipped snake_case (`v0.3.0`) and had
   to be renamed to kebab-case as a breaking change two releases later
   (`v0.5.0`) — a repo-specific case (mcpify doesn't generate an MCP-prompts
   capability at all), but the underlying lesson generalizes to any
   wire-visible identifier a design doc introduces.

8. **Set numeric targets (coverage, performance budgets) only after
   cataloging what's realistically testable, and document the boundary, not
   just the number.** This repo's own `plan.md` set an 85% coverage target
   that had to be lowered to 75% one release later (`v0.6.1`–`v0.6.2`) once
   the remaining gap was recognized as requiring disproportionate scope
   (mocking interactive prompts, a live-infra test tier, a simulated
   protocol handshake). Purely repo-specific — mcpify's PRD sets no
   generated-project coverage target at all.

## Doc gaps by date

### 2026-07-26 — Windows CI store-lock flakiness and a `HOME` env-var test race (v0.6.4–v0.6.7)

#### Doc gap

`docs/architecture.md` §2 ("Data Layer") describes `mcp_store.db` extraction
without ever mentioning concurrency or platform-specific file-handle
semantics — see mcpify's own **`docs/DOC-GAPS.md` entry "Windows-specific
file-locking/timing semantics unaddressed for the embedded store (v0.10.5 -
v0.11.11)"**, which documents this exact gap and lists the identical
progression of fixes (widen retry, widen again, skip the test on Windows CI
runners) against mcpify's *own* generated-Rust-target template. This repo's
`mcp_store.db`/credential-storage tests are downstream copies of that same
generic component; nothing here is specific to SQL Server.

#### Resulting work

Four patch releases mirroring mcpify's own v0.11.8–v0.11.11 fixes almost
exactly: `v0.6.4` widened the Windows file-lock retry to 10s and added a
`HOME_ENV_TEST_LOCK` guard around a credential-storage test that mutates the
real process-wide `HOME` variable; `v0.6.5` widened further; `v0.6.6`
widened to 60s; `v0.6.7` gave up widening and skipped the disk-lock test on
`GITHUB_ACTIONS`+Windows entirely, matching the same runner-specific skip
`cli_smoke.rs` already used.

**(shared mcpify-template gap — see mcpify's own `docs/DOC-GAPS.md`)**

### 2026-07-26 — stale generated-file banner after `add-version` (v0.6.5)

#### Doc gap

mcpify's `docs/architecture.md` §5 ("Ledger") *does* document that
`language`/`display_name`/`project_name` are "written once by `generate`
and never re-derived from a later spec" — so this isn't a fully-absent
spec. What's missing is the *consequence* of that documented freeze: no doc
warns a template author that baking `display_name` into a **user-facing**
banner/description comment is unsafe once a project outlives its first
`generate` (e.g. via `add-version`, which by design never touches
`display_name`). The architecture doc specifies the ledger's internal
behavior but not the implication for anything downstream that renders it as
prose.

#### Resulting work

The Rust target's generated banner/description comment (rendered once at
generation time, before this project's other three SQL Server versions were
added) still read "2025" long after `add-version` added `2022`/`2019`/`2017`
alongside it. Reworded to be version-neutral ("SQL Server -
master/msdb/sandbox combined catalog").

**(shared mcpify-template gap — not found in mcpify's own `docs/DOC-GAPS.md`; likely worth a fresh entry there)**

### 2026-07-26 — recurring `cargo fmt`/`clippy` CI failures (v0.1.1, v0.6.3, v0.6.5)

#### Doc gap

No mcpify doc states a pinned toolchain policy or captures the lint
baseline at generation time (see Lesson #1 above). This recurred three
times: unformatted/lint-failing code at initial generation (`v0.1.1`, before
any hand-editing), four files failing `cargo fmt --check` after later work
(`v0.6.3` — two of the four, `api_client.rs`/`sql_type.rs`, are the
hand-written SQL Server channel; the other two, `get_tool.rs`/
`cli_commands.rs`, are generic generated scaffold), and new lints
(`clippy::manual_repeat_n`, `clippy::len_zero`) "newly enforced by a more
recent stable toolchain" (`v0.6.5`, in generic scaffold files
`search_tool.rs`/`cli_commands.rs`). The gap is generic — it hits generated
and hand-written files alike whenever the toolchain moves and nothing pins
it.

#### Resulting work

Three separate one-off `fix:` commits; no toolchain pin or generation-time
lint gate was ever added to prevent a fourth occurrence.

**(shared mcpify-template gap — related to, but narrower than, mcpify's own `docs/DOC-GAPS.md` entry "no local pre-commit gate despite every CI pipeline enforcing one"; the zero-placeholder-bar sub-gap noted in Lesson #1 is not covered there and is a fresh finding)**

### 2026-07-26 — `call` tool read-only safety enforcement (v0.6.3)

#### Doc gap

No doc anywhere in this repo (prompts plan, pipeline README, README config
table) specified how a T-SQL backend — which has no session-level
"read only" setting the way PostgreSQL's `default_transaction_read_only`
does — should enforce a read-only mode. This is pure SQL Server wire/engine
semantics that only exists because this repo's channel talks TDS directly;
mcpify's generic HTTP-client architecture has no analogous concept to fall
short on, so there is nothing to compare against in mcpify's own docs.

#### Resulting work

Added `SQLSERVER_READ_ONLY` (default `true`) and an object-classification
enforcement in the `call` tool: refuse any operation whose catalog type
isn't `VIEW` or a `*_FUNCTION` kind (both are guaranteed side-effect-free by
SQL Server itself), reject stored procedures and unclassified operations
outright.

**(channel-adaptation-specific)**

### 2026-07-26 / 2026-07-21 — self-contained schema/`$ref` handling, two downstream iterations (v0.5.3, v0.6.3)

#### Doc gap

mcpify's own `docs/DOC-GAPS.md` entry **"self-contained schema/`$ref`
handling took three iterations (v0.5.12, v0.11.5, v0.11.7)"** covers this
gap directly, including the root cause: `v1-implementation-plan.md`'s
decision to embed schema copies "at the cost of some file size" was never
paired with a bound or a scale check (mcpify's own Lesson #4). This repo's
two fixes are downstream syncs of mcpify's v0.11.5 and v0.11.7 fixes
respectively, not independent discoveries.

#### Resulting work

`v0.5.3`: regenerated every catalog version via `mcpify add-version
--force` to pick up upstream mcpify generator fix 0.11.5 (embed `$defs`
alongside `$ref` so `get` responses are self-contained). `v0.6.3`:
recursively inlined every remaining `$ref` to match mcpify's second,
final companion fix (0.11.7 — full inlining instead of `$defs`
localization).

**(shared mcpify-template gap — see mcpify's own `docs/DOC-GAPS.md`)**

### 2026-07-25 / 2026-07-26 — coverage target set without a testability boundary (v0.6.1–v0.6.2)

#### Doc gap

This repo's own `docs/sqlserver-eda-openapi-pipeline/plan.md` set an 85%
coverage target with no documented accounting of what's realistically
testable without live infrastructure (a real SQL Server/Docker instance,
`inquire`'s interactive prompts with no stdin-injection utility available, a
real MCP client handshake). mcpify's PRD sets no coverage target for
generated projects at all (REQ-2.3.6/REQ-2.6 require tests to exist and
pass, never a numeric percentage), so there's no mcpify doc to compare
against either way.

#### Resulting work

Coverage was raised from 65.01% to 76.23% (`v0.6.1`) through real test
additions (tool dispatch, connection pooling, embeddings,
`populate_embeddings`, `AuthManager`'s credential cascade, extracted
`setup_wizard` selection-parsing into pure functions), then the target
itself was lowered from 85% to 75% (`v0.6.2`) once the remaining gap was
recognized as requiring disproportionate scope.

**(repo-specific)**

### 2026-07-25 — `mcpify sync` silently wipes hand-rewritten files (v0.6.0–v0.6.1)

#### Doc gap

See Lesson #4 above: mcpify's `docs/architecture.md` documents exactly what
`add-version` is allowed to touch, in a dedicated section — there is no
equivalent section, anywhere in mcpify's docs, specifying what `sync` is
allowed to clobber. This gap is **not** about SQL Server specifically —
`sync` fully regenerates a project's scaffolding from the manifest via Tera
templates regardless of target language or domain, so it would destroy
*any* hand-edited file in *any* mcpify-generated project, not just a
hand-written TDS channel. The channel code is simply what this repo had
hand-edited, so it's what took the hit. Confirmed **not present** in
mcpify's own `docs/DOC-GAPS.md` — none of its nine entries mention `sync`
at all.

#### Resulting work

`mcpify sync` reset the hand-rewritten TDS/SQL-Server auth and transport
layer back to mcpify's generic Basic/OAuth2/`reqwest`-HTTP template, and
separately silently dropped an already-implemented setup-wizard wording fix
(the "Windows auth is NTLM, not SSO" clarification) because the recovery
patch had been snapshotted before that specific edit landed — neither
compilation nor tests caught the loss, since nothing asserted on the
wizard's literal prompt text. The wording had to be re-added by hand in a
follow-up commit (`v0.6.1`), and this repo's own pipeline README/`plan.md`
were updated to document the `sync` vs. `add-version` distinction going
forward — a repo-local mitigation for a generator-level documentation gap.

**(shared mcpify-template gap — confirmed absent from mcpify's own `docs/DOC-GAPS.md`; likely the single highest-value fresh entry to add there, since it's a data-loss-class gap, not just a flaky-test-class one)**

### 2026-07-25 — guided-prompt content coupled to generated catalog contents with no drift contract (v0.6.0)

#### Doc gap

This repo's own `docs/mcp-prompts-workflow-plan.md` assumed `sp_executesql`
(and other extended stored procedures) would remain a permanent,
general-purpose "escape hatch" operation in the catalog, with nothing
documented as an assumption to re-verify if catalog generation ever
changed. MCP prompts are a feature this repo hand-built entirely outside
mcpify (mcpify's PRD's "3 Universal Tools," §1.5, has no prompts capability
at all), so there's no mcpify doc to compare against.

#### Resulting work

When the same-release catalog-generation rewrite (below) dropped
`sp_executesql` and all extended stored procedures from the generated
catalog entirely, three guided workflows that referenced it as the escape
hatch (dropping a linked server, `CREATE INDEX`, killing a blocking
session) had to be reworded to say the capability isn't available.

**(repo-specific)**

### 2026-07-25 — curated allowlist approach replaced by full sweep + ranking + dedup (v0.6.0)

#### Doc gap

The original pipeline design (curated `allowlist.yaml`, per-database
operation paths, documented only in this repo's own
`docs/sqlserver-eda-openapi-pipeline/README.md`) had no documented scaling
plan for an object existing identically across multiple system databases,
or for extending curation as SQL Server's shipped-object surface grows
across versions. This is this repo's own hand-built OpenAPI-authoring
pipeline (a plain mcpify HTTP-wrapper project just points mcpify at an
already-existing spec — it never needs to *author* one), so it has no
mcpify-side counterpart either.

#### Resulting work

A full rewrite: `sys.all_objects` sweep (`is_ms_shipped = 1`) replacing the
curated allowlist; `generate_openapi.py` rewritten for cross-database
deduplication and ranking, capped to the top 500 per version; a new
`execution_database` request property and resolution order added to the
server, since one operation can now represent the same object across more
than one database.

**(repo-specific)**

### 2026-07-21 — no default `timeout-minutes` on the generated CI/release workflow template (v0.5.2)

#### Doc gap

Neither `docs/prd.md` nor `docs/architecture.md` specifies a default job
timeout for the CI/release workflow templates every target emits (PRD
REQ-2.3.7 requires "a CI/CD pipeline... with automated versioning/
publishing" but says nothing about timeout-minutes). mcpify's own
`CHANGELOG.md` records fixes at `v0.11.3`/`v0.11.4` ("Capped the release
workflow's build job at a 45-minute timeout," "Capped mcpify's own CI test
job at a 20-minute timeout") — but reading the actual commits shows these
capped **mcpify's own repository's CI**, not the templates it generates for
downstream projects. The template-level fix is a separate commit
(`21f4e963`, `fix(rust): cap the CI test job at 20 minutes`, 2026-07-21,
touching `src/targets/rust/templates/.github/workflows/ci.yml.tera`) landed
the same day as this repo's own independent fix below — confirmed present
in the current template. Neither the template fix nor the distinction from
mcpify's own-CI fix is captured in mcpify's `docs/DOC-GAPS.md`.

#### Resulting work

Capped every `release.yml`/`ci.yml` job in this repo with `timeout-minutes`
— none had one, so a hang fell back to GitHub Actions' own 6-hour ceiling,
directly motivated by the same real incident (a hung test from mutex
poisoning, below) that also motivated mcpify's own template fix the same
day.

**(shared mcpify-template gap — the template-level fix exists upstream, same-day, but is not documented as its own `docs/DOC-GAPS.md` entry there; only the *mcpify-repo's-own-CI* half is recorded, in `CHANGELOG.md` only — worth flagging as a fresh, more precisely-scoped entry)**

### 2026-07-21 — mutex poisoning on the shared store connection (v0.5.1)

#### Doc gap

Covered by the same mcpify `docs/DOC-GAPS.md` "Windows-specific
file-locking/timing semantics unaddressed for the embedded store" entry
cited above — its own fix list includes `v0.11.2`: `fix(rust): recover from
mutex poisoning on the shared store connection`, the exact upstream source
of this repo's fix.

#### Resulting work

Recovered from poisoning at every lock site on the shared, process-wide
`cached_store_connection` instead of propagating it — a panic while holding
that lock previously took down every later `search`/`get`/`call` call
sharing the connection, even though all access through it is read-only and
safe to recover from.

**(shared mcpify-template gap — see mcpify's own `docs/DOC-GAPS.md`)**

### 2026-07-20 — no naming-convention decision for MCP prompt identifiers before ship (v0.5.0)

#### Doc gap

This repo's own `docs/mcp-prompts-workflow-plan.md` did not fix a naming
convention for wire-visible prompt identifiers before implementation.
Prompts are entirely hand-built (mcpify's PRD has no prompts capability),
so there's no mcpify doc to compare against.

#### Resulting work

A breaking rename from snake_case with a redundant `_workflow` segment
(`sqlserver_workflow_sql_agent_jobs`) to short kebab-case
(`sqlserver-sql-agent-jobs`), two releases after first ship, to match the
slash-command convention most MCP clients use.

**(repo-specific)**

### 2026-07-20 — SQL Server named-`EXEC` parameter syntax and input-schema `body`-nesting mismatch in the hand-rewritten channel (v0.4.1)

#### Doc gap

This bug is entirely a consequence of the hand-built TDS channel (see
Lesson #5): mcpify's own generated `reqwest`-based API client stays in sync
with mcpify's schema-generation code by construction (both live in the same
generator codebase — confirmed by inspecting `src/openapi/schema_resolve.rs`,
which wraps every operation's real parameters one level down under a single
`body` property, generically, for every target). A **plain, unmodified**
mcpify-generated HTTP-API-wrapper project would never hit this bug. It only
surfaced here because this repo's hand-written `api_client.rs` parses
`input_schema` itself and nothing told its author about the `body`-nesting
convention — and no design doc in this repo (not the README's one-line
mention of the TDS swap, not any other doc) records the contract the
hand-written channel needs to honor. The named-`EXEC` argument syntax
(`@name = @P1`, not `[name] = @P1`) is separately pure SQL Server wire-level
knowledge with no mcpify-side equivalent at all.

#### Resulting work

Two independent bugs meant every parameterized stored-procedure call
(`sp_executesql`, `sp_columns`, `sp_rename`, etc.) failed or silently sent
no real arguments, on every supported engine version. Existing unit tests
didn't catch either because they hand-built `Param` structs directly,
bypassing the real schema-parsing path. Fixed both, added regression tests
covering the inline-body and no-body-property schema shapes, and verified
end-to-end against a real SQL Server 2022 instance.

**(channel-adaptation-specific)**

### 2026-07-20 — guided-workflow prose referenced a nonexistent catalog operation (v0.4.0)

#### Doc gap

This repo's prompt-content authoring process (`docs/mcp-prompts-workflow-plan.md`)
had no required step to verify that an operation referenced in
guided-workflow prose actually exists in the generated catalog on every
supported engine version before writing about it. Prompts and the catalog
they reference are both entirely repo-specific.

#### Resulting work

`server_administration.md`'s linked-server guidance pointed at
`sp_droplinkedserver`, confirmed absent in all four supported catalog
versions — corrected to the two operations that do exist. The subsequent
addendum began cross-checking every new operation reference against all
four version stores directly, closing this gap going forward.

**(repo-specific)**

### 2026-07-19 — `dist host` rejects a hand-simplified `release.yml` (v0.2.1)

#### Doc gap

No mcpify doc specifies that a deliberately simplified, hand-written
single-job release workflow (versus cargo-dist's own auto-generated
plan/host/announce split) needs `allow-dirty` set explicitly, or
`dist host --steps=create` refuses to run against it as "out of date."
mcpify's own `CHANGELOG.md` confirms the identical fix landed upstream —
`v0.10.3` ("Generated Rust projects' release workflow now allows a dirty
git tree by default") and `v0.10.4` ("Moved the `allow-dirty` `cargo-dist`
setting to the correct `[dist]` table") — but neither is elevated to a
`docs/DOC-GAPS.md` entry over there.

#### Resulting work

Added `allow-dirty` to this repo's `release.yml`.

**(shared mcpify-template gap — fix confirmed in mcpify's own `CHANGELOG.md` [0.10.3]/[0.10.4], but not written up in `docs/DOC-GAPS.md`)**

### 2026-07-19 — embedding dimensionality vs. crates.io package-size limit not documented up front (v0.2.0)

#### Doc gap

mcpify's `docs/v1-implementation-plan.md` (line ~103) and
`docs/v2-implementation-plan.md` (lines ~29–30) hard-code the `768`-dim
`all-mpnet-base-v2` embedding model against the `mcp_store.db` vector
schema — a deliberate, documented decision, but with no accompanying
package-size budget or registry-limit check, the same class of gap mcpify's
own Lesson #4 names for schema `$ref` duplication ("attach a cost budget,
not just a correctness claim"). Nothing in either implementation plan flags
crates.io's 10 MiB package limit as a constraint interacting with model
choice.

#### Resulting work

An earlier, undocumented decision to switch to a 384-dim model (to fit
under the size limit, measured before store compression existed) was a real
quality regression, reverted once zstd compression (level 19) brought the
four-version catalog to 8.5 MiB compressed — comfortably under the limit —
restoring the 768-dim model and removing the now-dead `resize_embeddings.rs`
binary that existed solely to patch the dimension mismatch.

**(shared mcpify-template gap — not found in mcpify's own `docs/DOC-GAPS.md`; a fresh finding, directly parallel to mcpify's own Lesson #4 but on a different axis)**

### 2026-07-19 — upstream mcpify feature (zstd store compression) not present in a hand-forked project (v0.2.0)

#### Doc gap

The same underlying tension as the `mcpify sync` entry above — no mcpify
doc describes a process for selectively backporting an upstream generator
improvement into a project that can no longer run `mcpify sync` safely.
mcpify's own `CHANGELOG.md` confirms the feature existed upstream first:
`[0.10.0] - 2026-07-18`, "Each API-version store is embedded
zstd-compressed in the Rust target, reducing binary size" — landed in
mcpify's template a day before this repo manually ported the same behavior.
Not present in mcpify's own `docs/DOC-GAPS.md`.

#### Resulting work

Manually ported zstd store-compression into this project (not via `mcpify
sync`, which would have overwritten the hand-rewritten TDS channel),
including updating the hand-written `resize_embeddings.rs` binary to add
its own decompress-on-demand step. Brought four API versions' worth of
embedded stores from well past crates.io's 10 MiB limit down to 4.5 MiB.

**(shared mcpify-template gap — corroborated by mcpify's own `CHANGELOG.md` [0.10.0], but not in `docs/DOC-GAPS.md`; same root theme as the `mcpify sync` entry above — see Lesson #4)**

### 2026-07-19 — EDA pipeline generated summary text used underscored identifiers, hurting embedding quality (v0.2.0)

#### Doc gap

This repo's own `generate_openapi.py` (part of the hand-built EDA pipeline,
documented only in `docs/sqlserver-eda-openapi-pipeline/README.md`) had no
note that `object_summary()`'s fallback text is exactly what gets embedded
for semantic search, and that the embedding model is trained on natural
English prose where an underscored identifier like "all_parameters" embeds
worse than the same words space-separated. Entirely repo-specific tooling.

#### Resulting work

Changed only the human-readable summary rendering to use spaces; left
identifiers (`operationId`, dict keys) underscored.

**(repo-specific)**

### 2026-07-19 — literal `[sandbox]` database qualifier sent to SQL Server (v0.2.0)

#### Doc gap

No doc for the EDA pipeline's "sandbox" placeholder explained that it
represents "whatever database the connection is already in," not a literal
database name. This is query-building logic inside the hand-built TDS
channel (`build_statement` in `api_client.rs`) — the exact kind of thing a
plain mcpify-generated HTTP-API-wrapper project has no equivalent of at
all, since it has no concept of SQL database qualification.

#### Resulting work

`build_statement` now two-part qualifies (`schema.name`) for sandbox-tagged
operations by default, relying on the connection's own current database; a
caller needing a specific database passes a top-level `database` argument.
`master`/`msdb` operations (real, always-present system databases) are
unaffected.

**(channel-adaptation-specific)**

### 2026-07-18 — profiling CPU/heap conflation (v0.2.0 boundary)

#### Doc gap

No mcpify doc described the requirement to keep DHAT allocator
instrumentation isolated from an ordinary release build used for CPU
sampling. mcpify's own `CHANGELOG.md` `[0.10.0] - 2026-07-18` — the exact
same day as this repo's fix — records "Separated generated CPU and heap
profiling in the Rust target (previously conflated into one workflow)" and
"Generated profiling workflows are now self-contained," confirming the
upstream template got the identical fix simultaneously. `docs/profiling.md`
(mcpify's own profiling doc) only covers profiling *mcpify itself*, not the
requirements the generated-project profiling *templates* must satisfy —
that gap is what let this ship un-isolated in the first place. Not present
in mcpify's own `docs/DOC-GAPS.md`.

#### Resulting work

Separated Samply CPU profiling from DHAT heap instrumentation in this
repo's `scripts/profile.sh`/`scripts/profile-heap.sh`, added a repeated warm
search workload with persisted benchmark details.

**(shared mcpify-template gap — corroborated by mcpify's own `CHANGELOG.md` [0.10.0], same-day fix, but not in `docs/DOC-GAPS.md`)**

### 2026-07-18 — SQL parameter binding accepted invalid/out-of-range values (v0.2.0 boundary)

#### Doc gap

No doc anywhere specified that SQL parameter binding in the hand-built
channel (`sql_type.rs`'s `json_to_param` and friends) must reject
invalid/out-of-range values rather than passing them through to TDS. This
is entirely inside the hand-built channel — a plain mcpify HTTP-wrapper
project's generic JSON-Schema-based input validation has no equivalent
SQL-type-binding step to get wrong.

#### Resulting work

Rejected invalid and out-of-range SQL parameter values; expanded CLI/MCP/
HTTP/SQL conversion test coverage.

**(channel-adaptation-specific)**

### 2026-07-17 — generated identifiers unwieldy, rename swept incompletely, and macOS ThinLTO incompatibility (v0.1.3)

#### Doc gap

No mcpify doc flags that deriving package/binary/env-var names from the
OpenAPI spec title can produce an unwieldy result
(`sql-server-2025-master-msdb-sandbox-combined-catalog`), or that a rename
needs to sweep every generated file consistently. No corroborating fix was
found in mcpify's `CHANGELOG.md` for the identifier-derivation half
specifically — this may be particular to how this project's OpenAPI title
was worded, but the underlying risk (a rename must sweep every generated
file, including non-obvious constants) is still a real, generic template
concern. The ThinLTO half is more serious and **still live**: mcpify's
current `src/targets/rust/templates/Cargo.toml.tera` (`[profile.dist]`)
still sets `lto = "thin"` unconditionally, confirmed by direct inspection —
the exact setting this repo had to disable locally at `v0.1.3` because
GitHub's macOS runner ships an Xcode/libLTO too old to parse ThinLTO
bitcode from the pinned rustc's LLVM version. No doc anywhere in mcpify
records this as a known incompatibility, and the template has not been
fixed.

#### Resulting work

Shortened every generated identifier in this repo to `sqlserver-mcp` and
its bin-name derivatives; fixed two `ENV_PREFIX` constants
(`auth_manager.rs`, `config_manager.rs`) missed on the first rename pass,
which would have silently ignored the new env var names. Disabled ThinLTO
in `[profile.dist]` — a local-only fix, never upstreamed.

**(shared mcpify-template gap — the ThinLTO half is confirmed still unresolved in mcpify's current template as of this writing; not found in mcpify's own `docs/DOC-GAPS.md`, and arguably the single most actionable fresh finding here since every other mcpify-generated Rust project remains exposed to it)**

### 2026-07-17 — missing `publish-crate.yml` in generated scaffolding (v0.1.4)

#### Doc gap

No mcpify doc flagged `publish-crate.yml` as expected scaffolding for every
crates.io-published mcpify-generated Rust repo — its absence here was only
discovered because a sibling repo in the same family already had one.
Confirmed the template `src/targets/rust/templates/.github/workflows/publish-crate.yml.tera`
exists in mcpify's current source, so this was fixed upstream at some
point, but no corresponding `docs/DOC-GAPS.md` entry records why it was
missing in the first place.

#### Resulting work

`v0.1.3` had to be published to crates.io by hand before this repo added
the workflow itself at `v0.1.4`.

**(shared mcpify-template gap — fixed upstream (template confirmed present today) but undocumented as a `docs/DOC-GAPS.md` entry)**

### 2026-07-17 — generated `Cargo.toml` defaults not dist-able out of the box (v0.1.2)

#### Doc gap

No mcpify doc noted that `publish = false` combined with missing crates.io
metadata causes cargo-dist to skip the package entirely. Confirmed this was
fixed upstream: mcpify's current `src/targets/rust/templates/Cargo.toml.tera`
already handles it via a `publish_registry`-conditional branch and an
explicit `[package.metadata.dist] dist = true` override, with a code
comment that describes the *exact* failure mode this repo hit ("`publish =
false` above makes cargo-dist skip this crate's binaries by default... this
override tells it they're still meant to ship"). That comment is, in
effect, the retrospective doc-gap writeup — it just never made it into
`docs/DOC-GAPS.md` where the next reader would actually look for it.

#### Resulting work

Added the crates.io metadata cargo-dist expects and regenerated
`release.yml`, which had drifted from the current template.

**(shared mcpify-template gap — fixed upstream (confirmed via the current template's own code comment) but undocumented as a `docs/DOC-GAPS.md` entry)**
