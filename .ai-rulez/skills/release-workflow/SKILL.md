---
name: release-workflow
description: >-
  Release/publish the alef CLI crate end-to-end. Load when releasing or
  publishing alef — cutting a new version, tagging, running `gh release create`,
  and installing the released build locally. Covers version set via Taskfile,
  CHANGELOG roll, the clean-tree precondition, GitHub release, local install, and
  artifact cleanup.
---

# Alef Release Workflow

Ground truth: alef ships as a single root-flat crate (binary `alef`). One
`cargo publish` per tag — no multi-crate sequencing. The comprehensive
step-by-step lives in the `release-procedure` skill; this is the canonical
publish sequence.

## 1. Set the version

```bash
task set-version -- X.Y.Z        # e.g. task set-version -- 0.18.0 (or 0.18.0-rc.1)
```

`set-version` takes the version as a `--` positional argument (CLI_ARGS), strips
a leading `v`, validates semver, and rewrites `Cargo.toml`, `alef.toml`
(`alef_version`), and `src/core/template_versions.rs::ALEF_REV` in one shot, then
regenerates the schema and runs `cargo update`. Never hand-edit those files.

Verify:

```bash
grep -E '^version' Cargo.toml
grep -E '^alef_version' alef.toml
grep ALEF_REV src/core/template_versions.rs
```

## 2. Update the CHANGELOG

Move every bullet under `## [Unreleased]` in `CHANGELOG.md` into a new
`## [X.Y.Z] - YYYY-MM-DD` section (grouped `### Added`, `### Changed (BREAKING)`,
`### Fixed`, `### Removed`). Re-create an empty `## [Unreleased]`. Never tag an
empty section.

## 3. Clean-tree precondition (hard gate)

Never release a dirty or failing tree.

```bash
poly fmt --check .    # formatting clean
poly lint .           # lint clean
task test             # cargo test --workspace passes
```

Use `poly fmt --fix .` to apply formatting, then re-stage. If lint or tests fail,
fix the cause — do not release past a failure.

## 4. Commit, tag, and publish the GitHub release

```bash
git add -A
git commit -m "chore(release): X.Y.Z"
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin main
git push origin vX.Y.Z
gh release create vX.Y.Z --title "vX.Y.Z" --notes-from-tag --verify-tag
```

Add `--prerelease` for RC/beta tags. The tag push triggers the `Publish`
workflow (`cargo publish` once). A bare `git tag` is not a release — always run
`gh release create`.

## 5. Install the released build locally

```bash
cargo install --path . --force
```

Run from the alef repo root. `--path .` installs the `alef` binary from the
workspace root; the result shadows any crates.io install on `$PATH`. Confirm:
`which alef` points to `~/.cargo/bin/alef` and `alef --version` reflects X.Y.Z.

## 6. Clean up build artifacts

```bash
task clean        # cargo clean + rm -rf .alef/
```

`task clean` runs `cargo clean` (removing `target/` to reclaim space) plus
`rm -rf .alef/`.

## Anti-patterns

- Hand-editing `version` in `Cargo.toml`, `alef_version` in `alef.toml`, or
  `ALEF_REV` instead of `task set-version -- X.Y.Z`.
- Releasing a dirty or lint/test-failing tree.
- Tagging without `gh release create`.
- AI attribution in commit/tag/release text.
