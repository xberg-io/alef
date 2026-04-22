use alef_backend_rustler::RustlerBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, ElixirConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

/// Build a minimal AlefConfig for elixir tests.
fn make_config(app_name: &str) -> AlefConfig {
    AlefConfig {
        crate_config: CrateConfig {
            name: app_name.replace('_', "-"),
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
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: Some(ElixirConfig {
            app_name: Some(app_name.to_string()),
            features: None,
            serde_rename_all: None,
            exclude_functions: vec![],
            exclude_types: vec![],
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
        }),
        wasm: None,
        ffi: None,
        go: None,
        java: None,
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        test: None,
        e2e: None,
        trait_bridges: vec![],
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
    }
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
        core_wrapper: alef_core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
        newtype_wrapper: None,
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
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Options for conversion".to_string(),
            cfg: None,
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Convert HTML to Markdown".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
                },
                EnumVariant {
                    name: "Atx".to_string(),
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
        }],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let result = backend.generate_public_api(&api, &config);

    assert!(result.is_ok(), "generate_public_api should succeed: {:?}", result);
    let files = result.unwrap();

    let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();

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
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let native = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/native.ex"))
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
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Options for conversion".to_string(),
            cfg: None,
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
                },
                EnumVariant {
                    name: "Atx".to_string(),
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
        }],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let struct_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/conversion_options.ex"))
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
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib.ex"))
        .expect("my_lib.ex should be generated");

    let content = &main.content;

    // Should define the main module
    assert!(
        content.contains("defmodule MyLib do"),
        "Should define MyLib module; content:\n{content}"
    );

    // Should have wrapper for static method config_default/0
    assert!(
        content.contains("def config_default"),
        "Should have config_default/0 wrapper; content:\n{content}"
    );

    // Wrapper should call Native
    assert!(
        content.contains("MyLib.Native.config_default()"),
        "Should delegate to MyLib.Native.config_default(); content:\n{content}"
    );

    // Should have wrapper for instance method config_validate/1
    assert!(
        content.contains("def config_validate("),
        "Should have config_validate wrapper; content:\n{content}"
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
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    // Opaque types should not get struct modules
    let has_engine_struct = files
        .iter()
        .any(|f| f.path.to_string_lossy().ends_with("my_lib/engine.ex"));
    assert!(
        !has_engine_struct,
        "Opaque types should not get struct modules; got: {:?}",
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
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
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Heading style for Markdown output".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/heading_style.ex"))
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config("my_lib");
    let files = backend.generate_bindings(&api, &config).unwrap();

    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs should be generated");

    // The rustler::init! should use the .Native module name to match native.ex
    assert!(
        lib_rs.content.contains("Elixir.MyLib.Native"),
        "rustler::init! should reference Elixir.MyLib.Native; content:\n{}",
        &lib_rs.content[lib_rs.content.len().saturating_sub(200)..]
    );
}
