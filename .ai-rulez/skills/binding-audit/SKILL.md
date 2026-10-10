---
type: Playbook
title: Binding Audit
description: 'Audit bindings for coverage gaps — verify every public Rust item is exposed across all generated language bindings. Use this skill any time you need to check that a function/type is present in every target language, audit intentional exclusions, or investigate missing bindings in one or more languages. Covers the full audit flow: config review, attribute scan, item enumeration, cross-binding diff, gap reporting, and triage (alef vs Alef-owned workflow/action vs consumer config).'
x-ai-rulez:
  kind: skill
  id: binding-audit
  metadata:
    license: MIT
    name: binding-audit
---

# Binding Audit

Verify that every public Rust item has a corresponding binding in all target languages. Identify coverage gaps and triage them upstream.

## When to apply

- User asks to audit bindings or check coverage
- A function or type is missing from one or more generated bindings
- Preparing to release — confirm all public items are bound
- Investigating a "why isn't X available in language Y?" question
- After adding a new public Rust item — verify it appears everywhere

## Hard rules

1. **No guessing about intentional removals.** The real surfaces: `[crates.exclude]` (`types`/`functions`/`methods`/`fields`) inside a `[[crates]]` entry, crate-wide and unioned across every language; per-language `exclude_types` / `exclude_functions` directly on each `[crates.<lang>]` table; `[workspace.opaque_types]`, workspace-level only, which **remaps** a type rather than excluding it. At the attribute level, the extractor accepts three spellings — `#[alef::skip]`, `#[alef(skip)]`, and either nested in `#[cfg_attr(...)]` (the form in common use) — plus `#[doc(hidden)]`; `#[alef::exclude]` and `#[alef::opaque]` do not exist. Only flag items not covered by these.
2. **Every gap is triaged.** Never report a missing binding without identifying the root cause (alef codegen bug, action script error, or config oversight).
3. **All findings update `CHANGELOG.md`** — each upstream fix gets an `[Unreleased]` entry.
4. **Commit SHAs and workflow URLs** are recorded so consumer repos can pin the exact fix.

## Procedure

### 0. Gather config

From the **source repo** (the Rust library being bound, not alef itself):

```bash
# Open alef.toml and record:
# - [languages] enabled backends
# - [e2e] enabled language suites
# - [crates.exclude] items (types/functions/methods/fields) inside a [[crates]]
#   entry — crate-wide, unioned across every language
# - Per-language exclude_types / exclude_functions directly on each
#   [crates.<lang>] table (e.g. [crates.python].exclude_types,
#   [crates.ffi].exclude_functions) — unioned with the crate-wide list for
#   that language only
# - [workspace.opaque_types] — workspace-level only, no per-crate override.
#   This is a type-REMAPPING declaration (Rust type name -> external path
#   alef can't extract), not an exclusion list.
grep -E '^\[' alef.toml | head -20
```

There is **no** `[crates.skipped]`, no bare `exclude_types` key, and no per-crate override under
`[workspace.crates."<name>"]` — `[[crates]]` is a plain array (`WorkspaceConfig` has no `crates`
field; `RawCrateConfig` has no `skipped` field), so there is no name-keyed map to override into.
`src/docs/language_pages/excludes.rs::language_excludes` is the canonical per-language union of
the config surfaces above.

Record intentional removals. Anything listed here is not a gap.

The current backend surface is Python/PyO3, TypeScript/Node/NAPI, Ruby/Magnus, PHP, Go/cgo, Java, JNI,
C#, Elixir/Rustler, WASM, Dart, Kotlin, Kotlin Android, Swift, Zig, C FFI, R/extendr when enabled, and
Gleam when generated. Do not invent an expected package for a language that is not enabled in `alef.toml`.

### 1. Scan source for attributes

Grep the **source Rust crate** for intentional removal markers. The extractor accepts three
spellings of the skip attribute, plus `#[doc(hidden)]`
(`src/extract/extractor/helpers/attributes.rs::extract_binding_exclusion_reason`, lines 304-333):
`#[alef::skip]`, the list form `#[alef(skip)]`, and either of those nested in `#[cfg_attr(...)]`
(e.g. `#[cfg_attr(alef, alef(skip))]` — **the form in common use**; the extractor's own reason
string for it is literally `"alef(skip)"`, not `"alef::skip"`). A grep for only `#[alef::skip]`
misses the `cfg_attr` form and will manufacture false gaps.
**`#[alef::exclude]` and `#[alef::opaque]` do not exist in alef** — do not grep for or expect them.

```bash
# Find all skip-attribute spellings (bare, list-form, cfg_attr-nested) and #[doc(hidden)]
grep -rEln '#\[(cfg_attr\([^)]*,\s*)?(alef::skip|alef\(skip\))\]?|#\[doc\(hidden\)\]' --include='*.rs' .

# For each file found, inspect the context:
grep -B2 -A2 -E '#\[(cfg_attr\([^)]*,\s*)?(alef::skip|alef\(skip\))\]?|#\[doc\(hidden\)\]' <file.rs>
```

Record the annotated items — these are intentional and do **not** flag as gaps.

Both attributes set the `binding_excluded` flag on the item's IR node at extraction time. That
flag is honored **independently by every downstream consumer** — each backend, `src/core/jni.rs`,
`src/core/validation/readiness.rs`, docs generation, etc. all filter on it separately; there is no
single central enforcement point. Critically, `language_excludes` (step 0) **never consults
`binding_excluded`**; it only reads the config surfaces. So a `#[alef::skip]`'d (or
`#[doc(hidden)]`) item is correctly invisible in every generated binding, but tooling that treats
`language_excludes`'s answer as the *complete* set of intentional removals will misclassify that
skipped item as a real gap, because it never shows up in `language_excludes`'s output at all
(live defect: alef-task #329).

### 2. Enumerate public items

From the **source Rust crate**, list all public items. Adjust the path glob to the source repo's layout (`src/`, `crates/*/src/`, or a workspace path):

```bash
# Functions:
grep -rE "^pub fn " src --include="*.rs" | wc -l

# Types (structs, enums):
grep -rE "^pub struct|^pub enum|^pub trait" src --include="*.rs"

# Methods (on pub types):
grep -rE "impl.*pub fn" src --include="*.rs"
```

Build a reference set: `{module::ItemName}` for each public item, excluding those from step 1.

### 3. Walk each generated binding

For each enabled language under `packages/<lang>/`, `crates/*-<binding>/`, or language-native output dirs:

```bash
# Python (generated stubs):
ls -la packages/python/*.pyi
grep -E "^def |^class " packages/python/*.pyi

# TypeScript / Node (generated .d.ts or package entrypoint):
grep -R -E "export (function|class|type|const) " packages/typescript crates/*-node --include="*.ts" --include="*.d.ts"

# Ruby:
grep -R -E "^  def |^    def " packages/ruby crates/*-rb --include="*.rb"

# PHP:
grep -R -E "function |class " packages/php --include="*.php"

# Go (FFI):
grep -R -E "^func " packages/go --include="*.go"

# Java / JNI:
grep -R -E "^\s+(public static|public) (native )?" packages/java packages/jni --include="*.java"

# C#:
grep -R -E "^\s+public (static|extern|class|struct)" packages/csharp --include="*.cs"

# Elixir:
grep -R -E "def |defmodule " packages/elixir --include="*.ex"

# WASM:
grep -R -E "export (function|class|type|const) " packages/wasm --include="*.ts" --include="*.d.ts"

# Dart:
grep -R -E "class |^[a-zA-Z_][a-zA-Z0-9_]*\\(" packages/dart --include="*.dart"

# Kotlin / Kotlin Android:
grep -R -E "fun |class " packages/kotlin packages/kotlin-android --include="*.kt"

# Swift:
grep -R -E "public (func|class|struct|enum)" packages/swift --include="*.swift"

# Zig:
grep -R -E "pub (fn|const|const.*= struct|const.*= enum)" packages/zig --include="*.zig"

# C FFI headers:
grep -R -E "^[a-zA-Z_][a-zA-Z0-9_ *]+ [a-zA-Z_][a-zA-Z0-9_]+\\(" packages/c crates/*-ffi --include="*.h"

# R / extendr:
grep -R -E "^[a-zA-Z.][a-zA-Z0-9_.]* <- function|#' @export" packages/r --include="*.R"

# Gleam:
grep -R -E "^pub (fn|type)" packages/gleam --include="*.gleam"
```

For each language, build a set of exported items.

### 4. Diff and report gaps

For each public Rust item, check presence across all binding sets:

```bash
# Pseudo-algorithm:
all_langs = [
    "python", "typescript", "ruby", "php", "go", "java", "jni", "csharp", "elixir", "wasm",
    "dart", "kotlin", "kotlin_android", "swift", "zig", "c_ffi", "r", "gleam",
]
enabled_langs = [lang for lang in all_langs if lang is enabled in alef.toml and output exists]

for each item in reference_set:
    langs_present = [lang for lang in enabled_langs if item in binding_sets[lang]]
    if len(langs_present) < len(enabled_langs):
        report(item, langs_present, missing_from=enabled_langs - langs_present)
```

**Output:** gap report with columns:

- `Rust item` (function/type name)
- `Present in` (comma-separated languages)
- `Missing from` (comma-separated languages)
- `Intentional?` (yes if config or attribute covers it, no otherwise)

### 5. Triage

For each non-intentional gap:

- **Codegen issue:** Does the item appear in the source Rust but fail to generate in all backends? Root cause likely in `src/codegen/` or a specific `src/backends/<lang>/`. Fix in `../alef` repo.
- **Alef-owned workflow/action issue:** Does an Alef-maintained scaffold or publish workflow have a bug that skips a language? Fix the owning workflow/action repository and retag only the documented action tags for that repository.
- **Consumer config issue:** Is the gap listed in the **consuming repo's** `alef.toml` under `[crates.exclude]` or a per-language `exclude_types` / `exclude_functions` on `[crates.<lang>]`? That's intentional — no action needed upstream.
- **Package layout issue:** Does generated code exist but not in the expected package path? Fix the backend output path or package manifest wiring, not the Rust source item.
- **Unsupported type issue:** Does the Rust item use a type the backend cannot express? Add explicit conversion, an opaque wrapper, or an intentional exclusion in config.

### 6. Document and commit

For each upstream fix:

1. Update the **source repo's** `CHANGELOG.md` `[Unreleased]` section with the gap and the fix.
2. Commit the fix (codegen or action change) with a conventional commit message.
3. If the fix is in an Alef-owned workflow/action repository, follow that repository's documented retag procedure.
4. For alef fixes, follow the normal `release-procedure` skill.
5. Record the commit SHA and workflow URL in consumer issues so they can pin the fix.

## Anti-patterns

- Reporting a gap without checking `alef.toml` and all three skip-attribute spellings (including the `cfg_attr`-nested form) / `#[doc(hidden)]` first.
- Assuming a missing binding is a codegen bug without checking the consuming repo's config.
- Closing an audit issue without confirming every gap is triaged and documented.
- Fixing a codegen bug without adding a test under `tests/` or a fixture under `src/e2e/` to prevent regression.

## Quick reference

| Step | Command | Output |
|------|---------|--------|
| Config | `grep -E '^\[' alef.toml` | Intentional exclusions (`[crates.exclude]`, per-language `exclude_types`/`exclude_functions`, `[workspace.opaque_types]`) |
| Attributes | `grep -rEln '#\[(cfg_attr\([^)]*,\s*)?(alef::skip\|alef\(skip\))\]?\|#\[doc\(hidden\)\]' --include='*.rs' .` | Annotated items |
| Public items | `grep -rE "^pub fn\|^pub struct" src` | Reference set |
| Bindings | `grep -R -E "export\|def\|func\|public\|fun " packages crates` | Per-language sets |
| Gaps | Diff reference set vs per-language sets | Gap report |
| Triage | Root-cause analysis (config vs codegen vs action) | Fix location |
