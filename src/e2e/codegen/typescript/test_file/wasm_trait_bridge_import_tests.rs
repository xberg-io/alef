//! A WASM trait-bridge docs snippet must import only the enums its rendered body actually
//! references as whole identifiers, never one that merely prefixes a correctly-used name.
//!
//! Split out of `snippet.rs` (a remediation target at the 1,000-line cap and may not grow):
//! this is a self-contained regression with its own fixture, mirroring how
//! `wasm_snippet_prefix_tests.rs` was split out of `tests.rs` for the same reason. ~keep

use super::snippet::{SnippetContext, references_identifier, render_snippet_body};
use crate::core::ir::{EnumDef, EnumVariant, MethodDef, ReceiverKind, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn fixture() -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "quick_start".to_string(),
        category: None,
        description: "Quick start".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: Vec::new(),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
    }
}

/// Regression for the trait-bridge fixture failure where a WASM snippet imported
/// `WasmSampleBackend`, a symbol the package never exports (only `WasmSampleBackendType` exists).
/// The trait-bridge enum-import loop scans every enum in the crate's full registry and keeps
/// one whenever its wasm-prefixed name is a *substring* of the rendered body -- and
/// `WasmSampleBackend` is a substring of the correctly-used `WasmSampleBackendType` (the stub's
/// `backendType(): WasmSampleBackendType { ... }` return annotation), even though nothing in the
/// body ever names `WasmSampleBackend` as a whole identifier. `references_identifier`'s
/// word-boundary check must keep the coincidentally-prefixing enum out.
#[test]
fn wasm_trait_bridge_snippet_does_not_import_an_enum_that_merely_prefixes_another() {
    let mut fixture = fixture();
    fixture.id = "register_sample_backend_trait_bridge".into();
    fixture.args = vec![crate::e2e::config::ArgMapping {
        name: "backend".into(),
        field: "backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some("SampleBackend".into()),
    }];
    let mut e2e = E2eConfig::default();
    e2e.call.function = "registerSampleBackend".into();
    let config = crate::core::config::ResolvedCrateConfig {
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "SampleBackend".into(),
            register_fn: Some("register_sample_backend".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let trait_type = TypeDef {
        name: "SampleBackend".into(),
        rust_path: "sample_crate::SampleBackend".into(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "backend_type".into(),
            params: vec![],
            return_type: TypeRef::Named("SampleBackendType".into()),
            receiver: Some(ReceiverKind::Ref),
            error_type: Some("anyhow::Error".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    // `SampleBackend` sits in the crate's full enum registry for an entirely unrelated
    // reason (e.g. a config selector), never referenced by this trait's own methods --
    // modeling the coincidental name collision a crate's trait and its own
    // `SampleBackendType` enum reproduce.
    let enums = [
        EnumDef {
            name: "SampleBackendType".into(),
            rust_path: "sample_crate::SampleBackendType".into(),
            variants: vec![EnumVariant {
                name: "Builtin".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        EnumDef {
            name: "SampleBackend".into(),
            rust_path: "sample_crate::config::SampleBackend".into(),
            variants: vec![EnumVariant {
                name: "Default".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let type_defs = [trait_type];

    let body = render_snippet_body(SnippetContext {
        lang: "wasm",
        fixture: &fixture,
        module: "@example/wasm",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &type_defs,
        enums: &enums,
        functions: &[],
        wasm_type_prefix: "Wasm",
        config: &config,
    });
    assert!(
        body.contains("WasmSampleBackendType"),
        "the stub must reference the real, non-colliding enum, got: {body}"
    );
    assert!(
        !references_identifier(&body, "WasmSampleBackend"),
        "must not import the coincidentally-prefixing enum as a bare identifier, got: {body}"
    );
}
