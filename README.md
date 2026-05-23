[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/alef)](https://crates.io/crates/alef)
[![CI](https://github.com/kreuzberg-dev/alef/actions/workflows/ci-rust.yaml/badge.svg)](https://github.com/kreuzberg-dev/alef/actions)
[![Built with alef](https://img.shields.io/badge/built%20with-alef%20%D7%90-007ec6)](https://github.com/kreuzberg-dev/alef)

# alef

Polyglot binding generator for Rust libraries. alef generates complete language bindings from a single source-of-truth configuration, targeting 18 languages: Python, TypeScript/Node, WebAssembly, C FFI, Ruby, PHP, Elixir, R, Go, Java/JNI, Kotlin, Kotlin Android, C#, Dart, Gleam, Swift, Zig, and Brew CLI.

## Install

```shell
cargo install alef --locked
```

## Quick start

```shell
alef generate              # Generate all configured bindings
alef e2e generate         # Generate end-to-end test suites
alef readme               # Generate README templates for each language
```

See the [full documentation](https://docs.alef.kreuzberg.dev) for configuration, CLI reference, and architecture.

## Ecosystem

- [Kreuzberg](https://github.com/kreuzberg-dev/kreuzberg) — document intelligence: text, tables, metadata from 91+ formats with optional OCR.
- [Kreuzberg Cloud](https://github.com/kreuzberg-dev/kreuzberg-cloud) — managed extraction API with SDKs, dashboards, and observability.
- [kreuzcrawl](https://github.com/kreuzberg-dev/kreuzcrawl) — web crawling and scraping with HTML→Markdown and headless-Chrome fallback.
- [html-to-markdown](https://github.com/kreuzberg-dev/html-to-markdown) — fast, lossless HTML→Markdown engine.
- [liter-llm](https://github.com/kreuzberg-dev/liter-llm) — universal LLM API client with native bindings for 14 languages and 143 providers.
- [tree-sitter-language-pack](https://github.com/kreuzberg-dev/tree-sitter-language-pack) — tree-sitter grammars and code-intelligence primitives.

## License

MIT
