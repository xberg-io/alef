---
status: proposed
date: 2026-10-31
decision-makers: ["Project maintainers"]
---

# Publish source-bound draft releases before public promotion

## Context and Problem Statement

The published-release trigger exposed downloads before all five CLI archives existed. Its uploader
could undraft early and delete existing assets, and its finalizer only summarized results. Retries
must preserve verified public artifacts and distinguish the current controller from an older tag's source.

## Considered Options

* Publish a visible release first and repair missing assets afterward.
* Create a source-bound draft, verify selected CLI assets and crates, then promote.
* Implement one distributed transaction across GitHub, crates.io and optional taps.

## Decision Outcome

Chosen option: "Create a source-bound draft, verify selected CLI assets and crates, then promote",
because public visibility must follow verification without pretending separate registries are transactional.

Tag pushes and explicit dispatches run the controller at their immutable workflow revision. Existing tags
resolve to an immutable source SHA; Cargo version and exact archive names come from that source.
Only source-bound complete assets permit skipping builds. Dry runs still build and validate selected
artifacts, with no release, registry, tag or tap mutations. Legacy unbound state requires separately
reviewed repair rather than automatic clobbering, re-drafting or retagging.

Same-source uploads preserve matching source/digest-verified assets and add only missing archives.
Local tar/zip validation checks the exact regular nonempty CLI member without extracting files;
each native matrix leg separately smokes the binary's version, full source SHA and clean-tree banner.
Requested ABI, runtime and CLI crates are verified at the release version before public promotion.
Optional taps run only after public-ready verification and cannot re-draft a complete public release.

Stable promotion explicitly uses GitHub's server-side `make_latest: legacy`; prereleases and drafts
use `false`. This delegates creation-date/semantic-version selection to GitHub, not a strict maximum
SemVer policy. Per-tag concurrency does not serialize different tags, so no client-side Latest snapshot
controls promotion. Already-complete public retries do not PATCH release state.

### Consequences

* Good, because public CLI downloads become visible only after the complete verified asset set exists.
* Good, because same-source partial retries cannot delete or overwrite verified assets.
* Bad, because partial crates.io publication cannot be rolled back atomically; failures leave the release draft.
* Bad, because historical unbound releases need explicit repair and server-side Latest ordering remains GitHub policy.

### Confirmation

`scripts/tests/test_github_release_state.py` exercises offline recorded API state, bounded archive
validation, Latest interleavings and the actual workflow/DAG. `tests/publish_workflow_cli_asset_guard.rs`
checks source-derived matrix/archive equality and invokes that complete offline control suite.
Actionlint checks `.github/workflows/publish.yaml`; live matrix builds and registry/asset verification
remain required publication evidence, not a claim made by mocked unit tests.

## More Information

* [GitHub release update policy](https://docs.github.com/en/rest/releases/releases#update-a-release)
* Alef issue #761; repository ADR 0000 governs this decision log.
