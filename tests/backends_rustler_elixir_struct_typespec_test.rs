use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};

/// Build a minimal ResolvedCrateConfig for elixir tests.
fn make_config(app_name: &str) -> ResolvedCrateConfig {
    let crate_name = app_name.replace('_', "-");
    let toml = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "{app_name}"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

/// Build a minimal FieldDef.
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

#[test]
fn test_struct_module_emits_type_t_typespec_with_correct_field_types() {
    let struct_def = TypeDef {
        name: "CodeChunk".to_string(),
        rust_path: "my_crate::CodeChunk".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("content", TypeRef::String, true),
            make_field("start_byte", TypeRef::Primitive(PrimitiveType::U64), false),
            make_field("end_byte", TypeRef::Primitive(PrimitiveType::U64), false),
            make_field("is_valid", TypeRef::Primitive(PrimitiveType::Bool), false),
            make_field(
                "metadata",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                true,
            ),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "A code chunk".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![struct_def.clone()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = RustlerBackend;
    let generated = backend
        .generate_public_api(&api, &config)
        .expect("generation must succeed");

    let struct_module = generated
        .iter()
        .find(|f| {
            let path_str = f.path.to_string_lossy();
            path_str.contains("code_chunk") && path_str.ends_with(".ex")
        })
        .expect("should generate CodeChunk module");

    let content = &struct_module.content;

    assert!(
        content.contains("@type t ::"),
        "struct module must emit @type t typespec; got:\n{content}"
    );

    assert!(
        content.contains("defstruct"),
        "struct module must still have defstruct; got:\n{content}"
    );

    let type_pos = content.find("@type t ::").expect("@type t must be present");
    let defstruct_pos = content.find("defstruct").expect("defstruct must be present");
    assert!(type_pos < defstruct_pos, "@type t must appear before defstruct");

    assert!(
        content.contains("content: String.t() | nil"),
        "optional String field should be `String.t() | nil`; got:\n{content}"
    );
    assert!(
        content.contains("start_byte: non_neg_integer()"),
        "u64 field should be `non_neg_integer()`; got:\n{content}"
    );
    assert!(
        content.contains("is_valid: boolean()"),
        "bool field should be `boolean()`; got:\n{content}"
    );
    assert!(
        content.contains("metadata: map() | nil"),
        "optional map field should be `map() | nil`; got:\n{content}"
    );

    assert!(
        content.contains("%__MODULE__{"),
        "typespec must use %__MODULE__{{ ... }}; got:\n{content}"
    );
}

#[test]
fn test_struct_module_defstruct_defaults_align_with_typespec() {
    let struct_def = TypeDef {
        name: "DefaultAlignmentTest".to_string(),
        rust_path: "my_crate::DefaultAlignmentTest".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("nullable_string", TypeRef::String, true),
            make_field("required_string", TypeRef::String, false),
            make_field("nullable_number", TypeRef::Primitive(PrimitiveType::U32), true),
            make_field("required_number", TypeRef::Primitive(PrimitiveType::U32), false),
            make_field("nullable_bool", TypeRef::Primitive(PrimitiveType::Bool), true),
            make_field("required_bool", TypeRef::Primitive(PrimitiveType::Bool), false),
            make_field("nullable_list", TypeRef::Vec(Box::new(TypeRef::String)), true),
            make_field("required_list", TypeRef::Vec(Box::new(TypeRef::String)), false),
            make_field(
                "nullable_map",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                true,
            ),
            make_field(
                "required_map",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                false,
            ),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Test struct for default alignment".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![struct_def.clone()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = RustlerBackend;
    let generated = backend
        .generate_public_api(&api, &config)
        .expect("generation must succeed");

    let struct_module = generated
        .iter()
        .find(|f| {
            let path_str = f.path.to_string_lossy();
            path_str.contains("default_alignment_test") && path_str.ends_with(".ex")
        })
        .expect("should generate DefaultAlignmentTest module");

    let content = &struct_module.content;

    assert!(
        content.contains("nullable_string: nil"),
        "nullable string field should default to nil; got:\n{content}"
    );
    assert!(
        content.contains("nullable_number: nil"),
        "nullable number field should default to nil; got:\n{content}"
    );
    assert!(
        content.contains("nullable_bool: nil"),
        "nullable bool field should default to nil; got:\n{content}"
    );
    assert!(
        content.contains("nullable_list: nil"),
        "nullable list field should default to nil; got:\n{content}"
    );
    assert!(
        content.contains("nullable_map: nil"),
        "nullable map field should default to nil; got:\n{content}"
    );

    assert!(
        content.contains("required_string: nil"),
        "required string field should default to nil; got:\n{content}"
    );
    assert!(
        content.contains("required_number: 0"),
        "required number field should default to 0; got:\n{content}"
    );
    assert!(
        content.contains("required_bool: false"),
        "required bool field should default to false; got:\n{content}"
    );
    assert!(
        content.contains("required_list: []"),
        "required list field should default to []; got:\n{content}"
    );
    assert!(
        content.contains("required_map: %{}"),
        "required map field should default to empty map; got:\n{content}"
    );
}

#[test]
fn test_struct_module_with_named_type_field() {
    let struct_def = TypeDef {
        name: "Container".to_string(),
        rust_path: "my_crate::Container".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("inner", TypeRef::Named("InnerType".to_string()), false),
            make_field("optional_inner", TypeRef::Named("InnerType".to_string()), true),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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

    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![struct_def.clone()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = RustlerBackend;
    let generated = backend
        .generate_public_api(&api, &config)
        .expect("generation must succeed");

    let struct_module = generated
        .iter()
        .find(|f| {
            let path_str = f.path.to_string_lossy();
            path_str.contains("container") && path_str.ends_with(".ex")
        })
        .expect("should generate Container module");

    let content = &struct_module.content;

    assert!(
        content.contains("inner: map()"),
        "named type field should be `map()`; got:\n{content}"
    );
    assert!(
        content.contains("optional_inner: map() | nil"),
        "optional named type field should be `map() | nil`; got:\n{content}"
    );
}

#[test]
fn test_struct_module_with_known_named_type_fields() {
    let url_config = TypeDef {
        name: "UrlExtractionConfig".to_string(),
        rust_path: "my_crate::UrlExtractionConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("crawl", TypeRef::Named("CrawlConfig".to_string()), false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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
    let crawl_config = TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "my_crate::CrawlConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("content", TypeRef::Named("ContentConfig".to_string()), false),
            make_field("browser", TypeRef::Named("BrowserConfig".to_string()), false),
            make_field(
                "proxy",
                TypeRef::Optional(Box::new(TypeRef::Named("ProxyConfig".to_string()))),
                false,
            ),
            make_field(
                "ssrf_policy",
                TypeRef::Optional(Box::new(TypeRef::Named("SsrfPolicy".to_string()))),
                false,
            ),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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
    let content_config = TypeDef {
        name: "ContentConfig".to_string(),
        rust_path: "my_crate::ContentConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("markdown", TypeRef::Primitive(PrimitiveType::Bool), false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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
    let browser_config = TypeDef {
        name: "BrowserConfig".to_string(),
        rust_path: "my_crate::BrowserConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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
    let proxy_config = TypeDef {
        name: "ProxyConfig".to_string(),
        rust_path: "my_crate::ProxyConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("url", TypeRef::String, true)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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
    let ssrf_policy = TypeDef {
        name: "SsrfPolicy".to_string(),
        rust_path: "my_crate::SsrfPolicy".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field(
            "allow_loopback",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
        )],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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

    let config = make_config("xberg");
    let api = ApiSurface {
        crate_name: "xberg".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![
            url_config,
            crawl_config,
            content_config,
            browser_config,
            proxy_config,
            ssrf_policy,
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = RustlerBackend;
    let generated = backend
        .generate_public_api(&api, &config)
        .expect("generation must succeed");
    let url_module = generated
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("url_extraction_config.ex"))
        .expect("should generate UrlExtractionConfig module");
    let crawl_module = generated
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("crawl_config.ex"))
        .expect("should generate CrawlConfig module");

    assert!(
        url_module.content.contains("crawl: Xberg.CrawlConfig.t()"),
        "known nested DTO should use its public module type; got:\n{}",
        url_module.content
    );
    assert!(
        crawl_module.content.contains("content: Xberg.ContentConfig.t()"),
        "known nested DTO should use its public module type; got:\n{}",
        crawl_module.content
    );
    assert!(
        crawl_module.content.contains("browser: Xberg.BrowserConfig.t()"),
        "known nested DTO should use its public module type; got:\n{}",
        crawl_module.content
    );
    assert!(
        crawl_module.content.contains("proxy: Xberg.ProxyConfig.t() | nil"),
        "optional known nested DTO should be nilable; got:\n{}",
        crawl_module.content
    );
    assert!(
        crawl_module.content.contains("ssrf_policy: Xberg.SsrfPolicy.t() | nil"),
        "optional known nested DTO should be nilable; got:\n{}",
        crawl_module.content
    );
}

#[test]
fn test_struct_module_with_vec_fields() {
    let struct_def = TypeDef {
        name: "Collector".to_string(),
        rust_path: "my_crate::Collector".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("strings", TypeRef::Vec(Box::new(TypeRef::String)), false),
            make_field(
                "numbers",
                TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
                false,
            ),
            make_field(
                "optional_items",
                TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
                true,
            ),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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

    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![struct_def.clone()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = RustlerBackend;
    let generated = backend
        .generate_public_api(&api, &config)
        .expect("generation must succeed");

    let struct_module = generated
        .iter()
        .find(|f| {
            let path_str = f.path.to_string_lossy();
            path_str.contains("collector") && path_str.ends_with(".ex")
        })
        .expect("should generate Collector module");

    let content = &struct_module.content;

    assert!(
        content.contains("strings: [String.t()]"),
        "vec of string should be `[String.t()]`; got:\n{content}"
    );
    assert!(
        content.contains("numbers: [integer()]"),
        "vec of i32 should be `[integer()]`; got:\n{content}"
    );
    assert!(
        content.contains("optional_items: [map()] | nil"),
        "optional vec of named should be `[map()] | nil`; got:\n{content}"
    );
}
