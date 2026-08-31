use super::super::shared_pages::render_enum_for_shared_doc;
use super::*;
use crate::core::ir::{CoreWrapper, FieldDef};

/// `--lang c` / `--lang jni` CLI-resolver control tests -- split into a sibling file to keep
/// this one under the file-modularization line cap; see that module's own doc for why. ~keep
mod lang_cli_control;

/// A minimal `types.md`-triggering config type: any type whose name ends in `Config` lands
/// in the "Configuration Types" category, which is what emits the cross-page link to
/// `configuration.md` under test here.
fn make_config_type() -> TypeDef {
    TypeDef {
        name: "FooConfig".into(),
        rust_path: "mylib::FooConfig".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// The `ApiSurface` `test_types_doc_configuration_link_honors_reference_link_style` renders
/// under two different `[docs].reference_link_style` configs -- verbatim relocation of that
/// test's own setup, extracted only to stay under the per-function line cap. ~keep
fn api_for_configuration_link_test() -> ApiSurface {
    ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        types: vec![make_config_type()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// task: `types.md`'s link to `configuration.md` was hardcoded to a `.md`-suffixed relative
/// link, which a content-collection docs site (Astro Starlight) cannot resolve -- it needs
/// `./configuration/`. The default (no `[docs].reference_link_style` set) must keep emitting
/// the `.md`-suffixed form so plain Markdown viewers and unconfigured consumers see no
/// change; a consumer opting into `reference_link_style = "extensionless"` must get the
/// directory-style route instead, and only that. ~keep
#[test]
fn test_types_doc_configuration_link_honors_reference_link_style() {
    let api = api_for_configuration_link_test();

    let default_config = make_test_config();
    let default_files = generate_docs(&api, &default_config, &[Language::Python], "out").unwrap();
    let default_types_file = shared_page_content(&default_files, "types");
    assert!(
        default_types_file.contains("[Configuration Reference](configuration.md)"),
        "default (unconfigured) link style must stay `.md`-suffixed for backward compatibility: {}",
        default_types_file
    );

    let extensionless_raw: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.docs]
reference_link_style = "extensionless"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid toml");
    let extensionless_config = extensionless_raw.resolve().expect("resolve ok").remove(0);
    let extensionless_files = generate_docs(&api, &extensionless_config, &[Language::Python], "out").unwrap();
    let extensionless_types_file = shared_page_content(&extensionless_files, "types");
    assert!(
        extensionless_types_file.contains("[Configuration Reference](./configuration/)"),
        "`reference_link_style = \"extensionless\"` must emit a directory-style route, not a \
         `.md`-suffixed link a Starlight-style site cannot resolve: {}",
        extensionless_types_file
    );
    assert!(
        !extensionless_types_file.contains("configuration.md"),
        "the `.md`-suffixed form must not leak through once extensionless is configured: {}",
        extensionless_types_file
    );
}

/// The `TableModel` enum `test_generate_types_doc_renders_enum_variants` renders -- verbatim
/// relocation of that test's own setup, extracted only to stay under the per-function line cap.
/// ~keep
fn table_model_enum() -> EnumDef {
    use crate::core::ir::EnumVariant;
    EnumDef {
        name: "TableModel".into(),
        rust_path: "test::TableModel".into(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Tatr".into(),
                fields: vec![],
                doc: "TATR transformer (default).".into(),
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
                name: "SlanetWired".into(),
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
        doc: "Table structure model.".into(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

/// The `ApiSurface` `test_generate_types_doc_renders_enum_variants` renders -- verbatim
/// relocation of that test's own setup, extracted only to stay under the per-function line cap.
/// ~keep
fn api_with_table_model_enum() -> ApiSurface {
    ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![table_model_enum()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_generate_types_doc_renders_enum_variants() {
    let api = api_with_table_model_enum();
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let types_file = shared_page_content(&files, "types");
    assert!(types_file.contains("### Enums"));
    assert!(types_file.contains("#### TableModel"));
    assert!(types_file.contains("Table structure model."));
    assert!(types_file.contains("`Tatr`"));
    assert!(types_file.contains("TATR transformer"));
    assert!(types_file.contains("`SlanetWired`"));
}

/// The `HtmlTheme` enum `test_render_enum_for_shared_doc_emits_wire_value_column_when_rename_all_set`
/// renders -- verbatim relocation of that test's own setup, extracted only to stay under the
/// per-function line cap. ~keep
fn html_theme_enum_with_rename_all() -> EnumDef {
    use crate::core::ir::EnumVariant;
    EnumDef {
        name: "HtmlTheme".into(),
        rust_path: "test::HtmlTheme".into(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Default".into(),
                fields: vec![],
                doc: "Default theme.".into(),
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
                name: "Github".into(),
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
        doc: "HTML theme.".into(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: Some("lowercase".into()),
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn test_render_enum_for_shared_doc_emits_wire_value_column_when_rename_all_set() {
    let en = html_theme_enum_with_rename_all();
    let out = render_enum_for_shared_doc(&en, Language::Rust);
    assert!(out.contains("| Variant | Wire value | Description |"));
    assert!(out.contains("| `Default` | `default` |"));
    assert!(out.contains("| `Github` | `github` |"));
}

#[test]
fn test_render_enum_for_shared_doc_demotes_internal_headings() {
    use crate::core::ir::EnumVariant;
    let en = EnumDef {
        name: "OutputFormat".into(),
        rust_path: "test::OutputFormat".into(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Markdown".into(),
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
        }],
        methods: vec![],
        doc: "Output format specification.\n\n## Variants\n\nDetailed variant info.".into(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let out = render_enum_for_shared_doc(&en, Language::Rust);
    assert!(
        out.contains("#### Variants"),
        "internal heading must be demoted to #### (was ##): {out}"
    );
    assert!(
        !out.lines().any(|l| l == "## Variants"),
        "raw ## heading must not remain: {out}"
    );
    assert!(out.contains("Output format specification."));
}

/// The `ImageConfig.format` field `test_generate_configuration_doc_renders_referenced_enums_only`
/// renders -- verbatim relocation of that test's own setup, extracted only to stay under the
/// per-function line cap. ~keep
fn image_config_format_field() -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: "format".into(),
        ty: TypeRef::Named("mylib::ImageFormat".into()),
        optional: false,
        default: None,
        doc: "Output image format.".into(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// The `ImageConfig` type `test_generate_configuration_doc_renders_referenced_enums_only`
/// renders -- verbatim relocation of that test's own setup, extracted only to stay under the
/// per-function line cap. ~keep
fn image_config_type() -> TypeDef {
    TypeDef {
        name: "ImageConfig".into(),
        rust_path: "mylib::ImageConfig".into(),
        original_rust_path: String::new(),
        fields: vec![image_config_format_field()],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "Image config.".into(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// The `ImageFormat` enum `test_generate_configuration_doc_renders_referenced_enums_only`
/// renders -- verbatim relocation of that test's own setup, extracted only to stay under the
/// per-function line cap. ~keep
fn image_format_enum() -> EnumDef {
    use crate::core::ir::EnumVariant;
    EnumDef {
        name: "ImageFormat".into(),
        rust_path: "mylib::ImageFormat".into(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Png".into(),
            fields: vec![],
            doc: "PNG output.".into(),
            is_default: true,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: "Image format enum backed by `tl::parse`.".into(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// The `Unrelated` enum `test_generate_configuration_doc_renders_referenced_enums_only` renders
/// -- verbatim relocation of that test's own setup, extracted only to stay under the
/// per-function line cap. ~keep
fn unrelated_enum() -> EnumDef {
    use crate::core::ir::EnumVariant;
    EnumDef {
        name: "Unrelated".into(),
        rust_path: "mylib::Unrelated".into(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "A".into(),
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
        }],
        methods: vec![],
        doc: "Not referenced by any config type.".into(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// The `ApiSurface` `test_generate_configuration_doc_renders_referenced_enums_only` renders --
/// verbatim relocation of that test's own setup, extracted only to stay under the per-function
/// line cap. ~keep
fn api_with_image_config_and_enums() -> ApiSurface {
    ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        types: vec![image_config_type()],
        functions: vec![],
        enums: vec![image_format_enum(), unrelated_enum()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_generate_configuration_doc_renders_referenced_enums_only() {
    let api = api_with_image_config_and_enums();
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let cfg_file = shared_page_content(&files, "configuration");
    assert!(cfg_file.contains("### Enums"));
    assert!(cfg_file.contains("#### ImageFormat"));
    assert!(cfg_file.contains("`tl.parse`"));
    assert!(!cfg_file.contains("`tl::parse`"));
    assert!(
        !cfg_file.contains("#### Unrelated"),
        "configuration.md must filter out enums not referenced by any config-type field"
    );
}

/// One `PipelineMode` variant used by the cfg-union regression tests below: `cfg_feature` is
/// `None` for an ungated variant, or `Some("name")` for one gated on `feature = "name"`. Every
/// test in this group asserts on the RENDERED page content produced from the resulting
/// `ApiSurface`, never on this struct directly, so factoring construction out cannot change what
/// any test proves. ~keep
struct PipelineVariantSpec {
    name: &'static str,
    doc: &'static str,
    cfg_feature: Option<&'static str>,
    is_default: bool,
}

/// Build a named enum from a compact variant list, so a test states only the shape it actually
/// varies (which variants exist, which are cfg-gated, which name) instead of repeating the full
/// `EnumDef`/`EnumVariant` struct-literal boilerplate. [`pipeline_mode_enum`] specializes this to
/// the fixed `"PipelineMode"` name the cfg-union regression tests share. ~keep
fn named_variant_enum(name: &str, variants: &[PipelineVariantSpec]) -> EnumDef {
    use crate::core::ir::EnumVariant;
    EnumDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        doc: "Pipeline execution mode.".into(),
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                name: v.name.into(),
                doc: v.doc.into(),
                is_default: v.is_default,
                cfg: v.cfg_feature.map(|f| format!("feature = \"{f}\"")),
                ..EnumVariant::default()
            })
            .collect(),
        ..EnumDef::default()
    }
}

/// The `"PipelineMode"`-named specialization of [`named_variant_enum`] the cfg-union regression
/// tests below share. ~keep
fn pipeline_mode_enum(variants: &[PipelineVariantSpec]) -> EnumDef {
    named_variant_enum("PipelineMode", variants)
}

/// Wrap a single enum in an otherwise-empty `ApiSurface` -- the shape every regression test in
/// this group needs. ~keep
fn api_with_enum(en: EnumDef) -> ApiSurface {
    ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        enums: vec![en],
        ..ApiSurface::default()
    }
}

/// Wrap a `mode` field of type `PipelineModeConfig`, whose typed default falls back to the
/// enum's `#[default]` variant, inside a minimal `JobConfig`-shaped `ApiSurface` --
/// `test_configuration_doc_default_column_skips_cfg_omitted_variant`'s only setup shape. ~keep
fn api_with_job_config_mode_field(mode_enum: EnumDef) -> ApiSurface {
    use crate::core::ir::DefaultValue;
    let job_config = TypeDef {
        name: "JobConfig".into(),
        rust_path: "mylib::JobConfig".into(),
        doc: "Job configuration.".into(),
        has_default: true,
        fields: vec![FieldDef {
            name: "mode".into(),
            ty: TypeRef::Named("PipelineModeConfig".into()),
            doc: "Execution mode.".into(),
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        types: vec![job_config],
        enums: vec![mode_enum],
        ..ApiSurface::default()
    }
}

/// The rendered content of the shared page whose output path contains `path_contains` (e.g.
/// `"types"`, `"configuration"`) -- the lookup every test in this group repeats. Panics with a
/// clear message if `generate_docs` produced no matching file, which every caller here
/// unconditionally expects. ~keep
fn shared_page_content(files: &[GeneratedFile], path_contains: &str) -> String {
    files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains(path_contains))
        .unwrap_or_else(|| panic!("no generated file with path containing `{path_contains}`"))
        .content
        .clone()
}

/// One case of `test_shared_pages_respect_cfg_gate_for_enum_variants`, built by
/// [`pipeline_mode_cases`] and run by [`assert_pipeline_mode_case`].
struct PipelineModeCase {
    name: &'static str,
    toml_extra: &'static str,
    languages: &'static [Language],
    turbo_present: bool,
}

/// The three cases `test_shared_pages_respect_cfg_gate_for_enum_variants` runs. Every case
/// configures BOTH `python` and `wasm` (`[workspace] languages = ["python", "wasm"]`) --
/// `toml_extra` only decides which of the two enables `acceleration`, and `languages` is the
/// RENDERED subset a case passes to `generate_docs`, which need not equal the configured set. ~keep
fn pipeline_mode_cases() -> [PipelineModeCase; 3] {
    [
        PipelineModeCase {
            name: "acceleration disabled for both configured languages (python, wasm)",
            toml_extra: "",
            languages: &[Language::Python],
            turbo_present: false,
        },
        PipelineModeCase {
            // `turbo_present: true` here is a "union does not wrongly narrow" guard, not
            // evidence of the fix under review: both configured languages enable `acceleration`
            // via this workspace-level default, so a naive first-language-only union would
            // compute the same answer. `NeverShipped`'s absence (asserted by
            // `assert_pipeline_mode_case` for every case) is what this case actually proves. ~keep
            name: "acceleration enabled for both configured languages via a workspace default",
            toml_extra: "features = [\"acceleration\"]",
            languages: &[Language::Python],
            turbo_present: true,
        },
        PipelineModeCase {
            // Same caveat: both configured languages are also rendered here, so a
            // rendered-set-only union (the exact shape the follow-up review rejected) computes
            // the same `Turbo` answer as a configured-set union. See the two dedicated tests
            // below (`..._unrendered_configured_language` and `..._for_the_lang_c_cli_path`) for
            // cases that actually require reading `config.languages`. ~keep
            name: "acceleration enabled only for wasm, both configured languages rendered",
            toml_extra: "\n[crates.wasm]\nfeatures = [\"acceleration\"]",
            languages: &[Language::Python, Language::Wasm],
            turbo_present: true,
        },
    ]
}

/// Run one [`PipelineModeCase`]: build its config from `toml_extra`, render `languages`, and
/// assert against the resulting `types.md` content. `NeverShipped`'s absence is asserted
/// UNCONDITIONALLY, every case, regardless of `turbo_present` -- that is the assertion that
/// actually distinguishes working filtering from none: a `turbo_present: true` case's `Turbo`
/// check alone also passes with the shared-page filter deleted entirely (piping the raw,
/// unfiltered `ApiSurface` straight into the shared pages), since the raw surface contains
/// `Turbo` regardless. `NeverShipped` cannot pass that way -- it leaks through on every case
/// unless real filtering runs. These are the exact assertions each case ran inline before this
/// helper was extracted; extracting it changed nothing about which case proves what. ~keep
fn assert_pipeline_mode_case(api: &ApiSurface, case: &PipelineModeCase) {
    let toml_str = format!(
        "[workspace]\nlanguages = [\"python\", \"wasm\"]\n\n[[crates]]\nname = \"mylib\"\n\
         sources = [\"src/lib.rs\"]\n{}\n",
        case.toml_extra
    );
    let config = config_from_toml(&toml_str);
    let files = generate_docs(api, &config, case.languages, "out").unwrap();
    let types_file = shared_page_content(&files, "types");

    assert!(
        types_file.contains("`Standard`") && types_file.contains("Default, single-threaded mode."),
        "case {}: ungated variant must keep full docs in types.md; got:\n{}",
        case.name,
        types_file
    );

    assert!(
        !types_file.contains("NeverShipped"),
        "case {}: a feature no configured language ever enables must never appear on the \
         shared types.md page -- a filter that is missing entirely (raw ApiSurface piped \
         straight through) would let this leak through regardless of case; got:\n{}",
        case.name,
        types_file
    );

    if case.turbo_present {
        assert!(
            types_file.contains("`Turbo`") && types_file.contains("Multi-threaded accelerated mode."),
            "case {}: cfg-gated variant enabled by a configured language must stay fully \
             documented in types.md; got:\n{}",
            case.name,
            types_file
        );
    } else {
        assert!(
            !types_file.contains("Turbo"),
            "case {}: cfg-gated variant enabled by no configured language must not appear \
             anywhere in types.md; got:\n{}",
            case.name,
            types_file
        );
    }
}

/// The shared, language-neutral pages (`types.md`, `configuration.md`) must never advertise an
/// enum variant that no configured binding actually compiles, and must never drop a variant that
/// at least one configured binding does compile. See [`pipeline_mode_cases`] for the cases and
/// [`assert_pipeline_mode_case`] for what each one asserts and why. ~keep
#[test]
fn test_shared_pages_respect_cfg_gate_for_enum_variants() {
    let api = api_with_enum(pipeline_mode_enum(&[
        PipelineVariantSpec {
            name: "Standard",
            doc: "Default, single-threaded mode.",
            cfg_feature: None,
            is_default: true,
        },
        PipelineVariantSpec {
            name: "Turbo",
            doc: "Multi-threaded accelerated mode.",
            cfg_feature: Some("acceleration"),
            is_default: false,
        },
        PipelineVariantSpec {
            name: "NeverShipped",
            doc: "Not enabled by any case in this table.",
            cfg_feature: Some("never-enabled"),
            is_default: false,
        },
    ]));

    for case in pipeline_mode_cases() {
        assert_pipeline_mode_case(&api, &case);
    }
}

/// Non-vacuous regression for the follow-up fix: the shared-page cfg union must be sourced from
/// `config.languages` (every language the project CONFIGURES), never from the `languages`
/// argument a single `generate_docs` call was asked to RENDER. This config configures both
/// `python` and `wasm`; only `wasm` enables `acceleration`; the call renders ONLY `python`
/// (mirroring `alef docs --lang python`). Before this follow-up, the union was built from the
/// rendered set alone, so it never saw wasm's feature and dropped `Turbo` even though the wasm
/// binding compiles it -- exactly the bug the independent review reported. ~keep
#[test]
fn test_shared_pages_include_variant_enabled_only_by_an_unrendered_configured_language() {
    let api = api_with_enum(pipeline_mode_enum(&[
        PipelineVariantSpec {
            name: "Standard",
            doc: "Default, single-threaded mode.",
            cfg_feature: None,
            is_default: true,
        },
        PipelineVariantSpec {
            name: "Turbo",
            doc: "Multi-threaded accelerated mode.",
            cfg_feature: Some("acceleration"),
            is_default: false,
        },
    ]));

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python", "wasm"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.wasm]
features = ["acceleration"]
"#,
    );

    // Render ONLY python -- mirrors `alef docs --lang python` -- while `wasm` (configured but
    // not rendered this invocation) is the only language that enables `acceleration`.
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let types_file = shared_page_content(&files, "types");

    assert!(
        types_file.contains("`Turbo`") && types_file.contains("Multi-threaded accelerated mode."),
        "a variant enabled only by a configured-but-unrendered language (wasm) must still be \
         fully documented on the shared types.md page; the shared-page filter must read \
         `config.languages`, not the rendered subset. Got:\n{types_file}"
    );
}

/// Non-vacuous regression: `config.languages` -- not whether the rendered set is empty --
/// controls the raw-IR fallback too. This crate is configured with `languages = ["c"]` only, so
/// it has no real doc-owning target language at all (`C`/`Jni` never own a reference page; see
/// `canonical_docs_api`'s own doc), meaning the canonical union must be empty and the shared
/// pages must fall back to the unfiltered surface. Rendering `[Language::Rust]` here is the
/// distinguishing part: a rendered-set-sourced union (the pre-follow-up shape) would compute
/// Rust's OWN `effective_docs_features` (empty, since no `features` are configured anywhere)
/// and wrongly filter `Turbo` out; a config-languages-sourced union sees no real configured
/// language at all and falls back to keeping it. ~keep
#[test]
fn test_shared_pages_fall_back_to_unfiltered_surface_when_configured_languages_are_canonically_empty() {
    let api = api_with_enum(pipeline_mode_enum(&[PipelineVariantSpec {
        name: "Turbo",
        doc: "Multi-threaded accelerated mode.",
        cfg_feature: Some("acceleration"),
        is_default: false,
    }]));

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
languages = ["c"]
"#,
    );

    let files = generate_docs(&api, &config, &[Language::Rust], "out").unwrap();
    let types_file = shared_page_content(&files, "types");

    assert!(
        types_file.contains("`Turbo`"),
        "with no real doc-owning language configured, the shared pages must fall back to the \
         unfiltered surface rather than silently document nothing; got:\n{types_file}"
    );
}

/// A struct field whose typed default falls back to an enum's `is_default` variant (or its
/// first variant) must never resolve to a variant no configured binding compiles -- the
/// "Default" column in `configuration.md` is exactly the field-doc surface the defect this
/// guards against advertised a cfg-omitted variant through.
///
/// Scope note: unlike the `test_shared_pages_*` cases above, this test does NOT distinguish the
/// rendered-vs-configured-languages fix -- `config.languages == rendered == ["python"]` here, so
/// a rendered-set-sourced union computes the identical answer; it only proves the original
/// defect (the shared pages not being cfg-filtered at all) stays fixed. ~keep
#[test]
fn test_configuration_doc_default_column_skips_cfg_omitted_variant() {
    let mode_enum = named_variant_enum(
        "PipelineModeConfig",
        &[
            PipelineVariantSpec {
                name: "Turbo",
                doc: "Multi-threaded accelerated mode (default).",
                cfg_feature: Some("acceleration"),
                is_default: true,
            },
            PipelineVariantSpec {
                name: "Standard",
                doc: "Single-threaded mode.",
                cfg_feature: None,
                is_default: false,
            },
        ],
    );
    let api = api_with_job_config_mode_field(mode_enum);

    // No configured language enables `acceleration`, so the `is_default` variant (`Turbo`) is
    // not reachable by any binding; the default column must fall back to the next real variant
    // (`Standard`) rather than advertise `Turbo`.
    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let cfg_file = shared_page_content(&files, "configuration");

    assert!(
        !cfg_file.contains("Turbo"),
        "cfg-omitted default variant must not leak into configuration.md; got:\n{cfg_file}"
    );
    assert!(
        cfg_file.contains("PipelineModeConfig.STANDARD"),
        "the Default column must fall back to the reachable variant once the cfg-omitted \
         `is_default` variant is filtered out; got:\n{cfg_file}"
    );
}
