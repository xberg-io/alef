use crate::backends::php::layout::php_psr4_target;
use crate::backends::php::naming::php_autoload_namespace;
use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::detect_workspace_inheritance_for_crate, scaffold::render_extra_deps,
    scaffold::scaffold_meta,
};
use anyhow::Context as _;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Additional features to request on the core feature that would satisfy `pred`, given `active`
/// (the core features already unconditionally active on the dependency line, transitively
/// resolved — see [`crate::scaffold::core_feature_closure`]) and `core_defaults` (the core
/// crate's own declared `[features] default = [...]`).
///
/// - `Feature(X)`: nothing if `X` is already in `active`; otherwise `{X}`.
/// - `All(arms)`: the union of what each arm needs — every arm must hold.
/// - `Any(arms)`: **nothing** if any arm is already satisfied by `active` — do not union every
///   arm just because one wasn't. Only when *no* arm holds do we add one, and then exactly one:
///   an arm the core crate's own `default` list already picks if one exists, else the first arm.
///   Forcing every arm on is not conservative, it's wrong: `any(native-http, wasm-http)` are
///   mutually exclusive transports in the core crate (native-http pulls in the Tokio
///   multi-thread runtime; wasm-http is for browser/Node targets that cannot run it), so unioning
///   them would silently bloat every PHP build with an unused transport stack rather than pick
///   the one the core crate already treats as its default.
/// - `Not(_)` / `Other`: nothing — a `not(...)` can only be satisfied by *not* adding a feature,
///   which this function has no way to enforce either way, so it is left alone rather than
///   guessed at. ~keep
fn missing_features_for(
    pred: &crate::codegen::cfg::CfgPredicate,
    active: &BTreeSet<String>,
    core_defaults: &BTreeSet<String>,
) -> BTreeSet<String> {
    use crate::codegen::cfg::CfgPredicate;
    match pred {
        CfgPredicate::Feature(name) => {
            if active.contains(name) {
                BTreeSet::new()
            } else {
                BTreeSet::from([name.clone()])
            }
        }
        CfgPredicate::All(arms) => arms
            .iter()
            .flat_map(|arm| missing_features_for(arm, active, core_defaults))
            .collect(),
        CfgPredicate::Any(arms) => {
            let already_satisfied = arms
                .iter()
                .any(|arm| missing_features_for(arm, active, core_defaults).is_empty());
            if already_satisfied {
                return BTreeSet::new();
            }
            let plain_arms: Vec<String> = arms
                .iter()
                .filter_map(|arm| match arm {
                    CfgPredicate::Feature(name) => Some(name.clone()),
                    _ => None,
                })
                .collect();
            let chosen: Option<String> = plain_arms
                .iter()
                .find(|name| core_defaults.contains(name.as_str()))
                .cloned()
                .or_else(|| plain_arms.first().cloned());
            match chosen {
                Some(name) => BTreeSet::from([name]),
                // No plain-feature arm at all (e.g. `any(all(...), not(...))`): fall back to
                // whatever the first arm itself needs.
                None => arms
                    .first()
                    .map(|arm| missing_features_for(arm, active, core_defaults))
                    .unwrap_or_default(),
            }
        }
        CfgPredicate::Not(_) | CfgPredicate::Other => BTreeSet::new(),
    }
}

/// Core features PHP must additionally request beyond what's already configured, because
/// standalone functions carrying a `cfg` predicate are emitted unconditionally into the
/// `#[php_impl]` facade (see `rust_bindings.rs::generate_bindings`'s doc comment) and their
/// underlying core symbols must therefore always exist.
///
/// Deliberately over-inclusive in one respect: it does not replicate `generate_bindings`'s
/// `exclude_functions`/trait-bridge-managed filtering, so a function's cfg predicate can be
/// evaluated here even when that particular function ends up excluded from the PHP facade.
/// Requesting an unused core feature is harmless; the failure mode this function exists to
/// prevent — a required feature silently missing — is the one that actually breaks the build. ~keep
fn php_function_gated_core_features_to_add(api: &ApiSurface, config: &ResolvedCrateConfig) -> BTreeSet<String> {
    let requested = config.features_for_language(Language::Php);
    let (active, core_defaults) = crate::scaffold::core_feature_closure(config, requested);

    let mut to_add = BTreeSet::new();
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            let pred = crate::codegen::cfg::parse_cfg_predicate(cfg);
            to_add.extend(missing_features_for(&pred, &active, &core_defaults));
        }
    }
    to_add
}

/// Every feature name referenced by any top-level function's `cfg` predicate, flattened —
/// unlike [`php_function_gated_core_features_to_add`], this includes names that turned out to
/// already be satisfied (e.g. `tower`/`tokenizer` when the core dependency line already requests
/// `full`). Used only to keep `cfg_forwarding` from declaring these as toggleable php-crate
/// `[features]`: PHP never gates a function by cfg (see `rust_bindings.rs::generate_bindings`),
/// so none of these names should appear there regardless of whether anything needed adding for
/// them. ~keep
pub(crate) fn php_function_referenced_feature_names(api: &ApiSurface) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            crate::codegen::cfg::collect_cfg_feature_names(cfg, &mut out);
        }
    }
    out
}

/// The exact feature-name set this backend writes into the php crate's `[features]` table.
///
/// ~keep One source of truth for two consumers that MUST agree: `scaffold_php_cargo` renders the
/// table from it, and `rust_bindings` narrows every emitted `#[cfg(...)]` against it. When they
/// disagreed, codegen emitted a gate naming `url-ingestion` -- a name stripped from the table by
/// `php_function_referenced_feature_names` -- and rustc rejected it as
/// `unexpected_cfg_condition_value` under `-D warnings`. The comment on that stripping still says
/// PHP "never gates a function by cfg", which stayed true, but it is a FIELD gate that reaches the
/// table's namespace, so the two sets have to be derived together rather than each rebuilt.
pub(crate) fn php_declared_features(api: &ApiSurface, excluded_default_features: &[&str]) -> BTreeSet<String> {
    let mut features = crate::codegen::cfg::collect_cfg_features(api);
    for name in &php_function_referenced_feature_names(api) {
        features.remove(name);
    }
    features.extend(excluded_default_features.iter().map(|name| (*name).to_string()));
    features
}

/// Render the core dependency's `, features = [...]` clause, unioning the user-configured
/// per-language feature list (minus any name in `excluded_default_features` — see
/// `PhpConfig::excluded_default_features`) with `extra` (features PHP must always request
/// because it can't safely gate them — see [`php_function_gated_core_features_to_add`]). `extra`
/// is deliberately NOT filtered against `excluded_default_features`: those names are hard
/// compile-time requirements of an unconditionally-emitted function, not a default-features
/// convenience the exclusion knob is meant to suppress. Returns an empty string when there is
/// nothing to request, matching `crate::scaffold::core_dep_features`'s empty case.
fn merged_core_dep_features(
    config: &ResolvedCrateConfig,
    extra: &BTreeSet<String>,
    excluded_default_features: &std::collections::HashSet<&str>,
) -> String {
    let mut features: Vec<String> = config
        .features_for_language(Language::Php)
        .iter()
        .filter(|f| !excluded_default_features.contains(f.as_str()))
        .cloned()
        .collect();
    for name in extra {
        if !features.iter().any(|f| f == name) {
            features.push(name.clone());
        }
    }
    if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    }
}

pub(crate) fn scaffold_php_cargo(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let crate_dir = format!("crates/{core_crate_dir}-php");
    let ws = detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &crate_dir);
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-php"), version, "2024", &meta, &ws);

    let extra_deps = render_extra_deps(config, Language::Php);

    let has_trait_bridges = !config.trait_bridges.is_empty();
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    let needs_ahash = api.functions.iter().any(|f| f.params.iter().any(|p| p.map_is_ahash));
    let mut all_deps = extra_deps;
    if needs_ahash {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("ahash = \"0.8\"");
    }
    if has_trait_bridges && !all_deps.contains("async-trait") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("async-trait = \"0.1\"");
    }
    if has_trait_bridges && !all_deps.contains("tracing") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str(&format!("tracing = \"{}\"", tv::cargo::TRACING));
    }
    if has_streaming && !all_deps.contains("futures-util = ") && !all_deps.contains("futures-util =\"") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("futures-util = \"0.3\"");
    }

    let extra_deps_section = if all_deps.is_empty() {
        String::new()
    } else {
        format!("\n{all_deps}")
    };
    // does not use `#[async_trait]` after JSON-bridging. `ahash` is added when
    let mut machete_ignored: Vec<&str> = vec!["tokio", "ahash"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tracing");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    let machete_ignored_str = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    // Functions carrying a source `cfg` predicate are emitted unconditionally into the
    // `#[php_impl]` facade class (see rust_bindings.rs's `generate_bindings`: ext-php-rs's
    // `#[php_impl]` derive references every method by identifier in its registration array
    // regardless of `#[cfg]`, so a cfg'd-out method breaks the build). Their underlying core
    // features must therefore be required unconditionally on the core dependency line rather
    // than exposed as toggleable php-crate `[features]` — a toggleable feature that no generated
    // code actually gates is a defect (see `cfg_forwarding` below, which excludes these names).
    // `core_features_to_add` is deliberately *not* the flat union of every name a function's cfg
    // mentions: an `any(A, B)` predicate only needs A or B, and here A (`native-http`) is already
    // requested via the existing `features = [..., "full"]`, so nothing is added for it at all —
    // see `missing_features_for`'s doc comment for why unioning both arms would be wrong. ~keep
    let core_features_to_add = php_function_gated_core_features_to_add(api, config);
    let core_overrides = config
        .php
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let excluded_default_features: std::collections::HashSet<&str> = config
        .php
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let core_dep_path = config.core_crate_dep_path(std::path::Path::new(&crate_dir));
    let (core_dep_php, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &core_dep_path,
        &merged_core_dep_features(config, &core_features_to_add, &excluded_default_features),
        version,
        core_overrides,
    );
    let core_target_blocks_section = if core_target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{core_target_blocks}")
    };
    let mut dep_entries: Vec<String> = vec![
        format!("ext-php-rs = \"{}\"", tv::cargo::EXT_PHP_RS),
        "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
        "serde_json = \"1\"".to_string(),
        "tokio = { version = \"1\", features = [\"full\"] }".to_string(),
    ];
    if !core_dep_php.is_empty() {
        dep_entries.push(core_dep_php.clone());
    }
    if !all_deps.is_empty() {
        for line in all_deps.lines() {
            if !line.is_empty() {
                dep_entries.push(line.to_string());
            }
        }
    }
    crate::scaffold::sort_dependency_lines(&mut dep_entries);
    let dep_block = dep_entries.join("\n");
    let _ = extra_deps_section;

    // Forwards feature names that a `#[cfg(feature = "X")]` on a *type/field/enum* (never a
    // function — those are handled unconditionally above and excluded here) could still
    // reference, keeping such names known to Cargo's `[features]` table. PHP's own struct/enum
    // codegen currently drops cfg'd-out fields and variants outright rather than emitting a
    // `#[cfg]` for them, but declaring the passthrough keeps this backend consistent with the
    // other binding backends that share `collect_cfg_features` and protects against a future
    // codegen change that starts emitting such a `#[cfg]`.
    let core_dep_name = &config.name;
    let cfg_forwarding: String = {
        // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`) must
        // still get a forwarding entry below -- alef-task #374, regression in the `mod tests`
        // block above. Folded into `php_declared_features` so the narrowing in `rust_bindings`
        // sees the same set this table declares. ~keep
        let mut excluded_sorted: Vec<&str> = excluded_default_features.iter().copied().collect();
        excluded_sorted.sort_unstable();
        let features = php_declared_features(api, &excluded_sorted);
        if features.is_empty() {
            String::new()
        } else {
            // A name in `excluded_default_features` is still declared below (so
            // `cargo build --features <name>` keeps working) but dropped from `default`,
            // matching `RubyConfig::excluded_default_features`. ~keep
            let lines = crate::codegen::cfg::cfg_default_and_forwarding_lines(
                &features,
                core_dep_name,
                &excluded_default_features,
            );
            format!("{}\n", lines.join("\n"))
        }
    };

    let lints_section = crate::scaffold::cargo_lints_section(config);
    let content = format!(
        r#"{pkg_header}

# `ahash` and `futures-util` are conditionally included but not directly used in PHP code.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[lib]
crate-type = ["cdylib"]

[features]
extension-module = []
{cfg_forwarding}
[dependencies]
{dep_block}
{core_target_blocks_section}{lints_section}"#,
        pkg_header = pkg_header,
        lints_section = lints_section,
        dep_block = dep_block,
        core_target_blocks_section = core_target_blocks_section,
        machete_ignored_str = machete_ignored_str,
        cfg_forwarding = cfg_forwarding,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-php/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_php(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let ext_name = config.php_extension_name();
    let pkg_dir = config.package_dir(Language::Php);
    let php_namespace = php_autoload_namespace(config).replace('\\', "\\\\");

    let keywords_json = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };

    let (vendor, package_name) = if let Some(pkg) = config.php.as_ref().and_then(|p| p.composer_package.as_ref()) {
        let parts: Vec<&str> = pkg.split('/').collect();
        match parts.as_slice() {
            [v, p] => (v.to_string(), p.to_string()),
            _ => composer_package_name(config, &meta),
        }
    } else {
        composer_package_name(config, &meta)
    };

    let render_composer = |autoload_src: &str| -> String {
        let license_json = meta
            .license
            .as_deref()
            .map(|license| format!("  \"license\": \"{license}\",\n"))
            .unwrap_or_default();

        let pie_binary_block = if let Some(repo_url) = meta.configured_repository.as_deref() {
            format!(
                ",\n  \"extra\": {{\n    \"pie\": {{\n      \"binary\": {{\n        \"url-template\": \"{repo_url}/releases/download/{{Version}}/php_{ext_name}-{{Version}}_php{{PhpVersion}}-{{Arch}}-{{OSLower}}-{{Libc}}-{{TSMode}}.tgz\"\n      }}\n    }}\n  }}"
            )
        } else {
            String::new()
        };

        format!(
            r#"{{
  "name": "{vendor}/{package_name}",
  "description": "{description}",
{license_json}  "type": "php-ext",
  "require": {{
    "php": ">=8.2"
  }},
  "require-dev": {{
    "phpunit/phpunit": "{phpunit}"
  }},
  "autoload": {{
    "psr-4": {{
      "{php_namespace}\\": "{autoload_src}"
    }}
  }},
  "scripts": {{
    "format": "poly fmt --fix",
    "format:check": "poly fmt --check",
    "test": "php vendor/bin/phpunit",
    "lint": "poly lint",
    "lint:fix": "poly lint --fix && poly fmt --fix"
  }},
  "php-ext": {{
    "extension-name": "{ext_name}",
    "support-zts": true,
    "support-nts": true,
    "download-url-method": "pre-packaged-binary"
  }}{keywords}{pie_binary}
}}
"#,
            vendor = vendor,
            package_name = package_name,
            description = meta.description,
            license_json = license_json,
            php_namespace = php_namespace,
            autoload_src = autoload_src,
            ext_name = ext_name,
            keywords = keywords_json,
            pie_binary = pie_binary_block,
            phpunit = tv::packagist::PHPUNIT,
        )
    };

    // The root manifest autoloads whatever directory the backend writes the classes into, read
    // from `php_psr4_target` rather than re-derived here — a second derivation is how the e2e
    // manifest ended up naming a `src/` subdirectory of it that no stage writes. ~keep
    let co_located = config.output_paths.contains_key("php");
    let root_autoload_src = php_psr4_target(config);

    let mut files = vec![GeneratedFile {
        path: PathBuf::from("composer.json"),
        content: render_composer(&root_autoload_src),
        generated_header: false,
    }];

    // ~keep Exactly one manifest per layout. Both render the same composer `name`, and two
    // manifests declaring one package name is ambiguous by construction. Split layout keeps the
    // `pkg_dir` copy, because there the classes live under `packages/php/src/` and that manifest is
    // the installable package. Co-located layout does not: the classes are in `pkg_dir` itself and
    // the root manifest already autoloads them, so a second manifest beside them is unreachable —
    // Packagist reads the repository root, and every consumer reference (root autoload, e2e,
    // test_apps) targets the directory rather than the manifest. Emitting it regardless put a
    // duplicate of the published package name into every co-located consumer.
    if !co_located {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/composer.json")),
            content: render_composer("src/"),
            generated_header: false,
        });
    }

    Ok(files)
}

/// The `phpunit/phpunit` constraint `scaffold_php` emitted before the fix (`ddde77260`) that
/// widened it to span every PHPUnit major supporting the declared `"php": ">=8.2"` floor.
const STALE_PHPUNIT_CONSTRAINT: &str = "\"phpunit/phpunit\": \"^13.1\"";
/// The replacement constraint -- the exact value `tv::packagist::PHPUNIT` renders today.
const FIXED_PHPUNIT_CONSTRAINT: &str = "\"phpunit/phpunit\": \"^11.5 || ^12.0 || ^13.1\"";

/// Repair a pre-existing `composer.json` (root or `{pkg_dir}`) whose `require-dev` still pins
/// `phpunit/phpunit` to the PHPUnit-13-only constraint -- the exact defect fixed in
/// `ddde77260` ("widen the scaffolded PHPUnit constraint to the declared PHP floor").
///
/// `composer.json` is `generated_header: false` (create-only), so a repo scaffolded before that
/// fix keeps a `^13.1`-only constraint forever even though the same file declares
/// `"php": ">=8.2"`: PHPUnit 13 requires PHP >=8.4.1, so `composer install` cannot resolve on
/// 8.2 or 8.3, and Dependabot -- which resolves Composer against the declared platform floor
/// rather than the runtime PHP -- fails on every run. `^13.1` is the one and only value alef
/// itself has ever emitted here, so an exact substring match is a safe positive-identification
/// signature; the additional `"type": "php-ext"` check confirms this really is one of alef's
/// own PHP extension manifests and not a coincidental match in an unrelated project. This only
/// ever replaces the one matched substring -- a consumer's own `require`/`autoload`/`scripts`
/// customisations survive byte-for-byte. ~keep
pub(crate) fn migrate_php_composer_phpunit_constraint(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if !existing.contains("\"type\": \"php-ext\"") {
        return Ok(false);
    }
    if existing.matches(STALE_PHPUNIT_CONSTRAINT).count() != 1 {
        return Ok(false);
    }
    let migrated = existing.replacen(STALE_PHPUNIT_CONSTRAINT, FIXED_PHPUNIT_CONSTRAINT, 1);

    let parent = path.parent().context("composer.json path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing composer.json: widened the phpunit/phpunit constraint to span \
         every major supporting the declared PHP floor"
    );
    Ok(true)
}

fn composer_package_name(config: &ResolvedCrateConfig, meta: &crate::scaffold::ScaffoldMeta) -> (String, String) {
    let Some(repository) = meta.configured_repository.as_deref() else {
        return ("unconfigured".to_string(), config.name.to_lowercase());
    };
    let repo = repository
        .strip_prefix("https://github.com/")
        .or_else(|| repository.strip_prefix("http://github.com/"))
        .filter(|s| !s.is_empty())
        .unwrap_or(repository);

    let parts: Vec<&str> = repo.split('/').collect();
    match parts.as_slice() {
        [owner, repo_name, ..] => (owner.to_lowercase(), repo_name.to_lowercase()),
        _ => ("unconfigured".to_string(), config.name.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::cfg::CfgPredicate;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    /// Regression: an `any(A, B)` predicate must need nothing when one arm is already active —
    /// it must never fall back to requesting *both* arms just because the other one wasn't
    /// checked. `native-http`/`wasm-http` are mutually exclusive transports in the core crate;
    /// requesting both would silently bloat the build with an unused transport stack.
    #[test]
    fn any_needs_nothing_when_one_arm_already_active() {
        let pred = CfgPredicate::Any(vec![
            CfgPredicate::Feature("native-http".to_string()),
            CfgPredicate::Feature("wasm-http".to_string()),
        ]);
        let active = set(&["native-http", "full"]);
        let core_defaults = set(&[]);
        assert_eq!(missing_features_for(&pred, &active, &core_defaults), BTreeSet::new());
    }

    /// When no arm is active, exactly one must be added — the core crate's own declared
    /// `default` arm — never both.
    #[test]
    fn any_with_no_arm_active_picks_the_core_crates_default_arm() {
        let pred = CfgPredicate::Any(vec![
            CfgPredicate::Feature("wasm-http".to_string()),
            CfgPredicate::Feature("native-http".to_string()),
        ]);
        let active = set(&[]);
        let core_defaults = set(&["native-http"]);
        assert_eq!(
            missing_features_for(&pred, &active, &core_defaults),
            set(&["native-http"])
        );
    }

    /// With no arm active and no core-declared default among the arms, fall back to the first
    /// arm — still exactly one, never the union.
    #[test]
    fn any_with_no_arm_active_and_no_default_picks_first_arm() {
        let pred = CfgPredicate::Any(vec![
            CfgPredicate::Feature("native-http".to_string()),
            CfgPredicate::Feature("wasm-http".to_string()),
        ]);
        let active = set(&[]);
        let core_defaults = set(&[]);
        assert_eq!(
            missing_features_for(&pred, &active, &core_defaults),
            set(&["native-http"])
        );
    }

    #[test]
    fn all_needs_the_union_of_every_missing_arm() {
        let pred = CfgPredicate::All(vec![
            CfgPredicate::Feature("a".to_string()),
            CfgPredicate::Feature("b".to_string()),
        ]);
        let active = set(&[]);
        let core_defaults = set(&[]);
        assert_eq!(missing_features_for(&pred, &active, &core_defaults), set(&["a", "b"]));
    }

    #[test]
    fn all_skips_arms_that_are_already_active() {
        let pred = CfgPredicate::All(vec![
            CfgPredicate::Feature("a".to_string()),
            CfgPredicate::Feature("b".to_string()),
        ]);
        let active = set(&["a"]);
        let core_defaults = set(&[]);
        assert_eq!(missing_features_for(&pred, &active, &core_defaults), set(&["b"]));
    }

    #[test]
    fn feature_already_active_needs_nothing() {
        let pred = CfgPredicate::Feature("tower".to_string());
        let active = set(&["tower"]);
        let core_defaults = set(&[]);
        assert_eq!(missing_features_for(&pred, &active, &core_defaults), BTreeSet::new());
    }

    /// `not(...)` can only be satisfied by *not* adding a feature — this function has no way to
    /// enforce that, so it must contribute nothing, regardless of the inner feature's state.
    #[test]
    fn not_contributes_nothing_regardless_of_active_set() {
        let pred = CfgPredicate::Not(Box::new(CfgPredicate::Feature("x".to_string())));
        let core_defaults = set(&[]);
        assert_eq!(missing_features_for(&pred, &set(&[]), &core_defaults), BTreeSet::new());
        assert_eq!(
            missing_features_for(&pred, &set(&["x"]), &core_defaults),
            BTreeSet::new()
        );
    }

    /// Regression for alef-task #374: an `excluded_default_features` name that gates no item in
    /// the extracted API surface (e.g. a Cargo-only feature that only affects a dependency's
    /// `build.rs` linking, such as `libheif-sys` via `heic`) is never discovered by
    /// `crate::codegen::cfg::collect_cfg_features`, which walks `#[cfg(feature = "X")]`
    /// attributes on IR nodes. The forwarding block carrying `default = [...]` and the
    /// forwarding lines was empty when discovery found nothing, so a config-only name never got
    /// its promised opt-in forwarding entry at all -- breaking `cargo build -p <crate>-php
    /// --features <name>` on desktop, exactly the escape hatch `excluded_default_features`
    /// documents as always available.
    #[test]
    fn scaffold_php_cargo_forwards_excluded_feature_not_referenced_by_any_cfg_attribute() {
        use crate::core::config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "sample-lib"
sources = []
[crates.php]
excluded_default_features = ["heic"]
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };

        let files = scaffold_php_cargo(&api, &config).expect("scaffold_php_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        assert!(
            cargo_toml.contains(r#"heic = ["sample-lib/heic"]"#),
            "a config-only excluded_default_features name (not referenced by any \
             #[cfg(feature = ...)] in the API surface) must still get a forwarding entry so \
             `cargo build --features heic` keeps working:\n{cargo_toml}"
        );
        let default_line = cargo_toml
            .lines()
            .find(|line| line.starts_with("default = ["))
            .expect("default array present");
        assert!(
            !default_line.contains("heic"),
            "default = [...] must NOT contain excluded `heic`; got: {default_line}"
        );
    }
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    fn composer_json_with_phpunit_constraint(constraint: &str) -> String {
        format!(
            "{{\n  \"name\": \"acme/example-php\",\n  \"description\": \"An example crate\",\n  \"type\": \"php-ext\",\n  \"require\": {{\n    \"php\": \">=8.2\"\n  }},\n  \"require-dev\": {{\n    {constraint}\n  }},\n  \"autoload\": {{\n    \"psr-4\": {{\n      \"Acme\\\\Example\\\\\": \"src/\"\n    }}\n  }},\n  \"php-ext\": {{\n    \"extension-name\": \"example\"\n  }}\n}}\n"
        )
    }

    #[test]
    fn should_widen_stale_phpunit_13_only_constraint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = composer_json_with_phpunit_constraint("\"phpunit/phpunit\": \"^13.1\"");
        std::fs::write(dir.path().join("composer.json"), &stale).expect("write stale composer.json");

        let relative_path = Path::new("composer.json");
        let changed = migrate_php_composer_phpunit_constraint(dir.path(), relative_path).expect("must not error");
        assert!(
            changed,
            "a composer.json pinned to the PHPUnit-13-only constraint must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(dir.path().join("composer.json")).expect("read migrated file");
        let parsed: serde_json::Value = serde_json::from_str(&on_disk).expect("migrated file must be valid JSON");
        assert_eq!(parsed["require-dev"]["phpunit/phpunit"], "^11.5 || ^12.0 || ^13.1");
        assert_eq!(
            parsed["require"]["php"], ">=8.2",
            "unrelated fields must survive untouched"
        );
        assert_eq!(
            parsed["autoload"]["psr-4"]["Acme\\Example\\"], "src/",
            "autoload must survive untouched"
        );

        let changed_again = migrate_php_composer_phpunit_constraint(dir.path(), relative_path).expect("must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_not_touch_a_composer_json_with_a_different_phpunit_constraint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let customised = composer_json_with_phpunit_constraint("\"phpunit/phpunit\": \"^10.0\"");
        std::fs::write(dir.path().join("composer.json"), &customised).expect("write customised composer.json");

        let relative_path = Path::new("composer.json");
        let changed = migrate_php_composer_phpunit_constraint(dir.path(), relative_path).expect("must not error");
        assert!(!changed, "a hand-chosen phpunit constraint must never be touched");

        let on_disk = std::fs::read_to_string(dir.path().join("composer.json")).expect("read file");
        assert_eq!(
            on_disk, customised,
            "customised composer.json must survive byte-for-byte"
        );
    }

    #[test]
    fn should_not_touch_a_foreign_composer_json_without_the_php_ext_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let foreign =
            "{\n  \"name\": \"someone/else\",\n  \"require-dev\": {\n    \"phpunit/phpunit\": \"^13.1\"\n  }\n}\n"
                .to_string();
        std::fs::write(dir.path().join("composer.json"), &foreign).expect("write foreign composer.json");

        let relative_path = Path::new("composer.json");
        let changed = migrate_php_composer_phpunit_constraint(dir.path(), relative_path).expect("must not error");
        assert!(
            !changed,
            "a composer.json without alef's php-ext marker must never be touched"
        );

        let on_disk = std::fs::read_to_string(dir.path().join("composer.json")).expect("read file");
        assert_eq!(on_disk, foreign);
    }

    #[test]
    fn migrate_php_composer_phpunit_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = Path::new("composer.json");
        let changed = migrate_php_composer_phpunit_constraint(dir.path(), relative_path).expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }
}
