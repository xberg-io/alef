//! Merging referenced cfg features into an existing Cargo manifest, split out of cfg.rs.

use super::*;

/// Insert every cfg-referenced or explicitly preserved wrapper-default name missing from `existing`'s own
/// `[features]` table -- forwarding each to `core_crate_name` the same way the sibling rows
/// `scaffold_ruby_cargo`/`scaffold_elixir_cargo` already write do (`<feature> =
/// ["<core_crate_name>/<feature>"]`) -- and, separately, every referenced name missing from
/// `default`, appending it to that array.
///
/// Declaring a Cargo feature does not enable it: a forwarding row alone leaves `#[cfg(feature =
/// "X")]` false unless something turns `X` on, and neither `mix`/`rake-compiler`/`cargo` build
/// wrapper this repair supports passes a `--features` flag. `scaffold_ruby_cargo` and
/// `scaffold_elixir_cargo` already put every name [`collect_cfg_features`] finds straight into
/// `default` on a fresh scaffold (see their own `default = [...]` line); this mirrors that so a
/// feature that is already declared but was never added to `default` -- the exact shape a
/// manifest patched by an earlier version of this function is left in -- still gets fixed on the
/// next repair pass, not just a brand-new feature. ~keep
///
/// Returns `Ok(None)` when nothing needs to change (every requested feature is already declared
/// and enabled by default), so callers can distinguish "checked, no update needed" from "wrote
/// the merge" without a further content diff. Cfg-discovered names are limited to features the
/// core manifest declares; explicit wrapper defaults are trusted configuration and are preserved
/// verbatim. ~keep
///
/// Parses with `toml_edit::DocumentMut`, not the `toml` crate [`read_declared_cargo_features`]
/// uses: `toml_edit` preserves every byte it does not touch -- comments, key order, blank lines,
/// a hand-added `[package.metadata.*]` table -- so the only lines this can ever change are the
/// new feature rows it inserts and the `default` array entries it appends. A `[features]` table
/// absent from `existing` is created; `toml_edit` appends a new table at the document's end
/// rather than reflowing existing ones, so this never disturbs any other table's position. A
/// `default` array is created the same way if the table has none yet, and an existing one keeps
/// every entry it already has -- only missing names are pushed onto the end.
///
/// This is `alef scaffold`'s answer to the "re-run `alef scaffold`" remedy the compile-out warning
/// (`warn_on_undeclared_binding_cfg_features`) prescribes: the manifest this repairs is user-owned
/// and `write_scaffold_files_report`'s ownership guard rightly refuses to blindly overwrite it, but
/// a purely additive `[features]` change cannot corrupt, reorder, or drop anything else in the
/// file, so it is safe to apply on its own, narrower write path even when the guard would
/// otherwise refuse the whole manifest. ~keep
///
/// `excluded_default_features` is the same per-language list every scaffolder feeds
/// [`cfg_default_and_forwarding_lines`], and it binds here for the same reason: a name the
/// binding's own config deliberately keeps out of `default` must not be put back by a later
/// repair pass. Without it this function re-enabled exactly the names a scaffolder had just
/// excluded, so the manifest a scaffold run wrote and the manifest a scaffold-plus-repair run
/// wrote disagreed on `default` for every configured exclusion. An excluded name still gets its
/// forwarding row (so `cargo build --features <name>` keeps working), matching the scaffolders'
/// own treatment -- only the `default` array is held back. ~keep
pub fn merge_missing_cfg_features(
    existing: &str,
    api: &ApiSurface,
    wrapper_default_features: &[String],
    core_crate_name: &str,
    core_declared_features: &BTreeSet<String>,
    excluded_default_features: &HashSet<&str>,
) -> anyhow::Result<Option<String>> {
    validate_wrapper_default_features(
        wrapper_default_features,
        core_declared_features,
        excluded_default_features,
    )?;
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .context("existing manifest is not valid TOML")?;

    let features_table_ref = doc.get("features").and_then(toml_edit::Item::as_table);
    let declared: BTreeSet<String> = features_table_ref
        .map(|table| table.iter().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default();
    let enabled_by_default = features_reachable_from_default(|name| {
        features_table_ref
            .and_then(|table| table.get(name))
            .and_then(toml_edit::Item::as_array)
            .into_iter()
            .flat_map(toml_edit::Array::iter)
            .filter_map(toml_edit::Value::as_str)
            .filter(|member| !member.contains('/'))
            .map(str::to_owned)
            .collect()
    });

    let mut referenced: BTreeSet<String> = collect_cfg_features(api)
        .into_iter()
        .filter(|feature| core_declared_features.contains(feature))
        .collect();
    referenced.extend(wrapper_default_features.iter().filter(|name| !name.is_empty()).cloned());
    let needs_declaration: BTreeSet<String> = referenced.difference(&declared).cloned().collect();
    let needs_default: BTreeSet<String> = referenced
        .difference(&enabled_by_default)
        .filter(|name| !excluded_default_features.contains(name.as_str()))
        .cloned()
        .collect();

    if needs_declaration.is_empty() && needs_default.is_empty() {
        return Ok(None);
    }

    let features_table = doc
        .entry("features")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("[features] exists in the manifest but is not a table")?;

    for feature in &needs_declaration {
        let mut forwarded = toml_edit::Array::new();
        forwarded.push(format!("{core_crate_name}/{feature}"));
        features_table.insert(feature, toml_edit::Item::Value(toml_edit::Value::Array(forwarded)));
    }

    if !needs_default.is_empty() {
        append_default_features(features_table, &needs_default)?;
    }

    Ok(Some(doc.to_string()))
}

fn append_default_features(
    features_table: &mut toml_edit::Table,
    needs_default: &BTreeSet<String>,
) -> anyhow::Result<()> {
    let default_array = features_table
        .entry("default")
        .or_insert_with(|| toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())))
        .as_array_mut()
        .context("features.default exists but is not an array")?;
    let already_listed: BTreeSet<String> = default_array
        .iter()
        .filter_map(toml_edit::Value::as_str)
        .map(str::to_owned)
        .collect();
    for feature in needs_default {
        if !already_listed.contains(feature) {
            default_array.push(feature.clone());
        }
    }
    Ok(())
}

fn validate_wrapper_default_features(
    wrapper_default_features: &[String],
    core_declared_features: &BTreeSet<String>,
    excluded_default_features: &HashSet<&str>,
) -> anyhow::Result<()> {
    for feature in wrapper_default_features {
        crate::core::config::validation::validate_wrapper_default_feature_name(feature)
            .map_err(|error| anyhow::anyhow!("invalid wrapper default feature `{feature}`: {error}"))?;
        if !core_declared_features.contains(feature) {
            anyhow::bail!("wrapper default feature `{feature}` is not declared by the core crate");
        }
        if excluded_default_features.contains(feature.as_str()) {
            anyhow::bail!("wrapper default feature `{feature}` cannot also be listed in excluded_default_features");
        }
    }
    Ok(())
}
