//! Regression: a static method returning a borrowed reference to its own opaque type
//! (e.g. `Registry::global() -> &'static Registry`) must be omitted CONSISTENTLY across
//! the FFI backend and every language backend that resolves FFI symbols.
//!
//! The C FFI backend cannot box a borrow into an owned `*mut T` handle, so it never exports
//! a `{prefix}_{type}_{method}` symbol for such accessors. Before this fix the Java, C#, and
//! Zig backends still bound the (non-existent) symbol — Java's eager static MethodHandle
//! resolution threw `ExceptionInInitializerError` at class-init, and C#/Zig failed lazily on
//! first call. This test pins the consistent omission.

use alef::backends::csharp::CsharpBackend;
use alef::backends::ffi::FfiBackend;
use alef::backends::java::JavaBackend;
use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

fn config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi", "java", "csharp", "zig"]

[[crates]]
name = "kreuzberg"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "kreuzberg"

[crates.java]
package = "dev.kreuzberg"

[crates.csharp]
namespace = "Kreuzberg"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Opaque `Registry` with:
///  - `global() -> &'static Registry`  (returns_ref = true)  — must NOT produce an FFI symbol
///  - `load_embedded() -> Registry`      (owned constructor)  — must     produce an FFI symbol
fn registry_api() -> ApiSurface {
    let global = MethodDef {
        name: "global".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Registry".to_string()),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: "Return the global registry.".to_string(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: true,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let load_embedded = MethodDef {
        name: "load_embedded".to_string(),
        return_type: TypeRef::Named("Registry".to_string()),
        is_static: true,
        returns_ref: false,
        ..global.clone()
    };

    let registry = TypeDef {
        name: "Registry".to_string(),
        rust_path: "kreuzberg::Registry".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![global, load_embedded],
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
        doc: "Preset registry.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    ApiSurface {
        crate_name: "kreuzberg".to_string(),
        version: "0.1.0".to_string(),
        types: vec![registry],
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

#[test]
fn ffi_omits_ref_returning_static_but_keeps_owned_constructor() {
    let api = registry_api();
    let files = FfiBackend.generate_bindings(&api, &config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("kreuzberg_registry_global"),
        "FFI must NOT export a symbol for a borrow-returning static accessor"
    );
    assert!(
        lib.content.contains("kreuzberg_registry_load_embedded"),
        "FFI must still export the owned-value constructor symbol"
    );
}

#[test]
fn java_does_not_bind_missing_ref_returning_static_symbol() {
    let api = registry_api();
    let files = JavaBackend.generate_bindings(&api, &config()).unwrap();

    for file in &files {
        assert!(
            !file.content.contains("kreuzberg_registry_global") && !file.content.contains("KREUZBERG_REGISTRY_GLOBAL"),
            "Java must not resolve the non-existent `kreuzberg_registry_global` symbol \
             (would throw ExceptionInInitializerError); offender: {}",
            file.path.display()
        );
    }
    // The owned constructor must still be bound.
    assert!(
        files
            .iter()
            .any(|f| f.content.contains("kreuzberg_registry_load_embedded")),
        "Java must still bind the owned-value constructor symbol"
    );
}

#[test]
fn csharp_does_not_bind_missing_ref_returning_static_symbol() {
    let api = registry_api();
    let files = CsharpBackend.generate_bindings(&api, &config()).unwrap();

    for file in &files {
        assert!(
            !file.content.contains("kreuzberg_registry_global"),
            "C# must not declare a P/Invoke for the non-existent `kreuzberg_registry_global` \
             symbol (EntryPointNotFoundException); offender: {}",
            file.path.display()
        );
    }
    assert!(
        files
            .iter()
            .any(|f| f.content.contains("kreuzberg_registry_load_embedded")),
        "C# must still declare the owned-value constructor P/Invoke"
    );
}

#[test]
fn zig_does_not_call_missing_ref_returning_static_symbol() {
    let api = registry_api();
    let files = ZigBackend.generate_bindings(&api, &config()).unwrap();

    for file in &files {
        assert!(
            !file.content.contains("kreuzberg_registry_global"),
            "Zig must not call the non-existent `kreuzberg_registry_global` symbol; offender: {}",
            file.path.display()
        );
    }
    assert!(
        files
            .iter()
            .any(|f| f.content.contains("kreuzberg_registry_load_embedded")),
        "Zig must still call the owned-value constructor symbol"
    );
}
