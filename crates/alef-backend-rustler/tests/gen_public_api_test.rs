use alef_backend_rustler::RustlerBackend;
use alef_core::backend::Backend;
use alef_core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
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
        core_wrapper: alef_core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Convert HTML to Markdown".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![EnumDef {
            name: "HeadingStyle".to_string(),
            rust_path: "my_lib::HeadingStyle".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Setext".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: true,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
                    is_tuple: false,
                    doc: String::new(),
                    is_default: true,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    // Opaque types get a dedicated wrapper module that wraps a ResourceArc
    // reference (`defstruct [:ref]`), distinct from the value-struct modules
    // emitted for non-opaque types.
    let engine_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/engine.ex"))
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
                    is_tuple: false,
                    doc: String::new(),
                    is_default: true,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Atx".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Heading style for Markdown output".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
                original_type: None,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![make_field("content", TypeRef::String, false)],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/message.ex"))
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

/// The `force_build:` keyword in the generated `native.ex` must not exceed Elixir's
/// 98-character default formatter line width, otherwise `mix format` rewrites the file.
#[test]
fn test_native_ex_force_build_line_within_98_chars() {
    let backend = RustlerBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let native = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/native.ex"))
        .expect("native.ex should be generated");

    // Only check lines related to force_build — the ~w(...) targets line is a pre-existing
    // separate issue outside this fix's scope.
    let long_force_build_lines: Vec<(usize, &str)> = native
        .content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("force_build") && line.len() > 98)
        .collect();

    assert!(
        long_force_build_lines.is_empty(),
        "native.ex force_build lines exceed 98 chars (mix format limit):\n{}",
        long_force_build_lines
            .iter()
            .map(|(n, l)| format!("  line {}: {} chars: {l}", n + 1, l.len()))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Also assert the force_build keyword is present. The previous codegen used a
    // multi-line form with `force_build:\n`; the current emission keeps it on a
    // single line because the resulting line stays comfortably within 98 chars,
    // which is what `mix format` actually cares about.
    assert!(
        native.content.contains("force_build:"),
        "force_build: keyword should be present in native.ex; content:\n{}",
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
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Line".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/comment_kind.ex"))
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib.ex"))
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my_lib");
    let files = backend.generate_public_api(&api, &config).unwrap();

    let struct_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib/message.ex"))
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
