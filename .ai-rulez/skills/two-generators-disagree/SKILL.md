---
type: Playbook
title: Two Generators Disagree
description: 'Alef''s dominant defect shape: two components read the same config or IR and act on it differently. Use this skill when a generated package fails to build, a generated suite fails wholesale, or a lowering looks wrong in one backend but not others.'
x-ai-rulez:
  kind: skill
  id: two-generators-disagree
  metadata:
    license: MIT
    name: two-generators-disagree
---

# Two Generators Disagree

The most productive place to look for an alef bug is where two components read the same input and
act on it differently. This is not a tendency — audits across multiple consumer repos have
converged on it unprompted, and it recurs most often between the snippet generator and the e2e
generator reading the same IR.

## When to apply

- A generated package fails to build
- A generated test suite (snippets, e2e) fails wholesale rather than on isolated cases
- A lowering or assertion looks wrong in one backend but correct in its siblings
- Coverage counts differ between two surfaces that should agree (e.g. snippets vs. e2e for the
  same call)

## Hard rules

1. **Ask which two components disagree, don't assume one is simply wrong.** Compare the failing
   output against a sibling backend generated from the same config as a control — the correct
   implementation usually already exists somewhere in the tree.
2. **Fix the shared source of truth, not the emitter.** An IR-derived fix or a single named
   constant fixes every consumer of that fact at once; a per-emitter patch fixes only the one you
   noticed. Alef already has named seams for this pattern —
   `result_relative_path`, `swift_json_bridged_traversal_prefix`,
   `effective_ffi_default_features`, `gradle_build_task`, `expand_configured_features` — check
   whether the fact you're about to duplicate already has one before adding a new copy.
3. **Any asymmetry where one surface keeps a field/behavior and the other drops it (or vice versa)
   is a defect until proven intentional.** This includes: a config key honored by one generator and
   silently ignored by another; two hand-maintained "oracles" for what should be one derived fact
   (e.g. a config list and an IR field both claiming to answer the same question); two spellings of
   one filename or symbol where only one is actually read by the consuming tool.
4. **A generated suite that looks clean can mean the checker was weakened, not that the output is
   correct.** Alef sometimes generates the config that hides its own bugs (a loosened tsconfig,
   for example). When output looks suspiciously clean, check whether the generated *checker
   config* conceals real errors before trusting the green result.
5. **String-level tests pass on output that cannot compile.** Anything the target language must
   compile needs a real compile gate, not just an assertion on the emitted string; a shape that
   only a build catches will otherwise ship silently.

## Procedure

1. When a generated package or suite fails, identify every alef component that touches the failing
   fact (which backend, which config field, which generator: binding codegen, e2e, snippets,
   scaffold).
2. Diff behavior across those components for the same config/IR input. Note which ones agree and
   which one is the outlier — the outlier is not automatically the bug; check both directions.
3. Trace each disagreeing component back to where it reads the fact. If two components each read
   their own copy of the same fact (a duplicated constant, a parallel config list, a second
   hand-written accessor), that duplication is very likely the root cause.
4. Fix at the shared source of truth per Hard rule 2. Grep every sibling backend for the same
   pattern before calling the fix done — a fix that lands in one backend and not its siblings is
   how "one backend still has yesterday's bug" survives a whole release cycle.
5. Add or strengthen a compile-level (not just string-level) regression test per Hard rule 5 when
   the fix touches a target language that must build.

## Anti-patterns

- Patching the one backend you noticed without checking every sibling backend for the identical
  match-arm or accessor pattern.
- Treating a generated suite's green result as proof of correctness without checking whether the
  generated checker config was loosened.
- Adding a fourth copy of an answer alef already computes somewhere, instead of routing every
  caller through the existing seam.
- Assuming a coverage-count mismatch between two surfaces is intentional without confirming it in
  config or the IR.
