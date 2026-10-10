---
type: Decision
title: Jinja Templates
x-ai-rulez:
  kind: rule
  id: jinja-templates
  metadata:
    priority: critical
---

All parameterized generated code in `src/backends/<lang>/`, `src/codegen/`, and
`src/e2e/codegen/` must be emitted through Minijinja templates — never raw
`push_str(&format!(...))`, `write!`/`writeln!` for interpolated output, or a multiline
`format!(...)` that emits target code. Rust prepares typed values and small expression fragments;
templates own generated-code structure, indentation, and multiline blocks. See the `jinja-codegen`
skill for which `template_env` module to call, template registration, and engine settings.
