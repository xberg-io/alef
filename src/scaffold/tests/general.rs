use super::*;

#[test]
fn test_scaffold_csharp_omits_repository_when_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Csharp]).unwrap();
    let files = language_files(&all_files);
    let csproj = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".csproj"))
        .expect("C# project file must be emitted");

    assert!(
        !csproj.content.contains("<RepositoryUrl>"),
        "unconfigured C# scaffold must not invent repository metadata:\n{}",
        csproj.content
    );
}

#[test]
fn test_scaffold_wasm_omits_repository_when_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let files = language_files(&all_files);
    let package_json = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-wasm/package.json"))
        .expect("WASM package.json must be emitted");
    let parsed: serde_json::Value =
        serde_json::from_str(&package_json.content).expect("emitted package.json must be valid JSON");

    assert!(
        parsed.get("repository").is_none(),
        "unconfigured WASM manifest must not invent repository metadata:\n{}",
        package_json.content
    );
    assert_eq!(parsed["engines"]["node"], ">= 22");
}

fn wasm_package_json(toml: &str) -> serde_json::Value {
    let config = minimal_config_from_toml(toml);
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let files = language_files(&all_files);
    let package_json = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-wasm/package.json"))
        .expect("WASM package.json must be emitted");
    serde_json::from_str(&package_json.content).expect("emitted package.json must be valid JSON")
}

#[test]
fn wasm_targets_default_to_all_four_wasm_pack_targets() {
    let parsed = wasm_package_json("");
    assert_eq!(
        parsed["files"],
        serde_json::json!(["pkg", "*.wasm", "*.d.ts", "README.md"])
    );
    assert_eq!(parsed["main"], "pkg/nodejs/my_lib_wasm.js");
    assert_eq!(parsed["module"], "pkg/web/my_lib_wasm.js");
    assert_eq!(parsed["types"], "pkg/nodejs/my_lib_wasm.d.ts");
    let scripts = &parsed["scripts"];
    for t in ["web", "bundler", "nodejs", "deno"] {
        assert!(
            scripts.get(format!("build:wasm:{t}")).is_some(),
            "missing build:wasm:{t}"
        );
    }
    assert_eq!(
        scripts["build:all"],
        "npm run build:wasm:web && npm run build:wasm:bundler && npm run build:wasm:nodejs && npm run build:wasm:deno && find pkg -name .gitignore -delete"
    );
}

#[test]
fn wasm_targets_web_only_ships_single_target() {
    let parsed = wasm_package_json("[crates.wasm]\ntargets = [\"web\"]\n");
    assert_eq!(parsed["files"], serde_json::json!(["pkg/web", "README.md"]));
    assert_eq!(parsed["main"], "pkg/web/my_lib_wasm.js");
    assert_eq!(parsed["module"], "pkg/web/my_lib_wasm.js");
    assert_eq!(parsed["types"], "pkg/web/my_lib_wasm.d.ts");
    let scripts = &parsed["scripts"];
    assert!(scripts.get("build:wasm:web").is_some());
    for t in ["bundler", "nodejs", "deno"] {
        assert!(
            scripts.get(format!("build:wasm:{t}")).is_none(),
            "unexpected build:wasm:{t}"
        );
    }
    assert_eq!(
        scripts["build:all"],
        "npm run build:wasm:web && find pkg -name .gitignore -delete"
    );
    assert_eq!(scripts["build"], "wasm-pack build --target web --out-dir pkg/web");
}

#[test]
fn wasm_targets_rejects_unknown_target() {
    let config = minimal_config_from_toml("[crates.wasm]\ntargets = [\"webxr\"]\n");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::Wasm]).expect_err("unknown wasm target must error");
    assert!(
        err.to_string().contains("unknown target 'webxr'"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_java_requires_publish_metadata() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::Java]).expect_err("Java scaffold must require publish metadata");

    assert!(
        err.to_string()
            .contains("Java scaffold requires package metadata repository"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_kotlin_requires_publish_metadata() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::Kotlin]).expect_err("Kotlin scaffold must require publish metadata");

    assert!(
        err.to_string()
            .contains("Kotlin scaffold requires package metadata repository"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_r_requires_authors() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::R]).expect_err("R scaffold must require authors");

    assert!(
        err.to_string().contains("R scaffold requires package metadata authors"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_multiple() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 14);
}

#[test]
fn test_scaffold_gitattributes_covers_all_generated_dirs() {
    let config = test_config();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted by scaffold");

    assert!(
        !ga.generated_header,
        "generated_header must be false — create-once seed"
    );

    let content = &ga.content;
    assert!(content.contains("packages/python/**"), "must cover Python package dir");
    assert!(content.contains("crates/my-lib-node/**"), "must cover Node crate dir");
    assert!(content.contains("crates/my-lib-py/**"), "must cover PyO3 binding crate");
    assert!(content.contains("e2e/**"), "must cover e2e test output");
    for line in content.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        assert!(
            line.ends_with("linguist-generated=true"),
            "every non-comment line must set linguist-generated=true, got: {line}"
        );
    }
}

#[test]
fn test_scaffold_gitattributes_ffi_and_jni_use_crate_dirs() {
    let config = test_config();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Ffi, Language::Jni]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    let content = &ga.content;
    assert!(content.contains("crates/my-lib-ffi/**"), "must cover FFI crate dir");
    assert!(content.contains("crates/my-lib-jni/**"), "must cover JNI crate dir");
    assert!(!content.contains("packages/ffi"), "must not emit bogus packages/ffi");
    assert!(!content.contains("packages/jni"), "must not emit bogus packages/jni");
}

#[test]
fn test_scaffold_gitattributes_kotlin_native_uses_kotlin_native_dir() {
    use crate::core::config::NewAlefConfig;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://github.com/test/my-lib"

[crates.kotlin]
target = "native"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-native/**"),
        "native target must use packages/kotlin-native, got:\n{}",
        ga.content
    );
    assert!(
        !ga.content.contains("packages/kotlin/**"),
        "native target must not emit JVM dir, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_gitattributes_kotlin_mpp_uses_kotlin_mpp_dir() {
    use crate::core::config::NewAlefConfig;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://github.com/test/my-lib"

[crates.kotlin]
mode = "kmp"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-mpp/**"),
        "kmp mode must use packages/kotlin-mpp, got:\n{}",
        ga.content
    );
    assert!(
        !ga.content.contains("packages/kotlin/**"),
        "kmp mode must not emit JVM dir, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_gitattributes_kotlin_multiplatform_target_uses_kotlin_mpp_dir() {
    use crate::core::config::NewAlefConfig;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://github.com/test/my-lib"

[crates.kotlin]
target = "multiplatform"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-mpp/**"),
        "target=multiplatform must use packages/kotlin-mpp, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_gitattributes_kotlin_android_uses_kotlin_android_dir() {
    let config = test_config();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::KotlinAndroid]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-android/**"),
        "KotlinAndroid must use packages/kotlin-android, got:\n{}",
        ga.content
    );
}

#[test]
fn wasm_package_name_strips_node_suffix_from_scoped_package() {
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "@scope/foo-node"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    assert!(
        pkg_json.content.contains("\"@scope/foo-wasm\""),
        "expected @scope/foo-wasm, got:\n{}",
        pkg_json.content
    );
    assert!(
        !pkg_json.content.contains("foo-node-wasm"),
        "must not emit foo-node-wasm, got:\n{}",
        pkg_json.content
    );
}

#[test]
fn wasm_package_name_strips_node_suffix_from_unscoped_package() {
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "foo-node"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    assert!(
        pkg_json.content.contains("\"foo-wasm\""),
        "expected foo-wasm, got:\n{}",
        pkg_json.content
    );
}

#[test]
fn wasm_package_name_fallback_when_no_node_suffix() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    assert!(
        pkg_json.content.contains("\"my-lib-wasm\""),
        "expected my-lib-wasm, got:\n{}",
        pkg_json.content
    );
}

#[test]
fn wasm_package_name_uses_explicit_wasm_config() {
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "@scope/foo-node"

[crates.wasm]
package_name = "@scope/foo-web"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    let parsed: serde_json::Value = serde_json::from_str(&pkg_json.content).expect("valid wasm package.json");
    assert_eq!(parsed["name"], "@scope/foo-web");
    assert_eq!(parsed["publishConfig"]["access"], "public");
    assert_eq!(parsed["main"], "pkg/nodejs/my_lib_wasm.js");
    assert_eq!(parsed["types"], "pkg/nodejs/my_lib_wasm.d.ts");
}

#[test]
fn test_scaffold_r_authors_r_parses_name_and_email() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let description = files.iter().find(|f| f.path.ends_with("DESCRIPTION")).unwrap();

    assert!(
        description
            .content
            .contains(r#"Authors@R: person("Ada", "Lovelace", email = "ada@example.com", role = c("aut", "cre"))"#),
        "DESCRIPTION must split Authors@R name/email; content:\n{}",
        description.content
    );
}

/// The extendr backend emits `#[cfg(feature = "X")]` gates on cfg-gated functions
/// into the R crate's `lib.rs`. The generated `packages/r/src/rust/Cargo.toml` must
/// therefore declare a forwarding `[features]` table so those gates do not produce
/// `error: unexpected cfg condition value: X` under `RUSTFLAGS=-D warnings`.
#[test]
fn test_scaffold_r_cargo_forwards_cfg_features() {
    use crate::core::ir::{FunctionDef, TypeRef};

    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]
"#,
    );
    let mut api = test_api();
    api.functions = vec![
        FunctionDef {
            name: "extract_file_sync".to_string(),
            rust_path: "my_lib::extract_file_sync".to_string(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"tokio-runtime\"".to_string()),
            ..Default::default()
        },
        FunctionDef {
            name: "analyze_document".to_string(),
            rust_path: "my_lib::analyze_document".to_string(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"heuristics\"".to_string()),
            ..Default::default()
        },
    ];

    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("packages/r/src/rust/Cargo.toml"))
        .expect("R rust crate Cargo.toml must be emitted");

    assert!(
        cargo.content.contains("[features]"),
        "R Cargo.toml must contain a [features] block when cfg-gated items exist; content:\n{}",
        cargo.content
    );
    assert!(
        cargo.content.contains(r#"tokio-runtime = ["my-lib/tokio-runtime"]"#),
        "R Cargo.toml must forward `tokio-runtime` to the core dep; content:\n{}",
        cargo.content
    );
    assert!(
        cargo.content.contains(r#"heuristics = ["my-lib/heuristics"]"#),
        "R Cargo.toml must forward `heuristics` to the core dep; content:\n{}",
        cargo.content
    );
    let default_line = cargo
        .content
        .lines()
        .find(|l| l.starts_with("default = ["))
        .expect("R Cargo.toml [features] must declare a `default` list");
    assert!(
        default_line.contains("\"tokio-runtime\"") && default_line.contains("\"heuristics\""),
        "default feature list must include every cfg-forwarded feature; got: {default_line}"
    );
    toml::from_str::<toml::Value>(&cargo.content).expect("generated R Cargo.toml must be valid TOML");
}

#[test]
fn test_scaffold_r_cargo_explicit_features_match_wasm_defaults() {
    use crate::core::ir::{FunctionDef, TypeDef, TypeRef};

    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]

[crates.r]
features = ["wasm-target"]
default_features = false
"#,
    );
    let mut api = test_api();
    api.types = vec![TypeDef {
        name: "GatedType".to_string(),
        rust_path: "my_lib::GatedType".to_string(),
        cfg: Some(r#"any(feature = "wasm-target", feature = "extra")"#.to_string()),
        ..Default::default()
    }];
    api.functions = vec![FunctionDef {
        name: "extract".to_string(),
        rust_path: "my_lib::extract".to_string(),
        return_type: TypeRef::String,
        cfg: Some("feature = \"url-ingestion\"".to_string()),
        ..Default::default()
    }];

    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("packages/r/src/rust/Cargo.toml"))
        .expect("R rust crate Cargo.toml must be emitted");

    assert!(
        cargo.content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../crates/my-lib", default-features = false, features = ["wasm-target"] }"#,
        ),
        "R core dependency must disable defaults when explicit features are configured; content:\n{}",
        cargo.content
    );
    assert!(
        cargo.content.contains(r#"wasm-target = ["my-lib/wasm-target"]"#),
        "R Cargo.toml must declare cfg passthrough features; content:\n{}",
        cargo.content
    );
    assert!(
        !cargo.content.contains("default = ["),
        "R Cargo.toml must not auto-enable cfg passthrough features with explicit configured features; content:\n{}",
        cargo.content
    );
    toml::from_str::<toml::Value>(&cargo.content).expect("generated R Cargo.toml must be valid TOML");
}

/// When the API surface has no cfg-gated items the R Cargo.toml must omit the
/// `[features]` block entirely (no empty table).
#[test]
fn test_scaffold_r_cargo_omits_features_block_when_no_cfg() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("packages/r/src/rust/Cargo.toml"))
        .expect("R rust crate Cargo.toml must be emitted");

    assert!(
        !cargo.content.contains("[features]"),
        "R Cargo.toml must not emit a [features] block when no cfg-gated items exist; content:\n{}",
        cargo.content
    );
    toml::from_str::<toml::Value>(&cargo.content).expect("generated R Cargo.toml must be valid TOML");
}

#[test]
fn test_scaffold_r_cargo_no_workspace_inheritance() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("packages/r/src/rust/Cargo.toml"))
        .expect("R rust crate Cargo.toml must be emitted");

    assert!(
        !cargo.content.contains("version.workspace"),
        "R Cargo.toml must NOT use workspace inheritance for version (excluded from workspace); content:\n{}",
        cargo.content
    );
    assert!(
        !cargo.content.contains("license.workspace"),
        "R Cargo.toml must NOT use workspace inheritance for license (excluded from workspace); content:\n{}",
        cargo.content
    );

    assert!(
        cargo.content.contains(r#"version = "0.1.0""#),
        "R Cargo.toml must have concrete version field; content:\n{}",
        cargo.content
    );

    toml::from_str::<toml::Value>(&cargo.content).expect("generated R Cargo.toml must be valid TOML");
}
