use alef_backend_zig::ZigBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef,
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
        has_serde: false,
        super_traits: vec![],
    }
}

fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "demo".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
    format: ::alef_core::config::FormatConfig::default(),
    format_overrides: ::std::collections::HashMap::new(),
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

/// String parameter: wrapper takes `[]const u8`; body allocates a null-terminated
/// copy via `std.fmt.allocPrintZ` and frees it after the C call.
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
        }],
        enums: vec![],
        errors: vec![],
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
        content.contains("allocPrintZ") && content.contains("who_z"),
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
        }],
        enums: vec![],
        errors: vec![],
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
        }],
        enums: vec![],
        errors: vec![],
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
        content.contains("allocPrintZ") && content.contains("items_z"),
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
                doc: String::new(),
            }],
            doc: String::new(),
        }],
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

/// Async functions are silently skipped.  No `pub fn` wrapper is emitted.
/// A file-level comment notes that async is unsupported when at least one
/// function was skipped.
#[test]
fn async_function_is_skipped_with_header_comment() {
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
        }],
        enums: vec![],
        errors: vec![],
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    // File-level comment must note that async is unsupported.
    assert!(
        content.contains("Async functions are not supported in this backend."),
        "must emit async-unsupported header comment: {content}"
    );
    // No wrapper function must be emitted for the async function.
    assert!(
        !content.contains("pub fn fetch_async"),
        "must NOT emit async function wrapper: {content}"
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
                is_tuple: false,
                },
                EnumVariant {
                    name: "Inactive".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,

            is_copy: false,
        }],
        errors: vec![],
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
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("value: ?[:0]const u8,"), "missing optional: {content}");
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
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "timeout".into(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
        }],
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("pub const DemoError = error {"),
        "missing error set definition: {content}"
    );
    assert!(
        content.contains("ConnectionFailed,"),
        "missing ConnectionFailed tag: {content}"
    );
    assert!(content.contains("Timeout,"), "missing Timeout tag: {content}");
}
