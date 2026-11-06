---
type: Decision
title: Codegen Structure
paths:
  - "src/backends/**"
  - "src/codegen/**"
  - "src/e2e/**"
  - "src/**/*.jinja"
x-ai-rulez:
  kind: rule
  id: codegen-structure
  metadata:
    priority: high
---

- Parameterized generated code in `src/backends/<lang>/`, `src/codegen/` and `src/e2e/codegen/` goes through Minijinja templates, never `push_str(&format!(...))`, interpolating `write!`/`writeln!`, or a multiline `format!(...)` that emits target code.
- Rust prepares data: IR lookup, naming calls, type mapping, metadata structs, argument lists, small expression fragments.
- Templates own emitted structure: declarations, statements, signatures, attributes, doc blocks, indentation, multiline blocks.
- Shared codegen: backend-local helpers for shared data preparation, templates for shared emitted structure.
- Template mechanics (which `template_env`, registration, engine settings): `jinja-codegen` skill.
- Test the generator on every field, type and scope shape that its input can produce, not only the shape of one downstream project.
- Keep each generated declaration equal to the runtime object it describes: the same fields, types, optionality and defaults.
- Make the output depend only on pinned inputs, so two runs on the same inputs give the same bytes.
