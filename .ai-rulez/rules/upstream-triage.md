---
priority: high
---

When a binding gap or codegen issue is surfaced from a downstream consumer, triage to the correct upstream repo and execution path.

**Decision tree:**

- **Alef codegen bug** (issue reproduces in alef's own e2e tests or `src/extract/` misses an item): fix in `../alef`, update `CHANGELOG.md` `[Unreleased]`, commit normally, prepare a release per `release-procedure` skill.
- **Shared GH Action bug** (publish.yaml, pre-commit hook, or scaffold script has an error): fix in `../actions`, commit, then force-retag `v1.0.0` and `v1` to the new commit and force-push *only those two tags* (`git tag -f v1.0.0 && git tag -f v1 && git push -f origin v1.0.0 v1`). Do not bump the `main` branch version.
- **Consumer repo config gap** (downstream alef.toml is missing an `exclude_types` or has the wrong `[crates.lang]` override): fix the consuming repo's `alef.toml`; do not touch alef or actions.

**Hard rule:** Always update `CHANGELOG.md` `[Unreleased]` on every upstream commit. Never merge a fix without a changelog entry. Use `--no-verify` only when a hook is genuinely broken and you re-run hooks afterwards to verify the fix.
