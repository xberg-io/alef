---
priority: high
---

When a binding gap or codegen issue is surfaced from a consumer repo, triage to the correct upstream repo and execution path.

**Decision tree:**

- **Alef codegen bug** (issue reproduces in alef's own e2e tests or `src/extract/` misses an item): fix in `../alef`, update `CHANGELOG.md` `[Unreleased]`, commit normally, prepare a release per `release-procedure` skill.
- **Alef-owned workflow/action bug** (an Alef-maintained publish workflow, poly hook, or scaffold action has an error): fix the owning Alef workflow/action repository, commit, then retag only the documented action tags for that repository. Do not send unrelated consumer workflow issues to a hard-coded actions repository.
- **Consumer repo config gap** (consumer `alef.toml` is missing an `exclude_types` or has the wrong `[crates.lang]` override): fix the consuming repo's `alef.toml`; do not touch alef or actions.

**Hard rule:** Always update `CHANGELOG.md` `[Unreleased]` on every upstream commit. Never merge a fix without a changelog entry. Use `--no-verify` only when a hook is genuinely broken and you re-run hooks afterwards to verify the fix.
