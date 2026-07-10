//! Tests for auto-delegation of reference params, borrowed returns and mixed param shapes.
//!
//! These exercise the shared generators used by the String-Json backends (PyO3, extendr) and
//! the shared call-arg builders. They guard against regressions where alef bailed with a
//! `compile_error!` stub or emitted a wrong owned/borrow conversion.

use super::*;

fn ref_param(name: &str, ty: TypeRef, optional: bool, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn typed(name: &str, is_opaque: bool) -> TypeDef {
    let mut t = simple_type_def();
    t.name = name.to_string();
    t.rust_path = format!("my_crate::{name}");
    t.is_opaque = is_opaque;
    t.fields = vec![];
    t
}

#[test]
fn static_method_with_named_ref_param_delegates_with_owned_core_borrow() {
    let typ = typed("ConfidenceSignals", false);
    let method = MethodDef {
        name: "from_extraction_result".to_string(),
        params: vec![
            ref_param("result", TypeRef::Named("ExtractionResult".to_string()), false, true),
            ref_param(
                "schema_compliance",
                TypeRef::Named("SchemaCompliance".to_string()),
                false,
                false,
            ),
            ref_param("text_coverage", TypeRef::Primitive(PrimitiveType::F32), false, false),
        ],
        return_type: TypeRef::Named("ConfidenceSignals".to_string()),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let result = gen_static_method(
        &method,
        &RustMapper,
        &default_cfg(),
        &typ,
        &AdapterBodies::default(),
        &AHashSet::new(),
        &AHashSet::new(),
    );

    assert!(
        !result.contains("compile_error!"),
        "static method with &T param must auto-delegate, not bail:\n{result}"
    );
    assert!(
        result.contains("let result_core: my_crate::ExtractionResult = result.into();"),
        "should bind an owned core temporary for the &T param:\n{result}"
    );
    assert!(
        result.contains(
            "my_crate::ConfidenceSignals::from_extraction_result(&result_core, schema_compliance_core, text_coverage)"
        ),
        "should pass a borrow of the owned core temporary:\n{result}"
    );
}

#[test]
fn opaque_method_returning_option_ref_clones_before_convert() {
    let typ = typed("Registry", true);
    let method = MethodDef {
        name: "get".to_string(),
        params: vec![ref_param("id", TypeRef::String, false, true)],
        return_type: TypeRef::Optional(Box::new(TypeRef::Named("Preset".to_string()))),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: true,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut opaque = AHashSet::new();
    opaque.insert("Registry".to_string());
    let result = gen_method(
        &method,
        &RustMapper,
        &default_cfg(),
        &typ,
        true,
        &opaque,
        &AHashSet::new(),
        &AdapterBodies::default(),
    );

    assert!(
        !result.contains("compile_error!"),
        "opaque method returning Option<&T> must auto-delegate:\n{result}"
    );
    assert!(
        result.contains("self.inner.get(&id).map(|v| v.clone().into())"),
        "borrowed Option<&T> return should clone before converting:\n{result}"
    );
}

#[test]
fn locked_opaque_method_returning_option_ref_clones_before_convert() {
    let typ = typed("Registry", true);
    let method = MethodDef {
        name: "get".to_string(),
        params: vec![ref_param("id", TypeRef::String, false, true)],
        return_type: TypeRef::Optional(Box::new(TypeRef::Named("Preset".to_string()))),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: true,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut opaque = AHashSet::new();
    opaque.insert("Registry".to_string());
    let mut mutex = AHashSet::new();
    mutex.insert("Registry".to_string());
    let result = gen_method(
        &method,
        &RustMapper,
        &default_cfg(),
        &typ,
        true,
        &opaque,
        &mutex,
        &AdapterBodies::default(),
    );

    assert!(
        !result.contains("compile_error!"),
        "locked opaque method returning Option<&T> must auto-delegate:\n{result}"
    );
    assert!(
        result.contains("self.inner.lock().unwrap().get(&id).map(|v| v.clone().into())"),
        "borrowed Option<&T> return on a locked wrapper should lock, clone, then convert:\n{result}"
    );
}

#[test]
fn locked_opaque_method_returning_bare_ref_clones_before_convert() {
    let typ = typed("Registry", true);
    let method = MethodDef {
        name: "first".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Preset".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: true,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut opaque = AHashSet::new();
    opaque.insert("Registry".to_string());
    let mut mutex = AHashSet::new();
    mutex.insert("Registry".to_string());
    let result = gen_method(
        &method,
        &RustMapper,
        &default_cfg(),
        &typ,
        true,
        &opaque,
        &mutex,
        &AdapterBodies::default(),
    );

    assert!(
        !result.contains("compile_error!"),
        "locked opaque method returning &T must auto-delegate:\n{result}"
    );
    assert!(
        result.contains("self.inner.lock().unwrap().first().clone().into()"),
        "borrowed &T return on a locked wrapper should lock, clone, then convert:\n{result}"
    );
}

#[test]
fn free_fn_mixed_ref_json_and_map_params_delegate_with_json_str() {
    let mut context = ref_param(
        "context",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        false,
        true,
    );
    context.map_is_btree = true;
    let func = FunctionDef {
        name: "resolve".to_string(),
        rust_path: "my_crate::resolve".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ref_param("preset", TypeRef::Named("Preset".to_string()), false, true),
            ref_param("custom_schema", TypeRef::Json, true, false),
            context,
        ],
        return_type: TypeRef::Named("ResolvedPreset".to_string()),
        is_async: false,
        error_type: Some("ResolveError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let result = gen_function(
        &func,
        &RustMapper,
        &default_cfg(),
        &AdapterBodies::default(),
        &AHashSet::new(),
    );

    assert!(
        !result.contains("compile_error!"),
        "mixed free fn must delegate:\n{result}"
    );
    assert!(
        result.contains("let preset_core: my_crate::Preset = preset.into();"),
        "should bind an owned core temporary for the &Preset param:\n{result}"
    );
    assert!(
        result.contains("custom_schema.as_ref().and_then(|s| serde_json::from_str(s).ok())"),
        "optional Json param should be parsed from a String at the call site:\n{result}"
    );
    assert!(
        result.contains("&context.unwrap_or_default()"),
        "promoted &Map param should be materialised and borrowed:\n{result}"
    );
}

#[test]
fn call_args_json_str_variant_parses_json_string_params() {
    let opaque = AHashSet::new();
    let req = vec![ref_param("schema", TypeRef::Json, false, false)];
    assert_eq!(
        binding_helpers::gen_call_args_with_let_bindings_json_str(&req, &opaque),
        "serde_json::from_str(&schema).unwrap_or_default()",
    );
    let opt = vec![ref_param("schema", TypeRef::Json, true, false)];
    assert_eq!(
        binding_helpers::gen_call_args_with_let_bindings_json_str(&opt, &opaque),
        "schema.as_ref().and_then(|s| serde_json::from_str(s).ok())",
    );
}

#[test]
fn call_args_plain_variant_passes_json_value_params_through() {
    let opaque = AHashSet::new();
    let req = vec![ref_param("schema", TypeRef::Json, false, false)];
    assert_eq!(
        binding_helpers::gen_call_args_with_let_bindings(&req, &opaque),
        "schema",
    );
}

#[test]
fn call_args_btree_map_ref_param_collects_into_btreemap() {
    let opaque = AHashSet::new();
    let mut p = ref_param(
        "context",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        false,
        true,
    );
    p.map_is_btree = true;
    assert_eq!(
        binding_helpers::gen_call_args_with_let_bindings(std::slice::from_ref(&p), &opaque),
        "&context.into_iter().collect::<std::collections::BTreeMap<_, _>>()",
    );
}
