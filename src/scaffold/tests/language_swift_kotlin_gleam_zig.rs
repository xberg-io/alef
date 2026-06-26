use super::*;

#[test]
fn test_scaffold_swift() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Swift]).unwrap();
    let files = language_files(&all_files);
    // Original 6 + root Package.swift + .editorconfig + .swiftformat + README.md + Examples/Demo/main.swift = 11
    assert_eq!(
        files.len(),
        11,
        "Expected 11 files for Swift scaffold (original 6 + root Package.swift + 4 extras)"
    );

    let package_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Package.swift"))
        .unwrap();
    // Module name derives to PascalCase of "my-lib" → "MyLib"
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
    // Must declare RustBridge and RustBridgeC targets
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
    // RustBridge target must exist with unsafeFlags for in-tree development
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
    // The FFI dylib's install_name is @rpath/lib...dylib, so the manifest must emit a runtime
    // rpath (not just `-L` compile-time search) or `swift test` fails to dlopen the library.
    // The rpath path is resolved absolutely from the manifest location via Foundation/#filePath.
    assert!(
        package_swift.content.contains("import Foundation"),
        "Package.swift must import Foundation to resolve the absolute rpath; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("-Wl,-rpath,"),
        "Package.swift must emit a runtime -Wl,-rpath so the FFI dylib loads at runtime; got: {}",
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

    // RustBridgeC placeholder header (pure C target)
    let header = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridgeC/RustBridgeC.h"))
        .unwrap();
    assert!(
        header.content.contains("#ifndef RUST_BRIDGE_C_H"),
        "got: {}",
        header.content
    );

    // module.modulemap in RustBridge (kept as documentation comment)
    let modulemap = files.iter().find(|f| f.path.ends_with("module.modulemap")).unwrap();
    assert!(!modulemap.content.is_empty(), "module.modulemap must not be empty");

    // RustBridge placeholder Swift source
    let rust_bridge_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridge/RustBridge.swift"))
        .unwrap();
    assert!(
        !rust_bridge_swift.content.is_empty(),
        "RustBridge.swift must not be empty"
    );

    // Check for new production files
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
    // .editorconfig and .swiftformat must both declare 2-space indent to match
    // `swift-format` defaults, so editors and the formatter stay in sync.
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

    // Package.swift must use 2-space indentation — `swift-format` rewrites 4-space to 2.
    assert!(
        package_swift.content.contains("\n  name:"),
        "Package.swift must use 2-space indentation; got: {}",
        package_swift.content
    );
    // Single-element products array must not have a trailing comma (swift-format removes it).
    assert!(
        !package_swift
            .content
            .contains(".library(name: \"MyLib\", targets: [\"MyLib\"]),"),
        "Package.swift single-element products array must not have trailing comma; got: {}",
        package_swift.content
    );

    // Test stub must emit a blank line between import groups (swift-format requirement).
    let test_stub = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Tests") && f.path.extension().is_some_and(|e| e == "swift"))
        .expect("test stub .swift should be generated");
    assert!(
        test_stub.content.contains("import XCTest\n\n@testable"),
        "test stub must have blank line between import groups; got: {}",
        test_stub.content
    );

    // Demo must use 2-space indentation.
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

#[test]
fn test_scaffold_kotlin() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    // build.gradle.kts, settings.gradle.kts, .gitignore, .editorconfig, gradle.properties, README.md, Sample.kt
    assert_eq!(files.len(), 7, "Expected 7 files for Kotlin scaffold");
    assert_eq!(files[0].path, PathBuf::from("packages/kotlin/build.gradle.kts"));
    assert!(files[0].content.contains("kotlin(\"jvm\")"));
    assert!(files[0].content.contains("org.jlleitschuh.gradle.ktlint"));
    // jspecify is required by the alef-emitted Java facade.
    assert!(
        files[0].content.contains("org.jspecify:jspecify:"),
        "build.gradle.kts must declare jspecify; got:\n{}",
        files[0].content
    );
    // ktlint must skip the Java facade and build/generated dirs.
    assert!(
        files[0].content.contains("filter {")
            && files[0].content.contains("/packages/java/")
            && files[0].content.contains("**/build/**")
            && files[0].content.contains("**/generated/**"),
        "ktlint filter block missing or incomplete; got:\n{}",
        files[0].content
    );
    // ktlint must skip the alef-emitted binding-class file (pascal-cased crate name).
    // The `my-lib` test crate becomes `MyLib.kt`.
    assert!(
        files[0].content.contains(r#"endsWith("/MyLib.kt")"#),
        "ktlint filter must exclude alef-emitted binding-class file; got:\n{}",
        files[0].content
    );
    // Maven artifactId override disambiguates Kotlin module from sibling Java module.
    assert!(
        files[0].content.contains("artifactId = \"my-lib-kotlin\""),
        "publication artifactId override missing; got:\n{}",
        files[0].content
    );
    // Kotlin/JVM targets JDK 21 (KOTLIN_JVM_TARGET); JDK 25 is reserved for
    // the Java/Panama backend via JAVA_JVM_TARGET.
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
    // `mode = "android"` was removed in alef 0.16. Scaffolding must surface
    // a clear migration message rather than silently fall back.
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
    // gleam.toml + manifest.toml + .gitignore + test + .editorconfig + README.md + example
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

#[test]
fn test_scaffold_zig() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Zig]).unwrap();
    let files = language_files(&all_files);
    // build.zig + build.zig.zon + .gitignore + .editorconfig + README.md + example.zig + main.zig (re-export stub)
    assert_eq!(files.len(), 7, "Expected 7 files for Zig scaffold");

    let build_zig = &files[0];
    assert_eq!(build_zig.path, PathBuf::from("packages/zig/build.zig"));
    assert!(build_zig.content.contains("addModule"));

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

    let example = &files[5];
    assert_eq!(example.path, PathBuf::from("packages/zig/examples/example.zig"));
    assert!(example.content.contains("pub fn main"));

    let main = &files[6];
    assert_eq!(main.path, PathBuf::from("packages/zig/src/main.zig"));
    assert!(main.content.contains("pub const api"));
    assert!(main.content.contains(".zig"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Zig scaffold must not emit GitHub workflows"
    );
}

// ---------------------------------------------------------------------------
// `[scaffold.cargo]` workspace `.cargo/config.toml` rendering tests.
// ---------------------------------------------------------------------------
