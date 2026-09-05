//! Regression test for task #428: the Go backend's opaque-handle C type names must be
//! derived through `c_consumer::export_type_prefix` (cbindgen's actual `[export] prefix`
//! shouty-snake-casing), not a local `.to_uppercase()`.
//!
//! `src/backends/ffi/gen_bindings/helpers.rs::gen_cbindgen_toml` writes
//! `c_consumer::export_type_prefix(prefix)` into cbindgen's `[export] prefix`, which cbindgen
//! then prepends verbatim to every exported C type name. A Go backend call site that
//! plain-uppercases the FFI prefix instead spells a type that occurs zero times in the real
//! header whenever the prefix has an internal word boundary.

use alef::backends::go::GoBackend;
use alef::codegen::c_consumer;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn opaque_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// Prove that this fixture's prefix actually makes the two candidate formulas disagree.
/// `sample_core` (already snake_case) would make `to_shouty_snake_case` and `to_uppercase`
/// produce the identical string, so a test built on it would pass whether or not the backend
/// used the authority. `SampleCore` has no separator of its own, so shouty-snake-casing
/// (`SAMPLE_CORE`) and plain-uppercasing (`SAMPLECORE`) genuinely differ — this is the same
/// example `c_consumer::export_type_prefix`'s own doc comment uses. ~keep
const FIXTURE_PREFIX: &str = "SampleCore";

#[test]
fn go_opaque_handle_c_type_uses_shouty_snake_prefix_not_plain_uppercase() {
    let config = resolved_one(&format!(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "{FIXTURE_PREFIX}"

[crates.go]
module = "github.com/test/test-lib"
"#
    ));

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![opaque_type("Widget"), opaque_type("Canvas")],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let correct_prefix = c_consumer::export_type_prefix(FIXTURE_PREFIX);
    let buggy_prefix = FIXTURE_PREFIX.to_uppercase();
    assert_ne!(
        correct_prefix, buggy_prefix,
        "fixture prefix must make shouty-snake-case and plain-uppercase disagree"
    );
    assert_eq!(correct_prefix, "SAMPLE_CORE");
    assert_eq!(buggy_prefix, "SAMPLECORE");

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding_go = files.iter().find(|file| file.path.ends_with("binding.go")).unwrap();

    // Derived from the same authority `gen_cbindgen_toml` uses, so this test cannot drift out
    // of sync with what the real header actually declares.
    let correct_type_decl = format!("ptr C.{correct_prefix}Widget");
    let buggy_type_decl = format!("ptr C.{buggy_prefix}Widget");

    assert!(
        binding_go.content.contains(&correct_type_decl),
        "opaque handle field must name the cbindgen-prefixed C type ({correct_type_decl}), got:\n{}",
        binding_go.content
    );
    // Negative control: exactly the string the pre-fix `ffi_prefix.to_uppercase()` formula
    // produced. If the fix regresses, this assertion is what catches it.
    assert!(
        !binding_go.content.contains(&buggy_type_decl),
        "opaque handle field must not use the plain-uppercased prefix ({buggy_type_decl}), got:\n{}",
        binding_go.content
    );

    // The struct-field emitter above is the only Go site that spells a prefixed C type in
    // output this fixture can reach. `gen_bindings/methods.rs`'s two opaque-param casts were not
    // reachable by a plain, an optional, or a free-function opaque parameter -- Go passes an
    // opaque argument as `x.ptr`, which is already typed -- so they are unverified here and may
    // be dead in the same way `gen_bindings/service_api.rs`'s five `upper_prefix` sites are. ~keep
}
