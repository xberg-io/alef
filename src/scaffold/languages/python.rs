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

pub(crate) fn scaffold_python_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let module_name = config.python_module_name();
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-py"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

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
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
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
        crate_name = &config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Python),
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
        format!("authors = [ {} ]\n", entries.join(", "))
    };

    let keywords_toml = if meta.keywords.is_empty() {
        String::new()
    } else {
        let mut sorted_keywords = meta.keywords.clone();
        sorted_keywords.sort();
        let entries: Vec<String> = sorted_keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!("keywords = [ {} ]\n", entries.join(", "))
    };

    let homepage_toml = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage = \"{}\"\n", meta.homepage)
    };

    let dependencies_toml = match config.python.as_ref().map(|p| &p.pip_dependencies) {
        Some(deps) if !deps.is_empty() => {
            let entries: Vec<String> = deps.iter().map(|d| format!("\"{}\"", d)).collect();
            format!("dependencies = [ {} ]\n", entries.join(", "))
        }
        _ => String::new(),
    };

    let urls_line = format!("urls.repository = \"{}\"\n", meta.repository);

    let content = format!(
        r#"[build-system]
build-backend = "maturin"
requires = [ "{maturin_build_requires}" ]

[project]
name = "{pip_name}"
version = "{version}"
description = "{description}"
{keywords}license = "{license}"
{authors}requires-python = ">=3.10"
classifiers = [
  "Programming Language :: Python :: 3 :: Only",
  "Programming Language :: Python :: 3.10",
  "Programming Language :: Python :: 3.11",
  "Programming Language :: Python :: 3.12",
  "Programming Language :: Python :: 3.13",
  "Programming Language :: Python :: 3.14",
]
{urls_line}{homepage}{dependencies}[dependency-groups]
dev = [ "mypy{mypy}", "ruff{ruff}" ]

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
format.docstring-code-format = true
format.docstring-code-line-length = 120
lint.select = [ "ALL" ]
lint.ignore = [
  "ANN401", "ASYNC109", "ASYNC110", "BLE001", "COM812",
  "D100", "D104", "D107", "D205", "E501", "EM",
  "FBT", "FIX", "ISC001", "PD011", "PGH003", "PLR2004",
  "PLW0603", "S104", "S110", "S603", "TD", "TRY",
]
lint.mccabe.max-complexity = 15
lint.per-file-ignores."tests/**" = [ "ANN", "D103", "PLR2004", "S101" ]
# The alef Python codegen still emits cosmetic warnings on the wrapper
# modules: api.py keeps the legacy `from typing import AsyncIterator` and a
# single-line import block, options.py carries # noqa: TC001 / F401 markers
# that turn out unused on every regen, __init__.py star-imports re-sort with
# a different convention. Silence these specific rules on the wrappers until
# the codegen is updated to emit ruff-clean output.
lint.per-file-ignores."{python_package}/api.py" = [ "F401", "I001", "UP035" ]
lint.per-file-ignores."{python_package}/options.py" = [ "F401", "RUF100" ]
lint.per-file-ignores."{python_package}/__init__.py" = [ "I001" ]
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

# The alef-emitted `api.py` wrapper has a structural mismatch between its
# `options.*` dataclass signatures and the `_internal_bindings.*` pyclass
# types pyo3 accepts/returns at runtime. pyo3 reconciles them dynamically via
# FromPyObject — the Python e2e suite exercises the runtime path — but mypy
# sees only the static-type discrepancy. Disable the four error codes the
# discrepancy raises until the codegen emits matching `_to_rust_*` calls and
# casts the return values.
[[tool.mypy.overrides]]
module = "{python_package}.api"
disable_error_code = [ "call-arg", "arg-type", "return-value", "attr-defined" ]
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
        maturin_build_requires = tv::pypi::MATURIN_BUILD_REQUIRES,
        ruff = tv::pypi::RUFF,
        mypy = tv::pypi::MYPY,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/pyproject.toml")),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/{python_package}/py.typed")),
            content: "\n".to_string(),
            generated_header: false,
        },
    ])
}
