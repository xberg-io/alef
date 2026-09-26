use super::*;
use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};

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

#[test]
fn test_params_require_marshal_for_named_non_opaque() {
    let params = vec![make_param("options", TypeRef::Named("Config".to_string()))];
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(params_require_marshal(&params, &opaque));
}

#[test]
fn test_params_require_marshal_false_for_opaque() {
    let params = vec![make_param("client", TypeRef::Named("Client".to_string()))];
    let opaque: std::collections::HashSet<&str> = ["Client"].into();
    assert!(!params_require_marshal(&params, &opaque));
}

#[test]
fn test_is_bridge_param_matches_by_name() {
    let param = make_param("visitor", TypeRef::Named("VisitorHandle".to_string()));
    let bridge_names: HashSet<String> = ["visitor".to_string()].into();
    let aliases: HashSet<String> = HashSet::new();
    assert!(is_bridge_param(&param, &bridge_names, &aliases));
}

#[test]
fn test_params_require_marshal_for_vec() {
    let params = vec![make_param(
        "items",
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
    )];
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(params_require_marshal(&params, &opaque));
}

fn make_bytes_result_func(name: &str, with_bytes_param: bool) -> FunctionDef {
    let params = if with_bytes_param {
        vec![ParamDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
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
        }]
    } else {
        vec![]
    };
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params,
        return_type: TypeRef::Bytes,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
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

fn make_bytes_result_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        doc: String::new(),
        params: vec![ParamDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
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
        }],
        return_type: TypeRef::Bytes,
        is_static: false,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        receiver: None,
        cfg: None,
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

#[test]
fn test_is_bytes_result_func_detects_bytes_with_error() {
    let func = make_bytes_result_func("process_image", true);
    assert!(is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_func_detects_bytes_without_error() {
    let mut func = make_bytes_result_func("get_data", false);
    func.error_type = None;
    assert!(is_bytes_result_func(&func));
}

#[test]
fn test_gen_function_wrapper_named_handle_error_path_uses_zero_sentinel() {
    let mut func = make_bytes_result_func("load_config", true);
    func.return_type = TypeRef::Named("Config".to_string());
    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "sample",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(out.contains("if ptr != 0"));
    assert!(!out.contains("if ptr != nil"));
}

#[test]
fn test_is_bytes_result_func_false_for_string_with_error() {
    let func = FunctionDef {
        name: "get_text".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
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
    assert!(!is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_method_detects_correctly() {
    let method = make_bytes_result_method("render_page");
    assert!(is_bytes_result_method(&method));
}

#[test]
fn test_gen_function_wrapper_bytes_result_emits_out_params() {
    let func = make_bytes_result_func("process_image", true);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let bridge_names: HashSet<String> = HashSet::new();
    let bridge_aliases: HashSet<String> = HashSet::new();
    let value_only_types: HashSet<String> = HashSet::new();
    let enum_names: HashSet<String> = HashSet::new();
    let ffi_param_enum_names: HashSet<String> = HashSet::new();
    let reserved_type_names: HashSet<String> = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &bridge_names,
        &bridge_aliases,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
        &reserved_type_names,
    );
    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
    assert!(out.contains("outLen"), "missing outLen in:\n{out}");
    assert!(out.contains("outCap"), "missing outCap in:\n{out}");
    assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
    assert!(out.contains("&outLen"), "missing &outLen in:\n{out}");
    assert!(out.contains("&outCap"), "missing &outCap in:\n{out}");
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
}

/// Regression: `bytes_result_call.jinja` used to free the native buffer inline, after
/// `C.GoBytes`, with no `defer` at all — a panic during `C.GoBytes` (e.g. an allocation
/// failure) skipped the free and leaked the buffer. The free must now be `defer`-registered
/// before the fallible conversion runs, matching every other free in this backend's templates
/// (`free_string.jinja`, `free_type.jinja`, `c_result_defer_free.jinja`). ~keep
#[test]
fn test_gen_function_wrapper_bytes_result_defers_free_before_conversion() {
    let func = make_bytes_result_func("process_image", true);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let bridge_names: HashSet<String> = HashSet::new();
    let bridge_aliases: HashSet<String> = HashSet::new();
    let value_only_types: HashSet<String> = HashSet::new();
    let enum_names: HashSet<String> = HashSet::new();
    let ffi_param_enum_names: HashSet<String> = HashSet::new();
    let reserved_type_names: HashSet<String> = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &bridge_names,
        &bridge_aliases,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
        &reserved_type_names,
    );

    let defer_pos = match out.find("defer C.krz_free_bytes") {
        Some(pos) => pos,
        None => panic!("free must be `defer`-registered, not called inline, in:\n{out}"),
    };
    let gobytes_pos = out.find("C.GoBytes").expect("missing C.GoBytes");
    assert!(
        defer_pos < gobytes_pos,
        "the buffer free must be deferred before the fallible C.GoBytes conversion runs, so a \
         panic during conversion still frees the native buffer, got:\n{out}"
    );
}

#[test]
fn test_gen_function_wrapper_infallible_bytes_uses_owned_buffer_abi() {
    let mut func = make_bytes_result_func("read_file", false);
    func.error_type = None;
    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "sample",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(out.contains("&outPtr, &outLen, &outCap"));
    assert!(out.contains("C.GoBytes"));
    assert!(!out.contains("unmarshalBytes"));
}

fn make_capsule_func(name: &str, fallible: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![make_param("name", TypeRef::String)],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: if fallible {
            Some("SampleCrateError".to_string())
        } else {
            None
        },
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

fn capsule_cfg() -> crate::core::config::HostCapsuleTypeConfig {
    crate::core::config::HostCapsuleTypeConfig {
        host_type: "*my_pkg.Language".to_string(),
        package: "github.com/example/go-my-lib".to_string(),
        package_version: "v1.0.0".to_string(),
        construct_expr: "my_pkg.NewLanguage(unsafe.Pointer({ptr}))".to_string(),
        ..Default::default()
    }
}

#[test]
fn test_capsule_fallible_returns_error_tuple_and_checks_last_error() {
    let func = make_capsule_func("get_language", true);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);
    assert!(
        out.contains("(*my_pkg.Language, error)"),
        "fallible capsule must return (host, error):\n{out}"
    );
    assert!(
        out.contains("lastError()"),
        "fallible capsule must check lastError():\n{out}"
    );
    assert!(
        out.contains("return nil, err"),
        "fallible capsule must propagate the error:\n{out}"
    );
}

#[test]
fn test_capsule_infallible_returns_bare_host_type() {
    let func = make_capsule_func("builtin_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);
    assert!(
        !out.contains(", error)"),
        "infallible capsule must not return an error:\n{out}"
    );
    assert!(
        !out.contains("lastError()"),
        "infallible capsule must not check lastError():\n{out}"
    );
}

#[test]
fn test_capsule_errors_when_construct_expr_empty() {
    let func = make_capsule_func("get_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: "*my_pkg.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: String::new(),
        ..Default::default()
    };
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &cfg, &empty_s);
    assert!(
        out.contains("ALEF ERROR"),
        "empty construct_expr must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("construct_expr"),
        "error must mention the missing field. Got:\n{out}"
    );
}

#[test]
fn test_capsule_errors_when_host_type_empty() {
    let func = make_capsule_func("get_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: String::new(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "my_pkg.NewLanguage(unsafe.Pointer({ptr}))".to_string(),
        ..Default::default()
    };
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &cfg, &empty_s);
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must mention the missing field. Got:\n{out}"
    );
}

/// Regression test for the Go backend emitting opaque-pointer idioms for values that cross
/// the FFI boundary as alef's scalar generational `AlefHandle`, per
/// `backends::ffi::type_map::{c_param_optional, c_return_optional}` (both map every
/// `TypeRef::Named` to `AlefHandle` unconditionally). Every `TypeRef::Named` param or return
/// value is a `uint64_t` handle in the emitted C header, never a `T*` pointer, so the Go side
/// must declare locals as the scalar `C.<PREFIX>AlefHandle` type and compare them to `0`, not
/// `nil`. Prior to the fix, this emitted `var cOptions *C.HTMConversionOptions` and
/// `cOptions == nil` / `ptr == nil`, which fails to compile under cgo.
#[test]
fn gen_convert_with_visitor_wrapper_uses_scalar_handle_not_opaque_pointer() {
    let func = FunctionDef {
        name: "convert".to_string(),
        params: vec![
            make_param("html", TypeRef::String),
            make_param("options", TypeRef::Named("ConversionOptions".to_string())),
        ],
        return_type: TypeRef::Named("ConversionResult".to_string()),
        error_type: Some("ConversionError".to_string()),
        ..Default::default()
    };
    let opaque_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        options_type: Some("ConversionOptions".to_string()),
        ..Default::default()
    };
    let reserved_type_names: HashSet<String> = HashSet::new();

    let out = gen_convert_with_visitor_wrapper(
        &func,
        "htm",
        &opaque_names,
        &value_only_types,
        &bridge_cfg,
        &reserved_type_names,
    );

    // Positive sanity check: prove this slice actually covers the generated function body,
    // so the negative assertions below are not vacuously true.
    assert!(
        out.contains("func Convert(") && out.contains("C.htm_convert("),
        "expected a full Convert wrapper body with an htm_convert FFI call, got:\n{out}"
    );

    assert!(
        out.contains("var cOptions C.HTMAlefHandle"),
        "cOptions must be declared as the scalar AlefHandle type, got:\n{out}"
    );
    assert!(
        !out.contains("*C.HTMConversionOptions"),
        "cOptions must not be declared as an opaque pointer to the options struct, got:\n{out}"
    );
    assert!(
        !out.contains("cOptions == nil"),
        "handle comparisons must use == 0, not nil, got:\n{out}"
    );
    assert!(
        out.contains("cOptions == 0"),
        "expected a scalar handle nil-check for cOptions, got:\n{out}"
    );
    assert!(
        !out.contains("ptr == nil"),
        "the convert result handle must not be compared to nil, got:\n{out}"
    );
    assert!(
        out.contains("ptr == 0"),
        "expected a scalar handle nil-check for the convert result, got:\n{out}"
    );
}

/// `Optional<Bytes>` shares one C signature with bare `Bytes`
/// (`(args…, uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap) -> int32_t`), with
/// absence carried by `*out_ptr == NULL`. Modelling it as a direct `*mut u8` return links
/// fine against that signature and only misreads at run time, so the Go wrapper must go
/// through the byte-buffer out-param path exactly as bare `Bytes` does.
#[test]
fn optional_bytes_return_reads_the_three_out_params_and_maps_null_to_nil() {
    let mut func = make_bytes_result_func("thumbnail", true);
    func.return_type = TypeRef::Optional(Box::new(TypeRef::Bytes));

    assert!(
        is_bytes_result_func(&func),
        "Optional<Bytes> must use the same owned-byte-buffer ABI as bare Bytes"
    );

    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    // Positive control: the fixture must actually render a wrapper body for this function,
    // so the ABI assertions below cannot pass over empty output. ~keep
    assert!(
        out.contains("func Thumbnail(") && out.contains("C.krz_thumbnail("),
        "fixture must emit a real wrapper calling the C symbol, got:\n{out}"
    );

    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr declaration in:\n{out}");
    assert!(out.contains("var outLen, outCap"), "missing outLen/outCap in:\n{out}");
    assert!(
        out.contains("&outPtr, &outLen, &outCap"),
        "the C call must pass all three out-params in:\n{out}"
    );
    assert!(
        out.contains("if outPtr == nil"),
        "absence is carried by a NULL out_ptr and must map to Go's nil slice, got:\n{out}"
    );
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
    assert!(
        !out.contains("ptr :="),
        "Optional<Bytes> must not fall through to the direct-pointer return path in:\n{out}"
    );
}

/// Control for the fix above: widening the predicate must not make every optional return
/// take the byte-buffer path. `Optional<String>` is a nullable `*mut c_char` on the C side
/// and must keep the direct-pointer shape — without this a predicate that answered `true`
/// for any `Optional` would still pass the test above.
#[test]
fn optional_string_return_keeps_the_direct_pointer_shape() {
    let mut func = make_bytes_result_func("caption", false);
    func.return_type = TypeRef::Optional(Box::new(TypeRef::String));

    assert!(
        !is_bytes_result_func(&func),
        "only Bytes / Optional<Bytes> use the owned-byte-buffer ABI"
    );

    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(
        out.contains("func Caption(") && out.contains("C.krz_caption("),
        "fixture must emit a real wrapper calling the C symbol, got:\n{out}"
    );
    assert!(
        !out.contains("outPtr"),
        "no byte out-params for Optional<String> in:\n{out}"
    );
    assert!(
        !out.contains("outCap"),
        "no byte out-params for Optional<String> in:\n{out}"
    );
    assert!(
        !out.contains("([]byte, error)"),
        "Optional<String> must not be typed as []byte in:\n{out}"
    );
}

/// Regression test: a real-world consumer (an async free function returning `Vec<Named>`
/// over a `Named` config-struct parameter, e.g. `embed_sparse_async(texts, config)`) reported
/// its Go wrapper missing. `is_async` is never read by `gen_function_wrapper` — every FFI call
/// it emits is a blocking C call regardless of the Rust function's async-ness — so nothing here
/// should special-case async, but nothing previously exercised the async + Vec-return +
/// struct-param combination together, so a future accidental async gate would go unnoticed. ~keep
#[test]
fn async_free_function_returning_vec_of_named_over_config_struct_emits_wrapper() {
    let func = FunctionDef {
        name: "embed_sparse_async".to_string(),
        params: vec![
            make_param("texts", TypeRef::Vec(Box::new(TypeRef::String))),
            make_param("config", TypeRef::Named("SparseEmbeddingConfig".to_string())),
        ],
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("SparseEmbedding".to_string()))),
        is_async: true,
        error_type: Some("SampleCrateError".to_string()),
        ..Default::default()
    };
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_strings = HashSet::new();

    let out = gen_function_wrapper(
        &func,
        "sample",
        &opaque,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(
        out.contains(
            "func EmbedSparseAsync(texts []string, config SparseEmbeddingConfig) ([]SparseEmbedding, error) {"
        ),
        "expected the exact async Vec<Named>-over-config-struct signature, got:\n{out}"
    );
    assert!(
        out.contains("C.sample_embed_sparse_async("),
        "must call the FFI symbol for the underlying blocking C call, got:\n{out}"
    );
    assert!(
        out.contains("json.Unmarshal") && out.contains("[]SparseEmbedding"),
        "Vec<Named> return must unmarshal into a Go slice, got:\n{out}"
    );
}

// --- issue #380: `&mut T` DTO write-back --------------------------------------------------

fn make_mut_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        is_ref: true,
        is_mut: true,
        ..make_param(name, TypeRef::Named(type_name.to_string()))
    }
}

/// `fn tag_record(record: &mut Record)` must not silently drop the mutation: the generated
/// wrapper builds a temporary handle from JSON, calls the FFI mutator, reads the handle back
/// out via `_to_json`, and returns the decoded value. Asserting only that the signature grew
/// an `error` return proves nothing — the load-bearing check is the full round trip in the
/// rendered body, in the right order: call, then read-back, then decode, then return.
#[test]
fn test_gen_function_wrapper_mut_dto_param_writes_back_the_mutated_value() {
    let func = FunctionDef {
        name: "tag_record".to_string(),
        params: vec![make_mut_param("record", "Record")],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    };
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_strings = HashSet::new();

    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(
        out.contains("func TagRecord(record Record) (*Record, error) {"),
        "expected the write-back signature (record by value in, *Record + error out), got:\n{out}"
    );
    assert!(
        out.contains("cRecord := C.krz_record_from_json(tmpStrcRecord)"),
        "must still build the temporary handle from the caller's value, got:\n{out}"
    );
    assert!(
        out.contains("defer C.krz_record_free(cRecord)"),
        "must still free the temporary handle, got:\n{out}"
    );
    assert!(
        out.contains("C.krz_tag_record(cRecord)"),
        "must still call the FFI mutator, got:\n{out}"
    );
    assert!(
        out.contains("jsonPtr := C.krz_record_to_json(cRecord)"),
        "must read the mutated handle back out via the FFI's _to_json helper, got:\n{out}"
    );
    assert!(
        out.contains("var result Record")
            && out.contains("json.Unmarshal([]byte(C.GoString(jsonPtr)), &result)")
            && out.contains("return &result, nil"),
        "must decode the read-back JSON into a fresh Record and return it, got:\n{out}"
    );

    // Ordering: mutate, then read back, then free (defer fires at return regardless of
    // where in the body it is registered, so it is correct for the `defer` line to appear
    // earlier in source than the read-back -- what must not happen is the read-back being
    // itself deferred, or the mutating call appearing after the read-back).
    let call_pos = out.find("C.krz_tag_record(cRecord)").expect("call site");
    let readback_pos = out
        .find("jsonPtr := C.krz_record_to_json(cRecord)")
        .expect("read-back site");
    assert!(
        call_pos < readback_pos,
        "the mutating call must run before the read-back, got:\n{out}"
    );
    assert!(
        !out.contains("defer C.krz_record_to_json") && !out.contains("defer jsonPtr"),
        "the read-back must run as a plain statement, not be deferred, got:\n{out}"
    );

    // The pre-fix shape emitted the call and returned immediately, discarding the mutation.
    assert!(
        !out.contains("C.krz_tag_record(cRecord)\n\treturn nil\n}"),
        "must not regress to the lossy call-then-return-nil shape, got:\n{out}"
    );
}

/// Negative control: an immutable `&Record` DTO parameter must NOT gain a read-back. The host
/// already never mutates through it, so the pre-existing "call then return" shape is correct.
#[test]
fn test_gen_function_wrapper_immutable_dto_param_gets_no_writeback() {
    let func = FunctionDef {
        name: "read_record".to_string(),
        params: vec![ParamDef {
            is_ref: true,
            is_mut: false,
            ..make_param("record", TypeRef::Named("Record".to_string()))
        }],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    };
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_strings = HashSet::new();

    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(
        out.contains("func ReadRecord(record Record) error {"),
        "an immutable DTO param must keep the plain error-only signature, got:\n{out}"
    );
    assert!(
        !out.contains("_to_json"),
        "an immutable DTO param must not read anything back, got:\n{out}"
    );
    assert!(
        out.contains("C.krz_read_record(cRecord)\n\treturn nil\n}"),
        "must keep the original call-then-return-nil shape, got:\n{out}"
    );
}

/// Negative control: an owned (by-value) DTO parameter must render byte-for-byte the same as
/// before this fix -- the write-back path must never trigger for a parameter that was never a
/// reference in the first place.
#[test]
fn test_gen_function_wrapper_owned_dto_param_is_unchanged() {
    let func = FunctionDef {
        name: "consume_and_tag_record".to_string(),
        params: vec![make_param("record", TypeRef::Named("Record".to_string()))],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    };
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_strings = HashSet::new();

    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(
        out.contains("func ConsumeAndTagRecord(record Record) error {"),
        "an owned DTO param must keep the plain error-only signature, got:\n{out}"
    );
    assert!(
        !out.contains("_to_json"),
        "an owned DTO param must not read anything back, got:\n{out}"
    );
    assert_eq!(
        out,
        concat!(
            "// ConsumeAndTagRecord calls the FFI function.\n",
            "func ConsumeAndTagRecord(record Record) error {\n",
            "\truntime.LockOSThread()\n",
            "\tdefer runtime.UnlockOSThread()\n",
            "\tjsonBytescRecord, err := json.Marshal(record)\n",
            "\tif err != nil {\n",
            "\t\treturn fmt.Errorf(\"failed to marshal: %w\", err)\n",
            "\t}\n",
            "\t// When the parameter is a nil pointer (Option<&T> on the Rust side), json.Marshal\n",
            "\t// emits \"null\" which the FFI's _from_json rejects. Substitute \"{}\" so a default\n",
            "\t// instance is constructed instead \u{2014} semantically equivalent to None for query types\n",
            "\t// whose fields are all optional with serde(default).\n",
            "\tif string(jsonBytescRecord) == \"null\" {\n",
            "\t\tjsonBytescRecord = []byte(\"{}\")\n",
            "\t}\n",
            "\ttmpStrcRecord := C.CString(string(jsonBytescRecord))\n",
            "\tcRecord := C.krz_record_from_json(tmpStrcRecord)\n",
            "\tC.free(unsafe.Pointer(tmpStrcRecord))\n",
            "\tif cRecord == 0 {\n",
            "\t\treturn fmt.Errorf(\"failed to create record: %s\", C.GoString(C.krz_last_error_context()))\n",
            "\t}\n",
            "\tdefer C.krz_record_free(cRecord)\n",
            "\n",
            "\n",
            "\tC.krz_consume_and_tag_record(cRecord)\n",
            "\treturn nil\n",
            "}\n",
        ),
        "owned DTO param rendering must be byte-for-byte unchanged by the write-back fix, got:\n{out}"
    );
}

/// `reject_unsupported_writeback` must fire through the real backend path (not just the
/// codegen/mut_writeback unit tests) for a `&mut` DTO param combined with a non-unit return --
/// that shape has nowhere to put the return value once the write-back slot is taken.
#[test]
fn test_reject_unsupported_writeback_fires_for_mut_param_with_non_unit_return() {
    let func = FunctionDef {
        name: "tag_and_count".to_string(),
        params: vec![make_mut_param("record", "Record")],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        error_type: None,
        ..Default::default()
    };
    let opaque: ahash::AHashSet<String> = ahash::AHashSet::default();
    let result = crate::codegen::mut_writeback::reject_unsupported_writeback(
        &func.name,
        &func.params,
        &func.return_type,
        &opaque,
    );
    let err = result.expect_err("a &mut DTO param with a non-unit return must be rejected");
    let message = err.to_string();
    assert!(
        message.contains("tag_and_count"),
        "the diagnostic must name the offending function, got: {message}"
    );
}

// -- issue #439: LockOSThread around the FFI call + lastError() read + result conversion -- ~keep

/// #439: the native layer stores the last error per OS thread, but the generated wrapper reads
/// it with a *separate* cgo call (`lastError()`) after the FFI call that actually set it.
/// Nothing pinned the goroutine to one OS thread between the two calls, so the Go runtime could
/// resume the goroutine on a different thread and read that thread's (empty or stale) error
/// slot. The fix holds `runtime.LockOSThread()`/`defer runtime.UnlockOSThread()` across the
/// call, the `lastError()` read, and the result conversion that follows. This exercises the
/// full round trip for a fallible free function returning a non-opaque `Named` type, which is
/// the shape that also does a JSON conversion after the error check. ~keep
#[test]
fn test_gen_function_wrapper_locks_os_thread_across_call_read_and_convert() {
    let func = FunctionDef {
        name: "get_widget".to_string(),
        return_type: TypeRef::Named("Widget".to_string()),
        error_type: Some("SampleCrateError".to_string()),
        ..Default::default()
    };
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_strings = HashSet::new();

    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    let signature_pos = out.find("func GetWidget(").expect("signature must be emitted");
    let lock_pos = out.find("runtime.LockOSThread()").expect("must lock the OS thread");
    let call_pos = out.find("C.krz_get_widget(").expect("must call the FFI symbol");
    let last_error_pos = out.find("if err := lastError()").expect("must read lastError()");
    let unmarshal_pos = out.find("json.Unmarshal").expect("must convert the JSON result");

    assert!(
        signature_pos < lock_pos && lock_pos < call_pos && call_pos < last_error_pos && last_error_pos < unmarshal_pos,
        "expected lock, then call, then lastError() read, then conversion, in that order, got:\n{out}"
    );
    assert!(
        out.contains("defer runtime.UnlockOSThread()"),
        "the unlock must be deferred so every return path in the body releases it, got:\n{out}"
    );
    assert_eq!(
        out.matches("runtime.LockOSThread()").count(),
        1,
        "a synchronous wrapper must lock exactly once, got:\n{out}"
    );
}

#[test]
fn test_capsule_wrapper_locks_os_thread_when_fallible() {
    let func = make_capsule_func("get_language", true);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);

    let lock_pos = out.find("runtime.LockOSThread()").expect("must lock the OS thread");
    let call_pos = out
        .find("cLang := C.krz_get_language(")
        .expect("must call the FFI symbol");
    let last_error_pos = out.find("if err := lastError()").expect("must read lastError()");
    assert!(
        lock_pos < call_pos && call_pos < last_error_pos,
        "expected lock, then call, then lastError() read, got:\n{out}"
    );
    assert!(
        out.contains("defer runtime.UnlockOSThread()"),
        "the unlock must be deferred, got:\n{out}"
    );
}

/// The options-field visitor wrapper makes up to three separate FFI calls that each may set
/// the per-thread last error (`_from_json` for the options, `_from_json` for the default
/// options, and the conversion call itself); all three checks and the final JSON conversion
/// must run under one lock, not three independent ones.
#[test]
fn test_convert_with_visitor_wrapper_locks_os_thread_once_around_every_ffi_call() {
    let func = FunctionDef {
        name: "convert".to_string(),
        params: vec![
            make_param("html", TypeRef::String),
            make_param("options", TypeRef::Named("ConversionOptions".to_string())),
        ],
        return_type: TypeRef::Named("ConversionResult".to_string()),
        error_type: Some("ConversionError".to_string()),
        ..Default::default()
    };
    let opaque_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        options_type: Some("ConversionOptions".to_string()),
        ..Default::default()
    };
    let reserved_type_names: HashSet<String> = HashSet::new();

    let out = gen_convert_with_visitor_wrapper(
        &func,
        "htm",
        &opaque_names,
        &value_only_types,
        &bridge_cfg,
        &reserved_type_names,
    );

    let signature_pos = out.find("func Convert(").expect("signature must be emitted");
    let lock_pos = out.find("runtime.LockOSThread()").expect("must lock the OS thread");
    let call_pos = out.find("C.htm_convert(").expect("must call the FFI conversion symbol");
    assert!(
        signature_pos < lock_pos && lock_pos < call_pos,
        "the lock must be established before any of the wrapper's FFI calls, got:\n{out}"
    );
    assert_eq!(
        out.matches("runtime.LockOSThread()").count(),
        1,
        "one lock must cover every FFI call this wrapper makes, not one per call, got:\n{out}"
    );
    assert!(
        out.contains("defer runtime.UnlockOSThread()"),
        "the unlock must be deferred so it covers the final JSON conversion too, got:\n{out}"
    );
}
