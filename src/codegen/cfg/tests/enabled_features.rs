//! Coverage for [`core_default_features_active`] and [`enabled_features_for_language`].
//!
//! Split out of the parent test module so both files stay under the `file-modularization` cap.
//!
//! Regression for the E0004 hazard `expand_configured_features` used to leave open: a core crate
//! whose `[features] default = [...]` enables a feature that gates a FOREIGN cfg enum variant, and
//! whose binding `alef.toml` never names that feature explicitly, must still count that feature as
//! active for every language whose Cargo dependency edge to the core crate does not suppress
//! defaults (every language except R and WASM, each of which suppresses on its own condition --
//! see `core_default_features_active`'s doc). Getting this backward in either direction is a real
//! bug: undercounting drops a reachable variant's catch-all and produces a non-exhaustive match
//! (`error[E0004]`) the moment cargo actually turns the feature on; overcounting (treating a
//! feature as active when the binding's Cargo.toml genuinely never turns it on) would resurrect
//! the unreachable-catch-all warning noise the 0.72.0 cfg-reachability work was written to remove.

use crate::codegen::cfg::{core_default_features_active, enabled_features_for_language};
use crate::core::config::{Language, RConfig, ResolvedCrateConfig};
use std::collections::BTreeSet;

/// Write a minimal core crate manifest with the given `[features]` body under
/// `<dir>/crates/my-lib/Cargo.toml`, and return a `ResolvedCrateConfig` pointed at it.
fn config_with_core_features(dir: &std::path::Path, features_body: &str, r: Option<RConfig>) -> ResolvedCrateConfig {
    let core_dir = dir.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        format!("[package]\nname = \"my-lib\"\n\n[features]\n{features_body}"),
    )
    .expect("write core Cargo.toml");

    ResolvedCrateConfig {
        workspace_root: Some(dir.to_path_buf()),
        name: "my-lib".to_string(),
        sources: vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")],
        r,
        ..Default::default()
    }
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

/// [`core_default_features_active`] must be unconditionally `true` for every language but R and
/// WASM: none of the rest has a base-line (non-per-target-override) knob that can suppress the
/// core crate's own `default-features`. This is the authority [`enabled_features_for_language`]
/// asks before unioning the core crate's declared defaults in, so it must agree with what
/// `scaffold::render_core_dep`/`render_core_dep_with_overrides` actually emit for the base branch.
///
/// WASM is absent from this list on purpose: `backends::wasm::gen_bindings::cargo::gen_cargo_toml`
/// builds its own manifest rather than going through `scaffold::render_core_dep`, and emits
/// `default-features = false` on the base line whenever a wasm feature list is configured. ~keep
#[test]
fn core_default_features_active_is_unconditionally_true_outside_r_and_wasm() {
    let config = ResolvedCrateConfig::default();
    for lang in [
        Language::Go,
        Language::Java,
        Language::Kotlin,
        Language::KotlinAndroid,
        Language::Csharp,
        Language::Python,
        Language::Ruby,
        Language::Node,
        Language::Php,
        Language::Elixir,
        Language::Zig,
        Language::Dart,
        Language::Swift,
        Language::Ffi,
    ] {
        assert!(
            core_default_features_active(&config, lang),
            "{lang:?}'s base core dependency edge never emits `default-features = false`, so its \
             defaults must always read as active"
        );
    }
}

/// R's `default_features` flag is `None`/unset (the common case, and the same default
/// `scaffold_r_cargo` assumes via `unwrap_or(true)`): defaults stay active.
#[test]
fn core_default_features_active_for_r_defaults_to_true_when_unset() {
    let config = ResolvedCrateConfig {
        r: Some(RConfig {
            dependency_versions: Default::default(),
            features: Some(vec!["curated".to_string()]),
            default_features: None,
            ..r_config_defaults()
        }),
        ..Default::default()
    };
    assert!(core_default_features_active(&config, Language::R));
}

/// `[crates.r] default_features = false` with an EMPTY configured feature list must still read as
/// active: `scaffold_r_cargo` keeps the plain (defaults-active) dependency line whenever
/// `features_for_language(Language::R)` is empty, regardless of the flag -- there is nothing to
/// put in defaults' place on the emitted Cargo.toml line, so treating them as suppressed here
/// would disagree with what actually got scaffolded.
#[test]
fn core_default_features_active_for_r_stays_true_with_no_replacement_features() {
    let config = ResolvedCrateConfig {
        r: Some(RConfig {
            dependency_versions: Default::default(),
            features: Some(vec![]),
            default_features: Some(false),
            ..r_config_defaults()
        }),
        ..Default::default()
    };
    assert!(core_default_features_active(&config, Language::R));
}

/// The one real suppression case: `default_features = false` AND a non-empty configured list --
/// `scaffold_r_cargo` emits `default-features = false, features = [...]` on this exact
/// combination, so defaults must read as inactive.
#[test]
fn core_default_features_active_for_r_is_false_with_default_features_disabled_and_a_replacement_list() {
    let config = ResolvedCrateConfig {
        r: Some(RConfig {
            dependency_versions: Default::default(),
            features: Some(vec!["curated".to_string()]),
            default_features: Some(false),
            ..r_config_defaults()
        }),
        ..Default::default()
    };
    assert!(!core_default_features_active(&config, Language::R));
}

fn r_config_defaults() -> RConfig {
    RConfig {
        dependency_versions: Default::default(),
        package_name: None,
        features: None,
        default_features: None,
        serde_rename_all: None,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        rename_fields: std::collections::HashMap::new(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_makevars_prelude: Vec::new(),
        extra_pkg_libs: Vec::new(),
    }
}

/// THE E0004 REGRESSION: a foreign cfg gate satisfied only by the core crate's own declared
/// `default = [...]` -- never named in the binding's own configured feature list at all -- must
/// still count as enabled. Before this fix, `expand_configured_features` was handed
/// `features_for_language` alone, so a variant gated on `feature = "extended-mode"` here would be wrongly
/// "proven unreachable" by `enum_conversion_needs_catch_all_for_features`, its catch-all dropped,
/// and the generated match left non-exhaustive the moment cargo (which really does enable `extended-mode`
/// via the core crate's own default) compiled the real variant in.
#[test]
fn enabled_features_for_language_includes_a_core_default_the_binding_never_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_with_core_features(dir.path(), "default = [\"extended-mode\"]\nextended-mode = []\n", None);

    let enabled: BTreeSet<String> = enabled_features_for_language(&config, Language::Go)
        .into_iter()
        .collect();

    assert_eq!(
        enabled,
        set(&["extended-mode"]),
        "a core-declared default feature must be counted as active even though this binding's own \
         `alef.toml` never names it, got: {enabled:?}"
    );
}

/// A core-crate aggregate named only inside `default = [...]` (not as a leaf the binding
/// configured) must still expand to its transitive members, exactly like an aggregate the binding
/// configures directly -- the union with defaults happens before expansion, not instead of it.
#[test]
fn enabled_features_for_language_expands_an_aggregate_reachable_only_through_core_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_with_core_features(
        dir.path(),
        "default = [\"mobile-target\"]\nmobile-target = [\"tokenizer\"]\ntokenizer = []\n",
        None,
    );

    let enabled: BTreeSet<String> = enabled_features_for_language(&config, Language::Dart)
        .into_iter()
        .collect();

    assert_eq!(
        enabled,
        set(&["mobile-target", "tokenizer"]),
        "the aggregate named in core's own defaults, and its transitive member, must both count \
         as enabled, got: {enabled:?}"
    );
}

/// THE OTHER DIRECTION: a feature genuinely never turned on -- R with `default_features = false`
/// and a configured replacement list that does not include it -- must still be proven unreachable.
/// A fix that unconditionally unions in core defaults regardless of `core_default_features_active`
/// would make this assert fail, reintroducing the unreachable-catch-all warning noise the
/// reachability work this defends was written to remove.
#[test]
fn enabled_features_for_language_does_not_resurrect_a_feature_r_genuinely_suppressed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_with_core_features(
        dir.path(),
        "default = [\"extended-mode\"]\nextended-mode = []\ncurated = []\n",
        Some(RConfig {
            dependency_versions: Default::default(),
            features: Some(vec!["curated".to_string()]),
            default_features: Some(false),
            ..r_config_defaults()
        }),
    );

    let enabled: BTreeSet<String> = enabled_features_for_language(&config, Language::R)
        .into_iter()
        .collect();

    assert_eq!(
        enabled,
        set(&["curated"]),
        "`extended-mode` is a core default this R binding's Cargo.toml explicitly suppresses -- it must \
         stay absent, got: {enabled:?}"
    );
}

/// Write the same minimal core crate manifest as [`config_with_core_features`], but resolve the
/// config from TOML so a real `[crates.wasm]` table can be attached ([`WasmConfig`] has no
/// `Default`, so the struct-literal helper above cannot express one).
///
/// [`WasmConfig`]: crate::core::config::WasmConfig
fn wasm_config_with_core_features(
    dir: &std::path::Path,
    features_body: &str,
    wasm_table_body: &str,
) -> ResolvedCrateConfig {
    let core_dir = dir.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        format!("[package]\nname = \"my-lib\"\n\n[features]\n{features_body}"),
    )
    .expect("write core Cargo.toml");

    let toml_src = format!(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "my-lib"
sources = ["crates/my-lib/src/lib.rs"]
workspace_root = "{root}"
[crates.wasm]
{wasm_table_body}
"#,
        // TOML strings treat `\` as an escape, so a Windows temp path must be forward-slashed. ~keep
        root = dir.display().to_string().replace('\\', "/"),
    );
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&toml_src).expect("valid alef config");
    cfg.resolve().expect("resolve").remove(0)
}

/// WASM's suppression case, the mirror of R's: `backends::wasm::gen_bindings::cargo::gen_cargo_toml`
/// emits `default-features = false, features = [...]` on the core dep the moment
/// `features_for_language(Language::Wasm)` is non-empty, so the core crate's declared defaults are
/// genuinely off and must read as inactive here.
#[test]
fn core_default_features_active_for_wasm_is_false_once_a_feature_list_replaces_the_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = wasm_config_with_core_features(
        dir.path(),
        "default = [\"native-http\"]\nnative-http = []\nwasm-http = []\n",
        "features = [\"wasm-http\"]",
    );
    assert!(!core_default_features_active(&config, Language::Wasm));
}

/// The other side of the wasm branch: with nothing configured, `gen_cargo_toml` writes a plain
/// `{ path = "..." }` dependency line, Cargo's own `default-features = true` applies, and the core
/// crate's defaults really are active. Narrowing wasm unconditionally would fail here.
#[test]
fn core_default_features_active_for_wasm_stays_true_with_no_replacement_features() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = wasm_config_with_core_features(
        dir.path(),
        "default = [\"native-http\"]\nnative-http = []\nwasm-http = []\n",
        "",
    );
    assert!(core_default_features_active(&config, Language::Wasm));
}

/// THE MIO REGRESSION, at the level of the predicate the manifest is derived from: a core crate
/// declaring `default = ["native-http"]` whose wasm binding replaced that with `wasm-http` must
/// not report `native-http` (nor `telemetry`, reachable only through the same defaults) as
/// enabled. `backends::wasm::gen_bindings::cargo::gen_cargo_toml` intersects this result with the
/// cfg-referenced names to build the wasm crate's own `default = [...]`, so anything leaking here
/// is switched straight back on by a forwarding row -- which is how tokio's native `net`/`mio`
/// stack reached a wasm32 build. ~keep
#[test]
fn enabled_features_for_language_does_not_resurrect_a_core_default_the_wasm_dep_line_suppressed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = wasm_config_with_core_features(
        dir.path(),
        "default = [\"native-http\", \"telemetry\"]\nnative-http = []\nwasm-http = []\ntelemetry = []\n",
        "features = [\"wasm-http\"]",
    );

    let enabled: BTreeSet<String> = enabled_features_for_language(&config, Language::Wasm)
        .into_iter()
        .collect();

    assert_eq!(
        enabled,
        set(&["wasm-http"]),
        "the wasm dep line already turned the core defaults off; counting them as active hands \
         `gen_cargo_toml` a `default = [...]` that turns them back on, got: {enabled:?}"
    );
}

/// NATIVE CONTROL, on the exact same config object as the test above -- the only difference is the
/// language asked. Every native binding's base dependency edge leaves the core defaults on, so
/// `native-http` and `telemetry` must both still count as enabled. A fix that suppressed core
/// defaults globally, or dropped the union in `enabled_features_from` outright, would pass the
/// wasm assertion and fail here: that is what makes this a per-target divergence rather than a
/// downgrade. ~keep
#[test]
fn enabled_features_for_language_still_unions_core_defaults_for_native_languages() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = wasm_config_with_core_features(
        dir.path(),
        "default = [\"native-http\", \"telemetry\"]\nnative-http = []\nwasm-http = []\ntelemetry = []\n",
        "features = [\"wasm-http\"]",
    );

    for lang in [Language::Ffi, Language::Node, Language::Python, Language::Go] {
        let enabled: BTreeSet<String> = enabled_features_for_language(&config, lang).into_iter().collect();
        assert_eq!(
            enabled,
            set(&["native-http", "telemetry"]),
            "{lang:?} keeps the core crate's defaults on its dependency edge, so both must read \
             as enabled, got: {enabled:?}"
        );
    }
}
