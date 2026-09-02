use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::scaffold::naming::python_pip_name;
use crate::{
    scaffold::cargo_package_header, scaffold::core_dep_features, scaffold::detect_workspace_inheritance_for_crate,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use std::path::PathBuf;

/// e.g., "0.1.0-rc.1" -> "0.1.0rc1", "0.1.0-alpha.2" -> "0.1.0a2", "0.1.0-beta.3" -> "0.1.0b3"
/// Non-pre-release versions are returned unchanged.
fn to_pep440(version: &str) -> String {
    if let Some((base, pre)) = version.split_once('-') {
        let pep = pre
            .replace("alpha.", "a")
            .replace("alpha", "a")
            .replace("beta.", "b")
            .replace("beta", "b")
            .replace("rc.", "rc")
            .replace('.', "");
        format!("{base}{pep}")
    } else {
        version.to_string()
    }
}

/// pyproject-fmt's default `column_width` is 80 chars. Arrays whose inline
/// rendering (`prefix_len + "[ a, b ]".len()`) fits within this width are
/// emitted inline-with-inner-spaces (`[ "a", "b" ]`); otherwise they are
/// expanded to one-element-per-line with a trailing comma. Matching this rule
/// at emission time keeps prek's `pyproject-fmt` hook a no-op on every regen.
const PYPROJECT_FMT_COLUMN_WIDTH: usize = 80;

/// Format a list of pre-quoted TOML entries to match `pyproject-fmt`'s canonical
/// output. `prefix_len` is the on-screen column where the array opens (e.g.
/// `"keywords = ".len() == 11`); it is needed because pyproject-fmt picks
/// inline vs multi-line based on the total line length including the prefix.
///
/// - Empty: `[]`
/// - Inline form (`[ a, b, c ]`, inner spaces) when total length ≤
///   [`PYPROJECT_FMT_COLUMN_WIDTH`].
/// - Multi-line otherwise: 2-space indent, trailing comma after every element.
fn format_toml_array_with_prefix(entries: &[String], prefix_len: usize) -> String {
    if entries.is_empty() {
        return "[]".to_string();
    }
    let inline = format!("[ {} ]", entries.join(", "));
    if prefix_len + inline.len() <= PYPROJECT_FMT_COLUMN_WIDTH {
        return inline;
    }
    let inner = entries.iter().map(|e| format!("  {e},")).collect::<Vec<_>>().join("\n");
    format!("[\n{inner}\n]")
}

/// The `[tool.maturin] features` array, rendered from the same constant the binding crate's own
/// `[features]` table and the build command's `--features` flag are derived from.
///
/// maturin reads this array only when it resolves this `pyproject.toml` at all — which it does
/// for a wheel build rooted at the Python package directory, but not for the
/// `maturin develop --manifest-path crates/<crate>-py/Cargo.toml` alef runs from the repo root.
/// That is why the build path has to pass `--features` itself instead of relying on this. ~keep
fn maturin_extension_module_features() -> String {
    let entries: Vec<String> = crate::core::config::python_build::PYO3_EXTENSION_MODULE_PYO3_FEATURES
        .iter()
        .map(|feature| format!("\"{feature}\""))
        .collect();
    format_toml_array_with_prefix(&entries, "features = ".len())
}

/// Canonicalize a PEP 440 version specifier to `pyproject-fmt`'s normalized form.
///
/// `pyproject-fmt` strips redundant trailing `.0` release segments from each
/// version number in a specifier (e.g. `>=1.0,<2.0` → `>=1,<2`, `>=1.19.0`
/// → `>=1.19`). The renovate-tracked constants in [`crate::core::template_versions`]
/// keep their human-readable form; this normalizes them at emission time so the
/// generated `pyproject.toml` stays a no-op under the `pyproject-fmt` hook.
///
/// A trailing `.0` is only stripped when it is not the sole release segment — a
/// bare `0` (e.g. `==0`) is left untouched.
fn canonicalize_pep440_specifier(specifier: &str) -> String {
    specifier
        .split(',')
        .map(|clause| {
            let clause = clause.trim();
            let op_len = clause
                .char_indices()
                .find(|(_, c)| c.is_ascii_digit())
                .map(|(idx, _)| idx)
                .unwrap_or(clause.len());
            let (op, version) = clause.split_at(op_len);
            format!("{op}{}", canonicalize_pep440_version(version))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Strip redundant trailing `.0` release segments from a single PEP 440 version
/// number while preserving at least one release segment (`2.0` → `2`, `1.19.0`
/// → `1.19`, `0` → `0`). Pre/post/dev suffixes and local versions are left as-is.
fn canonicalize_pep440_version(version: &str) -> String {
    let release_len = version
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.'))
        .map(|(idx, _)| idx)
        .unwrap_or(version.len());
    let (release, suffix) = version.split_at(release_len);
    let mut segments: Vec<&str> = release.split('.').collect();
    while segments.len() > 1 && segments.last() == Some(&"0") {
        segments.pop();
    }
    format!("{}{}", segments.join("."), suffix)
}

pub(crate) fn scaffold_python_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let module_name = config.python_module_name();
    let core_crate_dir = config.core_crate_dir();
    let crate_dir = format!("crates/{core_crate_dir}-py");
    let ws = detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &crate_dir);
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-py"), version, "2024", &meta, &ws);

    let extra_deps = render_extra_deps(config, Language::Python);

    let has_trait_bridges = !config.trait_bridges.is_empty();
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    let mut all_deps = extra_deps;
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
    if (has_trait_bridges || has_streaming) && !all_deps.contains("tokio = ") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        let features = if has_streaming {
            "[\"rt-multi-thread\", \"sync\"]"
        } else {
            "[\"rt-multi-thread\"]"
        };
        all_deps.push_str(&format!("tokio = {{ version = \"1\", features = {features} }}"));
    }
    if has_streaming && !all_deps.contains("futures = ") && !all_deps.contains("futures =\"") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("futures = \"0.3\"");
    }
    let has_components = !config.components.is_empty();
    if has_components {
        let alef_version = env!("CARGO_PKG_VERSION");
        for (name, dependency) in [
            ("alef-component-abi", format!("alef-component-abi = \"{alef_version}\"")),
            (
                "alef-component-runtime",
                format!("alef-component-runtime = \"{alef_version}\""),
            ),
            ("directories", "directories = \"6\"".to_string()),
        ] {
            if crate::scaffold::cargo_dependency_declared(all_deps.lines(), name) {
                continue;
            }
            if !all_deps.is_empty() {
                all_deps.push('\n');
            }
            all_deps.push_str(&dependency);
        }
    }

    let extra_deps_section = if all_deps.is_empty() {
        String::new()
    } else {
        format!("\n{all_deps}")
    };
    let mut machete_ignored: Vec<&str> = vec!["pyo3-async-runtimes", "serde_json"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tracing");
    }
    if has_trait_bridges || has_streaming {
        machete_ignored.push("tokio");
    }
    if has_streaming {
        machete_ignored.push("futures");
    }
    let machete_ignored_str = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    // `[target.'cfg(...)'.dependencies]` blocks (core_dep_py is then empty).
    let core_overrides = config
        .python
        .as_ref()
        .map(|p| p.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let core_dep_path = config.core_crate_dep_path(std::path::Path::new(&crate_dir));
    let (core_dep_py, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &core_dep_path,
        &core_dep_features(config, Language::Python),
        version,
        core_overrides,
    );
    let core_target_blocks_section = if core_target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{core_target_blocks}")
    };
    let mut dep_entries: Vec<String> = vec![
        format!("pyo3 = {{ version = \"{}\" }}", tv::cargo::PYO3),
        format!(
            "pyo3-async-runtimes = {{ version = \"{}\", features = [\"tokio-runtime\"] }}",
            tv::cargo::PYO3_ASYNC_RUNTIMES
        ),
        "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
        "serde_json = \"1\"".to_string(),
    ];
    if !core_dep_py.is_empty() {
        dep_entries.push(core_dep_py.clone());
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

    // Forwards every feature name a generated `#[cfg(feature = "X")]` in this pyo3 crate's
    // source references (see `pyo3::gen_bindings`, which emits such gates on cfg'd-out
    // types/enums/fields/methods) into this crate's own `[features]` table, `<feature> =
    // ["<core>/<feature>"]`, plus a `default = [...]` line turning every one of them on. Without
    // this, rustc's `unexpected_cfgs` lint (denied under `-D warnings`) rejects every such gate:
    // this manifest previously declared only `extension-module`, never anything the core crate's
    // own gates could reference. Mirrors `scaffold_ruby_cargo`/`scaffold_node_cargo`/
    // `scaffold_php_cargo`, which have forwarded these features via the same
    // `codegen::cfg::cfg_default_and_forwarding_lines` helper since before pyo3 had any cfg
    // surface to gate at all -- pyo3 simply never grew this block alongside them (alef #464's
    // sibling gap). ~keep
    let cfg_features = crate::codegen::cfg::collect_cfg_features(api);
    let cfg_forwarding = if cfg_features.is_empty() {
        String::new()
    } else {
        let empty_exclusions = std::collections::HashSet::new();
        let lines =
            crate::codegen::cfg::cfg_default_and_forwarding_lines(&cfg_features, &config.name, &empty_exclusions);
        format!("\n{}\n", lines.join("\n"))
    };

    let lints_section = crate::scaffold::cargo_lints_section(config);
    let content = format!(
        r#"{pkg_header}

# `pyo3-async-runtimes` and `serde_json` are emitted unconditionally above so
# the manifest is stable across regens, but for umbrella crates with no
# async fns or no JSON-marshalled return types they are genuinely unused.
# The conditional `async-trait` / `tokio` / `futures` deps are similarly
# flagged when the umbrella has trait-bridge / streaming adapters configured
# but no actual async-trait / async callsite in the generated PyO3 shim.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[lib]
name = "{module_name}"
crate-type = ["cdylib"]

[features]
{extension_module_feature}
{cfg_forwarding}
[dependencies]
{dep_block}
{core_target_blocks_section}{lints_section}"#,
        extension_module_feature = crate::core::config::python_build::extension_module_feature_line(),
        pkg_header = pkg_header,
        lints_section = lints_section,
        module_name = module_name,
        dep_block = dep_block,
        core_target_blocks_section = core_target_blocks_section,
        machete_ignored_str = machete_ignored_str,
        cfg_forwarding = cfg_forwarding,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-py/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_python(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let pip_name = python_pip_name(config);
    let version = to_pep440(&api.version);
    let module_name = config.python_module_name();
    let core_crate_dir = config.core_crate_dir();
    let python_package = pip_name.replace('-', "_");
    let pkg_dir = config.package_dir(Language::Python);

    let authors_toml = if meta.authors.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.authors.iter().map(|a| format!("{{ name = \"{}\" }}", a)).collect();
        format!(
            "authors = {}\n",
            format_toml_array_with_prefix(&entries, "authors = ".len())
        )
    };

    let keywords_toml = if meta.keywords.is_empty() {
        String::new()
    } else {
        let mut sorted_keywords = meta.keywords.clone();
        sorted_keywords.sort();
        let entries: Vec<String> = sorted_keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(
            "keywords = {}\n",
            format_toml_array_with_prefix(&entries, "keywords = ".len())
        )
    };

    let homepage_toml = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage = \"{}\"\n", meta.homepage)
    };

    let dependencies_toml = match config.python.as_ref().map(|p| &p.pip_dependencies) {
        Some(deps) if !deps.is_empty() => {
            let entries: Vec<String> = deps.iter().map(|d| format!("\"{}\"", d)).collect();
            let inner = entries.iter().map(|e| format!("  {e},")).collect::<Vec<_>>().join("\n");
            format!("dependencies = [\n{inner}\n]\n")
        }
        _ => String::new(),
    };

    let sdist_include_toml = match config.python.as_ref().map(|p| &p.sdist_include) {
        Some(patterns) if !patterns.is_empty() => {
            let entries: Vec<String> = patterns.iter().map(|p| format!("\"{}\"", p)).collect();
            format!(
                "sdist-include = {}\n",
                format_toml_array_with_prefix(&entries, "sdist-include = ".len())
            )
        }
        _ => String::new(),
    };

    let urls_line = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("urls.repository = \"{repository}\"\n"))
        .unwrap_or_default();
    let license_toml = meta
        .license
        .as_deref()
        .map(|license| format!("license = \"{license}\"\nlicense-files = [ \"LICENSE\" ]\n"))
        .unwrap_or_default();

    // Only pyrefly (type-checker) is a real dev dependency: poly bundles ruff for
    // lint+format, so the generated package must not pull in a standalone ruff.
    let dev_group_entries = [format!(
        "\"pyrefly{}\"",
        canonicalize_pep440_specifier(tv::pypi::PYREFLY)
    )];
    let dev_group_array = format_toml_array_with_prefix(&dev_group_entries, "dev = ".len());

    let pyrefly_extra = config
        .poly
        .pyrefly_sub_configs
        .iter()
        .map(|(glob, codes)| {
            let errors = codes.iter().map(|code| format!("{code} = false\n")).collect::<String>();
            format!("\n[[tool.pyrefly.sub-config]]\nmatches = \"{glob}\"\n[tool.pyrefly.sub-config.errors]\n{errors}")
        })
        .collect::<String>();

    let content = format!(
        r#"[build-system]
build-backend = "maturin"
requires = [ "{maturin_build_requires}" ]

[project]
name = "{pip_name}"
version = "{version}"
description = "{description}"
{keywords}{license_toml}{authors}requires-python = ">=3.10"
classifiers = [
  "Programming Language :: Python :: 3 :: Only",
  "Programming Language :: Python :: 3.10",
  "Programming Language :: Python :: 3.11",
  "Programming Language :: Python :: 3.12",
  "Programming Language :: Python :: 3.13",
  "Programming Language :: Python :: 3.14",
]
{dependencies}{urls_line}{homepage}
[dependency-groups]
dev = {dev_group}

[tool.maturin]
profile = "release"
module-name = "{python_package}.{module_name}"
manifest-path = "../../crates/{crate_dir}-py/Cargo.toml"
# abi3-py310 produces a single wheel per platform that loads on Python 3.10+,
# avoiding a per-Python-version build matrix.
features = {maturin_features}
python-packages = [ "{python_package}" ]
{sdist_include}
[tool.pyrefly]
python-version = "3.10"
preset = "strict"
# `bad-return` and `bad-argument-type` used to be suppressed here for every generated
# `api.py`: the wrapper's declared type (an `options.*` dataclass) and the value pyo3
# actually returned/accepted (the native `_internal_bindings.*` pyclass) disagreed on
# every boundary crossing, and pyrefly correctly rejected all of them. The codegen now
# emits the `_to_rust_*` / `_from_native_*` conversions those boundaries need (alef-310),
# so both codes are gone from this list.
#
# The three below were re-audited (alef-334) against
# `bin_cli::all_commands::pyrefly_generated_package_tests`'s fixture, extended to cover
# each code's most plausible generated shape: a multi-parameter native-constructor call in
# `_to_rust_filter` (bad-argument-count), a `Vec<enum>` field routed through the
# `[_coerce_enum(_rust.X, v) for v in value.field]` comprehension (not-iterable), and a
# nested options-dataclass chain in `_to_rust_person` (missing-attribute). With this
# suppression lifted, that fixture's real generated `api.py` type-checks clean under real
# pyrefly 1.2.0. Hand-corrupting those exact call sites (an extra positional arg; an
# iteration over a non-iterable; a typoed attribute) reliably reproduces, respectively:
#   `Expected 3 positional arguments, got 4 in function `test_lib.test_lib.Filter.__init__`
#   [bad-argument-count]`
#   `Type `Literal[5]` is not iterable [not-iterable]`
#   `Object of class `Person` has no attribute `addresss` [missing-attribute]`
# proving the gate can and does catch each code -- a clean run is not vacuous. No defect
# was found under the shapes that fixture covers, but the pyo3 backend's less-common
# surfaces (service_api decorators, trait_bridge visitors, streaming adapters, capsule
# types) were not exercised by this audit; do not fold these back in without extending
# that fixture to cover whichever surface prompted the re-audit.
[[tool.pyrefly.sub-config]]
matches = "**/api.py"
[tool.pyrefly.sub-config.errors]
bad-argument-count = false
not-iterable = false
missing-attribute = false
{pyrefly_extra}"#,
        pip_name = pip_name,
        version = version,
        description = meta.description,
        license_toml = license_toml,
        authors = authors_toml,
        keywords = keywords_toml,
        homepage = homepage_toml,
        dependencies = dependencies_toml,
        sdist_include = sdist_include_toml,
        urls_line = urls_line,
        python_package = python_package,
        module_name = module_name,
        crate_dir = core_crate_dir,
        maturin_build_requires = canonicalize_pep440_specifier(tv::pypi::MATURIN_BUILD_REQUIRES),
        maturin_features = maturin_extension_module_features(),
        dev_group = dev_group_array,
        pyrefly_extra = pyrefly_extra,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/pyproject.toml")),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/{python_package}/py.typed")),
            content: String::new(),
            generated_header: false,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_pep440_specifier, canonicalize_pep440_version, scaffold_python_cargo};
    use crate::core::config::ComponentProfileConfig;
    use crate::core::ir::ApiSurface;

    /// `pyproject-fmt` strips redundant trailing `.0` release segments from a
    /// single version number, keeping at least one segment.
    #[test]
    fn canonicalize_version_strips_redundant_trailing_zero() {
        assert_eq!(canonicalize_pep440_version("2.0"), "2");
        assert_eq!(canonicalize_pep440_version("1.19.0"), "1.19");
        assert_eq!(canonicalize_pep440_version("1.0.0"), "1");
        assert_eq!(canonicalize_pep440_version("1.19"), "1.19");
        assert_eq!(canonicalize_pep440_version("0.14.8"), "0.14.8");
        assert_eq!(canonicalize_pep440_version("0"), "0");
        assert_eq!(canonicalize_pep440_version("1.0rc1"), "1rc1");
    }

    /// Multi-clause specifiers are normalized clause-by-clause and rejoined with
    /// a bare comma (no space), matching `pyproject-fmt`'s output.
    #[test]
    fn canonicalize_specifier_handles_comparison_operators_and_clauses() {
        assert_eq!(canonicalize_pep440_specifier(">=1.0,<2.0"), ">=1,<2");
        assert_eq!(canonicalize_pep440_specifier(">=1.19.0"), ">=1.19");
        assert_eq!(canonicalize_pep440_specifier(">=0.14.8"), ">=0.14.8");
        assert_eq!(canonicalize_pep440_specifier("==1.0"), "==1");
        assert_eq!(canonicalize_pep440_specifier(">=1.0, <2.0"), ">=1,<2");
    }

    #[test]
    fn component_runtime_dependencies_follow_alef_not_core_version() {
        let api = ApiSurface {
            version: "9.9.9".into(),
            ..ApiSurface::default()
        };
        let config = crate::core::config::ResolvedCrateConfig {
            name: "demo".into(),
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["x86_64-unknown-linux-gnu".into()],
            }],
            ..Default::default()
        };

        let manifest = scaffold_python_cargo(&api, &config).unwrap().remove(0).content;
        assert!(manifest.contains(&format!("alef-component-runtime = \"{}\"", env!("CARGO_PKG_VERSION"))));
        assert!(!manifest.contains("alef-component-runtime = \"9.9.9\""));
    }

    #[test]
    fn component_runtime_dependencies_do_not_duplicate_user_overrides() {
        let api = ApiSurface::default();
        let config = crate::core::config::ResolvedCrateConfig {
            name: "demo".into(),
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["x86_64-unknown-linux-gnu".into()],
            }],
            extra_dependencies: std::collections::HashMap::from([
                ("alef-component-abi".into(), toml::Value::String("0.58.1".into())),
                ("alef-component-runtime".into(), toml::Value::String("0.58.1".into())),
                ("directories".into(), toml::Value::String("6".into())),
            ]),
            ..Default::default()
        };

        let manifest = scaffold_python_cargo(&api, &config).unwrap().remove(0).content;
        for dependency in ["alef-component-abi", "alef-component-runtime", "directories"] {
            assert_eq!(manifest.matches(&format!("{dependency} =")).count(), 1);
        }
    }
}

/// The scaffold declares the extension-module feature; the build activates it. Nothing proved
/// the two named the same thing, and for one release they did not: the build named nothing at
/// all. This checks the agreement the only way that cannot go stale -- by running the build's own
/// probe against the manifest the scaffold just wrote. ~keep
#[cfg(test)]
mod extension_module_agreement_tests {
    use super::{maturin_extension_module_features, scaffold_python, scaffold_python_cargo};
    use crate::core::config::NewAlefConfig;
    use crate::core::config::python_build::{PYO3_EXTENSION_MODULE_FEATURE, declared_extension_module_feature};
    use crate::core::ir::ApiSurface;

    fn resolved() -> crate::core::config::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
"#,
        )
        .expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn api() -> ApiSurface {
        ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn the_build_probe_finds_the_feature_the_scaffolded_manifest_declares() {
        let files = scaffold_python_cargo(&api(), &resolved()).expect("scaffold_python_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, cargo_toml).expect("write manifest");

        assert_eq!(
            declared_extension_module_feature(&manifest),
            Some(PYO3_EXTENSION_MODULE_FEATURE),
            "the feature the build passes must be one this manifest declares:\n{cargo_toml}"
        );
    }

    #[test]
    fn the_pyproject_requests_the_pyo3_features_the_crate_feature_turns_on() {
        let files = scaffold_python(&api(), &resolved()).expect("scaffold_python ok");
        let pyproject = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("pyproject.toml"))
            .expect("pyproject.toml emitted")
            .content;

        assert!(
            pyproject.contains(&format!("features = {}", maturin_extension_module_features())),
            "the maturin feature list must stay derived from the same constant:\n{pyproject}"
        );
    }
}

/// Regression: the pyo3 crate's `[features]` table declared only `extension-module`, never
/// forwarding any `#[cfg(feature = "X")]`-referenced name into `<core>/<feature>` the way
/// `scaffold_ruby_cargo`/`scaffold_node_cargo`/`scaffold_php_cargo` already do. rustc's
/// `unexpected_cfgs` lint (denied under `-D warnings`) then rejects every generated
/// `#[cfg(feature = "X")]` gate the pyo3 backend emits for a cfg-gated function/type/enum, since
/// nothing in this manifest declares `X` at all.
#[cfg(test)]
mod cfg_feature_forwarding_tests {
    use super::scaffold_python_cargo;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};

    #[test]
    fn scaffold_python_cargo_forwards_a_cfg_referenced_feature_into_default() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            functions: vec![FunctionDef {
                name: "tokenize".to_string(),
                rust_path: "sample_lib::tokenize".to_string(),
                return_type: TypeRef::Unit,
                cfg: Some(r#"feature = "chunking-tokenizers""#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let files = scaffold_python_cargo(&api, &config).expect("scaffold_python_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        assert!(
            cargo_toml.contains(r#"chunking-tokenizers = ["sample-lib/chunking-tokenizers"]"#),
            "a cfg-referenced feature must get a forwarding entry to the core crate so the \
             generated #[cfg(feature = \"chunking-tokenizers\")] gate resolves against a \
             declared feature instead of tripping unexpected_cfgs under -D warnings:\n{cargo_toml}"
        );
        let default_line = cargo_toml
            .lines()
            .find(|line| line.trim_start().starts_with("default = ["))
            .unwrap_or_else(|| panic!("no default = [...] line in:\n{cargo_toml}"));
        assert!(
            default_line.contains("\"chunking-tokenizers\""),
            "the forwarded feature must also be turned on by default (no build wrapper here \
             passes --features): {default_line}"
        );
        assert!(
            cargo_toml.contains(r#"extension-module = ["pyo3/extension-module", "pyo3/abi3-py310"]"#),
            "the pre-existing extension-module feature must be preserved:\n{cargo_toml}"
        );
    }

    /// Negative control: an API surface with no cfg-gated item must not grow a forwarding block,
    /// matching the pre-existing shape of this manifest.
    #[test]
    fn scaffold_python_cargo_emits_no_forwarding_block_when_nothing_is_cfg_gated() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };

        let files = scaffold_python_cargo(&api, &config).expect("scaffold_python_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        assert!(
            !cargo_toml.contains("default = ["),
            "no [features] forwarding block should appear when nothing is cfg-gated:\n{cargo_toml}"
        );
    }

    /// NATIVE CONTROL for the wasm mio leak (see
    /// `backends::wasm::gen_bindings::cargo_feature_leak_tests`). The wasm fix narrows
    /// `codegen::cfg::core_default_features_active` for `Language::Wasm` only, so this native
    /// (PyO3) manifest must be untouched: every cfg-referenced feature still gets a forwarding row
    /// AND is still defaulted on, `native-http` included. A fix that stripped the native feature
    /// set globally -- or that narrowed `collect_cfg_features`/`cfg_default_and_forwarding_lines`
    /// instead of the wasm-only dependency-edge predicate -- would break every native binding
    /// while the wasm assertions still passed. ~keep
    #[test]
    fn scaffold_python_cargo_keeps_defaulting_a_native_only_feature_on() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            functions: vec![FunctionDef {
                name: "request".to_string(),
                rust_path: "sample_lib::request".to_string(),
                return_type: TypeRef::Unit,
                cfg: Some(r#"any(feature = "native-http", feature = "wasm-http")"#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let files = scaffold_python_cargo(&api, &config).expect("scaffold_python_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        let default_line = cargo_toml
            .lines()
            .find(|line| line.trim_start().starts_with("default = ["))
            .unwrap_or_else(|| panic!("no default = [...] line in:\n{cargo_toml}"));
        assert_eq!(
            default_line.trim(),
            r#"default = ["native-http", "wasm-http"]"#,
            "the native manifest must keep defaulting every cfg-referenced feature on -- the wasm \
             fix is a per-target divergence, not a global downgrade:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(r#"native-http = ["sample-lib/native-http"]"#),
            "the native-only forwarding row must survive:\n{cargo_toml}"
        );
    }
}
