//! Feature-gate helpers for generated swift-bridge crates.

use crate::codegen::cfg::{collect_cfg_feature_alternatives, collect_cfg_features};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::collections::{BTreeSet, HashSet};

/// Check whether the umbrella source crate exposes the given feature name in its
/// on-disk Cargo.toml.
pub(crate) fn source_crate_has_feature(config: &ResolvedCrateConfig, core_crate_dir: &str, feature: &str) -> bool {
    let root = match config.workspace_root.as_deref() {
        Some(p) => p.to_path_buf(),
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(_) => return false,
        },
    };
    let cargo_toml = root.join("crates").join(core_crate_dir).join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&cargo_toml) else {
        return false;
    };
    let needle_line_start = format!("{feature} =");
    let mut in_features = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && trimmed.starts_with(&needle_line_start) {
            return true;
        }
    }
    false
}

/// Widen `config`'s configured Swift feature list with any sibling feature the source crate
/// itself pairs with an already-active one.
///
/// A source crate may gate one capability behind either of two (or more) sibling Cargo features
/// (see [`collect_cfg_feature_alternatives`]) rather than nesting one feature inside another's own
/// feature list — for example, a full-dependency feature and a narrower one that swaps in a
/// substitute dependency compatible with cross-compiled targets. When the configured feature list
/// activates one side of such a pairing (directly, or via the `full` umbrella), the other side is
/// included too, but only when the source crate's on-disk `Cargo.toml` actually declares it — so a
/// crate that does not use this pattern for a given feature never has an unknown name injected.
///
/// A companion named in [`SwiftConfig::excluded_default_features`][excl] is never widened in, even
/// when its sibling in the `any(...)` group is active. This return value feeds straight into the
/// core dependency's own `features = [...]` line in the generated `Cargo.toml` (see
/// `cargo::emit_cargo_toml`'s `features` parameter) on a dependency edge that does **not** set
/// `default-features = false` — unlike [`effective_swift_codegen_features`], whose
/// `excluded_default_features` check only trims the wrapper crate's *own* `default = [...]` array.
/// Without this check, a group such as `any(feature = "ocr", feature = "heic")` (a shared gate
/// with no real alternation relationship — both are simply required by one item, not substitutes
/// for each other) would explicitly request `core/heic` the moment `ocr` is configured, activating
/// a pkg-config-only native dependency `excluded_default_features = ["heic"]` was written
/// specifically to keep off cross-compiled targets. Measured against a synthetic fixture workspace
/// (`cargo tree -e features`): a dependency line that lists an opt-in sibling feature explicitly
/// activates that feature's own optional dependency even though `default-features` is left on,
/// because Cargo does not otherwise turn on an opt-in feature nobody asked for — the widening loop
/// is a genuine new activation, not something feature unification would have done anyway. ~keep
///
/// [excl]: crate::core::config::languages::SwiftConfig::excluded_default_features
pub(crate) fn configured_swift_features(
    config: &ResolvedCrateConfig,
    core_crate_dir: &str,
    api: &ApiSurface,
) -> Vec<String> {
    let base_features = config.features_for_language(Language::Swift);
    let excluded: BTreeSet<&str> = config
        .swift
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut features: BTreeSet<String> = base_features.iter().cloned().collect();
    let full_active = features.contains("full");
    for group in collect_cfg_feature_alternatives(api) {
        if !full_active && group.is_disjoint(&features) {
            continue;
        }
        for companion in &group {
            if excluded.contains(companion.as_str()) {
                continue;
            }
            if !features.contains(companion) && source_crate_has_feature(config, core_crate_dir, companion) {
                features.insert(companion.clone());
            }
        }
    }
    features.into_iter().collect()
}

pub(crate) fn effective_swift_codegen_features(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    core_crate_dir: &str,
) -> Vec<String> {
    let mut features: BTreeSet<String> = configured_swift_features(config, core_crate_dir, api)
        .into_iter()
        .collect();
    let excluded: HashSet<&str> = config
        .swift
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    // A configured component's features gate implementation types the downloaded producer cdylib
    // compiles, not anything this binding wraps; see
    // `codegen::cfg::native_wrapper_default_features_for_config`. ~keep
    let component_features = crate::codegen::cfg::component_core_features(config);
    for feature in collect_cfg_features(api) {
        if !excluded.contains(feature.as_str()) && !component_features.contains(&feature) {
            features.insert(feature);
        }
    }
    features.into_iter().collect()
}

/// Returns `true` when the `cfg` condition is satisfied by `configured_features`.
///
/// Thin wrapper over [`crate::core::ir::cfg_feature_satisfied`] so the Rust
/// bridge crate and the high-level Swift facade share one cfg-matching
/// implementation (keeping their `visible_*` sets in lockstep).
pub(super) fn cfg_satisfied(cfg: Option<&str>, configured_features: &HashSet<&str>) -> bool {
    crate::core::ir::cfg_feature_satisfied(cfg, configured_features)
}

/// Return a copy of `api` whose types', enums' and errors' methods are restricted to those whose
/// `#[cfg(...)]` gate `configured_features` satisfies.
///
/// The sibling of the `visible_types`/`visible_enums`/`visible_functions` filters, one level
/// deeper. Swift cannot express the gate on either side it emits: an `extern "Rust"` declaration
/// inside `#[swift_bridge::bridge]` is a macro input, not a free-standing item that can carry
/// `#[cfg]` per entry (which is why a gated *type* wraps its whole extern block instead), and the
/// Swift facade has no conditional compilation at all. So a gated method has to be dropped, not
/// gated — and dropped from the bridge-crate side and the Swift side by the *same* predicate, or
/// Swift calls a symbol the extern block never declared.
///
/// Applied once at each side's facade rather than threaded into the six method loops
/// (`emit_extern_block_for_type_methods`, `emit_extern_block_for_first_class_dto_methods`,
/// `emit_type_method_shims`, `emit_first_class_dto_method_wrappers`, `emit_client_class`,
/// `emit_error`): the two sides derive `configured_features` independently, so sharing one
/// filtering function is what makes their agreement structural instead of a property six
/// signatures have to keep re-establishing.
///
/// Deliberately narrower than [`crate::core::ir::ApiSurface::with_cfg_filtered_deep`], which also
/// drops gated fields and enum variants: swift already filters fields inline at each site that
/// consumes them (`wrappers::getters`, `extern_block::constructor_fields`), and re-filtering them
/// here would move that decision without being asked to. ~keep
pub(crate) fn with_cfg_filtered_methods(api: &ApiSurface, configured_features: &HashSet<&str>) -> ApiSurface {
    let mut filtered = api.clone();
    for typ in &mut filtered.types {
        typ.methods.retain(|method| method.cfg_satisfied(configured_features));
    }
    for enum_def in &mut filtered.enums {
        enum_def
            .methods
            .retain(|method| method.cfg_satisfied(configured_features));
    }
    for error in &mut filtered.errors {
        error.methods.retain(|method| method.cfg_satisfied(configured_features));
    }
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_satisfied_feature_matching() {
        let mut features = HashSet::new();
        features.insert("pdf");
        features.insert("html");

        assert!(cfg_satisfied(Some("feature = \"pdf\""), &features));

        assert!(!cfg_satisfied(Some("feature = \"heuristics\""), &features));

        assert!(cfg_satisfied(None, &features));

        let mut full_features = HashSet::new();
        full_features.insert("full");
        assert!(cfg_satisfied(Some("feature = \"heuristics\""), &full_features));
    }

    #[test]
    fn test_cfg_satisfied_any_matching() {
        let mut features = HashSet::new();
        features.insert("ocr");

        assert!(cfg_satisfied(
            Some("any(feature = \"ocr\", feature = \"paddle-ocr\")"),
            &features
        ));

        assert!(!cfg_satisfied(
            Some("any(feature = \"heuristics\", feature = \"embeddings\")"),
            &features
        ));
    }

    /// A source crate can gate one capability behind either of two sibling features (see
    /// [`collect_cfg_feature_alternatives`]) instead of nesting one feature inside the other's own
    /// Cargo feature list. When the configured Swift feature list only names one side, the other
    /// side must still be widened in, provided the source crate's own `Cargo.toml` declares it.
    #[test]
    fn configured_swift_features_widens_an_active_alternative_pair() {
        let temp = tempfile::tempdir().expect("create fixture workspace root");
        let crate_dir = temp.path().join("crates").join("demo_core");
        std::fs::create_dir_all(&crate_dir).expect("create fixture crate dir");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[features]\nengine-native = []\nengine-portable = []\n",
        )
        .expect("write fixture Cargo.toml");

        let config = ResolvedCrateConfig {
            name: "demo".to_string(),
            features: vec!["engine-native".to_string()],
            workspace_root: Some(temp.path().to_path_buf()),
            ..ResolvedCrateConfig::default()
        };
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![crate::core::ir::TypeDef {
                name: "Engine".to_string(),
                rust_path: "demo::Engine".to_string(),
                cfg: Some(r#"any(feature = "engine-native", feature = "engine-portable")"#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let features = configured_swift_features(&config, "demo_core", &api);
        assert!(
            features.iter().any(|f| f == "engine-portable"),
            "expected the source crate's paired feature to be widened in, got: {features:?}"
        );
    }

    /// The companion side of the widening test above: a feature name that only appears in the
    /// `any(...)` cfg gate, and that the source crate's own `Cargo.toml` never declares, must never
    /// be injected — that would hand Cargo a feature name it does not recognise.
    #[test]
    fn configured_swift_features_does_not_inject_a_feature_the_source_crate_lacks() {
        let temp = tempfile::tempdir().expect("create fixture workspace root");
        let crate_dir = temp.path().join("crates").join("demo_core");
        std::fs::create_dir_all(&crate_dir).expect("create fixture crate dir");
        std::fs::write(crate_dir.join("Cargo.toml"), "[features]\nengine-native = []\n")
            .expect("write fixture Cargo.toml");

        let config = ResolvedCrateConfig {
            name: "demo".to_string(),
            features: vec!["engine-native".to_string()],
            workspace_root: Some(temp.path().to_path_buf()),
            ..ResolvedCrateConfig::default()
        };
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![crate::core::ir::TypeDef {
                name: "Engine".to_string(),
                rust_path: "demo::Engine".to_string(),
                cfg: Some(r#"any(feature = "engine-native", feature = "engine-portable")"#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let features = configured_swift_features(&config, "demo_core", &api);
        assert!(
            !features.iter().any(|f| f == "engine-portable"),
            "must not inject a feature the source crate never declares, got: {features:?}"
        );
    }

    /// Neither side of an alternative pair is active: no widening should happen even though the
    /// source crate declares both features, since nothing in the configured list triggers it.
    #[test]
    fn configured_swift_features_leaves_an_inactive_pair_alone() {
        let temp = tempfile::tempdir().expect("create fixture workspace root");
        let crate_dir = temp.path().join("crates").join("demo_core");
        std::fs::create_dir_all(&crate_dir).expect("create fixture crate dir");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[features]\nengine-native = []\nengine-portable = []\n",
        )
        .expect("write fixture Cargo.toml");

        let config = ResolvedCrateConfig {
            name: "demo".to_string(),
            features: vec!["unrelated".to_string()],
            workspace_root: Some(temp.path().to_path_buf()),
            ..ResolvedCrateConfig::default()
        };
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![crate::core::ir::TypeDef {
                name: "Engine".to_string(),
                rust_path: "demo::Engine".to_string(),
                cfg: Some(r#"any(feature = "engine-native", feature = "engine-portable")"#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let features = configured_swift_features(&config, "demo_core", &api);
        assert_eq!(features, vec!["unrelated".to_string()]);
    }

    /// Regression for the open risk in alef-task #319: widening a companion feature into the
    /// core dependency's `features = [...]` line bypassed `excluded_default_features` entirely
    /// -- that exclusion only trimmed the wrapper crate's own `default = [...]` array
    /// (`effective_swift_codegen_features`), not the list this function feeds straight into
    /// `cargo::emit_cargo_toml`'s dependency line. Measured against a synthetic fixture
    /// workspace (`cargo tree -e features`): the dependency line does not set
    /// `default-features = false`, and the companion feature is opt-in on the source crate (not
    /// part of its own `default`), so nothing else would have turned it on -- the widening was a
    /// genuine new activation of a pkg-config-only native dependency, not something feature
    /// unification would already have done. A name in `excluded_default_features` must never be
    /// widened in, even when its sibling in the same `any(...)` group is active.
    #[test]
    fn configured_swift_features_never_widens_an_excluded_companion() {
        let temp = tempfile::tempdir().expect("create fixture workspace root");
        let crate_dir = temp.path().join("crates").join("demo_core");
        std::fs::create_dir_all(&crate_dir).expect("create fixture crate dir");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[features]\nocr = []\nocr-wasm = []\nheic = []\n",
        )
        .expect("write fixture Cargo.toml");

        let config = ResolvedCrateConfig {
            name: "demo".to_string(),
            features: vec!["ocr".to_string()],
            workspace_root: Some(temp.path().to_path_buf()),
            swift: Some(crate::core::config::languages::SwiftConfig {
                excluded_default_features: vec!["heic".to_string()],
                ..Default::default()
            }),
            ..ResolvedCrateConfig::default()
        };
        // A three-way `any(...)` gate shared by unrelated capabilities (as in xberg's
        // `extraction/exif.rs`): `ocr` and `ocr-wasm` are genuine alternatives, `heic` merely
        // co-occurs in the same gate and pulls in a pkg-config-only native dependency.
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![crate::core::ir::TypeDef {
                name: "ExifReader".to_string(),
                rust_path: "demo::ExifReader".to_string(),
                cfg: Some(r#"any(feature = "ocr", feature = "ocr-wasm", feature = "heic")"#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let features = configured_swift_features(&config, "demo_core", &api);
        assert!(
            !features.iter().any(|f| f == "heic"),
            "an excluded companion must never be widened into the dependency's features list, got: {features:?}"
        );
        assert!(
            features.iter().any(|f| f == "ocr-wasm"),
            "a non-excluded companion in the same group must still be widened in, got: {features:?}"
        );
    }

    /// A crate whose cfg gates carry no `any(...)` alternative at all — every gate is a bare
    /// `feature = "..."` — has nothing for [`collect_cfg_feature_alternatives`] to find, so the
    /// configured feature list must pass through completely unchanged. The source crate declares
    /// (and separately, and validly, gates a different item behind) a second feature,
    /// `engine-portable`, so a mechanism that widened on any cfg-referenced name — not just one
    /// paired with an active feature inside a shared `any(...)` — would inject it here; this
    /// pins that only true `any(...)` alternation, not mere co-occurrence anywhere in the surface,
    /// triggers widening.
    #[test]
    fn configured_swift_features_unchanged_when_no_cfg_alternatives_exist() {
        let temp = tempfile::tempdir().expect("create fixture workspace root");
        let crate_dir = temp.path().join("crates").join("demo_core");
        std::fs::create_dir_all(&crate_dir).expect("create fixture crate dir");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[features]\nengine-native = []\nengine-portable = []\n",
        )
        .expect("write fixture Cargo.toml");

        let config = ResolvedCrateConfig {
            name: "demo".to_string(),
            features: vec!["engine-native".to_string()],
            workspace_root: Some(temp.path().to_path_buf()),
            ..ResolvedCrateConfig::default()
        };
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![
                crate::core::ir::TypeDef {
                    name: "Engine".to_string(),
                    rust_path: "demo::Engine".to_string(),
                    cfg: Some(r#"feature = "engine-native""#.to_string()),
                    ..Default::default()
                },
                crate::core::ir::TypeDef {
                    name: "PortableWidget".to_string(),
                    rust_path: "demo::PortableWidget".to_string(),
                    cfg: Some(r#"feature = "engine-portable""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let features = configured_swift_features(&config, "demo_core", &api);
        assert_eq!(features, vec!["engine-native".to_string()]);
    }
}
