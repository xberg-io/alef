---
type: Playbook
title: Release Procedure
description: "Cut, tag and publish an alef release: CHANGELOG roll-up, task set-version, lint, release commit, push, tag, gh release create, crates.io verification. Use when asked for an alef release, version bump or hotfix tag."
disable-model-invocation: true
x-ai-rulez:
  kind: skill
  id: release-procedure
  metadata:
    license: MIT
    name: release-procedure
---

# Alef Release Procedure

Every step has a check; do not skip one.

## Facts

- `gh release create` is what publishes: `Publish` triggers on `release: types: [published]`; a bare tag push runs nothing (v0.55.2 and v0.55.3 were tagged without a release and never reached crates.io).
- Three crates publish at the same version, in order: `alef-component-abi`, `alef-component-runtime`, `alef`. Check each registry version before retrying a partial publish.
- Versions change only via `task set-version -- X.Y.Z`, never by hand.
- `chore(release): X.Y.Z` carries only the version bump and changelog roll; fixes land in their own commits first, each with a regression test.
- No AI attribution in commit, tag or release text.

## 0. Pre-flight

`git status` (clean, or only release prep) and `git fetch origin`. If the branch diverged from upstream, stop and ask.

## 1. CHANGELOG.md

- Move every `## [Unreleased]` bullet into `## [X.Y.Z] - YYYY-MM-DD`; recreate an empty `## [Unreleased]`. Never release an empty section.
- Group under `### Added`, `### Changed (BREAKING)` (with migration guidance for `!` commits), `### Fixed`, `### Removed`.
- When folding drafted bullets, strip their heading lines; a stray heading reparents everything below it.
- Before and after any edit or `poly fmt` pass: `grep -c '^## \['` unchanged, `grep -c '^# '` equals 1. `poly.toml` `[fmt.markdown.rumdl] disable` includes `"MD025"` to stop rumdl demoting headings; keep it.

## 2. Version

`task set-version -- X.Y.Z` rewrites `Cargo.toml`, both component manifests and their dependency pins, `alef.toml` `alef_version`, `src/core/template_versions.rs::ALEF_REV`, regenerates `schemas/alef.schema.json`, and runs `cargo update`. Verify:

```bash
grep -E '^version' Cargo.toml crates/alef-component-abi/Cargo.toml crates/alef-component-runtime/Cargo.toml
grep -E '^alef_version' alef.toml
grep ALEF_REV src/core/template_versions.rs   # leading v
grep '"version"' schemas/alef.schema.json     # $id also carries vX.Y.Z
```

## 3. Lint and tests

`poly fmt --fix .` then `poly lint .`; re-stage rewritten files; fix real failures instead of `--no-verify`. Confirm every rolled-in `fix:`/`feat:` has a test.

## 4. Commit

```bash
git add -- CHANGELOG.md Cargo.toml Cargo.lock alef.toml schemas/alef.schema.json \
  src/core/template_versions.rs crates/alef-component-abi/Cargo.toml crates/alef-component-runtime/Cargo.toml
git commit -m "chore(release): X.Y.Z"
```

If a commit-msg hook (gitfluff) mangles the subject, fix the hook config; if the user authorized `--no-verify`, list the skipped hooks in the release notes.

## 5. Push, tag, release

Push `main` first, confirm it landed, then tag; a tag on a commit `origin/main` lacks needs delete-and-recreate.

```bash
git push origin main
git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin vX.Y.Z
gh release create vX.Y.Z --title "vX.Y.Z" --notes-from-tag --verify-tag
```

For notes from the changelog use `--notes-file <(awk '/^## \[X.Y.Z\]/,/^## \[/' CHANGELOG.md | head -n -1)`. Add `--prerelease` for RC/beta.

## 6. Verify the registry

```bash
gh release view vX.Y.Z && git ls-remote --tags origin vX.Y.Z
gh run list --workflow=publish.yaml --json databaseId,status,conclusion -L 5
gh run view <run-id> --json jobs --jq '.jobs[] | select(.name | test("crates")) | {name, conclusion}'
for crate in alef-component-abi alef-component-runtime alef; do
  curl -fsS "https://index.crates.io/al/ef/${crate}" \
    | jq -se --arg version X.Y.Z 'any(.[]; .vers == $version and .yanked == false)' || exit 1
done
```

- A failed job may already have published; check the index before rerunning (`gh run rerun --failed <run-id>`).
- A skipped job inside a green run can mean the crate never moved; read job conclusions.
- A `Publish` run stuck in `queued` is usually account-wide runner capacity; report it once, and if wedged `gh run cancel --force-cancel <run-id>` before retrying.

## 7. After

- Bump consumer pins in separate follow-up PRs.
- Optional: `cargo install --path . --force`, then `task clean` (`cargo clean` + `rm -rf .alef/`).
