---
type: Decision
title: Project Agnostic Codegen
paths:
  - "src/**"
  - "tests/**"
x-ai-rulez:
  kind: rule
  id: project-agnostic-codegen
  metadata:
    priority: high
---

- No hard-coded consumer identities (repo, package, module names, paths, product branches) in generator, extraction, scaffold, e2e, publish or CLI code.
- Model consumer differences through generic configuration: `alef.toml`, IR metadata, backend language config, feature flags, capability flags. Name the generic condition, not the consumer.
- Tests, fixtures, snapshots, docs and expected output use neutral sample names; rename consumer domain types (product-specific config, result, visitor, handle, backend names) before accepting snapshots.
