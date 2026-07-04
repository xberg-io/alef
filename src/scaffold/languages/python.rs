use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::scaffold::naming::python_pip_name;
use crate::{
    scaffold::cargo_package_header, scaffold::core_dep_features, scaffold::detect_workspace_inheritance,
    scaffold::render_extra_deps, scaffold::scaffold_meta, scaffold::to_pep440,
};
use std::path::PathBuf;

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
    // Split into comma-separated clauses, normalize each, rejoin without spaces
    // (pyproject-fmt emits `>=1,<2`, not `>=1, <2`).
    specifier
        .split(',')
        .map(|clause| {
            let clause = clause.trim();
            // Separate the comparison operator prefix from the version number.
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
    // Only touch the leading release segment (digits and dots before any
    // pre/post/dev/local marker); leave the remainder untouched.
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
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
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

    let extra_deps_section = if all_deps.is_empty() {
        String::new()
    } else {
        format!("\n{all_deps}")
    };
    // Build the cargo-machete ignored list. `pyo3-async-runtimes` and
    // `serde_json` are emitted unconditionally above so they are always
    // ignored. Conditional deps (`async-trait` / `tokio` for trait bridges
    // and streaming, `futures` for streaming) are appended only when the
    // scaffold actually adds them to `[dependencies]`, so cargo-machete
    // doesn't flap on umbrellas whose API surface doesn't exercise the
    // trait-bridge / streaming codepath.
    let mut machete_ignored: Vec<&str> = vec!["pyo3-async-runtimes", "serde_json"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
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
    // Build [dependencies] block alphabetically sorted to match cargo-sort.
    // When target_dep_overrides are configured, the core dep moves into
    // `[target.'cfg(...)'.dependencies]` blocks (core_dep_py is then empty).
    let core_overrides = config
        .python
        .as_ref()
        .map(|p| p.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let (core_dep_py, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &format!("../{core_crate_dir}"),
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
    dep_entries.sort();
    let dep_block = dep_entries.join("\n");
    let _ = extra_deps_section;

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
extension-module = ["pyo3/extension-module", "pyo3/abi3-py310"]

[dependencies]
{dep_block}
{core_target_blocks_section}
"#,
        pkg_header = pkg_header,
        module_name = module_name,
        dep_block = dep_block,
        core_target_blocks_section = core_target_blocks_section,
        machete_ignored_str = machete_ignored_str,
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
            // Force multi-line to match pyproject-fmt's output (dependencies always wraps)
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

    let dev_group_entries = [
        format!("\"pyrefly{}\"", canonicalize_pep440_specifier(tv::pypi::PYREFLY)),
        format!("\"ruff{}\"", canonicalize_pep440_specifier(tv::pypi::RUFF)),
    ];
    let dev_group_array = format_toml_array_with_prefix(&dev_group_entries, "dev = ".len());

    // Extra `[[tool.pyrefly.sub-config]]` blocks from `[workspace.poly.pyrefly-sub-configs]`.
    // Keyed by glob, each value is the list of pyrefly error codes to disable for
    // extension-generated modules whose runtime-reconciled pyo3 boundaries a static
    // checker cannot follow (same rationale as the built-in api.py sub-config).
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
features = [ "pyo3/extension-module", "pyo3/abi3-py310" ]
python-packages = [ "{python_package}" ]
{sdist_include}
[tool.pyrefly]
python-version = "3.10"
preset = "strict"
# The alef-emitted `api.py` wrapper has a structural mismatch between its
# `options.*` dataclass signatures and the `_internal_bindings.*` pyclass types
# pyo3 accepts/returns at runtime. pyo3 reconciles them dynamically via
# FromPyObject — the Python e2e suite exercises the runtime path — but a static
# checker sees only the discrepancy. Suppress the errors it raises on the
# wrapper until the codegen emits matching `_to_rust_*` calls and casts the
# return values.
[[tool.pyrefly.sub-config]]
matches = "**/api.py"
[tool.pyrefly.sub-config.errors]
bad-argument-type = false
bad-argument-count = false
bad-return = false
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
            // Empty (0 bytes): end-of-file-fixer leaves a 0-byte file alone, but strips a
            // file whose sole content is a trailing newline back to empty — emitting "\n"
            // here causes churn on every regen. py.typed is a PEP 561 marker; empty is correct.
            content: String::new(),
            generated_header: false,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_pep440_specifier, canonicalize_pep440_version};

    /// `pyproject-fmt` strips redundant trailing `.0` release segments from a
    /// single version number, keeping at least one segment.
    #[test]
    fn canonicalize_version_strips_redundant_trailing_zero() {
        assert_eq!(canonicalize_pep440_version("2.0"), "2");
        assert_eq!(canonicalize_pep440_version("1.19.0"), "1.19");
        assert_eq!(canonicalize_pep440_version("1.0.0"), "1");
        // Non-redundant segments are preserved.
        assert_eq!(canonicalize_pep440_version("1.19"), "1.19");
        assert_eq!(canonicalize_pep440_version("0.14.8"), "0.14.8");
        // A bare zero is the sole segment and must be kept.
        assert_eq!(canonicalize_pep440_version("0"), "0");
        // Suffixes (pre/post/dev/local) are left untouched.
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
        // Surrounding whitespace inside clauses is normalized away.
        assert_eq!(canonicalize_pep440_specifier(">=1.0, <2.0"), ">=1,<2");
    }
}
