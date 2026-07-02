---
priority: high
---

Short-form rules for publishing alef. The comprehensive procedure lives in the `release-procedure` skill.

Alef is a single root-flat crate as of 0.18.0. Publishing is a single `cargo publish` — no multi-crate sequencing, no crates.io index propagation race.

1. **Version sync** — always use `task set-version -- X.Y.Z` to bump the version. This rewrites `Cargo.toml`, `alef.toml`, and `src/core/template_versions.rs::ALEF_REV` in one shot and runs `cargo update`.
2. **CHANGELOG roll** — move all `[Unreleased]` entries into a new `[X.Y.Z] - YYYY-MM-DD` section. Group under `### Added`, `### Changed (BREAKING)`, `### Fixed`, `### Removed`.
3. **Lint pass** — run `poly fmt --fix .` then `poly lint .` until clean. Re-stage any files the formatter rewrites.
4. **Commit split** — keep commits atomic: dependency bumps separately, feature/fix commits separately, `chore(release): vX.Y.Z` last. Never squash release prep.
5. **Tag and push** — `git tag -a vX.Y.Z -m "vX.Y.Z"` and `git push origin main && git push origin vX.Y.Z`.
6. **GitHub release** — `gh release create vX.Y.Z --title "vX.Y.Z" --notes-file <extracted CHANGELOG section>`. The tag push triggers the `Publish` workflow, which runs `cargo publish` once.
7. **Monitor workflows** — both the `CI` workflow on `main` and the `Publish` workflow on the tag must go green. Never assume they will.

See `release-procedure` skill for the full step-by-step.
