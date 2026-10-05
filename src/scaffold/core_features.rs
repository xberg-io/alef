//! Core-crate feature resolution for binding manifests: passthrough feature-list
//! rendering, the core crate's own `[features]` closure, and the derived Android
//! `android-target` aggregate line.

use crate::core::config::{Language, ResolvedCrateConfig};

/// Checks for per-language feature overrides first, then falls back to `[crate] features`.
/// Returns an empty string if no features are configured, otherwise returns
/// `, features = ["feat1", "feat2"]`.
pub(crate) fn core_dep_features(config: &ResolvedCrateConfig, lang: Language) -> String {
    core_dep_features_excluding(config, lang, &std::collections::HashSet::new())
}

/// Like [`core_dep_features`], but drops any name in `excluded` from the core dependency's
/// `features = [...]` line.
///
/// Mirrors `scaffold::languages::ruby::ruby_core_dep_features`, generalized here because more
/// than two languages need it -- see `RubyConfig::excluded_default_features`'s doc comment for
/// the full rationale. The consumer-facing exclusion is meant to keep a feature off this
/// dependency edge entirely, not just out of the wrapper's own `default = [...]` array:
/// forwarding an excluded name into this explicit line unions it straight back into the core
/// crate via Cargo's feature unification, defeating a `target_dep_overrides` entry that tried to
/// turn it off for a specific cfg target. Missing this surface -- filtering only the wrapper's
/// own default array -- is precisely what left the original Swift widening defect reachable. ~keep
pub(crate) fn core_dep_features_excluding(
    config: &ResolvedCrateConfig,
    lang: Language,
    excluded: &std::collections::HashSet<&str>,
) -> String {
    let features: Vec<&str> = config
        .features_for_language(lang)
        .iter()
        .map(String::as_str)
        .filter(|f| !excluded.contains(f))
        .collect();
    if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    }
}

/// Locate the core crate's `Cargo.toml` for a resolved config.
///
/// Derives the crate directory from the first source path (walking up to the
/// `src/` parent, mirroring [`ResolvedCrateConfig::core_crate_dir`]) and joins
/// it against `workspace_root`. Returns `None` when there is no workspace root
/// (the binding is being scaffolded standalone) or when the path cannot be
/// derived — both cases simply skip the `android-target` aggregate emission.
pub(crate) fn core_crate_manifest_path(config: &ResolvedCrateConfig) -> Option<std::path::PathBuf> {
    let workspace_root = config.workspace_root.as_deref()?;
    let first_source = config.sources.first()?;
    let mut current = std::path::Path::new(first_source).parent();
    while let Some(dir) = current {
        if dir.file_name().is_some_and(|n| n == "src") {
            let crate_dir = dir.parent()?;
            return Some(workspace_root.join(crate_dir).join("Cargo.toml"));
        }
        current = dir.parent();
    }
    None
}

/// Resolve a core-crate aggregate feature to the transitive set of feature-name
/// tokens reachable from it.
///
/// BFS over the core crate's `[features]` map starting at `aggregate`. Every
/// member token that is itself a key in the map is followed; tokens of the
/// `dep:foo` or `crate/feat` form are skipped (they are not binding-side
/// passthrough features). The returned set therefore contains every plain
/// feature name reachable from the aggregate, including the aggregate's own
/// sub-aggregates' leaves. Returns `None` when the core crate has no feature by
/// that name (so callers can skip emission for repos that lack it).
fn resolve_core_aggregate_features(
    features_table: &toml::value::Table,
    aggregate: &str,
) -> Option<std::collections::BTreeSet<String>> {
    let _ = features_table.get(aggregate)?;
    let mut reachable: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut queue: Vec<String> = vec![aggregate.to_string()];
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(name) = queue.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(members) = features_table.get(&name).and_then(|v| v.as_array()) else {
            continue;
        };
        for member in members {
            let Some(token) = member.as_str() else { continue };
            if token.starts_with("dep:") || token.contains('/') {
                continue;
            }
            reachable.insert(token.to_string());
            if features_table.contains_key(token) {
                queue.push(token.to_string());
            }
        }
    }
    Some(reachable)
}

/// Read the core crate's own `[features]` table and resolve `requested` (the literal feature
/// names on the core dependency line, e.g. `["native-http", "full"]`) into the full transitive
/// set of features that ends up active, plus the core crate's own declared `default` feature
/// list. Each name in `requested` that is itself an aggregate in the core crate's `[features]`
/// table (e.g. `full`) is expanded via [`resolve_core_aggregate_features`]; plain leaf names are
/// kept as-is. Falls back to `requested` verbatim as the active set (and an empty default list)
/// when the core manifest can't be located, read, or parsed — the same permissive fallback
/// [`android_target_feature_line_for_dep`] uses.
pub(crate) fn core_feature_closure(
    config: &ResolvedCrateConfig,
    requested: &[String],
) -> (std::collections::BTreeSet<String>, std::collections::BTreeSet<String>) {
    let mut active: std::collections::BTreeSet<String> = requested.iter().cloned().collect();
    let no_defaults = std::collections::BTreeSet::new();
    let Some(manifest_path) = core_crate_manifest_path(config) else {
        return (active, no_defaults);
    };
    let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
        return (active, no_defaults);
    };
    let Ok(doc) = toml::from_str::<toml::Value>(&contents) else {
        return (active, no_defaults);
    };
    let Some(features_table) = doc.get("features").and_then(|v| v.as_table()) else {
        return (active, no_defaults);
    };
    for name in requested {
        if let Some(members) = resolve_core_aggregate_features(features_table, name) {
            active.extend(members);
        }
    }
    let defaults: std::collections::BTreeSet<String> = features_table
        .get("default")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(String::from).collect())
        .unwrap_or_default();
    (active, defaults)
}

/// The tokens of the core crate's own `default` feature, in declared order.
///
/// `None` when the core manifest cannot be located, read, or parsed -- callers must tell that
/// apart from `Some(empty)`, a readable manifest with no `default` feature, because only the
/// latter proves the core crate adds nothing by default.
pub(crate) fn core_default_feature_tokens(config: &ResolvedCrateConfig) -> Option<Vec<String>> {
    let contents = std::fs::read_to_string(core_crate_manifest_path(config)?).ok()?;
    let doc = toml::from_str::<toml::Value>(&contents).ok()?;
    let tokens = doc
        .get("features")
        .and_then(|features| features.get("default"))
        .and_then(|default| default.as_array())
        .map(|array| array.iter().filter_map(|v| v.as_str()).map(String::from).collect())
        .unwrap_or_default();
    Some(tokens)
}

/// Compute the binding-crate `android-target` aggregate feature line, if applicable.
///
/// The consuming repo's core crate may define an `android-target` aggregate (a
/// curated ORT-free, libheif-free feature set) so it can be cross-compiled for
/// Android via `cargo ndk ... --no-default-features --features android-target`.
/// The binding crate's own FFI exports are gated by its passthrough features, so
/// it must expose a matching `android-target` that enables the passthrough
/// features that are members of the core aggregate — not merely forward to the
/// core dep.
///
/// `passthrough_feature_names` is the binding crate's own forwarding feature set
/// (the names that appear in its `[features]` passthrough block, e.g. `pdf`,
/// `ocr`), excluding the `full` umbrella. Returns the emitted line:
///
/// ```text
/// android-target = ["<core>/android-target", <sorted passthrough ∩ core aggregate>]
/// ```
///
/// Returns `None` when the core crate has no `android-target` feature (so other
/// consuming repos are unaffected) or when its manifest cannot be read.
pub(crate) fn android_target_feature_line(
    config: &ResolvedCrateConfig,
    passthrough_feature_names: &[&str],
) -> Option<String> {
    android_target_feature_line_for_dep(config, &config.name, passthrough_feature_names)
}

/// Variant of [`android_target_feature_line`] that takes the core-crate cargo
/// dep key explicitly.
///
/// The FFI crate forwards via the cargo package name (`config.name`), whereas
/// the dart bridge crate forwards via the rust-ident dep key (e.g. `sample_lib`)
/// to match its other passthrough entries. Both share the same resolution logic.
pub(crate) fn android_target_feature_line_for_dep(
    config: &ResolvedCrateConfig,
    core_dep_key: &str,
    passthrough_feature_names: &[&str],
) -> Option<String> {
    let manifest_path = core_crate_manifest_path(config)?;
    let contents = std::fs::read_to_string(&manifest_path).ok()?;
    let doc = toml::from_str::<toml::Value>(&contents).ok()?;
    let features_table = doc.get("features")?.as_table()?;
    let aggregate_members = resolve_core_aggregate_features(features_table, "android-target")?;

    let mut selected: Vec<&str> = passthrough_feature_names
        .iter()
        .copied()
        .filter(|name| *name != "full" && aggregate_members.contains(*name))
        .collect();
    selected.sort_unstable();
    selected.dedup();

    let mut tokens: Vec<String> = vec![format!("{core_dep_key}/android-target")];
    tokens.extend(selected.iter().map(|name| (*name).to_string()));
    let list = tokens.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ");
    Some(format!("android-target = [{list}]"))
}
