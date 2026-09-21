# What actionlint-rs checks (and what it doesn't)

This is the map of everything the linter looks at. Use it to answer one question quickly:

> **"It didn't catch my problem — is that a bug, or does it just not check for that?"**

If your problem is in the **"We check this"** tables below and it wasn't flagged, that's a bug —
please [open an issue](https://github.com/jreed91/actionlint-rs/issues) with the workflow snippet.
If it's in the **"We don't check this (yet)"** list, it's a known gap, not a bug.

Every finding carries a **rule id** (e.g. `enum/permissions`). You'll see it in `--format sarif`
and `--format json` output, and you can silence any rule by message with `--ignore` or the
config file. The examples below show a minimal snippet that triggers each check.

---

## The 30-second version

- **On by default:** structure, `${{ }}` expressions, job graph, `uses:` format, dataflow
  (`steps`/`needs` references), reusable-workflow calls, events, permissions/shell enums, globs,
  security & misc, constant `if:`.
- **Needs a tool installed:** `run:` script linting (shellcheck / pyflakes). Silently skipped if
  the tool isn't on your `PATH`.
- **Opt-in (off by default):** runner-label checking (`--check-runner-labels`).
- **Not checked at all:** anything requiring the network (resolving a remote action's inputs,
  deprecated-action-version lists) and a handful of semantic checks listed at the bottom.

Turn anything off with `--ignore '<regex against the message>'` or the `ignore:` list in
`.github/actionlint.yaml`.

---

## We check this (on by default)

### Structure — is the YAML shaped like a valid workflow?

Validated against GitHub's own schema (see [Schema source](#schema-source)). This is the
"does this key exist / is it the right type / is it required" layer.

| Rule id | Catches | Example that trips it |
|---|---|---|
| `structure/required` | A required key is missing | a job with `steps:` but no `runs-on:` |
| `structure/type` | A value is the wrong type | `on: 42` |
| `structure/enum` | A value isn't one of the allowed choices | an invalid enum value the schema pins |
| `structure/pattern` | A value doesn't match the required format | a malformed string field |
| `structure/additional-properties` | An unknown / typo'd key | `jobs.build.runss-on:` |
| `structure/one-of` | A value matches none of the allowed shapes | a job that's neither a normal job nor a reusable call |

> Nice touch: a job with `with:`/`secrets:` but no `runs-on`/`uses` is reported as **missing
> `uses`** (you meant a reusable-workflow call), not the confusing "missing runs-on".

### Expressions — do your `${{ }}` expressions make sense?

Every `${{ }}` is parsed and type-checked against GitHub's contexts and functions.

| Rule id | Catches | Example |
|---|---|---|
| `expression/syntax` | The expression doesn't parse | `${{ github. }}` |
| `expression/type` | Unknown context / property / function, or wrong arity | `${{ gihtub.sha }}`, `${{ runner.oss }}`, `${{ contains('x') }}` |

Context **availability** is enforced too: using `secrets` in `runs-on` (where it isn't
available) is flagged.

### Dataflow — do your `steps`/`needs` references point at something real?

| Rule id | Catches | Example |
|---|---|---|
| `dataflow/step-output` | `steps.<id>` where no step in the job has that `id:` | `${{ steps.biuld.outputs.x }}` when the step is `id: build` |
| `dataflow/needs-output` | `needs.<job>` where `<job>` isn't in this job's `needs:` | using `needs.setup.outputs.x` without `needs: [setup]` |

> We check that the **step id / job exists**, not that a specific output *name* exists — a
> `run:` step's outputs are written dynamically, and a `uses:` step's outputs would need the
> action's `action.yml` (which we don't fetch). See the gaps list.

### Reusable workflows — does your `workflow_call` match the callee?

For a **local** callee (`uses: ./.github/workflows/x.yml`), we read the called workflow and
check your call against its declared contract.

| Rule id | Catches | Example |
|---|---|---|
| `reusable/input` | A missing required input, or an unknown one | calling a workflow without its required `environment:` input |
| `reusable/secret` | A missing required secret, or an unknown one | omitting a required secret (unless you use `secrets: inherit`) |

> Remote callees (`org/repo/....yml@ref`) are **skipped** — resolving them needs the network.

### Job graph — do your `needs:` make sense?

| Rule id | Catches | Example |
|---|---|---|
| `graph/needs` | A `needs:` on a job that doesn't exist, or a dependency cycle | `needs: [ghost]`, or A→B→A |

### Action references — is your `uses:` well-formed?

| Rule id | Catches | Example |
|---|---|---|
| `uses/format` | A `uses:` that isn't `owner/repo@ref`, `./local`, or `docker://image` | `uses: actions/checkout` (no `@ref`) |

### Events — is your `on:` real?

| Rule id | Catches | Example |
|---|---|---|
| `event/name` | A misspelled / non-existent event | `on: pull-request` (it's `pull_request`) |
| `event/types` | An invalid activity type, or `types:` on an event that has none | `types: [opend]`, or `push: { types: [...] }` |

### Enums — values GitHub restricts but the schema leaves loose

| Rule id | Catches | Example |
|---|---|---|
| `enum/permissions` | A bad permission scope or level (per-scope: `id-token` only allows `write`/`none`) | `permissions: { contents: readonly }`, `id-token: read` |
| `enum/shell` | An unknown `shell:` | `shell: fish` |

### Globs — filter patterns that can't match

| Rule id | Catches | Example |
|---|---|---|
| `glob/syntax` | An unclosed `[` character class or an empty pattern in `branches`/`tags`/`paths` | `branches: ["v[0-9"]` |

> Deliberately narrow — GitHub's glob dialect has subtle rules, so we only flag the
> unambiguous errors to avoid false alarms.

### Security & misc

| Rule id | Catches | Example |
|---|---|---|
| `security/script-injection` | Untrusted input interpolated into a `run:` script | `run: echo ${{ github.event.pull_request.title }}` |
| `security/hardcoded-credential` | A literal password in a `credentials:` block | `credentials: { password: hunter2 }` |
| `deprecated/workflow-command` | `::set-output::` / `::save-state::` | `run: echo ::set-output name=x::1` |
| `misc/cron` | A malformed `schedule.cron` | `cron: "99 0 * * *"` |
| `misc/constant-if` | An `if:` that's always true/false | `if: false`, `if: ${{ true }}` |

---

## We check this — but only if a tool is installed

`run:` blocks are linted by external tools, auto-detected on your `PATH`. If the tool isn't
installed, the check is **silently skipped** (never an error). Disable with `--no-external`.

| Rule id | Tool | Lints | Notes |
|---|---|---|---|
| `run/shellcheck` | [shellcheck](https://www.shellcheck.net/) | bash / sh / dash / ksh `run:` steps | `${{ }}` expressions are neutralized first, so GitHub expressions don't cause bogus shell-syntax errors |
| `run/pyflakes` | [pyflakes](https://pypi.org/project/pyflakes/) | `shell: python` `run:` steps | same expression handling |

> **Seeing shell warnings you don't expect?** Those are shellcheck's own opinions (e.g. SC2086
> "quote to prevent word splitting"), not our rules. Run `--no-external` to confirm, and
> address them in your script or `--ignore` them.

---

## We check this — but only if you ask (opt-in)

| Flag | Rule id | Catches | Why it's opt-in |
|---|---|---|---|
| `--check-runner-labels` | `runner/label` | A `runs-on:` label that isn't a known GitHub-hosted label or a declared self-hosted one | Larger-runner and self-hosted labels are unbounded and project-specific, so on-by-default would false-positive. Declare yours under `self-hosted-runner.labels` in the config. |

---

## We DON'T check this (known gaps, not bugs)

If your issue is here, the linter isn't broken — it just doesn't cover this yet. Tracked in
[`docs/ROADMAP.md`](ROADMAP.md).

- **Remote action inputs** — we don't fetch a `uses: owner/repo@ref` action's `action.yml`, so
  we can't check its `with:` inputs. (The *local reusable-workflow* equivalent **is** checked.)
  Needs network.
- **Deprecated action versions** — no "this action version is EOL" list. A reliable one is
  network-sourced and time-sensitive; a hardcoded list would go stale and misfire.
- **Output-name existence** — we verify `steps.<id>` / `needs.<job>` *exist*, but not that a
  specific `.outputs.<name>` was actually produced (it's dynamic / needs action metadata).
- **Reusable-workflow outputs** — we type inputs and secrets, not the callee's declared outputs.
- **Deep glob semantics** — only unclosed/empty patterns are flagged, not every subtle
  filter-pattern mistake.
- **Anything runtime** — whether a secret is actually set, whether a matrix value is valid at
  run time, whether an external service is reachable, etc.

Want one of these? Check the roadmap, or open an issue.

---

## Turning checks off

Three ways, in increasing scope:

```sh
# One run, by message regex (repeatable):
actionlint-rs --ignore 'unknown runner label' --ignore 'SC2086'
```

```yaml
# .github/actionlint.yaml — committed, shared by the whole team:
ignore:
  - 'unknown runner label'
self-hosted-runner:
  labels: [linux-arm64-16core]   # accepted by --check-runner-labels
```

```sh
actionlint-rs --no-external      # turn off shellcheck / pyflakes entirely
actionlint-rs --no-config        # ignore the config file
```

---

## Schema source

The structure and enum layers derive from **GitHub's own first-party workflow schema**
(transpiled to JSON Schema at runtime) by default. `--schema schemastore` switches to the
community [SchemaStore](https://json.schemastore.org/github-workflow.json) schema. Both are
vendored in the repo and kept current by an automated resync pipeline. Deriving the permission
scopes and event names from these schemas (rather than hardcoding them) is why those checks
stay accurate as GitHub evolves.

---

*This matrix is verified against the code: every rule id above appears in `src/`, and the
corpus (`corpus/`) exercises them with per-file snapshot tests. If you find a mismatch between
this doc and actual behavior, that's a doc bug — please report it.*
