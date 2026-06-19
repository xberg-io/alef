use super::super::RustlerBackend;
use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::ApiSurface;

fn test_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "my_lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn test_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
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

/// The generated lib.rs must be placed in `{name}_nif/src/lib.rs` by default —
/// matching the scaffold's Cargo.toml at `{name}_nif/Cargo.toml`.
///
/// Previously the backend used `{name}_rustler/src/` which caused a 3-way mismatch:
/// scaffold Cargo.toml in `_nif/`, generated lib.rs in `_rustler/`, native.ex `crate:` = `_nif`.
#[test]
fn test_generate_bindings_output_path_is_nif_not_rustler() {
    let config = test_config();
    let api = test_api();
    let backend = RustlerBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1, "expected exactly one generated file");
    let lib_rs_path = files[0].path.to_string_lossy();
    // With ResolvedCrateConfig the output_paths template resolves to packages/elixir/.
    // The important invariant is that the path never falls back to a _rustler/ directory.
    assert!(
        lib_rs_path.ends_with("lib.rs"),
        "generated file must be a lib.rs; got: {lib_rs_path}"
    );
    assert!(
        !lib_rs_path.contains("_rustler"),
        "generated lib.rs must not be inside a _rustler/ directory; got: {lib_rs_path}"
    );
}

/// The `crate:` field in native.ex must match the `[package] name` in the scaffold's Cargo.toml.
/// Both must be `{app_name}_nif` so rustler_precompiled can locate the shared library.
#[test]
fn test_native_ex_crate_field_matches_nif_crate_name() {
    let config = test_config();
    let api = test_api();
    let backend = RustlerBackend;
    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_ex = files.iter().find(|f| f.path.ends_with("native.ex")).unwrap();
    assert!(
        native_ex.content.contains("crate: \"my_lib_nif\""),
        "native.ex crate: field must match the _nif Cargo.toml package name; content: {}",
        native_ex.content
    );
}

/// When services are present, lib.rs must declare `pub mod service;` so that
/// the `service.rs` module (containing `#[rustler::nif]` functions) is included
/// in the module tree and discovered by the `rustler::init!` macro.
#[test]
fn test_service_module_included_when_services_present() {
    use crate::core::ir::{EntrypointDef, EntrypointKind, MethodDef, ServiceDef, TypeRef};

    let config = test_config();
    let mut api = test_api();

    // Add a minimal service to trigger service.rs generation.
    let service = ServiceDef {
        name: "TestService".to_string(),
        rust_path: "test::TestService".to_string(),
        constructor: MethodDef {
            name: "new".to_string(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create service".to_string(),
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
        },
        configurators: vec![],
        registrations: vec![],
        entrypoints: vec![EntrypointDef {
            method: "run".to_string(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![],
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Run service".to_string(),
        }],
        doc: "Test service".to_string(),
        cfg: None,
    };

    api.services.push(service);

    let backend = RustlerBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    assert!(
        lib_rs.content.contains("pub mod service;"),
        "lib.rs must declare 'pub mod service;' when services are present; content:\n{}",
        lib_rs.content
    );
}

/// Conversely, when no services are present, lib.rs should not declare the service module.
#[test]
fn test_service_module_omitted_when_no_services() {
    let config = test_config();
    let api = test_api();
    let backend = RustlerBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    assert!(
        !lib_rs.content.contains("pub mod service;"),
        "lib.rs must NOT declare 'pub mod service;' when no services are present; content:\n{}",
        lib_rs.content
    );
}

/// Build an `ApiSurface` mirroring the demo_crawler `interact_async(engine, url, Vec<PageAction>)`
/// shape. `PageAction` is a serde-tagged enum with `tag = "type", rename_all = "camelCase"` and
/// has unit, struct, and explicitly-renamed variants. The wrapper must therefore route the
/// `actions` param through a `encode_page_action/1` helper before `Jason.encode!`, and that
/// helper must accept tuple, atom, and map shapes — see the bug recap on the upstream task.
fn tagged_enum_api_surface() -> ApiSurface {
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

    let page_action = EnumDef {
        name: "PageAction".to_string(),
        rust_path: "demo_crawler::PageAction".to_string(),
        original_rust_path: "demo_crawler::PageAction".to_string(),
        variants: vec![
            EnumVariant {
                name: "Click".to_string(),
                fields: vec![FieldDef {
                    name: "selector".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "TypeText".to_string(),
                serde_rename: Some("type".to_string()),
                fields: vec![
                    FieldDef {
                        name: "selector".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                    FieldDef {
                        name: "text".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            EnumVariant {
                name: "Screenshot".to_string(),
                fields: vec![FieldDef {
                    name: "full_page".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    optional: true,
                    serde_rename: Some("fullPage".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Scrape".to_string(),
                fields: vec![],
                is_default: true,
                ..Default::default()
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: Some("camelCase".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Opaque engine type so the wrapper's first param is `reference()` and skipped by
    // the JSON-encode predicate.
    let engine_type = TypeDef {
        name: "CrawlEngine".to_string(),
        rust_path: "demo_crawler::CrawlEngine".to_string(),
        is_opaque: true,
        ..Default::default()
    };

    let interact = FunctionDef {
        name: "interact_async".to_string(),
        rust_path: "demo_crawler::interact_async".to_string(),
        original_rust_path: "demo_crawler::interact_async".to_string(),
        params: vec![
            ParamDef {
                name: "engine".to_string(),
                ty: TypeRef::Named("CrawlEngine".to_string()),
                ..Default::default()
            },
            ParamDef {
                name: "url".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            ParamDef {
                name: "actions".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
                ..Default::default()
            },
        ],
        return_type: TypeRef::Unit,
        is_async: true,
        error_type: Some("CrawlError".to_string()),
        ..Default::default()
    };

    let mut api = test_api();
    api.types.push(engine_type);
    api.enums.push(page_action);
    api.functions.push(interact);
    api
}

/// When a wrapper function takes a `Vec<TaggedEnum>`, the generated Elixir wrapper must
/// route that param through a per-enum `encode_<snake>/1` helper before `Jason.encode!`,
/// and the helper must be emitted exactly once in the module.
#[test]
fn test_tagged_enum_param_invokes_encoder_in_nif_call() {
    let config = test_config();
    let api = tagged_enum_api_surface();
    let backend = RustlerBackend;
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|f| f.path.ends_with("my_lib.ex"))
        .expect("expected my_lib.ex wrapper to be generated");
    let body = &wrapper.content;

    assert!(
        body.contains("Jason.encode!(Enum.map(actions, &encode_page_action/1))"),
        "interact_async must JSON-encode actions through the encode_page_action helper; got:\n{body}"
    );
    assert!(
        !body.contains("Jason.encode!(actions)"),
        "interact_async must NOT call Jason.encode!(actions) directly — Jason cannot encode tuples; got:\n{body}"
    );
}

/// The encoder helper must define dedicated clauses for each variant shape:
///   * unit variant accepts bare atom AND tuple form,
///   * struct variant accepts tuple form and emits the correct discriminator wire name,
///   * explicit `serde(rename = "...")` on a variant overrides `rename_all`,
///   * explicit `serde(rename = "...")` on a field is honored,
///   * a catch-all clause raises ArgumentError for unknown inputs.
#[test]
fn test_tagged_enum_encoder_emits_per_variant_clauses() {
    let config = test_config();
    let api = tagged_enum_api_surface();
    let backend = RustlerBackend;
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrapper = files.iter().find(|f| f.path.ends_with("my_lib.ex")).unwrap();
    let body = &wrapper.content;

    // Unit variant — bare atom form, camelCase wire name from rename_all.
    assert!(
        body.contains("defp encode_page_action(:scrape), do: %{\"type\" => \"scrape\"}"),
        "missing unit variant atom clause; body:\n{body}"
    );
    // Unit variant — tuple form is also accepted.
    assert!(
        body.contains("defp encode_page_action({:scrape, _}), do: %{\"type\" => \"scrape\"}"),
        "missing unit variant tuple clause; body:\n{body}"
    );
    // Struct variant — discriminator value camelCased by rename_all.
    assert!(
        body.contains("defp encode_page_action({:click, %{} = data}) do"),
        "missing click struct-variant clause; body:\n{body}"
    );
    assert!(
        body.contains("|> Map.put(\"type\", \"click\")"),
        "click clause must put discriminator with wire-cased value; body:\n{body}"
    );
    // Explicit `serde(rename = "type")` on TypeText overrides camelCase.
    assert!(
        body.contains("|> Map.put(\"type\", \"type\")"),
        "TypeText variant must use serde(rename = \"type\") as wire name; body:\n{body}"
    );
    // Explicit `serde(rename = "fullPage")` on Screenshot.full_page is honored as a
    // per-variant key-mapping arm so user input `{:screenshot, %{full_page: true}}`
    // round-trips to `%{"type" => "screenshot", "fullPage" => true}`.
    assert!(
        body.contains(":full_page -> \"fullPage\""),
        "Screenshot.full_page must be wire-renamed to fullPage; body:\n{body}"
    );
    // Map passthrough and catch-all error clauses.
    assert!(
        body.contains("defp encode_page_action(%{} = m), do: m"),
        "encoder must passthrough wire-shaped maps; body:\n{body}"
    );
    assert!(
        body.contains("raise(ArgumentError"),
        "encoder must raise ArgumentError for unrecognized inputs; body:\n{body}"
    );

    // Single emission only — the encoder must not be duplicated.
    let occurrences = body.matches("defp encode_page_action(:scrape),").count();
    assert_eq!(
        occurrences, 1,
        "encode_page_action must be emitted exactly once; got {occurrences} occurrences; body:\n{body}"
    );
}

/// Bug 2: Multi-clause defp functions must have blank lines between clauses.
/// When `mix format --check-formatted` runs on generated elixir code, it requires
/// blank lines between consecutive function clauses in multi-clause definitions.
/// This test ensures the encoder emits proper formatting that passes mix format.
#[test]
fn test_tagged_enum_encoder_blank_lines_between_clauses() {
    let config = test_config();
    let api = tagged_enum_api_surface();
    let backend = RustlerBackend;
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrapper = files.iter().find(|f| f.path.ends_with("my_lib.ex")).unwrap();
    let body = &wrapper.content;

    // Extract the defp encode_page_action section from the generated code
    let encoder_start = body.find("defp encode_page_action").expect("encoder must exist");
    let encoder_end = body[encoder_start..].rfind("end\n").expect("encoder must have an end");
    let encoder_section = &body[encoder_start..encoder_start + encoder_end + 4];

    // Count how many distinct defp clauses exist (line starting with "  defp encode_page_action")
    let clause_count = encoder_section.matches("  defp encode_page_action").count();
    assert!(
        clause_count >= 2,
        "test requires at least 2 defp clauses; got {}",
        clause_count
    );

    // Verify that unit variant clauses (:scrape and {:scrape, _}) have a blank line between them.
    // Look for the pattern that indicates proper formatting with blank line.
    let has_unit_spacing = encoder_section.contains(":scrape), do: %{\"type\" => \"scrape\"}\n\n  defp");
    assert!(
        has_unit_spacing,
        "unit variant clauses must have a blank line between them; got:\n{}",
        encoder_section
    );

    // Verify that struct variant clauses have blank lines between them.
    // Look for `end\n\n  defp` pattern which shows blank line before next clause.
    let has_struct_spacing = encoder_section.contains("end\n\n  defp");
    assert!(
        has_struct_spacing,
        "struct variant clauses must have a blank line between them; got:\n{}",
        encoder_section
    );
}

/// Bug 1: NIF [features] section respects nif_features config parameter.
/// When `[crates.elixir] nif_features = []` is set in alef.toml, the generated
/// Cargo.toml must use an empty [features] default instead of forwarding missing core features.
#[test]
fn test_elixir_config_parses_nif_features() {
    // Test 1: Empty nif_features should parse correctly
    let toml_empty = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "my_lib"
nif_features = []
"#;
    let cfg_empty: NewAlefConfig = toml::from_str(toml_empty).expect("config must parse");
    let config_empty = cfg_empty.resolve().expect("config must resolve").remove(0);

    // Verify nif_features was parsed as empty list
    assert!(
        config_empty
            .elixir
            .as_ref()
            .and_then(|e| e.nif_features.as_ref())
            .map(|f| f.is_empty())
            .unwrap_or(false),
        "nif_features = [] should be parsed as empty list; got: {:?}",
        config_empty.elixir.as_ref().and_then(|e| e.nif_features.as_ref())
    );

    // Test 2: Default (no nif_features set) should be None
    let toml_default = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "my_lib"
"#;
    let cfg_default: NewAlefConfig = toml::from_str(toml_default).expect("config must parse");
    let config_default = cfg_default.resolve().expect("config must resolve").remove(0);

    // When nif_features is not set, should be None (uses default behavior)
    assert!(
        config_default
            .elixir
            .as_ref()
            .and_then(|e| e.nif_features.as_ref())
            .is_none(),
        "unset nif_features should be None; got: {:?}",
        config_default.elixir.as_ref().and_then(|e| e.nif_features.as_ref())
    );

    // Test 3: Custom nif_features list
    let toml_custom = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "my_lib"
nif_features = ["foo", "bar"]
"#;
    let cfg_custom: NewAlefConfig = toml::from_str(toml_custom).expect("config must parse");
    let config_custom = cfg_custom.resolve().expect("config must resolve").remove(0);

    // Verify custom features were parsed
    let nif_features = config_custom
        .elixir
        .as_ref()
        .and_then(|e| e.nif_features.as_ref())
        .expect("should parse custom features");
    assert_eq!(nif_features.len(), 2, "should have 2 custom features");
    assert!(nif_features.contains(&"foo".to_string()), "should contain foo");
    assert!(nif_features.contains(&"bar".to_string()), "should contain bar");
}
