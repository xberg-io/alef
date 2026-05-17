use alef_backend_rustler::RustlerBackend;
use alef_core::backend::Backend;
use alef_core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef_core::ir::{ApiSurface, CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};

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
    // Create a simple struct with various field types
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
    };

    let backend = RustlerBackend;
    let generated = backend
        .generate_public_api(&api, &config)
        .expect("generation must succeed");

    // Find the generated struct module file
    let struct_module = generated
        .iter()
        .find(|f| {
            let path_str = f.path.to_string_lossy();
            path_str.contains("code_chunk") && path_str.ends_with(".ex")
        })
        .expect("should generate CodeChunk module");

    let content = &struct_module.content;

    // Verify that @type t typespec is present
    assert!(
        content.contains("@type t ::"),
        "struct module must emit @type t typespec; got:\n{content}"
    );

    // Verify the struct is still there
    assert!(
        content.contains("defstruct"),
        "struct module must still have defstruct; got:\n{content}"
    );

    // Verify @type t comes before defstruct
    let type_pos = content.find("@type t ::").expect("@type t must be present");
    let defstruct_pos = content.find("defstruct").expect("defstruct must be present");
    assert!(type_pos < defstruct_pos, "@type t must appear before defstruct");

    // Verify field types are correctly mapped
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

    // Verify proper struct form with %__MODULE__{}
    assert!(
        content.contains("%__MODULE__{"),
        "typespec must use %__MODULE__{{ ... }}; got:\n{content}"
    );
}

#[test]
fn test_struct_module_defstruct_defaults_align_with_typespec() {
    // G7: Test that defstruct field defaults align with @type specs.
    // Nilable fields (X | nil) must default to nil, regardless of Rust default.
    // Non-nilable strings default to "", numbers to 0, booleans to false, lists to [].
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

    // Verify nil defaults for nullable fields
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

    // Verify proper defaults for required fields
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
    // Create a struct that references another struct type
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

    // Named types should map to map() in the typespec
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
fn test_struct_module_with_vec_fields() {
    // Create a struct with vector fields
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

    // Vec types should map to lists in the typespec
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
