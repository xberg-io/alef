#[test]
fn visitor_bridge_uses_configured_context_and_result_metadata() {
    let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
    let output = super::gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
    )
    .expect("visitor bridge should generate");

    crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
    assert!(output.code.contains("\"display_name\""));
}

/// `let mut pairs = Vec::new();` followed by a statically known run of `pairs.push(...)` is
/// `clippy::vec_init_then_push`, which a consumer building the generated extendr crate under
/// `-D warnings` rejects. The context pairs must be a `vec![]` literal instead. ~keep
#[test]
fn visitor_context_helper_builds_pairs_as_a_vec_literal_not_init_then_push() {
    let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
    let output = super::gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
    )
    .expect("visitor bridge should generate");

    assert!(
        output
            .code
            .contains("let pairs: Vec<(&str, extendr_api::Robj)> = vec!["),
        "context pairs must be built as a vec! literal:\n{}",
        output.code
    );
    assert!(
        output
            .code
            .contains(r#"        ("display_name", extendr_api::Robj::from(AsRef::<str>::as_ref(&ctx.display_name))),"#),
        "each context field must render as a vec! element:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("pairs.push("),
        "context pairs must not be built by push:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("let mut pairs"),
        "context pairs must not need a mutable binding:\n{}",
        output.code
    );
}

use crate::backends::extendr::trait_bridge::{ExtendrBridgeGenerator, native_marshalled_extendr_struct_params};
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn struct_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        fields,
        has_serde: true,
        ..Default::default()
    }
}

fn named_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn greeter_trait_with(methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: "Greeter".to_string(),
        rust_path: "sample_core::Greeter".to_string(),
        is_trait: true,
        methods,
        ..Default::default()
    }
}

fn ref_named_param(name: &str, ty_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(ty_name.to_string()),
        is_ref: true,
        ..Default::default()
    }
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        ..Default::default()
    }
}

fn generator_with(struct_params: &[&str]) -> ExtendrBridgeGenerator {
    ExtendrBridgeGenerator {
        core_import: "sample_core".to_string(),
        type_paths: HashMap::new(),
        error_type: "SampleError".to_string(),
        struct_param_types: struct_params.iter().map(|s| s.to_string()).collect(),
        struct_return_types: std::collections::HashSet::new(),
        forwardable_defaulted: Default::default(),
    }
}

fn plugin_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
    TraitBridgeSpec {
        trait_def,
        bridge_config: bridge_cfg,
        core_import: "sample_core",
        wrapper_prefix: "R",
        type_paths: HashMap::new(),
        lifetime_type_names: HashSet::new(),
        error_type: "SampleError".to_string(),
        error_constructor: "SampleError::Message { message: {msg} }".to_string(),
    }
}

#[test]
fn allowlist_includes_serde_struct_excludes_enum_opaque_and_incompatible() {
    let mut api = ApiSurface::default();
    api.types
        .push(struct_typedef("Opts", vec![named_field("greeting", TypeRef::String)]));
    api.types.push(struct_typedef(
        "Bag",
        vec![named_field(
            "items",
            TypeRef::Vec(Box::new(TypeRef::Named("Opts".to_string()))),
        )],
    ));

    let trait_def = greeter_trait_with(vec![
        method(
            "greet",
            vec![ref_named_param("opts", "Opts"), ref_named_param("bag", "Bag")],
            TypeRef::Named("Doc".to_string()),
            false,
        ),
        method(
            "decorate",
            vec![ref_named_param("mood", "Mood"), ref_named_param("widget", "Widget")],
            TypeRef::Unit,
            false,
        ),
    ]);

    let allow = native_marshalled_extendr_struct_params(&trait_def, &api);
    assert!(allow.contains("Opts"), "serde struct param must qualify: {allow:?}");
    assert!(
        !allow.contains("Bag"),
        "extendr-incompatible struct must be excluded: {allow:?}"
    );
    assert!(!allow.contains("Mood"), "enum must be excluded: {allow:?}");
    assert!(!allow.contains("Widget"), "unknown type must be excluded: {allow:?}");
}

#[test]
fn sync_struct_param_marshalled_as_native_r_object_not_json_string() {
    let generator = generator_with(&["Opts"]);
    let trait_def = greeter_trait_with(vec![]);
    let bridge_cfg = TraitBridgeConfig::default();
    let spec = plugin_spec(&trait_def, &bridge_cfg);

    let m = method(
        "greet",
        vec![ref_named_param("opts", "Opts")],
        TypeRef::Named("Doc".to_string()),
        false,
    );
    let body = generator.gen_sync_method_body(&m, &spec);

    assert!(
        body.contains("extendr_api::Robj::from(Opts::from((*opts).clone()))"),
        "struct param must be built as the binding's native R object via From<core>:\n{body}"
    );
    assert!(
        !body.contains("serde_json::to_string(opts)"),
        "struct param must NOT be serialized to a JSON string:\n{body}"
    );
}

/// Return-side counterpart: a method returning a native-marshalled struct must first try to unwrap
/// the host's native `ExternalPtr` and convert via `From<Binding>` (mirroring the options decoder),
/// falling back to the JSON-string path. See issue #153.
#[test]
fn native_struct_return_unwraps_external_ptr_before_json() {
    let generator = ExtendrBridgeGenerator {
        core_import: "sample_core".to_string(),
        type_paths: HashMap::new(),
        error_type: "SampleError".to_string(),
        struct_param_types: std::collections::HashSet::new(),
        struct_return_types: std::collections::HashSet::from(["Doc".to_string()]),
        forwardable_defaulted: Default::default(),
    };
    let trait_def = greeter_trait_with(vec![]);
    let bridge_cfg = TraitBridgeConfig::default();
    let spec = plugin_spec(&trait_def, &bridge_cfg);

    let m = method("build", vec![], TypeRef::Named("Doc".to_string()), false);
    let body = generator.gen_sync_method_body(&m, &spec);

    assert!(
        body.contains("ExternalPtr::<Doc>::try_from(&val)") && body.contains("(*ext).clone().into()"),
        "native struct return must unwrap the ExternalPtr and convert via From<Doc>:\n{body}"
    );
    assert!(
        body.contains("serde_json::from_str"),
        "the JSON-string fallback must remain:\n{body}"
    );
}

#[test]
fn async_struct_param_marshalled_as_native_r_object_not_json_string() {
    let generator = generator_with(&["Opts"]);
    let trait_def = greeter_trait_with(vec![]);
    let bridge_cfg = TraitBridgeConfig::default();
    let spec = plugin_spec(&trait_def, &bridge_cfg);

    let m = method(
        "greet",
        vec![ref_named_param("opts", "Opts")],
        TypeRef::Named("Doc".to_string()),
        true,
    );
    let body = generator.gen_async_method_body(&m, &spec);

    assert!(
        body.contains("let opts_owned = (*opts).clone();"),
        "async preamble must clone the owned core struct value:\n{body}"
    );
    assert!(
        body.contains("extendr_api::Robj::from(Opts::from(opts_owned.clone()))"),
        "async struct param must be built as the binding's native R object:\n{body}"
    );
    assert!(
        !body.contains("opts_json"),
        "async struct param must NOT be serialized to a JSON string:\n{body}"
    );
}

#[test]
fn enum_and_unknown_named_params_keep_json_string_representation() {
    let generator = generator_with(&["Opts"]);
    let trait_def = greeter_trait_with(vec![]);
    let bridge_cfg = TraitBridgeConfig::default();
    let spec = plugin_spec(&trait_def, &bridge_cfg);

    let m = method(
        "decorate",
        vec![ref_named_param("mood", "Mood"), ref_named_param("widget", "Widget")],
        TypeRef::Unit,
        false,
    );
    let body = generator.gen_sync_method_body(&m, &spec);
    assert!(
        body.contains("serde_json::to_string(mood)") && body.contains("serde_json::to_string(widget)"),
        "non-struct Named params must keep the JSON-string representation:\n{body}"
    );
    assert!(
        !body.contains("Mood::from(") && !body.contains("Widget::from("),
        "non-struct Named params must NOT be built as native objects:\n{body}"
    );
}

#[test]
fn context_arguments_pass_the_existing_robj_directly() {
    for is_ref in [true, false] {
        let param = ParamDef {
            is_ref,
            ..ref_named_param("context", "Context")
        };
        let expected = if is_ref {
            "nodecontext_to_robj(context)"
        } else {
            "nodecontext_to_robj(&context)"
        };
        assert_eq!(super::build_extendr_arg(&param, Some("Context")), expected);
    }
}

#[test]
fn registration_maps_string_errors_with_the_variant_constructor() {
    let trait_def = greeter_trait_with(vec![]);
    let config = TraitBridgeConfig {
        register_fn: Some("register_backend".to_string()),
        registry_getter: Some("registry".to_string()),
        ..Default::default()
    };
    let output = generator_with(&[]).gen_registration_fn(&plugin_spec(&trait_def, &config));
    assert!(
        output.contains("::new(r_backend).map_err(extendr_api::Error::Other)?"),
        "{output}"
    );
}
