use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::scaffold::naming::{swift_min_ios, swift_min_macos};
use crate::scaffold::scaffold_meta;
use std::path::PathBuf;

pub(crate) fn scaffold_swift(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let module = config.swift_module();
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
import Foundation

// NOTE: Run `cargo build -p {binding_crate}` and then rerun `alef generate`
// before `swift build`. Alef materializes the swift-bridge Swift/C outputs into
// Sources/RustBridge and Sources/RustBridgeC when the Cargo build output exists.
// See README.md for the full workflow.

// Absolute path to the Cargo target dir, resolved from this manifest's own location so the
// runtime rpath is independent of the process working directory (`swift test` may chdir into
// fixture dirs). `#filePath` is a compile-time literal, so this performs no filesystem access.
let rustTargetDir = (#filePath as NSString).deletingLastPathComponent.appending("/../../target")

let package = Package(
  name: "{module}",
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
    // Both target/release and target/debug are searched so either cargo profile works.
    // The FFI library is needed because the generated Swift service API code (App.swift)
    // calls FFI functions directly via @_silgen_name declarations.
    .target(
      name: "RustBridge",
      dependencies: ["RustBridgeC"],
      path: "Sources/RustBridge",
      linkerSettings: [
        .unsafeFlags([
          "-L\(rustTargetDir)/release",
          "-L\(rustTargetDir)/debug",
          // Runtime search paths: the FFI dylib's install_name is @rpath/lib...dylib, so the
          // consumer (and any test bundle linking this target) needs an LC_RPATH to dlopen it.
          // swiftc rejects `-Wl,-rpath,<p>`; the driver-native spelling is `-Xlinker -rpath -Xlinker <p>`.
          "-Xlinker", "-rpath", "-Xlinker", "\(rustTargetDir)/release",
          "-Xlinker", "-rpath", "-Xlinker", "\(rustTargetDir)/debug",
        ]),
        .linkedLibrary("{binding_underscore}"),
        .linkedLibrary("{ffi_lib_name}"),
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

    let test_stub = format!(
        r#"import XCTest

@testable import {module}

final class {module}Tests: XCTestCase {{
  func testPlaceholder() throws {{
    // Placeholder test so `swift test` has a target to run.
    // Replace or extend with real tests against the {module} module.
    XCTAssertTrue(true)
  }}
}}
"#,
        module = module,
    );

    let rust_bridge_c_header = build_rust_bridge_c_header(&binding_crate_name);

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

    let readme = format!(
        r#"# {module}

{description}

## Installation

Add to your `Package.swift`:

```swift
.package(path: "packages/swift"),
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
  name: "{module}",
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
      // `system_configuration` crate → `SC*` symbols). The artifactbundle ships
      // only the `.a`, so these frameworks must be linked by the consumer.
      linkerSettings: [
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
            path: PathBuf::from("packages/swift/README.md"),
            content: readme,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/swift/Examples/Demo/main.swift"),
            content: demo_swift,
            generated_header: false,
        },
    ];
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
fn build_rust_bridge_c_header(binding_crate_name: &str) -> String {
    let fresh_headers = read_swift_bridge_headers(binding_crate_name);
    let existing_header = std::fs::read_to_string(RUST_BRIDGE_C_HEADER_PATH).ok();
    render_rust_bridge_c_header(binding_crate_name, fresh_headers, existing_header.as_deref())
}

/// A `RustBridgeC.h` is "populated" once it carries swift-bridge's generated C
/// function declarations. The placeholder only defines base typedefs and never
/// references a `__swift_bridge__$` symbol, so the presence of that prefix is a
/// reliable populated/placeholder discriminator — independent of whether the
/// header was produced by alef's umbrella or by a consumer's own concat script.
fn header_is_populated(header: &str) -> bool {
    header.contains("__swift_bridge__$")
}

/// Decide the content of `RustBridgeC.h`, given the optional fresh swift-bridge
/// build output and the optional header already present on disk.
///
/// Precedence:
/// 1. Fresh swift-bridge output (the binding crate was built) → regenerate the
///    umbrella header from `SwiftBridgeCore.h` + `{crate}.h`.
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
) -> String {
    if let Some((core_h, crate_h)) = fresh_headers {
        return format!(
            "#ifndef RUST_BRIDGE_C_H\n\
             #define RUST_BRIDGE_C_H\n\
             \n\
             // Auto-generated by alef — do not edit by hand.\n\
             // Concatenates SwiftBridgeCore.h and {binding_crate_name}.h produced by\n\
             // `cargo build -p {binding_crate_name}` via swift_bridge_build.\n\
             \n\
             {core_h}\n\
             {crate_h}\n\
             #endif /* RUST_BRIDGE_C_H */\n"
        );
    }

    if let Some(existing) = existing_header {
        if header_is_populated(existing) {
            return existing.to_string();
        }
    }

    format!(
        "#ifndef RUST_BRIDGE_C_H\n\
         #define RUST_BRIDGE_C_H\n\
         \n\
         // Placeholder header for the RustBridgeC SwiftPM target.\n\
         // Run `cargo build -p {binding_crate_name}` and re-run `alef all` to populate.\n\
         // The typedefs below are the minimum required for SwiftBridgeCore.swift\n\
         // to compile before the full cargo build has been run.\n\
         \n\
         #include <stdint.h>\n\
         #include <stdbool.h>\n\
         \n\
         typedef struct RustStr {{ uint8_t* const start; uintptr_t len; }} RustStr;\n\
         typedef struct __private__FfiSlice {{ void* const start; uintptr_t len; }} __private__FfiSlice;\n\
         typedef struct __private__OptionU8 {{ uint8_t val; bool is_some; }} __private__OptionU8;\n\
         typedef struct __private__OptionI8 {{ int8_t val; bool is_some; }} __private__OptionI8;\n\
         typedef struct __private__OptionU16 {{ uint16_t val; bool is_some; }} __private__OptionU16;\n\
         typedef struct __private__OptionI16 {{ int16_t val; bool is_some; }} __private__OptionI16;\n\
         typedef struct __private__OptionU32 {{ uint32_t val; bool is_some; }} __private__OptionU32;\n\
         typedef struct __private__OptionI32 {{ int32_t val; bool is_some; }} __private__OptionI32;\n\
         typedef struct __private__OptionU64 {{ uint64_t val; bool is_some; }} __private__OptionU64;\n\
         typedef struct __private__OptionI64 {{ int64_t val; bool is_some; }} __private__OptionI64;\n\
         typedef struct __private__OptionUsize {{ uintptr_t val; bool is_some; }} __private__OptionUsize;\n\
         typedef struct __private__OptionIsize {{ intptr_t val; bool is_some; }} __private__OptionIsize;\n\
         typedef struct __private__OptionF32 {{ float val; bool is_some; }} __private__OptionF32;\n\
         typedef struct __private__OptionF64 {{ double val; bool is_some; }} __private__OptionF64;\n\
         typedef struct __private__OptionBool {{ bool val; bool is_some; }} __private__OptionBool;\n\
         \n\
         #endif /* RUST_BRIDGE_C_H */\n"
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
        );
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
        let out = render_rust_bridge_c_header("my-lib-swift", None, Some(populated));
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
        assert!(header_is_populated(populated));
        let out = render_rust_bridge_c_header("my-lib-swift", None, Some(populated));
        assert_eq!(out, populated);
    }

    /// With neither fresh output nor a populated header on disk, emit the
    /// placeholder so SwiftPM accepts the target before the first build. An
    /// existing placeholder (typedefs only, no `__swift_bridge__$`) is not treated
    /// as populated and is replaced by the canonical placeholder.
    #[test]
    fn render_header_emits_placeholder_without_populated_source() {
        let placeholder_marker = "Placeholder header for the RustBridgeC SwiftPM target";

        let from_nothing = render_rust_bridge_c_header("my-lib-swift", None, None);
        assert!(
            from_nothing.contains(placeholder_marker),
            "expected placeholder, got:\n{from_nothing}"
        );
        assert!(
            !header_is_populated(&from_nothing),
            "placeholder must not look populated"
        );

        let stale_placeholder = "#ifndef RUST_BRIDGE_C_H\ntypedef struct RustStr { int x; } RustStr;\n";
        assert!(!header_is_populated(stale_placeholder));
        let from_placeholder = render_rust_bridge_c_header("my-lib-swift", None, Some(stale_placeholder));
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
}
