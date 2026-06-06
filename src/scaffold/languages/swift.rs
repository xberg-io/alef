use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::scaffold::naming::{swift_min_ios, swift_min_macos};
use crate::scaffold::scaffold_meta;
use std::path::PathBuf;

pub(crate) fn scaffold_swift(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let module = config.swift_module();
    // Strip the minor version component: "13.0" → "13", "16.0" → "16".
    // Swift PackageDescription uses e.g. `.v13` and `.v16`.
    let min_macos_major = swift_min_macos(config).split('.').next().unwrap_or("13").to_string();
    let min_ios_major = swift_min_ios(config).split('.').next().unwrap_or("16").to_string();

    // crate_name is e.g. "sample_core", the Cargo crate being wrapped.
    // The swift-bridge output files are named after the *binding* crate, e.g. "sample_core-swift".
    let crate_name = &config.name;
    let binding_crate_name = format!("{crate_name}-swift");
    let binding_crate_underscore = binding_crate_name.replace('-', "_");

    // Three-target layout:
    //   RustBridgeC  — pure C/headers target; Swift files import this to see C types
    //   RustBridge   — Swift bridge files + linker settings for the static library
    //   {Module}     — user-facing Swift API, depends on RustBridge
    //
    // Rationale: swift-bridge's generated SwiftBridgeCore.swift references C types
    // (RustStr, RustString, etc.) without import statements. SwiftPM's mixed-target
    // rules require those types to be exported from a separate C target so that
    // `import RustBridgeC` at the top of the generated Swift files brings them in scope.
    //
    // Linking the Rust staticlib: SwiftPM cannot drive Cargo, so the consumer must run
    // `cargo build -p {binding_crate}` first. We then declare `linkerSettings` on
    // RustBridge that pass `-L<repo>/target/{release,debug}` and `-l{binding_underscore}`
    // to the linker. The `-L` paths are relative to the package root (`packages/swift`).
    // Both `release` and `debug` are listed so either Cargo profile produces a runnable
    // `swift test`. macOS Frameworks (Security, CoreFoundation, SystemConfiguration) are
    // linked because the Rust binding pulls in `ureq` / `rustls-platform-verifier` /
    // `keyring`-style deps that reference them on macOS targets.
    //
    // `.unsafeFlags` prevents this package from being used as a `.package(url: ...)`
    // dependency by other packages. That is acceptable: the canonical distribution
    // channel for Apple platforms is a pre-built XCFramework. The linkerSettings
    // here only support the in-tree `swift test` workflow.
    // 2-space indentation and no trailing comma on single-element array literals match
    // `swift-format` defaults so the generated file is lint-clean without a post-pass.
    let package_swift = format!(
        r#"// swift-tools-version: 6.0
import PackageDescription

// NOTE: Run `cargo build -p {binding_crate}` and then rerun `alef generate`
// before `swift build`. Alef materializes the swift-bridge Swift/C outputs into
// Sources/RustBridge and Sources/RustBridgeC when the Cargo build output exists.
// See README.md for the full workflow.
let package = Package(
  name: "{module}",
  platforms: [
    .macOS(.v{min_macos}),
    .iOS(.v{min_ios}),
  ],
  products: [
    .library(name: "{module}", targets: ["{module}"])
  ],
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
    // linkerSettings wire the Rust staticlib (lib{binding_underscore}.a) produced by
    // `cargo build -p {binding_crate}` so `swift build` / `swift test` can resolve
    // the `__swift_bridge__$*` C symbols. Both target/release and target/debug are
    // searched so either cargo profile works.
    .target(
      name: "RustBridge",
      dependencies: ["RustBridgeC"],
      path: "Sources/RustBridge",
      linkerSettings: [
        .unsafeFlags([
          "-L../../target/release",
          "-L../../target/debug",
        ]),
        .linkedLibrary("{binding_underscore}"),
        .linkedFramework("Security", .when(platforms: [.macOS, .iOS])),
        .linkedFramework("CoreFoundation", .when(platforms: [.macOS, .iOS])),
        .linkedFramework("SystemConfiguration", .when(platforms: [.macOS])),
      ]
    ),
    .target(
      name: "{module}", dependencies: ["RustBridge"],
      path: "Sources/{module}"),
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
    );

    let gitignore = ".build/\nPackages/\nxcuserdata/\nDerivedData/\n.swiftpm/\n*.xcodeproj\n";

    // 2-space indentation matches `swift-format` defaults and a blank line between
    // import groups is required by `swift-format`'s import-ordering rules.
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

    // RustBridgeC header: umbrella that includes both swift-bridge generated headers, or
    // a placeholder when the cargo build hasn't run yet.
    // The umbrella form is used when `cargo build -p {binding_crate}` has already been run
    // and the output headers are available in `target/*/build/{binding_crate}-*/out/`.
    let rust_bridge_c_header = build_rust_bridge_c_header(&binding_crate_name);

    // RustBridge.swift placeholder — the target needs at least one Swift source.
    // SwiftBridgeCore.swift and {crate}-swift.swift are copied here (with
    // `import RustBridgeC` prepended) after `cargo build -p {binding_crate_name}` runs.
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

    // module.modulemap is not needed for the RustBridgeC target approach (SwiftPM
    // infers the module from publicHeadersPath), but we keep it in RustBridge for
    // documentation purposes. It is not strictly required.
    let module_modulemap = "// This modulemap is unused — the RustBridgeC target provides the C types.\n// SwiftPM discovers RustBridgeC.h via the publicHeadersPath setting.\n";

    // 2-space indent matches `swift-format` defaults so editors and the formatter agree.
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

    // 2-space indentation matches `swift-format` defaults.
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

    // Root-level Package.swift — published-distribution manifest.
    //
    // This file is what external consumers see when they reference the repo via
    // `.package(url: "<repo>", from: "X.Y.Z")`. SwiftPM REJECTS packages that
    // contain `.unsafeFlags` from `.package(url:)` resolution, so the root
    // manifest MUST use `.binaryTarget` referencing a pre-built artifactbundle
    // hosted on GitHub Releases. The source-based layout (with `.unsafeFlags`
    // and an absolute `cargo build` dependency) stays under
    // `packages/swift/Package.swift` for in-tree dev workflows.
    //
    // The URL placeholder `v__ALEF_SWIFT_VERSION__` is substituted by
    // `alef sync-versions` (so `task set-version X.Y.Z` keeps the URL in sync
    // with the workspace version). The checksum placeholder
    // `__ALEF_SWIFT_CHECKSUM__` is substituted by the publish flow
    // (`src/publish/package/swift.rs::patch_root_package_manifest`) when the
    // artifactbundle is produced and `ALEF_SWIFT_CHECKSUM` is exported. Both
    // placeholders are required — leaving the manifest source-based here
    // means every `alef all --clean` regen overwrites a previously published
    // binaryTarget manifest with an unsafe-flags variant, breaking remote
    // SwiftPM consumers (`error: the target 'RustBridge' in product
    // '{module}' contains unsafe build flags`).
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
  ],
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
      path: "packages/swift/Sources/RustBridge"
    ),
    .target(
      name: "{module}",
      dependencies: ["RustBridge", "RustBridgeC"],
      path: "packages/swift/Sources/{module}"
    ),
  ]
)
"#,
            module = module,
            min_macos = min_macos_major,
            min_ios = min_ios_major,
            repository = repository.trim_end_matches('/'),
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

/// Build the content for `Sources/RustBridgeC/RustBridgeC.h`.
///
/// When `cargo build -p {binding_crate}` has already been run, returns a thin umbrella
/// header that concatenates `SwiftBridgeCore.h` and `{binding_crate}.h` from the
/// swift-bridge build output. Otherwise returns a placeholder so SwiftPM can accept
/// the `RustBridgeC` target before the first build.
fn build_rust_bridge_c_header(binding_crate_name: &str) -> String {
    if let Some((core_h, crate_h)) = read_swift_bridge_headers(binding_crate_name) {
        format!(
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
        )
    } else {
        // Minimal placeholder that defines C structs used by swift-bridge's
        // generated `SwiftBridgeCore.swift`.  Without these typedefs the Swift compiler
        // reports "cannot find type" errors for every extension block that references
        // RustStr, __private__FfiSlice, or __private__OptionXX types.
        // The real `SwiftBridgeCore.h` (produced by `cargo build -p {binding_crate_name}`)
        // defines identical typedefs; the definitions are compatible and SwiftPM merges
        // them via the module map.
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
            // Use the mtime of the SwiftBridgeCore.h file (written by the build) rather
            // than the directory mtime, which macOS may update on reads.
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
}
