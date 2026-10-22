//! Regression coverage for the Java trait-bridge e2e stub's super-trait lifecycle overrides.
//!
//! `gen_interface_file` (`backends::java::gen_bindings::trait_bridge`) declares `name()` and
//! `version()` abstract on `I<Trait>` unconditionally whenever a bridge configures
//! `super_trait` -- it never looks up the super-trait's own `TypeDef`. The e2e stub built here
//! (`build_args_and_setup`'s `test_backend` arm) used to be the one place that *did* look it up,
//! by matching `TraitBridgeConfig::super_trait` against `TypeDef::rust_path`, and silently added
//! no lifecycle methods at all when that lookup missed -- exactly what happens when the
//! super-trait lives behind a private module re-exported via `pub use`, whose extracted
//! `rust_path` need not equal the configured value. The interface required the overrides either
//! way, so the generated stub failed to compile: "is not abstract and does not override abstract
//! method version()". See `trait_bridge_naming::SUPER_TRAIT_REQUIRED_METHODS`, the single
//! authority both sides now consult.

use super::args::{JavaArgsContext, build_args_and_setup};
use crate::backends::java::gen_bindings::trait_bridge;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::config::ArgMapping;
use crate::e2e::fixture::Fixture;
use std::collections::HashSet;

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn test_backend_arg(trait_name: &str) -> ArgMapping {
    ArgMapping {
        name: "backend".to_string(),
        field: "input.backend".to_string(),
        arg_type: "test_backend".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some(trait_name.to_string()),
    }
}

fn make_trait_type_def(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        methods,
        is_trait: true,
        is_opaque: true,
        ..TypeDef::default()
    }
}

fn make_method(name: &str, has_default_impl: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type: TypeRef::String,
        has_default_impl,
        ..MethodDef::default()
    }
}

fn render_stub(config: &ResolvedCrateConfig, type_defs: &[TypeDef]) -> String {
    let fixture = make_fixture("register_renderer_trait_bridge");
    let arg = test_backend_arg("Renderer");
    let mut teardown = String::new();
    let (setup, _args) = build_args_and_setup(
        &fixture.input,
        &[arg],
        JavaArgsContext {
            class_name: "Sample",
            options_type: None,
            fixture: &fixture,
            adapter_request_type: None,
            owner_handle_is_receiver: false,
            config,
            type_defs,
            enums: &[],
            target_params: TargetParams::IrAbsent,
            teardown_block: &mut teardown,
        },
    );
    setup.join("\n")
}

/// A trait bridge configured with `super_trait`, exercised with `type_defs` that hold the
/// primary trait but never resolve the super-trait's own `TypeDef` at all -- the exact shape a
/// `rust_path` mismatch (or, as here, a genuinely absent supertrait entry) produces. Before the
/// fix, the merge loop that copies the super-trait's methods into the stub silently ran zero
/// iterations and neither `name()` nor `version()` was ever emitted, while `IRenderer.java`
/// still declared both abstract -- a "does not override abstract method" compile failure.
///
/// `render_page` also carries `has_default_impl: true` to pin the other half of the diagnosis:
/// a default-implemented method declared directly on the *primary* trait was never dropped by
/// this filter (it only checks `ffi_skip_methods` and the literal names `description`/`author`),
/// so it must appear in the stub with or without the fix.
#[test]
fn stub_synthesizes_super_trait_lifecycle_methods_when_super_trait_type_def_is_unresolved() {
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "Renderer".to_string(),
            super_trait: Some("neutral_crate::plugins::Plugin".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    // No "Plugin" TypeDef anywhere in scope: the super-trait lookup in `build_args_and_setup`
    // finds nothing, exactly like a `rust_path` that does not match the configured string.
    let type_defs = [make_trait_type_def(
        "Renderer",
        "neutral_crate::plugins::Renderer",
        vec![make_method("render_page", true)],
    )];

    let stub = render_stub(&config, &type_defs);

    assert!(
        stub.contains("render_page"),
        "a default-implemented method declared directly on the primary trait must still be \
         stubbed, got:\n{stub}"
    );
    assert!(
        stub.contains("public String name()"),
        "super-trait lifecycle method name() must be synthesized when the super-trait's own \
         TypeDef cannot be resolved, got:\n{stub}"
    );
    assert!(
        stub.contains("public String version()"),
        "super-trait lifecycle method version() must be synthesized when the super-trait's own \
         TypeDef cannot be resolved -- this is the exact defect reported against IOcrBackend and \
         friends, got:\n{stub}"
    );
}

/// Negative control: when the super-trait's own `TypeDef` **does** resolve (its `rust_path`
/// matches the configured `super_trait` exactly), the real methods already supply `name()` and
/// `version()`. The synthetic fallback must not duplicate them.
#[test]
fn stub_does_not_duplicate_lifecycle_methods_when_super_trait_type_def_resolves() {
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "Renderer".to_string(),
            super_trait: Some("neutral_crate::plugins::Plugin".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [
        make_trait_type_def(
            "Renderer",
            "neutral_crate::plugins::Renderer",
            vec![make_method("render_page", false)],
        ),
        make_trait_type_def(
            "Plugin",
            "neutral_crate::plugins::Plugin",
            vec![make_method("name", false), make_method("version", true)],
        ),
    ];

    let stub = render_stub(&config, &type_defs);

    assert_eq!(
        stub.matches("public String name()").count(),
        1,
        "name() must appear exactly once, got:\n{stub}"
    );
    assert_eq!(
        stub.matches("public String version()").count(),
        1,
        "version() must appear exactly once -- the synthetic fallback must not duplicate a \
         method the real super-trait lookup already supplied, got:\n{stub}"
    );
}

/// Compares the two generated artifacts against each other, rather than against a hand-written
/// literal list: every abstract method name `gen_interface_file` declares on `I<Trait>` must
/// have an `@Override` in the stub `build_args_and_setup` assembles for the same trait bridge,
/// so the two generators cannot drift apart again silently.
#[test]
fn stub_overrides_every_abstract_method_the_interface_declares() {
    let trait_def = make_trait_type_def(
        "Renderer",
        "neutral_crate::plugins::Renderer",
        vec![make_method("render_page", false)],
    );
    let visible: HashSet<&str> = HashSet::new();
    let excluded: HashSet<String> = HashSet::new();
    let files = trait_bridge::gen_trait_bridge_files(
        &trait_def,
        "neutral",
        "dev.neutral",
        true,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );

    // Every abstract (non-`default`) method signature in the interface, as a bare method name.
    let abstract_method_names: Vec<&str> = files
        .interface_content
        .lines()
        .filter(|line| line.contains('(') && !line.contains("default "))
        .filter_map(|line| line.split('(').next())
        .filter_map(|before_paren| before_paren.split_whitespace().last())
        .collect();
    assert!(
        abstract_method_names.contains(&"name"),
        "test setup sanity: interface must declare name() abstract, got:\n{}",
        files.interface_content
    );
    assert!(
        abstract_method_names.contains(&"version"),
        "test setup sanity: interface must declare version() abstract, got:\n{}",
        files.interface_content
    );
    assert!(
        abstract_method_names.contains(&"render_page"),
        "test setup sanity: interface must declare render_page() abstract, got:\n{}",
        files.interface_content
    );

    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "Renderer".to_string(),
            super_trait: Some("neutral_crate::plugins::Plugin".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    // No Plugin TypeDef in scope, same as the reported failure.
    let type_defs = [trait_def];
    let stub = render_stub(&config, &type_defs);

    for method_name in &abstract_method_names {
        assert!(
            stub.contains(&format!("public String {method_name}()")),
            "stub must override every abstract method the interface declares; missing \
             `{method_name}()`, got:\n{stub}"
        );
    }
}
