use super::*;

// ==============================================================================
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

/// When `core_crate_override` remaps a source crate (e.g. `spikard` → `spikard_http`),
/// `gen_delegating_default_impl` must rewrite the Default body so it references the
/// override crate instead of the original source crate. Without the fix the body emits
/// `<spikard::ServerConfig as Default>::default()` → E0433 in the wasm binding.
#[test]
fn delegating_default_body_applies_source_crate_remaps() {
    let typ = TypeDef {
        name: "ServerConfig".to_string(),
        rust_path: "spikard::ServerConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: true,
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

    let remaps = [("spikard", "spikard_http")];
    let result = alef::codegen::generators::gen_delegating_default_impl(&typ, "spikard_http", "", &remaps);

    assert!(
        result.contains("<spikard_http::ServerConfig as Default>::default()"),
        "Default body must reference the overridden crate, not the original; got:\n{result}"
    );
    assert!(
        !result.contains("<spikard::ServerConfig"),
        "Default body must NOT reference the original source crate after remapping; got:\n{result}"
    );
}

/// Without remaps, the Default body falls through to the original rust_path unchanged.
#[test]
fn delegating_default_body_without_remaps_uses_original_path() {
    let typ = TypeDef {
        name: "ServerConfig".to_string(),
        rust_path: "spikard::ServerConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: true,
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

    let result = alef::codegen::generators::gen_delegating_default_impl(&typ, "spikard", "", &[]);

    assert!(
        result.contains("<spikard::ServerConfig as Default>::default()"),
        "Default body must use original rust_path when no remaps; got:\n{result}"
    );
}

/// When `emit_delegating_default_for_types` is set and the type is NOT in the set
/// (i.e. no From<core::T> impl will be emitted), the struct must keep `#[derive(Default)]`
/// and must NOT emit a delegating `impl Default`. Emitting the delegating impl without
/// From<core::T> causes E0277 at compilation.
#[test]
fn delegating_default_suppressed_when_type_not_in_convertible_set() {
    use ahash::AHashSet;

    let typ = TypeDef {
        name: "ServerConfig".to_string(),
        rust_path: "spikard::ServerConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("ExcludedOpaque".to_string()),
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
        has_default: true,
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

    let convertible_set: AHashSet<String> = AHashSet::new();
    let mut cfg = default_cfg();
    cfg.emit_delegating_default_impl = true;
    cfg.emit_delegating_default_for_types = Some(&convertible_set);

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(
        result.contains("#[derive(") && result.contains("Default"),
        "ServerConfig must keep #[derive(Default)] when not in the convertible set; got:\n{result}"
    );
    assert!(
        !result.contains("impl Default for ServerConfig"),
        "ServerConfig must NOT have a delegating impl Default when excluded from convertible set; got:\n{result}"
    );
}

/// When the type IS in `emit_delegating_default_for_types`, the delegating Default IS emitted
/// and `#[derive(Default)]` is suppressed — unchanged from the original behavior for
/// types that do have a matching From<core::T>.
#[test]
fn delegating_default_emitted_when_type_in_convertible_set() {
    use ahash::AHashSet;

    let typ = TypeDef {
        name: "Config".to_string(),
        rust_path: "my_crate::Config".to_string(),
        original_rust_path: String::new(),
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
        has_default: true,
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

    let mut convertible_set: AHashSet<String> = AHashSet::new();
    convertible_set.insert("Config".to_string());
    let mut cfg = default_cfg();
    cfg.core_import = "my_crate";
    cfg.emit_delegating_default_impl = true;
    cfg.emit_delegating_default_for_types = Some(&convertible_set);

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(
        result.contains("impl Default for Config"),
        "Config must have delegating impl Default when in convertible set; got:\n{result}"
    );
    assert!(
        result.contains("<my_crate::Config as Default>::default().into()"),
        "delegating Default must delegate via core type; got:\n{result}"
    );
    let derive_block = result.split("pub struct Config").next().unwrap_or("");
    assert!(
        !derive_block.contains("Default"),
        "#[derive(Default)] must be suppressed when delegating impl is emitted; got:\n{derive_block}"
    );
}

/// When a struct has a field whose type appears in `excluded_field_types`, that field
/// is excluded from the binding surface AND the From impl, so it cannot make the
/// parent struct non-convertible. Without the fix, `ServerConfig` was wrongly excluded
/// because `CorsConfig` (a type not defined in the surface) failed `is_field_convertible`.
///
/// With `excluded_field_types = ["CorsConfig"]`, the `cors` field is skipped in the
/// predicate, so `ServerConfig` stays in the convertible set.
#[test]
fn core_to_binding_convertible_excludes_excluded_type_fields() {
    let server_config = TypeDef {
        name: "ServerConfig".to_string(),
        rust_path: "my_crate::ServerConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "compression".to_string(),
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
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "cors".to_string(),
                ty: TypeRef::Named("CorsConfig".to_string()),
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
            },
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

    let surface = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![server_config],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let without_hint = alef::codegen::conversions::core_to_binding_convertible_types(&surface, &[]);
    assert!(
        !without_hint.contains("ServerConfig"),
        "ServerConfig must be absent when CorsConfig is unknown and no excluded_field_types hint is given; got set: {without_hint:?}"
    );

    let cors_excluded = vec!["CorsConfig".to_string()];
    let with_hint = alef::codegen::conversions::core_to_binding_convertible_types(&surface, &cors_excluded);
    assert!(
        with_hint.contains("ServerConfig"),
        "ServerConfig must be present when CorsConfig is in excluded_field_types; got set: {with_hint:?}"
    );
}
