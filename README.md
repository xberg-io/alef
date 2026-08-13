<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://cdn.jsdelivr.net/gh/xberg-io/assets@v1/banner/readme-banner-dark.svg">
    <img alt="Xberg" width="420" src="https://cdn.jsdelivr.net/gh/xberg-io/assets@v1/banner/readme-banner-light.svg">
  </picture>
</p>

# Alef

<div align="center" style="display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; margin: 20px 0;">
  <a href="https://crates.io/crates/alef">
    <img src="https://img.shields.io/crates/v/alef?label=Crates.io&color=007ec6" alt="Crates.io">
  </a>
  <a href="https://docs.rs/alef">
    <img src="https://img.shields.io/docsrs/alef?label=docs.rs&color=007ec6" alt="docs.rs">
  </a>
  <a href="https://github.com/xberg-io/alef/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/xberg-io/alef/ci.yml?branch=main&label=CI&color=007ec6" alt="CI">
  </a>
  <a href="https://github.com/xberg-io/alef/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/License-MIT-007ec6" alt="License">
  </a>
  <a href="https://www.rust-lang.org">
    <img src="https://img.shields.io/badge/Rust-1.85%2B-007ec6?logo=rust&logoColor=white" alt="Rust 1.85+">
  </a>
  <a href="#supported-targets">
    <img src="https://img.shields.io/badge/Targets-18-007ec6" alt="18 targets">
  </a>
</div>

<div align="center" style="margin: 28px 0 18px;">
  <div style="font-size: 72px; line-height: 1;">א</div>
  <strong>Rust in. Native bindings out.</strong>
</div>

<div align="center" style="display: flex; flex-wrap: wrap; gap: 12px; justify-content: center; margin: 24px 0;">
  <a href="https://github.com/xberg-io/alef">
    <img height="22" src="https://img.shields.io/badge/GitHub-xberg--io%2Falef-007ec6?logo=github&logoColor=white" alt="GitHub">
  </a>
  <a href="https://discord.gg/xt9WY3GnKR">
    <img height="22" src="https://img.shields.io/badge/Discord-Chat-007ec6?logo=discord&logoColor=white" alt="Discord">
  </a>
</div>

Alef is the polyglot binding generator behind the xberg.io ecosystem. It extracts a Rust API surface
and emits language-native bindings, package scaffolding, type stubs, README files, API docs, e2e
tests, and release metadata from one `alef.toml`.

**[Installation](#installation)** | **[Quick Start](#quick-start)** | **[Supported Targets](#supported-targets)** |
**[CLI Reference](#cli-reference)**

## Key Features

- **One source of truth** - Configure a Rust workspace once and generate every enabled language target from it.
- **Language-native bindings** - Emit host-language types, docs, errors, async wrappers, callbacks, and package files.
- **Multi-crate workspaces** - Drive multiple independently published binding packages from a shared workspace config.
- **End-to-end fixtures** - Generate cross-language test suites and registry-mode test apps from shared JSON fixtures.
- **Release-aware packaging** - Sync versions, generate registry metadata, build artifacts, and validate publication state.
- **Configurable pipelines** - Run setup, update, format, lint, test, clean, build, and publish commands per language.
- **Pluggable extension surface** - Author domain-specific codegen logic via the `Extension` trait; ship as linked binaries, dynamic libraries, or template-only declarations.
- **Staleness checks** - Cache inputs, embed generation hashes, and verify whether generated files are up to date.

## Installation

Alef requires Rust 1.85 or newer.

```bash
cargo install alef --locked
```

If you use [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall), Alef also publishes
binary-install metadata:

```bash
cargo binstall alef
```

## Quick Start

Create or edit `alef.toml` in your Rust workspace:

```toml
[workspace]
languages = ["python", "node", "ffi", "go"]
alef_version = "0.24.12"

[[crates]]
name = "sample_core"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
```

Then generate the language packages:

```bash
alef generate --format
alef scaffold
alef readme
alef docs --output docs/reference
alef verify --exit-code
```

For a new project, Alef can create the initial config and first generated files:

```bash
alef init --lang python,node,ffi
```

For the full local generation pass, use:

```bash
alef all --format
```

Use `--lang python,node` to restrict commands to selected targets and `--crate <name>` to restrict
commands to one configured crate.

## Supported Targets

| Target | Backend / package style |
| ------ | ----------------------- |
| Python | PyO3 bindings with Python type stubs |
| TypeScript / Node.js | NAPI-RS native addon with `.d.ts` output |
| WebAssembly | wasm-bindgen package for browser and JS runtimes |
| Ruby | Magnus native extension |
| PHP | Native PHP extension |
| Elixir | Rustler NIF package |
| R | extendr package |
| Go | cgo package over the generated C FFI layer |
| Java | JVM package over the generated native library |
| Kotlin | Kotlin/JVM package over generated native bindings |
| Kotlin Android | Android package with generated JNI shims |
| C# | .NET package using P/Invoke |
| Dart / Flutter | flutter_rust_bridge package |
| Swift | Swift package with Rust bridge support |
| Zig | Zig package over the generated C ABI |
| Gleam | Gleam package backed by Rustler |
| C FFI | C ABI, header, and shared-library glue |
| JNI | Rust JNI shim crate exercised by both kotlin_android (Android AAR) and host-JVM tests |

Canonical language slugs are `python`, `node`, `wasm`, `ruby`, `php`, `elixir`, `r`, `go`,
`java`, `csharp`, `kotlin`, `kotlin_android`, `swift`, `dart`, `gleam`, `zig`, `ffi`, and `jni`.

## Configuration Model

Alef uses the current multi-crate schema:

- `[workspace]` stores shared target languages, tool preferences, and pipeline defaults.
- `[[crates]]` describes each Rust API surface that should become one or more published packages.
- `[crates.<language>]` sections customize module names, package names, feature flags, output paths,
  field naming, dependency extras, and language-specific generation behavior.
- `[[crates.adapters]]`, trait bridge config, service API config, and e2e config opt into higher-level
  generated wrappers when a target supports them.

Generated binding files carry Alef hashes and are overwritten by generation commands. Scaffolded
package files are generated once unless the command explicitly opts into overwrite behavior; generated
README and API doc files are owned by `alef readme` and `alef docs`.

## Downloadable Native Components

Alef can split fixed Cargo feature profiles into signed native components instead of putting every
feature combination in every wheel. A contract is an explicitly selected Rust trait; Alef generates a
versioned C function table for it, a producer `cdylib`, and a host runtime. Component implementation
types currently construct through `Default`. Only C-compatible scalar,
UTF-8, byte-buffer, and opaque-handle representations cross the dynamic-library boundary—never Rust
trait objects or Rust-owned layouts.

```toml
[[crates.component_contracts]]
name = "engine"
trait_path = "my_core::Engine"
interface_version = 1

[[crates.components]]
name = "cuda"
contract = "engine"
implementation = "my_core::CudaEngine"
features = ["cuda"]
default_features = false
targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]

[crates.component_distribution]
url_template = "https://downloads.example.com/{component}/{version}/{target}/{artifact}"

[crates.component_distribution.public_keys]
release-2026 = "BASE64_ENCODED_32_BYTE_ED25519_PUBLIC_KEY"
```

Generate/scaffold first, then build and sign each configured target in CI:

```console
alef scaffold
alef component build
alef component package --signing-key "$CI_ED25519_PRIVATE_KEY_FILE" --key-id release-2026
alef component verify
alef component lock
```

The private Ed25519 key stays in protected CI secret storage. It signs canonical component manifests;
it does not encrypt the binary. Generated native binding packages compile in the contents of
`components.lock.json`, including the public verification keys, but do not contain the component
libraries. Each supported binding exposes idiomatic `load`, `prefetch`, `status`, and `cache path`
operations. The runtime selects the exact target, downloads on demand, verifies the pinned size,
SHA-256 digest, and Ed25519 signature, installs under a cache lock, validates the
ABI/contract/feature identity, and keeps the library loaded for the process lifetime.

| Component host surface | Bindings |
| ---------------------- | -------- |
| Direct native runtime | Python, Node.js, Ruby, PHP, Elixir, R, Dart/FRB, Swift |
| Shared generated C manager | C FFI, Go, Java, Kotlin/JVM, C#, Zig, Dart/FFI |
| Rustler manager | Gleam |
| Explicitly unsupported | WebAssembly, Kotlin Android, Android JNI, Apple mobile targets |

Unsupported bindings still generate the four management operations, but each operation returns a
clear platform error instead of silently omitting the API. Rust callers can use
`alef-component-runtime::ComponentManager` directly.

The initial loader supports x86-64 and ARM64 Linux GNU, x86-64 and ARM64 macOS, and x86-64 Windows
MSVC. Mobile and WebAssembly targets are intentionally excluded because they do not provide the same
dynamic-loading model.

## Extending Alef

Alef is opinionated about codegen and neutral about domain. The `Extension` trait lets you ship domain-specific generation logic (HTTP service APIs, plugin registries, custom bindings) without bloat in alef.

### Linked Extension

Consumer crate implements `alef::Extension`, ships a thin CLI binary:

```rust
fn main() {
    alef::run_with_extensions(vec![Box::new(MyDomainExtension)])
}
```

Full type safety. Recommended for frameworks that generate an HTTP service API.

### Dynamic Extension

Load a compiled `.so`/`.dylib`/`.dll` declaring a C-ABI factory function. Works when you can't ship a Rust binary.

```toml
[[extensions.dylib]]
path = "target/release/libmy_extension.dylib"
```

```rust
extern "C" fn alef_extension_factory() -> Box<dyn alef::Extension> {
    Box::new(MyExtension)
}
```

### Template-only Extension

Declare `[[extensions.template]]` blocks in `alef.toml` pointing to Jinja templates. Alef's built-in `TemplateExtension` emits them — no Rust required.

The full extension walkthrough covers trait references and per-language emission patterns.

## CLI Reference

| Command | Purpose |
| ------- | ------- |
| `alef init` | Create `alef.toml`, generate initial bindings, and scaffold package files. |
| `alef extract` | Extract Rust source into Alef IR JSON. |
| `alef generate` | Generate bindings, service API wrappers, public API wrappers, and type stubs. |
| `alef stubs` | Generate type stubs only. |
| `alef scaffold` | Generate package manifests, native build files, and package scaffolding. |
| `alef readme` | Generate per-language README files. |
| `alef docs` | Generate Markdown API reference pages. |
| `alef setup` | Install per-language development dependencies. |
| `alef fmt` / `alef lint` | Run configured formatters, linters, and type checks. |
| `alef test` | Run configured unit, integration, e2e, or coverage test commands. |
| `alef build` | Build language bindings using native tools. |
| `alef verify` | Check generated files and optional compile/lint state for CI. |
| `alef diff` | Show what generation would change without writing files. |
| `alef e2e` | Initialize, scaffold, validate, list, or generate local e2e suites. |
| `alef test-apps` | Generate and run standalone registry-mode test applications. |
| `alef publish` | Prepare, build, package, and validate release artifacts. |
| `alef all` | Run the full generation workflow in one command. |

Run `alef --help` or `alef <command> --help` for the full option set.

## Development

This repository uses `task` for common workflows:

```bash
task setup
task build
task test
task lint
```

The most useful targeted commands while working on Alef itself are:

```bash
cargo test <module_or_test_name>
cargo insta review
prek run --all-files
```

## Part of Xberg.io

- [Xberg](https://github.com/xberg-io/xberg) — the open-source content-intelligence engine: text, tables, and metadata from 101 formats (115 file extensions), with OCR, transcription, and code intelligence. MIT.
- [Xberg Pro](https://xberg.io) — a complete self-hosted content-intelligence backend in a single container. Commercial.
- [Xberg Enterprise](https://xberg.io) — the distributed, governed content-intelligence platform, scaled on Kubernetes with team governance and support. Commercial.
- [crawlberg](https://github.com/xberg-io/crawlberg) — web crawling and scraping with HTML→Markdown and headless-Chrome fallback.
- [html-to-markdown](https://github.com/xberg-io/html-to-markdown) — fast, lossless HTML→Markdown engine.
- [liter-llm](https://github.com/xberg-io/liter-llm) — universal LLM API client with native bindings for 14 languages and 165 providers.
- [tree-sitter-language-pack](https://github.com/xberg-io/tree-sitter-language-pack) — tree-sitter grammars and code-intelligence primitives.
- [alef](https://github.com/xberg-io/alef) — the polyglot binding generator that produces every per-language binding across the 5 polyglot repos.
- [Discord](https://discord.gg/xt9WY3GnKR) — community, roadmap, and release discussion.

## License

MIT - see [LICENSE](LICENSE) for details.
