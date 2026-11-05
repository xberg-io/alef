#![allow(clippy::print_stdout)]

use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../../tests/fixtures/swift_owned_values/core.rs.txt");

pub(super) fn write(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub(super) fn fixture(root: &Path) -> (ApiSurface, ResolvedCrateConfig) {
    write(root, "src/lib.rs", SOURCE);
    let source = root.join("src/lib.rs");
    let api = crate::extract::extractor::extract(&[source.as_path()], "sample_core", "0.0.0", None).unwrap();
    assert!(
        api.types
            .iter()
            .any(|ty| ty.name == "Container" && ty.fields.len() == 5)
    );
    assert!(api.types.iter().any(|ty| ty.name == "Factory" && ty.is_opaque));
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "sample_core"
sources = ["src/lib.rs"]
[crates.swift.client_constructor_body]
Factory = "Ok::<_, String>(Factory(sample_core::Factory::default()))"
Entry = "Ok::<_, String>(Entry(sample_core::sample_entry()))"
"#,
    )
    .unwrap();
    (api, config.resolve().unwrap().remove(0))
}

fn suite(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    use crate::e2e::codegen::{E2eCodegen, swift::SwiftE2eCodegen};
    use crate::e2e::config::{CallConfig, E2eConfig};
    use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};
    let assertions = [
        ("count_equals", "entries", serde_json::json!(2)),
        ("not_empty", "entries", serde_json::Value::Null),
        ("count_min", "entries", serde_json::json!(2)),
        ("count_equals", "labels", serde_json::json!(2)),
        ("equals", "labels[0]", serde_json::json!("alpha")),
        ("equals", "labels[1]", serde_json::json!("beta")),
    ]
    .into_iter()
    .map(|(kind, field, value)| Assertion {
        assertion_type: kind.into(),
        field: Some(field.into()),
        value: if value.is_null() { None } else { Some(value) },
        ..Default::default()
    })
    .collect();
    let e2e = E2eConfig {
        call: CallConfig {
            function: "sample_container".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    SwiftE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "owned".into(),
                fixtures: vec![Fixture {
                    id: "owned_values".into(),
                    assertions,
                    ..Default::default()
                }],
            }],
            &e2e,
            config,
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
        )
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("OwnedTests.swift"))
        .unwrap()
        .content
}

#[test]
fn full_generator_uses_owned_arrays_for_real_producer_fields_and_returns() {
    let root = tempfile::tempdir().unwrap();
    let (api, config) = fixture(root.path());
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    let accessors = files
        .iter()
        .find(|file| file.path.ends_with("ZSwiftOwnedVecAccessors.swift"))
        .unwrap();
    for marker in [
        "public func entries() -> [Entry]",
        "public func labels() -> [RustString]",
        "public func asyncEntries() async",
        "public func fallibleEntries(_ fail: Bool) throws",
    ] {
        assert!(
            accessors.content.contains(marker),
            "missing {marker}: {}",
            accessors.content
        );
    }
    let rust = files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap();
    assert!(rust.content.contains("fn maybe_entries(&self) -> String"));
    assert!(rust.content.contains("fn maybe_labels(&self) -> String"));
    let facade = files
        .iter()
        .find(|file| file.path.ends_with("Sources/SampleCore/SampleCore.swift"))
        .unwrap();
    assert!(
        facade
            .content
            .contains("return Entry(try alefBridgedCall { try RustBridge.entryFromJson(json) })"),
        "{}",
        facade.content
    );
    assert!(!facade.content.contains("isOwned = false"), "{}", facade.content);
    let suite = suite(&api, &config);
    assert!(!suite.contains("skipped:"), "{suite}");
    assert!(suite.contains(".count"), "{suite}");
    assert_eq!(suite.matches("XCTAssertEqual(").count(), 4, "{suite}");
    assert!(suite.contains("XCTAssertGreaterThanOrEqual"), "{suite}");
    assert_eq!(suite.matches("].toString()").count(), 2, "{suite}");
}

#[test]
fn full_generator_keeps_dto_string_array_elements_native() {
    let root = tempfile::tempdir().unwrap();
    let (_, config) = fixture(root.path());
    write(
        root.path(),
        "src/lib.rs",
        r#"
        #[derive(serde::Serialize, serde::Deserialize)]
        pub struct Container { pub entries: Vec<u32>, pub labels: Vec<String> }
        pub fn sample_container() -> Container {
            Container { entries: vec![1, 2], labels: vec!["alpha".into(), "beta".into()] }
        }
    "#,
    );
    let source = root.path().join("src/lib.rs");
    let api = crate::extract::extractor::extract(&[source.as_path()], "sample_core", "0.0.0", None).unwrap();
    let suite = suite(&api, &config);
    assert!(suite.contains("XCTAssertEqual(result.labels[0], \"alpha\")"), "{suite}");
    assert!(suite.contains("XCTAssertEqual(result.labels[1], \"beta\")"), "{suite}");
    assert!(!suite.contains(".toString()"), "{suite}");
    assert!(!suite.contains("skipped:"), "{suite}");
}

fn native_target(root: &Path) -> PathBuf {
    let Some(value) = std::env::var_os("ALEF_SWIFT_OWNED_NATIVE_TARGET") else {
        return root.join("native-target");
    };
    let target = PathBuf::from(value);
    assert!(target.is_absolute() && target.is_dir());
    let link = root.join("native-target");
    if !link.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        panic!("native cache override requires Unix symlinks");
    }
    assert_eq!(link.canonicalize().unwrap(), target.canonicalize().unwrap());
    target
}

pub(super) fn run(
    root: &Path,
    task: &str,
    case: &str,
    materialize: bool,
) -> crate::snippets::validators::CapturedStreams {
    let mut command = crate::test_support::mise_task::mise_run(root, task);
    command
        .env("CARGO_TARGET_DIR", native_target(root))
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("ALEF_OWNED_VALUES_CASE", case)
        .env("TEST_BINARY", std::env::current_exe().unwrap());
    if let Some(compiler) = std::env::var_os("SWIFT_EXEC") {
        let compiler = PathBuf::from(compiler);
        assert!(
            compiler.is_absolute() && compiler.is_file(),
            "explicit Swift compiler must exist"
        );
        command.env("SWIFT_EXEC", compiler);
    }
    if materialize {
        command.env("ALEF_SWIFT_OWNED_MATERIALIZE", root);
    }
    crate::snippets::validators::run_command_streams(&mut command, 600).unwrap()
}

pub(super) fn prepare(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) {
    for file in SwiftBackend
        .generate_bindings(api, config)
        .unwrap()
        .into_iter()
        .chain(crate::backends::ffi::FfiBackend.generate_bindings(api, config).unwrap())
        .chain(crate::scaffold::languages::scaffold_ffi(api, config).unwrap())
    {
        write(root, file.path.to_str().unwrap(), &file.content);
    }
    for (name, content) in [
        (
            "Cargo.toml",
            include_str!("../../../../tests/fixtures/swift_owned_values/core_Cargo.toml"),
        ),
        (
            "mise.toml",
            include_str!("../../../../tests/fixtures/swift_owned_values/mise.toml"),
        ),
        (
            "Package.swift",
            include_str!("../../../../tests/fixtures/swift_owned_values/Package.swift"),
        ),
        (
            "probe/main.swift",
            include_str!("../../../../tests/fixtures/swift_owned_values/main.swift"),
        ),
        (
            "packages/swift/Sources/RustBridgeC/module.modulemap",
            include_str!("../../../../tests/fixtures/swift_owned_values/module.modulemap"),
        ),
    ] {
        write(root, name, content);
    }
    write(root, "tests/OwnedTests.swift", &suite(api, config));
}

#[test]
#[ignore = "requires SwiftPM and compiling the actual Rust bridge"]
fn native_owned_values_outlive_containers() {
    if let Some(root) = std::env::var_os("ALEF_SWIFT_OWNED_MATERIALIZE") {
        crate::backends::swift::gen_bindings::bridge_artifacts::materialize_from_build_output(
            "sample_core-swift",
            Path::new("packages/swift"),
            Path::new(&root),
        )
        .unwrap();
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("alef-693-native-")
        .tempdir()
        .unwrap()
        .keep();
    let (api, config) = fixture(&root);
    prepare(&root, &api, &config);
    for task in ["rust", "materialize", "swift", "suite"] {
        let output = run(&root, task, "", task == "materialize");
        assert!(output.success, "{root:?} {task}: {}\n{}", output.stdout, output.stderr);
        if task == "swift" {
            assert!(
                output.stdout.contains("owned_swift_native_cases=23"),
                "{}",
                output.stdout
            );
        }
        if task == "suite" {
            assert!(output.stdout.contains("Executed 1 test"), "{}", output.stdout);
        }
    }
    for case in ["count", "first", "second", "label"] {
        let output = run(&root, "suite-replay", case, false);
        assert!(
            !output.success && output.stdout.contains("XCTAssertEqual failed"),
            "negative {case}: {}\n{}",
            output.stdout,
            output.stderr
        );
    }
    verify_native_abi(&root);
    verify_old_suffix_rejected(&root);
    verify_borrowed_shape_rejected(&root);
    println!(
        "owned_swift_native_cases=23 generated_xctest_cases=5 assertions_per_case=6 old_shape_compile_negatives=2 old_suffix_compile_negatives=1 native_abi_symbols=4 artifacts={}",
        root.display()
    );
}

fn verify_borrowed_shape_rejected(root: &Path) {
    let path = root.join("packages/swift/Sources/RustBridge/ZSwiftOwnedVecAccessors.swift");
    let owned = std::fs::read_to_string(&path).unwrap();
    for (method, signature, borrowed_signature, call, borrowed_call) in [
        (
            "entries",
            "public func entries() -> [Entry]",
            "public func entries() -> RustVec<Entry>",
            "self.__alef_owned_vec_entries().alefIntoOwnedArray()",
            "self.__alef_owned_vec_entries()",
        ),
        (
            "labels",
            "public func labels() -> [RustString]",
            "public func labels() -> RustVec<RustString>",
            "self.__alef_owned_vec_labels().alefIntoOwnedArray()",
            "self.__alef_owned_vec_labels()",
        ),
    ] {
        assert!(
            owned.contains(signature) && owned.contains(call),
            "old-shape control did not match {method}"
        );
        let old = owned
            .replace(signature, borrowed_signature)
            .replace(call, borrowed_call);
        std::fs::write(&path, &old).unwrap();
        write(root, &format!("proofs/old-{method}.swift"), &old);
        let output = run(root, "swift-build", "", false);
        let log = format!("{}\n{}", output.stdout, output.stderr);
        write(root, &format!("proofs/old-{method}.log"), &log);
        let expected_error = if method == "labels" {
            "value of type 'RustStringRef' has no member 'toString'"
        } else {
            "cannot convert return expression"
        };
        assert!(
            !output.success && log.contains(expected_error) && log.contains("main.swift"),
            "old {method} shape: {log}"
        );
    }
    std::fs::write(path, owned).unwrap();
    let output = run(root, "swift", "", false);
    assert!(
        output.success,
        "restored owned values: {}\n{}",
        output.stdout, output.stderr
    );
}

fn verify_old_suffix_rejected(root: &Path) {
    let directory = root.join("packages/swift/Sources/RustBridge");
    let originals: Vec<_> = ["sample_core-swift.swift", "ZSwiftOwnedVecAccessors.swift"]
        .into_iter()
        .map(|name| {
            let path = directory.join(name);
            let source = std::fs::read_to_string(&path).unwrap();
            (path, source)
        })
        .collect();
    let mut replacements = 0;
    for (path, source) in &originals {
        let mut old = source.clone();
        for (internal, suffix) in [
            ("__alef_owned_vec_sample_labels", "sampleLabelsRustVec"),
            ("__alef_owned_vec_labels", "labelsRustVec"),
        ] {
            replacements += old.matches(internal).count();
            old = old.replace(internal, suffix);
        }
        std::fs::write(path, &old).unwrap();
        write(
            root,
            &format!("proofs/old-suffix-{}", path.file_name().unwrap().to_str().unwrap()),
            &old,
        );
    }
    assert_eq!(replacements, 4, "declarations and wrapper calls must both change");
    let output = run(root, "swift-build", "", false);
    let log = format!("{}\n{}", output.stdout, output.stderr);
    write(root, "proofs/old-suffix.log", &log);
    assert!(
        !output.success && log.contains("ambiguous use") && log.contains("sampleLabelsRustVec"),
        "{log}"
    );
    for (path, source) in originals {
        std::fs::write(path, source).unwrap();
    }
    let output = run(root, "swift", "", false);
    assert!(
        output.success,
        "restored reserved aliases: {}\n{}",
        output.stdout, output.stderr
    );
}

fn verify_native_abi(root: &Path) {
    let header = std::fs::read_to_string(root.join("packages/swift/Sources/RustBridgeC/RustBridgeC.h")).unwrap();
    for name in [
        "__swift_bridge__$Container$labels(",
        "__swift_bridge__$Container$entries(",
        "__swift_bridge__$sample_labels(",
        "__swift_bridge__$sample_labels_rust_vec(",
    ] {
        assert!(
            header.contains(name),
            "missing unchanged native symbol {name}: {header}"
        );
    }
    assert!(
        !header.contains("__alef_owned_vec_"),
        "internal Swift aliases changed native symbols"
    );
}
