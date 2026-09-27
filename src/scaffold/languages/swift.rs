use crate::backends::swift::gen_bindings::bridge_artifacts::umbrella_header;
use crate::backends::swift::naming::swift_source_ident;
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, PrimitiveType, TypeDef, TypeRef};
use crate::scaffold::naming::{swift_min_ios, swift_min_macos};
use crate::scaffold::{readme_language_configured, scaffold_meta};
use anyhow::Context as _;
use heck::ToLowerCamelCase;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) fn scaffold_swift(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let (module, package_name) = (config.swift_module(), config.swift_package_name());
    let min_macos_major = swift_min_macos(config).split('.').next().unwrap_or("13").to_string();
    let min_ios_major = swift_min_ios(config).split('.').next().unwrap_or("16").to_string();

    let crate_name = &config.name;
    let binding_crate_name = format!("{crate_name}-swift");
    let binding_crate_underscore = binding_crate_name.replace('-', "_");

    let ffi_lib_name = config
        .ffi
        .as_ref()
        .and_then(|f| f.lib_name.as_ref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}_ffi", crate_name.replace('-', "_")));

    let swift_capsule: Vec<(String, String, String)> = config
        .swift
        .as_ref()
        .map(|c| {
            let mut deps: Vec<(String, String, String)> = c
                .capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .map(|cap| {
                    let product = crate::core::config::languages::zig_capsule_import_name(&cap.host_type)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| cap.host_type.clone());
                    (cap.package.clone(), cap.package_version.clone(), product)
                })
                .collect();
            deps.sort();
            deps.dedup();
            deps
        })
        .unwrap_or_default();
    let package_dependencies = if swift_capsule.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = swift_capsule
            .iter()
            .map(|(pkg, ver, _product)| {
                let ver_clause = if ver.is_empty() {
                    "branch: \"master\"".to_string()
                } else {
                    format!("from: \"{ver}\"")
                };
                format!("    .package(url: \"{pkg}\", {ver_clause}),")
            })
            .collect();
        format!("\n  dependencies: [\n{}\n  ],", entries.join("\n"))
    };
    let module_target_capsule_deps = if swift_capsule.is_empty() {
        String::new()
    } else {
        let product_names: Vec<String> = swift_capsule
            .iter()
            .map(|(pkg, _ver, product)| {
                let identity = pkg
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(pkg)
                    .trim_end_matches(".git");
                format!(", .product(name: \"{product}\", package: \"{identity}\")")
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        product_names.join("")
    };

    let package_swift = format!(
        r#"// swift-tools-version: 6.0
import PackageDescription
// `resolvedStaticLib` below needs one file-existence check and nothing else. Foundation would
// supply `FileManager`, but importing it makes a standalone Swift toolchain (swiftly 6.3.1,
// targeting macosx28.0 against an older host SDK) fail the manifest outright -- measured:
// Foundation exits 1 with a typecheck error, this POSIX import exits 0, and no import at all is
// fully clean. The residual `-target-arch-variant` warnings under such a toolchain come from the
// SDK skew itself, not from anything alef emits, and SwiftPM tolerates them. `access` is the
// smallest dependency that answers the question (alef issue #461). ~keep
#if canImport(Darwin)
import Darwin
#else
import Glibc
#endif

// NOTE: Run `cargo build -p {binding_crate}` and then rerun `alef generate`
// before `swift build`. Alef materializes the swift-bridge Swift/C outputs into
// Sources/RustBridge and Sources/RustBridgeC when the Cargo build output exists.
// See README.md for the full workflow.

// Absolute path to the Cargo target dir, resolved from this manifest's own location so
// library resolution is independent of the process working directory (`swift test` may
// chdir into fixture dirs). `#filePath` is a compile-time literal, so computing this string
// performs no filesystem access. `omittingEmptySubsequences: false` is load-bearing: `#filePath`
// starts with "/", so dropping empty subsequences would drop that leading empty component and
// `joined` would silently produce a relative path (no leading "/"), defeating the
// process-working-directory independence this comment describes. Deliberately avoids Foundation:
// under a standalone Swift toolchain, that module breaks `swiftc` outright (and any use of one
// of its types, like the former `NSString` coercion here, then fails to typecheck too). ~keep
let rustTargetDir =
  #filePath.split(separator: "/", omittingEmptySubsequences: false).dropLast().joined(separator: "/")
  + "/../../target"

// Resolve the static archive for a Rust crate explicitly, preferring `release` over `debug`.
// `crates/{binding_crate}` and the FFI crate both build `crate-type = ["cdylib", "staticlib"]`,
// so `target/{{release,debug}}` holds both `lib<name>.a` and `lib<name>.dylib`. A bare
// `.linkedLibrary("<name>")` / `-l<name>` lets the linker pick between them, and ld64 prefers
// the `.dylib` when both sit in the same search directory — but that dylib was built with
// `-undefined dynamic_lookup` and does not itself define the swift-bridge glue symbols (e.g.
// `__swift_bridge__$<Type>$_free`), so the link silently succeeds while the resulting binary
// fails to resolve those symbols at dlopen/runtime. Passing the archive's resolved absolute
// path forces static linking unambiguously.
func resolvedStaticLib(_ name: String) -> String {{
  let release = "\(rustTargetDir)/release/lib\(name).a"
  let debug = "\(rustTargetDir)/debug/lib\(name).a"
  return access(release, F_OK) == 0 ? release : debug
}}

let package = Package(
  name: "{package_name}",
  platforms: [
    .macOS(.v{min_macos}),
    .iOS(.v{min_ios}),
  ],
  products: [
    .library(name: "{module}", targets: ["{module}"])
  ],{package_dependencies}
  targets: [
    // RustBridgeC: pure C/headers target. Swift files in RustBridge import this
    // to access C types (RustStr, etc.) produced by swift-bridge.
    // publicHeadersPath: "." exposes RustBridgeC.h to dependents.
    .target(
      name: "RustBridgeC",
      path: "Sources/RustBridgeC",
      publicHeadersPath: "."
    ),
    // RustBridge: Swift wrapper around the Rust static library.
    // Depends on RustBridgeC so the generated Swift files can use the C types.
    // linkerSettings wire the Rust staticlibs (lib{binding_underscore}.a and lib{ffi_lib_name}.a)
    // produced by `cargo build -p {binding_crate}` and the FFI crate so
    // `swift build` / `swift test` can resolve the `__swift_bridge__$*` and FFI C symbols.
    // Explicit absolute paths (see `resolvedStaticLib` above) are used instead of
    // `.linkedLibrary(...)` so the linker cannot substitute the sibling `.dylib` artifacts.
    // The FFI library is needed because the generated Swift service API code (App.swift)
    // calls FFI functions directly via @_silgen_name declarations.
    .target(
      name: "RustBridge",
      dependencies: ["RustBridgeC"],
      path: "Sources/RustBridge",
      linkerSettings: [
        .unsafeFlags([
          resolvedStaticLib("{binding_underscore}"),
          resolvedStaticLib("{ffi_lib_name}"),
        ]),
        // The Rust staticlib records native-library dependencies (e.g. `lzma-sys`
        // via the archive/`xz2` path emits `cargo:rustc-link-lib`) that cargo would
        // resolve when it drives the final link, but a `staticlib` `.a` does not
        // embed them and SwiftPM does not read cargo's link metadata, so undefined
        // symbols like `_lzma_stream_decoder` surface at the swift link step. Link
        // the system library here. `liblzma` ships in the macOS SDK and on Linux.
        .linkedLibrary("lzma"),
        // Same staticlib-doesn't-embed-native-deps reasoning as lzma above: the
        // bzip2 crates (archive/zip/unhwp paths) emit `-lbz2`, surfacing undefined
        // `_BZ2_bzDecompress*` at the swift link step. `libbz2` ships in the macOS
        // SDK and on Linux.
        .linkedLibrary("bz2"),
        // The Rust staticlib pulls in C++ dependencies (onnxruntime, tesseract,
        // ClipperLib) that reference the C++ runtime/ABI (`__cxa_throw`,
        // `__gxx_personality_v0`, `__cxa_guard_acquire`, ...). A `staticlib` `.a`
        // does not carry the transitive `-lc++`/`-lstdc++` system-lib dependency,
        // so SwiftPM must link the C++ standard library explicitly or the final
        // link fails with undefined symbols from those crates.
        .linkedLibrary("c++", .when(platforms: [.macOS, .iOS])),
        .linkedLibrary("stdc++", .when(platforms: [.linux])),
        .linkedFramework("Security", .when(platforms: [.macOS, .iOS])),
        .linkedFramework("CoreFoundation", .when(platforms: [.macOS, .iOS])),
        .linkedFramework("SystemConfiguration", .when(platforms: [.macOS])),
      ]
    ),
    .target(
      name: "{module}", dependencies: ["RustBridge"{module_target_capsule_deps}],
      path: "Sources/{module}",
      exclude: ["LICENSE"]),
    .testTarget(
      name: "{module}Tests", dependencies: ["{module}"],
      path: "Tests/{module}Tests"),
  ]
)
"#,
        module = module,
        min_macos = min_macos_major,
        min_ios = min_ios_major,
        binding_crate = binding_crate_name,
        binding_underscore = binding_crate_underscore,
        ffi_lib_name = ffi_lib_name,
        package_dependencies = package_dependencies,
        module_target_capsule_deps = module_target_capsule_deps,
    );

    let gitignore = ".build/\nPackages/\nxcuserdata/\nDerivedData/\n.swiftpm/\n*.xcodeproj\n";

    let test_stub = scaffold_swift_test(api, config, &module);

    let rust_bridge_c_header = build_rust_bridge_c_header(&binding_crate_name)
        .with_context(|| format!("building {RUST_BRIDGE_C_HEADER_PATH} for `{binding_crate_name}`"))?;
    let rust_bridge_c_source = build_rust_bridge_c_source(&binding_crate_underscore);

    let rust_bridge_swift = format!(
        r#"// Placeholder Swift source for the RustBridge target.
// Run `cargo build -p {binding_crate}` and then rerun `alef generate` to replace
// this file with swift-bridge output. See README.md for instructions.
//
// This file is intentionally minimal so SwiftPM accepts the target before
// the cargo build step has been run.
public enum RustBridgePlaceholder {{}}
"#,
        binding_crate = binding_crate_name,
    );

    let module_modulemap = "// This modulemap is unused — the RustBridgeC target provides the C types.\n// SwiftPM discovers RustBridgeC.h via the publicHeadersPath setting.\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.swift]\nindent_style = space\nindent_size = 2\n";

    let swiftformat = "lineLength = 120\nindent = 2\nusesTabs = false\n";
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();

    // `.package(path: "packages/swift")` is a local filesystem path: it only
    // resolves inside a checkout of this monorepo and is unusable by any
    // consumer of a published package. This placeholder README only ships when
    // the language has no `[crates.readme.languages.swift]` config yet (see the
    // `readme_language_configured` guard below), but even then it must document
    // an installable reference, not a repo-relative path. ~keep
    let repository = config.github_repo();
    let version = &api.version;
    let readme = format!(
        r#"# {module}

{description}

## Installation

Add to your `Package.swift`:

```swift
.package(url: "{repository}", from: "{version}"),
```

## Building

```sh
cargo build -p {binding_crate}
alef generate --lang swift
swift build --package-path packages/swift
swift test --package-path packages/swift
```

Before the Cargo build output exists, Alef emits placeholder RustBridge files so
the generated package layout is complete. After Cargo produces swift-bridge
artifacts, rerunning Alef replaces the placeholders with the generated Swift and
C bridge sources.
"#,
        module = module,
        description = meta.description,
        binding_crate = binding_crate_name,
    ) + &license_section;

    let demo_swift = format!(
        r#"import {module}

@main
struct Demo {{
  static func main() {{
    print("Demo: {module} loaded successfully")
    // Add your API calls here after code generation
  }}
}}
"#,
        module = module,
    );

    let root_package_swift = meta.repository.as_deref().map(|repository| {
        format!(
            r#"// swift-tools-version: 6.0
// Root-level Package.swift — alef-generated for published distributions.
//
// This manifest uses `.binaryTarget` for pre-built XCFramework/artifact bundles.
// External consumers depend on this via `.package(url: "...", from: "...")`.
//
// For in-tree development, see `packages/swift/Package.swift` and
// `packages/swift/README.md` for the source-based workflow.
import PackageDescription

let package = Package(
  name: "{package_name}",
  platforms: [
    .macOS(.v{min_macos}),
    .iOS(.v{min_ios}),
  ],
  products: [
    .library(name: "{module}", targets: ["{module}"])
  ],{package_dependencies}
  targets: [
    // RustBridgeC: C headers target. Swift files in RustBridge import this to
    // access C types (RustStr, etc.) produced by swift-bridge.
    // publicHeadersPath: "." exposes the headers.
    .target(
      name: "RustBridgeC",
      path: "packages/swift/Sources/RustBridgeC",
      publicHeadersPath: "."
    ),
    // RustBridgeBinary: pre-built static library for macOS (arm64, x86_64),
    // iOS (device, simulator), and Linux (arm64, x86_64). The artifactbundle
    // ships `.a` files only — SwiftPM binary targets cannot supply Swift
    // modules, so the swift-bridge generated Swift sources live in the
    // sibling RustBridge target below and link against this binary.
    .binaryTarget(
      name: "RustBridgeBinary",
      url: "{repository}/releases/download/v__ALEF_SWIFT_VERSION__/{module}-rs.artifactbundle.zip",
      checksum: "__ALEF_SWIFT_CHECKSUM__"
    ),
    // RustBridge: Swift wrapper module owning the swift-bridge generated
    // sources. Depends on RustBridgeC for C type declarations and on
    // RustBridgeBinary so the linker picks up the static library symbols.
    .target(
      name: "RustBridge",
      dependencies: ["RustBridgeC", "RustBridgeBinary"],
      path: "packages/swift/Sources/RustBridge",
      // The pre-built static library inside RustBridgeBinary references Apple
      // system frameworks (e.g. reqwest's proxy detection pulls in the Rust
      // `system_configuration` crate → `SC*` symbols) and native system
      // libraries (e.g. the archive/`xz2` path pulls in `lzma-sys` →
      // `_lzma_stream_decoder`). The artifactbundle ships only the `.a`, so these
      // must be linked by the consumer. `liblzma` ships in the macOS SDK and on
      // Linux.
      linkerSettings: [
        .linkedLibrary("lzma"),
        // Same reasoning as lzma: the bzip2 crates (archive/zip/unhwp) emit
        // `-lbz2`, surfacing undefined `_BZ2_bzDecompress*`. `libbz2` ships in
        // the macOS SDK and on Linux.
        .linkedLibrary("bz2"),
        // The pre-built static library pulls in C++ dependencies (onnxruntime,
        // tesseract, ClipperLib) that reference the C++ runtime/ABI
        // (`__cxa_throw`, `__gxx_personality_v0`, `__cxa_guard_acquire`, ...). A
        // `.a` archive does not carry the transitive `-lc++`/`-lstdc++`
        // system-lib dependency, so the consumer must link the C++ standard
        // library explicitly or the final link fails with undefined symbols
        // from those crates.
        .linkedLibrary("c++", .when(platforms: [.macOS, .iOS])),
        .linkedLibrary("stdc++", .when(platforms: [.linux])),
        .linkedFramework("Security", .when(platforms: [.macOS, .iOS])),
        .linkedFramework("CoreFoundation", .when(platforms: [.macOS, .iOS])),
        .linkedFramework("SystemConfiguration", .when(platforms: [.macOS])),
      ]
    ),
    .target(
      name: "{module}",
      dependencies: ["RustBridge", "RustBridgeC"{module_target_capsule_deps}],
      path: "packages/swift/Sources/{module}"
    ),
  ]
)
"#,
            module = module,
            min_macos = min_macos_major,
            min_ios = min_ios_major,
            repository = repository.trim_end_matches('/'),
            package_dependencies = package_dependencies,
            module_target_capsule_deps = module_target_capsule_deps,
        )
    });

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from("packages/swift/Package.swift"),
            content: package_swift,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/swift/Tests/{module}Tests/{module}Tests.swift")),
            content: test_stub,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/Sources/RustBridgeC/RustBridgeC.h"),
            content: rust_bridge_c_header,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/Sources/RustBridgeC/RustBridgeC.c"),
            content: rust_bridge_c_source,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/Sources/RustBridge/module.modulemap"),
            content: module_modulemap.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/Sources/RustBridge/RustBridge.swift"),
            content: rust_bridge_swift,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/.swiftformat"),
            content: swiftformat.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/Examples/Demo/main.swift"),
            content: demo_swift,
            generated_header: false,
        },
    ];
    // The README module (`crate::readme`) owns `packages/swift/README.md` end-to-end
    // once `[crates.readme.languages.swift]` is configured — badges, "What This
    // Package Provides", Quick Start, feature/OCR sections, snippets. Emitting this
    // placeholder alongside that config makes scaffold a second, independent writer
    // for the same path: a run that only scaffolds (`alef scaffold`, a
    // `--lang`-scoped pass, or one that errors before the README stage) would ship
    // this skeleton note as the final content with the configured sections silently
    // dropped and no error (#555). Inserted at its original position (before the
    // Demo example) rather than appended, so file order is unchanged for languages
    // that still rely on this placeholder. ~keep
    if !readme_language_configured(config, "swift") {
        files.insert(
            9,
            GeneratedFile {
                path: PathBuf::from("packages/swift/README.md"),
                content: readme,
                generated_header: false,
            },
        );
    }
    if let Some(root_package_swift) = root_package_swift {
        files.insert(
            0,
            GeneratedFile {
                path: PathBuf::from("Package.swift"),
                content: root_package_swift,
                generated_header: false,
            },
        );
    }
    Ok(files)
}

/// Build the seed content for `Tests/{module}Tests/{module}Tests.swift`.
///
/// `write_scaffold_files_report` treats `generated_header: false` as create-only, so once
/// a real suite exists at this path alef never overwrites it; this only ever seeds a fresh
/// project. The seed must not be vacuous -- `XCTAssertTrue(true)` compiles and passes no
/// matter what the generated API looks like, which is exactly the "0 assertions, silently
/// green" defect the zig test-module fix (`scaffold_zig_test`) already closed one layer
/// down. So this asserts against the *real*, currently-generated API surface (`api`), in
/// order of how strong a check is safely synthesizable without duplicating the swift
/// binding emitter's full type-mapping surface:
///
/// 1. A visible, non-opaque DTO (`has_serde`, struct, all fields plain primitives/`String`,
///    no optional/cfg-gated fields) is round-tripped through `JSONEncoder`/`JSONDecoder` and
///    compared for equality -- the strongest check: it fails on a broken `Codable`
///    conformance, a renamed/dropped field, or a removed type, not just a missing symbol.
/// 2. Otherwise, any other visible type or enum is referenced by `.self` so the compiler
///    must resolve it -- weaker than a round trip (construction/field shape isn't checked)
///    but still a real, falsifiable fact about the generated output.
/// 3. Only when the API surface is genuinely empty (e.g. scaffolding before any Rust code
///    exists) does this fall back to asserting the module resolves at all under
///    `@testable import` -- there is nothing else to assert against yet, and once real
///    items exist this file is never regenerated over. ~keep
fn scaffold_swift_test(api: &ApiSurface, config: &ResolvedCrateConfig, module: &str) -> String {
    let (exclude_types, exclude_fields) = swift_binding_exclusions(config);

    let round_trip_candidate = api
        .types
        .iter()
        .filter(|t| swift_type_is_visible(t, &exclude_types))
        .filter(|t| !t.is_opaque && t.has_serde && !t.has_stripped_cfg_fields)
        .find_map(|t| simple_codable_fields(t, &exclude_fields).map(|fields| (t, fields)));
    if let Some((ty, fields)) = round_trip_candidate {
        return codable_round_trip_test(module, &ty.name, &fields);
    }

    let visible_type_name = api
        .types
        .iter()
        .filter(|t| swift_type_is_visible(t, &exclude_types))
        .map(|t| t.name.clone())
        .next();
    let visible_enum_name = || {
        api.enums
            .iter()
            .filter(|e| !e.binding_excluded && !exclude_types.contains(&e.name))
            .map(|e| e.name.clone())
            .next()
    };
    if let Some(name) = visible_type_name.or_else(visible_enum_name) {
        return type_reference_test(module, &name);
    }

    placeholder_test(module)
}

/// Repair a pre-existing `Tests/{module}Tests/{module}Tests.swift` that is still the vacuous
/// `XCTAssertTrue(true)` placeholder this scaffold used to emit, now that
/// [`scaffold_swift_test`] can generate a real assertion against the actual API surface.
///
/// Mirrors `migrate_build_zig_test_target` (`src/scaffold/languages/zig.rs`) and
/// [`crate::scaffold::migrate_dart_placeholder_test`] in strategy and for the same reason:
/// `Tests/*Tests.swift` is `generated_header: false` (create-only) on a markable `.swift`
/// extension, so `write_scaffold_files_report`'s ownership guard permanently refuses to
/// overwrite it once it exists. A generator fix to its *content* therefore can never reach an
/// existing repo through the normal write path at all, whatever `overwrite` says — every repo
/// scaffolded before the tiered seed landed would sit on a vacuous placeholder forever.
///
/// Detection is a *vacuity signature*, not a byte-for-byte template match, because the
/// placeholder's exact bytes have already drifted across the repos this exists to fix: the
/// current generator ([`placeholder_test`]) names the method `testModuleImportsSuccessfully`
/// and carries a three-line rationale comment, while the shape actually found on disk
/// (consumer A's `packages/swift/Tests/<Module>Tests/<Module>Tests.swift`)
/// names it `testPlaceholder` and carries a two-line "Placeholder test so `swift test` has a
/// target to run" comment instead. A constant validated only against the current generator's
/// own output matches neither historical shape it exists to repair.
///
/// So this fires only when the file's *sole* assertion is the tautology: it contains
/// `XCTAssertTrue(true)` (or the bare `XCTAssert(true)` spelling of the same tautology), and
/// contains **exactly one** `XCTAssert`-family call and **exactly one** `func test` in the
/// whole file — i.e. there is nothing in it to lose. See [`is_vacuous_swift_placeholder`] for
/// why neither count can be fooled by a longer identifier. Verified against all three real
/// consumer trees: consumer A (1 `XCTAssert`, 1 `func test`, tautology — fires);
/// consumer B (also 1 and 1, but its single assertion is a real
/// `XCTAssertEqual(decoded, message)` round trip — does not fire, which is exactly why the
/// counts alone are not the signature); consumer C (17 and 17, hand-written —
/// does not fire on either clause). Idempotent: the freshly generated replacement never
/// matches this signature once real API surface exists, and when the surface is still empty
/// (replacement is itself the placeholder) the byte-equality check below makes the second
/// pass a no-op. ~keep
pub(crate) fn migrate_swift_placeholder_test(
    base_dir: &Path,
    relative_path: &Path,
    replacement: &str,
) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if !is_vacuous_swift_placeholder(&existing) {
        return Ok(false);
    }
    if existing == replacement {
        return Ok(false);
    }

    let parent = path.parent().context("swift test path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, replacement.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing Tests/*Tests.swift: replaced the vacuous XCTAssertTrue(true) \
         placeholder with a real assertion against the generated API"
    );
    Ok(true)
}

/// The vacuity signature behind [`migrate_swift_placeholder_test`]: true when `content`'s only
/// assertion is the `XCTAssertTrue(true)` tautology, regardless of which placeholder-template
/// revision emitted it (see that function's doc for why a byte-match against the current
/// template does not survive the drift already present in the real repos this targets).
///
/// Neither count can be fooled by a longer identifier, and the two are counted differently on
/// purpose:
///
/// - `XCTAssert` is counted as a bare *prefix*, with no `(`, because the whole assertion family
///   shares it (`XCTAssertEqual`, `XCTAssertNil`, `XCTAssertThrowsError`, ...). Counting
///   `XCTAssert(` instead — the shape dart's `expect(` needs — would be the inverse of dart's
///   `expectLater(` bug: the 17 real assertions in consumer C of
///   [`migrate_swift_placeholder_test`]'s table would count as 0. Every
///   identifier that *extends* the prefix is itself an assertion, so counting it is correct, and
///   each extra match pushes the count past 1 and makes this return `false` — the safe
///   direction. `XCTUnwrap` does not contain the prefix and is deliberately not counted; a file
///   using it also carries at least one real `XCTAssert*` call in every real tree checked.
/// - `func test` likewise over-matches rather than under-matches (`func testing` counts), and
///   over-matching can only suppress the migration. It cannot under-match a runnable XCTest
///   method, since XCTest only discovers methods whose name begins with `test`. A
///   swift-testing (`@Test func`) suite is not matched at all, but such a file uses `#expect`
///   rather than `XCTAssert*` and so already fails the tautology clause.
///
/// The one accepted blind spot, also in the safe direction: a placeholder whose *comment* text
/// happens to mention `XCTAssert` counts twice and is left alone. Neither the current template
/// nor consumer A's on-disk shape does that.
fn is_vacuous_swift_placeholder(content: &str) -> bool {
    (content.contains("XCTAssertTrue(true)") || content.contains("XCTAssert(true)"))
        && content.matches("XCTAssert").count() == 1
        && content.matches("func test").count() == 1
}

/// Names excluded from Swift binding generation, mirroring the union
/// `[crates.swift] exclude_types`/`exclude_fields` plus `[crates.ffi] exclude_types` that
/// the real swift binding emitter honors. Kept in sync deliberately rather than shared,
/// since this seed-picker only needs *a* safe, visible name/field shape, not the emitter's
/// exhaustive filtered set.
fn swift_binding_exclusions(config: &ResolvedCrateConfig) -> (HashSet<String>, HashSet<String>) {
    let mut exclude_types: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    let exclude_fields: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_fields.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        exclude_types.extend(ffi.exclude_types.iter().cloned());
    }
    (exclude_types, exclude_fields)
}

/// A visible (non-trait, non-`binding_excluded`, not config-excluded) candidate type for the
/// scaffold seed to reference.
fn swift_type_is_visible(ty: &TypeDef, exclude_types: &HashSet<String>) -> bool {
    !ty.is_trait && !ty.binding_excluded && !exclude_types.contains(&ty.name)
}

/// A field simple enough to synthesize a literal Swift value for: a primitive or `String`,
/// never optional, never `#[cfg(...)]`-gated (whether it exists depends on active features,
/// which this scaffold-time seed cannot know).
struct SimpleCodableField {
    swift_label: String,
    literal: String,
}

/// Compute a literal-constructible field list for `ty`, or `None` when any visible field
/// falls outside the safely synthesizable subset (optional, cfg-gated, or a type other than
/// a primitive/`String` -- `Named`/`Vec`/`Map`/etc. would need recursive construction this
/// seed does not attempt). Bails on the *whole type* rather than partially constructing it,
/// since the real generated initializer requires every non-optional stored property.
fn simple_codable_fields(ty: &TypeDef, exclude_fields: &HashSet<String>) -> Option<Vec<SimpleCodableField>> {
    let mut fields = Vec::new();
    for field in binding_fields(&ty.fields) {
        if field.optional || field.cfg.is_some() {
            return None;
        }
        if exclude_fields.contains(&format!("{}.{}", ty.name, field.name)) {
            return None;
        }
        let literal = match &field.ty {
            TypeRef::Primitive(primitive) => primitive_literal(primitive),
            TypeRef::String => "\"alef-scaffold\"".to_string(),
            _ => return None,
        };
        fields.push(SimpleCodableField {
            swift_label: swift_source_ident(&field.name.to_lower_camel_case()),
            literal,
        });
    }
    if fields.is_empty() { None } else { Some(fields) }
}

/// A literal Swift value for a primitive type. Bool gets a non-default `true` and floats a
/// non-integral `1.5` so a decoder that silently returns a zero value rather than the
/// decoded one is still caught by the round-trip equality check.
fn primitive_literal(primitive: &PrimitiveType) -> String {
    match primitive {
        PrimitiveType::Bool => "true".to_string(),
        PrimitiveType::F32 | PrimitiveType::F64 => "1.5".to_string(),
        _ => "1".to_string(),
    }
}

/// The strongest safe check: round-trip a visible, literal-constructible DTO through
/// `JSONEncoder`/`JSONDecoder` and assert the decoded value equals the original, so a broken
/// `Codable` conformance or a field that silently stops encoding fails `swift test`
/// immediately instead of shipping green with a suite that asserts nothing.
fn codable_round_trip_test(module: &str, type_name: &str, fields: &[SimpleCodableField]) -> String {
    let init_args = fields
        .iter()
        .map(|f| format!("{}: {}", f.swift_label, f.literal))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "import XCTest\n\n\
         @testable import {module}\n\n\
         final class {module}Tests: XCTestCase {{\n  \
             /// Round-trips the generated `{type_name}` DTO through `JSONEncoder`/`JSONDecoder`,\n  \
             /// so a broken `Codable` conformance or a field that silently stops encoding fails\n  \
             /// `swift test` immediately instead of shipping green with a suite that asserts\n  \
             /// nothing about the generated API. Create-only scaffold seed. ~keep\n  \
             func test{type_name}RoundTripsThroughJSON() throws {{\n    \
                 let original = {type_name}({init_args})\n    \
                 let data = try JSONEncoder().encode(original)\n    \
                 let decoded = try JSONDecoder().decode({type_name}.self, from: data)\n    \
                 XCTAssertEqual(decoded, original)\n  \
             }}\n\
         }}\n",
    )
}

/// `name` isn't a literal-constructible Codable DTO this seed can safely round-trip
/// generically, so this checks the generated type exists and is referenceable at compile
/// time instead.
fn type_reference_test(module: &str, name: &str) -> String {
    format!(
        "import XCTest\n\n\
         @testable import {module}\n\n\
         final class {module}Tests: XCTestCase {{\n  \
             /// `{name}` isn't a literal-constructible Codable DTO this seed can safely\n  \
             /// round-trip generically, so this checks the generated type exists and is\n  \
             /// referenceable at compile time instead. Create-only scaffold seed. ~keep\n  \
             func test{name}Exists() {{\n    \
                 XCTAssertNotNil({name}.self)\n  \
             }}\n\
         }}\n",
    )
}

/// No generated API surface exists yet for this crate, so there is nothing to assert
/// against beyond the module resolving under `@testable import`. Once real types exist,
/// alef never regenerates over this file -- it is a create-only scaffold seed.
fn placeholder_test(module: &str) -> String {
    format!(
        "import XCTest\n\n\
         @testable import {module}\n\n\
         final class {module}Tests: XCTestCase {{\n  \
             /// No generated API surface exists yet for this crate, so there is nothing to\n  \
             /// assert against beyond the module resolving. Once real types exist, alef never\n  \
             /// regenerates over this file -- it is a create-only scaffold seed. ~keep\n  \
             func testModuleImportsSuccessfully() throws {{\n    \
                 XCTAssertTrue(true)\n  \
             }}\n\
         }}\n",
    )
}

/// Path `scaffold_swift` writes `RustBridgeC.h` to, relative to the generation
/// root. Alef commands run with the workspace root as the cwd, matching the
/// cwd-relative lookup in [`read_swift_bridge_headers`].
const RUST_BRIDGE_C_HEADER_PATH: &str = "packages/swift/Sources/RustBridgeC/RustBridgeC.h";

/// Build the content for `Sources/RustBridgeC/RustBridgeC.h`.
///
/// When `cargo build -p {binding_crate}` has already been run, returns a thin umbrella
/// header that concatenates `SwiftBridgeCore.h` and `{binding_crate}.h` from the
/// swift-bridge build output. Otherwise an already-populated header committed on disk
/// is preserved, and only when neither is available is a placeholder emitted.
fn build_rust_bridge_c_header(binding_crate_name: &str) -> anyhow::Result<String> {
    let fresh_headers = read_swift_bridge_headers(binding_crate_name);
    let existing_header = std::fs::read_to_string(RUST_BRIDGE_C_HEADER_PATH).ok();
    render_rust_bridge_c_header(binding_crate_name, fresh_headers, existing_header.as_deref())
}

/// Decide the content of `RustBridgeC.h`, given the optional fresh swift-bridge
/// build output and the optional header already present on disk.
///
/// Precedence:
/// 1. Fresh swift-bridge output (the binding crate was built) → hand it to
///    [`umbrella_header::resolve_fresh`], which assembles the umbrella, refuses a
///    partial assembly, and keeps the committed bytes when the assembly declares
///    the same C. Tier 1 is *not* an unconditional overwrite: two present input
///    files are not on their own evidence of a usable header.
/// 2. No fresh output, but an already-populated header is committed on disk →
///    preserve it. `alef all --clean` regenerates without compiling the binding
///    crate, so without this guard scaffold would overwrite the real
///    `__swift_bridge__$*` declarations with the placeholder and break every
///    SwiftPM consumer of the published source package. Mirrors the guard in
///    `backends::swift::gen_bindings::bridge_artifacts::emit_swift_bridge_files`.
/// 3. Otherwise → emit the placeholder so SwiftPM accepts the target before the
///    first build.
fn render_rust_bridge_c_header(
    binding_crate_name: &str,
    fresh_headers: Option<(String, String)>,
    existing_header: Option<&str>,
) -> anyhow::Result<String> {
    if let Some((core_h, crate_h)) = fresh_headers {
        return umbrella_header::resolve_fresh(binding_crate_name, &core_h, &crate_h, existing_header);
    }

    if let Some(existing) = existing_header
        && umbrella_header::is_populated(existing)
    {
        return Ok(existing.to_string());
    }

    Ok(format!(
        "#ifndef RUST_BRIDGE_C_H\n\
         #define RUST_BRIDGE_C_H\n\
         \n\
         // Placeholder header for the RustBridgeC SwiftPM target.\n\
         // Run `cargo build -p {binding_crate_name}` and re-run `alef all` to populate.\n\
         // The typedefs below are the minimum required for SwiftBridgeCore.swift\n\
         // to compile before the full cargo build has been run.\n\
         \n\
         #include <stdbool.h>\n\
         #include <stdint.h>\n\
         \n\
         typedef struct RustStr {{\n  \
         uint8_t *const start;\n  \
         uintptr_t len;\n\
         }} RustStr;\n\
         typedef struct __private__FfiSlice {{\n  \
         void *const start;\n  \
         uintptr_t len;\n\
         }} __private__FfiSlice;\n\
         typedef struct __private__OptionU8 {{\n  \
         uint8_t val;\n  \
         bool is_some;\n\
         }} __private__OptionU8;\n\
         typedef struct __private__OptionI8 {{\n  \
         int8_t val;\n  \
         bool is_some;\n\
         }} __private__OptionI8;\n\
         typedef struct __private__OptionU16 {{\n  \
         uint16_t val;\n  \
         bool is_some;\n\
         }} __private__OptionU16;\n\
         typedef struct __private__OptionI16 {{\n  \
         int16_t val;\n  \
         bool is_some;\n\
         }} __private__OptionI16;\n\
         typedef struct __private__OptionU32 {{\n  \
         uint32_t val;\n  \
         bool is_some;\n\
         }} __private__OptionU32;\n\
         typedef struct __private__OptionI32 {{\n  \
         int32_t val;\n  \
         bool is_some;\n\
         }} __private__OptionI32;\n\
         typedef struct __private__OptionU64 {{\n  \
         uint64_t val;\n  \
         bool is_some;\n\
         }} __private__OptionU64;\n\
         typedef struct __private__OptionI64 {{\n  \
         int64_t val;\n  \
         bool is_some;\n\
         }} __private__OptionI64;\n\
         typedef struct __private__OptionUsize {{\n  \
         uintptr_t val;\n  \
         bool is_some;\n\
         }} __private__OptionUsize;\n\
         typedef struct __private__OptionIsize {{\n  \
         intptr_t val;\n  \
         bool is_some;\n\
         }} __private__OptionIsize;\n\
         typedef struct __private__OptionF32 {{\n  \
         float val;\n  \
         bool is_some;\n\
         }} __private__OptionF32;\n\
         typedef struct __private__OptionF64 {{\n  \
         double val;\n  \
         bool is_some;\n\
         }} __private__OptionF64;\n\
         typedef struct __private__OptionBool {{\n  \
         bool val;\n  \
         bool is_some;\n\
         }} __private__OptionBool;\n\
         \n\
         #endif /* RUST_BRIDGE_C_H */\n"
    ))
}

/// Build the content for `Sources/RustBridgeC/RustBridgeC.c`.
///
/// `RustBridgeC` is otherwise a headers-only C target: `swift build` tolerates that, but
/// Xcode's XCBuild expects a `RustBridgeC.o` to link and fails without one. This trivial
/// translation unit gives the target an object file. `binding_crate_underscore` (already a
/// valid C identifier — hyphens replaced with underscores) namespaces the anchor symbol so
/// it cannot collide with another RustBridgeC-named C target linked into the same binary.
fn build_rust_bridge_c_source(binding_crate_underscore: &str) -> String {
    format!(
        "#include \"RustBridgeC.h\"\n\
         \n\
         // ~keep anchor TU so XCBuild emits RustBridgeC.o (issue #449)\n\
         void {binding_crate_underscore}_rust_bridge_c_anchor(void) {{}}\n"
    )
}

/// Try to locate and read the swift-bridge-generated C headers for the given binding
/// crate. Returns `(SwiftBridgeCore.h content, {crate}.h content)` when found.
fn read_swift_bridge_headers(binding_crate_name: &str) -> Option<(String, String)> {
    let cwd = std::env::current_dir().ok()?;
    let workspace_root = std::iter::once(cwd.clone())
        .chain(cwd.ancestors().skip(1).map(|p| p.to_path_buf()))
        .take(8)
        .find(|p| p.join("Cargo.lock").exists())?;
    let target = workspace_root.join("target");

    let crate_prefix = format!("{}-", binding_crate_name);
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;

    for profile in ["release", "debug"] {
        let build_dir = target.join(profile).join("build");
        let entries = match std::fs::read_dir(&build_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with(&crate_prefix) {
                continue;
            }
            let out = entry.path().join("out");
            let core_h = out.join("SwiftBridgeCore.h");
            let crate_h = out.join(binding_crate_name).join(format!("{binding_crate_name}.h"));
            if !core_h.exists() || !crate_h.exists() {
                continue;
            }
            let mtime = std::fs::metadata(&core_h)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                best = Some((mtime, out));
            }
        }
    }

    let out = best?.1;
    let core_h = std::fs::read_to_string(out.join("SwiftBridgeCore.h")).ok()?;
    let crate_h = std::fs::read_to_string(out.join(binding_crate_name).join(format!("{binding_crate_name}.h"))).ok()?;
    Some((core_h, crate_h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::ApiSurface;

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a GeneratedFile {
        files
            .iter()
            .find(|f| f.path == std::path::Path::new(path))
            .unwrap_or_else(|| panic!("missing scaffolded file: {path}"))
    }

    fn simple_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            has_serde: true,
            fields: vec![
                crate::core::ir::FieldDef {
                    name: "count".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                },
                crate::core::ir::FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// A visible DTO whose fields are all plain primitives/`String` is round-tripped
    /// through `JSONEncoder`/`JSONDecoder` and compared for equality -- the strongest
    /// safe check, since it fails on a broken `Codable` conformance or a dropped field,
    /// not just a missing symbol.
    #[test]
    fn scaffold_test_round_trips_a_simple_dto() {
        let api = ApiSurface {
            types: vec![simple_type("Widget")],
            ..Default::default()
        };
        let out = scaffold_swift_test(&api, &minimal_config(), "MyLib");

        assert!(out.contains("@testable import MyLib"), "got:\n{out}");
        assert!(
            out.contains("func testWidgetRoundTripsThroughJSON() throws {"),
            "got:\n{out}"
        );
        assert!(
            out.contains("let original = Widget(count: 1, label: \"alef-scaffold\")"),
            "got:\n{out}"
        );
        assert!(out.contains("try JSONEncoder().encode(original)"), "got:\n{out}");
        assert!(
            out.contains("try JSONDecoder().decode(Widget.self, from: data)"),
            "got:\n{out}"
        );
        assert!(out.contains("XCTAssertEqual(decoded, original)"), "got:\n{out}");
        assert!(
            !out.contains("XCTAssertTrue(true)"),
            "must not be the vacuous placeholder, got:\n{out}"
        );
    }

    /// An opaque type has no first-class Codable representation, so it can't be
    /// literal-constructed; the seed falls back to a compile-time existence check on the
    /// type name instead of skipping straight to the vacuous placeholder.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_an_opaque_type() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Client".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_swift_test(&api, &minimal_config(), "MyLib");

        assert!(out.contains("func testClientExists() {"), "got:\n{out}");
        assert!(out.contains("XCTAssertNotNil(Client.self)"), "got:\n{out}");
        assert!(!out.contains("JSONEncoder"), "got:\n{out}");
    }

    /// A DTO with an unsupported field shape (e.g. `Optional<T>`) can't be literal
    /// -constructed safely by this seed either -- it also falls back to the existence
    /// check rather than emitting a construction call with a guessed value.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_unsupported_field_shape() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Config".to_string(),
                has_serde: true,
                fields: vec![crate::core::ir::FieldDef {
                    name: "nickname".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_swift_test(&api, &minimal_config(), "MyLib");

        assert!(out.contains("func testConfigExists() {"), "got:\n{out}");
        assert!(out.contains("XCTAssertNotNil(Config.self)"), "got:\n{out}");
    }

    /// With no visible struct at all, a visible enum is checked for existence instead.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_an_enum_when_no_types_exist() {
        let api = ApiSurface {
            enums: vec![crate::core::ir::EnumDef {
                name: "Color".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_swift_test(&api, &minimal_config(), "MyLib");

        assert!(out.contains("func testColorExists() {"), "got:\n{out}");
        assert!(out.contains("XCTAssertNotNil(Color.self)"), "got:\n{out}");
    }

    /// `binding_excluded` types were never emitted into the generated Swift module, so the
    /// seed must skip them rather than asserting against a type that doesn't exist.
    #[test]
    fn scaffold_test_skips_binding_excluded_types() {
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    name: "Hidden".to_string(),
                    is_opaque: true,
                    binding_excluded: true,
                    ..Default::default()
                },
                TypeDef {
                    name: "Visible".to_string(),
                    is_opaque: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let out = scaffold_swift_test(&api, &minimal_config(), "MyLib");

        assert!(out.contains("Visible"), "got:\n{out}");
        assert!(!out.contains("Hidden"), "got:\n{out}");
    }

    /// A genuinely empty API surface (no Rust code written yet) has nothing to assert
    /// against beyond the module resolving -- the only honest seed content, and the
    /// only case where the placeholder assertion is legitimate.
    #[test]
    fn scaffold_test_falls_back_to_placeholder_when_api_surface_is_empty() {
        let out = scaffold_swift_test(&ApiSurface::default(), &minimal_config(), "MyLib");

        assert!(out.contains("@testable import MyLib"), "got:\n{out}");
        assert!(
            out.contains("func testModuleImportsSuccessfully() throws {"),
            "got:\n{out}"
        );
        assert!(out.contains("XCTAssertTrue(true)"), "got:\n{out}");
    }

    /// End-to-end through `scaffold_swift`: the emitted test file at
    /// `Tests/{module}Tests/{module}Tests.swift` must carry a real assertion against the
    /// generated API (not the vacuous `XCTAssertTrue(true)` placeholder) whenever the API
    /// surface has something to assert against, and must be `generated_header: false` so
    /// the create-only write-path guard (`write_scaffold_files_report`'s `can_skip`) never
    /// overwrites a real hand-written suite once one exists at that path.
    #[test]
    fn scaffold_swift_emits_real_test_assertions_and_is_create_only() {
        let api = ApiSurface {
            types: vec![simple_type("Widget")],
            ..Default::default()
        };
        let files = scaffold_swift(&api, &minimal_config()).expect("scaffold");
        let test_file = find_file(&files, "packages/swift/Tests/MyLibTests/MyLibTests.swift");

        assert!(
            !test_file.generated_header,
            "test seed must be generated_header: false (create-only)"
        );
        assert!(test_file.content.contains("JSONEncoder"), "got:\n{}", test_file.content);
        assert!(
            !test_file.content.contains("func testPlaceholder"),
            "must not emit the old vacuous placeholder test, got:\n{}",
            test_file.content
        );
    }

    /// Positive control: scaffolding into a project with no pre-existing test file must
    /// still produce the seed. The create-only guard lives in the write path
    /// (`write_scaffold_files_report`'s `can_skip = !overwrite && !file.generated_header
    /// && full_path.exists()`), not in generation -- `scaffold_swift` itself always
    /// returns the test file so that guard has something to gate.
    #[test]
    fn scaffold_swift_always_emits_a_test_file_seed() {
        let files = scaffold_swift(&ApiSurface::default(), &minimal_config()).expect("scaffold");
        let test_file = find_file(&files, "packages/swift/Tests/MyLibTests/MyLibTests.swift");

        assert!(test_file.content.contains("@testable import MyLib"));
    }

    fn minimal_config() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []
"#,
        )
    }

    /// Fresh swift-bridge build output always wins: the umbrella header is
    /// regenerated even if a stale populated header is on disk.
    #[test]
    fn render_header_prefers_fresh_build_output() {
        let out = render_rust_bridge_c_header(
            "my-lib-swift",
            Some((
                "// core\nvoid __swift_bridge__$core(void);\n".into(),
                "// crate\n".into(),
            )),
            Some("// stale __swift_bridge__$old\n"),
        )
        .expect("a complete assembly must resolve");
        assert!(
            out.contains("Concatenates SwiftBridgeCore.h"),
            "expected umbrella, got:\n{out}"
        );
        assert!(
            out.contains("__swift_bridge__$core"),
            "expected fresh core decls, got:\n{out}"
        );
    }

    /// Regression: `alef all --clean` runs without compiling the binding crate, so
    /// no fresh output exists. A previously-populated header committed on disk must
    /// be preserved verbatim rather than reverted to the placeholder — otherwise the
    /// published source package loses every `__swift_bridge__$*` declaration and no
    /// SwiftPM consumer can compile.
    #[test]
    fn render_header_preserves_committed_populated_header() {
        let populated = "#include <stdint.h>\nvoid __swift_bridge__$RustStr$partial_eq(void);\n";
        let out = render_rust_bridge_c_header("my-lib-swift", None, Some(populated)).expect("render");
        assert_eq!(
            out, populated,
            "populated header must be preserved verbatim when no fresh output"
        );
    }

    /// A consumer's own concat script may emit a populated header without alef's
    /// umbrella marker; it must still be preserved (discriminated by the presence
    /// of a `__swift_bridge__$` symbol, not the umbrella comment).
    #[test]
    fn render_header_preserves_markerless_populated_header() {
        let populated = "#include <stdint.h>\nvoid __swift_bridge__$Vec_u8$new(void);\n";
        assert!(umbrella_header::is_populated(populated));
        let out = render_rust_bridge_c_header("my-lib-swift", None, Some(populated)).expect("render");
        assert_eq!(out, populated);
    }

    /// With neither fresh output nor a populated header on disk, emit the
    /// placeholder so SwiftPM accepts the target before the first build. An
    /// existing placeholder (typedefs only, no `__swift_bridge__$`) is not treated
    /// as populated and is replaced by the canonical placeholder.
    #[test]
    fn render_header_emits_placeholder_without_populated_source() {
        let placeholder_marker = "Placeholder header for the RustBridgeC SwiftPM target";

        let from_nothing = render_rust_bridge_c_header("my-lib-swift", None, None).expect("render");
        assert!(
            from_nothing.contains(placeholder_marker),
            "expected placeholder, got:\n{from_nothing}"
        );
        assert!(
            !umbrella_header::is_populated(&from_nothing),
            "placeholder must not look populated"
        );

        let stale_placeholder = "#ifndef RUST_BRIDGE_C_H\ntypedef struct RustStr { int x; } RustStr;\n";
        assert!(!umbrella_header::is_populated(stale_placeholder));
        let from_placeholder =
            render_rust_bridge_c_header("my-lib-swift", None, Some(stale_placeholder)).expect("render");
        assert!(
            from_placeholder.contains(placeholder_marker),
            "expected placeholder, got:\n{from_placeholder}"
        );
    }

    /// The root `Package.swift` must use `.binaryTarget` with version + checksum
    /// placeholders so that SwiftPM consumers depending on the repo via
    /// `.package(url: ...)` can resolve the package. Source-based targets with
    /// `.unsafeFlags` are rejected by SwiftPM in remote-dependency resolution
    /// (`error: the target ... contains unsafe build flags`).
    ///
    /// The placeholders are filled in by:
    ///   - `__ALEF_SWIFT_VERSION__` → `alef sync-versions`
    ///   - `__ALEF_SWIFT_CHECKSUM__` → publish flow when building the artifactbundle.
    #[test]
    fn root_package_swift_uses_binary_target_with_placeholders() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []
[crates.package_metadata]
repository = "https://github.com/example/my-lib"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_swift(&api, &config).expect("scaffold");
        let root = find_file(&files, "Package.swift");

        assert!(
            root.content.contains(".binaryTarget("),
            "root Package.swift must use .binaryTarget, got:\n{}",
            root.content
        );
        assert!(
            !root.content.contains(".unsafeFlags"),
            "root Package.swift must not contain .unsafeFlags (breaks remote SwiftPM consumers), got:\n{}",
            root.content
        );
        assert!(
            root.content.contains("v__ALEF_SWIFT_VERSION__"),
            "root Package.swift must contain __ALEF_SWIFT_VERSION__ placeholder for sync-versions, got:\n{}",
            root.content
        );
        assert!(
            root.content.contains("__ALEF_SWIFT_CHECKSUM__"),
            "root Package.swift must contain __ALEF_SWIFT_CHECKSUM__ placeholder for publish flow, got:\n{}",
            root.content
        );
        assert!(
            root.content
                .contains("https://github.com/example/my-lib/releases/download/v__ALEF_SWIFT_VERSION__/"),
            "root Package.swift URL must point at configured repository, got:\n{}",
            root.content
        );
        assert!(
            root.content.contains("RustBridgeC"),
            "root Package.swift must declare RustBridgeC target for C types, got:\n{}",
            root.content
        );
        assert!(
            root.content.contains(r#"dependencies: ["RustBridge", "RustBridgeC"]"#),
            "root Package.swift must declare bridge dependencies for the Swift target, got:\n{}",
            root.content
        );
    }

    /// The in-tree `packages/swift/Package.swift` keeps the source-based layout
    /// with `.unsafeFlags` linker settings — that variant is used by `swift test
    /// --package-path packages/swift` during local development.
    #[test]
    fn in_tree_package_swift_keeps_source_based_layout() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_swift(&api, &config).expect("scaffold");
        let pkg = find_file(&files, "packages/swift/Package.swift");
        assert!(
            pkg.content.contains(".unsafeFlags"),
            "in-tree packages/swift/Package.swift must keep source-based layout"
        );
        assert!(
            !pkg.content.contains(".binaryTarget("),
            "in-tree packages/swift/Package.swift must not use .binaryTarget"
        );
    }

    /// Both the in-tree and root Package.swift manifests must link the C++
    /// standard library, platform-conditionally. The Rust staticlib/binary
    /// target pulls in C++ dependencies (onnxruntime, tesseract, ClipperLib)
    /// that reference the C++ runtime/ABI (`__cxa_throw`,
    /// `__gxx_personality_v0`, ...); a `.a` archive does not carry that
    /// transitive system-lib dependency, so SwiftPM must link it explicitly
    /// or `swift test` fails at link time with undefined symbols. Regression
    /// test for the rc.32 published-Swift-package link failure.
    #[test]
    fn package_swift_links_cxx_standard_library_per_platform() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []
[crates.package_metadata]
repository = "https://github.com/example/my-lib"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_swift(&api, &config).expect("scaffold");

        let in_tree = find_file(&files, "packages/swift/Package.swift");
        assert!(
            in_tree
                .content
                .contains(r#".linkedLibrary("c++", .when(platforms: [.macOS, .iOS]))"#),
            "in-tree Package.swift must link libc++ on Apple platforms, got:\n{}",
            in_tree.content
        );
        assert!(
            in_tree
                .content
                .contains(r#".linkedLibrary("stdc++", .when(platforms: [.linux]))"#),
            "in-tree Package.swift must link libstdc++ on Linux, got:\n{}",
            in_tree.content
        );

        let root = find_file(&files, "Package.swift");
        assert!(
            root.content
                .contains(r#".linkedLibrary("c++", .when(platforms: [.macOS, .iOS]))"#),
            "root Package.swift must link libc++ on Apple platforms, got:\n{}",
            root.content
        );
        assert!(
            root.content
                .contains(r#".linkedLibrary("stdc++", .when(platforms: [.linux]))"#),
            "root Package.swift must link libstdc++ on Linux, got:\n{}",
            root.content
        );
    }

    /// When capsule dependencies are present, `products:` must precede `dependencies:`
    /// in the Package initializer — SwiftPM requires this argument order.
    #[test]
    fn in_tree_package_swift_with_capsules_has_correct_argument_order() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []

[crates.swift.capsule_types.Language]
host_type = "SwiftTreeSitter.Language"
package = "https://github.com/tree-sitter/tree-sitter-swift"
package_version = "0.25.0"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_swift(&api, &config).expect("scaffold");
        let pkg = find_file(&files, "packages/swift/Package.swift");

        let products_pos = pkg.content.find("products: [").expect("must have products: argument");
        let dependencies_pos = pkg
            .content
            .find("dependencies: [")
            .expect("must have dependencies: argument when capsules present");

        assert!(
            products_pos < dependencies_pos,
            "products: must precede dependencies: in Package(...) initializer. \
             Found products: at byte {}, dependencies: at byte {}. Full content:\n{}",
            products_pos,
            dependencies_pos,
            pkg.content
        );

        assert!(
            pkg.content.contains("tree-sitter-swift"),
            "capsule package reference must be present in dependencies: block"
        );
        assert!(
            pkg.content.contains("0.25.0"),
            "capsule package version must be present in dependencies: block"
        );
    }

    /// The root (published-distribution) Package.swift must inject the same host-native
    /// capsule dependencies as the in-tree manifest. Without them, remote consumers fail
    /// to compile the generated `import SwiftTreeSitter` with `no such module 'SwiftTreeSitter'`.
    #[test]
    fn root_package_swift_injects_capsule_dependencies() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []

[crates.scaffold]
repository = "https://github.com/acme/my-lib"

[crates.swift.capsule_types.Language]
host_type = "SwiftTreeSitter.Language"
package = "https://github.com/tree-sitter/swift-tree-sitter"
package_version = "0.25.0"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_swift(&api, &config).expect("scaffold");
        let pkg = find_file(&files, "Package.swift");

        assert!(
            pkg.content
                .contains(".package(url: \"https://github.com/tree-sitter/swift-tree-sitter\""),
            "root manifest must declare the capsule package dependency. Full content:\n{}",
            pkg.content
        );
        assert!(
            pkg.content
                .contains(".product(name: \"SwiftTreeSitter\", package: \"swift-tree-sitter\")"),
            "root manifest module target must depend on the capsule product. Full content:\n{}",
            pkg.content
        );
    }

    /// Literal-for-literal reference set used by the migration tests below, each read
    /// straight out of the named repo's real
    /// `packages/swift/Tests/<Name>Tests/<Name>Tests.swift`: html-to-markdown carries the
    /// historical placeholder shape (`testPlaceholder`, a two-line rationale comment --
    /// exactly the shape a byte-match against the *current* generator's
    /// `testModuleImportsSuccessfully` output would miss), while liter-llm and
    /// tree-sitter-language-pack are real suites that must never be touched.
    fn h2m_historical_placeholder() -> &'static str {
        r##"import XCTest

@testable import HtmlToMarkdown

final class HtmlToMarkdownTests: XCTestCase {
  func testPlaceholder() throws {
    // Placeholder test so `swift test` has a target to run.
    // Replace or extend with real tests against the HtmlToMarkdown module.
    XCTAssertTrue(true)
  }
}
"##
    }

    /// liter-llm's real suite: it has the *same* counts as the h2m placeholder (one
    /// `XCTAssert`-family call, one `func test`), so it is the case that proves the counts
    /// alone are not the signature -- its single assertion is a real `XCTAssertEqual`
    /// round trip rather than the tautology, and only the tautology clause keeps this file
    /// safe.
    fn liter_llm_hand_written_suite() -> &'static str {
        r##"import XCTest

@testable import LiterLlm

final class LiterLlmTests: XCTestCase {
  func testSystemMessageRoundTripsThroughJSON() throws {
    let message = SystemMessage(content: .text(field0: "be concise"), name: "system")
    let data = try JSONEncoder().encode(message)
    let decoded = try JSONDecoder().decode(SystemMessage.self, from: data)
    XCTAssertEqual(decoded, message)
  }
}
"##
    }

    /// Verbatim copy of tree-sitter-language-pack's real
    /// `packages/swift/Tests/TreeSitterLanguagePackTests/TreeSitterLanguagePackTests.swift`
    /// (17 `XCTAssert`-family calls across 17 `func test` methods, five `XCTestCase`
    /// classes) -- the largest and structurally most different of the three real trees
    /// checked against this predicate. It also carries two `XCTUnwrap` calls, which
    /// deliberately do not count toward the `XCTAssert` prefix, and `XCTAssertTrue(` calls
    /// whose argument is a real expression rather than the `true` literal, so it fails both
    /// clauses independently.
    fn tree_sitter_language_pack_hand_written_suite() -> &'static str {
        r##"import XCTest
import Foundation

import TreeSitterLanguagePack
import SwiftTreeSitter

// Requires the native `tree-sitter-language-pack-swift` + `ts-pack-core-ffi` Rust crates to
// be built and wired via `scripts/setup-swift-bridge.sh` (see `task swift:build` /
// `task swift:test`), compiled with TSLP_LINK_MODE=static and TSLP_LANGUAGES=mojo,nim,norg
// (see .task/swift.yml). Every test below that needs a real grammar (parsing, root-node
// kind, `process()`) uses "nim", one of those three statically-compiled languages, so this
// suite needs no network access and no warm download cache. "python"/"rust"/"markdown"
// appear only as literal data values in the pure language-detection tests below, which
// consult a static extension/shebang lookup table and never touch a parser.

/// Pure functions that need no grammar at all: extension/path/shebang lookup tables.
final class LanguageDetectionTests: XCTestCase {
    func testDetectLanguageFromExtensionMapsPyToPython() {
        XCTAssertEqual(
            TreeSitterLanguagePack.detectLanguageFromExtension(ext: "py"),
            "python",
            "\"py\" is a well-known extension and must resolve to \"python\""
        )
    }

    func testDetectLanguageFromExtensionIsCaseInsensitive() {
        XCTAssertEqual(
            TreeSitterLanguagePack.detectLanguageFromExtension(ext: "RS"),
            "rust",
            "extension matching must be case-insensitive per documented behavior"
        )
    }

    func testDetectLanguageFromExtensionReturnsNilForUnknownExtension() {
        XCTAssertNil(TreeSitterLanguagePack.detectLanguageFromExtension(ext: "this-extension-does-not-exist"))
    }

    func testDetectLanguageFromPathMatchesRustFile() {
        XCTAssertEqual(TreeSitterLanguagePack.detectLanguageFromPath(path: "src/main.rs"), "rust")
    }

    func testDetectLanguageFromPathReturnsNilWithoutExtension() {
        XCTAssertNil(
            TreeSitterLanguagePack.detectLanguageFromPath(path: "Makefile"),
            "a path with no extension has nothing to detect from"
        )
    }

    func testDetectLanguageFromContentMatchesPythonShebang() {
        XCTAssertEqual(
            TreeSitterLanguagePack.detectLanguageFromContent(content: "#!/usr/bin/env python3\npass"),
            "python"
        )
    }

    func testDetectLanguageFromContentReturnsNilWithoutShebang() {
        XCTAssertNil(TreeSitterLanguagePack.detectLanguageFromContent(content: "no shebang here"))
    }

    func testDetectLanguageAliasResolvesPathExtension() {
        XCTAssertEqual(
            TreeSitterLanguagePack.detectLanguage(path: "README.md"),
            "markdown",
            "detectLanguage is documented as a path/extension detection alias"
        )
    }
}

/// Bundled query lookup: also pure, no grammar required.
final class BundledQueryTests: XCTestCase {
    func testGetHighlightsQueryReturnsNilForUnknownLanguage() {
        XCTAssertNil(TreeSitterLanguagePack.getHighlightsQuery(language: "this-language-does-not-exist"))
    }
}

/// Registry checks against "nim", which `task swift:test` compiles in statically
/// (TSLP_LANGUAGES=mojo,nim,norg), so these never touch the network.
final class RegistryTests: XCTestCase {
    func testHasLanguageIsTrueForStaticallyCompiledLanguage() {
        XCTAssertTrue(
            TreeSitterLanguagePack.hasLanguage(name: "nim"),
            "nim is compiled in by the swift build task (TSLP_LANGUAGES=mojo,nim,norg)"
        )
    }

    func testHasLanguageIsFalseForUnknownLanguage() {
        XCTAssertFalse(TreeSitterLanguagePack.hasLanguage(name: "totally-bogus-language-name"))
    }

    func testAvailableLanguagesContainsStaticallyCompiledLanguage() {
        XCTAssertTrue(TreeSitterLanguagePack.availableLanguages().contains("nim"))
    }

    func testLanguageCountMatchesAvailableLanguagesCount() {
        let count = TreeSitterLanguagePack.languageCount()
        let names = TreeSitterLanguagePack.availableLanguages()
        XCTAssertEqual(
            count,
            UInt(names.count),
            "languageCount() must always equal availableLanguages().count; a mismatch means one of the two "
                + "accessors is stale relative to the other"
        )
    }
}

/// Real parsing through the upstream SwiftTreeSitter.Parser, mirroring
/// CapsulePassthroughTests.swift's hand-written pattern: `getLanguage()` hands back a real
/// `SwiftTreeSitter.Language` capsule usable directly with `SwiftTreeSitter.Parser`.
///
/// Uses "nim" (statically compiled, see the file-header comment) rather than "python": the
/// upstream e2e CapsulePassthroughTests.swift precedent parses python, but that grammar is
/// not one of TSLP_LANGUAGES=mojo,nim,norg baked into this package's build, so it would make
/// this suite network-dependent.
final class ParsingTests: XCTestCase {
    // parsers/nim/src/grammar.json names its start rule "module", and node-types.json
    // confirms "module" is a named node type — verified directly against the grammar
    // sources vendored in this repo, not assumed from another language's doc example.
    func testGetLanguageProducesAParserUsableNimLanguage() throws {
        let nimLanguage = try TreeSitterLanguagePack.getLanguage(name: "nim")

        var parser = Parser()
        try parser.setLanguage(nimLanguage)

        let tree = try XCTUnwrap(parser.parse("echo \"hello\""), "parsing valid nim source must produce a tree")
        let root = try XCTUnwrap(tree.rootNode)

        XCTAssertEqual(root.nodeType, "module", "nim's tree-sitter grammar names its root node \"module\"")
    }

    // fixtures/smoke/nim.json asserts `not_error` for this exact source, so it is
    // known-valid nim, not a guess.
    func testProcessEchoesBackTheRequestedLanguageForValidNim() throws {
        let configObj = try TreeSitterLanguagePack.processConfigFromJson("{\"language\":\"nim\"}")
        let result = try TreeSitterLanguagePack.process(source: "echo \"hello\"", config: configObj)

        XCTAssertEqual(result.language().toString(), "nim")
    }

    // No structure-extraction test: crates/ts-pack-core/src/intel/intelligence.rs
    // `structure_kind_at()` matches an exact, hardcoded set of tree-sitter node kind names
    // (`function_definition`, `function_item`, `struct_item`, ...), and nim's grammar
    // (parsers/nim/src/node-types.json) uses none of them — its declarations are named
    // `declaration` / `declColonEquals`. Structure extraction is therefore unimplemented for
    // nim and would always report an empty list; asserting that would be vacuous, so the
    // test is omitted rather than weakened to a truthiness check.
}

/// Error paths: unknown languages and invalid configuration must fail loudly, never
/// silently succeed with garbage output.
final class ErrorHandlingTests: XCTestCase {
    func testGetLanguageThrowsForUnknownLanguage() {
        XCTAssertThrowsError(try TreeSitterLanguagePack.getLanguage(name: "this-language-does-not-exist-anywhere"))
    }

    func testProcessThrowsForEmptyLanguageName() throws {
        let configObj = try TreeSitterLanguagePack.processConfigFromJson("{\"language\":\"\"}")
        XCTAssertThrowsError(try TreeSitterLanguagePack.process(source: "hello", config: configObj))
    }
}
"##
    }

    /// `is_vacuous_swift_placeholder` fires for html-to-markdown's real on-disk shape
    /// (1 `XCTAssert`, 1 `func test`, tautology) and refuses liter-llm's and
    /// tree-sitter-language-pack's real suites (1/1 without the tautology, and 17/17
    /// respectively) -- pinning the predicate directly against the trees the migration
    /// exists to repair, not just against synthetic fixtures.
    #[test]
    fn vacuity_signature_matches_real_consumer_trees() {
        assert!(is_vacuous_swift_placeholder(h2m_historical_placeholder()));
        assert!(!is_vacuous_swift_placeholder(liter_llm_hand_written_suite()));
        assert!(!is_vacuous_swift_placeholder(
            tree_sitter_language_pack_hand_written_suite()
        ));
    }

    /// Requirement (a): the *current* generator's own placeholder output -- regenerated
    /// here, never hardcoded, since hardcoding it is how a byte-match migration silently
    /// stops matching its own generator after the template drifts -- still migrates.
    #[test]
    fn migrates_the_current_generators_placeholder_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/swift/Tests/MyLibTests");
        std::fs::create_dir_all(&test_dir).expect("create Tests/MyLibTests");
        let current_placeholder = placeholder_test("MyLib");
        std::fs::write(test_dir.join("MyLibTests.swift"), &current_placeholder).expect("write current placeholder");

        let replacement = codable_round_trip_test(
            "MyLib",
            "Widget",
            &[SimpleCodableField {
                swift_label: "count".to_string(),
                literal: "1".to_string(),
            }],
        );
        let relative_path = std::path::Path::new("packages/swift/Tests/MyLibTests/MyLibTests.swift");
        let changed =
            migrate_swift_placeholder_test(dir.path(), relative_path, &replacement).expect("migration must not error");
        assert!(
            changed,
            "the current generator's own placeholder must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(test_dir.join("MyLibTests.swift")).expect("read migrated file");
        assert_eq!(on_disk, replacement);
    }

    /// Requirement (b): the exact shape found on disk in html-to-markdown -- the historical
    /// placeholder revision this migration exists to reach, and the one a byte-match
    /// against only the current template would miss -- migrates, and a second pass over the
    /// result is a no-op.
    #[test]
    fn migrates_the_h2m_historical_shape_and_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/swift/Tests/HtmlToMarkdownTests");
        std::fs::create_dir_all(&test_dir).expect("create Tests/HtmlToMarkdownTests");
        std::fs::write(test_dir.join("HtmlToMarkdownTests.swift"), h2m_historical_placeholder())
            .expect("write h2m historical placeholder");

        let replacement = codable_round_trip_test(
            "HtmlToMarkdown",
            "Widget",
            &[SimpleCodableField {
                swift_label: "count".to_string(),
                literal: "1".to_string(),
            }],
        );
        let relative_path = std::path::Path::new("packages/swift/Tests/HtmlToMarkdownTests/HtmlToMarkdownTests.swift");
        let changed =
            migrate_swift_placeholder_test(dir.path(), relative_path, &replacement).expect("migration must not error");
        assert!(
            changed,
            "h2m's real on-disk placeholder shape must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(test_dir.join("HtmlToMarkdownTests.swift")).expect("read migrated file");
        assert_eq!(on_disk, replacement);

        let changed_again = migrate_swift_placeholder_test(dir.path(), relative_path, &replacement)
            .expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    /// Requirement (c), the hard case: liter-llm's real suite has exactly the same 1/1
    /// counts as the placeholder, so only the tautology clause distinguishes them. It must
    /// survive byte for byte.
    #[test]
    fn does_not_touch_the_liter_llm_hand_written_suite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/swift/Tests/LiterLlmTests");
        std::fs::create_dir_all(&test_dir).expect("create Tests/LiterLlmTests");
        let hand_written = liter_llm_hand_written_suite();
        std::fs::write(test_dir.join("LiterLlmTests.swift"), hand_written).expect("write hand-written test");

        let relative_path = std::path::Path::new("packages/swift/Tests/LiterLlmTests/LiterLlmTests.swift");
        let changed = migrate_swift_placeholder_test(dir.path(), relative_path, "anything else entirely")
            .expect("migration must not error");
        assert!(!changed, "a real suite must never be reported as changed");

        let on_disk = std::fs::read_to_string(test_dir.join("LiterLlmTests.swift")).expect("read file");
        assert_eq!(on_disk, hand_written, "a real suite must survive byte-for-byte");
    }

    /// Requirement (c), the large case: tree-sitter-language-pack's real 17-test suite must
    /// never be reported as a migration candidate either.
    #[test]
    fn does_not_touch_the_tree_sitter_language_pack_hand_written_suite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/swift/Tests/TreeSitterLanguagePackTests");
        std::fs::create_dir_all(&test_dir).expect("create Tests/TreeSitterLanguagePackTests");
        let hand_written = tree_sitter_language_pack_hand_written_suite();
        std::fs::write(test_dir.join("TreeSitterLanguagePackTests.swift"), hand_written)
            .expect("write hand-written test");

        let relative_path =
            std::path::Path::new("packages/swift/Tests/TreeSitterLanguagePackTests/TreeSitterLanguagePackTests.swift");
        let changed = migrate_swift_placeholder_test(dir.path(), relative_path, "anything else entirely")
            .expect("migration must not error");
        assert!(!changed, "a hand-written suite must never be reported as changed");

        let on_disk = std::fs::read_to_string(test_dir.join("TreeSitterLanguagePackTests.swift")).expect("read file");
        assert_eq!(on_disk, hand_written, "hand-written suite must survive byte-for-byte");
    }

    /// Requirement (d): a `Tests/<Name>Tests/<Name>Tests.swift` that does not exist yet
    /// (nothing scaffolded so far) must not be created and must not error -- there is
    /// nothing to migrate.
    #[test]
    fn migrate_swift_placeholder_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = std::path::Path::new("packages/swift/Tests/MyLibTests/MyLibTests.swift");
        let changed = migrate_swift_placeholder_test(dir.path(), relative_path, "new content").expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }
}
