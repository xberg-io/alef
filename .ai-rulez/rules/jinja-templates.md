---
priority: critical
---

All parameterized generated code in `src/backends/<lang>/`, `src/codegen/`, and
`src/e2e/codegen/` must be emitted through Minijinja templates.

Use `crate::backends::<lang>::template_env::render()`, module-local
`template_env::render()`, or the relevant generator template environment.

Use templates for every parameterized emitted declaration, statement, signature, attribute,
doc block, package/config block, or target-language code block.

Never add new raw generated-code assembly with:

- `push_str(&format!(...))`
- `write!` / `writeln!` for interpolated output
- multiline `format!(...)` or `format!(r#"..."#)` that emits target code
- `format!(...)` strings containing `\n` for generated code
- catch-all passthrough templates such as `formatted_line.jinja` receiving
  `content => format!(...)`

Rust prepares typed values, identifiers, escaped literals, symbol names, type names, file paths,
enum/field metadata, booleans, comma-joined argument lists, and small expression fragments when
they are not a logical emitted unit. Templates own generated-code structure, indentation, and
multiline blocks.

Pattern:

```rust
out.push_str(&crate::template_env::render("block_name.jinja", minijinja::context! {
    key => value,
}));
```

Template settings (set in `make_env()`): `trim_blocks = true`, `lstrip_blocks = true`, `keep_trailing_newline = true`.

Register templates in the backend's `template_env.rs` via `include_str!`:

```rust
("block_name.jinja", include_str!("templates/block_name.jinja")),
```

Inline templates are allowed only for single-line fragments used inside a larger template or join
operation. Inline renders must call `.trim_end()` or `.trim_end_matches('\n')` at the call site
because templates keep their trailing newline.

One template per logical unit (class header, method signature, enum variant, etc.). Static
`push_str("literal\n")` with no interpolation is fine to leave as-is — no template needed for
non-parameterized strings. Do not create generic line/content passthrough templates to bypass
this rule.
