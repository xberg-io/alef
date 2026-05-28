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
    let content = format!(
        r#"{pkg_header}

[lib]
name = "{module_name}"
crate-type = ["cdylib"]

[dependencies]
{core_dep}
pyo3 = {{ version = "{pyo3}" }}
pyo3-async-runtimes = {{ version = "{pyo3_async_runtimes}", features = ["tokio-runtime"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"{extra_deps_section}

# `pyo3-async-runtimes` and `serde_json` are emitted unconditionally above so
# the manifest is stable across regens, but for umbrella crates with no
# async fns or no JSON-marshalled return types they are genuinely unused.
# The conditional `async-trait` / `tokio` / `futures` deps are similarly
# flagged when the umbrella has trait-bridge / streaming adapters configured
# but no actual async-trait / async callsite in the generated PyO3 shim.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[features]
extension-module = ["pyo3/extension-module", "pyo3/abi3-py310"]

"#,
        pkg_header = pkg_header,
        module_name = module_name,
        core_dep = crate::scaffold::render_core_dep(
            &config.name,
            &format!("../{core_crate_dir}"),
            &core_dep_features(config, Language::Python),
            version,
        ),
        pyo3 = tv::cargo::PYO3,
        pyo3_async_runtimes = tv::cargo::PYO3_ASYNC_RUNTIMES,
        extra_deps_section = extra_deps_section,
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

    let urls_line = format!("urls.repository = \"{}\"\n", meta.repository);

    let dev_group_entries = [
        format!("\"mypy{}\"", canonicalize_pep440_specifier(tv::pypi::MYPY)),
        format!("\"ruff{}\"", canonicalize_pep440_specifier(tv::pypi::RUFF)),
    ];
    let dev_group_array = format_toml_array_with_prefix(&dev_group_entries, "dev = ".len());

    // The `disable_error_code` array lives inside an inline table inside the
    // `overrides = [...]` array, so the on-disk prefix exceeds pyproject-fmt's
    // 80-char column width and the array would always wrap. pyproject-fmt
    // keeps the array inline regardless (matching its handling of nested
    // inline tables), so render it inline with inner spaces directly.
    let mypy_disable_codes = format!(
        "[ {} ]",
        ["\"call-arg\"", "\"arg-type\"", "\"return-value\"", "\"attr-defined\""].join(", ")
    );

    let content = format!(
        r#"[build-system]
build-backend = "maturin"
requires = [ "{maturin_build_requires}" ]

[project]
name = "{pip_name}"
version = "{version}"
description = "{description}"
{keywords}license = "{license}"
license-files = [ "LICENSE" ]
{authors}requires-python = ">=3.10"
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
module-name = "{python_package}.{module_name}"
manifest-path = "../../crates/{crate_dir}-py/Cargo.toml"
# abi3-py310 produces a single wheel per platform that loads on Python 3.10+,
# avoiding a per-Python-version build matrix.
features = [ "pyo3/extension-module", "pyo3/abi3-py310" ]
python-packages = [ "{python_package}" ]

[tool.ruff]
target-version = "py310"
line-length = 120
format.docstring-code-line-length = 120
format.docstring-code-format = true
lint.select = [ "ALL" ]
lint.ignore = [
  "ANN401",
  "ASYNC109",
  "ASYNC110",
  "BLE001",
  "COM812",
  "D100",
  "D104",
  "D107",
  "D205",
  "E501",
  "EM",
  "FBT",
  "FIX",
  "ISC001",
  "PD011",
  "PGH003",
  "PLR2004",
  "PLW0603",
  "S104",
  "S110",
  "S603",
  "TD",
  "TRY",
]
lint.per-file-ignores."{python_package}/__init__.py" = [ "I001" ]
# The alef Python codegen still emits cosmetic warnings on the wrapper
# modules: api.py keeps the legacy `from typing import AsyncIterator` and a
# single-line import block, options.py carries # noqa: TC001 / F401 markers
# that turn out unused on every regen, __init__.py star-imports re-sort with
# a different convention. Silence these specific rules on the wrappers until
# the codegen is updated to emit ruff-clean output.
lint.per-file-ignores."{python_package}/api.py" = [ "F401", "I001", "UP035" ]
lint.per-file-ignores."{python_package}/options.py" = [ "F401", "RUF100" ]
lint.per-file-ignores."tests/**" = [ "ANN", "D103", "PLR2004", "S101" ]
lint.mccabe.max-complexity = 15
lint.pydocstyle.convention = "google"
lint.pylint.max-args = 10
lint.pylint.max-branches = 15
lint.pylint.max-returns = 10

[tool.mypy]
python_version = "3.10"
strict = true
show_error_codes = true
implicit_reexport = false
namespace_packages = true
overrides = [
  # The alef-emitted `api.py` wrapper has a structural mismatch between its
  # `options.*` dataclass signatures and the `_internal_bindings.*` pyclass
  # types pyo3 accepts/returns at runtime. pyo3 reconciles them dynamically via
  # FromPyObject — the Python e2e suite exercises the runtime path — but mypy
  # sees only the static-type discrepancy. Disable the four error codes the
  # discrepancy raises until the codegen emits matching `_to_rust_*` calls and
  # casts the return values.
  {{ module = "{python_package}.api", disable_error_code = {mypy_disable_codes} }},
]
"#,
        pip_name = pip_name,
        version = version,
        description = meta.description,
        license = meta.license,
        authors = authors_toml,
        keywords = keywords_toml,
        homepage = homepage_toml,
        dependencies = dependencies_toml,
        urls_line = urls_line,
        python_package = python_package,
        module_name = module_name,
        crate_dir = core_crate_dir,
        maturin_build_requires = canonicalize_pep440_specifier(tv::pypi::MATURIN_BUILD_REQUIRES),
        dev_group = dev_group_array,
        mypy_disable_codes = mypy_disable_codes,
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
