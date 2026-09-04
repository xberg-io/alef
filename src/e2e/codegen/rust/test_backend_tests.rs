//! Coverage for `emit_test_backend`: the Rust trait-bridge stub emitted for e2e fixtures.
//!
//! Split out of `rust/mod.rs`, which sits over the repo's 1,000-line cap (see
//! `file-modularization` in CLAUDE.md). ~keep

use super::{emit_test_backend, make_fixture, test_method};

#[test]
fn emit_test_backend_rust_generates_struct_and_arc_expr() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_test_trait".to_string()),
        ..Default::default()
    };

    let m1 = test_method("do_work", TypeRef::String, false, None);
    let m2 = test_method(
        "process_async",
        TypeRef::Named("WorkResult".to_string()),
        true,
        Some("WorkError"),
    );
    let methods = [&m1, &m2];

    let fixture = make_fixture("my_test_fixture", serde_json::json!({ "name": "my-test-backend" }));

    let emission = emit_test_backend(&bridge, &methods, &fixture);

    // setup_block must contain the stub struct and impl.
    assert!(
        emission.setup_block.contains("TestStubMyTestFixture"),
        "setup_block should contain stub name, got: {}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("TestTrait"),
        "setup_block should reference trait by name, got: {}",
        emission.setup_block
    );
    // Must NOT hardcode any sample_core-domain trait name.
    assert!(
        !emission.setup_block.contains("OcrBackend"),
        "setup_block must not hardcode OcrBackend"
    );
    assert!(
        !emission.setup_block.contains("DocumentExtractor"),
        "setup_block must not hardcode DocumentExtractor"
    );

    // name() emitted because super_trait is Some.
    assert!(
        emission.setup_block.contains("fn name("),
        "setup_block should emit name() when super_trait is set"
    );

    // Required methods emitted.
    assert!(
        emission.setup_block.contains("fn do_work("),
        "required method do_work should be in setup_block"
    );
    assert!(
        emission.setup_block.contains("fn process_async("),
        "required async method process_async should be in setup_block"
    );

    // arg_expr wraps in Arc::new.
    assert!(
        emission.arg_expr.contains("Arc::new"),
        "arg_expr should use Arc::new, got: {}",
        emission.arg_expr
    );
    assert!(
        emission.arg_expr.contains("TestStubMyTestFixture"),
        "arg_expr should reference stub struct, got: {}",
        emission.arg_expr
    );
}

#[test]
fn emit_test_backend_rust_skips_default_impl_methods() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        ..Default::default()
    };

    let required = test_method("required_method", TypeRef::String, false, None);
    let mut optional = test_method("optional_method", TypeRef::String, false, None);
    optional.has_default_impl = true;
    let methods = [&required, &optional];

    let fixture = make_fixture("skip_defaults_fixture", serde_json::json!({}));
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission.setup_block.contains("fn required_method("),
        "required method should be emitted"
    );
    assert!(
        !emission.setup_block.contains("fn optional_method("),
        "method with default impl should be skipped"
    );
}

#[test]
fn emit_test_backend_rust_name_extracted_from_input() {
    use crate::core::config::TraitBridgeConfig;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let fixture = make_fixture(
        "name_extraction_fixture",
        serde_json::json!({ "backend": { "name": "extracted-name" } }),
    );

    let emission = emit_test_backend(&bridge, &[], &fixture);

    assert!(
        emission.arg_expr.contains("extracted-name"),
        "arg_expr should contain the name from input.backend.name, got: {}",
        emission.arg_expr
    );
}

/// The stub's `Result<_, E>` and its `use` import both come from the method's own
/// `error_type`. Emitting any other `*Error` the crate happens to declare produces an
/// unresolvable import (E0432) because module-private error types are not re-exported.
#[test]
fn emit_test_backend_rust_pins_error_type_to_the_method_signature() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let method = test_method(
        "embed",
        TypeRef::Vec(Box::new(TypeRef::String)),
        true,
        Some("SampleCrateError"),
    );
    let methods = [&method];

    let fixture = make_fixture("error_type_fixture", serde_json::json!({ "name": "backend" }));
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission
            .setup_block
            .contains("async fn embed(&self) -> Result<Vec<String>, SampleCrateError>"),
        "stub signature must use the method's declared error type, got: {}",
        emission.setup_block
    );
    assert!(
        emission.type_imports.contains(&"SampleCrateError".to_string()),
        "the declared error type must be imported, got: {:?}",
        emission.type_imports
    );
    assert_eq!(
        emission
            .type_imports
            .iter()
            .filter(|import| import.ends_with("Error"))
            .collect::<Vec<_>>(),
        vec![&"SampleCrateError".to_string()],
        "no error type other than the declared one may be imported, got: {:?}",
        emission.type_imports
    );
}

/// A single-argument `Result<T>` alias reaches the emitter as the `anyhow::Error` sentinel and
/// must render as the crate's own `Result` alias, never as a bare `anyhow::Error` import.
#[test]
fn emit_test_backend_rust_renders_alias_result_through_the_crate_module() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("sample_core::plugins::Plugin".to_string()),
        ..Default::default()
    };

    let method = test_method("validate", TypeRef::Unit, true, Some("anyhow::Error"));
    let methods = [&method];

    let fixture = make_fixture("alias_result_fixture", serde_json::json!({ "name": "backend" }));
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission
            .setup_block
            .contains("async fn validate(&self) -> sample_core::Result<()>"),
        "single-arg Result alias must render through the crate module, got: {}",
        emission.setup_block
    );
    assert!(
        !emission.type_imports.iter().any(|import| import.contains("Error")),
        "the alias sentinel must not become an error-type import, got: {:?}",
        emission.type_imports
    );
}

/// A stub method returning a non-boolean integer primitive must return a
/// non-degenerate literal (`1`), not the type's zero — a caller that
/// validates its inputs (e.g. rejecting a zero-valued count) would otherwise
/// reject the stub itself, never exercising the real registration path.
#[test]
fn emit_test_backend_rust_integer_return_is_nonzero() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{PrimitiveType, TypeRef};

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        ..Default::default()
    };

    let method = test_method("count", TypeRef::Primitive(PrimitiveType::Usize), false, None);
    let methods = [&method];

    let fixture = make_fixture("integer_return_fixture", serde_json::json!({ "name": "backend" }));
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission.setup_block.contains("fn count(&self) -> usize { 1 }"),
        "integer-returning stub method must return 1, got: {}",
        emission.setup_block
    );
}

/// A stub method returning a collection keeps today's empty-collection
/// default — only the integer-primitive case is degenerate enough to reject
/// a validating caller, so this pins the collection behavior against a
/// future change accidentally widening the non-degenerate-default fix.
#[test]
fn emit_test_backend_rust_collection_return_stays_empty() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        ..Default::default()
    };

    let method = test_method("items", TypeRef::Vec(Box::new(TypeRef::String)), false, None);
    let methods = [&method];

    let fixture = make_fixture("collection_return_fixture", serde_json::json!({ "name": "backend" }));
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission
            .setup_block
            .contains("fn items(&self) -> Vec<String> { Vec::new() }"),
        "collection-returning stub method must stay empty, got: {}",
        emission.setup_block
    );
}
