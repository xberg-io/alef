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
        ..Default::default()
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
