---
name: alef
description: "Alef Rust-to-polyglot binding generation: alef.toml, bindings, READMEs, API/CLI/MCP docs and e2e suites, safe command order, generated-output rules. Use when configuring Alef, running mise run alef:* or e2e:*, bumping versions, or debugging stale output."
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

Use this skill in a repo that uses Alef.

## Core Rules

- Treat Rust source plus `alef.toml` as the source of truth.
- Read the local `alef.toml` before proposing config or generation changes.
- Do not hand-edit Alef-managed generated files except when adopting an existing
  file into Alef management.

## Standard Consumer Workflow

From a repo that uses Alef, read its mise tasks first and use its `alef:*`, README and docs
commands when available. For direct CLI use, this sequence refreshes the configured surfaces:

```bash
alef generate
alef stubs
alef scaffold
alef readme
alef docs
alef verify
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
alef verify
```

Use `--skip-compile` for source-only generation when the repository builds Swift bridge
artifacts and C headers separately. Use `--skip-snippet-validation` for docs generation before
language artifacts exist; run snippet validation after building them. Both flags report the
work they defer. `alef generate` does not refresh `test_apps/`; use `alef test-apps generate`
or the corresponding repository task.

When testing an unreleased local Alef, use the repo's local-build task or run the alef checkout
in the polyrepo root through Cargo (`../alef` from a root checkout, `../../../alef` from
`.worktrees/<repo>/<name>`):

```bash
cargo run -q --manifest-path <polyrepo-root>/alef/Cargo.toml -- all --clean
```

## Consumer Tasks

Consumer repos wrap these commands in their own tasks, whose names and flags differ by repo and
change over time. List them with `mise tasks ls` and read one before running it; when no task
fits, run the Alef command directly:

- `alef all --clean` regenerates all Alef-managed output. Without `--skip-compile` it also
  compiles the Swift bridge and runs the FFI header gate, so a subagent without build scope must
  pass `--skip-compile` or leave regeneration to the orchestrator.
- `alef verify` checks that generated output is fresh; regeneration does not run it.
- `alef e2e generate`, then `alef test --e2e` (which also runs each language's own suite), is the
  e2e loop.
- `alef build` builds the language bindings; `cargo build` builds the Rust core only.
- `alef fmt` formats generated output.

## Regeneration Is an Audit

- `alef generate`/`all` exit 0 while warning. Capture the log and read every warning (`refusing
  to write`, `Unavailable`, `Unsupported`, `0 files`, suspiciously round counts).
- A handful of files where many were expected, or `Generated 0 files` after a config change, is a
  cache hit: rerun with `--clean` (`alef cache clear` drops `.alef/`). Changing `alef_version`
  does not change the cache key.
- A pre-existing file without an alef marker stays un-owned until `alef adopt <path> --write`;
  `RUST_LOG=alef=debug` shows `refusing to write`.
- Most diffs are `alef:hash:` churn; review `git diff --numstat | awk '$1>2 || $2>2'`.
- `alef verify` content-verifies marked files and files it can byte-compare with a fresh render.
  It lists the rest (create-once seeds, user-owned files) as "present but NOT content-verified"
  and passes them, and a drifted create-once seed (`frozen path(s) DRIFTED`) never fails the
  exit code.
- Never run two `alef snippets check` passes at once in one repo; they share `.alef/snippets`
  session dirs.
- CI's `install-alef` installs the `alef.toml` `alef_version` pin (the newest release only when
  unpinned); a step's `version:` overrides it. Compare the pin with `alef --version`.

## Versions

- `Cargo.toml` is the single source of the version. Never hand-edit a binding manifest's version.
- Use the full version task: `version:sync`, `version:bump:{major,minor,patch}` and
  `version:check` (plus `version:set` in some repos). Besides the manifests, it regenerates
  everything derived from the version (generate, stubs, scaffold, readme, test-apps, e2e) and
  syncs the coding-agent plugin pin (`plugin/.ai-rulez/config.plugins.toml` `[plugin].version`).
- `alef sync-versions` alone rewrites only the manifests and leaves the plugin pin and derived
  files behind; that is how releases have been tagged broken. A repo's `alef:sync` task may be
  that narrow command or may delegate to the full sync, so read it before relying on it.

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
alef verify
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
