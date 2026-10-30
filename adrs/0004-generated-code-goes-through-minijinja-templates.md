---
status: accepted
date: 2026-05-08
decision-makers: ["Na'aman Hirschfeld"]
---

# Generated code goes through minijinja templates

## Context and Problem Statement

The backends emitted target-language code by building strings in Rust: `push_str(&format!(...))`, interpolating
`write!` and `writeln!`, and multiline `format!` literals, as the first FFI backend in 56033e56c did. `minijinja`
was already a dependency from that first commit, used from 2026-04-13 for README templates (c25ec6353). Through
early May the backends were moved onto per-backend template environments one at a time (425305de1, 81d486fe7,
c6818a9d3).

## Considered Options

* Rust prepares the data, and minijinja templates own the emitted structure
* Emit target code from Rust with `format!`, `write!` and `push_str`

## Decision Outcome

Chosen option: "Rust prepares the data, and minijinja templates own the emitted structure", because that is the
direction the commits and the `codegen-structure` rule record; none of them states a rationale. 86daa2faa (2026-05-08)
migrated "all alef-backend-* code emission to Jinja templates", and the 0.15.0 release (a1983c180) was staged as the
"Jinja backend migration".

The `codegen-structure` rule (first committed in be684a4e1, 2026-06-06) states the split:

* Rust does IR lookup, naming calls, type mapping, metadata structs, argument lists and small expression
  fragments.
* Templates own declarations, statements, signatures, attributes, doc blocks, indentation and multiline blocks.
* Parameterized generated code in `src/backends/<lang>/`, `src/codegen/` and `src/e2e/codegen/` never goes through
  `push_str(&format!(...))`, an interpolating `write!` or `writeln!`, or a multiline `format!`.

### Consequences

* Good, because the shape of emitted code is reviewable as a template next to the language it produces.
* Bad, because the migration is incomplete: 517 `push_str(&format!` call sites remain in non-test files under
  `src/backends`, `src/codegen` and `src/e2e/codegen`, and no automated check enforces the rule.
* Bad, because a template error surfaces at render time, not at compile time.

### Confirmation

* `Cargo.toml:83` depends on `minijinja` 3.0.
* 25 backends and generators carry a `template_env.rs`, and `src/` holds 2,938 `.jinja` templates.
* `.ai-rulez/rules/codegen-structure.md` states the rule, and the `jinja-codegen` skill covers template
  registration.
