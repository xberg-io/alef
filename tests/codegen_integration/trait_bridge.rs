use super::*;

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
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
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
        doc: "A test trait.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn simple_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
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
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
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
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
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
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
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
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_wrapper_struct(&spec, &generator);

    assert!(
        result.contains("pub struct PythonMyTraitBridge {"),
        "should have wrapper struct"
    );
    assert!(
        result.contains("inner: MockObject"),
        "should have inner field with foreign type"
    );
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
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_trait_impl(&spec, &generator);

    assert!(
        result.contains("impl my_crate::MyTrait for PythonMyTraitBridge {"),
        "should implement the trait"
    );
    assert!(result.contains("fn execute("), "should have required execute method");
    // optional_method has has_default_impl=true — it must NOT get a generated body so the
    // trait's own default implementation takes effect.
    assert!(
        !result.contains("fn optional_method("),
        "should NOT generate body for optional_method (has_default_impl=true); got:\n{result}"
    );
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
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_all(&spec, &generator);

    assert!(
        result.imports.contains(&"mock::MockObject".to_string()),
        "should include bridge imports"
    );
    assert!(
        result.code.contains("pub struct PythonMyTraitBridge"),
        "should have wrapper struct"
    );
    assert!(
        result.code.contains("impl PythonMyTraitBridge {"),
        "should have constructor impl"
    );
    assert!(
        result.code.contains("impl my_crate::MyTrait for PythonMyTraitBridge"),
        "should have trait impl"
    );
    // No register_fn configured, so registration function should be absent
    assert!(
        !result.code.contains("pub fn register"),
        "should not have registration fn when not configured"
    );
}

#[test]
fn test_gen_bridge_all_includes_registration_fn_when_configured() {
    let trait_def = simple_trait_def();
    let bridge_config = TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: Some("register_my_trait".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_config,
        core_import: "my_crate",
        wrapper_prefix: "Python",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };
    let generator = MockBridgeGenerator;

    let result = gen_bridge_all(&spec, &generator);

    assert!(
        result.code.contains("pub fn register_my_trait"),
        "should include registration function when register_fn is set"
    );
}

#[test]
fn test_format_type_ref_primitives() {
    let type_paths = HashMap::new();

    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::Bool), &type_paths),
        "bool"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::U8), &type_paths),
        "u8"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::U16), &type_paths),
        "u16"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::U32), &type_paths),
        "u32"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::U64), &type_paths),
        "u64"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::I32), &type_paths),
        "i32"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::I64), &type_paths),
        "i64"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::F32), &type_paths),
        "f32"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::F64), &type_paths),
        "f64"
    );
    assert_eq!(
        format_type_ref(&TypeRef::Primitive(PrimitiveType::Usize), &type_paths),
        "usize"
    );
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

    let map = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Primitive(PrimitiveType::U64)),
    );
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    };

    assert_eq!(
        format_param_type(&param, &type_paths),
        "String",
        "without is_ref, String is owned"
    );
}
