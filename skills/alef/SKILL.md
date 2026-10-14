---
name: alef
description: >-
  Use Alef correctly for Rust-to-polyglot binding generation. Trigger when
  configuring alef.toml, generating bindings, READMEs, API/CLI/MCP docs,
  llms.txt, agent skills, e2e suites, or debugging stale/missing generated
  output in Alef-powered repositories. Covers the safe command sequence,
  config ownership, generated-output rules, snippet validation, downstream
  smoke testing, and Alef development workflow.
license: MIT
metadata:
  author: xberg-io
  version: "1.0"
  repository: https://github.com/xberg-io/alef
---

# Alef

Alef extracts a Rust public API surface and generates language-native bindings,
package scaffolding, type stubs, READMEs, docs, e2e tests, and release metadata
from `alef.toml`.

Use this skill when working in Alef itself or in a downstream repo that uses
Alef.

## Core Rules

- Treat Rust source plus `alef.toml` as the source of truth.
- Read the local `alef.toml` before proposing config or generation changes.
- Do not hand-edit Alef-managed generated files except when adopting an existing
  file into Alef management.
- Preserve user changes in dirty worktrees. Stage and commit only the requested
  scope.
- Prefer narrow commands while iterating, then run the broader verification
  command before committing.
- For user-visible Alef behavior changes, update `CHANGELOG.md`.

## Standard Consumer Workflow

From a repo that uses Alef, read its Taskfile first and use its `alef:*`, README and docs
commands when available. For direct CLI use, this sequence refreshes the configured surfaces:

```bash
alef generate
alef stubs
alef scaffold
alef readme
alef docs
alef verify --exit-code
```

Use the combined command when a full refresh is expected. `--clean` bypasses the cache;
it does not delete files or authorize overwriting unmanaged seeds. Generation applies
configured formatters automatically; `--strict` fails if a configured formatter is missing:

```bash
alef all --clean
```

Use filters to keep iteration small:

```bash
alef generate --lang python,node
alef docs --output docs/reference
alef test --lang python
alef verify --exit-code
```

Use `--skip-compile` for source-only generation when the repository builds Swift bridge
artifacts and C headers separately. Use `--skip-snippet-validation` for docs generation before
language artifacts exist; run snippet validation after building them. Both flags report the
work they defer. `alef generate` does not refresh `test_apps/`; use `alef test-apps generate`
or the corresponding repository task.

When testing an unreleased local Alef from a sibling repo, use its configured local-build task
or run the binary through Cargo instead of using the installed version:

```bash
cargo run -q --manifest-path ../alef/Cargo.toml -- docs
cargo run -q --manifest-path ../alef/Cargo.toml -- all --clean
```

## Config Model

Current Alef configs use:

- `[workspace]` for shared languages, tools, DTO defaults, docs defaults, and
  pipeline defaults.
- `[[crates]]` for each generated package/API surface.
- `[crates.<language>]` or `[workspace.<language>]` for target-specific output,
  package names, feature flags, excludes, and stubs.
- `source_crates` when a facade crate re-exports API from multiple Rust crates.
- `features` when cfg-gated public fields/types must be considered present.

Use `include` for small curated APIs. Use `exclude` for large APIs where most
public items should bind except known internal, generic, trait, or unsupported
items.

## Generated Docs, llms.txt, and Skills

Alef can generate docs in this order:

1. API reference docs from extracted Rust API.
2. CLI reference docs from configured Clap sources.
3. MCP reference docs from configured rmcp-style sources.
4. Snippet index and configured snippet validation.
5. Template-rendered `llms.txt`.
6. Template-rendered grouped skills.

Important rules:

- `llms.txt` and skills are template-owned. Missing templates are hard errors.
- Alef should not invent full prose for `llms.txt` or skills.
- Existing unmanaged outputs require explicit `adopt_existing = true`.
- Generated Markdown keeps frontmatter first, then Alef's managed hash marker.
- Warn only for actionable skips: missing configured sources, configured
  extractors discovering nothing, missing configured snippet dirs, or unavailable
  snippet toolchains.

Common docs config shape:

```toml
[workspace.docs]
reference_output = "docs/reference"

[workspace.docs.cli]
sources = ["crates/my-cli/src/main.rs"]

[workspace.docs.mcp]
sources = ["crates/my-lib/src/mcp/server.rs"]

[workspace.docs.llms]
template = "templates/docs/llms.txt.jinja"
output = "docs/llms.txt"
adopt_existing = true

[workspace.docs.skills]
template_dir = "templates/docs/skills"
outputs = [".codex/skills", ".agents/skills", ".claude/skills", ".github/skills"]
adopt_existing = true

[workspace.docs.snippets]
dirs = ["docs/snippets"]
docs_dirs = ["docs"]
required_languages = ["python", "rust"]
validation_level = "syntax"
```

Skill templates default to grouped `api`, `cli`, and `mcp` skills:

```text
templates/docs/skills/
├── api/SKILL.md.jinja
├── cli/SKILL.md.jinja
└── mcp/SKILL.md.jinja
```

## Snippets

Use snippets as maintained examples, not generated filler. Configure validation
instead of silently trusting examples:

- `dirs`: snippet roots.
- `docs_dirs`: docs/template roots to scan for includes.
- `required_languages`: language variants every grouped snippet should have.
- `validation_level`: `syntax`, `typecheck`, `compile`, or `run`.
- `include_base_paths`: paths matching MkDocs snippet include roots.

Unreferenced snippets should normally warn, not fail. Missing references,
missing required language variants, unknown languages, and skip annotations
without reasons should fail.

## Debugging

Missing type or function:

```bash
alef extract -o /tmp/api.json
jq 'keys' /tmp/api.json
```

Then check:

- Is the source file listed in `sources` or `source_crates`?
- Is the item public and reachable from the configured source?
- Is it excluded in config or with an Alef attribute?
- Does it depend on an unsupported generic, trait object, or external type?
- Does the target language need an FFI layer (`ffi`) or explicit type mapping?

Stale output:

```bash
alef verify --exit-code
alef diff
alef generate --clean
```

Cache issues:

```bash
alef cache status
alef generate --clean
```

Docs generation issues:

- Confirm template paths are relative to the workspace root.
- Confirm configured CLI/MCP source paths exist.
- Confirm generated outputs have Alef headers or `adopt_existing = true`.
- Use `-v`/`RUST_LOG` when a warning is expected but not visible.

## Working on Alef Itself

In the Alef repo, prefer focused checks while iterating:

```bash
task format:check
task lint
task test:run -- docs:: --nocapture
```

Use `task test:run -- <test-filter>` for a specific regression. Before committing behavior changes, run the highest-signal relevant tests. For
docs/template work, also smoke test downstream repos with the local binary:

```bash
cargo run -q --manifest-path ../alef/Cargo.toml -- docs
git diff --check
```

Use the sibling repos that exercise Alef broadly:

- `../crawlberg`
- `../html-to-markdown`
- `../liter-llm`
- `../tree-sitter-language-pack`

Choose downstream consumers relevant to the changed surface; Xberg is also a consumer when its configuration exercises the change.

## Release Notes and Commits

- Update `CHANGELOG.md` under `[Unreleased]` for user-visible changes.
- Keep release commits separate from feature/fix commits.
- Do not add AI attribution to commits, tags, or release notes.
- Use the existing release procedure skill when cutting a version.
