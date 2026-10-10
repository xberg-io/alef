---
type: Playbook
title: Regen Audit
description: Treat running `alef generate`/`alef all`/`alef verify` in a consumer repo as an audit, not a build step. Use this skill whenever you run a regen, read its log or diff, or investigate "the fix didn't work" after regenerating a consumer repo's bindings.
x-ai-rulez:
  kind: skill
  id: regen-audit
  metadata:
    license: MIT
    name: regen-audit
---

# Regen Is an Audit

`alef generate` / `alef all` exits 0 while warning about conditions that produce silently-wrong
bindings. Read the whole log; every warning and error is a finding to triage, not noise to skim
past for the exit code.

## When to apply

- Running `alef generate`, `alef all`, or `alef verify` in a consumer repo
- Reading a regen log or a regen diff
- Investigating "I fixed the codegen bug but the consumer still shows the old behavior"
- A/B-testing two alef versions or two config states against the same repo

## Hard rules

1. **Capture the full log**, don't let it scroll. Sweep it for `warn`, `error`, `skip`, `refus`,
   `Unavailable`, `Unsupported`, `0 files`, and any count that is suspiciously round or zero. A
   whole validation pass can be silently disabled by a config gap and still report success.
2. **Prove the generator actually ran before trusting a diff (or a lack of one).**
   `compute_inputs_hash` (`src/core/hash.rs`) deliberately excludes `ALEF_REV`, and
   `strip_alef_version_pin` strips `alef.toml`'s `alef_version` before hashing — so bumping a
   consumer's version pin, or flipping it back and forth for an A/B test, changes nothing about
   the cache key. The real invalidation lever is `CODEGEN_FORMAT_VERSION`
   (`src/core/template_versions.rs`), bumped rarely. A regen that produces a handful of files when
   thousands were expected is very likely a cache hit that examined nothing, not a "no diff"
   result — check `.alef/hashes/`/`.alef/<crate>/` timestamps against the run, or force
   `--cache off`, before concluding two versions or two configs behave identically.
3. **When alef's own error is confusing, check whether a more accurate one already exists and
   never reached you.** Re-run with `RUST_LOG=alef=debug` and grep for `refusing to write` before
   believing the headline error. A pre-existing consumer file with no `alef:hash:` marker is
   invisible-by-default and permanently un-owned until `alef adopt <path> --write` — a confusing
   downstream symptom (a missing generated function, a stale forwarded feature) can trace back to
   this rather than to a codegen bug.
4. **Split provenance churn from real changes before reading a diff.** Consumer repos routinely
   sit with hundreds to thousands of uncommitted files mid-regen; the large majority typically
   change only their `alef:hash:` line (the per-file hash folds in a global `inputs_hash`, so any
   source change rehashes every file). That's expected, not a bug:

   ```bash
   git diff --numstat | awk '$1>2 || $2>2 {print $1"\t"$2"\t"$3}'   # the changes worth reading
   ```

   Never discard this uncommitted state (see `safe-git-operations`) — it is normal, not damage.
5. **Check both version pins.** Consumer repos carry two: `alef.toml` `[workspace] alef_version`
   and `.github/workflows/*.yaml` `alef-version:`. These drift apart; grep for both when diagnosing
   a "which alef version actually ran" question.
6. **Never run two `alef snippets check` (or other validation) passes concurrently against one
   repo.** They share `.alef/snippets` session directories; contention produces fake per-language
   failures that look exactly like real ones.

## Procedure

1. Run the regen, capturing full output to a file.
2. Grep the log per Hard rule 1. Triage every hit via the `upstream-triage` rule: consumer
   `alef.toml` misconfiguration → fix the consumer repo; alef codegen/extraction → fix `../alef`
   with a `CHANGELOG.md` entry and a regression test.
3. If a diff looks smaller or larger than expected, apply Hard rule 2 before trusting it either
   way.
4. If an error is confusing, apply Hard rule 3 before accepting the headline message.
5. Read the diff via Hard rule 4's split — review the real changes, not the churn.
6. Do not carry a warning forward across releases on the theory that "it has always been there" —
   an un-owned warning is how a check silently rots.

## Anti-patterns

- Treating a successful exit code as proof the run examined anything.
- Flipping a version pin back and forth as an A/B test and trusting the result without proving the
  cache actually invalidated.
- Discarding a consumer repo's uncommitted regen output because it "looks like a huge unrelated
  diff."
- Running a second validation pass against a repo while one is already in flight.
