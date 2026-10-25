---
type: Playbook
title: Binding Audit
description: "Audit a consumer repo's generated bindings for coverage gaps: every public Rust item present in every enabled language, exclusions checked, gaps triaged. Use when asked to audit bindings or why item X is missing in language Y."
disable-model-invocation: true
x-ai-rulez:
  kind: skill
  id: binding-audit
  metadata:
    license: MIT
    name: binding-audit
---

# Binding Audit

Run from the source repo (the Rust library being bound), not alef itself. Every gap is triaged to a root cause; every upstream fix gets a `CHANGELOG.md` `[Unreleased]` entry; record commit SHAs and workflow URLs so consumers can pin the fix.

## 1. Intentional removals (not gaps)

Config surfaces in `alef.toml`:

- `[crates.exclude]` (`types`/`functions`/`methods`/`fields`) inside a `[[crates]]` entry — crate-wide, unioned across every language.
- Per-language `exclude_types` / `exclude_functions` on each `[crates.<lang>]` table — unioned with the crate-wide list for that language.
- `[workspace.opaque_types]` — workspace-level only; it remaps a type to an external path, it does not exclude it.
- There is no `[crates.skipped]`, no bare `exclude_types` key, and no `[workspace.crates."<name>"]` override; `[[crates]]` is a plain array.
- `src/docs/language_pages/excludes.rs::language_excludes` is the canonical per-language union of these surfaces.

Attributes (`src/extract/extractor/helpers/attributes.rs::extract_binding_exclusion_reason`): `#[alef::skip]`, `#[alef(skip)]`, either nested in `#[cfg_attr(...)]` (the common form, e.g. `#[cfg_attr(alef, alef(skip))]`), and `#[doc(hidden)]`. `#[alef::exclude]` and `#[alef::opaque]` do not exist.

```bash
grep -rEn '#\[(cfg_attr\([^)]*,\s*)?(alef::skip|alef\(skip\))\]?|#\[doc\(hidden\)\]' --include='*.rs' .
```

- These set `binding_excluded` on the IR node; each backend, `src/core/jni.rs`, `src/core/validation/readiness.rs` and docs generation filter on it separately.
- `language_excludes` never reads `binding_excluded`, so tooling that treats it as the full exclusion set misreports skipped items as gaps (alef-task #329).

## 2. Reference set

List public items in the source crate (adjust `src` to the layout, e.g. `crates/*/src/`): `grep -rE "^pub (fn|struct|enum|trait) " src --include="*.rs"`. Drop the step 1 items.

## 3. Per-language export sets

Only for languages enabled in `alef.toml` with existing output (Python/PyO3, Node/NAPI, Ruby/Magnus, PHP, Go, Java, JNI, C#, Elixir/Rustler, WASM, Dart, Kotlin, Kotlin Android, Swift, Zig, C FFI, R/extendr, Gleam). Grep the declarations under `packages/<lang>/`, `crates/*-<binding>/` or the language output dir, e.g. `.pyi` `def`/`class`, `.d.ts` `export`, Go `^func `, C# `public`, Kotlin `fun`/`class`, Swift `public func`, Zig `pub fn`, C headers, R `<- function`, Gleam `pub fn`.

## 4. Gap report

Per item: Rust item, present in, missing from, intentional (yes if step 1 covers it).

## 5. Triage

- Codegen: item fails to generate → `src/codegen/` or `src/backends/<lang>/`; fix in alef with a test under `tests/` or a fixture under `src/e2e/`.
- Alef-owned workflow/action: a scaffold or publish workflow skips a language → fix that repository, retag only its documented action tags.
- Consumer config: an exclusion in the consumer's `alef.toml` → intentional, or fix that `alef.toml`.
- Package layout: code generated in the wrong path → fix the backend output path or manifest wiring.
- Unsupported type: add a conversion, an opaque wrapper, or an explicit config exclusion.

For alef fixes, release via the `release-procedure` skill.
