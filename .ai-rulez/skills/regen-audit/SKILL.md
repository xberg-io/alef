---
type: Playbook
title: Regen Audit
description: "Treat alef generate/all/verify in a consumer repo as an audit: read every warning, prove the cache did not skip work, split hash churn from real diffs. Use when regenerating a consumer repo or when a codegen fix seems to have no effect."
x-ai-rulez:
  kind: skill
  id: regen-audit
  metadata:
    license: MIT
    name: regen-audit
---

# Regen Is an Audit

`alef generate` / `alef all` exits 0 while warning about conditions that produce wrong bindings. Every warning is a finding.

## Use the local binary

- After every change under `src/`, run `cargo install --path . --force` from the alef root; `~/.cargo/bin/alef` shadows a crates.io install.
- Check `which alef` and `alef --version`. Do not `cargo install alef` from crates.io until the change is released.

## Rules

1. Capture the full log to a file. Grep for `warn`, `error`, `skip`, `refus`, `Unavailable`, `Unsupported`, `0 files`, and zero or suspiciously round counts; a config gap can disable a whole validation pass that still reports success.
2. Prove the generator ran. `compute_inputs_hash` (`src/core/hash.rs`) excludes `ALEF_REV`, and `strip_alef_version_pin` strips `alef_version` before hashing, so changing a version pin does not change the cache key. The invalidation lever is `CODEGEN_FORMAT_VERSION` (`src/core/template_versions.rs`). A handful of files where thousands were expected is a cache hit: check `.alef/hashes/` and `.alef/<crate>/` timestamps or use `--cache off`.
3. When an error is confusing, re-run with `RUST_LOG=alef=debug` and grep `refusing to write`. A pre-existing consumer file without an `alef:hash:` marker stays un-owned until `alef adopt <path> --write`.
4. Split provenance churn from real changes: most files change only their `alef:hash:` line (the per-file hash folds in a global `inputs_hash`). Read `git diff --numstat | awk '$1>2 || $2>2'`. Never discard this uncommitted state.
5. Check both version pins: `alef.toml` `[workspace] alef_version` and `.github/workflows/*.yaml` `alef-version:`; they drift.
6. Never run two `alef snippets check` (or other validation) passes concurrently on one repo; they share `.alef/snippets` session dirs and produce fake failures.

## Triage each finding

- Alef codegen/extraction bug (reproduces in alef's e2e tests, or `src/extract/` misses an item): fix in alef with a `CHANGELOG.md` `[Unreleased]` entry and a regression test.
- Alef-owned workflow/action bug: fix that repository, retag only its documented action tags.
- Consumer config gap (missing `exclude_types`, wrong `[crates.<lang>]` override): fix the consumer's `alef.toml`, not alef.
- Do not carry a warning across releases because "it has always been there".
