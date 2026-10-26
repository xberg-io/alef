---
type: Playbook
title: Jinja Codegen
description: "Mechanics of alef's Minijinja templates: which template_env module to call, registering a template, inline templates, engine settings. Use when adding or changing a generated-code template in a backend, codegen or e2e module."
paths:
  - "src/**/*.jinja"
  - "src/**/template_env.rs"
  - "src/backends/**"
  - "src/codegen/**"
  - "src/e2e/**"
x-ai-rulez:
  kind: skill
  id: jinja-codegen
  metadata:
    license: MIT
    name: jinja-codegen
---

# Jinja Codegen Mechanics

The `codegen-structure` rule says parameterized generated code goes through templates. This covers how.

## Which `template_env`

There is no `crate::template_env`. Each module owns one: `src/backends/<lang>/template_env.rs`, and `src/{codegen,core,docs,readme,scaffold,e2e}/template_env.rs`. Call the one local to the code:

```rust
out.push_str(&crate::backends::<lang>::template_env::render("block_name.jinja", minijinja::context! {
    key => value,
}));
```

## Registering

In that `template_env.rs`: `("block_name.jinja", include_str!("templates/block_name.jinja")),`. One template per logical unit (class header, method signature, enum variant); no generic line/content passthrough templates.

## Engine settings

Each `make_env()` sets `trim_blocks = true`, `lstrip_blocks = true`, `keep_trailing_newline = true`.

## Inline templates

Only for single-line fragments inside a larger template or join. Call `.trim_end()` or `.trim_end_matches('\n')` at the call site; templates keep their trailing newline, so omitting it doubles blank lines.

## Rust vs template

- Rust prepares typed values, identifiers, escaped literals, symbol and type names, paths, field metadata, booleans, joined argument lists, small fragments.
- Templates own structure, indentation, multiline blocks.
- Static `push_str("literal\n")` without interpolation needs no template.
- Anti-patterns: `format!` strings with `\n` for generated code, `format!(r#"..."#)` blocks, catch-all templates such as `formatted_line.jinja` taking `content => format!(...)`.
