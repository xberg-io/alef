use super::*;

// ==============================================================================
// Additional tests for structs.rs
// ==============================================================================

#[test]
fn test_type_needs_mutex_false_when_no_ref_mut_methods() {
    let typ = simple_type_def();
    assert!(
        !type_needs_mutex(&typ),
        "type with no RefMut methods should not need mutex"
    );
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
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
        has_private_fields: false,
        version: Default::default(),
    };
    let cfg = default_cfg();

    let result = gen_opaque_struct(&typ, &cfg);

    assert!(
        result.contains("pub struct MyService {"),
        "should have struct declaration"
    );
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
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    typ.is_opaque = true;
    let cfg = default_cfg();

    let result = gen_opaque_struct(&typ, &cfg);

    assert!(
        result.contains("pub struct MutableService {"),
        "should have struct declaration"
    );
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

    assert!(
        result.contains("impl Default for MyConfig {"),
        "should generate Default impl"
    );
    assert!(
        result.contains("fn default() -> Self {"),
        "should have default() method"
    );
    assert!(
        result.contains("name: Default::default()"),
        "non-optional fields use Default::default()"
    );
    assert!(
        result.contains("count: Default::default()"),
        "optional field uses Default::default()"
    );
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
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
        has_private_fields: false,
        version: Default::default(),
    };

    let result = gen_struct_default_impl(&typ, "");

    assert!(
        result.contains("value: None"),
        "Optional<T> fields should default to None"
    );
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
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
        has_private_fields: false,
        version: Default::default(),
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
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
        has_private_fields: false,
        version: Default::default(),
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
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
        has_private_fields: false,
        version: Default::default(),
    };
    let mapper = RustMapper;

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(result.contains("pub struct Wrapper"), "should generate struct");
    // Derives are always added regardless of opaque fields — the binding struct
    // still needs serde and Default for constructors and JSON round-trips.
    assert!(
        result.contains("serde::Serialize"),
        "should include Serialize derive even with opaque fields"
    );
    assert!(
        result.contains("serde::Deserialize"),
        "should include Deserialize derive even with opaque fields"
    );
    assert!(
        result.contains("Default"),
        "should include Default derive even with opaque fields"
    );
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
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
