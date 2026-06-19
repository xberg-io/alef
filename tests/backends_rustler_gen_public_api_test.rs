use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, DefaultValue, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef,
    MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

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
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// Build a FieldDef with a typed default.
fn make_field_with_default(name: &str, ty: TypeRef, default: DefaultValue) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(default),
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

/// Build a MethodDef with no receiver (static method).
fn make_static_method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: true,
        error_type: None,
        doc: format!("Static method {name}"),
        receiver: None,
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

/// Build a MethodDef with a receiver (instance method).
fn make_instance_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: format!("Instance method {name}"),
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
    }
}

#[test]
fn test_generate_public_api_creates_all_files() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "my_lib::ConversionOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("heading_style", TypeRef::Named("HeadingStyle".to_string()), false),
                make_field("wrap_width", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("debug", TypeRef::Primitive(PrimitiveType::Bool), false),
            ],
            methods: vec![make_static_method(
                "default",
                TypeRef::Named("ConversionOptions".to_string()),
            )],
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
            doc: "Options for conversion".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "html".to_string(),
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
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ConversionOptions".to_string()),
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Convert markup conversion".to_string(),
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
        enums: vec![EnumDef {
            name: "HeadingStyle".to_string(),
            rust_path: "my_lib::HeadingStyle".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Setext".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config("my_lib");
    let result = backend.generate_public_api(&api, &config);

    assert!(result.is_ok(), "generate_public_api should succeed: {:?}", result);
    let files = result.unwrap();

    let paths: Vec<String> = files
        .iter()
        .map(|f| f.path.to_string_lossy().replace('\\', "/"))
        .collect();

    // Should generate the main module file
    assert!(
        paths.iter().any(|p| p.ends_with("my_lib.ex")),
        "Should generate my_lib.ex; got: {paths:?}"
    );

    // Should generate native.ex
    assert!(
        paths.iter().any(|p| p.ends_with("my_lib/native.ex")),
        "Should generate my_lib/native.ex; got: {paths:?}"
    );

    // Should generate struct module for ConversionOptions
    assert!(
        paths.iter().any(|p| p.ends_with("my_lib/conversion_options.ex")),
        "Should generate my_lib/conversion_options.ex; got: {paths:?}"
    );

    // Should generate enum module for HeadingStyle
    assert!(
        paths.iter().any(|p| p.ends_with("my_lib/heading_style.ex")),
        "Should generate my_lib/heading_style.ex; got: {paths:?}"
    );
}

#[test]
fn test_native_ex_has_all_nif_stubs() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "my_lib::ConversionOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("debug", TypeRef::Primitive(PrimitiveType::Bool), false)],
            methods: vec![
                make_static_method("default", TypeRef::Named("ConversionOptions".to_string())),
                make_instance_method("any_enabled", vec![], TypeRef::Primitive(PrimitiveType::Bool)),
            ],
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
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "html".to_string(),
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

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let native = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/native.ex")
        })
        .expect("native.ex should be generated");

    let content = &native.content;

    // Should declare the module correctly
    assert!(
        content.contains("defmodule MyLib.Native do"),
        "Should define MyLib.Native; content:\n{content}"
    );

    // Should use RustlerPrecompiled
    assert!(
        content.contains("use RustlerPrecompiled"),
        "Should use RustlerPrecompiled; content:\n{content}"
    );

    // Should have otp_app atom
    assert!(
        content.contains("otp_app: :my_lib"),
        "Should have otp_app: :my_lib; content:\n{content}"
    );

    // Should have stub for convert/1
    assert!(
        content.contains("def convert(") && content.contains("nif_not_loaded"),
        "Should have convert stub; content:\n{content}"
    );

    // Should have stub for static method: conversionoptions_default/0
    assert!(
        content.contains("def conversionoptions_default"),
        "Should have conversionoptions_default/0 stub; content:\n{content}"
    );

    // Should have stub for instance method: conversionoptions_any_enabled/1
    assert!(
        content.contains("def conversionoptions_any_enabled("),
        "Should have conversionoptions_any_enabled stub; content:\n{content}"
    );

    // base_url and targets are emitted single-line; mix format handles layout per
    // downstream `.formatter.exs` `line_length:` (default 98). Pre-wrapping at the
    // generator side caused an idempotence fight against repos with line_length > 98.
    let base_url_line = content
        .lines()
        .find(|l| l.trim_start().starts_with("base_url:"))
        .expect("base_url line should be present");
    assert!(
        base_url_line.contains("\"http"),
        "base_url should be emitted single-line with URL inline: {base_url_line:?}"
    );
}

#[test]
fn test_struct_module_has_defstruct() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "my_lib::ConversionOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field_with_default(
                    "heading_style",
                    TypeRef::Named("HeadingStyle".to_string()),
                    DefaultValue::EnumVariant("Setext".to_string()),
                ),
                make_field_with_default(
                    "wrap_width",
                    TypeRef::Primitive(PrimitiveType::U32),
                    DefaultValue::IntLiteral(80),
                ),
                make_field_with_default(
                    "debug",
                    TypeRef::Primitive(PrimitiveType::Bool),
                    DefaultValue::BoolLiteral(false),
                ),
                make_field("title", TypeRef::String, true),
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
            doc: "Options for conversion".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "HeadingStyle".to_string(),
            rust_path: "my_lib::HeadingStyle".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Setext".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let struct_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/conversion_options.ex")
        })
        .expect("conversion_options.ex should be generated");

    let content = &struct_file.content;

    // Should define the correct module
    assert!(
        content.contains("defmodule MyLib.ConversionOptions do"),
        "Should define MyLib.ConversionOptions; content:\n{content}"
    );

    // Should have defstruct
    assert!(
        content.contains("defstruct "),
        "Should have defstruct; content:\n{content}"
    );

    // Should have correct field defaults
    assert!(
        content.contains("heading_style: :setext"),
        "Should have heading_style: :setext; content:\n{content}"
    );
    assert!(
        content.contains("wrap_width: 80"),
        "Should have wrap_width: 80; content:\n{content}"
    );
    assert!(
        content.contains("debug: false"),
        "Should have debug: false; content:\n{content}"
    );
    // Optional fields default to nil
    assert!(
        content.contains("title: nil"),
        "Should have title: nil; content:\n{content}"
    );
}

#[test]
fn test_main_module_has_method_wrappers() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "my_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![
                make_static_method("default", TypeRef::Named("Config".to_string())),
                make_instance_method("validate", vec![], TypeRef::Primitive(PrimitiveType::Bool)),
            ],
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

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("my_lib.ex"))
        .expect("my_lib.ex should be generated");

    let content = &main.content;

    // Should define the main module
    assert!(
        content.contains("defmodule MyLib do"),
        "Should define MyLib module; content:\n{content}"
    );

    // Methods should NOT be in main module; they're in type modules now
    assert!(
        !content.contains("def config_default"),
        "Methods should NOT be in main module; got:\n{content}"
    );
}

#[test]
fn test_trait_bridge_unregister_and_clear_specs_match_atom_returns() {
    let backend = RustlerBackend;
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
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
    let mut config = make_config("my_lib");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let files = backend.generate_public_api(&api, &config).unwrap();
    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("my_lib.ex"))
        .expect("my_lib.ex should be generated");
    let content = &main.content;

    assert!(
        content.contains("@spec unregister_ocr_backend(String.t()) :: :ok | :error")
            && content.contains("@spec clear_ocr_backends() :: :ok | :error"),
        "unregister/clear specs must match Rustler NIF atom returns; got:\n{content}"
    );
    assert!(
        !content.contains("{:ok, nil}") && !content.contains("{:error, atom, String.t()}"),
        "unregister/clear specs must not advertise tuple returns; got:\n{content}"
    );
}

#[test]
fn test_opaque_types_not_get_struct_module() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Engine".to_string(),
            rust_path: "my_lib::Engine".to_string(),
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

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    // Opaque types get a dedicated wrapper module that wraps a ResourceArc
    // reference (`defstruct [:ref]`), distinct from the value-struct modules
    // emitted for non-opaque types.
    let engine_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/engine.ex")
        })
        .expect("opaque type Engine should produce an engine.ex wrapper module");
    assert!(
        engine_file.content.contains("defstruct [:ref]"),
        "opaque wrapper module must use ResourceArc-reference defstruct; content:\n{}",
        engine_file.content
    );
    assert!(
        !engine_file.content.contains("@type t :: %__MODULE__{") || engine_file.content.contains("ref: reference()"),
        "opaque wrapper module's typespec must describe a Rustler ResourceArc reference; content:\n{}",
        engine_file.content
    );
}

#[test]
fn test_simple_enum_module_has_type_and_accessors() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "HeadingStyle".to_string(),
            rust_path: "my_lib::HeadingStyle".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Setext".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Heading style for Markdown output".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let enum_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/heading_style.ex")
        })
        .expect("heading_style.ex should be generated");

    let content = &enum_file.content;

    // Correct module name
    assert!(
        content.contains("defmodule MyLib.HeadingStyle do"),
        "Should define MyLib.HeadingStyle; content:\n{content}"
    );

    // Moduledoc from enum doc
    assert!(
        content.contains("@moduledoc \"Heading style for Markdown output\""),
        "Should have moduledoc from enum doc; content:\n{content}"
    );

    // @type t union of atoms
    assert!(
        content.contains("@type t :: :setext | :atx"),
        "Should have @type t with atom union; content:\n{content}"
    );

    // Accessor functions for each variant
    assert!(
        content.contains("def setext"),
        "Should have setext/0 accessor; content:\n{content}"
    );
    assert!(
        content.contains("def atx"),
        "Should have atx/0 accessor; content:\n{content}"
    );
}

#[test]
fn test_generate_bindings_nif_init_uses_native_module() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "html".to_string(),
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

    let config = make_config("my_lib");
    let files = backend.generate_bindings(&api, &config).unwrap();

    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("lib.rs"))
        .expect("lib.rs should be generated");

    // The rustler::init! should use the .Native module name to match native.ex
    assert!(
        lib_rs.content.contains("Elixir.MyLib.Native"),
        "rustler::init! should reference Elixir.MyLib.Native; content:\n{}",
        &lib_rs.content[lib_rs.content.len().saturating_sub(200)..]
    );
}

/// A data-enum variant named `Function` snake-cases to `function`, which is an Elixir
/// built-in type. The generated `@type` declaration must use `function_variant` to avoid
/// a `Kernel.TypespecError: type function/0 is a built-in type and it cannot be redefined`.
#[test]
fn test_builtin_type_function_variant_uses_safe_type_name() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "my_lib::Message".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Function".to_string(),
                    fields: vec![make_field("name", TypeRef::String, false)],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![make_field("content", TypeRef::String, false)],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let enum_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/message.ex")
        })
        .expect("message.ex should be generated");

    let content = &enum_file.content;

    assert!(
        !content.contains("@type function ::"),
        "Must not emit reserved `@type function ::`; content:\n{content}"
    );
    assert!(
        content.contains("@type function_variant ::"),
        "Should emit `@type function_variant ::` for the renamed type; content:\n{content}"
    );
}

/// The generated `native.ex` must include the force-build guard used in local
/// test/dev builds and explicit environment overrides.
#[test]
fn test_native_ex_emits_force_build_guard() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
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

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let native = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/native.ex")
        })
        .expect("native.ex should be generated");

    assert!(
        native
            .content
            .contains(r#"force_build: System.get_env("MY_LIB_BUILD") in ["1", "true"] or Mix.env() in [:dev]"#),
        "force_build guard should be present in native.ex; content:\n{}",
        &native.content
    );
}

/// A simple-enum variant named `Doc` snake-cases to `doc`, which is a reserved Elixir
/// module attribute. Emitting `@doc :doc` causes a compiler error. The generator must
/// use `@doc_attr :doc` and `def doc, do: @doc_attr` instead.
#[test]
fn test_reserved_attr_doc_variant_uses_safe_name() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "CommentKind".to_string(),
            rust_path: "my_lib::CommentKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Doc".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Line".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let enum_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/comment_kind.ex")
        })
        .expect("comment_kind.ex should be generated");

    let content = &enum_file.content;

    assert!(
        !content.contains("@doc :doc"),
        "Must not emit reserved `@doc :doc`; content:\n{content}"
    );
    assert!(
        content.contains("@doc_attr :doc"),
        "Should emit `@doc_attr :doc` for the safe attribute name; content:\n{content}"
    );
    assert!(
        content.contains("def doc, do: @doc_attr"),
        "Should emit `def doc, do: @doc_attr`; content:\n{content}"
    );
    assert!(
        content.contains("@spec doc() :: t()"),
        "Should emit `@spec doc() :: t()`; content:\n{content}"
    );
}

/// Functions with trailing optional params must emit a single def with `opts \\ []`
/// instead of one def per arity combination.
#[test]
fn test_trailing_optional_params_emit_keyword_opts_function() {
    let backend = RustlerBackend;

    fn make_param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional,
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

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "create_client".to_string(),
            rust_path: "my_lib::create_client".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_param("api_key", TypeRef::String, false),
                make_param("base_url", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_param(
                    "timeout_secs",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                    true,
                ),
            ],
            return_type: TypeRef::Named("Client".to_string()),
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Create a client".to_string(),
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

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("my_lib.ex"))
        .expect("my_lib.ex should be generated");

    let content = &main.content;

    // (a) Exactly one `def create_client` definition — not one per arity.
    let def_count = content.matches("def create_client(").count();
    assert_eq!(
        def_count, 1,
        "Should emit exactly one def create_client (keyword-opts form); got {def_count}; content:\n{content}"
    );

    // (b) Keyword.get pattern for optional params.
    assert!(
        content.contains("Keyword.get(opts, :base_url)"),
        "Should emit Keyword.get(opts, :base_url); content:\n{content}"
    );
    assert!(
        content.contains("Keyword.get(opts, :timeout_secs)"),
        "Should emit Keyword.get(opts, :timeout_secs); content:\n{content}"
    );

    // (c) The `opts \\ []` default is present in the function signature.
    assert!(
        content.contains("opts \\\\ []"),
        "Should emit opts \\\\ [] default argument; content:\n{content}"
    );

    // (d) Required param is positional, not keyword.
    assert!(
        content.contains("def create_client(api_key, opts"),
        "Required param api_key must be positional; content:\n{content}"
    );
}

/// `defstruct` for struct types must default String fields to `nil` (not `""`).
/// Empty-string sentinels hide absence — `nil` is the idiomatic Elixir absent value.
#[test]
fn test_defstruct_string_fields_default_to_nil() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Message".to_string(),
            rust_path: "my_lib::Message".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("role", TypeRef::String, false),
                make_field("content", TypeRef::String, false),
                make_field("name", TypeRef::String, false),
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

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let struct_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/message.ex")
        })
        .expect("message.ex should be generated");

    let content = &struct_file.content;

    // (c) defstruct fields must default to nil, not "".
    assert!(
        !content.contains(": \"\""),
        "defstruct String fields must not default to \"\"; content:\n{content}"
    );
    assert!(
        content.contains("role: nil"),
        "defstruct String field must default to nil; content:\n{content}"
    );
    assert!(
        content.contains("content: nil"),
        "defstruct String field must default to nil; content:\n{content}"
    );
}

/// Build a FunctionDef with no params and an explicit `doc`.
fn make_function_with_doc(name: &str, doc: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: doc.to_string(),
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

fn render_native_ex(functions: Vec<FunctionDef>) -> String {
    let backend = RustlerBackend;
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions,
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config("my_lib");
    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api should succeed");
    files
        .into_iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/native.ex")
        })
        .expect("native.ex should be generated")
        .content
}

#[test]
fn test_native_ex_emits_single_line_doc_above_nif_stub() {
    let content = render_native_ex(vec![make_function_with_doc("convert", "Convert markup conversion.")]);
    // Single-line doc → `@doc "..."` directly above the def, no blank between them.
    assert!(
        content.contains("  @doc \"Convert markup conversion.\"\n  def convert"),
        "Single-line @doc must attach directly to its def; content:\n{content}"
    );
}

#[test]
fn test_native_ex_emits_multiline_doc_heredoc_above_nif_stub() {
    let doc = "Convert markup conversion.\n\nSupports nested lists and tables.";
    let content = render_native_ex(vec![make_function_with_doc("convert", doc)]);
    assert!(
        content.contains(
            "  @doc \"\"\"\n  Convert markup conversion.\n\n  Supports nested lists and tables.\n  \"\"\"\n  def convert"
        ),
        "Multi-line @doc must emit an indented heredoc attached to its def; content:\n{content}"
    );
}

#[test]
fn test_native_ex_omits_doc_when_function_has_no_rustdoc() {
    let content = render_native_ex(vec![make_function_with_doc("convert", "")]);
    // No @doc anywhere in the Native module when the function has no doc.
    assert!(
        !content.contains("@doc"),
        "Native module must not emit @doc when the source has no rustdoc; content:\n{content}"
    );
    // Stub itself is still emitted.
    assert!(
        content.contains("  def convert"),
        "Stub must still be present; content:\n{content}"
    );
}

#[test]
fn test_native_ex_escapes_quotes_in_single_line_doc() {
    let content = render_native_ex(vec![make_function_with_doc("convert", "Quote: \"hi\" and slash: \\.")]);
    // Both double-quotes and backslashes must be escaped inside the "..." form.
    assert!(
        content.contains("  @doc \"Quote: \\\"hi\\\" and slash: \\\\.\"\n"),
        "Embedded \" and \\ must be escaped in single-line @doc; content:\n{content}"
    );
}

#[test]
fn test_native_ex_breaks_triple_quote_in_multiline_doc() {
    let doc = "Example:\n\"\"\"\ncode\n\"\"\"";
    let content = render_native_ex(vec![make_function_with_doc("convert", doc)]);
    // Triple-quote sequences inside the body must be broken so they don't close the heredoc.
    assert!(
        !content.contains("\n  \"\"\"\n  code"),
        "Embedded \\\"\\\"\\\" must not survive verbatim and close the heredoc early; content:\n{content}"
    );
    assert!(
        content.contains("\"\" \""),
        "Embedded \\\"\\\"\\\" must be split into `\"\" \"`; content:\n{content}"
    );
}

#[test]
fn test_native_ex_separates_consecutive_docced_stubs_with_blank_line() {
    let content = render_native_ex(vec![
        make_function_with_doc("first", "First."),
        make_function_with_doc("second", "Second."),
    ]);
    // Two single-line def blocks with @doc must be separated by exactly one blank line
    // (mix format requires the @doc-block to be visually distinct).
    assert!(
        content.contains("  def first, do: :erlang.nif_error(:nif_not_loaded)\n\n  @doc \"Second.\"\n  def second"),
        "Consecutive docced stubs must have a blank line separator; content:\n{content}"
    );
}

/// Regression test for M2: the wrapper Elixir module must emit the full first-paragraph
/// summary (physical lines joined with a space) rather than only the first physical line.
#[test]
fn test_wrapper_module_doc_uses_full_first_paragraph_summary() {
    let backend = RustlerBackend;
    // Two-line summary that wraps across physical lines (rustdoc convention).
    let doc = "Convert markup conversion, returning\na ConversionResult.\n\n# Arguments\n\n* `html` - Input.";
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: doc.to_string(),
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
    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("my_lib.ex"))
        .expect("my_lib.ex must be generated");
    let content = &wrapper.content;
    assert!(
        content.contains("Convert markup conversion, returning a ConversionResult."),
        "wrapper @doc must join wrapped summary lines; content:\n{content}"
    );
    assert!(
        !content.contains("Convert markup conversion, returning\n"),
        "wrapper @doc must not retain the physical newline mid-paragraph; content:\n{content}"
    );
}

fn make_error_with_methods() -> ErrorDef {
    ErrorDef {
        name: "DemoError".into(),
        rust_path: "demo::DemoError".into(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "NotFound".into(),
            message_template: Some("not found".into()),
            fields: vec![],
            has_source: false,
            has_from: false,
            is_unit: true,
            is_tuple: false,
            doc: String::new(),
        }],
        doc: String::new(),
        methods: vec![
            MethodDef {
                name: "status_code".into(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::U16),
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
                name: "is_transient".into(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::Bool),
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
                name: "error_type".into(),
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn error_methods_emit_nif_shims_in_lib_rs() {
    let backend = RustlerBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![make_error_with_methods()],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config("demo");
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("lib.rs"))
        .expect("lib.rs must be generated");
    let content = &lib_rs.content;
    assert!(
        content.contains("fn demoerror_status_code(_msg: String) -> u16"),
        "status_code NIF shim missing:\n{content}"
    );
    assert!(
        content.contains("fn demoerror_is_transient(_msg: String) -> bool"),
        "is_transient NIF shim missing:\n{content}"
    );
    assert!(
        content.contains("fn demoerror_error_type(_msg: String) -> String"),
        "error_type NIF shim missing:\n{content}"
    );
}

#[test]
fn error_methods_emit_elixir_spec_and_def_wrappers() {
    let backend = RustlerBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![make_error_with_methods()],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config("demo");
    let files = backend.generate_public_api(&api, &config).unwrap();
    let ex_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("demo.ex"))
        .expect("demo.ex must be generated");
    let content = &ex_file.content;
    assert!(
        content.contains("@spec demoerror_status_code(String.t()) :: non_neg_integer()"),
        "status_code @spec missing:\n{content}"
    );
    assert!(
        content.contains("@spec demoerror_is_transient(String.t()) :: boolean()"),
        "is_transient @spec missing:\n{content}"
    );
    assert!(
        content.contains("@spec demoerror_error_type(String.t()) :: String.t()"),
        "error_type @spec missing:\n{content}"
    );
    assert!(
        content.contains("def demoerror_status_code(msg)"),
        "status_code def missing:\n{content}"
    );
    assert!(
        content.contains("def demoerror_is_transient(msg)"),
        "is_transient def missing:\n{content}"
    );
    assert!(
        content.contains("def demoerror_error_type(msg)"),
        "error_type def missing:\n{content}"
    );
}

/// Regression test: every Rust NIF emitted for error introspection methods
/// (`<errname>_status_code`, `<errname>_is_transient`, `<errname>_error_type`) must
/// have a matching `:erlang.nif_error(:nif_not_loaded)` stub in the `<App>.Native`
/// Elixir module. Without these stubs, rustler-precompiled's `on_load` fails with
/// `{:error, {:bad_lib, ~c"Function not found ..."}}` and the BEAM aborts module load.
#[test]
fn error_methods_emit_matching_native_ex_stubs() {
    let backend = RustlerBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![make_error_with_methods()],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config("demo");

    // Rust NIF emission side
    let bindings = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = bindings
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("lib.rs"))
        .expect("lib.rs must be generated");
    let rust_content = &lib_rs.content;
    assert!(
        rust_content.contains("fn demoerror_status_code(_msg: String) -> u16"),
        "Rust NIF status_code shim missing — test premise broken:\n{rust_content}"
    );

    // Matching Elixir Native stub side — the actual fix.
    let public = backend.generate_public_api(&api, &config).unwrap();
    let native_ex = public
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("native.ex"))
        .expect("native.ex must be generated");
    let native_content = &native_ex.content;
    assert!(
        native_content.contains("def demoerror_status_code(_msg), do: :erlang.nif_error(:nif_not_loaded)"),
        "native.ex stub demoerror_status_code missing:\n{native_content}"
    );
    assert!(
        native_content.contains("def demoerror_is_transient(_msg), do: :erlang.nif_error(:nif_not_loaded)"),
        "native.ex stub demoerror_is_transient missing:\n{native_content}"
    );
    assert!(
        native_content.contains("def demoerror_error_type(_msg), do: :erlang.nif_error(:nif_not_loaded)"),
        "native.ex stub demoerror_error_type missing:\n{native_content}"
    );
}

/// Regression test: opaque types with static constructor methods (like `new`) that return
/// `Self` must wrap the NIF return value in the struct so instance methods receive
/// `%SampleModule{ref: ...}` instead of a raw reference.
///
/// Issue: #119 — Elixir e2e RouteBuilder.new(method, path) returned a raw NIF Reference,
/// not wrapped in %RouteBuilder{ref: ref}. Subsequent calls like request_schema_json(obj, ...)
/// failed with BadMapError when trying to extract obj.ref.
#[test]
fn opaque_static_constructor_wraps_return_in_struct() {
    let backend = RustlerBackend;

    // Create an opaque type with a static `new` constructor that returns Self.
    let opaque_type = TypeDef {
        name: "RouteBuilder".to_string(),
        rust_path: "sample::RouteBuilder".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            // Static constructor that returns Self
            make_static_method("new", TypeRef::Named("RouteBuilder".to_string())),
            // Instance method that uses the wrapped struct
            MethodDef {
                name: "handler_name".into(),
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
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Named("RouteBuilder".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Set handler name".into(),
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
        ],
        is_opaque: true,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Builder for routes".into(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![opaque_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config("demo");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let ex_file = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("route_builder.ex")
        })
        .expect("route_builder.ex must be generated");
    let content = &ex_file.content;

    // Static `new` must return a wrapped struct, not raw reference
    assert!(
        content.contains("def new do") || content.contains("def new("),
        "new method not found in:\n{content}"
    );

    // The key assertion: `new` wraps the NIF result in the struct
    assert!(
        content.contains("ref = Native.routebuilder_new(") && content.contains("%__MODULE__{ref: ref}"),
        "static constructor `new` must wrap NIF result in struct; got:\n{content}"
    );

    // Instance method `handler_name` unpacks obj.ref for the NIF call
    assert!(
        content.contains("def handler_name(obj, name) do")
            && content.contains("Native.routebuilder_handler_name(obj.ref, name)"),
        "instance method handler_name must unpack obj.ref; got:\n{content}"
    );
}
