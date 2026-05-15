use crate::naming::{swift_min_ios, swift_min_macos};
use crate::scaffold_meta;
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

pub(crate) fn scaffold_swift(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let module = config.swift_module();
    // Strip the minor version component: "13.0" → "13", "16.0" → "16".
    // Swift PackageDescription uses e.g. `.v13` and `.v16`.
    let min_macos_major = swift_min_macos(config).split('.').next().unwrap_or("13").to_string();
    let min_ios_major = swift_min_ios(config).split('.').next().unwrap_or("16").to_string();

    // crate_name is e.g. "kreuzberg", the Cargo crate being wrapped.
    // The swift-bridge output files are named after the *binding* crate, e.g. "kreuzberg-swift".
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
    let package_swift = format!(
        r#"// swift-tools-version: 6.0
import PackageDescription

// NOTE: Run `cargo build -p {binding_crate}` before `swift build`.
// The build step generates Swift + C bridge sources; copy them into Sources/RustBridge
// and Sources/RustBridgeC before building. See README.md for the full workflow.
let package = Package(
    name: "{module}",
    platforms: [
        .macOS(.v{min_macos}),
        .iOS(.v{min_ios}),
    ],
    products: [
        .library(name: "{module}", targets: ["{module}"]),
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
        .target(name: "{module}", dependencies: ["RustBridge"], path: "Sources/{module}"),
        .testTarget(name: "{module}Tests", dependencies: ["{module}"], path: "Tests/{module}Tests"),
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

    // 2-space indentation matches `swift format` defaults — keeps the scaffolded
    // file lint-clean against `swift format lint`.
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
// Run `cargo build -p {binding_crate}` and copy the generated Swift files here
// (with `import RustBridgeC` prepended). See README.md for instructions.
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

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.swift]\nindent_style = space\nindent_size = 4\n";

    let swiftformat = "lineLength = 120\nindent = 4\nusesTabs = false\n";

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
OUT=$(ls -dt target/debug/build/{binding_crate}-*/out 2>/dev/null | head -1)
cat "$OUT/SwiftBridgeCore.h" "$OUT/{binding_crate}/{binding_crate}.h" \
    > packages/swift/Sources/RustBridgeC/RustBridgeC.h
{{ echo "import RustBridgeC"; cat "$OUT/SwiftBridgeCore.swift"; }} \
    > packages/swift/Sources/RustBridge/SwiftBridgeCore.swift
{{ echo "import RustBridgeC"; cat "$OUT/{binding_crate}/{binding_crate}.swift"; }} \
    > packages/swift/Sources/RustBridge/{binding_crate}.swift
swift build --package-path packages/swift
swift test --package-path packages/swift
```

The generated `Sources/RustBridgeC` and `Sources/RustBridge` artifacts are
rewritten after each Cargo clean or rebuild.

## License

{license}
"#,
        module = module,
        description = meta.description,
        binding_crate = binding_crate_name,
        license = meta.license,
    );

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

    Ok(vec![
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
    ])
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
        format!(
            "#ifndef RUST_BRIDGE_C_H\n\
             #define RUST_BRIDGE_C_H\n\
             \n\
             // Placeholder header for the RustBridgeC SwiftPM target.\n\
             // Run `cargo build -p {binding_crate_name}` and re-run `alef all` to populate.\n\
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
            let mtime = std::fs::metadata(&out)
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
