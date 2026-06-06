---
priority: high
---

Keep code generation structured by responsibility.

- Rust prepares data: IR lookup, naming calls, type mapping, metadata structs, argument lists,
  and small expression fragments.
- Jinja emits structure: declarations, statements, signatures, attributes, doc blocks,
  package/config blocks, and target-language code blocks.
- Extract repeated codegen only after the third repetition. Prefer backend-local helpers for
  shared data preparation and templates for shared emitted structure.
- Shared helpers must have one reason to change. If two backends or call sites are likely to
  evolve differently, keep them separate.
- Naming and wire values must come from `src/codegen/naming.rs` or backend helpers that delegate
  to it. Do not add backend-local generic casing or serde rename logic.
