//! Regression tests for `emit_test_backend`/`emit_ts_stub_method`'s trait-bridge stub codegen.
//!
//! Split out of `typescript/mod.rs`, which was already at the repo's 1,000-line cap (see
//! `file-modularization` in CLAUDE.md), so new coverage for the struct-return cast fix went into
//! a fresh module instead of growing it further. ~keep

use super::*;

fn make_fixture(id: &str, input: serde_json::Value) -> crate::e2e::fixture::Fixture {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "description": "test fixture",
        "input": input,
        "assertions": []
    }))
    .expect("minimal fixture JSON must parse")
}

#[test]
fn emit_test_backend_ts_generates_class_and_new_expr() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let m1 = test_method("syncOp", TypeRef::String, false, false);
    let m2 = test_method("asyncOp", TypeRef::Named("WorkResult".to_string()), true, false);
    let methods = [&m1, &m2];

    let fixture = make_fixture("ts_test_fixture", serde_json::json!({ "name": "my-ts-backend" }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[], "");

    // setup_block must define a TS class.
    assert!(
        emission.setup_block.contains("class _TestStub_ts_test_fixture"),
        "setup_block should define the stub class, got: {}",
        emission.setup_block
    );
    // Must NOT hardcode sample_core-domain trait names.
    assert!(
        !emission.setup_block.contains("OcrBackend"),
        "setup_block must not hardcode OcrBackend"
    );
    assert!(
        !emission.setup_block.contains("DocumentExtractor"),
        "setup_block must not hardcode DocumentExtractor"
    );

    // name() emitted because super_trait is set.
    assert!(
        emission.setup_block.contains("name()"),
        "setup_block should emit name() method"
    );
    assert!(
        emission.setup_block.contains("my-ts-backend"),
        "name() should return the backend name"
    );

    // Required methods emitted.
    assert!(
        emission.setup_block.contains("syncOp("),
        "required sync method should be emitted"
    );
    assert!(
        emission.setup_block.contains("async asyncOp("),
        "required async method should be emitted with async keyword"
    );
    assert!(
        emission.setup_block.contains("syncOp(): string"),
        "sync method should return the generated sync shape, got: {}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("async asyncOp(): Promise<WorkResult>"),
        "a Named-returning async method must declare the real struct type, matching the \
         interface it stands in for, not the generic 'string' fallback, got: {}",
        emission.setup_block
    );

    // arg_expr uses new keyword.
    assert_eq!(
        emission.arg_expr, "new _TestStub_ts_test_fixture()",
        "arg_expr should use new constructor"
    );

    // Named return type must use a cast "{}" literal, not `new WorkResult()`: the napi-rs
    // bridge coerces the JS return value to a string and parses it as JSON, so the runtime
    // value must stay a JSON-round-tripping string -- but the stub's OWN declared return
    // type now names the real struct (`Promise<WorkResult>` above), so the literal needs
    // `as unknown as WorkResult` to satisfy that declared type.
    assert!(
        emission.setup_block.contains("return \"{}\" as unknown as WorkResult;"),
        "Named return type should emit a cast \"{{}}\" literal, not a bare string or a \
         constructor call, got: {}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("new WorkResult()"),
        "Named return type must not emit a constructor call, got: {}",
        emission.setup_block
    );
}

/// `Vec<T>`-returning trait methods (e.g. `EmbeddingBackend::embed() ->
/// Vec<Vec<f32>>`, `OcrBackend::supported_languages() -> Vec<String>`) must
/// get a matching array return-type annotation on the generated stub.
///
/// Before this fix, `ts_stub_return_type` fell through to its `"string"`
/// default for any non-primitive, non-unit type -- including `Vec<T>` --
/// while `default_val` correctly emitted `[]` for the same type. The
/// mismatch between the declared `Promise<string>` and the actual `[]`
/// return value is a `tsc` TS2322 ("Type 'never[]' is not assignable to
/// type 'string'"), and it fails every generated trait-bridge stub whose
/// interface has an array-returning method: embedding, OCR, and reranker
/// backends all failed this way (see register_embedding_backend_trait_bridge.md,
/// register_ocr_backend_trait_bridge.md, register_reranker_backend_trait_bridge.md).
#[test]
fn emit_test_backend_ts_vec_return_type_uses_array_annotation_not_string() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{PrimitiveType, TypeRef};

    let bridge = TraitBridgeConfig {
        trait_name: "EmbeddingBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    // Vec<Vec<f32>>, matching EmbeddingBackend::embed's real return type.
    let embed = test_method(
        "embed",
        TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32))))),
        true,
        false,
    );
    // Vec<String>, matching OcrBackend::supported_languages's real return type.
    let supported_languages = test_method(
        "supportedLanguages",
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        false,
    );
    let methods = [&embed, &supported_languages];

    let fixture = make_fixture("vec_return_fixture", serde_json::json!({ "name": "my-embedder" }));
    let emission = emit_test_backend(&bridge, &methods, &fixture, &[], "");

    assert!(
        emission
            .setup_block
            .contains("async embed(): Promise<Array<Array<number>>> { return []; }"),
        "Vec<Vec<f32>> return type must be declared as Array<Array<number>>, got: {}",
        emission.setup_block
    );
    assert!(
        emission
            .setup_block
            .contains("supportedLanguages(): Array<string> { return []; }"),
        "Vec<String> return type must be declared as Array<string>, got: {}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains(": Promise<string> { return []; }"),
        "an array-typed method must never declare a bare 'string' return type, got: {}",
        emission.setup_block
    );
}

/// A trait method returning a `Named` enum (e.g. `PostProcessor::processing_stage()
/// -> ProcessingStage`) must be annotated with the real enum type, not the generic
/// `string` fallback, and its stub body must return a value that actually satisfies
/// that type at the napi-rs bridge boundary.
///
/// The bridge coerces the JS return value to a string and parses it with
/// `serde_json::from_str`, so the emitted body must return the JSON-quoted variant
/// name (`"Early"`, quotes included) cast through `unknown` to satisfy the nominal
/// enum return annotation — see `emit_ts_stub_method`.
#[test]
fn emit_test_backend_ts_named_enum_return_uses_enum_type_and_valid_variant() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{EnumDef, EnumVariant, TypeRef};

    let bridge = TraitBridgeConfig {
        trait_name: "PostProcessor".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let processing_stage = test_method(
        "processingStage",
        TypeRef::Named("ProcessingStage".to_string()),
        false,
        false,
    );
    let methods = [&processing_stage];

    let enums = [EnumDef {
        name: "ProcessingStage".to_string(),
        variants: vec![
            EnumVariant {
                name: "Early".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Middle".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Late".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];

    let fixture = make_fixture("enum_return_fixture", serde_json::json!({ "name": "my-processor" }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, &enums, "");

    assert!(
        emission
            .setup_block
            .contains("processingStage(): ProcessingStage { return \"\\\"Early\\\"\" as unknown as ProcessingStage; }"),
        "Named enum return must be annotated with the real enum type and return a \
         JSON-quoted first variant cast to that type, got: {}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("processingStage(): string"),
        "Named enum return must not fall through to the generic 'string' annotation, got: {}",
        emission.setup_block
    );
    assert_eq!(
        emission.type_imports,
        vec!["ProcessingStage".to_string()],
        "the enum type must be recorded so callers can wire up the import"
    );
}

#[test]
fn emit_test_backend_ts_extracts_fixture_values_for_numeric_and_string_defaults() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{PrimitiveType, TypeRef};

    let bridge = TraitBridgeConfig {
        trait_name: "EmbeddingBackend".to_string(),
        super_trait: Some("OcrBackend".to_string()),
        ..Default::default()
    };

    let m1 = test_method("dimensions", TypeRef::Primitive(PrimitiveType::U32), false, false);
    let m2 = test_method("model", TypeRef::String, false, false);
    let methods = [&m1, &m2];

    // Fixture with backend input containing dimensions value
    let fixture = make_fixture(
        "embedding_fixture",
        serde_json::json!({
            "name": "my-embedder",
            "backend": {
                "dimensions": 768,
                "model": "all-MiniLM-L6-v2"
            }
        }),
    );

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[], "");

    assert!(
        emission.setup_block.contains("dimensions(): number { return 768; }"),
        "numeric method should extract value from fixture.input.backend, got: {}",
        emission.setup_block
    );
    assert!(
        emission
            .setup_block
            .contains("model(): string { return \"all-MiniLM-L6-v2\"; }"),
        "string method should extract value from fixture.input.backend, got: {}",
        emission.setup_block
    );
}

#[test]
fn emit_test_backend_ts_emits_default_impl_noops() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        ..Default::default()
    };

    let required = test_method("mustImplement", TypeRef::String, false, false);
    let optional = test_method("mayImplement", TypeRef::String, false, true);
    let methods = [&required, &optional];

    let fixture = make_fixture("ts_skip_defaults", serde_json::json!({}));
    let emission = emit_test_backend(&bridge, &methods, &fixture, &[], "");

    assert!(
        emission.setup_block.contains("mustImplement("),
        "required method should be emitted"
    );
    assert!(
        emission.setup_block.contains("mayImplement("),
        "default-impl method should be emitted as a no-op stub"
    );
}

/// The napi trait bridge builds a `ThreadsafeFunction` for every `Plugin` lifecycle method in its
/// constructor, so a stub that only defines `name()` throws
/// `Object property 'version' type mismatch ... received Undefined` at `register_<trait>` time.
/// The stub must define all four before the trait's own methods are reached. ~keep
#[test]
fn emit_test_backend_ts_emits_every_plugin_lifecycle_method_for_a_super_trait() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeRef;

    let bridge = TraitBridgeConfig {
        trait_name: "EmbeddingBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };
    let embed = test_method("embed", TypeRef::String, false, false);
    let methods = [&embed];
    let fixture = make_fixture("lifecycle_fixture", serde_json::json!({ "name": "lifecycle-backend" }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[], "");

    for method in [
        "name(): string",
        "version(): string",
        "initialize(): void",
        "shutdown(): void",
    ] {
        assert!(
            emission.setup_block.contains(method),
            "stub must define `{method}`, got: {}",
            emission.setup_block
        );
    }
    assert_eq!(
        emission.setup_block.matches("version()").count(),
        1,
        "version() must be emitted exactly once, got: {}",
        emission.setup_block
    );
}
