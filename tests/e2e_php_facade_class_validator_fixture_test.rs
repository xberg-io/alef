//! Regression fixture for the php e2e `E2E (php)` hard-blocker: `[e2e.call.overrides.php]
//! class = "Xberg\\XbergApi"` was rejected by `validate_call_class_overrides` even though it
//! names the exact class the php-ext backend emits at runtime.
//!
//! `crate_facade_class_names`'s Php arm used to re-derive the facade name with the generic
//! `naming::to_class_name(php_extension_name)` helper, which only reproduces the hand-facing
//! wrapper class (`Xberg`) and never the separate `#[php_class]` extension facade
//! (`XbergApi`) that `rust_bindings::generate_bindings` actually emits and that free-function
//! e2e calls resolve against (`php_ext_api_class_name`'s doc comment). So the only override
//! value accepted at validation time (`Xberg\Xberg`) named a class that does not exist at
//! runtime, and the only value correct at runtime (`Xberg\XbergApi`) was rejected as a typo.
//!
//! This is a standalone reproduction crate (crate name `xberg`, mirroring the real xberg
//! config's identifiers) rather than a copy of xberg's own `alef.toml`, which is mid-migration
//! in that repo and not available here. It exercises the same emitter/validator pair xberg hit:
//! `PhpBackend::generate_bindings` (real emitter) and `validate_call_class_overrides` (real
//! validator), both called through their public API exactly as `generate_e2e` and the php
//! backend's own codegen call them.

use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use alef::e2e::validate_call_class::validate_call_class_overrides;

fn config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "xberg"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "Xberg"
result_var = "result"
async = false
returns_result = true

[crates.e2e.call.overrides.php]
class = "Xberg\\XbergApi"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("fixture config must parse");
    cfg.resolve().expect("fixture config must resolve").remove(0)
}

/// A single crate-level free function -- this is what forces `rust_bindings::generate_bindings`
/// to emit the `XbergApi` facade class at all (see `rust_bindings.rs`'s
/// `!included_functions.is_empty()` gate).
fn free_function_api() -> ApiSurface {
    let convert = FunctionDef {
        name: "convert".to_string(),
        rust_path: "xberg::convert".to_string(),
        params: vec![ParamDef {
            name: "input".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        return_type: TypeRef::String,
        doc: "Convert input text.".to_string(),
        ..Default::default()
    };

    ApiSurface {
        crate_name: "xberg".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![convert],
        ..Default::default()
    }
}

/// A single trivial type -- `validate_call_class_overrides` skips validation entirely when
/// both `type_defs` and `enums` are empty (see its doc comment), so a non-empty IR is required
/// to exercise the real check rather than the empty-IR no-op path.
fn minimal_type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "ConvertResult".to_string(),
        fields: vec![FieldDef {
            name: "text".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..TypeDef::default()
    }]
}

/// The emitter half: `PhpBackend::generate_bindings` must actually produce a
/// `#[php_class] #[php(name = "Xberg\XbergApi")]` struct for a crate with one free function --
/// confirming `Xberg\XbergApi` is a genuine, runtime-real class name, not a hypothetical.
#[test]
fn php_ext_backend_emits_the_api_facade_class_the_override_names() {
    use alef::backends::php::PhpBackend;
    use alef::core::backend::Backend;

    let files = PhpBackend
        .generate_bindings(&free_function_api(), &config())
        .expect("php-ext bindings generation succeeds");
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content;

    assert!(
        lib_rs.contains("#[php_class]\n#[php(name = \"Xberg\\\\XbergApi\")]\npub struct XbergApi;"),
        "the php-ext backend must emit a #[php_class] struct named \"Xberg\\\\XbergApi\" for a \
         crate with a free function; lib.rs:\n{lib_rs}"
    );
}

/// The validator half: an `[e2e.call.overrides.php] class = "Xberg\XbergApi"` override --
/// naming exactly the class the assertion above proves is real -- must validate cleanly.
#[test]
fn class_override_naming_the_real_api_facade_passes_validation() {
    let resolved = config();
    let e2e_config = {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["php"]

[[crates]]
name = "xberg"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "Xberg"
result_var = "result"
async = false
returns_result = true

[crates.e2e.call.overrides.php]
class = "Xberg\\XbergApi"
"#,
        )
        .expect("fixture config must parse");
        cfg.crates[0].e2e.clone().expect("e2e config present")
    };

    let errors = validate_call_class_overrides(
        &e2e_config,
        &resolved,
        &minimal_type_defs(),
        &[],
        &["php".to_string()],
    );

    assert!(
        errors.is_empty(),
        "class = \"Xberg\\\\XbergApi\" names the real #[php_class] facade the backend emits and \
         must pass validation; got: {errors:?}"
    );
}

/// Sanity check on the other side of the original bug: the wrapper class name
/// (`Xberg\Xberg`, with no `Api` suffix) must *also* still validate -- it is the real,
/// separately-emitted hand-facing wrapper class (`php_public_class_name`), not a typo.
#[test]
fn class_override_naming_the_public_wrapper_also_passes_validation() {
    let resolved = config();
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "xberg"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "Xberg"
result_var = "result"
async = false
returns_result = true

[crates.e2e.call.overrides.php]
class = "Xberg\\Xberg"
"#,
    )
    .expect("fixture config must parse");
    let e2e_config = cfg.crates[0].e2e.clone().expect("e2e config present");

    let errors = validate_call_class_overrides(
        &e2e_config,
        &resolved,
        &minimal_type_defs(),
        &[],
        &["php".to_string()],
    );

    assert!(
        errors.is_empty(),
        "class = \"Xberg\\\\Xberg\" names the real public wrapper class and must pass \
         validation; got: {errors:?}"
    );
}
