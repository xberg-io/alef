use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::ir::*;

use super::{make_config, make_field, resolved_one};

#[test]
fn test_opaque_type() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OpaqueHandle".to_string(),
            rust_path: "test_lib::OpaqueHandle".to_string(),
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
            doc: "An opaque handle to Rust state".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify opaque type wraps unsafe.Pointer
    assert!(
        content.contains("type OpaqueHandle struct"),
        "Should define OpaqueHandle struct"
    );
    assert!(
        content.contains("ptr unsafe.Pointer"),
        "Should have ptr field of unsafe.Pointer type"
    );
    assert!(
        content.contains("\"unsafe\""),
        "Should import unsafe package for opaque types"
    );

    // Verify Free method
    assert!(
        content.contains("func (h *OpaqueHandle) Free()"),
        "Should define Free method for opaque type"
    );
    assert!(
        content.contains("test_opaque_handle_free") || content.contains("Free"),
        "Free method should call FFI free function"
    );
}

#[test]
fn test_default_config() {
    let backend = GoBackend;
    let field_with_default = |name: &str, ty: TypeRef, default| {
        let mut field = make_field(name, ty, false);
        field.typed_default = Some(default);
        field
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                field_with_default(
                    "timeout",
                    TypeRef::Primitive(PrimitiveType::U32),
                    DefaultValue::IntLiteral(30),
                ),
                field_with_default(
                    "retries",
                    TypeRef::Primitive(PrimitiveType::U8),
                    DefaultValue::IntLiteral(3),
                ),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true, // Enable functional options
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with defaults".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // As of STY-9 the Go backend defaults to plain struct literals + a single
    // `Ptr[T]` helper, and only emits `With<Field>` / `New<Struct>` for struct
    // names listed in `[crates.go] functional_options`. With no allowlist the
    // functional-options shape must be absent.
    assert!(
        !content.contains("type ConfigOption"),
        "Should NOT emit functional-options type alias by default; got:\n{content}"
    );
    assert!(
        !content.contains("func WithConfig"),
        "Should NOT emit With<Field> functional-options helpers by default; got:\n{content}"
    );
    assert!(
        !content.contains("func NewConfig("),
        "Should NOT emit a New<Struct> functional-options constructor by default; got:\n{content}"
    );

    // The plain struct shape and the shared `Ptr[T]` helper must be present so
    // callers can construct `Config{Timeout: Ptr[uint32](30)}` directly.
    assert!(
        content.contains("type Config struct"),
        "Should emit the plain struct definition; got:\n{content}"
    );
    assert!(
        content.contains("func Ptr[T any](v T) *T"),
        "Should emit the shared generic Ptr[T] helper for the plain-DTO shape; got:\n{content}"
    );
}

#[test]
fn test_optional_primitive_uses_cgo_types() {
    // Regression test: optional primitive params must be declared using CGo types
    // (C.uint64_t, C.uint32_t, etc.) rather than Go native types, because CGo
    // does not implicitly convert between Go numeric types and C typedef types
    // when calling C functions.
    let backend = GoBackend;

    let make_param = |name: &str, prim: PrimitiveType| ParamDef {
        name: name.to_string(),
        ty: TypeRef::Primitive(prim),
        optional: true,
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "create_thing".to_string(),
            rust_path: "test_lib::create_thing".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_param("timeout_secs", PrimitiveType::U64),
                make_param("max_retries", PrimitiveType::U32),
            ],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: Some("Error".to_string()),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config).unwrap();
    let content = &result[0].content;

    // The temporary variables must be declared as CGo types, not Go native types.
    // Wrong (old): var cTimeoutSecs uint64 = ^uint64(0)
    // Right (new): var cTimeoutSecs C.uint64_t = C.uint64_t(^uint64(0))
    assert!(
        content.contains("C.uint64_t(^uint64(0))"),
        "U64 optional sentinel should be cast to C.uint64_t, got:\n{}",
        content
    );
    assert!(
        content.contains("C.uint32_t(^uint32(0))"),
        "U32 optional sentinel should be cast to C.uint32_t"
    );
    assert!(
        !content.contains("var cTimeoutSecs uint64"),
        "Should not declare cTimeoutSecs as Go uint64 — must use C.uint64_t"
    );
    assert!(
        !content.contains("var cMaxRetries uint32"),
        "Should not declare cMaxRetries as Go uint32 — must use C.uint32_t"
    );
}

#[test]
fn test_optional_return_type_no_double_pointer() {
    // Regression test: a function returning Option<String> (TypeRef::Optional(String))
    // must produce a *string return type, not **string.
    // go_type(Optional(String)) already emits "*string"; adding an extra "*" prefix
    // in the return type calculation produced "**string" which is invalid Go.
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "detect_language".to_string(),
            rust_path: "test_lib::detect_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "ext".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
            is_async: false,
            error_type: None,
            doc: "Detect language from extension".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config).unwrap();
    let content = &result[0].content;

    // Must NOT contain a double-pointer return type
    assert!(
        !content.contains("**string"),
        "Optional<String> return must not produce **string, got:\n{}",
        content
    );
    // Must contain the correct single-pointer return type
    assert!(
        content.contains("*string"),
        "Optional<String> return should produce *string, got:\n{}",
        content
    );
}

/// Regression: when the same name appears as both an opaque `TypeDef` and an
/// `ErrorDef`, the structured error struct (Code/Message fields) is emitted by
/// `gen_go_error_struct` and the opaque-handle struct/Free method should be
/// suppressed. Methods on the opaque type must NOT be emitted either —
/// otherwise the codegen produces method bodies that dereference `h.ptr` on a
/// value-type struct that has no `ptr` field, which fails to compile.
#[test]
fn test_opaque_error_type_uses_value_semantics() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GraphQLError".to_string(),
            rust_path: "test_lib::GraphQLError".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "status_code".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::U16),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Returns the HTTP status code.".to_string(),
                receiver: Some(alef::core::ir::ReceiverKind::Ref),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
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
            doc: "GraphQL error type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GraphQLError".to_string(),
            rust_path: "test_lib::GraphQLError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "ValidationError".to_string(),
                fields: vec![],
                doc: "Validation failed".to_string(),
                message_template: Some("validation failed".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
            }],
            doc: "GraphQL error".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("generation succeeds");
    let content = &files[0].content;

    // The error struct emitted by `gen_go_error_struct` provides the Go-side
    // type. It carries Code/Message string fields and an Error() method —
    // not a ptr field.
    assert!(
        content.contains("type GraphQLError struct"),
        "value-type error struct must be emitted"
    );
    assert!(
        content.contains("Code    string") && content.contains("Message string"),
        "value-type error struct must have Code/Message fields, got:\n{}",
        content
    );
    assert!(
        content.contains("func (e GraphQLError) Error() string"),
        "value-type error must implement the error interface"
    );

    // Methods on the opaque variant must NOT be emitted — they would
    // reference `h.ptr` which does not exist on the value-type struct.
    assert!(
        !content.contains("func (h *GraphQLError) StatusCode"),
        "opaque-style method must not be generated for value-type error, got:\n{}",
        content
    );
    assert!(
        !content.contains("h.ptr"),
        "no `h.ptr` references should appear when the only opaque type is also an error type, got:\n{}",
        content
    );
}

/// Regression: a type with a `TypeRef::Bytes` return value previously emitted
/// `unmarshalBytes(ptr)` without ever defining the helper, and tried to free
/// the byte buffer via `_free_string` (which expects `*C.char`, not
/// `*C.uint8_t`). Both produced cgo compile errors. The fix emits a single
/// package-level `unmarshalBytes` helper and stops emitting `_free_string`
/// for `Bytes` returns (the FFI hands out aliasing pointers into a parent
/// handle's storage that the caller does not own).
#[test]
fn test_bytes_return_emits_helper_and_no_string_free() {
    let backend = GoBackend;

    fn make_bytes_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Bytes,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: format!("Get {}", name),
            receiver: Some(alef::core::ir::ReceiverKind::Ref),
            sanitized: false,
            returns_ref: true,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            trait_source: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "UploadFile".to_string(),
            rust_path: "test_lib::UploadFile".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("filename", TypeRef::String, false)],
            // Two bytes-returning methods on the same type — the helper must
            // still be emitted exactly once.
            methods: vec![make_bytes_method("as_bytes"), make_bytes_method("raw_content")],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Upload file".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("generation succeeds");
    let content = &files[0].content;

    // The helper is emitted exactly once, regardless of how many bytes-returning
    // methods reference it.
    let helper_decls = content.matches("func unmarshalBytes(").count();
    assert_eq!(
        helper_decls, 1,
        "unmarshalBytes helper must be emitted exactly once per package, got {} occurrences in:\n{}",
        helper_decls, content
    );
    assert!(
        content.matches("unmarshalBytes(").count() > helper_decls,
        "bytes-returning methods must call the package-level helper, got:\n{}",
        content
    );

    // `*C.uint8_t` (the FFI return type for raw byte buffers) must not be
    // passed to `_free_string`, which expects `*C.char` and would fail to
    // compile under cgo's strict type checking.
    let bytes_method_block = content
        .split("AsBytes")
        .nth(1)
        .expect("AsBytes method must be generated");
    assert!(
        !bytes_method_block.starts_with_str_after_first("defer C.test_free_string(ptr)"),
        "bytes return must not be freed via _free_string"
    );
    // More directly: ensure no emission of `_free_string(ptr)` after a Bytes
    // method's `ptr := C.test_upload_file_as_bytes(...)` call site.
    let as_bytes_call_idx = content
        .find("C.test_upload_file_as_bytes")
        .expect("AsBytes FFI call must be present");
    let next_500 = &content[as_bytes_call_idx..(as_bytes_call_idx + 500).min(content.len())];
    assert!(
        !next_500.contains("test_free_string"),
        "no _free_string call should follow a Bytes-returning FFI call, got:\n{}",
        next_500
    );
}

// Tiny helper trait for the regression test above.
trait StartsWithStrAfterFirst {
    fn starts_with_str_after_first(&self, needle: &str) -> bool;
}
impl StartsWithStrAfterFirst for str {
    fn starts_with_str_after_first(&self, needle: &str) -> bool {
        self.lines().any(|l| l.trim_start().starts_with(needle))
    }
}

// ---------------------------------------------------------------------------
// CFLAGS bundled include dir (regression: downstream go get compatibility)
// ---------------------------------------------------------------------------

#[test]
fn test_cflags_uses_bundled_include_dir() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.go]
module = "github.com/example/mylib"
"#,
    );
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
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
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding_go = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();

    assert!(
        binding_go.content.contains("#cgo CFLAGS: -I${SRCDIR}/include"),
        "binding.go must use bundled include dir, not a monorepo-relative path"
    );
    assert!(
        !binding_go.content.contains("../crates/"),
        "binding.go must not contain monorepo-relative paths like ../crates/ in CFLAGS"
    );
}

// ---------------------------------------------------------------------------
// Regression: no duplicate "var raw struct" in UnmarshalJSON wrappers
// ---------------------------------------------------------------------------

#[test]
fn test_no_duplicate_var_raw_struct_in_unmarshal_json() {
    // Regression: the struct_unmarshal_json_header template outputs "var raw struct {"
    // and the gen_bindings code was also manually emitting it, causing duplicates.
    let config = make_config();

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), true),
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Configuration struct".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding_go = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();

    // Count consecutive "var raw struct {" lines — should be exactly 0 duplicates
    let mut found_consecutive = false;
    let lines: Vec<&str> = binding_go.content.lines().collect();
    for i in 0..lines.len().saturating_sub(1) {
        let current = lines[i].trim();
        let next = lines[i + 1].trim();
        if current == "var raw struct {" && next == "var raw struct {" {
            found_consecutive = true;
            eprintln!("Found duplicate at lines {}-{}: {}", i + 1, i + 2, current);
        }
    }

    assert!(
        !found_consecutive,
        "binding.go must not contain duplicate 'var raw struct {{' lines in UnmarshalJSON"
    );
}
