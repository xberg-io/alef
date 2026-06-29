use super::*;
use crate::core::ir::{FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};

fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: Vec::<FieldDef>::new(),
        methods,
        is_opaque: true,
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
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: error_type.map(|s| s.to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: super_trait.map(|s| s.to_string()),
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

#[test]
fn trait_vtable_includes_free_string_and_status_lifecycle_callbacks() {
    let trait_def = make_trait_def("Backend", vec![make_method("run", vec![], TypeRef::Unit, None)]);
    let bridge_cfg = make_bridge_cfg("Backend", Some("Plugin"));
    let mut out = String::new();

    emit_trait_bridge(
        "sample",
        "SampleError",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    assert!(out.contains("free_string: ?*const fn (ptr: [*c]u8) callconv(.c) void = null"));
    assert!(out.contains("name_fn: ?*const fn (user_data: ?*anyopaque, out_name: ?*?[*c]u8, out_error: ?*?[*c]u8) callconv(.c) i32 = null"));
    assert!(out.contains("version_fn: ?*const fn (user_data: ?*anyopaque, out_version: ?*?[*c]u8, out_error: ?*?[*c]u8) callconv(.c) i32 = null"));
}

#[test]
fn single_method_trait_emits_vtable_and_register() {
    let trait_def = make_trait_def(
        "Validator",
        vec![make_method(
            "validate",
            vec![make_param("input", TypeRef::String)],
            TypeRef::Primitive(PrimitiveType::Bool),
            None,
        )],
    );
    let bridge_cfg = make_bridge_cfg("Validator", None);

    let mut out = String::new();
    emit_trait_bridge(
        "demo",
        "error",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    // Vtable struct
    assert!(
        out.contains("pub const IValidator = extern struct {"),
        "missing vtable struct: {out}"
    );
    // Method slot present
    assert!(out.contains("validate:"), "missing validate slot: {out}");
    // user_data first arg
    assert!(out.contains("user_data: ?*anyopaque"), "missing user_data: {out}");
    // callconv(.c) present
    assert!(out.contains("callconv(.c)"), "missing callconv: {out}");
    // free_user_data slot
    assert!(out.contains("free_user_data:"), "missing free_user_data: {out}");
    // Registration shim
    assert!(out.contains("pub fn register_validator("), "missing register fn: {out}");
    assert!(out.contains("c.demo_register_validator("), "wrong C symbol: {out}");
    // Unregistration shim
    assert!(
        out.contains("pub fn unregister_validator("),
        "missing unregister fn: {out}"
    );
    assert!(
        out.contains("c.demo_unregister_validator("),
        "wrong unregister C symbol: {out}"
    );
    // No plugin lifecycle when no super_trait
    assert!(
        !out.contains("name_fn:"),
        "should not emit name_fn without super_trait: {out}"
    );
}

#[test]
fn emit_trait_bridge_emits_clear_fn_when_configured() {
    let trait_def = make_trait_def(
        "PluginBackend",
        vec![make_method(
            "process",
            vec![make_param("input", TypeRef::String)],
            TypeRef::String,
            Some("PluginError"),
        )],
    );
    let mut bridge_cfg = make_bridge_cfg("PluginBackend", Some("sample_crate::plugins::Plugin"));
    bridge_cfg.clear_fn = Some("clear_plugin_backends".to_string());

    let mut out = String::new();
    emit_trait_bridge(
        "sample_crate",
        "SampleCrateError",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    assert!(
        out.contains("pub fn clear_plugin_backends() SampleCrateError!void"),
        "missing clear_plugin_backends signature: {out}"
    );
    // C symbol uses the singular trait-snake suffix to match sample_core-ffi naming.
    assert!(
        out.contains("c.sample_crate_clear_plugin_backend(&_out_error)"),
        "wrong C symbol target for clear wrapper: {out}"
    );
    // Doc comment present.
    assert!(
        out.contains("/// Remove ALL registered `PluginBackend` plugins"),
        "missing clear doc comment: {out}"
    );
}

#[test]
fn emit_trait_bridge_omits_clear_fn_when_not_configured() {
    let trait_def = make_trait_def(
        "PluginBackend",
        vec![make_method(
            "process",
            vec![make_param("input", TypeRef::String)],
            TypeRef::String,
            Some("PluginError"),
        )],
    );
    let bridge_cfg = make_bridge_cfg("PluginBackend", Some("sample_crate::plugins::Plugin"));
    // clear_fn left as None.

    let mut out = String::new();
    emit_trait_bridge(
        "sample_crate",
        "SampleCrateError",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    assert!(
        !out.contains("pub fn clear_"),
        "should not emit any clear_* fn when clear_fn is None: {out}"
    );
}

#[test]
fn multi_method_trait_with_super_trait_emits_lifecycle_slots() {
    let trait_def = make_trait_def(
        "PluginBackend",
        vec![
            make_method(
                "process_payload",
                vec![
                    make_param("payload_bytes", TypeRef::Bytes),
                    make_param("config", TypeRef::String),
                ],
                TypeRef::String,
                Some("PluginError"),
            ),
            make_method(
                "supports_mode",
                vec![make_param("mode", TypeRef::String)],
                TypeRef::Primitive(PrimitiveType::Bool),
                None,
            ),
        ],
    );
    let bridge_cfg = make_bridge_cfg("PluginBackend", Some("sample_crate::plugins::Plugin"));

    let mut out = String::new();
    emit_trait_bridge(
        "sample_crate",
        "SampleCrateError",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    // Struct name
    assert!(
        out.contains("pub const IPluginBackend = extern struct {"),
        "missing vtable: {out}"
    );
    // Plugin lifecycle slots emitted
    assert!(out.contains("name_fn:"), "missing name_fn: {out}");
    assert!(out.contains("version_fn:"), "missing version_fn: {out}");
    assert!(out.contains("initialize_fn:"), "missing initialize_fn: {out}");
    assert!(out.contains("shutdown_fn:"), "missing shutdown_fn: {out}");
    // Trait method slots
    assert!(out.contains("process_payload:"), "missing process_payload slot: {out}");
    assert!(out.contains("supports_mode:"), "missing supports_mode slot: {out}");
    // Bytes param expands to ptr + len
    assert!(out.contains("payload_bytes_ptr:"), "missing bytes ptr expansion: {out}");
    assert!(out.contains("payload_bytes_len:"), "missing bytes len expansion: {out}");
    // Fallible method gets out_error
    assert!(
        out.contains("out_error:"),
        "missing out_error for fallible method: {out}"
    );
    // C symbols use sample_core prefix
    assert!(
        out.contains("c.sample_crate_register_plugin_backend("),
        "wrong register symbol: {out}"
    );
    assert!(
        out.contains("c.sample_crate_unregister_plugin_backend("),
        "wrong unregister symbol: {out}"
    );
    // Registration shim signature
    assert!(
        out.contains("pub fn register_plugin_backend("),
        "missing register_plugin_backend fn: {out}"
    );
}

// -----------------------------------------------------------------
// make_*_vtable tests
// -----------------------------------------------------------------

#[test]
fn make_vtable_emits_comptime_function_and_thunk() {
    let trait_def = make_trait_def(
        "Validator",
        vec![make_method(
            "validate",
            vec![make_param("input", TypeRef::String)],
            TypeRef::Primitive(PrimitiveType::Bool),
            None,
        )],
    );
    let bridge_cfg = make_bridge_cfg("Validator", None);

    let mut out = String::new();
    emit_trait_bridge(
        "demo",
        "error",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    // Helper function declaration
    assert!(
        out.contains("pub fn make_validator_vtable(comptime T: type, instance: *T)"),
        "missing make_validator_vtable: {out}"
    );
    // Returns the vtable type
    assert!(out.contains("IValidator{"), "missing vtable literal: {out}");
    // Thunk casts user_data
    assert!(out.contains("@ptrCast(@alignCast(ud))"), "missing @ptrCast cast: {out}");
    // callconv(.c) in thunk
    assert!(out.contains("callconv(.c)"), "missing callconv(.c) in thunk: {out}");
    // validate thunk field
    assert!(out.contains(".validate ="), "missing .validate thunk field: {out}");
    // free_user_data thunk
    assert!(
        out.contains(".free_user_data ="),
        "missing .free_user_data thunk: {out}"
    );
    // No lifecycle stubs without super_trait
    assert!(
        !out.contains(".name_fn ="),
        "must not emit .name_fn without super_trait: {out}"
    );
}

#[test]
fn make_vtable_with_super_trait_emits_lifecycle_stubs() {
    let trait_def = make_trait_def("PluginBackend", vec![]);
    let bridge_cfg = make_bridge_cfg("PluginBackend", Some("sample_crate::Plugin"));

    let mut out = String::new();
    emit_trait_bridge(
        "sample_crate",
        "SampleCrateError",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    assert!(
        out.contains("pub fn make_plugin_backend_vtable(comptime T: type, instance: *T)"),
        "missing make_plugin_backend_vtable: {out}"
    );
    assert!(out.contains(".name_fn ="), "missing .name_fn stub: {out}");
    assert!(out.contains(".version_fn ="), "missing .version_fn stub: {out}");
    assert!(out.contains(".initialize_fn ="), "missing .initialize_fn stub: {out}");
    assert!(out.contains(".shutdown_fn ="), "missing .shutdown_fn stub: {out}");
}

#[test]
fn make_vtable_bytes_param_passes_c_pointer_in_thunk() {
    let trait_def = make_trait_def(
        "Processor",
        vec![make_method(
            "process",
            vec![make_param("data", TypeRef::Bytes)],
            TypeRef::Unit,
            None,
        )],
    );

    let mut out = String::new();
    emit_make_vtable(
        "Processor",
        false,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
        &[],
    );

    // Thunk receives ptr+len params
    assert!(out.contains("data_ptr: [*c]const u8"), "missing data_ptr param: {out}");
    assert!(out.contains("data_len: usize"), "missing data_len param: {out}");
    // The Zig vtable ABI passes the raw C pointer through; the len is discarded.
    assert!(out.contains("_ = data_len;"), "thunk must discard the len param: {out}");
    // Thunk calls self.process with the C pointer (not a reconstructed slice).
    assert!(
        out.contains("self.process(data_ptr);"),
        "thunk must call self.process with the C pointer: {out}"
    );
}

#[test]
fn make_vtable_fallible_method_returns_i32_error_code() {
    let trait_def = make_trait_def(
        "Parser",
        vec![make_method("parse", vec![], TypeRef::Unit, Some("ParseError"))],
    );
    let bridge_cfg = make_bridge_cfg("Parser", None);

    let mut out = String::new();
    emit_trait_bridge(
        "demo",
        "error",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    // Thunk returns i32 (fallible → i32 return)
    assert!(
        out.contains("callconv(.c) i32"),
        "fallible thunk must return i32: {out}"
    );
    // Returns 0 on success
    assert!(out.contains("return 0;"), "must return 0 on success: {out}");
    // Returns 1 on error
    assert!(out.contains("return 1;"), "must return 1 on error: {out}");
    // Error branch writes to out_error
    assert!(out.contains("out_error"), "must write to out_error: {out}");
}

#[test]
fn make_vtable_primitive_return_passes_through() {
    let trait_def = make_trait_def(
        "Counter",
        vec![make_method(
            "count",
            vec![],
            TypeRef::Primitive(PrimitiveType::I32),
            None,
        )],
    );
    let bridge_cfg = make_bridge_cfg("demo", None);

    let mut out = String::new();
    emit_trait_bridge(
        "demo",
        "error",
        &bridge_cfg,
        &trait_def,
        &std::collections::HashSet::new(),
        &mut out,
    );

    // Infallible primitive method: thunk returns the value directly
    assert!(
        out.contains("return self.count()"),
        "primitive return must be forwarded directly: {out}"
    );
}

// -----------------------------------------------------------------
// ZigTraitBridgeGenerator tests
// -----------------------------------------------------------------

fn make_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
    use crate::codegen::generators::trait_bridge::TraitBridgeSpec;
    use std::collections::HashMap;
    TraitBridgeSpec {
        trait_def,
        bridge_config: bridge_cfg,
        core_import: "sample_crate",
        wrapper_prefix: "Zig",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "SampleCrateError".to_string(),
        error_constructor: "SampleCrateError::msg({msg})".to_string(),
    }
}

#[test]
fn gen_unregistration_fn_emits_wrapper_when_configured() {
    let trait_def = make_trait_def("PluginBackend", vec![]);
    let mut bridge_cfg = make_bridge_cfg("PluginBackend", None);
    bridge_cfg.unregister_fn = Some("unregister_plugin_backend".to_string());

    let generator = ZigTraitBridgeGenerator::new("sample_crate");
    let spec = make_spec(&trait_def, &bridge_cfg);
    let out = generator.gen_unregistration_fn(&spec);

    assert!(!out.is_empty(), "expected non-empty output when unregister_fn is set");
    assert!(
        out.contains("pub fn unregister_plugin_backend("),
        "wrong function name: {out}"
    );
    assert!(
        out.contains("c.sample_crate_unregister_plugin_backend("),
        "wrong C symbol: {out}"
    );
    assert!(
        out.contains("out_error: ?*?[*c]u8") || out.contains("out_error"),
        "missing out_error param: {out}"
    );
    assert!(out.contains("return "), "missing return statement: {out}");
    assert!(out.ends_with("}\n"), "missing closing brace: {out}");
}

#[test]
fn gen_unregistration_fn_returns_empty_when_not_configured() {
    let trait_def = make_trait_def("PluginBackend", vec![]);
    let bridge_cfg = make_bridge_cfg("PluginBackend", None); // unregister_fn is None

    let generator = ZigTraitBridgeGenerator::new("sample_crate");
    let spec = make_spec(&trait_def, &bridge_cfg);
    let out = generator.gen_unregistration_fn(&spec);

    assert!(
        out.is_empty(),
        "expected empty output when unregister_fn is None, got: {out}"
    );
}

#[test]
fn gen_clear_fn_emits_wrapper_when_configured() {
    let trait_def = make_trait_def("PluginBackend", vec![]);
    let mut bridge_cfg = make_bridge_cfg("PluginBackend", None);
    bridge_cfg.clear_fn = Some("clear_plugin_backends".to_string());

    let generator = ZigTraitBridgeGenerator::new("sample_crate");
    let spec = make_spec(&trait_def, &bridge_cfg);
    let out = generator.gen_clear_fn(&spec);

    assert!(!out.is_empty(), "expected non-empty output when clear_fn is set");
    assert!(
        out.contains("pub fn clear_plugin_backends("),
        "wrong function name: {out}"
    );
    assert!(
        out.contains("c.sample_crate_clear_plugin_backends("),
        "wrong C symbol: {out}"
    );
    assert!(
        out.contains("out_error: ?*?[*c]u8") || out.contains("out_error"),
        "missing out_error param: {out}"
    );
    assert!(out.contains("return "), "missing return statement: {out}");
    assert!(out.ends_with("}\n"), "missing closing brace: {out}");
}

#[test]
fn gen_clear_fn_returns_empty_when_not_configured() {
    let trait_def = make_trait_def("PluginBackend", vec![]);
    let bridge_cfg = make_bridge_cfg("PluginBackend", None); // clear_fn is None

    let generator = ZigTraitBridgeGenerator::new("sample_crate");
    let spec = make_spec(&trait_def, &bridge_cfg);
    let out = generator.gen_clear_fn(&spec);

    assert!(
        out.is_empty(),
        "expected empty output when clear_fn is None, got: {out}"
    );
}

#[test]
fn gen_unregistration_fn_uses_snake_case_function_name_verbatim() {
    // The configured `unregister_fn` name is used as-is (not re-derived from the trait).
    let trait_def = make_trait_def("DocumentExtractor", vec![]);
    let mut bridge_cfg = make_bridge_cfg("DocumentExtractor", None);
    bridge_cfg.unregister_fn = Some("unregister_extractor".to_string());

    let generator = ZigTraitBridgeGenerator::new("demo");
    let spec = make_spec(&trait_def, &bridge_cfg);
    let out = generator.gen_unregistration_fn(&spec);

    assert!(
        out.contains("pub fn unregister_extractor("),
        "must use configured fn name verbatim: {out}"
    );
    assert!(
        out.contains("c.demo_unregister_extractor("),
        "must use configured fn name in C symbol: {out}"
    );
}

#[test]
fn gen_clear_fn_uses_configured_fn_name_verbatim() {
    let trait_def = make_trait_def("DocumentExtractor", vec![]);
    let mut bridge_cfg = make_bridge_cfg("DocumentExtractor", None);
    bridge_cfg.clear_fn = Some("clear_all_extractors".to_string());

    let generator = ZigTraitBridgeGenerator::new("demo");
    let spec = make_spec(&trait_def, &bridge_cfg);
    let out = generator.gen_clear_fn(&spec);

    assert!(
        out.contains("pub fn clear_all_extractors("),
        "must use configured fn name verbatim: {out}"
    );
    assert!(
        out.contains("c.demo_clear_all_extractors("),
        "must use configured fn name in C symbol: {out}"
    );
}

#[test]
fn vtable_preserves_named_types_for_c_abi_compatibility() {
    // Test that VTable signatures do NOT substitute excluded types.
    // The vtable is a C ABI struct and must preserve the exact C types.
    let mut excluded = std::collections::HashSet::new();
    excluded.insert("InternalDocument".to_string());
    excluded.insert("ExtractionResult".to_string());

    let trait_def = make_trait_def(
        "DocumentExtractor",
        vec![
            make_method(
                "extract_bytes",
                vec![
                    make_param("content", TypeRef::Bytes),
                    make_param("mime_type", TypeRef::String),
                ],
                TypeRef::Named("InternalDocument".to_string()),
                Some("SampleCrateError"),
            ),
            make_method(
                "process_result",
                vec![make_param("result", TypeRef::Named("ExtractionResult".to_string()))],
                TypeRef::Unit,
                None,
            ),
        ],
    );
    let bridge_cfg = make_bridge_cfg("DocumentExtractor", None);

    let mut out = String::new();
    emit_trait_bridge(
        "sample_crate",
        "SampleCrateError",
        &bridge_cfg,
        &trait_def,
        &excluded,
        &mut out,
    );

    // VTable struct must be present with the trait name
    assert!(
        out.contains("pub const IDocumentExtractor = extern struct {"),
        "missing vtable struct"
    );

    // Method slots must NOT have type substitution — they should use C ABI types
    // ([*c]const u8, i32, etc.) not Zig types. The excluded types should appear
    // as C pointers, not as Json or other substitutions.
    assert!(
        out.contains("extract_bytes:") && out.contains("callconv(.c)"),
        "extract_bytes method slot missing"
    );
    assert!(out.contains("process_result:"), "process_result method slot missing");

    // Bytes param expands to ptr + len in vtable signature
    assert!(
        out.contains("content_ptr: [*c]const u8") && out.contains("content_len: usize"),
        "Bytes param should expand to ptr+len in C ABI"
    );

    // The result param should be [*c]const u8 (C string), not the Zig type
    // ExtractionResult or Json or any substitution
    assert!(
        out.contains("result: [*c]const u8"),
        "Named types in vtable should map to [*c]const u8, not be substituted"
    );

    // Return type should be i32 (error code) for fallible methods, not substituted
    let has_fallible_return = out.contains("callconv(.c) i32");
    assert!(has_fallible_return, "fallible method should return i32 for error code");
}

#[test]
fn make_vtable_thunks_preserve_c_abi_types() {
    // Test that thunk function signatures preserve C ABI types.
    let mut excluded = std::collections::HashSet::new();
    excluded.insert("InternalDocument".to_string());

    let trait_def = make_trait_def(
        "Renderer",
        vec![make_method(
            "render",
            vec![make_param("doc", TypeRef::Named("InternalDocument".to_string()))],
            TypeRef::Bytes,
            Some("SampleCrateError"),
        )],
    );
    let bridge_cfg = make_bridge_cfg("Renderer", None);

    let mut out = String::new();
    emit_trait_bridge(
        "sample_crate",
        "SampleCrateError",
        &bridge_cfg,
        &trait_def,
        &excluded,
        &mut out,
    );

    // make_renderer_vtable should exist
    assert!(
        out.contains("pub fn make_renderer_vtable(comptime T: type, instance: *T)"),
        "make_renderer_vtable helper missing"
    );

    // Thunk for render method should use C ABI types in its signature
    assert!(out.contains(".render ="), "render thunk field missing");

    // Thunk should have callconv(.c) and i32 return for the fallible method
    assert!(
        out.contains("callconv(.c) i32"),
        "thunk should return i32 for error code"
    );

    // The parameter should be [*c]const u8 (C string from doc param)
    assert!(
        out.contains("doc: [*c]const u8"),
        "thunk param should be C ABI type, not substituted"
    );
    assert!(
        !out.contains("unreachable"),
        "generated vtable helpers must not use unreachable stubs: {out}"
    );
    // Complex fallible returns serialize to JSON ([]u8). When JSON serialization
    // is not yet implemented, the thunk returns null as a placeholder.
    // The vtable still compiles, allowing e2e tests to run (they'll exercise
    // the null path and validate error handling).
    assert!(
        out.contains("ptr.* = null") || out.contains("ptr.* = ."),
        "complex fallible vtable returns must return a safe placeholder: {out}"
    );
}
