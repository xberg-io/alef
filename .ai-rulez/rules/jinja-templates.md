---
priority: critical
---

All parameterized code emission in `alef-backend-*` crates must use `crate::template_env::render()`. Never use `push_str(&format!(…))` or `writeln!(out, …)` for interpolated output.

Pattern:

```rust
out.push_str(&crate::template_env::render("block_name.jinja", minijinja::context! {
    key => value,
}));
```

Template settings (set in `make_env()`): `trim_blocks = true`, `lstrip_blocks = true`, `keep_trailing_newline = true`.

Register templates in `src/template_env.rs` via `include_str!`:

```rust
("block_name.jinja", include_str!("../templates/block_name.jinja")),
```

One template per logical unit (class header, method signature, enum variant, etc.). Static `push_str("literal\n")` with no interpolation is fine to leave as-is — no template needed for non-parameterized strings.
