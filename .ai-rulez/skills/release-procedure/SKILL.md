---
type: Playbook
title: Release Procedure
description: "Cut, tag and publish an alef release: CHANGELOG roll-up, mise run set-version, lint, release commit, push, tag, source-bound draft promotion and crates.io verification. Use when asked for an alef release, version bump or hotfix tag."
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

- `Publish` triggers on `push.tags: [v*]`, `workflow_dispatch`, or `repository_dispatch: publish-release`. Push the release tag after the source commit. Do not create or undraft a release manually: the workflow owns draft creation, source-bound assets and promotion (ADR 0009).
- Three crates publish at the same version, in order: `alef-component-abi`, `alef-component-runtime`, `alef`. Check each registry version before retrying a partial publish.
- Versions change only via `mise run set-version X.Y.Z`, never by hand.
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

`mise run set-version X.Y.Z` rewrites `Cargo.toml`, both component manifests and their dependency pins, `alef.toml` `alef_version`, `src/core/template_versions.rs::ALEF_REV`, regenerates `schemas/alef.schema.json`, and runs `cargo update`. Verify:

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

Push `main` first, confirm it landed, then tag. If a release tag is wrong, stop for separately reviewed recovery; never move it automatically.

```bash
git push origin main
git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin vX.Y.Z
```

The workflow binds the existing tag to its immutable Cargo version and CLI target list, creates a draft,
builds and smokes every selected native CLI target, uploads only missing verified archives, verifies
all three requested crate versions, then promotes. A prerelease version produces a prerelease;
never change classification manually. Preserve the `alef-release-state` body marker when adding human notes.

Stable promotion sends GitHub's `make_latest: legacy`; prereleases send `false`. Latest therefore
follows GitHub's documented creation-date/semantic-version policy, not a custom strict maximum-SemVer
ordering. Different tags may publish concurrently; the controller does not make a stale client-side
`GET latest` comparison.

For a retry or dry run of an **existing** tag, dispatch the current controller on main:

```bash
gh workflow run publish.yaml --ref main -f tag=vX.Y.Z -f targets=crates,cli -f dry_run=true
```

Dry runs build and validate selected CLI artifacts even when a complete release already exists,
but write no release, tag, registry or tap state. A complete source-bound public CLI release is not
edited on retry; selected missing crates or tap updates still run. Partial same-source drafts upload only missing assets. Legacy/unbound releases, mismatched source
or assets, and force-clobber requests fail closed. Repair needs a separately reviewed scope;
never delete assets, move a tag, or re-draft a public release to bypass the guard.

Homebrew/Scoop are explicit opt-ins and require already-public complete CLI assets. Tap-only retries
never manufacture a release, and optional tap failures cannot hide a successfully published core release.

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

- A failed job may already have published individual crates. Check all three registry versions before retrying. Use the current workflow with an existing tag for source-bound missing-only retries; re-running a legacy workflow retains its legacy behavior.
- A skipped job inside a green run can mean the crate never moved; read job conclusions.
- A `Publish` run stuck in `queued` is usually account-wide runner capacity; report it once, and if wedged `gh run cancel --force-cancel <run-id>` before retrying.

## 7. After

- Bump consumer pins in separate follow-up PRs.
- Optional: `cargo install --path . --force`, then `mise run clean` (`cargo clean` + `rm -rf .alef/`).
