//! Verifies that the Rustler backend emits `@moduledoc`, `@typedoc`, and `@doc` heredocs on
//! Elixir DTO modules, enum modules, and per-variant accessors so ExDoc shows complete
//! coverage for the binding.

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

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

fn make_field(name: &str, ty: TypeRef, optional: bool, doc: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: doc.to_string(),
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

fn find_module<'a>(generated: &'a [alef::core::backend::GeneratedFile], needle: &str) -> &'a str {
    let module = generated
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.contains(needle) && p.ends_with(".ex")
        })
        .unwrap_or_else(|| {
            panic!(
                "should generate module containing {needle}; got: {:?}",
                generated
                    .iter()
                    .map(|f| f.path.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            )
        });
    &module.content
}

#[test]
fn test_struct_module_emits_moduledoc_heredoc_when_doc_present() {
    let struct_def = TypeDef {
        name: "ProcessConfig".to_string(),
        rust_path: "my_crate::ProcessConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("name", TypeRef::String, false, "")],
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
        doc: "Configuration for a processing pipeline.\n\nDescribes the input and output channels.".to_string(),
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
        types: vec![struct_def],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let generated = RustlerBackend.generate_public_api(&api, &config).unwrap();
    let content = find_module(&generated, "process_config");

    assert!(
        content.contains("@moduledoc \"\"\""),
        "multi-line module doc must use heredoc form; got:\n{content}"
    );
    assert!(
        content.contains("Configuration for a processing pipeline."),
        "module doc body must be preserved; got:\n{content}"
    );
    assert!(
        content.contains("Describes the input and output channels."),
        "second paragraph must be preserved; got:\n{content}"
    );
    assert!(
        content.contains("@typedoc"),
        "@typedoc must be emitted; got:\n{content}"
    );
    // @typedoc must come before @type t to be picked up by ExDoc.
    let typedoc_pos = content.find("@typedoc").expect("@typedoc must be present");
    let type_t_pos = content.find("@type t ::").expect("@type t must be present");
    assert!(
        typedoc_pos < type_t_pos,
        "@typedoc must precede @type t; got:\n{content}"
    );
}

#[test]
fn test_struct_module_emits_moduledoc_false_when_doc_empty() {
    let struct_def = TypeDef {
        name: "Anon".to_string(),
        rust_path: "my_crate::Anon".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("v", TypeRef::Primitive(PrimitiveType::U32), false, "")],
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
        types: vec![struct_def],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let generated = RustlerBackend.generate_public_api(&api, &config).unwrap();
    let content = find_module(&generated, "anon");
    assert!(
        content.contains("@moduledoc false"),
        "docless module must emit @moduledoc false; got:\n{content}"
    );
    assert!(
        !content.contains("@typedoc"),
        "docless module must not emit @typedoc; got:\n{content}"
    );
}

#[test]
fn test_unit_enum_module_emits_doc_on_each_variant_accessor() {
    let enum_def = EnumDef {
        name: "Severity".to_string(),
        rust_path: "my_crate::Severity".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Error".into(),
                fields: vec![],
                doc: "A blocking issue.".into(),
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
                name: "Warning".into(),
                fields: vec![],
                doc: "A non-blocking caveat.".into(),
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
        doc: "Severity levels for diagnostics.".into(),
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
    };
    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![],
        enums: vec![enum_def],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let generated = RustlerBackend.generate_public_api(&api, &config).unwrap();
    let content = find_module(&generated, "severity");

    assert!(
        content.contains("@moduledoc"),
        "enum module must emit @moduledoc; got:\n{content}"
    );
    assert!(
        content.contains("Severity levels for diagnostics."),
        "enum @moduledoc body must be preserved; got:\n{content}"
    );
    assert!(
        content.contains("@typedoc"),
        "unit enum should emit @typedoc on @type t; got:\n{content}"
    );
    assert!(
        content.contains("A blocking issue."),
        "Error variant doc must appear in @doc above accessor; got:\n{content}"
    );
    assert!(
        content.contains("A non-blocking caveat."),
        "Warning variant doc must appear in @doc above accessor; got:\n{content}"
    );
    // Verify @doc precedes the corresponding @spec/def pair.
    let doc_pos = content.find("A blocking issue.").unwrap();
    let def_error_pos = content
        .find("def error,")
        .unwrap_or_else(|| content.find("def error ").unwrap_or(usize::MAX));
    assert!(
        doc_pos < def_error_pos,
        "@doc must precede def for variant accessor; got:\n{content}"
    );
}

#[test]
fn test_data_enum_module_emits_typedoc_on_each_variant_alias() {
    let enum_def = EnumDef {
        name: "Diagnostic".to_string(),
        rust_path: "my_crate::Diagnostic".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Error".into(),
                fields: vec![FieldDef {
                    name: "msg".into(),
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
                doc: "Hard failure diagnostic.".into(),
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
                name: "Warning".into(),
                fields: vec![FieldDef {
                    name: "msg".into(),
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
                doc: "Soft warning diagnostic.".into(),
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
        doc: "All diagnostic variants emitted by the analyzer.".into(),
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
    };
    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![],
        enums: vec![enum_def],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let generated = RustlerBackend.generate_public_api(&api, &config).unwrap();
    let content = find_module(&generated, "diagnostic");

    assert!(
        content.contains("All diagnostic variants emitted by the analyzer."),
        "data-enum @moduledoc body must be preserved; got:\n{content}"
    );
    assert!(
        content.contains("Hard failure diagnostic."),
        "Error variant doc must appear in @typedoc above type alias; got:\n{content}"
    );
    assert!(
        content.contains("Soft warning diagnostic."),
        "Warning variant doc must appear in @typedoc above type alias; got:\n{content}"
    );
    // The @typedoc for a variant must precede the corresponding @type alias line.
    let typedoc_idx = content.find("Hard failure diagnostic.").unwrap();
    let alias_idx = content
        .find("@type error ::")
        .or_else(|| content.find("@type :\"Error\" ::"))
        .expect("variant type alias must be present");
    assert!(
        typedoc_idx < alias_idx,
        "variant @typedoc must precede its @type alias; got:\n{content}"
    );
}
