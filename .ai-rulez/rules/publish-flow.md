---
type: Decision
title: Publish Flow
x-ai-rulez:
  kind: rule
  id: publish-flow
  metadata:
    priority: high
---

Alef is a single root-flat crate as of 0.18.0: one release means exactly one `cargo publish`, no multi-crate sequencing. Version sync uses `task set-version -- X.Y.Z` only — never hand-edit `Cargo.toml`, `alef.toml`, or `ALEF_REV`. Push `main` before creating the tag, not after. **`gh release create` is what publishes** — `Publish` triggers on `release: types: [published]`, not on the tag; a bare tag push runs nothing. This is not theoretical: v0.55.2 and v0.55.3 were tagged and pushed with no release created and never reached crates.io. A green `Publish` run is not proof the crate shipped — verify the crates.io index, not just the release object.

Full step-by-step, CHANGELOG folding rules, and registry verification: `release-procedure` skill.
