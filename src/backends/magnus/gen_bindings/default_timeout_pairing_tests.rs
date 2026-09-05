//! Agreement test for the two emitters that decide, independently, whether a `request_timeout`/
//! `timeout` field gets Magnus's hardcoded `default_timeout` fallback.
//!
//! The module-level pass in `gen_bindings::generate_bindings` decides whether to emit the free
//! `fn default_timeout() -> u64 { 30000 }` at all; `classes::gen_struct`'s per-field pass decides
//! whether to attach `#[serde(default = "default_timeout")]` to a given field. They used to
//! re-derive the check independently — the free-function side matched `FieldDef::ty` directly and
//! never looked at `field.optional`, while the per-field side matched the *type-mapped* Rust
//! string against the literal `"u64"` (which an `Option<u64>` field maps to `"Option<u64>"`).
//! The two answers agreed only because a `request_timeout`/`timeout` field typed `Option<u64>` or
//! `Option<Duration>` never occurred in the fixtures that exercised this path: had one occurred,
//! the free function would still be emitted (dead code, referenced by nothing) because the
//! per-field side would never attach the attribute that names it. This test drives both real
//! emitters from one IR fixture through `MagnusBackend::generate_bindings` and asserts the
//! function is never defined without being referenced, and never referenced without being
//! defined. ~keep

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};

const DEFAULT_TIMEOUT_FN_DEF: &str = "fn default_timeout() -> u64";
const DEFAULT_TIMEOUT_ATTR: &str = "#[serde(default = \"default_timeout\")]";

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
    )
}

fn timeout_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn type_with_field(name: &str, field: FieldDef) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![field],
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

fn api_with_types(types: Vec<TypeDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types,
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn generated_lib_rs(api: &ApiSurface) -> String {
    let backend = MagnusBackend;
    let config = make_config();
    let files = backend
        .generate_bindings(api, &config)
        .expect("magnus bindings generated");
    files[0].content.clone()
}

/// The bug this guards against: a field the free-function predicate accepted (it ignored
/// `field.optional`) but the per-field predicate never referenced (its mapped type includes the
/// `Option<...>` wrapper, not a bare `"u64"`), leaving `default_timeout` generated and dead.
/// Before the shared-predicate fix this failed: `fn default_timeout` was defined with zero
/// `#[serde(default = "default_timeout")]` references anywhere in the file. ~keep
#[test]
fn default_timeout_is_never_defined_without_a_referencing_field() {
    let api = api_with_types(vec![type_with_field(
        "Config",
        timeout_field("timeout", TypeRef::Primitive(PrimitiveType::U64), true),
    )]);
    let content = generated_lib_rs(&api);

    let is_defined = content.contains(DEFAULT_TIMEOUT_FN_DEF);
    let is_referenced = content.contains(DEFAULT_TIMEOUT_ATTR);

    assert!(
        !(is_defined && !is_referenced),
        "`default_timeout` was defined but never referenced by a field attribute — dead code the \
         two independent predicates used to disagree on:\n{content}"
    );
    // For this fixture (the only qualifying-by-name field is `Option<u64>`), the correct answer
    // is that NEITHER side fires — an optional field does not need the hardcoded fallback.
    assert!(
        !is_defined && !is_referenced,
        "an Option<u64> `timeout` field must not trigger `default_timeout` on either side, got:\n{content}"
    );
}

/// A bare (non-optional) `u64` `timeout`/`request_timeout` field is the case the fallback exists
/// for — narrowing the shared predicate must not silently drop it. ~keep
#[test]
fn non_optional_timeout_field_still_gets_both_the_function_and_the_attribute() {
    let api = api_with_types(vec![type_with_field(
        "Config",
        timeout_field("request_timeout", TypeRef::Primitive(PrimitiveType::U64), false),
    )]);
    let content = generated_lib_rs(&api);

    assert!(
        content.contains(DEFAULT_TIMEOUT_FN_DEF),
        "expected `fn default_timeout` to be generated, got:\n{content}"
    );
    assert!(
        content.contains(DEFAULT_TIMEOUT_ATTR),
        "expected the field to reference `default_timeout`, got:\n{content}"
    );
}

/// When some other field in the surface legitimately triggers `default_timeout`, an `Option<u64>`
/// `timeout` field elsewhere must still not reference it — the function existing for someone else
/// is not license to attach the attribute to a field the predicate rejects. ~keep
#[test]
fn optional_timeout_field_does_not_reference_a_function_emitted_for_another_field() {
    let api = api_with_types(vec![
        type_with_field(
            "Config",
            timeout_field("request_timeout", TypeRef::Primitive(PrimitiveType::U64), false),
        ),
        type_with_field(
            "OtherConfig",
            timeout_field("timeout", TypeRef::Primitive(PrimitiveType::U64), true),
        ),
    ]);
    let content = generated_lib_rs(&api);

    assert!(
        content.contains(DEFAULT_TIMEOUT_FN_DEF),
        "expected `fn default_timeout` to be generated for the non-optional field, got:\n{content}"
    );
    assert_eq!(
        content.matches(DEFAULT_TIMEOUT_ATTR).count(),
        1,
        "expected exactly one field to reference `default_timeout` (the non-optional one), got:\n{content}"
    );
}
