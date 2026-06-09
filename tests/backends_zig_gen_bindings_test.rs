use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
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
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,

        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    }
}

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_trait_bridge_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.trait_bridges]]
trait_name = "Renderer"
register_fn = "register_renderer"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_trait_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    }
}

#[test]
fn struct_emits_zig_struct() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Point",
            vec![
                make_field("x", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("y", TypeRef::Primitive(PrimitiveType::I32), false),
            ],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    assert!(
        content.contains("@cImport(@cInclude(\"demo.h\"))"),
        "missing cImport: {content}"
    );
    assert!(content.contains("pub const Point = struct {"));
    assert!(content.contains("x: i32,"));
    assert!(content.contains("y: i32,"));
}

#[test]
fn trait_bridge_complex_return_is_explicitly_unsupported() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_trait_type(
            "Renderer",
            vec![MethodDef {
                name: "render".into(),
                params: vec![make_param("input", TypeRef::String)],
                return_type: TypeRef::Bytes,
                is_async: false,
                is_static: false,
                error_type: Some("RenderError".into()),
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
            }],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_trait_bridge_config()).unwrap();
    let content = &files[0].content;

    // The Zig backend currently emits a placeholder thunk carrying an unsupported marker
    // for complex trait-vtable return types — the prior @compileError contract
    // was softened so downstream packages compile even when a slot is not yet
    // wired. The unsupported marker is the load-bearing invariant: it ensures a
    // regression to a silent default still surfaces. See alef issue tracking
    // JSON serialization for complex trait-vtable return types.
    assert!(
        content.contains("Unsupported: JSON serialization for this complex return type"),
        "complex Zig trait-vtable return must carry an unsupported marker so it isn't silently shipped: {content}"
    );
}

/// String parameter: wrapper takes `[]const u8`; body allocates a null-terminated
/// copy via `std.fmt.allocPrintSentinel` and frees it after the C call.
#[test]
fn string_param_allocates_z_string_and_frees() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "greet".into(),
            rust_path: "demo::greet".into(),
            original_rust_path: String::new(),
            params: vec![make_param("who", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::I32),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // Wrapper signature uses []const u8 (Zig slice), not the C sentinel-terminated form.
    assert!(
        content.contains("pub fn greet(who: []const u8)"),
        "wrapper must accept []const u8 for String param: {content}"
    );
    // Body allocates a null-terminated copy.
    assert!(
        content.contains("allocPrintSentinel") && content.contains("who_z"),
        "body must allocate a null-terminated copy: {content}"
    );
    // The null-terminated copy is passed to the C function.
    assert!(
        content.contains("c.demo_greet(who_z)"),
        "C call must use who_z: {content}"
    );
    // The allocation is freed after the call.
    assert!(
        content.contains("c_allocator.free") && content.contains("who_z"),
        "body must free the null-terminated copy: {content}"
    );
}

/// Bytes parameter: wrapper takes `[]const u8`; body passes `.ptr` and `.len`
/// as separate arguments matching the C ABI (`*const u8`, `usize`).
#[test]
fn bytes_param_passes_ptr_and_len() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process".into(),
            rust_path: "demo::process".into(),
            original_rust_path: String::new(),
            params: vec![make_param("data", TypeRef::Bytes)],
            return_type: TypeRef::Unit,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // Wrapper signature uses []const u8.
    assert!(
        content.contains("pub fn process(data: []const u8)"),
        "wrapper must accept []const u8 for Bytes param: {content}"
    );
    // Body passes data.ptr and data.len as separate C arguments.
    assert!(
        content.contains("data.ptr") && content.contains("data.len"),
        "body must pass .ptr and .len for Bytes: {content}"
    );
}

/// Vec<T> parameter: wrapper takes `[]const u8` (caller supplies JSON).
/// Body allocates a null-terminated copy to pass to the C string parameter.
#[test]
fn vec_param_takes_json_slice() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "upload".into(),
            rust_path: "demo::upload".into(),
            original_rust_path: String::new(),
            params: vec![make_param(
                "items",
                TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
            )],
            return_type: TypeRef::Unit,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // Wrapper parameter is []const u8 (JSON).
    assert!(
        content.contains("pub fn upload(items: []const u8)"),
        "Vec param must be []const u8 (JSON): {content}"
    );
    // Body allocates a null-terminated copy.
    assert!(
        content.contains("allocPrintSentinel") && content.contains("items_z"),
        "body must allocate null-terminated copy for Vec param: {content}"
    );
}

/// Result-returning function: wrapper emits an error union return type and
/// checks `last_error_code()` after the C call (not a brittle `result == null`
/// comparison that does not typecheck in Zig).
#[test]
fn result_function_checks_last_error_code() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract".into(),
            rust_path: "demo::extract".into(),
            original_rust_path: String::new(),
            params: vec![make_param("path", TypeRef::String)],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("DemoError".into()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Connection".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // Return type must include the error union.
    assert!(
        content.contains("DemoError") && content.contains("!"),
        "must emit error-union return type: {content}"
    );
    // Error check uses last_error_code(), not a broken `result == null or result == 0`.
    assert!(
        content.contains("last_error_code() != 0"),
        "must check last_error_code() for error detection: {content}"
    );
    assert!(
        !content.contains("result == null or result == 0"),
        "must NOT emit the broken null/0 check: {content}"
    );
    // C call is present.
    assert!(content.contains("c.demo_extract("), "must call C function: {content}");
}

/// Async Rust functions ARE emitted as synchronous Zig wrappers.
/// The Zig C FFI uses block_on internally, so every function is synchronous
/// from Zig's perspective regardless of the Rust `async` annotation.
#[test]
fn async_function_is_emitted_as_sync() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch_async".into(),
            rust_path: "demo::fetch_async".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // No "async unsupported" warning should appear — all functions are sync via C FFI.
    assert!(
        !content.contains("Async functions are not supported in this backend."),
        "must NOT emit async-unsupported comment: {content}"
    );
    // The wrapper function must be emitted.
    assert!(
        content.contains("pub fn fetch_async"),
        "must emit async function wrapper as sync: {content}"
    );
}

/// Standard helpers `_free_string` and `_last_error` are always emitted.
#[test]
fn helpers_are_always_emitted() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub fn _free_string"),
        "must emit _free_string helper: {content}"
    );
    assert!(
        content.contains("pub fn _last_error"),
        "must emit _last_error helper: {content}"
    );
    assert!(
        content.contains("demo_free_string"),
        "_free_string must call the prefixed C symbol: {content}"
    );
    assert!(
        content.contains("demo_last_error_code"),
        "_last_error must call the prefixed C symbol: {content}"
    );
}

#[test]
fn enum_emits_zig_enum_or_union() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".into(),
            rust_path: "demo::Status".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Inactive".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("pub const Status = enum {"));
    assert!(content.contains("active,"));
    assert!(content.contains("inactive,"));
}

#[test]
fn optional_field_uses_zig_optional_syntax() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Maybe",
            vec![make_field("value", TypeRef::Optional(Box::new(TypeRef::String)), false)],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("value: ?[]const u8,"), "missing optional: {content}");
}

#[test]
fn error_set_emits_zig_error_with_pascal_case_tags() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "connection_failed".into(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "timeout".into(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("pub const DemoError = error{"),
        "missing error set definition: {content}"
    );
    assert!(
        content.contains("ConnectionFailed,"),
        "missing ConnectionFailed tag: {content}"
    );
    assert!(content.contains("Timeout,"), "missing Timeout tag: {content}");
}

/// Opaque handle types with no methods (e.g. Language) must still be emitted
/// as a Zig struct so functions that return them compile without
/// "use of undeclared identifier" errors.
#[test]
fn opaque_handle_with_no_methods_is_emitted() {
    // Language is an opaque type with no instance methods — it is a bare
    // newtype around a C pointer returned by get_language(). Before the fix,
    // the emission loop filtered on `!t.methods.is_empty()`, silently skipping
    // it and causing Zig to reject functions whose return type is `Language`.
    let language_type = TypeDef {
        name: "Language".to_string(),
        rust_path: "demo::Language".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![], // <-- no methods: the key regression trigger
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: "A tree-sitter language handle.".to_string(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let get_language_fn = FunctionDef {
        name: "get_language".to_string(),
        rust_path: "demo::get_language".to_string(),
        original_rust_path: String::new(),
        params: vec![make_param("name", TypeRef::String)],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: Some("DemoError".to_string()),
        doc: "Get a language by name.".to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![language_type],
        functions: vec![get_language_fn],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // The Language struct must be declared even though it has no methods.
    assert!(
        content.contains("pub const Language = struct {"),
        "opaque handle with no methods must still be emitted as a Zig struct: {content}"
    );
    // It must have the _handle field.
    assert!(
        content.contains("_handle: *anyopaque,"),
        "opaque handle struct must have _handle field: {content}"
    );
    // get_language must reference the declared Language type.
    assert!(
        content.contains("pub fn get_language("),
        "get_language function must be emitted: {content}"
    );
    // The function return type must reference Language by name.
    assert!(
        content.contains(")!Language") || content.contains("Language {"),
        "get_language return type or body must reference Language: {content}"
    );
}

/// A function that returns `bool` wraps the C `i32` result with `!= 0` so the
/// Zig compiler does not reject the implicit i32→bool coercion.
///
/// The C ABI represents `bool` as `int` (i32). Zig's type system is strict and
/// does not allow assigning an `i32` to a `bool` variable. The Zig backend must
/// emit `return _result != 0;` (or `return _result != 0` in an infallible body).
#[test]
fn bool_return_emits_not_zero_conversion() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "has_feature".into(),
            rust_path: "demo::has_feature".into(),
            original_rust_path: String::new(),
            params: vec![make_param("name", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            error_type: None,
            doc: "Check whether a feature is enabled.".into(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // The wrapper return type must be `bool`.
    assert!(
        content.contains(") error{OutOfMemory}!bool") || content.contains(") bool"),
        "return type must be bool: {content}"
    );
    // The C function result must be converted with `!= 0` so Zig accepts it.
    assert!(
        content.contains("_result != 0"),
        "bool return must emit `_result != 0` conversion: {content}"
    );
    // Must NOT return the raw `_result` (i32) directly — that would fail to
    // compile because Zig does not coerce i32 to bool.
    assert!(
        !content.contains("return _result;"),
        "must NOT return raw _result (i32) for bool return: {content}"
    );
}

/// A fallible function that returns `bool` (error union `!bool`) also emits the
/// `!= 0` conversion so that both the fallible and infallible paths are covered.
#[test]
fn bool_return_in_error_union_emits_not_zero_conversion() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "check_auth".into(),
            rust_path: "demo::check_auth".into(),
            original_rust_path: String::new(),
            params: vec![make_param("token", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            error_type: Some("DemoError".into()),
            doc: "Check auth token validity.".into(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Unauthorized".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // The wrapper return type must be an error union over bool.
    assert!(
        content.contains("!bool"),
        "fallible bool return type must include !bool: {content}"
    );
    // The C function result must be converted with `!= 0`.
    assert!(
        content.contains("_result != 0"),
        "fallible bool return must emit `_result != 0` conversion: {content}"
    );
}

/// An infallible function with a String parameter must emit `defer` free
/// immediately after the allocPrintSentinel call, so the sentinel buffer is
/// alive when the C function is called.
///
/// Regression test for the free-before-use bug: previously the codegen emitted
/// `c_allocator.free(name_z)` before the C call, which passed a dangling pointer.
#[test]
fn string_param_infallible_defers_free_after_c_call() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "has_feature".into(),
            rust_path: "demo::has_feature".into(),
            original_rust_path: String::new(),
            params: vec![make_param("name", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // `defer` must appear after allocPrintSentinel and before the C call.
    let alloc_pos = content
        .find("allocPrintSentinel")
        .expect("must allocate sentinel string");
    let defer_pos = content.find("defer std.heap.c_allocator.free(name_z)");
    let c_call_pos = content.find("c.demo_has_feature(name_z)");

    assert!(
        defer_pos.is_some(),
        "must emit `defer std.heap.c_allocator.free(name_z)` for infallible String param: {content}"
    );
    let defer_pos = defer_pos.unwrap();
    let c_call_pos = c_call_pos.expect("C call must use name_z as argument: {content}");

    assert!(
        alloc_pos < defer_pos,
        "defer must come after allocPrintSentinel: {content}"
    );
    assert!(
        defer_pos < c_call_pos,
        "defer must come before the C call (free-before-use bug): {content}"
    );

    // Must NOT have a bare (non-deferred) free before the C call.
    let pre_call = &content[..c_call_pos];
    assert!(
        !pre_call.contains("c_allocator.free(name_z)") || pre_call.contains("defer std.heap.c_allocator.free(name_z)"),
        "must not emit bare (non-deferred) free before C call: {content}"
    );
}

/// Error set must include `OutOfMemory` as a variant so allocator failures can be
/// propagated without requiring a `||error{OutOfMemory}` concat on every return type.
/// Return types for fallible functions must be `ErrorSet!T`, not `(ErrorSet||error{OutOfMemory})!T`.
#[test]
fn error_set_includes_out_of_memory_and_return_type_is_single_error_set() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_bytes".into(),
            rust_path: "demo::extract_bytes".into(),
            original_rust_path: String::new(),
            params: vec![make_param("bytes", TypeRef::Bytes)],
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: Some("DemoError".into()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Extraction".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // Return type must be the single error set, not a double union concat.
    assert!(
        content.contains("DemoError![]u8"),
        "return type must be single error set DemoError![]u8, got: {content}"
    );
    // Must NOT contain the verbose double error union concat.
    assert!(
        !content.contains("||error{OutOfMemory}"),
        "must NOT emit ||error{{OutOfMemory}} concat: {content}"
    );
    // OutOfMemory must be present as a variant in the DemoError set definition.
    assert!(
        content.contains("OutOfMemory,"),
        "DemoError must include OutOfMemory variant: {content}"
    );
}

/// A fallible function with a String parameter must also defer the free, so
/// the sentinel buffer is alive across the C call AND the error-code check.
#[test]
fn string_param_fallible_defers_free_after_c_call() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "lookup".into(),
            rust_path: "demo::lookup".into(),
            original_rust_path: String::new(),
            params: vec![make_param("key", TypeRef::String)],
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
            is_async: false,
            error_type: Some("DemoError".into()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    let alloc_pos = content
        .find("allocPrintSentinel")
        .expect("must allocate sentinel string");
    let defer_pos = content.find("defer std.heap.c_allocator.free(key_z)");
    let c_call_pos = content.find("c.demo_lookup(key_z)");

    assert!(
        defer_pos.is_some(),
        "must emit `defer std.heap.c_allocator.free(key_z)` for fallible String param: {content}"
    );
    let defer_pos = defer_pos.unwrap();
    let c_call_pos = c_call_pos.expect("C call must use key_z as argument");

    assert!(
        alloc_pos < defer_pos,
        "defer must come after allocPrintSentinel: {content}"
    );
    assert!(defer_pos < c_call_pos, "defer must come before the C call: {content}");
}

#[test]
fn string_return_uses_len_companion_and_pointer_slice() {
    // Verifies: when a free function returns a `*mut c_char`-mapped type
    // (String/Path/Json/Vec/Map), the Zig wrapper pairs the primary call with
    // alef-backend-ffi's `_len()` companion and builds an exact-length slice via
    // `ptr[0..len]` — no `std.mem.sliceTo`/sentinel scan required.
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "describe".into(),
            rust_path: "demo::describe".into(),
            original_rust_path: String::new(),
            params: vec![make_param("topic", TypeRef::String)],
            return_type: TypeRef::String,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("topic: []const u8"),
        "String param must map to []const u8 (no :0 sentinel): {content}"
    );
    assert!(
        content.contains("const _result = c.demo_describe(topic_z);"),
        "primary C call must be captured into _result: {content}"
    );
    assert!(
        content.contains("const _result_len = c.demo_describe_len(topic_z);"),
        "_len() companion must be called with the same args and captured into _result_len: {content}"
    );
    assert!(
        content.contains("const slice = _result[0.._result_len];"),
        "wrapper must slice the C pointer with ptr[0..len] (no sentinel scan): {content}"
    );
    assert!(
        !content.contains("std.mem.sliceTo(_result, 0)"),
        "wrapper must not NUL-scan _result: {content}"
    );
}

#[test]
fn optional_string_return_uses_len_companion_with_null_guard() {
    // Verifies: `Option<String>` returns gate the slice construction on a null
    // check of `_result`, then build `ptr[0..len]` from the `_len()` companion.
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "lookup".into(),
            rust_path: "demo::lookup".into(),
            original_rust_path: String::new(),
            params: vec![make_param("key", TypeRef::String)],
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("const _result_len = c.demo_lookup_len(key_z);"),
        "optional-string return must also call the _len() companion: {content}"
    );
    assert!(
        content.contains("if (_result == null) break :blk null;"),
        "optional return must guard slice construction on a null check: {content}"
    );
    assert!(
        content.contains("const slice = _result[0.._result_len];"),
        "optional return must slice _result[0.._result_len] after the null check: {content}"
    );
}

#[test]
fn from_json_params_check_null_and_defer_handle_cleanup() {
    let config_type = TypeDef {
        name: "Config".into(),
        rust_path: "demo::Config".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![config_type],
        functions: vec![FunctionDef {
            name: "configure".into(),
            rust_path: "demo::configure".into(),
            original_rust_path: String::new(),
            params: vec![make_param("config", TypeRef::Named("Config".into()))],
            return_type: TypeRef::String,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub fn configure(config: []const u8) error{OutOfMemory,InvalidJson}![]u8"),
        "from_json failure must be part of the generated error union: {content}"
    );
    assert!(
        content.contains("const config_handle = c.demo_config_from_json(config_z);"),
        "must create a handle via _from_json: {content}"
    );
    assert!(
        content.contains("if (config_handle == null) return error.InvalidJson;"),
        "must check _from_json handle creation before the primary call: {content}"
    );
    assert!(
        content.contains("defer c.demo_config_free(config_handle);"),
        "non-null _from_json handles must be cleaned up with defer: {content}"
    );
}

#[test]
fn client_constructors_emits_create_function() {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[workspace.client_constructors.DefaultClient]
body = "demo::DefaultClient::new(api_key)"
error_type = "String"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "demo::DefaultClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub fn create_default_client("),
        "should emit create_default_client function: {content}"
    );
    assert!(
        content.contains("api_key: []const u8"),
        "string param should map to []const u8: {content}"
    );
    assert!(
        content.contains("c.demo_default_client_new("),
        "should call FFI constructor: {content}"
    );
    assert!(
        content.contains("_first_error(anyerror)"),
        "should return error on null handle: {content}"
    );
}

/// A streaming adapter owned by an opaque handle type must emit a Zig wrapper
/// method that uses the iterator-handle pattern (`_start` / `_next` / `_free`)
/// and accumulates every chunk into a JSON array — not the generic single-call
/// wrapper, and not a last-chunk-only emission. Regression coverage for the
/// audit that previously reported streaming missing on `CrawlEngineHandle`.
#[test]
fn streaming_adapter_emits_iterator_pattern_on_opaque_handle() {
    let toml = r#"
[workspace]
languages = ["zig", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.adapters]]
name = "crawl_stream"
pattern = "streaming"
core_path = "demo::crawl_stream"
owner_type = "CrawlEngineHandle"
item_type = "CrawlEvent"
error_type = "DemoError"
request_type = "demo::CrawlStreamRequest"

[[crates.adapters.params]]
name = "req"
type = "CrawlStreamRequest"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    // The IR method name must match the adapter `name` for the zig backend to
    // recognise it as streaming (see `streaming_item_types` map in mod.rs).
    let crawl_stream_method = MethodDef {
        name: "crawl_stream".into(),
        params: vec![make_param("req", TypeRef::Named("CrawlStreamRequest".into()))],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: Some("DemoError".into()),
        doc: "Stream crawl events for a single URL.".into(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let engine_type = TypeDef {
        name: "CrawlEngineHandle".into(),
        rust_path: "demo::CrawlEngineHandle".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![crawl_stream_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![engine_type],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Network".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Streaming struct type must be emitted (before the opaque handle).
    assert!(
        content.contains("pub const CrawlEventStream = struct {"),
        "must emit CrawlEventStream struct type: {content}"
    );
    // Struct must have _handle field holding the FFI stream handle.
    assert!(
        content.contains("_handle: *c.DEMOCrawlEventStream,"),
        "struct must have _handle field with FFI stream type: {content}"
    );
    // Struct must have next() method that returns optional item or error.
    assert!(
        content.contains("pub fn next(self: *CrawlEventStream)"),
        "struct must have next() method: {content}"
    );
    assert!(
        content.contains("!?CrawlEvent"),
        "next() must return error union of optional item: {content}"
    );
    // next() must call _next to fetch a chunk.
    assert!(
        content.contains("c.demo_crawl_engine_handle_crawl_stream_next(self._handle)"),
        "next() must call _next to fetch the next chunk: {content}"
    );
    // next() must distinguish clean EOS (null + errno==0) from mid-stream error (null + errno!=0).
    // The emitter uses the canonical `c.{prefix}_last_error_code() != 0` form rather than an
    // undefined `_has_error()` helper.
    assert!(
        content.contains("if (c.demo_last_error_code() != 0) return _first_error(DemoError);"),
        "next() must check error state on null chunk via last_error_code: {content}"
    );
    assert!(
        content.contains("return null;"),
        "next() must return null on clean EOS: {content}"
    );
    // next() must parse the chunk to the item type via `std.json.parseFromSliceLeaky`.
    assert!(
        content.contains("std.json.parseFromSliceLeaky(CrawlEvent,"),
        "next() must parse JSON to item type via parseFromSliceLeaky: {content}"
    );
    // Struct must have deinit() method to release the stream handle.
    assert!(
        content.contains("pub fn deinit(self: *CrawlEventStream) void {"),
        "struct must have deinit() method: {content}"
    );
    assert!(
        content.contains("c.demo_crawl_engine_handle_crawl_stream_free(self._handle)"),
        "deinit() must call _free to release the stream handle: {content}"
    );
    // Streaming wrapper method must be emitted on the CrawlEngineHandle struct.
    assert!(
        content.contains("pub fn crawl_stream(self: *CrawlEngineHandle"),
        "must emit streaming wrapper on opaque handle: {content}"
    );
    // Return type must be the struct (not a JSON array).
    assert!(
        content.contains("!CrawlEventStream {"),
        "streaming return type must be `!CrawlEventStream` (not `![]u8`): {content}"
    );
    // Body must build the request handle from JSON via the request_type's _from_json.
    assert!(
        content.contains("c.demo_crawl_stream_request_from_json("),
        "must build request handle from JSON: {content}"
    );
    // Body must call the iterator `_start` symbol to begin the stream.
    assert!(
        content.contains("c.demo_crawl_engine_handle_crawl_stream_start("),
        "must call `_start` to begin the stream: {content}"
    );
    // Body must return the struct (not defer-free it).
    assert!(
        content.contains("return CrawlEventStream{ ._handle = _stream_handle };"),
        "must return the stream struct (caller owns it via deinit()): {content}"
    );
    // Must NOT eagerly collect chunks into a JSON array.
    assert!(
        !content.contains("while (true) {"),
        "must NOT eagerly loop over chunks in the binding function: {content}"
    );
    assert!(
        !content.contains("try _buf.append(std.heap.c_allocator, '[')"),
        "must NOT build a JSON array in the binding function: {content}"
    );
}

/// Regression test: streaming adapters must emit iterator-based structs with next() and deinit().
/// This test verifies that the struct has the correct methods and that intermediate chunks can
/// be inspected without draining the entire stream.
#[test]
fn streaming_struct_has_next_and_deinit_methods() {
    let toml = r#"
[workspace]
languages = ["zig", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[[crates.adapters]]
name = "crawl_stream"
pattern = "streaming"
core_path = "demo::crawl_stream"
owner_type = "CrawlEngineHandle"
item_type = "CrawlEvent"
error_type = "DemoError"
request_type = "demo::CrawlStreamRequest"

[[crates.adapters.params]]
name = "req"
type = "CrawlStreamRequest"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let crawl_stream_method = MethodDef {
        name: "crawl_stream".into(),
        params: vec![make_param("req", TypeRef::Named("CrawlStreamRequest".into()))],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: Some("DemoError".into()),
        doc: "Stream crawl events for a single URL.".into(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let engine_type = TypeDef {
        name: "CrawlEngineHandle".into(),
        rust_path: "demo::CrawlEngineHandle".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![crawl_stream_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![engine_type],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Network".into(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The next() method must be present and take `self: *CrawlEventStream`.
    assert!(
        content.contains("pub fn next(self: *CrawlEventStream)"),
        "next() method must be present on CrawlEventStream: {content}"
    );

    // The deinit() method must be present and take `self: *CrawlEventStream`.
    assert!(
        content.contains("pub fn deinit(self: *CrawlEventStream) void {"),
        "deinit() method must be present on CrawlEventStream: {content}"
    );

    // next() must return an optional item (or error).
    assert!(
        content.contains("!?CrawlEvent"),
        "next() must return error union of optional item type: {content}"
    );

    // deinit() must release the handle via _free.
    assert!(
        content.contains("c.demo_crawl_engine_handle_crawl_stream_free(self._handle);"),
        "deinit() must call the _free FFI function: {content}"
    );
}

#[test]
fn named_json_return_guards_against_null_to_json_pointer() {
    // Regression: `<prefix>_<snake>_to_json` is allowed to return NULL when
    // serialisation fails (e.g., a result contains a field the FFI layer can't
    // represent). The previous template called `std.mem.sliceTo(_json_ptr, 0)`
    // unconditionally and panicked with `reached unreachable code` on the
    // `ptr != null` assert deep in std.mem.lenSliceTo, crashing the test
    // process. The fix returns `_first_error(<ErrorSet>)` when the pointer
    // is NULL so callers see an error (a member of the function's declared
    // error set) instead of an ABRT.
    let result_type = TypeDef {
        name: "ExtractionResult".into(),
        rust_path: "demo::ExtractionResult".into(),
        original_rust_path: String::new(),
        fields: vec![make_field("content", TypeRef::String, false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![result_type],
        functions: vec![FunctionDef {
            name: "extract".into(),
            rust_path: "demo::extract".into(),
            original_rust_path: String::new(),
            params: vec![make_param("path", TypeRef::String)],
            return_type: TypeRef::Named("ExtractionResult".into()),
            is_async: false,
            error_type: Some("Error".into()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("if (_json_ptr == null) return _first_error(anyerror);"),
        "named struct return must guard against NULL to_json pointer with _first_error(<ErrorSet>): {content}"
    );
    // And the slice/dupe must come AFTER the guard, never before.
    let guard_pos = content
        .find("if (_json_ptr == null) return _first_error(anyerror);")
        .expect("guard line present");
    let slice_pos = content
        .find("std.mem.sliceTo(_json_ptr, 0)")
        .expect("slice line present");
    assert!(
        guard_pos < slice_pos,
        "null-guard must precede sliceTo so the assertion never fires"
    );
}
