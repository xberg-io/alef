# Alef CLI Reference

All commands accept these global flags:

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to `alef.toml` (default: `alef.toml`) |
| `-j`, `--jobs <n>` | Maximum parallel jobs (`0` = all cores, `1` = sequential, default `0`) |
| `-v`, `--verbose` | Increase log verbosity (`-v` info, `-vv` debug, `-vvv` trace). Overridden by `RUST_LOG`. |
| `-q`, `--quiet` | Suppress everything below `error`. Overridden by `RUST_LOG`. |
| `--no-color` | Disable ANSI colour in log output |
| `-V`, `--version` | Print the alef CLI version and exit |
| `-h`, `--help` | Print help |

```text
alef [GLOBAL OPTIONS] <command> [OPTIONS]
```

---

## `alef extract`

Extract API surface from Rust source into an intermediate representation (IR) JSON file.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-o`, `--output` | path | `.alef/ir.json` | Output IR JSON file path |

```bash
alef extract
alef extract --output api-surface.json
```

The IR contains all extracted types, functions, enums, and errors from the configured Rust source files.

---

## `alef generate`

Generate language bindings from the extracted IR. Also generates type stubs and public API wrappers when enabled in config.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to generate bindings for |
| `--clean` | bool | `false` | Ignore cache and regenerate everything |
| `--format` | bool | `false` | Run post-generation formatters on emitted files (off by default) |

```bash
alef generate
alef generate --lang python,node
alef generate --clean
alef generate --format          # also run language formatters after generation
alef generate --lang ruby --clean
```

Formatters are **opt-in via `--format`**. With `--format`, alef invokes language-native formatters (`cargo fmt`, `ruff format`, `biome format`, `gofmt`, etc.) plus any repo-configured `[lint.<lang>].format` commands as a best-effort post-generation step: a missing tool, a failing `before` hook, or a non-zero formatter exit emits a warning and the run continues. Without `--format`, generated files are written exactly as the codegen emits them (already whitespace-normalised). Formatters run **only for languages whose bindings actually regenerated this run** — unchanged languages skip the formatter pass entirely.

Caching is based on blake3 content hashing of source files and config -- use `--clean` to force regeneration.

---

## `alef stubs`

Generate type stub files for editor support and static analysis: `.pyi` (Python), `.rbs` (Ruby), `.d.ts` (TypeScript).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to generate stubs for |

```bash
alef stubs
alef stubs --lang python
```

Caching: skips generation when the IR and config have not changed since the last run.

---

## `alef scaffold`

Generate complete package manifests for each language (`pyproject.toml`, `package.json`, `.gemspec`, `composer.json`, `mix.exs`, `go.mod`, `pom.xml`, `.csproj`, `DESCRIPTION`, `Cargo.toml`).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to scaffold for |

```bash
alef scaffold
alef scaffold --lang go,java
```

Caching: skips generation when the IR and config have not changed since the last run.

---

## `alef readme`

Generate per-language README files from templates.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to generate READMEs for |

```bash
alef readme
alef readme --lang python,node
```

Caching: skips generation when the IR and config have not changed since the last run.

---

## `alef docs`

Generate API reference documentation in Markdown format (suitable for mkdocs).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to generate docs for |
| `--output` | string | `docs/reference` | Output directory for generated documentation |

```bash
alef docs
alef docs --lang python --output docs/api
```

Caching: skips generation when the IR and config have not changed since the last run.

---

## `alef sync-versions`

Sync the version from `Cargo.toml` (or the file specified by `[crate].version_from`) to all package manifests and any configured `[sync]` targets.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--bump` | string | -- | Bump version before syncing: `major`, `minor`, or `patch` |

```bash
alef sync-versions
alef sync-versions --bump patch
alef sync-versions --bump minor
```

Updates all auto-detected package manifests plus any files listed in `[sync].extra_paths` and `[[sync.text_replacements]]`.

---

## `alef build`

Build language bindings using native tools (`maturin`, `napi build`, `wasm-pack`, `cargo build` + `cbindgen`).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to build |
| `-r`, `--release` | bool | `false` | Build with release optimizations |

```bash
alef build
alef build --lang node
alef build --release
alef build --lang python,wasm --release
```

The build profile is `dev` by default and `release` when `--release` is passed. Post-processing steps (such as patching `.d.ts` files for `verbatimModuleSyntax` compatibility) run automatically.

Build commands are configurable via `[build_commands.<lang>]` in `alef.toml`. Built-in defaults cover all supported languages; override them when your project uses non-standard tooling.

---

## `alef lint`

Run configured lint and format commands on generated output. Commands are defined in `[lint.<lang>]` sections of `alef.toml`. Built-in defaults are provided for all 12 languages.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to lint |

```bash
alef lint
alef lint --lang python
```

---

## `alef fmt`

Run only the format phase of lint (the `format` field from `[lint.<lang>]`). A subset of `alef lint` that skips check and typecheck commands.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to format |

```bash
alef fmt
alef fmt --lang python,node
```

---

## `alef test`

Run configured test suites for each language. Commands are defined in `[test.<lang>]` sections of `alef.toml`.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to test |
| `--e2e` | bool | `false` | Also run e2e tests (uses `[test.<lang>].e2e` commands) |
| `--coverage` | bool | `false` | Run coverage commands (uses `[test.<lang>].coverage` commands) |

```bash
alef test
alef test --lang python,go
alef test --e2e
alef test --lang node --e2e
alef test --coverage
alef test --lang python --coverage
```

---

## `alef update`

Update dependencies for each language using per-language defaults or `[update.<lang>]` config. Safe updates (compatible versions) are run by default; `--latest` enables aggressive upgrades including incompatible/major version bumps.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to update |
| `--latest` | bool | `false` | Use the `upgrade` commands for aggressive/incompatible updates |

```bash
alef update
alef update --lang python,node
alef update --latest
alef update --lang rust --latest
```

Output is streamed live to the terminal, prefixed with `[<lang>] ` when multiple languages run in parallel. Failures surface as `✗ update failed: <lang> — <error>`.

---

## `alef setup`

Install dependencies for each language using per-language defaults or `[setup.<lang>]` config. Runs before building or testing when dependencies are not yet installed.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to set up |

```bash
alef setup
alef setup --lang python,node
```

Output is streamed live to the terminal, prefixed with `[<lang>] ` when multiple languages run in parallel — installers like `pnpm install`, `bundle install`, and `uv sync` print progress immediately instead of buffering until the command exits. Failures surface as `✗ setup failed: <lang> — <error>`.

---

## `alef clean`

Clean build artifacts for each language using per-language defaults or `[clean.<lang>]` config.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from config | Languages to clean |

```bash
alef clean
alef clean --lang rust,node
```

---

## `alef verify`

Confirm every alef-generated file on disk reflects the current Rust source and was not edited or mutated post-format.

### Mental model (what verify is)

`alef verify` is a **per-file source+output hash comparison**, not a regenerate-and-diff. The hash baked into the comment header of every alef-generated file (the `alef:hash:<hex>` line) is:

```text
alef:hash:<hex> = blake3( sources_hash || file_content_without_hash_line )
sources_hash    = blake3( sorted(rust_source_files) )
```

— a per-file fingerprint of (a) the rust sources alef parses to build the IR and (b) the actual on-disk byte content of *this* file. The hash deliberately does **not** include the alef CLI version or `alef.toml`: any input change that affects the generated bytes is already reflected by hashing the file content itself, and excluding the alef version is what makes verify idempotent across alef CLI upgrades.

`alef generate` finalises the embedded hash *after* every formatter has run (rustfmt, rubocop, dotnet format, spotless, biome, mix format, php-cs-fixer, ruff, taplo, …), so the on-disk hash always describes the actual on-disk byte content.

`alef verify` does, in pseudocode:

```text
sources_hash = blake3(sorted(rust_sources))
walk repo (skip target/ node_modules/ _build/ parsers/ dist/ vendor/ .alef/ .git/ … )
for each generated-language file:
    if `alef:hash:<hex>` line is present:
        expected = blake3(sources_hash || strip_hash_line(content))
        if hex != expected: report stale
```

That is the whole algorithm. Verify never runs codegen, never invokes a formatter, never writes anything. With `--exit-code` it exits 1 on the first mismatch (after listing all stale files); without, it just prints them.

### Properties

- **Idempotent across alef CLI versions.** Upgrading the alef CLI does not by itself flag any file as stale; the alef version is not a hash dimension.
- **Symmetric with generate.** The same `compute_file_hash(sources_hash, content)` runs on both sides; verify just recomputes what generate finalised on disk.
- **Repo-wide.** Walks the whole repo for alef-headered files (binding glue, stubs, READMEs, scaffolds with the marker, e2e tests). User-owned scaffolds without the marker (Cargo.toml templates, composer.json, gemspec, package.json shims, lockfiles) are skipped silently — alef has no claim on files it didn't tag.

### Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--exit-code` | bool | `false` | Exit with code 1 if any output is stale (CI mode) |
| `--compile` | bool | `false` | **Accepted but ignored.** Verify is a hash-only check; use `alef build` to compile. |
| `--lint` | bool | `false` | **Accepted but ignored.** Use `alef lint` to lint. |
| `--lang` | string (comma-separated) | all from config | **Accepted but ignored.** Verify is a per-file hash compare; the rust-source side is repo-wide. |

```bash
alef verify              # report any stale alef-generated files
alef verify --exit-code  # exit 1 in CI when stale
```

### When verify reports stale

A file is stale if its embedded `alef:hash` doesn't match the recomputed per-file hash. That happens when (and only when):

- A `[crate].sources` Rust file changed (edited / added / removed / renamed).
- An alef-generated file was edited by hand or mutated by a tool that ran *after* `alef generate` finalised hashes.
- `alef.toml` (or alef itself) changed in a way that produced different generated bytes.

Fix: `alef generate` (and `alef e2e generate` if `[e2e]` is configured). One regenerate finalises the new per-file hash in every alef-headered file in a single pass.

### What verify is not

- **Not a regenerate.** It does not run codegen, stubs, README, or scaffold pipelines. Use `alef diff` if you want to see the actual content that would be written.
- **Not a formatter check.** Run the language's formatter (or `alef fmt`) to enforce style; verify only checks the embedded hash.
- **Not a compile check.** Use `alef build` (or `cargo check` / `cargo build`) to confirm generated code compiles.

### Migration from v0.10.0

v0.9.0–v0.10.0 used a single repo-wide input fingerprint that included the alef CLI version, which forced consumers to regenerate after every alef bump. After upgrading to v0.10.1+, every existing alef-generated file still carries the old uniform hash. Run `alef generate` (and `alef e2e generate` if applicable) once after the upgrade — every alef-headered file is rewritten with the new per-file hash. Generated content itself is unchanged; only the `alef:hash:` value in the header differs.

---

## `alef diff`

Show what files would change without writing anything. Useful for previewing the effect of config or source changes.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--exit-code` | bool | `false` | Exit with code 1 if any changes exist (CI mode) |

```bash
alef diff
alef diff --exit-code
```

Always operates on all configured languages (no `--lang` filter).

---

## `alef all`

Run the full pipeline: generate + stubs + public API + scaffold + readme + docs + e2e generation (when configured) + sync. Equivalent to running `generate`, `stubs`, `scaffold`, `readme`, `docs`, and `e2e generate` in sequence.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--clean` | bool | `false` | Ignore cache and regenerate everything |
| `--format` | bool | `false` | Run post-generation formatters on emitted files (off by default) |

```bash
alef all
alef all --clean
alef all --format
```

Always operates on all configured languages. Like `alef generate`, formatting is opt-in via `--format` and only runs for languages that actually regenerated this run.

---

## `alef init`

Initialize a new `alef.toml` configuration file in the current directory.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | -- | Languages to include in the generated config |
| `--format` | bool | `false` | Run post-generation formatters on emitted files (off by default) |

```bash
alef init
alef init --lang python,node,ruby,go
alef init --format
```

---

## `alef e2e` -- E2E Test Subcommands

Generate and manage fixture-driven e2e test suites. Requires an `[e2e]` section in `alef.toml`.

### `alef e2e generate`

Generate e2e test projects from JSON fixture files.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from `[e2e].languages` | Languages to generate tests for |
| `--registry` | bool | `false` | Generate standalone test apps using published registry package versions instead of local path dependencies |

```bash
alef e2e generate
alef e2e generate --lang python,rust
```

Caching: skips generation when fixtures, IR, and config have not changed. The cache hash includes the full contents of the fixtures directory.

Per-language formatters are run automatically on generated test files.

### `alef e2e init`

Initialize the fixture directory with a JSON schema file and an example fixture.

```bash
alef e2e init
```

No flags. Creates the directory specified by `[e2e].fixtures` if it does not exist.

### `alef e2e scaffold`

Scaffold a new fixture JSON file with the correct structure.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--id` | string | *required* | Fixture ID (snake_case, used as test function name) |
| `--category` | string | *required* | Category name (e.g., `smoke`, `basic`, `edge-case`) |
| `--description` | string | *required* | Human-readable description of the test |

```bash
alef e2e scaffold --id parse_empty_input --category edge-case --description "Parsing empty input returns error"
```

### `alef e2e list`

List all fixtures with counts per category.

```bash
alef e2e list
```

No flags. Reads from the directory specified by `[e2e].fixtures`.

### `alef e2e validate`

Validate all fixture files against the JSON schema. Exits with code 1 if validation errors are found.

```bash
alef e2e validate
```

No flags. Reports all validation errors with details.

---

## `alef cache` -- Cache Management

Manage the `.alef/` build cache directory. Alef uses blake3-based content hashing to skip regeneration when source files and config have not changed.

### `alef cache clear`

Clear the entire `.alef/` cache directory.

```bash
alef cache clear
```

### `alef cache status`

Show current cache status, including which stages have cached hashes.

```bash
alef cache status
```

---

## `alef publish` -- Release Pipeline

Vendor, cross-compile, and package release artifacts for distribution to language package registries. Configured via `[publish]` in `alef.toml`. Each subcommand respects per-language `precondition` / `before` / `after` hooks declared in `[publish.languages.<lang>]`.

### `alef publish prepare`

Stage everything needed for a publishable build: vendor the core crate (Ruby, Elixir, R via `cargo vendor` for `vendor_mode = "full"`, or path-rewriting copy for `"core-only"`), copy FFI shared libraries and headers into the language package directories.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from `[publish.languages]` | Languages to prepare for |
| `--target` | string | host triple | Rust target triple for cross-compilation (e.g. `x86_64-unknown-linux-gnu`) |
| `--dry-run` | bool | `false` | Show what would be done without executing |

```bash
alef publish prepare
alef publish prepare --lang ruby,elixir
alef publish prepare --target aarch64-apple-darwin --dry-run
```

### `alef publish build`

Build release artifacts for a specific platform. Auto-selects the right tool per language: `cargo` / `cross` for Rust core and FFI, `maturin build` for Python, `napi build` for Node, `wasm-pack build` for Wasm.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from `[publish.languages]` | Languages to build |
| `--target` | string | host triple | Rust target triple |
| `--use-cross` | bool | `false` | Use [`cross`](https://github.com/cross-rs/cross) instead of `cargo` for cross-compilation |

```bash
alef publish build --target x86_64-unknown-linux-gnu --use-cross
alef publish build --lang python,node --target aarch64-apple-darwin
```

### `alef publish package`

Assemble distributable archives. Output formats per language: C FFI tarball with pkg-config + CMake configs; PHP PIE archive; Go FFI tarball; per-platform wheels / npm packages / `.gem` files for the high-level binding languages.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--lang` | string (comma-separated) | all from `[publish.languages]` | Languages to package |
| `--target` | string | host triple | Rust target triple (auto-maps to language-specific platform names — Go's `linux_amd64`, Node's `linux-x64-gnu`, etc.) |
| `-o`, `--output` | string | `dist` | Output directory for packages |
| `--version` | string | from `Cargo.toml` | Override the version string written into archives |
| `--dry-run` | bool | `false` | Show what would be packaged without executing |

```bash
alef publish package --target x86_64-unknown-linux-gnu
alef publish package --lang ruby --output ./artifacts
alef publish package --version 1.2.3 --dry-run
```

### `alef publish validate`

Sanity-check that everything required for `cargo publish` / `pnpm publish` / `gem push` etc. is consistent: version readability, package directory existence, manifest presence, vendored dependencies. Designed to run in CI before invoking the actual `publish` step in your release workflow.

```bash
alef publish validate
```

No flags. Exits non-zero on validation failures with a list of issues.
