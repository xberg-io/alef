---
name: binding-audit
description: >-
  Audit bindings for coverage gaps — verify every public Rust item is exposed
  across all generated language bindings. Use this skill any time you need to
  check that a function/type is present in every target language, audit
  intentional exclusions, or investigate missing bindings in one or more
  languages. Covers the full audit flow: config review, attribute scan, item
  enumeration, cross-binding diff, gap reporting, and triage (alef vs actions
  vs consumer config).
license: MIT
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

1. **No guessing about intentional removals.** Config (`alef.toml`) and attributes (`#[alef::skip]`, `#[alef::exclude]`, `#[alef::opaque]`) are canonical. Only flag items not covered by them.
2. **Every gap is triaged.** Never report a missing binding without identifying the root cause (alef codegen bug, action script error, or config oversight).
3. **All findings update `CHANGELOG.md`** — each upstream fix gets an `[Unreleased]` entry.
4. **Commit SHAs and workflow URLs** are recorded so downstream consumers can pin the exact fix.

## Procedure

### 0. Gather config

From the **source repo** (the Rust library being bound, not alef itself):

```bash
# Open alef.toml and record:
# - [languages] enabled backends
# - [e2e] enabled language suites
# - [crates.exclude] items (global exclusions)
# - [crates.<lang>.exclude] items (per-language exclusions)
# - [workspace.exclude_types]
# - [crates.opaque_types], [workspace.opaque_types]
# - Any per-crate overrides under [workspace.crates."<name>"]
grep -E '^\[' alef.toml | head -20
```

Record intentional removals. Anything listed here is not a gap.

The current backend surface is Python/PyO3, TypeScript/Node/NAPI, Ruby/Magnus, PHP, Go/cgo, Java, JNI,
C#, Elixir/Rustler, WASM, Dart, Kotlin, Kotlin Android, Swift, Zig, C FFI, R/extendr when enabled, and
Gleam when generated. Do not invent an expected package for a language that is not enabled in `alef.toml`.

### 1. Scan source for attributes

Grep the **source Rust crate** for intentional skip/exclude/opaque markers:

```bash
# Find all #[alef::skip], #[alef::exclude], #[alef::opaque]
find . -name "*.rs" -type f -exec grep -l "#\[alef::" {} \;

# For each file found, inspect the context:
grep -B2 -A2 "#\[alef::" <file.rs>
```

Record the annotated items — these are intentional and do **not** flag as gaps.

### 2. Enumerate public items

From the **source Rust crate**, list all public items:

```bash
# Functions:
grep -r "^pub fn " crates/*/src --include="*.rs" | wc -l

# Types (structs, enums):
grep -r "^pub struct\|^pub enum\|^pub trait" crates/*/src --include="*.rs"

# Methods (on pub types):
grep -r "impl.*pub fn" crates/*/src --include="*.rs"
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

- **Codegen issue:** Does the item appear in the source Rust but fail to generate in all backends? Root cause likely in `alef-codegen` or a specific `alef-backend-*`. Fix in `../alef` repo.
- **Action script issue:** Does the scaffold or publish workflow have a bug that skips a language? Root cause in `../actions`. Fix there, then retag `v1.0.0` and `v1`.
- **Consumer config issue:** Is the gap listed in the **consuming repo's** `alef.toml` under `[crates.exclude]` or `[crates.<lang>.exclude]`? That's intentional — no action needed upstream.
- **Package layout issue:** Does generated code exist but not in the expected package path? Fix the backend output path or package manifest wiring, not the Rust source item.
- **Unsupported type issue:** Does the Rust item use a type the backend cannot express? Add explicit conversion, an opaque wrapper, or an intentional exclusion in config.

### 6. Document and commit

For each upstream fix:

1. Update the **source repo's** `CHANGELOG.md` `[Unreleased]` section with the gap and the fix.
2. Commit the fix (codegen or action change) with a conventional commit message.
3. If the fix is in `../actions`, follow the retag procedure: `git tag -f v1.0.0 && git tag -f v1 && git push -f origin v1.0.0 v1`.
4. For alef fixes, follow the normal `release-procedure` skill.
5. Record the commit SHA and workflow URL in downstream consumer issues so they can pin the fix.

## Anti-patterns

- Reporting a gap without checking `alef.toml` and `#[alef::*]` attributes first.
- Assuming a missing binding is a codegen bug without checking the consuming repo's config.
- Closing an audit issue without confirming every gap is triaged and documented.
- Fixing a codegen bug without adding a test to `alef-e2e/` to prevent regression.

## Quick reference

| Step | Command | Output |
|------|---------|--------|
| Config | `grep -E '^\[' alef.toml` | Intentional exclusions |
| Attributes | `find . -name "*.rs" -exec grep -l "#\[alef::" {} \;` | Annotated items |
| Public items | `grep -r "^pub fn\|^pub struct" crates/*/src` | Reference set |
| Bindings | `grep -R -E "export\|def\|func\|public\|fun " packages crates` | Per-language sets |
| Gaps | Diff reference set vs per-language sets | Gap report |
| Triage | Root-cause analysis (config vs codegen vs action) | Fix location |
