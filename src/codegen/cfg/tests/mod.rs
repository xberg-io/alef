use super::*;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, TypeDef};
use tracing_test::traced_test;

mod drift;
mod enabled_features;
mod feature_forwarding;
mod restrict_declared;

#[test]
fn combine_gates_drops_an_owner_the_member_already_requires() {
    assert_eq!(
        combine_gates(
            r#"feature = "client""#,
            r#"all(feature = "client", feature = "streaming")"#
        ),
        r#"all(feature = "client", feature = "streaming")"#
    );
}

#[test]
fn combine_gates_keeps_both_operands_when_the_member_does_not_imply_the_owner() {
    assert_eq!(
        combine_gates(r#"feature = "client""#, r#"feature = "streaming""#),
        r#"all(feature = "client", feature = "streaming")"#
    );
}

#[test]
fn combine_gates_does_not_collapse_two_predicates_the_parser_cannot_read() {
    // Both sides parse to `CfgPredicate::Other`, which compares equal without being the same
    // condition. Collapsing here would compile the member in on a target the owner excludes.
    assert_eq!(
        combine_gates("target_os = \"macos\"", "target_os = \"linux\""),
        "all(target_os = \"macos\", target_os = \"linux\")"
    );
}

#[test]
fn combine_gates_does_not_treat_a_disjunct_as_implying_the_owner() {
    // `any(a, b)` holding does not guarantee `a`; only conjunction licenses the drop.
    assert_eq!(
        combine_gates(
            r#"feature = "client""#,
            r#"any(feature = "client", feature = "server")"#
        ),
        r#"all(feature = "client", any(feature = "client", feature = "server"))"#
    );
}

#[test]
fn collect_cfg_feature_names_simple_feature() {
    let mut out = BTreeSet::new();
    collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
    assert_eq!(out, BTreeSet::from(["pdf".to_string()]));
}

#[test]
fn collect_cfg_feature_names_any_compound() {
    let mut out = BTreeSet::new();
    collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
    let want: BTreeSet<String> = ["html", "xml"].into_iter().map(String::from).collect();
    assert_eq!(out, want);
}

#[test]
fn collect_cfg_feature_names_all_compound() {
    let mut out = BTreeSet::new();
    collect_cfg_feature_names(
        r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
        &mut out,
    );
    let want: BTreeSet<String> = ["layout-types", "wasm-target"].into_iter().map(String::from).collect();
    assert_eq!(out, want);
}

#[test]
fn parse_cfg_predicate_simple_feature() {
    assert_eq!(
        parse_cfg_predicate(r#"feature = "tokenizer""#),
        CfgPredicate::Feature("tokenizer".to_string())
    );
}

#[test]
fn parse_cfg_predicate_any_preserves_arms() {
    assert_eq!(
        parse_cfg_predicate(r#"any(feature = "native-http", feature = "wasm-http")"#),
        CfgPredicate::Any(vec![
            CfgPredicate::Feature("native-http".to_string()),
            CfgPredicate::Feature("wasm-http".to_string()),
        ])
    );
}

#[test]
fn parse_cfg_predicate_all_preserves_arms() {
    assert_eq!(
        parse_cfg_predicate(r#"all(feature = "layout-types", not(feature = "wasm-target"))"#),
        CfgPredicate::All(vec![
            CfgPredicate::Feature("layout-types".to_string()),
            CfgPredicate::Not(Box::new(CfgPredicate::Feature("wasm-target".to_string()))),
        ])
    );
}

#[test]
fn parse_cfg_predicate_not() {
    assert_eq!(
        parse_cfg_predicate(r#"not(feature = "wasm-target")"#),
        CfgPredicate::Not(Box::new(CfgPredicate::Feature("wasm-target".to_string())))
    );
}

#[test]
fn parse_cfg_predicate_unrecognised_is_other() {
    assert_eq!(parse_cfg_predicate(r#"target_arch = "wasm32""#), CfgPredicate::Other);
}

#[test]
fn collect_cfg_feature_names_ignores_non_feature_cfg() {
    let mut out = BTreeSet::new();
    collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
    assert!(out.is_empty());
}

#[test]
fn collect_cfg_feature_names_whitespace_normalisation() {
    let mut out = BTreeSet::new();
    collect_cfg_feature_names(r#"any (feature = "a" , feature = "b")"#, &mut out);
    let want: BTreeSet<String> = ["a", "b"].into_iter().map(String::from).collect();
    assert_eq!(out, want);
}

#[test]
fn collect_cfg_features_walks_types_enums_functions() {
    let mut out = BTreeSet::new();
    collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
    collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
    collect_cfg_feature_names(
        r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
        &mut out,
    );
    collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
    let want: BTreeSet<String> = ["html", "layout-types", "pdf", "wasm-target", "xml"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(out, want);
}

#[test]
fn any_group_feature_names_reads_a_two_arm_any_gate() {
    let want: BTreeSet<String> = ["engine-native", "engine-portable"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        any_group_feature_names(r#"any(feature = "engine-native", feature = "engine-portable")"#),
        Some(want)
    );
}

#[test]
fn any_group_feature_names_normalises_stringified_whitespace() {
    let want: BTreeSet<String> = ["a", "b"].into_iter().map(String::from).collect();
    assert_eq!(
        any_group_feature_names(r#"any (feature = "a" , feature = "b")"#),
        Some(want)
    );
}

#[test]
fn any_group_feature_names_rejects_a_single_feature_gate() {
    assert_eq!(any_group_feature_names(r#"feature = "engine-native""#), None);
}

#[test]
fn any_group_feature_names_rejects_an_all_gate() {
    assert_eq!(
        any_group_feature_names(r#"all(feature = "engine-native", feature = "engine-portable")"#),
        None
    );
}

#[test]
fn any_group_feature_names_ignores_a_nested_non_feature_arm() {
    // Only two bare `feature = "..."` arms are extracted; a third arm that is not a bare feature
    // (here nested inside `not(...)`) does not count toward the two-arm minimum on its own, but
    // does not block the two arms that do qualify either.
    let want: BTreeSet<String> = ["engine-native", "engine-portable"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        any_group_feature_names(
            r#"any(feature = "engine-native", feature = "engine-portable", not(target_arch = "wasm32"))"#
        ),
        Some(want)
    );
}

/// The relationship [`collect_cfg_feature_alternatives`] exists to expose: a source crate gates
/// one capability behind either of two sibling features, and a caller widening a binding's
/// literal Cargo feature list needs to discover that pairing from the parsed cfg strings instead
/// of a hard-coded feature name.
#[test]
fn collect_cfg_feature_alternatives_finds_an_any_gated_pair() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        types: vec![TypeDef {
            name: "Engine".to_string(),
            rust_path: "mylib::Engine".to_string(),
            cfg: Some(r#"any(feature = "engine-native", feature = "engine-portable")"#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let groups = collect_cfg_feature_alternatives(&api);
    let want: BTreeSet<String> = ["engine-native", "engine-portable"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(groups, vec![want]);
}

#[test]
fn collect_cfg_feature_alternatives_ignores_a_single_feature_gate() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        types: vec![TypeDef {
            name: "Engine".to_string(),
            rust_path: "mylib::Engine".to_string(),
            cfg: Some(r#"feature = "engine-native""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(collect_cfg_feature_alternatives(&api).is_empty());
}

#[test]
fn collect_cfg_features_full_surface_walk() {
    let api = ApiSurface {
        types: vec![TypeDef {
            name: "PdfDoc".to_string(),
            rust_path: "mylib::PdfDoc".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Native".to_string(),
                    cfg: None,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Heic".to_string(),
                    cfg: Some(r#"feature = "heic""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let features = collect_cfg_features(&api);
    let want: BTreeSet<String> = ["heic", "pdf"].into_iter().map(String::from).collect();
    assert_eq!(features, want);
}

#[test]
fn collect_cfg_features_includes_method_gates() {
    // A Rust-emitting backend re-emits a gated method's `#[cfg(feature = "X")]` into its own
    // crate, so `X` must reach that crate's `[features]` table or the build fails with
    // `unexpected cfg condition value`. ~keep
    use crate::core::ir::MethodDef;

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "mylib::Client".to_string(),
            methods: vec![
                MethodDef {
                    name: "ping".to_string(),
                    ..Default::default()
                },
                MethodDef {
                    name: "stream".to_string(),
                    cfg: Some(r#"feature = "streaming""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        enums: vec![EnumDef {
            name: "Format".to_string(),
            rust_path: "mylib::Format".to_string(),
            methods: vec![MethodDef {
                name: "from_mime".to_string(),
                cfg: Some(r#"all(feature = "mime", feature = "sniff")"#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let want: BTreeSet<String> = ["mime", "sniff", "streaming"].into_iter().map(String::from).collect();
    assert_eq!(collect_cfg_features(&api), want);
}

/// A Rust-emitting backend that re-emits a gated service's constructor or configurator gate
/// (mirroring how it already re-emits a gated method's) needs `X` declared in the manifest for
/// the same `unexpected cfg condition value` reason `collect_cfg_features_includes_method_gates`
/// covers for methods. `ServiceDef` and its `constructor`/`configurators` `MethodDef`s were
/// previously not walked at all, unlike `backends::ffi::gen_bindings::helpers::cbindgen_feature_defines`,
/// which already reads `ServiceDef::cfg` for the FFI header's `#if` guards — this test pins the
/// shared helper to the same coverage. ~keep
#[test]
fn collect_cfg_features_includes_service_and_configurator_gates() {
    use crate::core::ir::{MethodDef, ServiceDef};

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        services: vec![ServiceDef {
            name: "ClientConfig".to_string(),
            rust_path: "mylib::client::ClientConfig".to_string(),
            constructor: MethodDef {
                name: "new".to_string(),
                ..Default::default()
            },
            configurators: vec![
                MethodDef {
                    name: "with_timeout".to_string(),
                    ..Default::default()
                },
                MethodDef {
                    name: "with_tower_layer".to_string(),
                    cfg: Some(r#"feature = "tower""#.to_string()),
                    ..Default::default()
                },
            ],
            registrations: vec![],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        }],
        ..Default::default()
    };

    let want: BTreeSet<String> = ["tower".to_string()].into_iter().collect();
    assert_eq!(collect_cfg_features(&api), want);
}

/// A service merged from a foreign `[[crates.source_crates]]` crate must not forward its cfg
/// to the host crate's `[features]` table, for the same reason a merged type/enum doesn't
/// (see `collect_cfg_features_excludes_external_source_crate_cfgs`) — forwarding would
/// reference a feature the host crate does not define.
#[test]
fn collect_cfg_features_excludes_external_source_crate_service_cfgs() {
    use crate::core::ir::{MethodDef, ServiceDef};

    let api = ApiSurface {
        crate_name: "hostlib".to_string(),
        services: vec![ServiceDef {
            name: "OtherService".to_string(),
            rust_path: "otherlib::OtherService".to_string(),
            constructor: MethodDef::default(),
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![],
            doc: String::new(),
            cfg: Some(r#"feature = "foreign-only""#.to_string()),
        }],
        ..Default::default()
    };

    assert!(
        collect_cfg_features(&api).is_empty(),
        "a foreign-owned service's cfg must not forward to the host crate"
    );
}

#[test]
fn collect_cfg_features_excludes_external_source_crate_cfgs() {
    // A type/enum merged from `[[crates.source_crates]]` carries the foreign crate's rust_path ~keep
    // and cfg gates. Its features must NOT be forwarded to the host crate (they'd map to ~keep
    // `<host>/<feat>` for a feature the host does not define, breaking cargo resolution). ~keep
    let api = ApiSurface {
        crate_name: "hostlib".to_string(),
        types: vec![TypeDef {
            name: "HostDoc".to_string(),
            rust_path: "hostlib::HostDoc".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        enums: vec![EnumDef {
            name: "Strategy".to_string(),
            rust_path: "otherlib::Strategy".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Auto".to_string(),
                    cfg: None,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Advanced".to_string(),
                    cfg: Some(r#"any(test, feature = "foreign-only")"#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let features = collect_cfg_features(&api);
    assert_eq!(
        features,
        BTreeSet::from(["pdf".to_string()]),
        "host `pdf` must forward; the foreign `foreign-only` feature must not leak into host passthrough"
    );
}

/// [`is_host_owned_rust_path`] is the single authority `collect_cfg_gates` and backend arm/
/// variant emitters (`gen_enum_from_i32_rs_helper`, the Swift enum wrapper) both call to decide
/// whether a cfg-gated item's cfg is safe to re-emit verbatim. Pinning its own behavior here
/// keeps `collect_cfg_features_excludes_external_source_crate_cfgs`'s coverage of the
/// feature-collection side from silently diverging from the emitter side.
#[test]
fn is_host_owned_rust_path_matches_the_leading_path_segment() {
    assert!(is_host_owned_rust_path("hostlib", "hostlib::Strategy"));
    assert!(!is_host_owned_rust_path("hostlib", "otherlib::Strategy"));
}

#[test]
fn is_host_owned_rust_path_normalises_dashes_in_the_host_crate_name() {
    assert!(is_host_owned_rust_path("host-lib", "host_lib::Strategy"));
}

#[test]
fn is_host_owned_rust_path_is_permissive_for_an_unknown_host_crate_name() {
    assert!(
        is_host_owned_rust_path("", "otherlib::Strategy"),
        "an unknown (empty) host crate name must not falsely mark everything foreign"
    );
}

/// An unqualified `rust_path` (no `::`) is only permissive by accident of comparing an empty
/// first segment; a non-empty single segment is compared against the host crate name like any
/// other leading segment, so it reads as foreign unless it happens to equal the host name.
#[test]
fn is_host_owned_rust_path_compares_a_bare_name_like_any_leading_segment() {
    assert!(is_host_owned_rust_path("hostlib", "hostlib"));
    assert!(!is_host_owned_rust_path("hostlib", "Strategy"));
}

fn api_with_gated_functions(names_and_cfgs: &[(&str, Option<&str>)]) -> ApiSurface {
    use crate::core::ir::FunctionDef;
    ApiSurface {
        crate_name: "test_lib".to_string(),
        functions: names_and_cfgs
            .iter()
            .map(|(name, cfg)| FunctionDef {
                name: (*name).to_string(),
                rust_path: format!("test_lib::{name}"),
                cfg: cfg.map(|s| s.to_string()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn undeclared_cfg_features_returns_only_missing_names() {
    let api = api_with_gated_functions(&[
        ("count_tokens", Some(r#"feature = "tokenizer""#)),
        ("with_tower_layer", Some(r#"feature = "tower""#)),
        ("completion_cost", None),
    ]);
    let declared: BTreeSet<String> = ["tower".to_string()].into_iter().collect();
    assert_eq!(
        undeclared_cfg_features(&api, &declared),
        BTreeSet::from(["tokenizer".to_string()]),
        "only the feature missing from `declared` should be reported"
    );
}

#[test]
fn undeclared_cfg_features_empty_when_everything_declared() {
    let api = api_with_gated_functions(&[("count_tokens", Some(r#"feature = "tokenizer""#))]);
    let declared: BTreeSet<String> = ["tokenizer".to_string()].into_iter().collect();
    assert!(undeclared_cfg_features(&api, &declared).is_empty());
}

#[test]
fn read_declared_cargo_features_none_when_file_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    assert!(
        read_declared_cargo_features(&manifest).is_none(),
        "an unscaffolded crate has nothing to verify against yet"
    );
}

#[test]
fn read_declared_cargo_features_reads_features_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"x\"\n\n[features]\ndefault = [\"native-http\"]\nnative-http = [\"core/native-http\"]\n",
    )
    .expect("write manifest");
    assert_eq!(
        read_declared_cargo_features(&manifest),
        Some(BTreeSet::from(["default".to_string(), "native-http".to_string()]))
    );
}

#[test]
fn read_declared_cargo_features_empty_set_when_no_features_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(&manifest, "[package]\nname = \"x\"\n").expect("write manifest");
    assert_eq!(
        read_declared_cargo_features(&manifest),
        Some(BTreeSet::new()),
        "a manifest scaffolded before any cfg-gated symbol existed has no [features] table at \
         all, which must still be distinguishable from a missing file"
    );
}

/// Reproduces the sample-core incident: the binding crate's Cargo.toml declares only the
/// features it had at scaffold time (`native-http`), but the generated source now references
/// `tokenizer` because the core crate gained a cfg-gated function since then.
#[traced_test]
#[test]
fn warn_on_undeclared_binding_cfg_features_warns_when_feature_is_undeclared() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"x\"\n\n[features]\ndefault = [\"native-http\"]\nnative-http = [\"core/native-http\"]\n",
    )
    .expect("write manifest");
    let api = api_with_gated_functions(&[("count_tokens", Some(r#"feature = "tokenizer""#))]);

    warn_on_undeclared_binding_cfg_features(&api, Language::Ruby, &manifest);

    assert!(
        logs_contain("does not enable by default"),
        "a stale manifest missing a referenced feature must produce a warning"
    );
}

/// Reproduces the actual liter-llm-style incident this fix targets: an earlier repair pass (or a
/// hand-edit) added the forwarding row (`tokenizer = ["core/tokenizer"]`) but never added
/// `tokenizer` to `default`. A manifest in exactly this shape must still warn -- declaring a
/// feature is not the same as turning it on -- which is the whole reason the check now keys on
/// [`read_default_enabled_cargo_features`] instead of [`read_declared_cargo_features`].
#[traced_test]
#[test]
fn warn_on_undeclared_binding_cfg_features_warns_when_declared_but_not_in_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"x\"\n\n[features]\ndefault = [\"native-http\"]\n\
         native-http = [\"core/native-http\"]\ntokenizer = [\"core/tokenizer\"]\n",
    )
    .expect("write manifest");
    let api = api_with_gated_functions(&[("count_tokens", Some(r#"feature = "tokenizer""#))]);

    warn_on_undeclared_binding_cfg_features(&api, Language::Ruby, &manifest);

    assert!(
        logs_contain("does not enable by default"),
        "a feature that is declared but unreachable from `default` must still warn -- \
         `cargo rustc --print cfg` proves it is not actually on"
    );
}

#[traced_test]
#[test]
fn warn_on_undeclared_binding_cfg_features_silent_when_manifest_declares_everything() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"x\"\n\n[features]\ndefault = [\"tokenizer\"]\ntokenizer = [\"core/tokenizer\"]\n",
    )
    .expect("write manifest");
    let api = api_with_gated_functions(&[("count_tokens", Some(r#"feature = "tokenizer""#))]);

    warn_on_undeclared_binding_cfg_features(&api, Language::Ruby, &manifest);

    assert!(
        !logs_contain("does not enable by default"),
        "a fully up-to-date manifest must not warn"
    );
}

#[traced_test]
#[test]
fn warn_on_undeclared_binding_cfg_features_silent_when_manifest_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("Cargo.toml");
    let api = api_with_gated_functions(&[("count_tokens", Some(r#"feature = "tokenizer""#))]);

    warn_on_undeclared_binding_cfg_features(&api, Language::Ruby, &manifest);

    assert!(
        !logs_contain("does not enable by default"),
        "an unscaffolded crate (no manifest on disk yet) must not warn -- there is nothing to \
         verify against"
    );
}

fn resolved_config(toml_text: &str) -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(toml_text).expect("valid config");
    cfg.resolve().expect("resolve").remove(0)
}

/// [`effective_ffi_default_features`] must union the FFI language's own configured feature list
/// with every feature name discovered by walking the API surface's cfg gates, and it must
/// exclude `extra_features` (declare-only, never defaulted) from that union even when the same
/// name is also discovered from a gate.
#[test]
fn effective_ffi_default_features_unions_configured_and_cfg_discovered_names() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["shared"]
extra_features = ["wasm-http"]
"#,
    );
    let api = api_with_gated_functions(&[
        ("configured_only", None),
        ("discovered_gate", Some(r#"feature = "sample-gate""#)),
        ("declare_only_gate", Some(r#"feature = "wasm-http""#)),
    ]);

    let effective: BTreeSet<String> = effective_ffi_default_features(&api, &config).into_iter().collect();

    assert_eq!(
        effective,
        BTreeSet::from(["shared".to_string(), "sample-gate".to_string()]),
        "must include the configured FFI feature and the discovered gate, but exclude the \
         declare-only extra_features entry even though it is also gated on"
    );
}

/// Table-driven coverage for [`merge_missing_cfg_features`]: whether it patches an existing
/// binding manifest's `[features]` table, and what it must never touch while doing so.
struct MergeCase {
    name: &'static str,
    existing: &'static str,
    gated_functions: &'static [(&'static str, Option<&'static str>)],
    core_declared_features: &'static [&'static str],
    /// Substrings the patched output must contain. Empty when [`Self::expect_none`] is `true`.
    expect_contains: &'static [&'static str],
    /// Substrings that must survive verbatim -- proof the merge is additive-only, not a
    /// regeneration. Checked against `existing` when [`Self::expect_none`] is `true`.
    expect_preserved: &'static [&'static str],
    /// `true` when nothing referenced is missing (or nothing missing may be invented), so the
    /// function must return `Ok(None)` rather than rewrite the file.
    expect_none: bool,
}

#[test]
fn merge_missing_cfg_features_table() {
    let cases = [
        MergeCase {
            name: "a manifest missing one gated feature gets it added",
            existing: "[package]\nname = \"x\"\n\n[features]\n\
                       default = [\"native-http\"]\n\
                       native-http = [\"core/native-http\"]\n\n\
                       [dependencies]\nserde = \"1\"\n",
            gated_functions: &[("count_tokens", Some(r#"feature = "tokenizer""#))],
            core_declared_features: &["native-http", "tokenizer"],
            expect_contains: &[
                r#"tokenizer = ["core/tokenizer"]"#,
                r#"default = ["native-http", "tokenizer"]"#,
            ],
            expect_preserved: &[
                r#"native-http = ["core/native-http"]"#,
                "[dependencies]",
                "serde = \"1\"",
            ],
            expect_none: false,
        },
        MergeCase {
            name: "a manifest already declaring and defaulting the feature is left unchanged",
            existing: "[package]\nname = \"x\"\n\n[features]\n\
                       default = [\"tokenizer\"]\n\
                       tokenizer = [\"core/tokenizer\"]\n",
            gated_functions: &[("count_tokens", Some(r#"feature = "tokenizer""#))],
            core_declared_features: &["tokenizer"],
            expect_contains: &[],
            expect_preserved: &[r#"tokenizer = ["core/tokenizer"]"#],
            expect_none: true,
        },
        MergeCase {
            name: "a feature declared but excluded from default is added to default, not redeclared",
            existing: "[package]\nname = \"x\"\n\n[features]\n\
                       default = [\"native-http\"]\n\
                       native-http = [\"core/native-http\"]\n\
                       tokenizer = [\"core/tokenizer\"]\n",
            gated_functions: &[("count_tokens", Some(r#"feature = "tokenizer""#))],
            core_declared_features: &["native-http", "tokenizer"],
            expect_contains: &[r#"default = ["native-http", "tokenizer"]"#],
            expect_preserved: &[
                r#"tokenizer = ["core/tokenizer"]"#,
                r#"native-http = ["core/native-http"]"#,
            ],
            expect_none: false,
        },
        MergeCase {
            name: "a manifest with no [features] table at all gets one created, feature enabled by default",
            existing: "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
            gated_functions: &[("count_tokens", Some(r#"feature = "tokenizer""#))],
            core_declared_features: &["tokenizer"],
            expect_contains: &[
                "[features]",
                r#"tokenizer = ["core/tokenizer"]"#,
                r#"default = ["tokenizer"]"#,
            ],
            expect_preserved: &["[dependencies]", "serde = \"1\""],
            expect_none: false,
        },
        MergeCase {
            name: "a gated feature the core crate does not declare is never invented or defaulted",
            existing: "[package]\nname = \"x\"\n\n[features]\ndefault = []\n",
            gated_functions: &[("count_tokens", Some(r#"feature = "tokenizer""#))],
            core_declared_features: &[],
            expect_contains: &[],
            expect_preserved: &["default = []"],
            expect_none: true,
        },
    ];

    for case in cases {
        let api = api_with_gated_functions(case.gated_functions);
        let core_declared: BTreeSet<String> = case.core_declared_features.iter().map(|s| s.to_string()).collect();

        let result = merge_missing_cfg_features(case.existing, &api, "core", &core_declared)
            .unwrap_or_else(|error| panic!("case `{}`: merge failed: {error}", case.name));

        if case.expect_none {
            assert!(
                result.is_none(),
                "case `{}`: expected no change, got:\n{:?}",
                case.name,
                result
            );
            for needle in case.expect_preserved {
                assert!(
                    case.existing.contains(needle),
                    "case `{}`: control assertion failed -- `{needle}` must be in the original fixture",
                    case.name
                );
            }
            continue;
        }

        let patched = result.unwrap_or_else(|| panic!("case `{}`: expected a patch, got None", case.name));
        for needle in case.expect_contains {
            assert!(
                patched.contains(needle),
                "case `{}`: patched manifest missing `{needle}`, got:\n{patched}",
                case.name
            );
        }
        for needle in case.expect_preserved {
            assert!(
                patched.contains(needle),
                "case `{}`: patch dropped or reformatted `{needle}`, got:\n{patched}",
                case.name
            );
        }
    }
}

/// Beyond substring checks: every line of a manifest untouched by the merge must survive
/// byte-for-byte, including comments and a section the merge has no knowledge of -- proof this
/// is a surgical insert, not a parse/reserialize that happens to look similar.
#[test]
fn merge_missing_cfg_features_preserves_untouched_lines_verbatim() {
    let existing = "# top-of-file comment, hand-written\n\
                     [package]\n\
                     name = \"x\"\n\
                     version = \"0.1.0\"\n\n\
                     [features]\n\
                     # a feature comment the user wrote\n\
                     default = [\"native-http\"]\n\
                     native-http = [\"core/native-http\"]\n\n\
                     [package.metadata.cargo-machete]\n\
                     ignored = [\"rb-sys\"]\n";
    let api = api_with_gated_functions(&[("count_tokens", Some(r#"feature = "tokenizer""#))]);
    let core_declared: BTreeSet<String> = ["native-http", "tokenizer"].into_iter().map(String::from).collect();

    let patched = merge_missing_cfg_features(existing, &api, "core", &core_declared)
        .expect("merge must succeed")
        .expect("a missing feature must produce a patch");

    // The `default` line is the one deliberate exception: enabling the backfilled feature is the
    // point of this merge, so its own value array is expected to change. ~keep
    for line in existing.lines().filter(|line| !line.starts_with("default = ")) {
        assert!(
            patched.contains(line),
            "line `{line}` from the original manifest did not survive the merge verbatim, got:\n{patched}"
        );
    }
    assert!(
        patched.contains(r#"tokenizer = ["core/tokenizer"]"#),
        "the missing feature must still be declared, got:\n{patched}"
    );
    assert!(
        patched.contains(r#"default = ["native-http", "tokenizer"]"#),
        "the backfilled feature must also be enabled by default, not merely declared, got:\n{patched}"
    );
}

/// [`core_crate_declared_features`] must read the core crate's own `[features]` table off disk,
/// resolved the same way [`crate::scaffold::core_crate_manifest_path`] locates it, and must fall
/// back to an empty set (never a panic or a guess) when the manifest cannot be found.
#[test]
fn core_crate_declared_features_reads_the_core_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws_root = dir.path();
    let core_dir = ws_root.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\n\n[features]\ndefault = []\ntokenizer = []\ntower = []\n",
    )
    .expect("write core Cargo.toml");

    let config = crate::core::config::ResolvedCrateConfig {
        workspace_root: Some(ws_root.to_path_buf()),
        name: "my-lib".to_string(),
        sources: vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")],
        ..Default::default()
    };

    assert_eq!(
        core_crate_declared_features(&config),
        BTreeSet::from(["default".to_string(), "tokenizer".to_string(), "tower".to_string()])
    );
}

#[test]
fn core_crate_declared_features_empty_when_workspace_root_unset() {
    let config = crate::core::config::ResolvedCrateConfig::default();
    assert!(core_crate_declared_features(&config).is_empty());
}

/// [`expand_configured_features`] must follow the core crate's own `[features]` table so a
/// configured aggregate name also stands for every member it enables -- transitively, through a
/// nested aggregate -- while leaving unrelated names alone.
///
/// Without this, `cfg_feature_satisfied` (which matches literally and hard-codes only `full` as a
/// universal umbrella) reports every `#[cfg(feature = "<member>")]` gate unsatisfied for a
/// binding configured with the aggregate, and the gated items vanish from that surface with no
/// diagnostic even though cargo compiles them.
#[test]
fn expand_configured_features_follows_the_core_crates_feature_graph() {
    let dir = tempfile::tempdir().expect("tempdir");
    let core_dir = dir.path().join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\n\n[features]\ndefault = []\n\
         mobile-target = [\"tokenizer\", \"text\"]\ntext = [\"markup\"]\nmarkup = []\n\
         tokenizer = [\"dep:tok\", \"other-crate/tok\"]\nunrelated = []\n",
    )
    .expect("write core Cargo.toml");
    let config = crate::core::config::ResolvedCrateConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        name: "my-lib".to_string(),
        sources: vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")],
        ..Default::default()
    };

    let expanded: BTreeSet<String> = expand_configured_features(&config, &["mobile-target".to_string()])
        .into_iter()
        .collect();

    assert_eq!(
        expanded,
        BTreeSet::from([
            "mobile-target".to_string(),
            "tokenizer".to_string(),
            "text".to_string(),
            "markup".to_string(),
        ]),
        "the aggregate, its direct members, and the nested aggregate's members must all count as \
         enabled; `dep:` and `crate/feature` tokens are not local feature names, and an unrelated \
         feature must not be swept in"
    );
}

/// A manifest that cannot be located must leave the requested list exactly as it was: widening
/// the enabled set is only safe when the manifest proves the members are on.
#[test]
fn expand_configured_features_passes_through_when_the_manifest_is_unreachable() {
    let config = crate::core::config::ResolvedCrateConfig::default();
    assert_eq!(
        expand_configured_features(&config, &["tokenizer".to_string()]),
        vec!["tokenizer".to_string()],
        "no workspace_root means no manifest to read, so the list must pass through untouched"
    );
}
