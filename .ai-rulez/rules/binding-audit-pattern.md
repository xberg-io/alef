---
priority: high
---

Audit bindings for coverage gaps — is every public Rust item exposed in every target language binding?

**Steps:**

1. Enumerate intentional removals in the source repo's `alef.toml`: `[crates.exclude]`, `[crates.skipped]`, `exclude_types`, `[crates.opaque_types]`, per-crate overrides under `[workspace.crates."<name>"]`. Record these.
2. Grep the source Rust crate for `#[alef::skip]`, `#[alef::exclude]`, `#[alef::opaque]` attributes — these are intentional removals; do not flag them.
3. For every public Rust item (functions, types, methods, enum variants) not captured in steps 1–2, verify its presence across every generated binding under `packages/<lang>/`.
4. Diff across targets: identify items present in some bindings but not others. That gap is the bug.
5. **Triage outcomes:**
   - **Codegen issue** → fix in alef repo (`../alef`), update `CHANGELOG.md` `[Unreleased]`, commit, normal release flow.
   - **Shared action bug** → fix in `../actions`, commit, then retag `v1.0.0` and `v1` on the new commit and force-push *only those two tags*.
   - **Per-target config gap** → fix the downstream repo's `alef.toml`; do not touch alef or actions.
6. Always update `CHANGELOG.md` `[Unreleased]` on each upstream commit; use `--no-verify` only when hooks block a critical fix and re-run hooks afterwards.
