---
type: Playbook
title: Two Generators Disagree
description: "Debug alef's dominant defect shape: two components read the same config or IR and act differently. Use when a generated package fails to build, a generated suite fails wholesale, or one backend's output is wrong and its siblings' is not."
paths:
  - "src/backends/**"
  - "src/codegen/**"
  - "src/e2e/**"
  - "src/snippets/**"
  - "src/scaffold/**"
x-ai-rulez:
  kind: skill
  id: two-generators-disagree
  metadata:
    license: MIT
    name: two-generators-disagree
---

# Two Generators Disagree

Alef bugs cluster where two components read the same input and act on it differently, most often the snippet generator and the e2e generator reading the same IR.

## Rules

1. Ask which two components disagree. Compare against a sibling backend generated from the same config; the correct implementation usually exists in the tree.
2. Fix the shared source of truth, not the emitter. Existing seams: `result_relative_path`, `swift_json_bridged_traversal_prefix`, `effective_ffi_default_features`, `gradle_build_task`, `expand_configured_features`. Reuse one before adding a copy.
3. An asymmetry (a config key honored by one generator and ignored by another, two hand-maintained oracles for one fact, two spellings of a file or symbol where one is read) is a defect until proven intentional.
4. A clean generated suite can mean alef generated a weakened checker config (e.g. a loosened tsconfig); check it before trusting green.
5. String-level tests pass on output that cannot compile; target code needs a compile gate.

## Procedure

1. List every component touching the failing fact: backend, config field, generator (binding codegen, e2e, snippets, scaffold).
2. Diff their behavior for the same input; the outlier is not automatically the bug.
3. Trace each to where it reads the fact; duplicated constants, parallel config lists or second accessors are the likely root cause.
4. Fix at the shared source; grep every sibling backend for the same pattern.
5. Add a compile-level regression test when the target language must build.
