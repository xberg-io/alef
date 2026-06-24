use super::*;

fn trait_bridge_config_for_tests() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "test_lib::Plugin"
registry_getter = "test_lib::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"
"#,
    )
}

#[test]
fn extendr_module_registers_trait_bridge_register_unregister_clear() {
    // Regression: register_<trait> / unregister_<trait> / clear_<trait> are emitted
    // as `#[extendr]` functions by the trait-bridge generator but were missing from
    // the `extendr_module!` block, so the wrap__<symbol> entry points never reached
    // the .so and R callers could not invoke them.
    let backend = ExtendrBackend;
    let config = trait_bridge_config_for_tests();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated");
    for sym in ["register_ocr_backend", "unregister_ocr_backend", "clear_ocr_backends"] {
        assert!(
            lib_rs.content.contains(&format!("fn {sym};")),
            "extendr_module! must register `{sym}`:\n{}",
            lib_rs.content
        );
    }
}

#[test]
fn extendr_wrappers_emits_trait_bridge_register_unregister_clear() {
    // Regression: extendr-wrappers.R only iterated `api.functions` and so omitted
    // the trait-bridge register/unregister/clear functions. R callers had no way to
    // invoke `wrap__register_text_backend` because no R wrapper existed.
    let backend = ExtendrBackend;
    let config = trait_bridge_config_for_tests();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        content.contains("register_ocr_backend <- function(r_backend) .Call(\"wrap__register_ocr_backend\""),
        "register wrapper must accept an R object and call wrap__register_ocr_backend:\n{content}"
    );
    assert!(
        content.contains("unregister_ocr_backend <- function(name) .Call(\"wrap__unregister_ocr_backend\""),
        "unregister wrapper must accept a name and call wrap__unregister_ocr_backend:\n{content}"
    );
    assert!(
        content.contains("clear_ocr_backends <- function() .Call(\"wrap__clear_ocr_backends\""),
        "clear wrapper must take no arguments:\n{content}"
    );
}

#[test]
fn namespace_exports_trait_bridge_register_unregister_clear() {
    // Regression: without explicit `export()` entries in NAMESPACE, the
    // trait-bridge wrappers would be loaded internally but unreachable via
    // `pkg::register_<trait>(...)`.
    let backend = ExtendrBackend;
    let config = trait_bridge_config_for_tests();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let namespace = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
        .expect("NAMESPACE must be generated");
    for sym in ["register_ocr_backend", "unregister_ocr_backend", "clear_ocr_backends"] {
        assert!(
            namespace.content.contains(&format!("export({sym})")),
            "NAMESPACE must export `{sym}`:\n{}",
            namespace.content
        );
    }
}

#[test]
fn extendr_excludes_trait_bridge_functions_when_language_excluded() {
    // The bridge structs already honour `exclude_languages`. Their register / unregister /
    // clear free functions must follow the same gate so the module/wrappers/namespace stay in sync.
    let config = resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "test_lib::Plugin"
registry_getter = "test_lib::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"
exclude_languages = ["r"]
"#,
    );
    let collected = trait_bridge_wrappers::collect_trait_bridge_functions(&config);
    assert!(
        collected.is_empty(),
        "no trait-bridge entries should be collected when r is excluded: {:?}",
        collected.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
}

#[test]
fn regression_namespace_exports_functions_types_enums() {
    // Regression test: Verify that NAMESPACE exports ALL functions, types, and enums.
    // A bug caused NAMESPACE to only contain `useDynLib(...)` with no exports.
    let backend = ExtendrBackend;
    let config = make_config();
    let mut api = make_api_surface();
    // Add extra exported types and enums to exercise namespace completeness.
    api.types.push(TypeDef {
        name: "DocumentMetadata".to_string(),
        rust_path: "test_lib::DocumentMetadata".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("title", TypeRef::String, true)],
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
    });
    // Add a flat data enum (has variant with data, single field)
    api.enums.push(EnumDef {
        name: "ConversionResult".to_string(),
        rust_path: "test_lib::ConversionResult".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Ok".to_string(),
                fields: vec![make_field("content", TypeRef::String, false)],
                is_default: false,
                serde_rename: None,
                is_tuple: true,
                doc: String::new(),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Err".to_string(),
                fields: vec![make_field("msg", TypeRef::String, false)],
                is_default: false,
                serde_rename: None,
                is_tuple: true,
                doc: String::new(),
                binding_excluded: false,
                binding_exclusion_reason: None,
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
    });
    let files = backend.generate_public_api(&api, &config).unwrap();
    let namespace = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
        .expect("NAMESPACE must be generated");
    let content = &namespace.content;
    // Check for the useDynLib line
    assert!(
        content.contains("useDynLib(testlib, .registration = TRUE)"),
        "NAMESPACE must have useDynLib: {content}"
    );
    // Check for function exports
    assert!(
        content.contains("export(process)"),
        "NAMESPACE must export free functions, got: {content}"
    );
    // Check for type exports
    assert!(
        content.contains("export(Config)"),
        "NAMESPACE must export types like Config: {content}"
    );
    assert!(
        content.contains("export(DocumentMetadata)"),
        "NAMESPACE must export DocumentMetadata: {content}"
    );
    // Check for enum exports (flat data enums)
    assert!(
        content.contains("export(ConversionResult)"),
        "NAMESPACE must export flat data enums: {content}"
    );
    // Make sure NAMESPACE is NOT just 2 lines (the bug symptom)
    let line_count = content.lines().count();
    assert!(
        line_count > 10,
        "NAMESPACE should have many more than 10 lines, got {line_count}: {content}"
    );
}

#[test]
fn register_wrapper_roxygen_documents_typed_host_callback_shape() {
    // The `register_<trait>` R wrapper must surface a typed host-interface contract in roxygen:
    // one documented line per callback method the host backend must implement, naming the struct
    // param type and the return type, and flagging native-object params. This is R's equivalent
    // of the typed plugin Protocol other bindings emit. Neutral fixtures: Greeter / Opts / Doc.
    let backend = ExtendrBackend;
    let config = resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

[[crates.trait_bridges]]
trait_name = "Greeter"
super_trait = "test_lib::Plugin"
registry_getter = "test_lib::get_greeter_registry"
register_fn = "register_greeter"
unregister_fn = "unregister_greeter"
clear_fn = "clear_greeters"
"#,
    );

    let opts = TypeDef {
        name: "Opts".to_string(),
        rust_path: "test_lib::Opts".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("greeting", TypeRef::String, false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let doc_ty = TypeDef {
        name: "Doc".to_string(),
        rust_path: "test_lib::Doc".to_string(),
        has_serde: true,
        ..opts.clone()
    };
    let greeter = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        fields: vec![],
        methods: vec![MethodDef {
            name: "greet".to_string(),
            params: vec![ParamDef {
                name: "opts".to_string(),
                ty: TypeRef::Named("Opts".to_string()),
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Named("Doc".to_string()),
            ..Default::default()
        }],
        has_serde: false,
        ..opts.clone()
    };

    let mut api = make_api_surface();
    api.types.push(opts);
    api.types.push(doc_ty);
    api.types.push(greeter);

    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;

    // The register roxygen names the method, its struct param type, and the return type.
    assert!(
        content.contains("greet(opts: Opts (native object)) -> Doc"),
        "register roxygen must document the typed callback shape with native struct param:\n{content}"
    );
    assert!(
        content.contains("must implement the following methods"),
        "register roxygen must introduce the host-interface contract:\n{content}"
    );
}

#[test]
fn r_field_long_descriptions_are_truncated_to_fit_120_char_lines() {
    // Ensure roxygen2 @field lines don't exceed 120 chars to satisfy lintr.
    // Each @field line has format: "#' @field <name> <description>"
    // which is 10 + len(name) + 1 + len(description) chars.
    // So description must be truncated to fit within 120 total.
    let backend = ExtendrBackend;
    let config = make_config();
    let long_doc = "Open Graph metadata (og:* properties) for social media Keys like \"title\", \"description\", \"image\", \"url\", etc.";
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "DocumentMetadata".to_string(),
            rust_path: "test_lib::DocumentMetadata".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                doc: long_doc.to_string(),
                ..make_field("open_graph", TypeRef::String, true)
            }],
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
            doc: "Document metadata".to_string(),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;

    // Find the @field line and verify it's under 120 chars.
    for line in content.lines() {
        if line.contains("@field open_graph") {
            assert!(
                line.len() <= 120,
                "@field line must be <= 120 chars, got {} chars: {}",
                line.len(),
                line
            );
            // Also verify it's not just truncated to empty — should have real description.
            assert!(
                line.contains("Open Graph metadata"),
                "@field description was over-truncated: {}",
                line
            );
            return;
        }
    }
    panic!("Could not find @field open_graph line in:\n{}", content);
}
