use ahash::AHashSet;
use alef_codegen::generators::{
    AdapterBodies, AsyncPattern, RustBindingConfig, binding_helpers, gen_constructor, gen_enum, gen_function,
    gen_impl_block, gen_method, gen_opaque_impl_block, gen_static_method, gen_struct,
};
use alef_codegen::generators::enums::enum_has_data_variants;
use alef_codegen::generators::functions::{collect_explicit_core_imports, collect_trait_imports};
use alef_codegen::generators::structs::{
    can_generate_default_impl, gen_opaque_struct, gen_struct_default_impl, type_needs_mutex,
};
use alef_codegen::generators::trait_bridge::{
    TraitBridgeGenerator, TraitBridgeSpec, format_param_type, format_type_ref, gen_bridge_all,
    gen_bridge_trait_impl, gen_bridge_wrapper_struct,
};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType,
    ReceiverKind, TypeDef, TypeRef,
};
use std::borrow::Cow;
use std::collections::HashMap;

/// Minimal TypeMapper using plain Rust type names (no backend-specific overrides).
struct RustMapper;

impl TypeMapper for RustMapper {
    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }
}

fn default_cfg<'a>() -> RustBindingConfig<'a> {
    RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &["Clone", "Debug"],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "#[no_mangle]",
        enum_attrs: &[],
        enum_derives: &["Clone", "Debug", "PartialEq"],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "my_crate",
        async_pattern: AsyncPattern::None,
        has_serde: false,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
    }
}

fn simple_type_def() -> TypeDef {
    TypeDef {
        name: "MyConfig".to_string(),
        rust_path: "my_crate::MyConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "The config name.".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
            },
            FieldDef {
                name: "count".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: true,
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
            },
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "A minimal config type.".to_string(),
        cfg: None,
    }
}

fn simple_function_def() -> FunctionDef {
    FunctionDef {
        name: "process".to_string(),
        rust_path: "my_crate::process".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "input".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        error_type: None,
        doc: "Process a string input.".to_string(),
        cfg: None,
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

fn simple_enum_def() -> EnumDef {
    EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "my_crate::OutputFormat".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Json".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
            },
            EnumVariant {
                name: "Csv".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
            EnumVariant {
                name: "Plain".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
        ],
        doc: "Output format options.".to_string(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,
    }
}

#[test]
fn test_gen_struct_produces_struct_definition() {
    let typ = simple_type_def();
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(
        result.contains("pub struct MyConfig"),
        "should contain struct declaration"
    );
    assert!(result.contains("name: String"), "should contain String field");
    assert!(
        result.contains("count: Option<u32>"),
        "should contain optional u32 field"
    );
    assert!(
        result.contains("#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]"),
        "should have derives"
    );
}

#[test]
fn test_gen_function_produces_function_signature() {
    let func = simple_function_def();
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("pub fn process"), "should contain function name");
    assert!(result.contains("input: String"), "should contain input param");
    assert!(result.contains("-> u32"), "should contain return type");
}

#[test]
fn test_gen_enum_produces_enum_with_variants() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("pub enum OutputFormat"),
        "should contain enum declaration"
    );
    assert!(
        result.contains("Json = 0"),
        "should contain first variant with discriminant"
    );
    assert!(result.contains("Csv = 1"), "should contain second variant");
    assert!(result.contains("Plain = 2"), "should contain third variant");
    assert!(
        result.contains("#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]"),
        "should have derives"
    );
}

#[test]
fn test_gen_enum_produces_default_impl() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("#[default]"),
        "should have #[default] attribute on first variant"
    );
    assert!(result.contains("Default"), "should derive Default");
}

#[test]
fn test_gen_struct_with_empty_fields() {
    let typ = TypeDef {
        name: "Empty".to_string(),
        rust_path: "my_crate::Empty".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(result.contains("pub struct Empty"), "should generate empty struct");
}

// ==============================================================================
// Tests for method code generation
// ==============================================================================

#[test]
fn test_gen_constructor_produces_new_method() {
    let typ = simple_type_def();
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_constructor(&typ, &mapper, &cfg);

    assert!(result.contains("pub fn new"), "should contain new function");
    assert!(result.contains("name: String"), "should accept name parameter");
    assert!(result.contains("count: Option<u32>"), "should accept count parameter");
    assert!(result.contains("Self {"), "should construct Self");
    assert!(result.contains("name"), "should include name field in struct literal");
    assert!(result.contains("count"), "should include count field in struct literal");
}

#[test]
fn test_gen_instance_method_with_ref_receiver() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "get_name".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn get_name"), "should contain method name");
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> String"), "should have String return type");
}

#[test]
fn test_gen_static_method_without_receiver() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "create".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Named("MyConfig".to_string()),
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let mutex_types = AHashSet::new();
    let result = gen_static_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        &adapter_bodies,
        &opaque_types,
        &mutex_types,
    );

    assert!(result.contains("pub fn create"), "should contain static method name");
    assert!(!result.contains("&self"), "should not have &self");
    assert!(result.contains("config: String"), "should accept config parameter");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
}

#[test]
fn test_gen_async_method_generates_async_signature() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "process_async".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(
        result.contains("pub fn process_async"),
        "should contain async method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    // The return type wrapping depends on the error_type and async handling
    assert!(
        result.contains("u32") || result.contains("impl"),
        "should reference u32 return type"
    );
}

#[test]
fn test_gen_method_with_multiple_params() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "compute".to_string(),
        params: vec![
            ParamDef {
                name: "a".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
            },
            ParamDef {
                name: "b".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
            },
            ParamDef {
                name: "label".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
            },
        ],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn compute"), "should contain method name");
    assert!(result.contains("a: u32"), "should have parameter a");
    assert!(result.contains("b: u32"), "should have parameter b");
    assert!(result.contains("label: String"), "should have parameter label");
    assert!(result.contains("-> u32"), "should have return type");
}

#[test]
fn test_gen_method_with_error_type() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "validate".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("ValidationError".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn validate"), "should contain method name");
    // Should be wrapped in Result due to error_type
    assert!(result.contains("Result"), "should return Result when error_type is set");
    assert!(result.contains("String"), "should contain return type in Result");
    // Should have #[allow(clippy::missing_errors_doc)] when returning Result
    assert!(
        result.contains("missing_errors_doc"),
        "should suppress missing_errors_doc lint"
    );
}

#[test]
fn test_gen_impl_block_with_constructor_and_methods() {
    let mut typ = simple_type_def();
    typ.methods = vec![
        MethodDef {
            name: "get_name".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        },
        MethodDef {
            name: "create".to_string(),
            params: vec![],
            return_type: TypeRef::Named("MyConfig".to_string()),
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
        },
    ];

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_impl_block(&typ, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("impl MyConfig {"), "should contain impl block");
    assert!(result.contains("pub fn new"), "should contain constructor");
    assert!(result.contains("pub fn get_name"), "should contain instance method");
    assert!(result.contains("pub fn create"), "should contain static method");
    assert!(result.starts_with("impl"), "should start with impl");
    assert!(result.ends_with("}"), "should end with closing brace");
}

#[test]
fn test_gen_method_with_optional_param() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "configure".to_string(),
        params: vec![ParamDef {
            name: "timeout".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: true,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn configure"), "should contain method name");
    assert!(result.contains("Option<u32>"), "should wrap optional param in Option");
    assert!(result.contains("-> ()"), "should return unit type");
}

// ==============================================================================
// Tests for binding_helpers module
// ==============================================================================

#[test]
fn test_wrap_return_primitive_passthrough() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Primitive(PrimitiveType::U32),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_unit_passthrough() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::Unit, "MyType", &opaque_types, false, false, false);
    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_string_ref_conversion() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::String, "MyType", &opaque_types, false, true, false);
    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_string_owned_passthrough() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::String, "MyType", &opaque_types, false, false, false);
    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_path_conversion() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::Path, "MyType", &opaque_types, false, false, false);
    assert_eq!(result, "result.to_string_lossy().to_string()");
}

#[test]
fn test_wrap_return_duration_conversion() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Duration,
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.as_millis() as u64");
}

#[test]
fn test_wrap_return_opaque_self_owned() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        true,
        false,
        false,
    );
    assert_eq!(result, "Self { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_other_opaque_type() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("OtherType".to_string());
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Named("OtherType".to_string()),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "OtherType { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_non_opaque_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Named("SomeType".to_string()),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_optional_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Named("SomeType".to_string()))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.map(Into::into)");
}

#[test]
fn test_wrap_return_vec_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.into_iter().map(Into::into).collect()");
}

#[test]
fn test_wrap_return_optional_vec_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.map(|v| v.into_iter().map(Into::into).collect())");
}

#[test]
fn test_wrap_return_optional_vec_opaque_named() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Item".to_string());
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(
        result,
        "result.map(|v| v.into_iter().map(|x| Item { inner: Arc::new(x) }).collect())"
    );
}

#[test]
fn test_gen_call_args_string_param() {
    let params = vec![ParamDef {
        name: "input".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "input");
}

#[test]
fn test_gen_call_args_primitive_param() {
    let params = vec![ParamDef {
        name: "count".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "count");
}

#[test]
fn test_gen_call_args_opaque_param() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "&obj.inner");
}

#[test]
fn test_gen_call_args_non_opaque_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "config.into()");
}

#[test]
fn test_gen_call_args_optional_non_opaque_ref_param() {
    // When a core function takes Option<&T> where T is a non-opaque type,
    // we have Option<T> on the binding side and need to convert to Option<&T>.
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: true,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "config.as_ref()");
}

#[test]
fn test_gen_call_args_path_param() {
    let params = vec![ParamDef {
        name: "file_path".to_string(),
        ty: TypeRef::Path,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "std::path::PathBuf::from(file_path)");
}

#[test]
fn test_gen_call_args_duration_param() {
    let params = vec![ParamDef {
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "std::time::Duration::from_millis(timeout)");
}

#[test]
fn test_gen_call_args_multiple_params() {
    let opaque_types = AHashSet::new();
    let params = vec![
        ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        },
        ParamDef {
            name: "count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        },
    ];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "name, count");
}

#[test]
fn test_gen_call_args_with_let_bindings_opaque() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    assert_eq!(result, "&obj.inner");
}

#[test]
fn test_gen_call_args_with_let_bindings_non_opaque() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    assert_eq!(result, "config_core");
}

#[test]
fn test_gen_named_let_bindings_empty_params() {
    let opaque_types = AHashSet::new();
    let params = vec![];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert_eq!(result, "");
}

#[test]
fn test_gen_named_let_bindings_non_opaque_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(result.contains("let config_core: my_crate::Config = config.into();"));
}

#[test]
fn test_gen_named_let_bindings_optional_ref_param() {
    // When a core function takes Option<&T> where T is a non-opaque type,
    // we need to generate `let config_core = config.as_ref();`
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: true,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(result.contains("let config_owned: Option<my_crate::Config> = config.map(Into::into);"));
    assert!(result.contains("let config_core = config_owned.as_ref();"));
}

#[test]
fn test_gen_call_args_with_let_bindings_optional_ref_param() {
    // When a core function takes Option<&T> where T is a non-opaque type,
    // and we have a let binding `let config_core = config.as_ref();` that creates Option<&T>,
    // the call args should use `config_core` directly (NOT `&config_core`).
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: true,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    // The result should be `config_core` (Option<&Config>) not `&config_core` (&Option<&Config>)
    assert_eq!(result, "config_core");
}

#[test]
fn test_gen_call_args_with_let_bindings_optional_ref_vec_named() {
    // When a core function takes Option<&[T]> where T is a non-opaque type,
    // and `optional=true, is_ref=true` for Vec<Named>, no let binding is generated.
    // The call args should use `param.as_deref()` directly.
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        optional: true,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    // The result should use the _core let binding which was created by gen_named_let_bindings
    assert_eq!(result, "items_core.as_deref()");
}

#[test]
fn test_gen_named_let_bindings_opaque_skipped() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert_eq!(result, "");
}

#[test]
fn test_has_named_params_returns_true() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    assert!(binding_helpers::has_named_params(&params, &opaque_types));
}

#[test]
fn test_has_named_params_returns_false() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    assert!(!binding_helpers::has_named_params(&params, &opaque_types));
}

#[test]
fn test_is_simple_non_opaque_param_string() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::String));
}

#[test]
fn test_is_simple_non_opaque_param_primitive() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::Primitive(
        PrimitiveType::U32
    )));
}

#[test]
fn test_is_simple_non_opaque_param_path() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::Path));
}

#[test]
fn test_is_simple_non_opaque_param_duration() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::Duration));
}

#[test]
fn test_is_simple_non_opaque_param_vec_is_false() {
    assert!(!binding_helpers::is_simple_non_opaque_param(&TypeRef::Vec(Box::new(
        TypeRef::String
    ))));
}

#[test]
fn test_is_simple_non_opaque_param_named_is_false() {
    assert!(!binding_helpers::is_simple_non_opaque_param(&TypeRef::Named(
        "Config".to_string()
    )));
}

#[test]
fn test_gen_async_body_pyo3_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;

    let result = binding_helpers::gen_async_body("inner.process()", &cfg, true, "result", false, "", false, None);

    assert!(result.contains("pyo3_async_runtimes::tokio::future_into_py"));
    assert!(result.contains("await"));
    assert!(result.contains("map_err"));
}

#[test]
fn test_gen_async_body_napi_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;

    let result = binding_helpers::gen_async_body("CoreType::process()", &cfg, true, "result", false, "", false, None);

    assert!(result.contains("await"));
    assert!(result.contains("map_err"));
    assert!(result.contains("napi::Error"));
}

#[test]
fn test_gen_async_body_wasm_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::WasmNativeAsync;

    let result = binding_helpers::gen_async_body("process_async()", &cfg, true, "result", false, "", false, None);

    assert!(result.contains("await"));
    assert!(result.contains("JsValue"));
}

#[test]
fn test_gen_async_body_with_inner_clone_line() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;

    let result = binding_helpers::gen_async_body(
        "inner.process()",
        &cfg,
        false,
        "()",
        false,
        "let inner = self.inner.clone();\n        ",
        true,
        None,
    );

    assert!(result.contains("let inner = self.inner.clone();"));
    assert!(result.contains("pyo3_async_runtimes::tokio::future_into_py"));
}

#[test]
fn test_gen_unimplemented_body_with_error() {
    let cfg = default_cfg();
    let params = vec![ParamDef {
        name: "input".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    }];

    let empty_opaque = AHashSet::new();
    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::String,
        "unimplemented_fn",
        true,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert!(result.contains("let _ = input;"));
    assert!(result.contains("Err(\"Not implemented"));
}

#[test]
fn test_gen_unimplemented_body_string_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::String,
        "unimplemented_fn",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert!(result.contains("[unimplemented"));
}

#[test]
fn test_gen_unimplemented_body_bool_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Primitive(PrimitiveType::Bool),
        "is_valid",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert!(result.contains("false"));
}

#[test]
fn test_gen_unimplemented_body_vec_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Vec(Box::new(TypeRef::String)),
        "list_items",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert!(result.contains("Vec::new()"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_sanitized() {
    let mut typ = simple_type_def();
    typ.fields[0].sanitized = true;

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate", false);

    assert!(result.contains("let core_self"));
    assert!(result.contains("name: Default::default(),"));
    assert!(result.contains("count:"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_non_sanitized() {
    let typ = simple_type_def();

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate", false);

    assert!(result.contains("let core_self"));
    assert!(result.contains("my_crate::MyConfig {"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_with_duration() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
        optional: false,
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
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate", false);

    assert!(result.contains("timeout: std::time::Duration::from_millis(self.timeout),"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_with_duration_optional_flag() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "request_timeout".to_string(),
        ty: TypeRef::Duration,
        optional: true,
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
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate", false);

    assert!(
        result.contains("request_timeout: self.request_timeout.map(std::time::Duration::from_millis),"),
        "got: {result}"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_with_optional_duration_type() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "request_timeout".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Duration)),
        optional: false,
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
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate", false);

    assert!(
        result.contains("request_timeout: self.request_timeout.map(|v| std::time::Duration::from_millis(v as u64)),"),
        "got: {result}"
    );
}

#[test]
fn test_gen_method_builder_pattern_opaque() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    typ.name = "MyConfig".to_string();

    let method = MethodDef {
        name: "with_name".to_string(),
        params: vec![ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Owned),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = {
        let mut set = AHashSet::new();
        set.insert("MyConfig".to_string());
        set
    };

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        true,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(
        result.contains("pub fn with_name"),
        "should contain builder method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
    assert!(
        result.contains("Self { inner: Arc::new"),
        "should wrap result in Self with Arc"
    );
    assert!(!result.contains("compile_error!"), "should not emit compile_error");
}

#[test]
fn test_gen_method_builder_pattern_non_opaque() {
    let mut typ = simple_type_def();
    typ.is_opaque = false;
    typ.name = "MyConfig".to_string();

    let method = MethodDef {
        name: "with_count".to_string(),
        params: vec![ParamDef {
            name: "count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(
        result.contains("pub fn with_count"),
        "should contain builder method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
    assert!(result.contains(".into()"), "should convert result back to MyConfig");
    assert!(!result.contains("compile_error!"), "should not emit compile_error");
}

// ==============================================================================
// Tests for functional RefMut pattern (non-opaque types, frozen PyO3/WASM)
// ==============================================================================

#[test]
fn test_gen_method_functional_ref_mut_unit_return() {
    // A RefMut method returning () on a non-opaque type should be generated as a
    // functional clone-mutate-return pattern: &self -> Self, not &mut self -> ().
    let typ = simple_type_def();
    let method = MethodDef {
        name: "apply_update".to_string(),
        params: vec![ParamDef {
            name: "count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    // Signature must use &self (not &mut self) and return Self
    assert!(result.contains("pub fn apply_update"), "should contain method name");
    assert!(result.contains("&self"), "should use &self receiver, not &mut self");
    assert!(!result.contains("&mut self"), "should not use &mut self");
    assert!(result.contains("-> Self"), "should return Self (functional pattern)");
    // Body: construct mutable core, call the method, return converted result
    assert!(result.contains("let mut core_self"), "should declare mutable core_self");
    assert!(
        result.contains("core_self.apply_update("),
        "should call core method on core_self"
    );
    assert!(
        result.contains("core_self.into()"),
        "should convert mutated core back to Self"
    );
}

#[test]
fn test_gen_method_functional_ref_mut_with_named_param() {
    // A RefMut method taking a Named (non-opaque) param should also use the functional pattern.
    // gen_call_args handles Named params via .into(), so this should work.
    let mut typ = simple_type_def();
    typ.name = "ConversionOptions".to_string();
    typ.rust_path = "my_crate::ConversionOptions".to_string();

    let method = MethodDef {
        name: "apply_update".to_string(),
        params: vec![ParamDef {
            name: "update".to_string(),
            ty: TypeRef::Named("ConversionOptionsUpdate".to_string()),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn apply_update"), "should contain method name");
    assert!(result.contains("&self"), "should use &self receiver");
    assert!(!result.contains("&mut self"), "should not use &mut self");
    assert!(result.contains("-> Self"), "should return Self");
    assert!(result.contains("let mut core_self"), "should declare mutable core_self");
    // Named param should be converted via .into() in the call args
    assert!(
        result.contains("update.into()"),
        "should convert Named param via .into()"
    );
    assert!(
        result.contains("core_self.into()"),
        "should convert mutated core back to Self"
    );
}

#[test]
fn test_gen_method_functional_ref_mut_with_error_type() {
    // A fallible RefMut method should return Result<Self, E> instead of ().
    let typ = simple_type_def();
    let method = MethodDef {
        name: "try_apply".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("MyError".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn try_apply"), "should contain method name");
    assert!(result.contains("&self"), "should use &self receiver");
    assert!(!result.contains("&mut self"), "should not use &mut self");
    // Should return Result<Self, ...>
    assert!(result.contains("Result<Self"), "should return Result<Self>");
    assert!(result.contains("let mut core_self"), "should declare mutable core_self");
    assert!(result.contains("core_self.try_apply("), "should call core method");
    // On success, return the mutated self converted back
    assert!(
        result.contains("Ok(core_self.into())"),
        "should return Ok(core_self.into()) on success"
    );
}

// ==============================================================================
// Additional tests for structs.rs
// ==============================================================================

#[test]
fn test_type_needs_mutex_false_when_no_ref_mut_methods() {
    let typ = simple_type_def();
    assert!(!type_needs_mutex(&typ), "type with no RefMut methods should not need mutex");
}

#[test]
fn test_type_needs_mutex_true_when_ref_mut_method_present() {
    let mut typ = simple_type_def();
    typ.methods = vec![MethodDef {
        name: "mutate".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }];
    assert!(type_needs_mutex(&typ), "type with RefMut method should need mutex");
}

#[test]
fn test_gen_opaque_struct_arc_inner() {
    let typ = TypeDef {
        name: "MyService".to_string(),
        rust_path: "my_crate::MyService".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    let cfg = default_cfg();

    let result = gen_opaque_struct(&typ, &cfg);

    assert!(result.contains("pub struct MyService {"), "should have struct declaration");
    assert!(
        result.contains("inner: Arc<my_crate::MyService>"),
        "should have Arc<...> inner field"
    );
    assert!(!result.contains("Mutex"), "plain opaque should not use Mutex");
}

#[test]
fn test_gen_opaque_struct_mutex_when_ref_mut_method() {
    let mut typ = TypeDef {
        name: "MutableService".to_string(),
        rust_path: "my_crate::MutableService".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "update".to_string(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::RefMut),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }],
        is_opaque: true,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    typ.is_opaque = true;
    let cfg = default_cfg();

    let result = gen_opaque_struct(&typ, &cfg);

    assert!(result.contains("pub struct MutableService {"), "should have struct declaration");
    assert!(
        result.contains("Arc<std::sync::Mutex<my_crate::MutableService>>"),
        "should use Arc<Mutex<...>> for RefMut types"
    );
}

#[test]
fn test_gen_opaque_struct_trait_uses_dyn() {
    let typ = TypeDef {
        name: "MyTrait".to_string(),
        rust_path: "my_crate::MyTrait".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    let cfg = default_cfg();

    let result = gen_opaque_struct(&typ, &cfg);

    assert!(
        result.contains("Arc<dyn my_crate::MyTrait + Send + Sync>"),
        "trait opaque should use Arc<dyn Trait + Send + Sync>"
    );
}

#[test]
fn test_gen_struct_default_impl_generates_correct_impl() {
    let typ = simple_type_def();

    let result = gen_struct_default_impl(&typ, "");

    assert!(result.contains("impl Default for MyConfig {"), "should generate Default impl");
    assert!(result.contains("fn default() -> Self {"), "should have default() method");
    assert!(
        result.contains("name: Default::default()"),
        "non-optional fields use Default::default()"
    );
    assert!(result.contains("count: Default::default()"), "optional field uses Default::default()");
}

#[test]
fn test_gen_struct_default_impl_with_name_prefix() {
    let typ = simple_type_def();

    let result = gen_struct_default_impl(&typ, "Js");

    assert!(
        result.contains("impl Default for JsMyConfig {"),
        "should use prefixed name"
    );
}

#[test]
fn test_gen_struct_default_impl_optional_field_uses_none() {
    let typ = TypeDef {
        name: "OptConfig".to_string(),
        rust_path: "my_crate::OptConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "value".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: false,
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
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };

    let result = gen_struct_default_impl(&typ, "");

    assert!(result.contains("value: None"), "Optional<T> fields should default to None");
}

#[test]
fn test_can_generate_default_impl_all_primitives() {
    let typ = simple_type_def();
    let known: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(
        can_generate_default_impl(&typ, &known),
        "type with only primitives and strings can generate Default"
    );
}

#[test]
fn test_can_generate_default_impl_named_not_in_known_set() {
    let typ = TypeDef {
        name: "Compound".to_string(),
        rust_path: "my_crate::Compound".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "inner".to_string(),
            ty: TypeRef::Named("UnknownType".to_string()),
            optional: false,
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
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    let known: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(
        !can_generate_default_impl(&typ, &known),
        "type with Named field not in known set cannot generate Default"
    );
}

#[test]
fn test_can_generate_default_impl_named_in_known_set() {
    let typ = TypeDef {
        name: "Compound".to_string(),
        rust_path: "my_crate::Compound".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "inner".to_string(),
            ty: TypeRef::Named("KnownType".to_string()),
            optional: false,
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
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    let mut known: std::collections::HashSet<&str> = std::collections::HashSet::new();
    known.insert("KnownType");
    assert!(
        can_generate_default_impl(&typ, &known),
        "type with Named field in known set can generate Default"
    );
}

#[test]
fn test_gen_struct_with_opaque_field_skips_serde_derives() {
    let mut cfg = default_cfg();
    let opaque_names = vec!["OpaqueHandle".to_string()];
    cfg.opaque_type_names = &opaque_names;

    let typ = TypeDef {
        name: "Wrapper".to_string(),
        rust_path: "my_crate::Wrapper".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "handle".to_string(),
            ty: TypeRef::Named("OpaqueHandle".to_string()),
            optional: false,
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
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };
    let mapper = RustMapper;

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(result.contains("pub struct Wrapper"), "should generate struct");
    assert!(!result.contains("serde::Serialize"), "should skip Serialize derive for opaque fields");
    assert!(!result.contains("serde::Deserialize"), "should skip Deserialize derive for opaque fields");
    assert!(!result.contains("Default"), "should skip Default derive for opaque fields");
}

#[test]
fn test_gen_opaque_impl_block_returns_empty_when_no_methods() {
    let typ = simple_type_def();
    let mapper = RustMapper;
    let cfg = default_cfg();
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();
    let adapter_bodies = AdapterBodies::default();

    // simple_type_def has no methods and has fields, but gen_opaque_impl_block
    // returns empty when there are no emittable methods (fields are ignored)
    let result = gen_opaque_impl_block(&typ, &mapper, &cfg, &opaque_types, &mutex_types, &adapter_bodies);

    assert!(result.is_empty(), "opaque impl block with no methods should be empty");
}

#[test]
fn test_gen_opaque_impl_block_generates_impl_with_method() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    typ.methods = vec![MethodDef {
        name: "run".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }];

    let mapper = RustMapper;
    let cfg = default_cfg();
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();
    let adapter_bodies = AdapterBodies::default();

    let result = gen_opaque_impl_block(&typ, &mapper, &cfg, &opaque_types, &mutex_types, &adapter_bodies);

    assert!(result.contains("impl MyConfig {"), "should generate impl block");
    assert!(result.contains("pub fn run"), "should contain the method");
}

// ==============================================================================
// Additional tests for enums.rs
// ==============================================================================

#[test]
fn test_enum_has_data_variants_false_for_unit_variants() {
    let enum_def = simple_enum_def();
    assert!(!enum_has_data_variants(&enum_def), "unit-only enum should not have data variants");
}

#[test]
fn test_enum_has_data_variants_true_when_fields_present() {
    let enum_def = EnumDef {
        name: "DataEnum".to_string(),
        rust_path: "my_crate::DataEnum".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Variant".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                optional: false,
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
            }],
            doc: String::new(),
            is_default: false,
            serde_rename: None,
        }],
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,
    };
    assert!(enum_has_data_variants(&enum_def), "enum with fields should have data variants");
}

#[test]
fn test_gen_enum_with_single_variant_uses_discriminant_zero() {
    let enum_def = EnumDef {
        name: "Single".to_string(),
        rust_path: "my_crate::Single".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Only".to_string(),
            fields: vec![],
            doc: String::new(),
            is_default: true,
            serde_rename: None,
        }],
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,
    };
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("pub enum Single {"), "should have enum declaration");
    assert!(result.contains("Only = 0"), "single variant has discriminant 0");
    assert!(result.contains("#[default]"), "first variant gets #[default]");
}

#[test]
fn test_gen_enum_with_enum_attrs() {
    let enum_def = simple_enum_def();
    let mut cfg = default_cfg();
    let attrs = vec!["repr(u8)"];
    cfg.enum_attrs = &attrs;

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("#[repr(u8)]"), "should include enum attrs");
}

#[test]
fn test_gen_enum_always_derives_serde() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("serde::Serialize"), "should always derive Serialize");
    assert!(result.contains("serde::Deserialize"), "should always derive Deserialize");
}

#[test]
fn test_gen_enum_discriminant_increments_correctly() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: "my_crate::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
            },
            EnumVariant {
                name: "Inactive".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
            EnumVariant {
                name: "Pending".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
            EnumVariant {
                name: "Deleted".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
        ],
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,
    };
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("Active = 0"), "first variant = 0");
    assert!(result.contains("Inactive = 1"), "second variant = 1");
    assert!(result.contains("Pending = 2"), "third variant = 2");
    assert!(result.contains("Deleted = 3"), "fourth variant = 3");
    // Only first variant has #[default]
    assert!(result.contains("#[default]"), "should have #[default]");
}

#[test]
fn test_gen_enum_with_pyo3_pyclass_attr_renames_python_keywords() {
    let enum_def = EnumDef {
        name: "PythonKeywords".to_string(),
        rust_path: "my_crate::PythonKeywords".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
            },
            EnumVariant {
                name: "True".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
            EnumVariant {
                name: "Normal".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            },
        ],
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,
    };
    let mut cfg = default_cfg();
    let attrs = ["pyclass(eq, eq_int)"];
    cfg.enum_attrs = &attrs;

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("#[pyo3(name = \"None_\")]"),
        "Python keyword 'None' should be renamed"
    );
    assert!(
        result.contains("#[pyo3(name = \"True_\")]"),
        "Python keyword 'True' should be renamed"
    );
    assert!(
        !result.contains("#[pyo3(name = \"Normal_\")]"),
        "non-keyword 'Normal' should not be renamed"
    );
}

#[test]
fn test_gen_enum_without_pyclass_does_not_rename_python_keywords() {
    let enum_def = EnumDef {
        name: "Formats".to_string(),
        rust_path: "my_crate::Formats".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "None".to_string(),
            fields: vec![],
            doc: String::new(),
            is_default: true,
            serde_rename: None,
        }],
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,
    };
    let cfg = default_cfg(); // no pyclass attr

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        !result.contains("#[pyo3(name = \"None_\")]"),
        "without pyclass, should not emit pyo3 rename"
    );
    assert!(result.contains("None = 0"), "variant should still appear");
}

// ==============================================================================
// Additional tests for functions.rs
// ==============================================================================

#[test]
fn test_gen_function_async_produces_async_signature() {
    let mut func = simple_function_def();
    func.is_async = true;

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("pub async fn process"), "async function should have async keyword");
}

#[test]
fn test_gen_function_with_error_type_wraps_in_result() {
    let mut func = simple_function_def();
    func.error_type = Some("MyError".to_string());

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("-> Result"), "function with error_type should return Result");
    assert!(
        result.contains("missing_errors_doc"),
        "should suppress missing_errors_doc lint"
    );
}

#[test]
fn test_gen_function_with_no_params_generates_empty_param_list() {
    let func = FunctionDef {
        name: "get_version".to_string(),
        rust_path: "my_crate::get_version".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    };

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("pub fn get_version()"), "should have empty parameter list");
    assert!(result.contains("-> String"), "should have String return type");
}

#[test]
fn test_gen_function_with_optional_param_wraps_in_option() {
    let func = FunctionDef {
        name: "search".to_string(),
        rust_path: "my_crate::search".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "query".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
            },
            ParamDef {
                name: "limit".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: true,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
            },
        ],
        return_type: TypeRef::Vec(Box::new(TypeRef::String)),
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    };

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("query: String"), "required param should be plain type");
    assert!(result.contains("limit: Option<u32>"), "optional param should be wrapped in Option");
}

#[test]
fn test_gen_function_uses_function_attr() {
    let func = simple_function_def();
    let mapper = RustMapper;
    let cfg = default_cfg(); // function_attr = "#[no_mangle]"
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("#[no_mangle]"), "should include function_attr");
}

#[test]
fn test_collect_trait_imports_empty_when_no_trait_methods() {
    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![simple_type_def()],
        enums: vec![],
        functions: vec![],
        errors: vec![],
    };

    let result = collect_trait_imports(&api);

    assert!(result.is_empty(), "no trait methods means no trait imports");
}

#[test]
fn test_collect_trait_imports_deduplicates_by_trait_name() {
    let mut typ1 = simple_type_def();
    typ1.methods = vec![MethodDef {
        name: "execute".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: Some("my_crate::Executor".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }];

    let mut typ2 = simple_type_def();
    typ2.name = "OtherType".to_string();
    typ2.methods = vec![MethodDef {
        name: "execute".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: Some("my_crate::Executor".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }];

    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![typ1, typ2],
        enums: vec![],
        functions: vec![],
        errors: vec![],
    };

    let result = collect_trait_imports(&api);

    // Should deduplicate to one entry
    assert_eq!(result.len(), 1, "should deduplicate same trait path");
    assert_eq!(result[0], "my_crate::Executor");
}

#[test]
fn test_collect_explicit_core_imports_returns_type_and_enum_names() {
    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![simple_type_def()],
        enums: vec![simple_enum_def()],
        functions: vec![],
        errors: vec![],
    };

    let result = collect_explicit_core_imports(&api);

    assert!(result.contains(&"MyConfig".to_string()), "should include type name");
    assert!(result.contains(&"OutputFormat".to_string()), "should include enum name");
}

#[test]
fn test_collect_explicit_core_imports_is_sorted() {
    let mut typ_b = simple_type_def();
    typ_b.name = "Bravo".to_string();
    let mut typ_a = simple_type_def();
    typ_a.name = "Alpha".to_string();

    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![typ_b, typ_a],
        enums: vec![],
        functions: vec![],
        errors: vec![],
    };

    let result = collect_explicit_core_imports(&api);

    assert_eq!(result, vec!["Alpha", "Bravo"], "imports should be alphabetically sorted");
}

// ==============================================================================
// Tests for trait_bridge.rs
// ==============================================================================

/// Minimal TraitBridgeGenerator for testing the shared bridge helpers.
struct MockBridgeGenerator;

impl TraitBridgeGenerator for MockBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "MockObject"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["mock::MockObject".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        format!("unimplemented!(\"sync: {}\")", method.name)
    }

    fn gen_async_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        format!("unimplemented!(\"async: {}\")", method.name)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        format!(
            "impl {wrapper} {{\n    pub fn new(obj: MockObject) -> Self {{ Self {{ inner: obj, cached_name: String::new() }} }}\n}}"
        )
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let fn_name = spec.bridge_config.register_fn.as_deref().unwrap_or("register");
        format!("pub fn {fn_name}() {{}}")
    }
}

fn simple_trait_def() -> TypeDef {
    TypeDef {
        name: "MyTrait".to_string(),
        rust_path: "my_crate::MyTrait".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            MethodDef {
                name: "execute".to_string(),
                params: vec![ParamDef {
                    name: "input".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                }],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
            },
            MethodDef {
                name: "optional_method".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: true,
            },
        ],
        is_opaque: false,
        is_clone: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "A test trait.".to_string(),
        cfg: None,
    }
}

fn simple_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: None,
        param_name: None,
    }
}

#[test]
fn test_trait_bridge_spec_wrapper_name() {
    let trait_def = simple_trait_def();
    let bridge_config = simple_bridge_config();
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };

    assert_eq!(spec.wrapper_name(), "PythonMyTraitBridge");
}

#[test]
fn test_trait_bridge_spec_trait_snake() {
    let trait_def = simple_trait_def();
    let bridge_config = simple_bridge_config();
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };

    assert_eq!(spec.trait_snake(), "my_trait");
}

#[test]
fn test_trait_bridge_spec_required_vs_optional_methods() {
    let trait_def = simple_trait_def();
    let bridge_config = simple_bridge_config();
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };

    let required = spec.required_methods();
    let optional = spec.optional_methods();

    assert_eq!(required.len(), 1, "should have 1 required method");
    assert_eq!(required[0].name, "execute");
    assert_eq!(optional.len(), 1, "should have 1 optional method");
    assert_eq!(optional[0].name, "optional_method");
}

#[test]
fn test_gen_bridge_wrapper_struct_contains_foreign_type_and_cached_name() {
    let trait_def = simple_trait_def();
    let bridge_config = simple_bridge_config();
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_wrapper_struct(&spec, &generator);

    assert!(result.contains("pub struct PythonMyTraitBridge {"), "should have wrapper struct");
    assert!(result.contains("inner: MockObject"), "should have inner field with foreign type");
    assert!(result.contains("cached_name: String"), "should have cached_name field");
}

#[test]
fn test_gen_bridge_trait_impl_generates_methods() {
    let trait_def = simple_trait_def();
    let bridge_config = simple_bridge_config();
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_trait_impl(&spec, &generator);

    assert!(
        result.contains("impl my_crate::MyTrait for PythonMyTraitBridge {"),
        "should implement the trait"
    );
    assert!(result.contains("fn execute("), "should have execute method");
    assert!(result.contains("fn optional_method("), "should have optional_method");
    assert!(result.contains("&self"), "should have self receiver");
}

#[test]
fn test_gen_bridge_all_includes_imports_struct_and_trait_impl() {
    let trait_def = simple_trait_def();
    let bridge_config = simple_bridge_config();
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_all(&spec, &generator);

    assert!(result.contains("use mock::MockObject;"), "should include bridge imports");
    assert!(result.contains("pub struct PythonMyTraitBridge"), "should have wrapper struct");
    assert!(result.contains("impl PythonMyTraitBridge {"), "should have constructor impl");
    assert!(
        result.contains("impl my_crate::MyTrait for PythonMyTraitBridge"),
        "should have trait impl"
    );
    // No register_fn configured, so registration function should be absent
    assert!(!result.contains("pub fn register"), "should not have registration fn when not configured");
}

#[test]
fn test_gen_bridge_all_includes_registration_fn_when_configured() {
    let trait_def = simple_trait_def();
    let bridge_config = TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: Some("register_my_trait".to_string()),
        type_alias: None,
        param_name: None,
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_all(&spec, &generator);

    assert!(
        result.contains("pub fn register_my_trait"),
        "should include registration function when register_fn is set"
    );
}

#[test]
fn test_format_type_ref_primitives() {
    let type_paths = HashMap::new();

    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::Bool), &type_paths), "bool");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::U8), &type_paths), "u8");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::U16), &type_paths), "u16");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::U32), &type_paths), "u32");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::U64), &type_paths), "u64");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::I32), &type_paths), "i32");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::I64), &type_paths), "i64");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::F32), &type_paths), "f32");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::F64), &type_paths), "f64");
    assert_eq!(format_type_ref(&TypeRef::Primitive(PrimitiveType::Usize), &type_paths), "usize");
}

#[test]
fn test_format_type_ref_special_types() {
    let type_paths = HashMap::new();

    assert_eq!(format_type_ref(&TypeRef::String, &type_paths), "String");
    assert_eq!(format_type_ref(&TypeRef::Bytes, &type_paths), "Vec<u8>");
    assert_eq!(format_type_ref(&TypeRef::Path, &type_paths), "std::path::PathBuf");
    assert_eq!(format_type_ref(&TypeRef::Unit, &type_paths), "()");
    assert_eq!(format_type_ref(&TypeRef::Json, &type_paths), "serde_json::Value");
    assert_eq!(format_type_ref(&TypeRef::Duration, &type_paths), "std::time::Duration");
}

#[test]
fn test_format_type_ref_optional_and_vec() {
    let type_paths = HashMap::new();

    let opt = TypeRef::Optional(Box::new(TypeRef::String));
    assert_eq!(format_type_ref(&opt, &type_paths), "Option<String>");

    let vec = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
    assert_eq!(format_type_ref(&vec, &type_paths), "Vec<u32>");
}

#[test]
fn test_format_type_ref_map() {
    let type_paths = HashMap::new();

    let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Primitive(PrimitiveType::U64)));
    assert_eq!(
        format_type_ref(&map, &type_paths),
        "std::collections::HashMap<String, u64>"
    );
}

#[test]
fn test_format_type_ref_named_uses_type_paths() {
    let mut type_paths = HashMap::new();
    type_paths.insert("Config".to_string(), "my_crate::Config".to_string());

    let named = TypeRef::Named("Config".to_string());
    assert_eq!(
        format_type_ref(&named, &type_paths),
        "my_crate::Config",
        "should use qualified path from type_paths"
    );
}

#[test]
fn test_format_type_ref_named_falls_back_to_name() {
    let type_paths = HashMap::new();

    let named = TypeRef::Named("UnknownType".to_string());
    assert_eq!(
        format_type_ref(&named, &type_paths),
        "UnknownType",
        "should fall back to unqualified name when not in type_paths"
    );
}

#[test]
fn test_format_param_type_string_with_is_ref() {
    let type_paths = HashMap::new();
    let param = ParamDef {
        name: "text".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(format_param_type(&param, &type_paths), "&str");
}

#[test]
fn test_format_param_type_bytes_with_is_ref() {
    let type_paths = HashMap::new();
    let param = ParamDef {
        name: "data".to_string(),
        ty: TypeRef::Bytes,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(format_param_type(&param, &type_paths), "&[u8]");
}

#[test]
fn test_format_param_type_path_with_is_ref() {
    let type_paths = HashMap::new();
    let param = ParamDef {
        name: "file_path".to_string(),
        ty: TypeRef::Path,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(format_param_type(&param, &type_paths), "&std::path::Path");
}

#[test]
fn test_format_param_type_vec_with_is_ref() {
    let type_paths = HashMap::new();
    let param = ParamDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(format_param_type(&param, &type_paths), "&[u32]");
}

#[test]
fn test_format_param_type_named_with_is_ref() {
    let mut type_paths = HashMap::new();
    type_paths.insert("Config".to_string(), "my_crate::Config".to_string());

    let param = ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(format_param_type(&param, &type_paths), "&my_crate::Config");
}

#[test]
fn test_format_param_type_primitive_with_is_ref_passes_by_value() {
    // Primitives (Copy types) are passed by value even when is_ref is true
    let type_paths = HashMap::new();
    let param = ParamDef {
        name: "count".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(
        format_param_type(&param, &type_paths),
        "u32",
        "Copy primitives should be passed by value even when is_ref=true"
    );
}

#[test]
fn test_format_param_type_without_is_ref_passes_by_value() {
    let type_paths = HashMap::new();
    let param = ParamDef {
        name: "text".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
    };

    assert_eq!(format_param_type(&param, &type_paths), "String", "without is_ref, String is owned");
}

// ==============================================================================
// Additional tests for binding_helpers.rs — wrap_return_with_mutex
// ==============================================================================

#[test]
fn test_wrap_return_with_mutex_self_opaque_plain() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        false,
        false,
    );

    assert_eq!(result, "Self { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_with_mutex_self_opaque_mutex_type() {
    let opaque_types = AHashSet::new();
    let mut mutex_types = AHashSet::new();
    mutex_types.insert("MyType".to_string());

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        false,
        false,
    );

    assert_eq!(result, "Self { inner: Arc::new(std::sync::Mutex::new(result)) }");
}

#[test]
fn test_wrap_return_with_mutex_other_opaque_type() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("OtherType".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("OtherType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "OtherType { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_with_mutex_non_opaque_named_uses_into() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("SomeType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_with_mutex_string_returns_ref_uses_into() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::String,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true, // returns_ref
        false,
    );

    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_with_mutex_string_owned_passthrough() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::String,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_with_mutex_returns_cow_owned_named() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("SomeType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        true, // returns_cow
    );

    assert_eq!(result, "result.into_owned().into()");
}

#[test]
fn test_wrap_return_with_mutex_duration() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Duration,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.as_millis() as u64");
}

#[test]
fn test_wrap_return_with_mutex_optional_opaque() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Handle".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Named("Handle".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.map(|v| Handle { inner: Arc::new(v) })");
}

#[test]
fn test_wrap_return_with_mutex_vec_opaque() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Item".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.into_iter().map(|v| Item { inner: Arc::new(v) }).collect()");
}
