use super::dispatch::gen_trampoline;
use super::helpers::{c_trampoline_signature, substitute_excluded_types};
use super::orchestration::gen_trait_bridge;
use super::registration::{gen_clear_fn, gen_unregistration_fn};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{TypeDef, TypeRef};
use heck::ToPascalCase;
use std::collections::HashSet;

#[test]
fn test_vtable_struct_name_derivation() {
    // Test the pattern: {CRATE_UPPER}{CratePascal}{TraitPascal}VTable
    let crate_name = "sample_crate";
    let crate_upper = crate_name.to_uppercase();
    let crate_pascal = crate_name.to_pascal_case();
    let trait_name = "OcrBackend";
    let trait_pascal = trait_name.to_pascal_case();

    let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");

    assert_eq!(c_vtable_struct, "SAMPLE_CRATESampleCrateOcrBackendVTable");
}

#[test]
fn test_register_function_name_format() {
    // Test the pattern: {ffi_prefix}_register_{trait_snake}
    let ffi_prefix = "sample_crate";
    let trait_name = "OcrBackend";
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();

    let register_fn = format!("{}_register_{}", ffi_prefix, trait_snake);
    assert_eq!(register_fn, "sample_crate_register_ocr_backend");
}

#[test]
fn test_unregister_function_name_format() {
    // Test the pattern: {ffi_prefix}_unregister_{trait_snake}
    let ffi_prefix = "sample_crate";
    let trait_name = "PostProcessor";
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();

    let unregister_fn = format!("{}_unregister_{}", ffi_prefix, trait_snake);
    assert_eq!(unregister_fn, "sample_crate_unregister_post_processor");
}

#[test]
fn test_vtable_struct_name_multiple_traits() {
    // Verify correct naming for multiple traits
    let test_cases = vec![
        ("sample_crate", "OcrBackend", "SAMPLE_CRATESampleCrateOcrBackendVTable"),
        (
            "sample_crate",
            "PostProcessor",
            "SAMPLE_CRATESampleCratePostProcessorVTable",
        ),
        ("sample_crate", "Validator", "SAMPLE_CRATESampleCrateValidatorVTable"),
        (
            "sample_crate",
            "EmbeddingBackend",
            "SAMPLE_CRATESampleCrateEmbeddingBackendVTable",
        ),
    ];

    for (crate_name, trait_name, expected_struct) in test_cases {
        let crate_upper = crate_name.to_uppercase();
        let crate_pascal = crate_name.to_pascal_case();
        let trait_pascal = trait_name.to_pascal_case();
        let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");

        assert_eq!(
            c_vtable_struct, expected_struct,
            "Mismatch for crate={}, trait={}",
            crate_name, trait_name
        );
    }
}

#[test]
fn gen_unregistration_fn_returns_empty_when_none() {
    let cfg = crate::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        unregister_fn: None,
        clear_fn: None,
        ..Default::default()
    };
    let result = gen_unregistration_fn(&cfg, "sample_crate", "OcrBackend");
    assert!(result.is_empty(), "expected empty output when unregister_fn is None");
}

#[test]
fn gen_unregistration_fn_emits_wrapper_when_set() {
    let cfg = crate::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        unregister_fn: Some("remove_ocr_backend".to_string()),
        clear_fn: None,
        ..Default::default()
    };
    let result = gen_unregistration_fn(&cfg, "sample_crate", "OcrBackend");
    assert!(
        !result.is_empty(),
        "expected non-empty output when unregister_fn is set"
    );
    assert!(
        result.contains("func RemoveOcrBackend(name string) error"),
        "generated function signature not found in:\n{result}"
    );
    assert!(
        result.contains("C.sample_crate_unregister_ocr_backend"),
        "C call not found in:\n{result}"
    );
}

#[test]
fn gen_clear_fn_returns_empty_when_none() {
    let cfg = crate::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        unregister_fn: None,
        clear_fn: None,
        ..Default::default()
    };
    let result = gen_clear_fn(&cfg, "sample_crate", "OcrBackend");
    assert!(result.is_empty(), "expected empty output when clear_fn is None");
}

#[test]
fn gen_clear_fn_emits_wrapper_when_set() {
    let cfg = crate::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        unregister_fn: None,
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    };
    let result = gen_clear_fn(&cfg, "sample_crate", "OcrBackend");
    assert!(!result.is_empty(), "expected non-empty output when clear_fn is set");
    assert!(
        result.contains("func ClearOcrBackends() error"),
        "generated function signature not found in:\n{result}"
    );
    assert!(
        result.contains("C.sample_crate_clear_ocr_backend"),
        "C call not found in:\n{result}"
    );
}

#[test]
fn substitute_excluded_types_replaces_excluded_named_with_json() {
    let mut excluded = HashSet::new();
    excluded.insert("InternalDocument");
    // Excluded named type collapses to Json so the Go trait-bridge interface
    // can fall back to `json.RawMessage`.
    let result = substitute_excluded_types(&TypeRef::Named("InternalDocument".to_string()), &excluded);
    assert!(matches!(result, TypeRef::Json), "expected Json, got {:?}", result);
}

#[test]
fn substitute_excluded_types_leaves_non_excluded_named_intact() {
    let excluded: HashSet<&str> = HashSet::new();
    let result = substitute_excluded_types(&TypeRef::Named("ExtractionConfig".to_string()), &excluded);
    match result {
        TypeRef::Named(ref n) => assert_eq!(n, "ExtractionConfig"),
        other => panic!("expected Named, got {:?}", other),
    }
}

#[test]
fn substitute_excluded_types_recurses_into_optional_vec_map() {
    let mut excluded = HashSet::new();
    excluded.insert("X");
    excluded.insert("Y");
    excluded.insert("Z");
    // Optional<Named("X")> → Optional<Json>
    let opt = TypeRef::Optional(Box::new(TypeRef::Named("X".to_string())));
    match substitute_excluded_types(&opt, &excluded) {
        TypeRef::Optional(inner) => assert!(matches!(*inner, TypeRef::Json)),
        other => panic!("expected Optional<Json>, got {:?}", other),
    }
    // Vec<Named("Y")> → Vec<Json>
    let v = TypeRef::Vec(Box::new(TypeRef::Named("Y".to_string())));
    match substitute_excluded_types(&v, &excluded) {
        TypeRef::Vec(inner) => assert!(matches!(*inner, TypeRef::Json)),
        other => panic!("expected Vec<Json>, got {:?}", other),
    }
    // Map<String, Named("Z")> → Map<String, Json>
    let m = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Z".to_string())));
    match substitute_excluded_types(&m, &excluded) {
        TypeRef::Map(k, v) => {
            assert!(matches!(*k, TypeRef::String));
            assert!(matches!(*v, TypeRef::Json));
        }
        other => panic!("expected Map<String, Json>, got {:?}", other),
    }
}

/// Regression (#114): the CGo trampoline signature for a Bytes parameter must include
/// a companion `{name}Len C.size_t` parameter.  Without it the trampoline has no way
/// to bound the read when the payload contains embedded NUL bytes (0x00).
#[test]
fn trampoline_bytes_param_includes_len_companion() {
    let method = crate::core::ir::MethodDef {
        name: "process".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "payload".to_string(),
            ty: TypeRef::Bytes,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
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
    let mut out = String::new();
    gen_trampoline(&mut out, "Ingester", "Ingester", &method);

    // The CGo trampoline must declare the length companion so the Go body can
    // use unsafe.Slice(ptr, len) rather than C.GoString() which stops at 0x00.
    assert!(
        out.contains("payloadLen C.size_t"),
        "trampoline must include `payloadLen C.size_t` for Bytes param;\nactual:\n{out}"
    );
    // The conversion body must use unsafe.Slice, not GoString (which NUL-truncates).
    assert!(
        out.contains("unsafe.Slice"),
        "trampoline conversion must use unsafe.Slice for Bytes param;\nactual:\n{out}"
    );
    // Must NOT fall back to the old base64 roundtrip.
    assert!(
        !out.contains("base64"),
        "trampoline must not use base64 encoding for Bytes param;\nactual:\n{out}"
    );
}

/// Regression (#114): the C preamble extern declaration for a Bytes parameter
/// must include `size_t {name}_len` so the linker resolves the trampoline correctly.
#[test]
fn c_trampoline_signature_bytes_param_includes_len_companion() {
    let method = crate::core::ir::MethodDef {
        name: "process".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "payload".to_string(),
            ty: TypeRef::Bytes,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
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
    let sig = c_trampoline_signature("goIngesterProcess", &method);
    assert!(
        sig.contains("size_t payload_len"),
        "C preamble sig must include `size_t payload_len`;\nactual:\n{sig}"
    );
}

#[test]
fn trait_bridge_register_uses_c_vtable_helper_and_free_string_callback() {
    let trait_def = TypeDef {
        name: "OcrBackend".to_string(),
        rust_path: "sample_crate::OcrBackend".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: None,
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let mut out = String::new();
    let excluded = HashSet::new();

    gen_trait_bridge(
        &mut out,
        &trait_def,
        &bridge_cfg,
        "sample_crate",
        "sample_crate",
        &excluded,
        "ocr_backend",
    );

    assert!(out.contains("vtable := C.sample_crate_ocr_backend_vtable_new("));
    assert!(out.contains("defer C.free(unsafe.Pointer(vtable))"));
    assert!(out.contains("func goOcrBackendFreeString(ptr *C.char)"));
    assert!(out.contains("C.free(unsafe.Pointer(ptr))"));
}

#[test]
fn substitute_excluded_types_passes_through_primitives_and_other_atoms() {
    let excluded: HashSet<&str> = HashSet::new();
    assert!(matches!(
        substitute_excluded_types(&TypeRef::String, &excluded),
        TypeRef::String
    ));
    assert!(matches!(
        substitute_excluded_types(&TypeRef::Bytes, &excluded),
        TypeRef::Bytes
    ));
    assert!(matches!(
        substitute_excluded_types(&TypeRef::Unit, &excluded),
        TypeRef::Unit
    ));
}

/// Regression (C9): go cgo vtable cimport convention must be consistent.
/// C function signature takes `struct TypeVTable` by value, so Go call must pass
/// value type, not `&vtable` (pointer).  The register_c_call template should emit
/// `vtable` (value), not `&vtable` (pointer).
#[test]
fn register_c_call_passes_vtable_by_value() {
    let trait_def = TypeDef {
        name: "Backend".to_string(),
        rust_path: "test_crate::Backend".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Backend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let mut out = String::new();
    let excluded = HashSet::new();

    gen_trait_bridge(
        &mut out,
        &trait_def,
        &bridge_cfg,
        "test_crate",
        "test_crate",
        &excluded,
        "backend",
    );

    // The registration call must pass vtable by value, not by reference.
    // The C header declares the parameter as `struct TestVTable vtable` (value type),
    // so the Go call must use `vtable` not `&vtable`.
    assert!(
        out.contains("C.test_crate_register_backend(\n\t\tcName,\n\t\tvtable,"),
        "register_c_call must pass vtable by value (not &vtable);\nactual:\n{out}"
    );
    // Ensure we're NOT passing a pointer
    assert!(
        !out.contains("&vtable,"),
        "register_c_call must not pass &vtable (pointer);\nactual:\n{out}"
    );
}
