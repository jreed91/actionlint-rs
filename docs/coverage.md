# Test coverage policy

**Requirement:** 100% coverage of *reachable* library code.

- Measured with `cargo llvm-cov --lib`.
- `src/main.rs` (CLI process glue — argument parsing, stdin, exit codes, filesystem
  discovery) is **excluded** from the library coverage number and is instead exercised by
  integration tests in `tests/cli.rs` that run the built binary.
- CI gate: `cargo llvm-cov --lib --fail-under-lines 98 --fail-under-regions 98`.

## Why the floor is 98%, not 100%

A handful of library lines are **provably unreachable** through the public API and are left
in place deliberately (defensive robustness), so they cannot be covered without either
removing safety code or writing tests that never actually execute them:

| File | Line(s) | Why unreachable |
|------|---------|-----------------|
| `src/yaml.rs` | `to_json` `_ =>` arm | saphyr's default loader parses scalars into `Value`, resolves aliases in place, and errors out of `parse()` rather than emitting `Representation`/`Alias`/`BadValue`. |
| `src/yaml.rs` | `key_to_string` `_ =>` arm | Same loader invariant; workflow mapping keys are always scalars. |
| `src/span.rs` | `key_matches` `_ =>` arm | Same. |
| `src/schema.rs` | `assert!` message | The failure-message expression only executes when the assertion fails (i.e. never, in a passing suite). |
| `src/lint.rs`  | `assert!` message | Same. |

These are documented rather than deleted: keeping the defensive arms is worth more than a
vanity 100%. If saphyr's loader behavior changes in a future upgrade, the arms become
reachable and should then be tested directly.
