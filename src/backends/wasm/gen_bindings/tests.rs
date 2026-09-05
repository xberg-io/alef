use super::{
    WasmBackend, cargo::gen_cargo_toml, fix_dropped_payload_enum_option_fields,
    types_needing_self_delegation_reverse_impl,
};
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
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

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn wasm_backend_name_is_wasm() {
    assert_eq!(WasmBackend.name(), "wasm");
}

#[test]
fn generate_bindings_empty_api_produces_files() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].path.to_string_lossy().ends_with("lib.rs"));
    assert!(files[1].path.to_string_lossy().ends_with("Cargo.toml"));
}

#[test]
fn extra_dependency_overrides_builtin_without_duplicate_key() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dependencies]
serde = { version = "1", features = ["derive", "rc"] }
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);

    let serde_lines = cargo_toml
        .lines()
        .filter(|l| l.trim_start().starts_with("serde =") || l.trim_start().starts_with("serde="))
        .count();
    assert_eq!(serde_lines, 1, "expected exactly one `serde` key, got:\n{cargo_toml}");
    assert!(
        cargo_toml.contains(r#"features = ["derive", "rc"]"#),
        "extra_dependencies override should win:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

/// `[crates.cargo_lints]` must round-trip into the emitted wasm `Cargo.toml` as a
/// `[lints.rust]` / `[lints.clippy]` block, and produce valid TOML. The wasm crate has no
/// hand-written `[lints.rust]` block of its own (unlike dart/swift/elixir's `unexpected_cfgs`
/// allowlist), so the `rust` table is a plain splice; `clippy` still merges with
/// [`crate::core::config::CargoLintsConfig`]'s builtin deny defaults regardless of backend.
#[test]
fn cargo_toml_emits_configured_cargo_lints() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.cargo_lints.rust]
unused_must_use = "deny"

[crates.cargo_lints.clippy]
print_stdout = "deny"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let cargo_toml = gen_cargo_toml(&empty_api(), &config);

    assert!(
        cargo_toml.contains("[lints.rust]\nunused_must_use = \"deny\""),
        "expected [lints.rust] block, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "expected the configured clippy entry to merge with the builtin deny defaults, got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml with cargo_lints must be valid TOML");
}

/// Absence of `[crates.cargo_lints]` must still emit the builtin `[lints.clippy]` deny
/// block; no `[lints.rust]` table is emitted since nothing configures it.
#[test]
fn cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&empty_api(), &config);
    assert!(
        !cargo_toml.contains("[lints.rust]"),
        "no [lints.rust] table should be emitted when cargo_lints.rust is unset, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "the builtin [lints.clippy] deny block must survive even when cargo_lints is unset, got:\n{cargo_toml}"
    );
}

#[test]
fn cargo_toml_emits_passthrough_features_for_type_cfg_attrs() {
    use crate::core::ir::TypeDef;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "PdfThing".to_string(),
            rust_path: "test_lib::PdfThing".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        cargo_toml.contains(r#"pdf = ["test-lib/pdf"]"#),
        "expected `pdf = [\"test-lib/pdf\"]` in:\n{cargo_toml}"
    );
    assert_eq!(
        cargo_toml.matches("\n[features]\n").count(),
        1,
        "exactly one [features] block expected:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);
    assert!(
        !cargo_toml.contains("[features]"),
        "expected no [features] block:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_declares_configured_extra_features_without_enabling_them() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
extra_features = ["sceptre-wasm", "", "sceptre-wasm", "telemetry"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert_eq!(
        cargo_toml
            .matches(r#"sceptre-wasm = ["test-lib/sceptre-wasm"]"#)
            .count(),
        1,
        "extra features must be deduplicated in:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"telemetry = ["test-lib/telemetry"]"#),
        "expected telemetry passthrough in:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("default = ["),
        "extra features must remain opt-in in:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_enables_configured_binding_features_by_default() {
    use crate::core::ir::TypeDef;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
features = ["wasm-target"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GatedType".to_string(),
            rust_path: "test_lib::GatedType".to_string(),
            cfg: Some(r#"any(feature = "wasm-target", feature = "extra")"#.to_string()),
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);
    assert!(
        cargo_toml.contains(r#"extra = ["test-lib/extra"]"#),
        "expected `extra` passthrough:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"wasm-target = ["test-lib/wasm-target"]"#),
        "wasm-target must be declared as passthrough so rustc sees the feature:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"default = ["wasm-target"]"#),
        "configured core features must also enable matching binding-side cfg gates:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains(r#"default = ["extra"]"#),
        "unconfigured discovered gates must remain opt-in:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_has_no_issues_docs_line_and_getrandom_deps_are_alphabetical() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        !cargo_toml.contains("Issues & docs:"),
        "Cargo.toml must not contain 'Issues & docs:' line — cargo-sort strips it and \
             alef re-emits it, causing prek to loop forever:\n{cargo_toml}"
    );

    let pos_02 = cargo_toml
        .find("getrandom_02")
        .expect("getrandom_02 must be present in target deps");
    let pos_03 = cargo_toml
        .find("getrandom_03")
        .expect("getrandom_03 must be present in target deps");
    assert!(
        pos_02 < pos_03,
        "getrandom_02 must appear before getrandom_03 (alphabetical order for cargo-sort \
             compatibility); got getrandom_02 at {pos_02}, getrandom_03 at {pos_03}:\n{cargo_toml}"
    );

    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_emits_extra_dev_dependencies() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dev_dependencies]
wasm-bindgen-test = "0.3"
serde_json = { version = "1", features = ["preserve_order"] }
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        cargo_toml.contains("[dev-dependencies]"),
        "expected a [dev-dependencies] section in:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"wasm-bindgen-test = "0.3""#),
        "expected the string-valued dev dependency in:\n{cargo_toml}"
    );
    let parsed: toml::Value = toml::from_str(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    let dev = parsed
        .get("dev-dependencies")
        .expect("dev-dependencies table must exist");
    assert!(dev.get("serde_json").and_then(|v| v.get("features")).is_some());

    let plain = gen_cargo_toml(&api, &make_config());
    assert!(
        !plain.contains("[dev-dependencies]"),
        "unexpected dev-deps in:\n{plain}"
    );
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders manifest
/// tables `[dependencies]` -> `[target.'cfg(...)'.dependencies]` ->
/// `[build-dependencies]` -> `[dev-dependencies]`. The wasm binding crate
/// always carries a `[target.'cfg(target_arch = "wasm32")'.dependencies]`
/// block for `getrandom`, so whenever `extra_dev_dependencies` also produces a
/// `[dev-dependencies]` section, the target block must come first — cargo-sort
/// rejects a manifest with `[dev-dependencies]` before a later `[target.*]`
/// table.
#[test]
fn cargo_toml_orders_target_block_before_dev_dependencies() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dev_dependencies]
wasm-bindgen-test = "0.3"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let cargo_toml = gen_cargo_toml(&api, &config);

    let target_pos = cargo_toml
        .find("[target.'cfg(target_arch = \"wasm32\")'.dependencies]")
        .expect("expected the wasm32 target block");
    let dev_pos = cargo_toml
        .find("[dev-dependencies]")
        .expect("expected a [dev-dependencies] section");

    assert!(
        target_pos < dev_pos,
        "the [target.*] block must precede [dev-dependencies]; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

/// Regression test for a wasm-only E0308: a type that is never a function/method *parameter*
/// (directly or transitively) has no reason to appear in `input_type_names`, so the
/// binding->core `From` impl is normally skipped for it. But if that same type has an
/// auto-delegated instance method (e.g. `PageRange::page_count(&self) -> u32`, only ever
/// *returned*, never taken as input), `gen_method` still emits
/// `{core}::{Type}::from(self.clone()).{method}(..)`, which requires exactly that impl to exist.
/// `types_needing_self_delegation_reverse_impl` must flag such types so the reverse impl gets
/// generated regardless of `input_type_names`.
#[test]
fn types_needing_self_delegation_reverse_impl_flags_return_only_delegating_type() {
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "PageRange".to_string(),
        rust_path: "test_lib::PageRange".to_string(),
        fields: vec![
            FieldDef {
                name: "start".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
            FieldDef {
                name: "end".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
        ],
        methods: vec![MethodDef {
            name: "page_count".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        }],
        ..Default::default()
    }];

    let needed = types_needing_self_delegation_reverse_impl(&api, &ahash::AHashSet::default());
    assert!(
        needed.contains("PageRange"),
        "a type with a self-delegating instance method must require the binding->core reverse \
         impl even though it is never used as an input, got {needed:?}"
    );
}

/// A type none of whose methods reach the self-delegation branch must NOT be flagged — doing so
/// would only add dead, unused `From` impls.
///
/// Note the `&mut self` method has to be non-delegatable for this to hold. `gen_method` routes an
/// opaque type's *non*-mut methods through the mutex-lock path
/// (`self.inner.lock().unwrap().{method}(..)`, methods.rs:156), but its `&mut self` methods fall
/// through to the `self.clone()` self-delegation form — so an opaque type with any delegatable
/// `&mut self` method genuinely does need the reverse impl. `sanitized` is what makes `resize`
/// non-delegatable here.
#[test]
fn types_needing_self_delegation_reverse_impl_ignores_opaque_mutex_delegated_type() {
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "Pool".to_string(),
        rust_path: "test_lib::Pool".to_string(),
        is_opaque: true,
        methods: vec![
            MethodDef {
                name: "resize".to_string(),
                return_type: TypeRef::Primitive(PrimitiveType::Bool),
                receiver: Some(ReceiverKind::RefMut),
                cfg: None,
                sanitized: true,
                ..Default::default()
            },
            MethodDef {
                name: "len".to_string(),
                return_type: TypeRef::Primitive(PrimitiveType::Usize),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let opaque_types: ahash::AHashSet<String> = ["Pool".to_string()].into_iter().collect();

    let needed = types_needing_self_delegation_reverse_impl(&api, &opaque_types);
    assert!(
        !needed.contains("Pool"),
        "an opaque type whose non-mut methods route through the mutex-lock path needs no \
         binding->core reverse impl, got {needed:?}"
    );
}

/// End-to-end coverage: a type that is only ever returned (never an input) but has an
/// auto-delegated instance method must get a real `impl From<Wasm{T}> for {core}::{T}` in the
/// actual generated `lib.rs`, and `gen_method`'s self-delegation call must reference that exact
/// core type -- a real downstream wasm crate failed to compile with E0308 before this fix.
#[test]
fn generated_lib_rs_has_reverse_impl_for_return_only_delegating_type() {
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "PageRange".to_string(),
        rust_path: "test_lib::PageRange".to_string(),
        fields: vec![
            FieldDef {
                name: "start".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
            FieldDef {
                name: "end".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
        ],
        methods: vec![MethodDef {
            name: "page_count".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        }],
        ..Default::default()
    }];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        lib_rs.contains("impl From<WasmPageRange> for test_lib::PageRange {"),
        "expected a binding->core reverse impl for PageRange:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("test_lib::PageRange::from(self.clone()).page_count()"),
        "expected the self-delegation call the reverse impl above exists to support:\n{lib_rs}"
    );
}

#[test]
fn instance_method_with_borrowed_named_input_delegates_to_core() {
    let method = MethodDef {
        name: "evaluate".to_string(),
        params: vec![ParamDef {
            name: "options".to_string(),
            ty: TypeRef::Named("Options".to_string()),
            is_ref: true,
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        error_type: Some("EvaluationError".to_string()),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Evaluator".to_string(),
        methods: vec![method.clone()],
        ..Default::default()
    };
    let mapper = crate::backends::wasm::type_map::WasmMapper::new(Default::default(), "Wasm".to_string());
    let output = super::methods::gen_method(
        &method,
        &mapper,
        "Evaluator",
        "sample_core",
        &Default::default(),
        "Wasm",
        &typ,
        &Default::default(),
        &Default::default(),
    );

    assert!(output.contains("let options_core: sample_core::Options"), "{output}");
    assert!(
        output.contains("sample_core::Evaluator::from(self.clone()).evaluate(&options_core)"),
        "{output}"
    );
    assert!(!output.contains("Not implemented"), "{output}");
}

/// Regression test for a wasm-only E0282: a field whose Rust type is a payload-carrying enum
/// (`#[serde(tag = "type")]` with struct variants) has no wasm-bindgen representation, so
/// `gen_struct` drops it from the generated Wasm struct. The shared binding->core `From`
/// conversion generator does not know that, and for an `Option<Box<T>>` field falls back to
/// `Default::default().map(Box::new)` -- untypeable, since nothing pins down `T`. The post-process
/// fixup must replace it with a self-documenting `None`.
#[test]
fn fix_dropped_payload_enum_option_fields_replaces_untypeable_default_with_documented_none() {
    let content = "impl From<test_lib::LlmConfig> for WasmLlmConfig {\n    fn from(val: test_lib::LlmConfig) -> Self {\n        Self {\n            model: val.model,\n        }\n    }\n}\nimpl From<WasmLlmConfig> for test_lib::LlmConfig {\n    fn from(val: WasmLlmConfig) -> Self {\n        Self {\n            model: val.model,\n            credential_provider: Default::default().map(Box::new),\n        }\n    }\n}\n".to_string();

    let fixed = fix_dropped_payload_enum_option_fields(content);

    assert!(
        !fixed.contains("Default::default().map(Box::new)"),
        "untypeable expression must be fully replaced:\n{fixed}"
    );
    assert!(
        fixed.contains("credential_provider: None,"),
        "field must fall back to a literal `None`:\n{fixed}"
    );
    assert!(
        fixed.contains("// ALEF-OMITTED: `credential_provider` is always None on wasm"),
        "the omission must be documented in the generated source so a reader learns why the \
         field is always None:\n{fixed}"
    );
}

/// The fixup must be a no-op on content that never had the buggy pattern -- it must not, for
/// example, touch ordinary `field: Default::default(),` lines that don't end in `.map(Box::new)`.
#[test]
fn fix_dropped_payload_enum_option_fields_is_noop_without_the_pattern() {
    let content = "Self {\n    reason: ChunkingReason::default(),\n    other: Default::default(),\n}\n".to_string();
    let fixed = fix_dropped_payload_enum_option_fields(content.clone());
    assert_eq!(fixed, content, "content without the buggy pattern must be unchanged");
}

/// Regression test: wasm-pack's own `pkg/nodejs/package.json` (produced by
/// `--target nodejs --out-dir pkg/nodejs`) declares a `"name"` derived from the wasm crate's
/// `Cargo.toml`, not `config.wasm_package_name()` — the name every e2e-generated `file:`
/// dependency and `require()`/`import` specifier actually uses. Without a post-build step to
/// reconcile them, the specifier names a package the built directory does not declare.
#[test]
fn build_config_with_config_rewrites_wasm_pack_package_json_name() {
    let backend = WasmBackend;
    let config = make_config();

    let build_config = backend
        .build_config_with_config(&config)
        .expect("wasm backend must report a build config");

    let rewrite_step = build_config
        .post_build
        .iter()
        .find_map(|step| match step {
            crate::core::backend::PostBuildStep::RewriteWasmPackageName {
                package_json_path,
                package_name,
            } => Some((package_json_path.clone(), package_name.clone())),
            _ => None,
        })
        .expect("build_config_with_config must attach a RewriteWasmPackageName post-build step");

    assert_eq!(rewrite_step.1, "test-lib-wasm");
    assert_eq!(
        rewrite_step.0,
        std::path::PathBuf::from("crates/test-lib-wasm/pkg/nodejs/package.json"),
        "the rewrite target must be the exact directory `build_command_for`'s wasm-pack \
         arm builds into: {:?}",
        rewrite_step.0
    );
}

fn rewrite_target_for(toml_src: &str) -> std::path::PathBuf {
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    let config = cfg.resolve().unwrap().remove(0);

    WasmBackend
        .build_config_with_config(&config)
        .expect("wasm backend must report a build config")
        .post_build
        .iter()
        .find_map(|step| match step {
            crate::core::backend::PostBuildStep::RewriteWasmPackageName { package_json_path, .. } => {
                Some(package_json_path.clone())
            }
            _ => None,
        })
        .expect("build_config_with_config must attach a RewriteWasmPackageName post-build step")
}

/// Failure path: when `[crates.output] wasm` is set, `build_command_for` resolves the crate
/// dir from *that* path (dropping a trailing `src`, which holds the generated sources rather
/// than the crate root) and ignores the `package_dir` default formula. This step must follow
/// the same rule, or the build writes `pkg/nodejs` under one directory while the rewrite
/// looks under another — and since a missing file is only debug-logged, the rewrite would
/// silently never fire and the name mismatch would survive the build. The two directories
/// below deliberately disagree, which is exactly what a `package_dir`-only derivation gets
/// wrong. ~keep
#[test]
fn rewrite_target_follows_explicit_output_rather_than_the_package_dir_formula() {
    let target = rewrite_target_for(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.output]
wasm = "crates/renamed-wasm-crate/src/"
"#,
    );

    assert_eq!(
        target,
        std::path::PathBuf::from("crates/renamed-wasm-crate/pkg/nodejs/package.json")
    );
    assert!(
        !target.starts_with("crates/test-lib-wasm"),
        "must not fall back to the package_dir formula when [crates.output] wasm is set: {target:?}"
    );
}

/// An explicit output that already names the crate root (no trailing `src`) must be used
/// verbatim rather than having its last component stripped.
#[test]
fn rewrite_target_keeps_an_explicit_output_that_is_already_the_crate_root() {
    let target = rewrite_target_for(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.output]
wasm = "crates/renamed-wasm-crate"
"#,
    );

    assert_eq!(
        target,
        std::path::PathBuf::from("crates/renamed-wasm-crate/pkg/nodejs/package.json")
    );
}

/// Async instance methods return `{mapped}::from(result)`, which only compiles when the mapped
/// type has a `From<CoreType>`. The wasm mapper collapses every `Map` (and every `Json`, and any
/// `Named` a `type_overrides` entry redirects) onto the opaque `JsValue`, which has no such impl
/// — `JsValue::from(HashMap<..>)` is an `E0277`. The value must cross through serde instead,
/// exactly as the generated `From<CoreType> for WasmType` bodies do for degraded fields.
#[test]
fn async_method_returning_map_bridges_through_serde_not_from() {
    let method = MethodDef {
        name: "headers".to_string(),
        is_async: true,
        return_type: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Request".to_string(),
        methods: vec![method.clone()],
        ..Default::default()
    };
    let mapper = crate::backends::wasm::type_map::WasmMapper::new(Default::default(), "Wasm".to_string());

    let output = super::methods::gen_method(
        &method,
        &mapper,
        "Request",
        "sample_core",
        &Default::default(),
        "Wasm",
        &typ,
        &Default::default(),
        &Default::default(),
    );

    assert!(
        output.contains("serde_wasm_bindgen::to_value(&result)"),
        "a JsValue-mapped return must be serialized:\n{output}"
    );
    assert!(
        !output.contains("JsValue::from(result)"),
        "JsValue has no From<HashMap<..>>:\n{output}"
    );
}

/// Positive control for the above: a `Named` return really does map to a generated wrapper with
/// a `From<CoreType>`, so the turbofish `from` must stay.
#[test]
fn async_method_returning_named_still_uses_from() {
    let method = MethodDef {
        name: "report".to_string(),
        is_async: true,
        return_type: TypeRef::Named("Report".to_string()),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Request".to_string(),
        methods: vec![method.clone()],
        ..Default::default()
    };
    let mapper = crate::backends::wasm::type_map::WasmMapper::new(Default::default(), "Wasm".to_string());

    let output = super::methods::gen_method(
        &method,
        &mapper,
        "Request",
        "sample_core",
        &Default::default(),
        "Wasm",
        &typ,
        &Default::default(),
        &Default::default(),
    );

    assert!(
        output.contains("WasmReport::from(result)"),
        "a wrapper-mapped return must keep the direct From conversion:\n{output}"
    );
    assert!(
        !output.contains("serde_wasm_bindgen::to_value(&result)"),
        "a wrapper-mapped return must not detour through serde:\n{output}"
    );
}

/// The WASM binding manifest is emitted by this backend rather than by `alef scaffold`, so it
/// needs its own guard that `cargo sort --check` would accept it -- in particular that
/// `[lints.*]` trails every dependency table instead of sitting after `[package]`. ~keep
#[test]
fn cargo_toml_tables_are_in_cargo_sort_canonical_order() {
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&empty_api(), &config);

    crate::test_support::cargo_sort_order::assert_canonical_table_order("wasm Cargo.toml", &cargo_toml);
    assert!(
        crate::test_support::cargo_sort_order::assert_dependency_keys_sorted("wasm Cargo.toml", &cargo_toml) > 0,
        "the wasm manifest must carry dependency keys for the key-order check to examine"
    );

    let lints_at = cargo_toml
        .find("[lints.")
        .expect("wasm manifest must carry a lints table");
    for table in ["[dependencies]", "[dev-dependencies]", "[target."] {
        if let Some(table_at) = cargo_toml.find(table) {
            assert!(
                lints_at > table_at,
                "`{table}` must precede the lints table:\n{cargo_toml}"
            );
        }
    }
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated wasm Cargo.toml must be valid TOML");
}

/// The manifest's core-crate `path` must agree with where the manifest is actually written.
///
/// `gen_cargo_toml` hard-coded `path = "../{core_crate_dir}"`, which resolves only when the core
/// crate is a `crates/` sibling of the wasm crate. For a root-flat core crate -- Cargo.toml at
/// the project root, alef's own shape since 0.18.0 -- the emitted tree has no `crates/<core>`,
/// so cargo failed with "failed to read .../crates/toolkit/Cargo.toml" before compiling a line.
/// Same defect the FFI/Python/Node/PHP scaffolders had; the wasm backend builds its own manifest
/// and so was missed. ~keep
#[test]
fn core_dep_path_matches_the_layout_the_manifest_is_written_into() {
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&empty_api(), &config);

    let expected = config.core_crate_dep_path(&super::wasm_output_layout(&config).root);
    assert_eq!(
        expected, "../..",
        "sanity: the default layout puts the wasm crate two levels below a root-flat core crate"
    );
    assert!(
        cargo_toml.contains(&format!(r#"path = "{expected}""#)),
        "the core dep path must be derived from the emitted layout, got:\n{cargo_toml}"
    );
}

/// Every dependency the wasm manifest declares must either be referenced by the generated Rust
/// or be listed in `[package.metadata.cargo-machete].ignored`.
///
/// `cargo-machete` runs as a pre-commit stage in consumer repos, so a dependency this emitter
/// declares unconditionally while only *sometimes* generating a reference to it blocks every
/// commit in a repo whose surface never triggers that reference. The manifest is alef-generated,
/// which leaves the consumer with nothing to fix on their side. `serde-wasm-bindgen` was exactly
/// that case: emitted for every wasm crate, referenced only when a field, trait bridge or return
/// value actually bridges through it. Asserting the general rule rather than that one name keeps
/// the next unconditional dependency from reintroducing it. ~keep
#[test]
fn every_unreferenced_wasm_dependency_is_declared_machete_ignored() {
    let config = make_config();
    let files = WasmBackend.generate_bindings(&empty_api(), &config).unwrap();
    let generated_rust: String = files
        .iter()
        .filter(|f| f.path.extension().is_some_and(|e| e == "rs"))
        .map(|f| f.content.as_str())
        .collect();

    let manifest: toml::Value =
        toml::from_str(&gen_cargo_toml(&empty_api(), &config)).expect("manifest must be valid TOML");
    let ignored: std::collections::HashSet<&str> = manifest["package"]["metadata"]["cargo-machete"]["ignored"]
        .as_array()
        .expect("cargo-machete.ignored must be an array")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    let dependencies = manifest["dependencies"]
        .as_table()
        .expect("[dependencies] must be a table");

    // A `path` dependency is the crate's own core, which the generated code always calls into and
    // whose key alef derives rather than fixes; every registry name has to earn its place either
    // by appearing in the emitted Rust or by being ignored. ~keep
    let unjustified: Vec<&str> = dependencies
        .iter()
        .filter(|(_, spec)| spec.get("path").is_none())
        .map(|(name, _)| name.as_str())
        .filter(|name| !ignored.contains(name))
        .filter(|name| !generated_rust.contains(&name.replace('-', "_")))
        .collect();

    assert!(
        unjustified.is_empty(),
        "cargo-machete will flag these declared-but-unreferenced dependencies, and the consumer \
         cannot fix an alef-generated manifest: {unjustified:?}"
    );
}
