use crate::naming::python_pip_name;
use crate::{
    cargo_package_header, core_dep_features, detect_workspace_inheritance, render_extra_deps, scaffold_meta, to_pep440,
};
use alef_core::backend::GeneratedFile;
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
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
    let mut all_deps = extra_deps;
    if has_trait_bridges && !all_deps.contains("async-trait") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("async-trait = \"0.1\"");
    }
    if has_trait_bridges && !all_deps.contains("tokio = ") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("tokio = { version = \"1\", features = [\"rt-multi-thread\"] }");
    }

    let extra_deps_section = if all_deps.is_empty() {
        String::new()
    } else {
        format!("\n{all_deps}")
    };
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

[features]
extension-module = ["pyo3/extension-module"]

[lints]
workspace = true
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
        let entries: Vec<String> = meta
            .authors
            .iter()
            .map(|a| format!("    {{ name = \"{}\" }}", a))
            .collect();
        format!("authors = [\n{}\n]\n", entries.join(",\n"))
    };

    let keywords_toml = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!("keywords = [{}]\n", entries.join(", "))
    };

    let homepage_toml = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage = \"{}\"\n", meta.homepage)
    };

    let content = format!(
        r#"[build-system]
requires = ["{maturin_build_requires}"]
build-backend = "maturin"

[project]
name = "{pip_name}"
version = "{version}"
description = "{description}"
license = "{license}"
requires-python = ">=3.10"
classifiers = [
  "Programming Language :: Python :: 3 :: Only",
  "Programming Language :: Python :: 3.10",
  "Programming Language :: Python :: 3.11",
  "Programming Language :: Python :: 3.12",
  "Programming Language :: Python :: 3.13",
  "Programming Language :: Python :: 3.14",
]
{authors}{keywords}{homepage}[project.urls]
repository = "{repository}"

[tool.maturin]
module-name = "{python_package}.{module_name}"
manifest-path = "../../crates/{crate_dir}-py/Cargo.toml"
features = ["pyo3/extension-module"]
python-packages = ["{python_package}"]

[dependency-groups]
dev = ["ruff{ruff}", "mypy{mypy}"]

[tool.ruff]
target-version = "py310"
line-length = 120

[tool.ruff.lint]
select = ["ALL"]
ignore = [
  "ANN401", "ASYNC109", "ASYNC110", "BLE001", "COM812",
  "D100", "D104", "D107", "D205", "E501", "EM",
  "FBT", "FIX", "ISC001", "PD011", "PGH003", "PLR2004",
  "PLW0603", "S104", "S110", "S603", "TD", "TRY",
]

[tool.ruff.lint.mccabe]
max-complexity = 15

[tool.ruff.lint.pylint]
max-args = 10
max-branches = 15
max-returns = 10

[tool.ruff.lint.pydocstyle]
convention = "google"

[tool.ruff.lint.per-file-ignores]
"tests/**" = ["S101", "D103", "ANN", "PLR2004"]

[tool.ruff.format]
docstring-code-line-length = 120
docstring-code-format = true

[tool.mypy]
python_version = "3.10"
strict = true
show_error_codes = true
implicit_reexport = false
namespace_packages = true
"#,
        pip_name = pip_name,
        version = version,
        description = meta.description,
        license = meta.license,
        authors = authors_toml,
        keywords = keywords_toml,
        homepage = homepage_toml,
        repository = meta.repository,
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
            content: String::new(),
            generated_header: false,
        },
    ])
}
