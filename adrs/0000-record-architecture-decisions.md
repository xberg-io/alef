---
status: accepted
date: 2026-10-30
decision-makers: ["Na'aman Hirschfeld"]
---

# Record architecture decisions

## Context and Problem Statement

alef's lasting design choices (how it reads a crate, how values cross the C ABI, which files it owns, how backends
emit code) are recorded only in commit messages, the CHANGELOG, `.ai-rulez/` rules and code comments. None of these
says what was replaced, and nothing marks when a decision changes. The organization adopted a MADR log checked by
adrlint in
[org ADR-0000 (Record architecture decisions)](https://github.com/xberg-io/agent-conventions/blob/main/adrs/0000-record-architecture-decisions.md),
which expects each repository to keep its own decisions in its own log.

## Considered Options

* A repository log of MADR records in `adrs/`, checked by adrlint, as org ADR-0000 sets out

No alternatives were recorded.

## Decision Outcome

Chosen option: "A repository log of MADR records in `adrs/`, checked by adrlint, as org ADR-0000 sets out", because
it gives each alef decision one immutable record that a later change supersedes or amends instead of overwriting.

Records follow MADR 4 as `adrlint new` generates it, numbered from 0000. Records 0001 onward are seeds: decisions
already in force when the log started, dated to the commit that made each one. Org-wide decisions are cited as
`org ADR-NNNN`, not restated; the decision that the libraries generate their bindings with alef is
[org ADR-0002 (Rust cores with alef-generated bindings)](https://github.com/xberg-io/agent-conventions/blob/main/adrs/0002-rust-cores-with-alef-generated-bindings.md).
The index is `docs/adr-index.md`, outside the log.

### Consequences

* Good, because a reviewer can find why the generator works the way it does, and what it replaced.
* Good, because adrlint checks the shape and numbering of every record, and, once the log is locked, its
  immutability.
* Bad, because the seed records are written after the fact, so their context is limited to what the history shows.

### Confirmation

`.adrlint.toml` sets `dir = "adrs"` and `start = 0`. `adrlint lint` and `adrlint format --check` exit 0 on `adrs/`,
and the poly hooks `adrlint-lint`, `adrlint-format-check` and `adrlint-lock-check` run them on commit.

## More Information

* [MADR](https://adr.github.io/madr/)
* [adrlint](https://github.com/Goldziher/adrlint)
* [Org decision log](https://github.com/xberg-io/agent-conventions/blob/main/docs/adr-index.md)
