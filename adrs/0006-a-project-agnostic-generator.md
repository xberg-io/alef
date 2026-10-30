---
status: accepted
date: 2026-05-27
decision-makers: ["Na'aman Hirschfeld"]
---

# A project-agnostic generator

## Context and Problem Statement

alef is built for the organization's own Rust libraries but published as a general binding generator for any Rust
library (`Cargo.toml` describes it as an "Opinionated polyglot binding generator for Rust libraries"). Generator
code had started to special-case those consumers by name and by their domain types: 159402386 (2026-06-02, "fix:
remove consumer-specific codegen leakage") removed such branches, and the guard commit itself renamed consumer names
out of backend code and test fixtures.

## Considered Options

* Keep the generator free of consumer identities, and model consumer differences through generic configuration
* Special-case known consumers by name in the generator

## Decision Outcome

Chosen option: "Keep the generator free of consumer identities, and model consumer differences through generic
configuration", because that is the rule the guard added in 186e17779 (2026-05-27) enforces; no commit states a
rationale beyond the rule itself: "Alef must stay project-agnostic; model downstream differences through `alef.toml` or
generic config, not project-name special cases."

The `project-agnostic-codegen` rule, added in the same commit, sets the scope:

* No consumer repository, package, module or path names, and no product branches, in generator, extraction,
  scaffold, e2e, publish or CLI code.
* Differences are modeled through `alef.toml`, IR metadata, backend language configuration, feature flags and
  capability flags, named for the generic condition rather than the consumer.
* Tests, fixtures, snapshots and docs use neutral sample names.

### Consequences

* Good, because any Rust library can adopt alef without inheriting another project's special cases.
* Good, because a consumer's need becomes a configuration option that every consumer can use.
* Bad, because a quick fix for one consumer must be generalized first, and neutral fixtures must be renamed before
  a snapshot is accepted.

### Confirmation

* `hooks/check_project_mentions.py` rejects consumer names and hard-coded downstream domain types in enforced files.
* `tests/project_mentions_hook.rs` and `tests/cli_no_project_special_casing.rs:37` run that hook in the test suite.
* `.ai-rulez/rules/project-agnostic-codegen.md` states the rule.
