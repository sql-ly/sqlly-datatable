# Agent Guidelines

## Commit policy

**NEVER COMMIT changes without first getting a clean result from `cargo fmt` and `cargo clippy`.**

Before every commit, in order:

1. `cargo fmt --all` — then verify with `cargo fmt --all --check` (must exit 0).
2. `cargo clippy -p sqlly-datatable --all-targets` — must be clean.
3. `cargo clippy -p sqlly-datatable-sample --all-targets` — must be clean.

Only the transitive `block v0.1.6` future-incompat warning is acceptable; treat any other warning or error as a failure and fix it before committing. If format or clippy is not clean, do NOT commit.
