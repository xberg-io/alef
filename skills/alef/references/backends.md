# Language Backend Reference

Alef supports 16 language backends: Python (PyO3), TypeScript/Node (NAPI-RS), WebAssembly (wasm-bindgen), Ruby (Magnus), PHP (ext-php-rs), Go (cgo), Java/Kotlin (Panama FFM), C# (P/Invoke), Elixir (Rustler), Gleam (Rustler NIF + `@external`), R (extendr), Swift (swift-bridge), Dart (flutter_rust_bridge), Zig (C FFI), and C (cbindgen). Each backend implements the `Backend` trait from `alef-core` and generates binding code from the extracted IR (`ApiSurface`).

## Backend Trait

Every backend implements:

- `name()` -- identifier string (e.g., `"pyo3"`)
- `language()` -- `Language` enum variant
- `capabilities()` -- what the backend supports (async, classes, enums, options, results, callbacks, streaming)
- `generate_bindings()` -- primary Rust binding source code
- `generate_type_stubs()` -- optional `.pyi`, `.rbs`, `.d.ts` stubs
- `generate_scaffold()` -- optional package manifests
- `generate_public_api()` -- optional idiomatic wrappers
- `build_config()` -- build tool, crate suffix, FFI dependency flag

---

## Python (PyO3)

- **Framework**: [PyO3](https://pyo3.rs)
- **Crate**: `alef-backend-pyo3`
- **Build tool**: `maturin`
- **Crate suffix**: `-py`
- **Depends on FFI**: No

### DTO styles

Configured via `[dto] python = "..."` and optionally `[dto] python_output = "..."` (output types can differ from input types).

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `@dataclass` | `"dataclass"` (default) | Standard immutable data containers. Good general-purpose choice. |
| `TypedDict` | `"typed-dict"` | When consumers prefer plain dicts. Useful for output/return types. |
| `pydantic.BaseModel` | `"pydantic"` | When validation, serialization, and schema generation are needed. |
| `msgspec.Struct` | `"msgspec"` | When high-performance serialization is the priority. |

### Generated output

- `generated_bindings.rs` -- `#[pyclass]`/`#[pymethods]` wrappers, `From` impls, module init
- `options.py` -- Python-side enums (`StrEnum`) and config dataclasses/TypedDicts
- `.pyi` stubs (when `[python] stubs` is configured)

### Key configuration

```toml
[python]
module_name = "_my_lib"       # Python native module name (default: _{crate_name})
async_runtime = "tokio"       # Async runtime for pyo3-async-runtimes
serde_rename_all = "snake_case"  # Field naming (default: snake_case)

[python.stubs]
output = "packages/python/src/my_lib/"
```

### Known limitations

- `Duration` fields on `has_default` types become `Option<u64>` to avoid `Duration::ZERO` defaults overriding the core type's `Default`
- Python keywords (`from`, `type`, `class`, etc.) get an `_` suffix appended automatically
- GIL must be explicitly released for CPU-intensive work via `gil_release = true` on adapters

---

## TypeScript / Node.js (NAPI-RS)

- **Framework**: [NAPI-RS](https://napi.rs)
- **Crate**: `alef-backend-napi`
- **Build tool**: `napi`
- **Crate suffix**: `-node`
- **Depends on FFI**: No

### DTO styles

Configured via `[dto] node = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `interface` | `"interface"` (default) | Standard TypeScript interfaces. Best for most use cases. |
| `zod` | `"zod"` | When runtime validation schemas are needed alongside type definitions. |

### Generated output

- `generated_bindings.rs` -- `#[napi]` wrappers with `Js`-prefixed struct names
- `index.d.ts` -- auto-generated TypeScript definitions (post-processed to fix `const enum` to `enum`)

### Key configuration

```toml
[node]
package_name = "my-lib"          # npm package name
serde_rename_all = "camelCase"   # Field naming (default: camelCase)
```

### Known limitations

- Post-build step patches `export declare const enum` to `export declare enum` in `.d.ts` files
- `BigInt` used for `u64`/`i64` types

---

## WebAssembly (wasm-bindgen)

- **Framework**: [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/)
- **Crate**: `alef-backend-wasm`
- **Build tool**: `wasm-pack`
- **Crate suffix**: `-wasm`
- **Depends on FFI**: No

### DTO styles

No configurable DTO styles. Uses wasm-bindgen's native type mapping with `JsValue` for dynamic types.

### Generated output

- `generated_bindings.rs` -- `#[wasm_bindgen]` annotated types and functions

### Key configuration

```toml
[wasm]
exclude_functions = ["blocking_fn"]  # Functions to skip (e.g., blocking I/O)
exclude_types = ["InternalType"]     # Types to skip
type_overrides = { "Path" = "String" }  # Remap types for WASM compatibility
serde_rename_all = "camelCase"       # Field naming (default: camelCase)
```

### Known limitations

- No `std::thread` (single-threaded environment)
- No synchronous I/O operations
- Panics become JS exceptions
- `exclude_functions` and `exclude_types` needed to skip APIs that cannot work in WASM

---

## Ruby (Magnus)

- **Framework**: [Magnus](https://github.com/matsadler/magnus)
- **Crate**: `alef-backend-magnus`
- **Build tool**: `cargo`
- **Crate suffix**: `-rb`
- **Depends on FFI**: No

### DTO styles

Configured via `[dto] ruby = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `Struct` | `"struct"` (default) | Ruby `Struct` class. Simple and idiomatic. |
| `Dry::Struct` | `"dry-struct"` | When using the dry-rb ecosystem with type coercion and validation. |
| `Data` | `"data"` | Ruby 3.2+ `Data` class for frozen value objects. |

### Generated output

- `generated_bindings.rs` -- Magnus class definitions with `define_class`, `define_method`
- `.rbs` stubs (when `[ruby] stubs` is configured)

### Key configuration

```toml
[ruby]
gem_name = "my_lib"              # RubyGems package name
serde_rename_all = "snake_case"  # Field naming (default: snake_case)

[ruby.stubs]
output = "packages/ruby/sig/"
```

### Known limitations

- Async methods use `tokio::runtime::Runtime::new()` + `block_on` (Ruby lacks native async)
- GVL (Global VM Lock) must be managed for CPU-intensive operations

---

## PHP (ext-php-rs)

- **Framework**: [ext-php-rs](https://github.com/davidcole1340/ext-php-rs)
- **Crate**: `alef-backend-php`
- **Build tool**: `cargo`
- **Crate suffix**: `-php`
- **Depends on FFI**: No

### DTO styles

Configured via `[dto] php = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `readonly class` | `"readonly-class"` (default) | PHP 8.2+ readonly classes. Type-safe and immutable. |
| `array` | `"array"` | When consumers prefer associative arrays over objects. |

### Generated output

- `generated_bindings.rs` -- `#[php_class]`/`#[php_impl]` annotated types
- PHP stub files (when `[php] stubs` is configured)

### Key configuration

```toml
[php]
extension_name = "my_lib"            # PHP extension name
feature_gate = "extension-module"    # Feature gate for ext-php-rs
serde_rename_all = "snake_case"      # Field naming (default: snake_case)

[php.stubs]
output = "packages/php/src/"
```

### Known limitations

- Async methods use a `WORKER_RUNTIME.block_on()` pattern (PHP lacks native async)
- PHP 8.2+ required

---

## C FFI (cbindgen)

- **Framework**: `#[no_mangle] extern "C"` functions with [cbindgen](https://github.com/mozilla/cbindgen)
- **Crate**: `alef-backend-ffi`
- **Build tool**: `cargo`
- **Crate suffix**: `-ffi`
- **Depends on FFI**: No (this IS the FFI layer)

### DTO styles

No configurable DTO styles. Types are serialized as JSON strings over the C boundary.

### Generated output

- `generated_bindings.rs` -- `extern "C"` functions, opaque handle types, `_new()`/`_free()` pairs
- C header file (name configurable, default: `{prefix}.h`)
- Visitor/callback FFI support (when `visitor_callbacks = true`)

### Key configuration

```toml
[ffi]
prefix = "my_lib"              # Function prefix (e.g., my_lib_new, my_lib_free)
error_style = "last_error"     # Error reporting style (default: "last_error")
header_name = "my_lib.h"       # Generated C header filename
lib_name = "my_lib_ffi"        # Native library name for linking
visitor_callbacks = false       # Generate visitor/callback FFI support
serde_rename_all = "snake_case"  # Field naming (default: snake_case)
```

### Known limitations

- No native async support (`supports_async: false`) -- async is flattened to sync via `block_on`
- All complex types are serialized to JSON strings across the boundary
- Every `_new()` must have a matching `_free()` -- caller owns returned `*mut`
- Foundation layer for Go, Java, and C# backends

---

## Go (cgo + FFI)

- **Framework**: [cgo](https://pkg.go.dev/cmd/cgo) wrapping the C FFI layer
- **Crate**: `alef-backend-go` (generates Go source; depends on `alef-backend-ffi` at build time)
- **Build tool**: `go`
- **Crate suffix**: (none -- generates into `packages/go/`)
- **Depends on FFI**: Yes

### DTO styles

Configured via `[dto] go = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `struct` | `"struct"` (default) | Standard Go structs with JSON tags. Only option currently. |

### Generated output

- Go source files with `cgo` `#include` directives, struct definitions, and wrapper functions
- `go.mod` with module path

### Key configuration

```toml
[go]
module = "github.com/kreuzberg-dev/my-lib"  # Go module path
package_name = "mylib"                       # Go package name
serde_rename_all = "snake_case"              # JSON field naming (default: snake_case)
```

### Known limitations

- All calls go through C FFI with JSON serialization/deserialization overhead
- `C.CString` allocations require `defer C.free(unsafe.Pointer(...))` management
- Streaming adapter pattern not supported

---

## Java (Panama FFM)

- **Framework**: [Panama Foreign Function & Memory API](https://openjdk.org/jeps/454) (Java 21+)
- **Crate**: `alef-backend-java` (generates Java source; depends on `alef-backend-ffi` at build time)
- **Build tool**: `mvn`
- **Crate suffix**: (none -- generates into `packages/java/`)
- **Depends on FFI**: Yes

### DTO styles

Configured via `[dto] java = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `record` | `"record"` (default) | Java 17+ records. Immutable, concise, with auto-generated `equals`/`hashCode`. Only option currently. |

### Generated output

- `NativeLib.java` -- Panama FFM method handles and `Linker.downcallHandle` bindings
- Record classes for each DTO type
- Enum classes
- Main client class with instance methods

### Key configuration

```toml
[java]
package = "dev.kreuzberg"        # Java package name (also used as Maven groupId)
ffi_style = "panama"             # FFI mechanism (default: "panama")
serde_rename_all = "camelCase"   # Field naming (default: camelCase)
```

### Known limitations

- Requires Java 21+ for Panama FFM API
- All calls go through C FFI with JSON serialization overhead
- Streaming adapter pattern not supported
- `Arena.ofConfined()` scoped memory management required

---

## C# (P/Invoke)

- **Framework**: [P/Invoke](https://learn.microsoft.com/en-us/dotnet/standard/native-interop/pinvoke) with `[DllImport]`
- **Crate**: `alef-backend-csharp` (generates C# source; depends on `alef-backend-ffi` at build time)
- **Build tool**: `dotnet`
- **Crate suffix**: (none -- generates into `packages/csharp/`)
- **Depends on FFI**: Yes

### DTO styles

Configured via `[dto] csharp = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `record` | `"record"` (default) | C# record types with `PascalCase` properties. Only option currently. |

### Generated output

- C# source files with `[DllImport]` declarations, record types, and enum definitions
- `.csproj` project file

### Key configuration

```toml
[csharp]
namespace = "MyLib"                  # C# namespace (default: PascalCase of crate name)
target_framework = "net8.0"          # Target .NET framework
serde_rename_all = "camelCase"       # Field naming (default: camelCase)
```

### Known limitations

- All calls go through C FFI with JSON serialization overhead
- `Marshal.PtrToStringUTF8` used for string marshaling
- `IntPtr.Zero` check required for error detection
- Streaming adapter pattern not supported

---

## Elixir (Rustler)

- **Framework**: [Rustler](https://github.com/rusterlium/rustler)
- **Crate**: `alef-backend-rustler`
- **Build tool**: `mix`
- **Crate suffix**: `-rustler`
- **Depends on FFI**: No

### DTO styles

Configured via `[dto] elixir = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `struct` | `"struct"` (default) | Elixir `defstruct`. Standard and idiomatic. |
| `typed_struct` | `"typed-struct"` | When using the `TypedStruct` library for compile-time type enforcement. |

### Generated output

- `generated_bindings.rs` -- Rustler NIF functions with `#[rustler::nif]`, `NifStruct`/`NifUnitEnum` derives

### Key configuration

```toml
[elixir]
app_name = "my_lib"              # Elixir application name
serde_rename_all = "snake_case"  # Field naming (default: snake_case)
```

### Known limitations

- Async methods use `tokio::runtime::Runtime::new()` + `block_on` (BEAM scheduler considerations)
- Long-running NIFs should use `#[rustler::nif(schedule = "DirtyCpu")]` to avoid blocking the BEAM scheduler
- Errors returned as `{:ok, value}` / `{:error, reason}` tuples

---

## R (extendr)

- **Framework**: [extendr](https://extendr.github.io/)
- **Crate**: `alef-backend-extendr`
- **Build tool**: `cargo`
- **Crate suffix**: `-extendr`
- **Depends on FFI**: No

### DTO styles

Configured via `[dto] r = "..."`.

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `list` | `"list"` (default) | R named lists. Simple, works with all R code. |
| `R6` | `"r6"` | R6 reference classes for OOP-style APIs with mutable state. |

### Generated output

- `generated_bindings.rs` -- extendr-annotated functions and types

### Key configuration

```toml
[r]
package_name = "mylib"             # R package name
serde_rename_all = "snake_case"    # Field naming (default: snake_case)
```

### Known limitations

- No native async support (`supports_async: false`)
- Async methods use `tokio::runtime::Runtime::new()` + `block_on`
- R is single-threaded; all Rust calls block the R session

---

## Kotlin (Panama FFM)

- **Framework**: [Panama FFM](https://openjdk.org/jeps/454) (shared with the Java backend)
- **Crate**: `alef-backend-kotlin`
- **Build tool**: `gradle`
- **Crate suffix**: shares `-java` (no separate Rust crate emitted)
- **Depends on FFI**: Yes — emits a Kotlin source layer over the Java Panama bindings

### DTO styles

| Style | TOML value | When to use |
|-------|-----------|-------------|
| `data class` | `"data-class"` (default) | Idiomatic Kotlin records with structural equality and copy semantics |
| `sealed class` | `"sealed-class"` | Polymorphic enum-with-data shapes |

### Generated output

- `packages/kotlin/src/main/kotlin/{package}/{Module}.kt` — Kotlin object namespace + bridge calls
- `packages/kotlin/build.gradle.kts` — gradle build file with kotlin-jvm + Panama deps

### Key configuration

```toml
[kotlin]
package = "dev.kreuzberg.kreuzberg"
target  = "jvm"             # or "native" — Kotlin/Native via FFI (planned)
exclude_functions = []
exclude_types     = []
```

### Trait bridges

Trait bridges emit a Kotlin `interface` plus a typealias to the Java FFI-managed bridge handle. Implement the interface in Kotlin and pass instances to the bridge factory function.

---

## Gleam (Rustler NIF + `@external`)

- **Framework**: [Rustler](https://github.com/rusterlium/rustler) NIF, Gleam side via `@external(erlang, …)`
- **Crate**: `alef-backend-gleam`
- **Build tool**: `gleam`
- **Crate suffix**: shares `-rustler` (Gleam emits Gleam source, not a separate crate)
- **Depends on FFI**: No (rides on Rustler/BEAM)

### Generated output

- `packages/gleam/src/{module}.gleam` — record types, `@external` shims, per-trait callback response shims
- `packages/gleam/gleam.toml` — Gleam package manifest

### Key configuration

```toml
[gleam]
app_name   = "kreuzberg"
nif_module = "Elixir.Kreuzberg.Native"
```

### Trait bridges

For each `[[trait_bridges]]`, the backend emits:
* `register_<trait>(pid, plugin_name) -> Nil` — registers the calling Erlang process as the trait implementor.
* Per-method `<trait>_<method>_response(call_id, result)` shims — called from `handle_info/2` when Rust dispatches a callback.

---

## Swift (swift-bridge)

- **Framework**: [swift-bridge](https://github.com/chinedufn/swift-bridge) 0.1.59
- **Crate**: `alef-backend-swift`
- **Build tool**: `cargo` + `swift build`
- **Crate suffix**: `-swift`
- **Depends on FFI**: No (uses swift-bridge's C-compatible glue)

### Generated output

- `packages/swift/rust/` — full Rust crate (`Cargo.toml`, `build.rs`, `src/lib.rs` with `#[swift_bridge::bridge] mod ffi { … }`)
- `packages/swift/Package.swift` — SwiftPM manifest with three targets: `RustBridgeC` (C headers), `RustBridge` (swift-bridge generated Swift), `Kreuzberg` (host wrapper)
- `packages/swift/Sources/Kreuzberg/Kreuzberg.swift` — public Swift types (`Codable` structs and enums) + namespace
- `packages/swift/BUILDING.md` — manual `cargo build` + copy-step instructions because SwiftPM cannot invoke Cargo directly

### Key configuration

```toml
[swift]
module_name        = "Kreuzberg"
exclude_types      = []
exclude_functions  = []
exclude_fields     = []   # "TypeName.field_name" entries skip getter emission
```

### Trait bridges

Each trait emits:
* Opaque `*Box` Rust types in the `extern "Rust"` block (held in Swift as opaque handles)
* Per-method trampolines that dispatch into a host-language callback
* Phantom `fn alef_phantom_vec_<trait>() -> Vec<TraitBox>` to pin swift-bridge's `Vec<FooBox>` C symbols (workaround for swift-bridge 0.1.59)

### Known limitations

- swift-bridge 0.1.59 does not support `#[swift_bridge(async)]`; async functions block on `tokio::runtime::Runtime::new()` at the bridge boundary.
- Functions whose params or returns require lossy bridging (e.g. `Vec<(Vec<u8>, T)>`, opaque enum wrappers, JSON returns of non-serde types) are skipped from the extern surface entirely rather than emitting panicking shims.
- The host wrapper (`Kreuzberg.swift`) currently exposes Codable types; calling RustBridge directly is the recommended path until full type-conversion bridging lands.

---

## Dart (flutter_rust_bridge)

- **Framework**: [flutter_rust_bridge](https://cjycode.com/flutter_rust_bridge/) v2
- **Crate**: `alef-backend-dart`
- **Build tool**: `cargo` + `flutter_rust_bridge_codegen`
- **Crate suffix**: `-dart`
- **Depends on FFI**: No

### Generated output

- `packages/dart/rust/` — Rust crate with `#[frb(mirror)]` struct/enum mirrors and bridge functions
- `packages/dart/pubspec.yaml` — Dart package manifest
- `packages/dart/lib/src/traits.dart` — abstract Dart classes for each `[[trait_bridges]]` entry
- `packages/dart/test/{module}_test.dart` — placeholder so `dart test` succeeds before FRB codegen runs

### Key configuration

```toml
[dart]
pubspec_name = "kreuzberg"
frb_version  = "2"
```

### Trait bridges

Each trait emits:
* `*DartImpl` opaque struct + `impl Trait for *DartImpl` on the Rust side
* Factory function `create_<trait>_dart_impl(...)` accepting `DartFnFuture<T>` callbacks
* Dart-side `abstract class <Trait>` in `lib/src/traits.dart` with `Future<T>`-returning methods

### Known limitations

- `flutter_rust_bridge_codegen generate` must be installed and run after `cargo build -p kreuzberg-dart` — see `packages/dart/BUILDING.md`.
- Tuple-of-bytes containers like `Vec<(Vec<u8>, T)>` cannot survive FRB's JSON round-trip; affected functions are skipped from emission rather than emitting panicking shims.
- Async-fn with non-trivial signatures may be deferred; check the generated `packages/dart/rust/src/lib.rs` for a `// TODO: async function` comment.

---

## Zig (C FFI)

- **Framework**: C FFI via `@cImport` against the alef-emitted C header
- **Crate**: `alef-backend-zig`
- **Build tool**: `zig build`
- **Crate suffix**: shares `-ffi` (Zig wraps the C FFI crate)
- **Depends on FFI**: Yes (consumes the C header generated by `alef-backend-ffi`)

### Generated output

- `packages/zig/src/{module}.zig` — Zig structs, error sets, function shims, comptime vtable helpers
- `packages/zig/build.zig` — build script with C FFI link flags
- `packages/zig/build.zig.zon` — package manifest

### Key configuration

```toml
[zig]
module_name = "kreuzberg"
```

### Trait bridges

For each trait, the backend emits a `pub fn make_<trait>_vtable(comptime T: type, instance: *T) I<Trait>` helper. The helper uses `comptime` reflection to produce `callconv(.C)` thunks from the Zig type's methods, eliminating the per-method boilerplate that hand-written Zig FFI usually requires.

---

## Common Configuration

### Output directories

```toml
[output]
python = "crates/{name}-py/src/"
node = "crates/{name}-node/src/"
ruby = "crates/{name}-rb/src/"
php = "crates/{name}-php/src/"
elixir = "crates/{name}-rustler/src/"
gleam = "packages/gleam/src/"
wasm = "crates/{name}-wasm/src/"
ffi = "crates/{name}-ffi/src/"
go = "packages/go/"
java = "packages/java/src/main/java/"
kotlin = "packages/kotlin/src/main/kotlin/"
csharp = "packages/csharp/src/"
r = "crates/{name}-extendr/src/"
swift = "packages/swift/Sources/"
dart = "packages/dart/lib/"
zig = "packages/zig/src/"
```

### DTO config section

```toml
[dto]
python = "dataclass"
python_output = "typed-dict"   # Optional: different style for return types
node = "interface"
ruby = "struct"
php = "readonly-class"
elixir = "struct"
go = "struct"
java = "record"
csharp = "record"
r = "list"
```

### Serde rename defaults

| Default `camelCase` | Default `snake_case` |
|---------------------|---------------------|
| Node, WASM, Java, Kotlin, C#, Swift, Dart | Python, Ruby, PHP, Go, FFI, Elixir, Gleam, R, Zig |
