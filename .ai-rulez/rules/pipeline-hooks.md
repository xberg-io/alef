---
type: Decision
title: Pipeline Hooks
paths:
  - "src/core/config/**"
x-ai-rulez:
  kind: rule
  id: pipeline-hooks
  metadata:
    priority: high
---

- Since 0.82.0, `lint`/`setup`/`update`/`clean`/`build_commands` are not configurable in `alef.toml`; their configs are internal data carriers returned by `core::config::{lint,setup,update,build,clean}_defaults`, with no `Deserialize`/schema surface.
- `TestConfig` is the exception: `[test.<lang>]` stays configurable in `[workspace]`/`[[crates]]` because `test.e2e` has no code default except for Dart.
- Each command config has `precondition: Option<String>` (must exit 0, else skip with warning) and `before: Option<StringOrVec>` (abort on failure). Order: precondition → before → main command(s).
- Rust builds always use `build_defaults::default_build_config(Language::Rust, ..)`; the backend registry panics for `Language::Rust`, and there is no `[build_commands.rust]`.
