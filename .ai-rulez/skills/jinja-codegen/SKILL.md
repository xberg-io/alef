---
type: Playbook
title: Jinja Codegen
description: 'Mechanics of alef''s Minijinja template system: which template_env module to call, how to register a template, inline-template rules, and engine settings. Use this skill when adding or changing generated-code templates in any backend, codegen, or e2e generator module.'
x-ai-rulez:
  kind: skill
  id: jinja-codegen
  metadata:
    license: MIT
    name: jinja-codegen
---

# Jinja Codegen Mechanics

The `jinja-templates` rule states the invariant: all parameterized generated code goes through
Minijinja templates, never raw `format!`/`push_str`/`write!` assembly. This skill covers the
mechanics of doing that correctly.

## Which `template_env` to call

There is no `crate::template_env` module. Each backend and generator owns its own
`template_env.rs`:

- `src/backends/<lang>/template_env.rs` — per-language backend templates
- `src/codegen/template_env.rs`, `src/core/template_env.rs`, `src/docs/template_env.rs`,
  `src/readme/template_env.rs`, `src/scaffold/template_env.rs`, `src/e2e/template_env.rs` —
  shared/generator-specific templates

Call the one local to the code you are generating from:

```rust
out.push_str(&crate::backends::<lang>::template_env::render("block_name.jinja", minijinja::context! {
    key => value,
}));
```

## Registering a template

Register in the backend's `template_env.rs` via `include_str!`:

```rust
("block_name.jinja", include_str!("templates/block_name.jinja")),
```

One template per logical unit (class header, method signature, enum variant, etc.) — do not
create generic line/content passthrough templates to bypass the `jinja-templates` rule.

## Engine settings

Set in each `make_env()`: `trim_blocks = true`, `lstrip_blocks = true`,
`keep_trailing_newline = true`.

## Inline templates

Allowed only for single-line fragments used inside a larger template or join operation. Inline
renders must call `.trim_end()` or `.trim_end_matches('\n')` at the call site, because templates
keep their trailing newline — omitting the trim doubles blank lines in the emitted output.

## What Rust prepares vs. what the template owns

Rust prepares typed values, identifiers, escaped literals, symbol names, type names, file paths,
enum/field metadata, booleans, comma-joined argument lists, and small expression fragments when
they are not a logical emitted unit. Templates own generated-code structure, indentation, and
multiline blocks.

Static `push_str("literal\n")` with no interpolation is fine to leave as-is — no template needed
for non-parameterized strings.

## Anti-patterns

- `push_str(&format!(...))`, `write!`/`writeln!` for interpolated output, or multiline
  `format!(...)`/`format!(r#"..."#)` that emits target code.
- `format!(...)` strings containing `\n` for generated code.
- Catch-all passthrough templates (e.g. `formatted_line.jinja` receiving `content =>
  format!(...)`) used to route around the templating requirement.
- Calling `crate::template_env::render` — that module does not exist; use the local one.
