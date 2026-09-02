//! xUnit parallelizes test classes within an assembly by default. A generated test class that
//! mutates a process-global trait-bridge registry (`register_fn` / `unregister_fn` / `clear_fn`
//! from a `[[trait_bridges]]` entry -- see [`crate::core::config::trait_bridge::TraitBridgeConfig`])
//! can therefore run concurrently with another class that reads the same registry through the
//! ordinary call path, producing an intermittent failure that never reproduces when the suite
//! runs single-threaded or when only the failing class runs alone.
//!
//! `CSharpCodegen::generate` must detect this from the crate's declared trait bridges -- not from
//! any specific trait or fixture name -- and serialize the whole assembly (`[assembly:
//! CollectionBehavior(DisableTestParallelization = true)]` in `TestSetup.cs`) whenever a fixture
//! that will actually run identifies a registry-mutating call. A crate with no such fixture must
//! not pay the cost: the attribute must be absent.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::CallConfig;
use crate::core::config::trait_bridge::TraitBridgeConfig;
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::csharp::CSharpCodegen;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};

fn config_with_ocr_backend_bridge() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            register_fn: Some("register_ocr_backend".to_string()),
            unregister_fn: Some("unregister_ocr_backend".to_string()),
            clear_fn: Some("clear_ocr_backends".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn e2e_config_with_callable_default() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "Process".to_string(),
            module: "SampleLib".to_string(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

fn assembly_attribute_present(files: &[GeneratedFile]) -> bool {
    let test_setup = files
        .iter()
        .find(|f| f.path.ends_with("TestSetup.cs"))
        .expect("csharp e2e generation must always emit TestSetup.cs");
    test_setup
        .content
        .contains("[assembly: CollectionBehavior(DisableTestParallelization = true)]")
}

#[test]
fn a_fixture_calling_a_declared_clear_fn_disables_assembly_parallelization() {
    let config = config_with_ocr_backend_bridge();
    let e2e_config = e2e_config_with_callable_default();
    let groups = [FixtureGroup {
        category: "ocr_backend_management".to_string(),
        fixtures: vec![Fixture {
            id: "ocr_backends_clear".to_string(),
            description: "Clearing all OCR backends leaves none registered".to_string(),
            call: Some("clear_ocr_backends".to_string()),
            ..Fixture::default()
        }],
    }];

    let files = CSharpCodegen
        .generate(&groups, &e2e_config, &config, &[], &[], &[], &[])
        .expect("csharp e2e generation succeeds for a registry-mutating fixture");

    assert!(
        assembly_attribute_present(&files),
        "a fixture calling a declared `clear_fn` must disable assembly-wide test parallelization \
         so it cannot race a class reading the same registry, got TestSetup.cs:\n{}",
        files
            .iter()
            .find(|f| f.path.ends_with("TestSetup.cs"))
            .map(|f| f.content.as_str())
            .unwrap_or_default()
    );
}

#[test]
fn a_fixture_calling_a_declared_register_fn_also_disables_assembly_parallelization() {
    let config = config_with_ocr_backend_bridge();
    let e2e_config = e2e_config_with_callable_default();
    let groups = [FixtureGroup {
        category: "ocr_backend_management".to_string(),
        fixtures: vec![Fixture {
            id: "ocr_backends_register".to_string(),
            description: "Registering an OCR backend makes it available".to_string(),
            call: Some("register_ocr_backend".to_string()),
            ..Fixture::default()
        }],
    }];

    let files = CSharpCodegen
        .generate(&groups, &e2e_config, &config, &[], &[], &[], &[])
        .expect("csharp e2e generation succeeds for a registry-mutating fixture");

    assert!(
        assembly_attribute_present(&files),
        "a fixture calling a declared `register_fn` must also disable assembly-wide test \
         parallelization, got TestSetup.cs:\n{}",
        files
            .iter()
            .find(|f| f.path.ends_with("TestSetup.cs"))
            .map(|f| f.content.as_str())
            .unwrap_or_default()
    );
}

#[test]
fn fixtures_with_no_registry_mutating_call_keep_parallelization_enabled() {
    let config = config_with_ocr_backend_bridge();
    let e2e_config = e2e_config_with_callable_default();
    let groups = [FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![Fixture {
            id: "ocr_force_all_pages".to_string(),
            description: "Force OCR across every page".to_string(),
            input: serde_json::json!({"ocr": {"backend": "tesseract"}, "force_ocr": true}),
            ..Fixture::default()
        }],
    }];

    let files = CSharpCodegen
        .generate(&groups, &e2e_config, &config, &[], &[], &[], &[])
        .expect("csharp e2e generation succeeds for a fixture set with no registry mutation");

    assert!(
        !assembly_attribute_present(&files),
        "a fixture set with no registry-mutating call must not pay the assembly-wide \
         parallelization cost, got TestSetup.cs:\n{}",
        files
            .iter()
            .find(|f| f.path.ends_with("TestSetup.cs"))
            .map(|f| f.content.as_str())
            .unwrap_or_default()
    );
}

#[test]
fn a_crate_with_no_trait_bridges_never_disables_parallelization() {
    let e2e_config = e2e_config_with_callable_default();
    let groups = [FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![Fixture {
            id: "clear_ocr_backends".to_string(),
            description: "A fixture id that merely LOOKS destructive, with no declared bridge".to_string(),
            ..Fixture::default()
        }],
    }];

    let files = CSharpCodegen
        .generate(
            &groups,
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("csharp e2e generation succeeds when no trait bridge is configured");

    assert!(
        !assembly_attribute_present(&files),
        "detection must be keyed on the configured `[[trait_bridges]]` operations, not on a \
         fixture id or function name that happens to look destructive, got TestSetup.cs:\n{}",
        files
            .iter()
            .find(|f| f.path.ends_with("TestSetup.cs"))
            .map(|f| f.content.as_str())
            .unwrap_or_default()
    );
}
