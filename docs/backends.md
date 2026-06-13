# Language Backends Reference

Alef ships 18 language backends. Each implements the `Backend` trait and generates bindings, type
stubs, and package scaffolding for one target language.

## Backends vs Extensions

Backends emit one language target from the extracted `ApiSurface`. Extensions augment what backends
emit or add custom IR sections to the surface. They are complementary: a backend handles
"generate Python bindings"; an extension handles "generate HTTP service API alongside the bindings."

Backends are part of alef itself. Consumers author extensions.

## The Backend Trait

Each backend implements:

- `name()` — identifier string (for example, `"pyo3"`)
- `language()` — `Language` enum variant
- `capabilities()` — what the backend supports (async, classes, enums, options, results, callbacks, streaming)
- `generate_bindings()` — Rust binding source code via `#[pyclass]`, `#[napi]`, Magnus, and so on.
- `generate_type_stubs()` — language-specific `.pyi`, `.d.ts`, `.rbs` type definitions
- `generate_scaffold()` — package manifests (`pyproject.toml`, `package.json`, `Gemfile`, etc.)
- `generate_public_api()` — optional idiomatic wrappers
- `build_config()` — build tool, output crate suffix, FFI library dependency

See `src/core/backend.rs` in the alef repository for the full trait definition.

## Built-in Backends

| Language | Backend | Source | Emits |
|---|---|---|---|
| Python | `Pyo3Backend` | `src/backends/pyo3/` | PyO3 module, `.pyi` stubs, package scaffold |
| TypeScript / Node | `NapiBackend` | `src/backends/napi/` | NAPI-RS addon, `.d.ts` types, package scaffold |
| WebAssembly | `WasmBindgenBackend` | `src/backends/wasm/` | wasm-bindgen package and `.d.ts` stubs |
| Ruby | `MagnusBackend` | `src/backends/magnus/` | Magnus extension, `.rbs` types, gem scaffold |
| PHP | `ExtPhpRsBackend` | `src/backends/php/` | ext-php-rs native extension with `composer.json` and PHP type stubs |
| Go | `CgoBackend` | `src/backends/go/` | cgo wrapper over generated C FFI with idiomatic receiver methods |
| Java | `PanamaBackend` | `src/backends/java/` | JVM package via Panama FFM (Java 19+) with Maven scaffolding |
| Kotlin / JVM | `KotlinBackend` | `src/backends/kotlin/` | Kotlin/JVM wrappers over JNI shims with Gradle build files |
| Kotlin Android | `KotlinAndroidBackend` | `src/backends/kotlin_android/` | Android AAR with JNI bridge objects |
| C# | `CsharpBackend` | `src/backends/csharp/` | .NET package via P/Invoke with `.csproj` metadata |
| Dart / Flutter | `DartBackend` | `src/backends/dart/` | flutter_rust_bridge package and Dart types |
| Swift | `SwiftBackend` | `src/backends/swift/` | Swift package with C interop and `.swiftmodule` type declarations |
| Zig | `ZigBackend` | `src/backends/zig/` | Zig package wrapping the C ABI with `build.zig` and Zig type bindings |
| R | `ExtendRBackend` | `src/backends/r/` | extendr R package with roxygen docs and CRAN scaffolding |
| Elixir | `RustlerBackend` | `src/backends/rustler/` | Rustler NIF package with `mix.exs` |
| Gleam | `GleamBackend` | `src/backends/gleam/` | Gleam package via Rustler NIFs with `gleam.toml` and type stubs |
| C FFI | `FfiBackend` | `src/backends/ffi/` | C header file via cbindgen and shared library wrapper |
| JNI | `JniBackend` | `src/backends/jni/` | JNI shim crate emitting `Java_*` extern functions for Android and JVM use |

## Adding a New Backend

Backend extensibility (consumer-defined backends) is not yet supported. To add a new backend, contribute to alef itself:

1. Create `src/backends/{language}/` with a new backend struct implementing `Backend`
2. Register the backend in `src/cli/registry.rs` in the backend registry match
3. Add language-specific generation modules and templates
4. Add a new test fixture under `tests/fixtures/backends/{language}/`

This is distinct from the Extension trait, which lets consumers augment existing backends with
domain-specific files without modifying alef itself.
