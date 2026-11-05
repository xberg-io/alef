#![allow(clippy::print_stdout, reason = "native fixture artifact and case receipts")]

use super::owned_values_tests::{fixture, prepare, run, write};
use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use std::path::Path;

fn input_fixture(root: &Path) -> (ApiSurface, ResolvedCrateConfig) {
    let (_, config) = fixture(root);
    let source = root.join("src/lib.rs");
    let mut text = std::fs::read_to_string(&source).unwrap();
    text.push_str(include_str!(
        "../../../../tests/fixtures/swift_owned_inputs/core.rs.txt"
    ));
    write(root, "src/lib.rs", &text);
    let api = crate::extract::extractor::extract(&[source.as_path()], "sample_core", "0.0.0", None).unwrap();
    assert!(
        api.types
            .iter()
            .any(|ty| ty.name == "OwnedInput" && ty.is_clone && !ty.has_serde)
    );
    assert!(
        api.types
            .iter()
            .any(|ty| ty.name == "MovingInput" && !ty.is_clone && !ty.has_serde)
    );
    (api, config)
}

#[test]
fn owned_vector_forwarders_prepare_distinct_native_values() {
    let root = tempfile::tempdir().unwrap();
    let (api, config) = input_fixture(root.path());
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    let facade = &files
        .iter()
        .find(|file| file.path.ends_with("Sources/SampleCore/SampleCore.swift"))
        .unwrap()
        .content;
    assert!(facade.contains("x.__alefOwnedVectorInput()"), "{facade}");
    let rust = &files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    assert!(rust.contains("fn __alef_clone_owned_input"), "{rust}");
    assert_eq!(
        rust.matches("type OwnedInput;").count(),
        1,
        "declare each native owner once"
    );
    assert!(
        !rust.contains("fn __alef_clone_moving_input"),
        "NonClone is never guessed"
    );
    let guards = &files
        .iter()
        .find(|file| file.path.ends_with("ZSwiftOwnedVectorInputs.swift"))
        .unwrap()
        .content;
    assert_eq!(guards.matches("precondition(isOwned").count(), 2);
    assert!(guards.find("precondition(isOwned").unwrap() < guards.find("return self").unwrap());
}

fn prepare_native_fixture(root: &Path) {
    let (api, config) = input_fixture(root);
    prepare(root, &api, &config);
    write(
        root,
        "probe/main.swift",
        include_str!("../../../../tests/fixtures/swift_owned_inputs/main.swift"),
    );
    write(
        root,
        "Package.swift",
        include_str!("../../../../tests/fixtures/swift_owned_inputs/Package.swift"),
    );
    write(
        root,
        "packages/swift/Sources/RustBridge/OwnershipProbe.swift",
        include_str!("../../../../tests/fixtures/swift_owned_inputs/OwnershipProbe.swift"),
    );
    for task in ["rust", "materialize", "swift-build"] {
        let result = run(root, task, "", task == "materialize");
        assert!(result.success, "{task}: {}\n{}", result.stdout, result.stderr);
    }
}

#[test]
fn borrowed_vector_elements_and_strings_keep_existing_lowering() {
    let root = tempfile::tempdir().unwrap();
    let (mut api, config) = input_fixture(root.path());
    for function in &mut api.functions {
        if function.name.starts_with("consume_owned_inputs") {
            function.params[0].vec_inner_is_ref = true;
        }
    }
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    let guards = &files
        .iter()
        .find(|file| file.path.ends_with("ZSwiftOwnedVectorInputs.swift"))
        .unwrap()
        .content;
    assert!(!guards.contains("extension OwnedInput"));
    assert!(guards.contains("extension MovingInput"));
    let facade = &files
        .iter()
        .find(|file| file.path.ends_with("Sources/SampleCore/SampleCore.swift"))
        .unwrap()
        .content;
    assert!(
        facade.contains("v.push(value: RustString(s))"),
        "string vectors stay native"
    );
    let rust = &files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    assert!(!rust.contains("fn __alef_clone_owned_input"));
}

#[test]
#[ignore = "requires SwiftPM and the actual generated native bridge"]
fn owned_vector_inputs_support_repeated_clone_handles_and_checked_moves() {
    let root = tempfile::Builder::new()
        .prefix("alef-owned-inputs-")
        .tempdir()
        .unwrap()
        .keep();
    println!("owned_vector_artifacts={}", root.display());
    prepare_native_fixture(&root);
    for case in [
        "unique",
        "moving-unique",
        "clone-reuse",
        "clone-async-reuse",
        "moving-state",
    ] {
        let result = run(&root, "swift", case, false);
        assert!(
            result.success && result.stdout.contains(&format!("owned_vector_success={case}")),
            "{case}: {}\n{}",
            result.stdout,
            result.stderr
        );
        println!("owned_vector_verified={case}");
    }
}

#[test]
fn dto_and_primitive_vector_inputs_keep_their_existing_conversion() {
    let root = tempfile::tempdir().unwrap();
    let (_, mut config) = fixture(root.path());
    config.swift.as_mut().unwrap().client_constructor_body.clear();
    let path = root.path().join("controls.rs");
    write(
        root.path(),
        "controls.rs",
        r#"
        #[derive(serde::Serialize, serde::Deserialize)]
        pub struct PlainDto { pub value: u32 }
        pub fn consume_dtos(inputs: Vec<PlainDto>) -> u32 { inputs.iter().map(|v| v.value).sum() }
        pub fn sum_numbers(inputs: Vec<u32>) -> u32 { inputs.iter().sum() }
    "#,
    );
    let api = crate::extract::extractor::extract(&[path.as_path()], "sample_core", "0.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 2);
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    assert!(
        !files
            .iter()
            .any(|file| file.path.ends_with("ZSwiftOwnedVectorInputs.swift"))
    );
    let facade = &files
        .iter()
        .find(|file| file.path.ends_with("Sources/SampleCore/SampleCore.swift"))
        .unwrap()
        .content;
    assert!(facade.contains("JSONEncoder().encode(item)"));
    assert!(facade.contains("RustVec<UInt32>"));
}
