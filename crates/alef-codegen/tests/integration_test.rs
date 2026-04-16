use ahash::AHashSet;
use alef_codegen::generators::{
    AdapterBodies, AsyncPattern, RustBindingConfig, binding_helpers, gen_constructor, gen_enum, gen_function,
    gen_impl_block, gen_method, gen_static_method, gen_struct,
};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{
    CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind,
    TypeDef, TypeRef,
};
use std::borrow::Cow;

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
    }
}

fn simple_type_def() -> TypeDef {
    TypeDef {
        name: "MyConfig".to_string(),
        rust_path: "my_crate::MyConfig".to_string(),
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
        doc: "A minimal config type.".to_string(),
        cfg: None,
    }
}

fn simple_function_def() -> FunctionDef {
    FunctionDef {
        name: "process".to_string(),
        rust_path: "my_crate::process".to_string(),
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
    assert!(result.contains("#[derive(Clone, Debug)]"), "should have derives");
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
        result.contains("#[derive(Clone, Debug, PartialEq)]"),
        "should have derives"
    );
}

#[test]
fn test_gen_enum_produces_default_impl() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("impl Default for OutputFormat"),
        "should have Default impl"
    );
    assert!(result.contains("Self::Json"), "default should be first variant");
}

#[test]
fn test_gen_struct_with_empty_fields() {
    let typ = TypeDef {
        name: "Empty".to_string(),
        rust_path: "my_crate::Empty".to_string(),
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(&method, &mapper, &cfg, &typ, false, &opaque_types, &adapter_bodies);

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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_static_method(&method, &mapper, &cfg, &typ, &adapter_bodies, &opaque_types);

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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(&method, &mapper, &cfg, &typ, false, &opaque_types, &adapter_bodies);

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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(&method, &mapper, &cfg, &typ, false, &opaque_types, &adapter_bodies);

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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(&method, &mapper, &cfg, &typ, false, &opaque_types, &adapter_bodies);

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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(&method, &mapper, &cfg, &typ, false, &opaque_types, &adapter_bodies);

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

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types);
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

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types);
    assert!(result.contains("let config_core = config.into();"));
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

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types);
    assert!(result.contains("let config_core = config.as_ref();"));
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
    // The result should be `items.as_deref()` which converts Option<Vec<Item>> to Option<&[Item]>
    assert_eq!(result, "items.as_deref()");
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

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types);
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

    let result = binding_helpers::gen_async_body("inner.process()", &cfg, true, "result", false, "", false);

    assert!(result.contains("pyo3_async_runtimes::tokio::future_into_py"));
    assert!(result.contains("await"));
    assert!(result.contains("map_err"));
}

#[test]
fn test_gen_async_body_napi_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;

    let result = binding_helpers::gen_async_body("CoreType::process()", &cfg, true, "result", false, "", false);

    assert!(result.contains("await"));
    assert!(result.contains("map_err"));
    assert!(result.contains("napi::Error"));
}

#[test]
fn test_gen_async_body_wasm_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::WasmNativeAsync;

    let result = binding_helpers::gen_async_body("process_async()", &cfg, true, "result", false, "", false);

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

    let result = binding_helpers::gen_unimplemented_body(&TypeRef::String, "unimplemented_fn", true, &cfg, &params);

    assert!(result.contains("let _ = input;"));
    assert!(result.contains("Err(\"Not implemented"));
}

#[test]
fn test_gen_unimplemented_body_string_return() {
    let cfg = default_cfg();
    let params = vec![];

    let result = binding_helpers::gen_unimplemented_body(&TypeRef::String, "unimplemented_fn", false, &cfg, &params);

    assert!(result.contains("[unimplemented"));
}

#[test]
fn test_gen_unimplemented_body_bool_return() {
    let cfg = default_cfg();
    let params = vec![];

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Primitive(PrimitiveType::Bool),
        "is_valid",
        false,
        &cfg,
        &params,
    );

    assert!(result.contains("false"));
}

#[test]
fn test_gen_unimplemented_body_vec_return() {
    let cfg = default_cfg();
    let params = vec![];

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Vec(Box::new(TypeRef::String)),
        "list_items",
        false,
        &cfg,
        &params,
    );

    assert!(result.contains("Vec::new()"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_sanitized() {
    let mut typ = simple_type_def();
    typ.fields[0].sanitized = true;

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate");

    assert!(result.contains("let core_self"));
    assert!(result.contains("name: Default::default(),"));
    assert!(result.contains("count:"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_non_sanitized() {
    let typ = simple_type_def();

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate");

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

    let result = binding_helpers::gen_lossy_binding_to_core_fields(&typ, "my_crate");

    assert!(result.contains("timeout: std::time::Duration::from_millis(self.timeout),"));
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = {
        let mut set = AHashSet::new();
        set.insert("MyConfig".to_string());
        set
    };

    let result = gen_method(&method, &mapper, &cfg, &typ, true, &opaque_types, &adapter_bodies);

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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(&method, &mapper, &cfg, &typ, false, &opaque_types, &adapter_bodies);

    assert!(
        result.contains("pub fn with_count"),
        "should contain builder method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
    assert!(result.contains(".into()"), "should convert result back to MyConfig");
    assert!(!result.contains("compile_error!"), "should not emit compile_error");
}
