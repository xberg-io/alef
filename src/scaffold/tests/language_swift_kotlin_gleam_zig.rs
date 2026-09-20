use super::*;

#[test]
fn test_scaffold_swift() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Swift]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(
        files.len(),
        12,
        "Expected 12 files for Swift scaffold (original 6 + root Package.swift + 4 extras + RustBridgeC.c)"
    );

    let package_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Package.swift"))
        .unwrap();
    assert!(
        package_swift.content.contains("name: \"MyLib\""),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains(".macOS(.v13)"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains(".iOS(.v16)"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("swift-tools-version: 6.0"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("Sources/MyLib"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("Tests/MyLibTests"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("\"RustBridge\""),
        "Package.swift must declare RustBridge target; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("\"RustBridgeC\""),
        "Package.swift must declare RustBridgeC target; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("name: \"RustBridge\""),
        "Package.swift must declare RustBridge target; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("unsafeFlags"),
        "In-tree Package.swift must include unsafeFlags for local development; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("import Foundation"),
        "Package.swift must import Foundation to resolve the absolute rpath; got: {}",
        package_swift.content
    );
    assert!(
        package_swift
            .content
            .contains("func resolvedStaticLib(_ name: String) -> String"),
        "Package.swift must resolve staticlibs by explicit .a path so ld64 cannot substitute the sibling .dylib; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("resolvedStaticLib(\"my_lib_swift\")"),
        "Package.swift must link the swift-bridge staticlib via resolvedStaticLib; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("resolvedStaticLib(\"my_lib_ffi\")"),
        "Package.swift must link the FFI staticlib via resolvedStaticLib; got: {}",
        package_swift.content
    );
    assert!(
        !package_swift.content.contains("\"-Xlinker\", \"-rpath\", \"-Xlinker\""),
        "Package.swift must not rely on bare -rpath linking now that staticlibs are linked by explicit path; got: {}",
        package_swift.content
    );
    assert!(
        package_swift
            .content
            .contains("let rustTargetDir = (#filePath as NSString)"),
        "Package.swift must derive the target dir from the manifest path; got: {}",
        package_swift.content
    );
    assert!(
        package_swift
            .content
            .contains("Run `cargo build -p my-lib-swift` and then rerun `alef generate`"),
        "Package.swift must document the Alef materialization step; got: {}",
        package_swift.content
    );

    let gitignore = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.gitignore"))
        .unwrap();
    assert_eq!(gitignore.path, PathBuf::from("packages/swift/.gitignore"));
    assert!(gitignore.content.contains(".build/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains(".swiftpm/"), "got: {}", gitignore.content);

    let header = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridgeC/RustBridgeC.h"))
        .unwrap();
    assert!(
        header.content.contains("#ifndef RUST_BRIDGE_C_H"),
        "got: {}",
        header.content
    );

    let source = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridgeC/RustBridgeC.c"))
        .expect("RustBridgeC.c must be generated so XCBuild has an object file to link (#449)");
    assert!(
        source.content.contains("#include \"RustBridgeC.h\""),
        "got: {}",
        source.content
    );
    assert!(
        source
            .content
            .contains("void my_lib_swift_rust_bridge_c_anchor(void) {}"),
        "RustBridgeC.c must define a namespaced anchor symbol so the object file is never \
         stripped and cannot collide with another package's RustBridgeC target; got: {}",
        source.content
    );

    let modulemap = files.iter().find(|f| f.path.ends_with("module.modulemap")).unwrap();
    assert!(!modulemap.content.is_empty(), "module.modulemap must not be empty");

    let rust_bridge_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridge/RustBridge.swift"))
        .unwrap();
    assert!(
        !rust_bridge_swift.content.is_empty(),
        "RustBridge.swift must not be empty"
    );

    let readme = files.iter().find(|f| f.path == Path::new("packages/swift/README.md"));
    assert!(readme.is_some(), "README.md should be generated");
    assert!(
        readme.unwrap().content.contains("swift build"),
        "README.md must document build process"
    );
    let readme_content = &readme.unwrap().content;
    assert!(
        readme_content.contains("alef generate --lang swift"),
        "README.md must tell users to rerun Alef instead of manually copying swift-bridge output: {readme_content}"
    );
    assert!(
        !readme_content.contains("cat \"$OUT/SwiftBridgeCore.h\""),
        "README.md must not imply manual copied bridge output is the generated-package contract: {readme_content}"
    );
    let editorconfig = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.editorconfig"))
        .expect(".editorconfig should be generated");
    assert!(
        editorconfig.content.contains("indent_size = 2"),
        ".editorconfig must use 2-space indent; got: {}",
        editorconfig.content
    );
    let swiftformat = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.swiftformat"))
        .expect(".swiftformat should be generated");
    assert!(
        swiftformat.content.contains("indent = 2"),
        ".swiftformat must use 2-space indent; got: {}",
        swiftformat.content
    );

    assert!(
        package_swift.content.contains("\n  name:"),
        "Package.swift must use 2-space indentation; got: {}",
        package_swift.content
    );
    assert!(
        !package_swift
            .content
            .contains(".library(name: \"MyLib\", targets: [\"MyLib\"]),"),
        "Package.swift single-element products array must not have trailing comma; got: {}",
        package_swift.content
    );

    let test_stub = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Tests") && f.path.extension().is_some_and(|e| e == "swift"))
        .expect("test stub .swift should be generated");
    assert!(
        test_stub.content.contains("import XCTest\n\n@testable"),
        "test stub must have blank line between import groups; got: {}",
        test_stub.content
    );

    let demo = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Examples/Demo/main.swift"))
        .expect("Demo example should be generated");
    assert!(
        demo.content.contains("\n  static func main()"),
        "Demo must use 2-space indentation; got: {}",
        demo.content
    );

    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Swift scaffold must not emit GitHub workflows"
    );
}

/// `[crates.swift].package_name` is validated at resolve time (`validate_swift_coordinates` /
/// `ResolvedCrateConfig::swift_package_name`) precisely because it is spliced into the
/// top-level `Package(name: ...)` argument -- a coordinate distinct from `module_name`, since
/// real published SwiftPM packages routinely use kebab-case here. Both the in-tree and root
/// manifests must actually use it, not silently fall back to the module name, or the validated
/// field would have no effect on generated output.
#[test]
fn package_swift_uses_configured_package_name_not_module_name() {
    let config = test_config_from_toml(
        r#"
[crates.swift]
module_name = "SwiftPkgModule"
package_name = "swift-pkg-name"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Swift]).unwrap();
    let files = language_files(&all_files);

    let in_tree = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Package.swift"))
        .unwrap();
    assert!(
        in_tree.content.contains("name: \"swift-pkg-name\","),
        "in-tree Package.swift must use the configured package_name, got:\n{}",
        in_tree.content
    );
    assert!(
        in_tree.content.contains(".library(name: \"SwiftPkgModule\""),
        "in-tree Package.swift library/target names must still use module_name, got:\n{}",
        in_tree.content
    );

    let root = files.iter().find(|f| f.path == Path::new("Package.swift")).unwrap();
    assert!(
        root.content.contains("name: \"swift-pkg-name\","),
        "root Package.swift must use the configured package_name, got:\n{}",
        root.content
    );
    assert!(
        root.content.contains(".library(name: \"SwiftPkgModule\""),
        "root Package.swift library/target names must still use module_name, got:\n{}",
        root.content
    );
}

// Regression for #555: once `[crates.readme.languages.swift]` is configured, the
// README module (`crate::readme`) owns `packages/swift/README.md` end-to-end, and
// scaffold must not emit a second, independent copy at the same path. A run that
// only scaffolds (or errors before the README stage runs) would otherwise ship
// this skeleton note as the final content with every configured section silently
// dropped.
#[test]
fn should_not_emit_placeholder_readme_when_readme_module_configures_swift() {
    let config = test_config_from_toml(
        r#"
[crates.readme.languages.swift]
template = "language_package.md"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Swift]).unwrap();
    let files = language_files(&all_files);
    assert!(
        files.iter().all(|f| f.path != Path::new("packages/swift/README.md")),
        "scaffold must not emit packages/swift/README.md once the README module is \
         configured for swift (#555)"
    );
}

#[test]
fn test_scaffold_kotlin() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 7, "Expected 7 files for Kotlin scaffold");
    assert_eq!(files[0].path, PathBuf::from("packages/kotlin/build.gradle.kts"));
    assert!(files[0].content.contains("kotlin(\"jvm\")"));
    assert!(
        files[0].content.contains("org.jspecify:jspecify:"),
        "build.gradle.kts must declare jspecify; got:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("ktlint"),
        "ktlint must not be wired into the plain-kotlin build (single formatter is ktfmt); got:\n{}",
        files[0].content
    );
    assert!(
        files[0].content.contains("artifactId = \"my-lib-kotlin\""),
        "publication artifactId override missing; got:\n{}",
        files[0].content
    );
    assert!(
        files[0].content.contains("JavaVersion.VERSION_21") && files[0].content.contains("JvmTarget.JVM_21"),
        "build.gradle.kts must target JDK 21; got:\n{}",
        files[0].content
    );
    assert_eq!(files[1].path, PathBuf::from("packages/kotlin/settings.gradle.kts"));
    assert_eq!(files[2].path, PathBuf::from("packages/kotlin/.gitignore"));
    assert_eq!(files[3].path, PathBuf::from("packages/kotlin/.editorconfig"));
    assert!(files[3].content.contains("*.kt"));
    assert_eq!(files[4].path, PathBuf::from("packages/kotlin/gradle.properties"));
    assert!(files[4].content.contains("org.gradle.parallel=true"));
    assert_eq!(files[5].path, PathBuf::from("packages/kotlin/README.md"));
    assert!(files[5].content.contains("my_lib"));
    assert!(files[5].content.contains(":my-lib-kotlin:0.1.0"));
    assert!(files[5].content.contains("gradle build"));
    assert_eq!(
        files[6].path,
        PathBuf::from("packages/kotlin/src/main/kotlin/com/github/test/sample/Sample.kt")
    );
    assert!(files[6].content.contains("object"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Kotlin scaffold must not emit GitHub workflows"
    );
    assert!(
        files[0].content.contains("native.lib.path") && !files[0].content.contains("kb.lib.path"),
        "Kotlin scaffold must use generic native.lib.path override; got:\n{}",
        files[0].content
    );
}

#[test]
fn test_scaffold_kotlin_scm_uses_configured_non_github_host() {
    let config = minimal_config_from_toml(
        r#"
[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://gitlab.example.com/acme/my-lib"
authors = ["Alice"]
keywords = ["test"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    let build_gradle = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin/build.gradle.kts"))
        .expect("build.gradle.kts must be emitted");

    assert!(
        build_gradle
            .content
            .contains("scm:git:git://gitlab.example.com/acme/my-lib.git")
    );
    assert!(
        build_gradle
            .content
            .contains("scm:git:ssh://git@gitlab.example.com/acme/my-lib.git")
    );
    assert!(!build_gradle.content.contains("github.com/acme/my-lib"));
}

#[test]
fn test_scaffold_kotlin_android_mode_returns_helpful_error() {
    let config = test_config_from_toml(
        r#"
[crates.kotlin]
mode = "android"
"#,
    );
    let api = test_api();
    let err =
        scaffold(&api, &config, &[Language::Kotlin]).expect_err("scaffold must reject deprecated kotlin android mode");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("kotlin_android"),
        "error must point at the new Language::KotlinAndroid slug; got: {msg}"
    );
}

#[test]
fn test_scaffold_kotlin_native_target() {
    let config = test_config_from_toml(
        r#"
[crates.kotlin]
target = "native"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 5, "Expected 5 files for Kotlin Native scaffold");
    let build_gradle = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin-native/build.gradle.kts"))
        .unwrap();
    assert!(build_gradle.content.contains(r#"kotlin("multiplatform")"#));
    assert!(build_gradle.content.contains("linuxX64"));
    let def_file = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin-native/my-lib.def"))
        .unwrap();
    assert!(def_file.content.contains("headers = my_lib.h"));
    assert!(
        def_file
            .content
            .contains("linkerOpts = -L../../../target/release -lmy_lib")
    );
}

#[test]
fn test_scaffold_kotlin_multiplatform_mode() {
    let config = test_config_from_toml(
        r#"
[crates.kotlin]
mode = "kmp"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 5, "Expected 5 files for Kotlin Multiplatform scaffold");
    let build_gradle = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin-mpp/build.gradle.kts"))
        .unwrap();
    assert!(build_gradle.content.contains(r#"kotlin("multiplatform")"#));
    assert!(build_gradle.content.contains("jvm()"));
    assert!(build_gradle.content.contains("linuxX64"));
    assert!(build_gradle.content.contains("macosArm64"));
    assert!(
        files
            .iter()
            .any(|f| f.path == Path::new("packages/kotlin-mpp/my-lib.def")),
        "KMP scaffold must include cinterop .def file"
    );
}

#[test]
fn test_scaffold_gleam() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Gleam]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 7, "Expected 7 files for Gleam scaffold");

    let gleam_toml = &files[0];
    assert_eq!(gleam_toml.path, PathBuf::from("packages/gleam/gleam.toml"));
    assert!(
        gleam_toml.content.contains("description"),
        "gleam.toml should include description"
    );
    assert!(
        gleam_toml.content.contains("licences = [\"MIT\"]"),
        "gleam.toml should include licences"
    );

    let manifest = &files[1];
    assert_eq!(manifest.path, PathBuf::from("packages/gleam/manifest.toml"));

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/gleam/.gitignore"));
    assert!(gitignore.content.contains("build/"));

    assert!(files[3].path.to_string_lossy().ends_with("_test.gleam"));

    let editorconfig = &files[4];
    assert_eq!(editorconfig.path, PathBuf::from("packages/gleam/.editorconfig"));
    assert!(editorconfig.content.contains("*.gleam"));

    let readme = &files[5];
    assert_eq!(readme.path, PathBuf::from("packages/gleam/README.md"));
    assert!(readme.content.contains("gleam build"));

    assert!(files[6].path.to_string_lossy().ends_with("_example.gleam"));
    assert!(files[6].content.contains("Nil"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Gleam scaffold must not emit GitHub workflows"
    );
}

#[test]
fn test_scaffold_gleam_uses_configured_license_and_no_fake_github_dependency() {
    let config = minimal_config_from_toml(
        r#"
[crates.scaffold]
description = "Test library"
license = "Apache-2.0"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Gleam]).unwrap();
    let files = language_files(&all_files);
    let gleam_toml = files
        .iter()
        .find(|f| f.path == Path::new("packages/gleam/gleam.toml"))
        .expect("gleam.toml must be emitted");
    let readme = files
        .iter()
        .find(|f| f.path == Path::new("packages/gleam/README.md"))
        .expect("README.md must be emitted");

    assert!(gleam_toml.content.contains("licences = [\"Apache-2.0\"]"));
    assert!(
        !readme.content.contains("github = \"example/"),
        "Gleam README must not invent GitHub dependency metadata:\n{}",
        readme.content
    );
    assert!(readme.content.contains("{path = \"../packages/gleam\"}"));
}

/// The shared [`test_api`] fixture is completely empty — no functions, types, or enums — so every
/// Zig scaffold assertion written against it was exercising the empty-surface path, and the seed
/// it produced was the vacuous "module imports successfully" fallback. `zig ast-check` passed on
/// that fallback, which is why nothing here ever noticed. These tests need a surface with one
/// real, visible item so they cover the path a consumer repo actually takes. ~keep
fn zig_test_api() -> crate::core::ir::ApiSurface {
    crate::core::ir::ApiSurface {
        functions: vec![crate::core::ir::FunctionDef {
            name: "ping".to_string(),
            return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            ..Default::default()
        }],
        ..test_api()
    }
}

#[test]
fn test_scaffold_zig() {
    let config = test_config();
    let api = zig_test_api();
    let all_files = scaffold(&api, &config, &[Language::Zig]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 8, "Expected 8 files for Zig scaffold");

    let build_zig = &files[0];
    assert_eq!(build_zig.path, PathBuf::from("packages/zig/build.zig"));
    assert!(build_zig.content.contains("addModule"));
    assert!(
        build_zig
            .content
            .contains(r#".root_source_file = b.path("test/my_lib_test.zig")"#),
        "test_module must point at the seeded test file, not the production source; got: {}",
        build_zig.content
    );
    assert!(
        build_zig.content.contains(r#"test_module.addImport("my_lib", module)"#),
        "test_module must be able to import the package under test; got: {}",
        build_zig.content
    );
    assert!(
        build_zig.content.contains(r#"b.step("example", "Run the example")"#),
        "build.zig must compile examples/example.zig via an example step; got: {}",
        build_zig.content
    );

    let build_zig_zon = &files[1];
    assert_eq!(build_zig_zon.path, PathBuf::from("packages/zig/build.zig.zon"));
    assert!(build_zig_zon.content.contains(".fingerprint"));

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/zig/.gitignore"));
    assert!(gitignore.content.contains("zig-cache/"));

    let editorconfig = &files[3];
    assert_eq!(editorconfig.path, PathBuf::from("packages/zig/.editorconfig"));
    assert!(editorconfig.content.contains("*.zig"));

    let readme = &files[4];
    assert_eq!(readme.path, PathBuf::from("packages/zig/README.md"));
    assert!(readme.content.contains("zig build"));

    let test_seed = &files[5];
    assert_eq!(test_seed.path, PathBuf::from("packages/zig/test/my_lib_test.zig"));
    assert!(
        test_seed.content.contains("test \""),
        "seed must contain a real `test` block, not zero — that is the defect being fixed; got: {}",
        test_seed.content
    );

    let example = &files[6];
    assert_eq!(example.path, PathBuf::from("packages/zig/examples/example.zig"));
    assert!(example.content.contains("pub fn main"));

    let main = &files[7];
    assert_eq!(main.path, PathBuf::from("packages/zig/src/main.zig"));
    assert!(main.content.contains("pub const api"));
    assert!(main.content.contains(".zig"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Zig scaffold must not emit GitHub workflows"
    );
}

/// End-to-end counterpart through `scaffold()`: with nothing in the API surface to assert
/// against, the seed and the `test` step that runs it are both absent. `zig build test` then
/// fails with `error: no step named 'test'` instead of exiting 0 on zero test blocks — the whole
/// point of the gate, since a vacuous pass and a real one are the same terminal event.
#[test]
fn scaffold_zig_emits_no_test_step_for_an_empty_api_surface() {
    let all_files = scaffold(&test_api(), &test_config(), &[Language::Zig]).unwrap();
    let files = language_files(&all_files);

    assert_eq!(files.len(), 7, "the test seed must be absent, leaving 7 files");
    assert!(
        !files
            .iter()
            .any(|f| f.path == std::path::Path::new("packages/zig/test/my_lib_test.zig")),
        "no seed may be written when there is nothing to assert against"
    );
    let build_zig = &files[0];
    assert_eq!(build_zig.path, PathBuf::from("packages/zig/build.zig"));
    assert!(
        !build_zig.content.contains(r#"b.step("test""#),
        "build.zig must declare no test step; got: {}",
        build_zig.content
    );
    assert!(
        build_zig.content.contains(r#"b.step("example", "Run the example")"#),
        "the example step must survive; got: {}",
        build_zig.content
    );
}

#[test]
fn scaffold_zig_example_passes_zig_ast_check() {
    let files = scaffold(&test_api(), &test_config(), &[Language::Zig]).unwrap();
    let example = files
        .iter()
        .find(|file| file.path == Path::new("packages/zig/examples/example.zig"))
        .expect("Zig scaffold must emit an example");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("example.zig");
    std::fs::write(&path, &example.content).unwrap();
    let output = std::process::Command::new("zig")
        .arg("ast-check")
        .arg(path)
        .current_dir(dir.path())
        .output()
        .expect("Zig must be installed to verify scaffold compatibility");

    assert!(
        output.status.success(),
        "zig ast-check failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn scaffold_zig_test_seed_passes_zig_ast_check() {
    let files = scaffold(&zig_test_api(), &test_config(), &[Language::Zig]).unwrap();
    let test_seed = files
        .iter()
        .find(|file| file.path == Path::new("packages/zig/test/my_lib_test.zig"))
        .expect("Zig scaffold must emit a test seed");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("my_lib_test.zig");
    std::fs::write(&path, &test_seed.content).unwrap();
    let output = std::process::Command::new("zig")
        .arg("ast-check")
        .arg(path)
        .current_dir(dir.path())
        .output()
        .expect("Zig must be installed to verify scaffold compatibility");

    assert!(
        output.status.success(),
        "zig ast-check failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Regression for #555: see `should_not_emit_placeholder_readme_when_readme_module_configures_swift`.
#[test]
fn should_not_emit_placeholder_readme_when_readme_module_configures_zig() {
    let config = test_config_from_toml(
        r#"
[crates.readme.languages.zig]
template = "language_package.md"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Zig]).unwrap();
    let files = language_files(&all_files);
    assert!(
        files.iter().all(|f| f.path != Path::new("packages/zig/README.md")),
        "scaffold must not emit packages/zig/README.md once the README module is \
         configured for zig (#555)"
    );
}

#[test]
fn test_scaffold_zig_uses_configured_ffi_output_path() {
    let mut config = test_config();
    config.explicit_output.ffi = Some("crates/sample-native-ffi/src".into());
    let all_files = scaffold(&test_api(), &config, &[Language::Zig]).unwrap();
    let files = language_files(&all_files);
    let build_zig = &files[0].content;

    assert!(build_zig.contains("../../crates/sample-native-ffi/include"));
    assert!(!build_zig.contains("my-lib-ffi/include"));
}
