use super::*;
use crate::core::ir::ParamDef;

fn get_language_fn() -> FunctionDef {
    FunctionDef {
        name: "get_language".to_string(),
        rust_path: "sample::get_language".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
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
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: None,
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
    }
}

fn make_cfg(host_type: &str, construct_expr: &str) -> HostCapsuleTypeConfig {
    HostCapsuleTypeConfig {
        host_type: host_type.to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: construct_expr.to_string(),
        ..Default::default()
    }
}

#[test]
fn capsule_method_emits_configured_host_type_and_construct_expr() {
    let func = get_language_fn();
    let cfg = make_cfg(
        "io.github.example.jtreesitter.Language",
        "new io.github.example.jtreesitter.Language({ptr})",
    );
    let mut out = String::new();
    gen_capsule_function_method(
        &mut out,
        CapsuleFunctionMethodParams {
            func: &func,
            prefix: "tsp",
            class_name: "LanguagePack",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            config: &cfg,
        },
    );
    assert!(
        out.contains("io.github.example.jtreesitter.Language"),
        "must use configured host_type. Got:\n{out}"
    );
    assert!(
        out.contains("new io.github.example.jtreesitter.Language(resultPtr)"),
        "must use configured construct_expr with ptr substituted. Got:\n{out}"
    );
}

#[test]
fn capsule_method_registers_named_temporary_before_invocation() {
    let mut func = get_language_fn();
    func.params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        ..Default::default()
    }];
    let cfg = make_cfg("Language", "new Language({ptr})");
    let mut out = String::new();

    gen_capsule_function_method(
        &mut out,
        CapsuleFunctionMethodParams {
            func: &func,
            prefix: "tsp",
            class_name: "LanguagePack",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            config: &cfg,
        },
    );

    let registration = out
        .find("nativeResources.register(cconfig, handle -> NativeLib.TSP_CONFIG_FREE.invoke(handle))")
        .expect("temporary registration");
    let invocation = out
        .find("NativeLib.TSP_GET_LANGUAGE.invoke")
        .expect("capsule invocation");
    assert!(out.contains("var nativeResources = new NativeResources()"), "{out}");
    assert!(registration < invocation, "{out}");
}

#[test]
fn capsule_method_errors_when_host_type_empty() {
    let func = get_language_fn();
    let cfg = make_cfg("", "new MyLanguage({ptr})");
    let mut out = String::new();
    gen_capsule_function_method(
        &mut out,
        CapsuleFunctionMethodParams {
            func: &func,
            prefix: "tsp",
            class_name: "LanguagePack",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            config: &cfg,
        },
    );
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must mention the missing field. Got:\n{out}"
    );
}

#[test]
fn capsule_method_errors_when_construct_expr_empty() {
    let func = get_language_fn();
    let cfg = make_cfg("io.github.example.jtreesitter.Language", "");
    let mut out = String::new();
    gen_capsule_function_method(
        &mut out,
        CapsuleFunctionMethodParams {
            func: &func,
            prefix: "tsp",
            class_name: "LanguagePack",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            config: &cfg,
        },
    );
    assert!(
        out.contains("ALEF ERROR"),
        "empty construct_expr must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("construct_expr"),
        "error must mention the missing field. Got:\n{out}"
    );
}

// --- issue #380: `&mut T` DTO write-back --------------------------------------------------

fn mut_dto_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        is_mut: true,
        ..Default::default()
    }
}

fn tag_record_fn(return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: "tag_record".to_string(),
        params: vec![mut_dto_param("record", "Record")],
        return_type,
        error_type: None,
        ..Default::default()
    }
}

/// `fn tag_record(record: &mut Record)` must not silently drop the mutation: the generated
/// method invokes the FFI mutator, reads the already-marshaled `crecord` handle back out via
/// `_to_json` (without re-registering it for free -- it is already registered during parameter
/// marshalling), decodes it, and returns the decoded value. Asserting only that the signature
/// grew a `Record` return proves nothing -- the load-bearing check is the full round trip, in
/// order: marshal (which already registers the free), invoke, read back, decode, return.
#[test]
fn mut_dto_param_writes_back_the_mutated_value() {
    let func = tag_record_fn(TypeRef::Unit);
    let mut out = String::new();
    gen_sync_function_method(
        &mut out,
        SyncFunctionMethodParams {
            func: &func,
            prefix: "krz",
            class_name: "SampleClient",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            has_visitor_bridge: false,
            clear_fn_handles: &AHashMap::new(),
            capsule_types: &HashMap::new(),
        },
    );

    assert!(
        out.contains("public static Record tagRecord(final Record record) throws SampleClientException {"),
        "expected the write-back signature (Record in, Record out), got:\n{out}"
    );
    assert!(
        out.contains("nativeResources.register(crecord, handle -> NativeLib.KRZ_RECORD_FREE.invoke(handle));"),
        "must still marshal and register the free for the parameter handle, got:\n{out}"
    );
    assert!(
        out.contains("NativeLib.KRZ_TAG_RECORD.invoke(crecord);"),
        "must still call the FFI mutator, got:\n{out}"
    );
    assert!(
        out.contains("var jsonPtr = (MemorySegment) NativeLib.KRZ_RECORD_TO_JSON.invoke(crecord);"),
        "must read the mutated handle back out via the FFI's _to_json helper, got:\n{out}"
    );
    assert!(
        out.contains("return MAPPER.readValue(json, Record.class);"),
        "must decode the read-back JSON into a fresh Record and return it, got:\n{out}"
    );
    // Exactly one registration for crecord: the marshalling registration, never a second one
    // from the write-back read (that would be a double free).
    assert_eq!(
        out.matches("nativeResources.register(crecord,").count(),
        1,
        "crecord must be registered for free exactly once, got:\n{out}"
    );

    let marshal_pos = out
        .find("nativeResources.register(crecord,")
        .expect("marshal registration");
    let invoke_pos = out
        .find("NativeLib.KRZ_TAG_RECORD.invoke(crecord);")
        .expect("mutator call");
    let readback_pos = out
        .find("NativeLib.KRZ_RECORD_TO_JSON.invoke(crecord);")
        .expect("read-back call");
    assert!(
        marshal_pos < invoke_pos && invoke_pos < readback_pos,
        "must marshal, then mutate, then read back, in that order, got:\n{out}"
    );

    // The pre-fix shape emitted the call and returned nothing, discarding the mutation.
    assert!(
        !out.contains("static void tagRecord"),
        "must not regress to a void return, got:\n{out}"
    );
}

/// Negative control: an immutable `&Record` DTO parameter must NOT gain a read-back.
#[test]
fn immutable_dto_param_gets_no_writeback() {
    let func = FunctionDef {
        params: vec![ParamDef {
            is_ref: true,
            is_mut: false,
            ..mut_dto_param("record", "Record")
        }],
        ..tag_record_fn(TypeRef::Unit)
    };
    let mut out = String::new();
    gen_sync_function_method(
        &mut out,
        SyncFunctionMethodParams {
            func: &func,
            prefix: "krz",
            class_name: "SampleClient",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            has_visitor_bridge: false,
            clear_fn_handles: &AHashMap::new(),
            capsule_types: &HashMap::new(),
        },
    );

    assert!(
        out.contains("public static void tagRecord(final Record record) throws SampleClientException {"),
        "an immutable DTO param must keep the plain void signature, got:\n{out}"
    );
    assert!(
        !out.contains("_TO_JSON"),
        "an immutable DTO param must not read anything back, got:\n{out}"
    );
}

/// Negative control: an owned (by-value) DTO parameter must render the same as before this fix.
#[test]
fn owned_dto_param_is_unchanged() {
    let func = FunctionDef {
        params: vec![ParamDef {
            is_ref: false,
            is_mut: false,
            ..mut_dto_param("record", "Record")
        }],
        ..tag_record_fn(TypeRef::Unit)
    };
    let mut out = String::new();
    gen_sync_function_method(
        &mut out,
        SyncFunctionMethodParams {
            func: &func,
            prefix: "krz",
            class_name: "SampleClient",
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
            has_visitor_bridge: false,
            clear_fn_handles: &AHashMap::new(),
            capsule_types: &HashMap::new(),
        },
    );

    assert!(
        out.contains("public static void tagRecord(final Record record) throws SampleClientException {"),
        "an owned DTO param must keep the plain void signature, got:\n{out}"
    );
    assert!(
        !out.contains("_TO_JSON"),
        "an owned DTO param must not read anything back, got:\n{out}"
    );
}
