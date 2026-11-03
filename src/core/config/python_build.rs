//! One source of truth for how alef invokes maturin against the generated pyo3 crate.
//!
//! Three surfaces used to restate the same two facts independently, and the build path was the
//! one that disagreed with the other two:
//!
//! - `scaffold::languages::python` writes `crates/<crate>-py/Cargo.toml` declaring an
//!   `extension-module` feature *outside* `default`, and a `pyproject.toml` whose
//!   `[tool.maturin] features` requests the pyo3 features that feature turns on.
//! - `core::config::build_defaults` and `cli::pipeline::commands::build::build_command` each
//!   built a `maturin develop` command that activated neither, and resolved a bare `maturin`
//!   off `PATH` regardless of what `[workspace.tools] python_package_manager` selected.
//!
//! Everything both build paths need to agree with the scaffold lives here. ~keep

use super::tools::ToolsConfig;
use serde::Serialize;
use std::path::Path;

const SUPPORTED_PYTHON_MINIMUMS: &[(&str, &str)] = &[
    ("3.11", "pyo3/abi3-py311"),
    ("3.12", "pyo3/abi3-py312"),
    ("3.13", "pyo3/abi3-py313"),
    ("3.14", "pyo3/abi3-py314"),
];

pub(crate) const PYTHON_RUNTIME_VERSION_KEY: &str = "runtime:python";
pub(crate) const PYTHON_MINIMUM_VERSION: &str = SUPPORTED_PYTHON_MINIMUMS[0].0;

/// The feature name the generated pyo3 binding crate declares in its own `[features]` table to
/// switch pyo3 into extension-module (and stable-ABI) mode. Emitted by
/// `scaffold::languages::python`; read back by both build paths so the flag they pass names
/// exactly the feature the manifest defines rather than a literal copy of it. ~keep
pub const PYO3_EXTENSION_MODULE_FEATURE: &str = "extension-module";

/// The pyo3 features the crate-level [`PYO3_EXTENSION_MODULE_FEATURE`] turns on.
///
/// `pyo3/extension-module` is what stops pyo3's build script from linking libpython
/// (`pyo3_build_config::is_linking_libpython_for_target` returns true for *every* target when the
/// feature is off) and, on Darwin, what adds `-undefined dynamic_lookup`. `pyo3/abi3-py311` is
/// what makes the artifact stable-ABI, so one wheel per platform covers Python 3.11+. Neither is
/// on by default, so a build that omits the feature silently produces a libpython-linked,
/// version-locked module wherever the link happens to succeed. ~keep
pub const PYO3_EXTENSION_MODULE_PYO3_FEATURES: &[&str] = &["pyo3/extension-module", SUPPORTED_PYTHON_MINIMUMS[0].1];

/// The `[features]` line `scaffold::languages::python` emits into the binding crate's
/// `Cargo.toml`, rendered from the two constants above so the declared feature and the flag the
/// build passes cannot drift apart.
pub fn extension_module_feature_line() -> String {
    format_extension_module_feature_line(PYO3_EXTENSION_MODULE_PYO3_FEATURES)
}

fn format_extension_module_feature_line(features: &[&str]) -> String {
    let values = features
        .iter()
        .map(|feature| format!("\"{feature}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{PYO3_EXTENSION_MODULE_FEATURE} = [{values}]")
}

#[derive(Serialize)]
pub(crate) struct PythonVersionPolicy {
    pub(crate) version: &'static str,
    pub(crate) abi_tag: String,
    pub(crate) classifier_lines: String,
    abi_feature: &'static str,
}

impl Default for PythonVersionPolicy {
    fn default() -> Self {
        Self::from_supported_index(0)
    }
}

impl PythonVersionPolicy {
    fn from_supported_index(index: usize) -> Self {
        let (version, abi_feature) = SUPPORTED_PYTHON_MINIMUMS[index];
        let classifier_lines = std::iter::once("Programming Language :: Python :: 3 :: Only".to_string())
            .chain(
                SUPPORTED_PYTHON_MINIMUMS[index..]
                    .iter()
                    .map(|(version, _)| format!("Programming Language :: Python :: {version}")),
            )
            .map(|classifier| format!("  \"{classifier}\",\n"))
            .collect();
        Self {
            version,
            abi_tag: format!("abi3-py{}", version.replace('.', "")),
            abi_feature,
            classifier_lines,
        }
    }

    pub(crate) fn pyo3_features(&self) -> [&str; 2] {
        ["pyo3/extension-module", self.abi_feature]
    }

    pub(crate) fn extension_module_feature_line(&self) -> String {
        format_extension_module_feature_line(&self.pyo3_features())
    }
}

pub(crate) fn python_version_policy(minimum: Option<&str>) -> anyhow::Result<PythonVersionPolicy> {
    let minimum = minimum.unwrap_or(PYTHON_MINIMUM_VERSION);
    let index = SUPPORTED_PYTHON_MINIMUMS
        .iter()
        .position(|(version, _)| *version == minimum)
        .ok_or_else(|| {
            let supported = SUPPORTED_PYTHON_MINIMUMS
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!(
                "unsupported Python runtime minimum {minimum:?} for `{PYTHON_RUNTIME_VERSION_KEY}`; \
                 expected one of {supported}"
            )
        })?;
    Ok(PythonVersionPolicy::from_supported_index(index))
}

pub(crate) fn template_python_version_policy(
    overrides: minijinja::value::Serde<Option<std::collections::BTreeMap<String, String>>>,
) -> Result<minijinja::Value, minijinja::Error> {
    let minimum = overrides
        .0
        .as_ref()
        .and_then(|versions| versions.get(PYTHON_RUNTIME_VERSION_KEY));
    python_version_policy(minimum.map(String::as_str))
        .map(crate::template::to_value)
        .map_err(|error| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, error.to_string()))
}

/// The command prefix that runs a Python build tool inside the environment
/// `[workspace.tools] python_package_manager` locks, or `None` when the build should keep
/// resolving the tool off `PATH`.
///
/// Returning `None` for an *unset* key is deliberate: `ToolsConfig::python_pm` defaults to `uv`
/// for commands alef composes from scratch (test, lint, setup), but the build must not invent a
/// package manager a repo never asked for — a repo with no uv would get an unrunnable command.
/// An unset key therefore keeps today's bare invocation byte-for-byte, and only an explicit
/// selection redirects it.
///
/// The arms cover exactly the values `ToolsConfig::python_package_manager` documents. `pip` has
/// no run wrapper (its "environment" is whichever interpreter is already active), and an
/// undocumented value is left alone rather than guessed at, since a wrong wrapper fails harder
/// than no wrapper.
pub fn python_tool_runner(tools: &ToolsConfig) -> Option<&'static str> {
    match tools.python_package_manager.as_deref()? {
        // `--frozen --only-dev` and not a plain `uv run`: `uv run` would first sync the project
        // itself, which for a maturin-backed project means building the very extension this
        // command exists to build, using whichever maturin uv picked to bootstrap it. Restricting
        // the sync to the dev group installs the pinned maturin and nothing else. ~keep
        "uv" => Some("uv run --frozen --only-dev"),
        "poetry" => Some("poetry run"),
        _ => None,
    }
}

/// The tool a Python build readiness check must find on `PATH`.
///
/// When a package manager runs the build, *it* is the binary that has to exist — maturin comes
/// from the locked environment and may legitimately not be on `PATH` at all. Gating on `maturin`
/// there would skip the build on exactly the machines the package-manager selection exists to
/// support. ~keep
pub fn python_build_precondition_tool(tools: &ToolsConfig) -> &'static str {
    match python_tool_runner(tools) {
        Some(runner) => runner.split_whitespace().next().unwrap_or("maturin"),
        None => "maturin",
    }
}

/// Prefix `command` with the configured package manager's runner, if any.
pub fn run_through_python_package_manager(command: String, tools: &ToolsConfig) -> String {
    match python_tool_runner(tools) {
        Some(runner) => format!("{runner} {command}"),
        None => command,
    }
}

/// The extension-module feature the binding crate at `manifest_path` actually declares, or `None`
/// when that manifest declares no such feature (a hand-maintained crate, or one alef has not
/// generated yet).
///
/// Probing the manifest rather than assuming keeps the flag off a crate that would reject it:
/// `cargo` fails outright on `--features` naming a feature the package does not define, so an
/// unconditional flag would turn a working build into a hard error for those crates.
pub fn declared_extension_module_feature(manifest_path: &Path) -> Option<&'static str> {
    let contents = std::fs::read_to_string(manifest_path).ok()?;
    let manifest = toml::from_str::<toml::Value>(&contents).ok()?;
    let declared = manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .is_some_and(|features| features.contains_key(PYO3_EXTENSION_MODULE_FEATURE));
    declared.then_some(PYO3_EXTENSION_MODULE_FEATURE)
}

/// The ` --features <name>` fragment to append to a maturin invocation for the crate at
/// `manifest_path`, or an empty string when that crate declares no extension-module feature.
pub fn extension_module_feature_flag(manifest_path: &Path) -> String {
    declared_extension_module_feature(manifest_path)
        .map(|feature| format!(" --features {feature}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools_with(package_manager: Option<&str>) -> ToolsConfig {
        ToolsConfig {
            python_package_manager: package_manager.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn unset_package_manager_leaves_the_command_untouched() {
        let tools = tools_with(None);
        assert_eq!(python_tool_runner(&tools), None);
        assert_eq!(
            run_through_python_package_manager("maturin develop".to_string(), &tools),
            "maturin develop"
        );
        assert_eq!(python_build_precondition_tool(&tools), "maturin");
    }

    #[test]
    fn configured_package_manager_runs_the_build_in_its_locked_environment() {
        let tools = tools_with(Some("uv"));
        assert_eq!(
            run_through_python_package_manager("maturin develop".to_string(), &tools),
            "uv run --frozen --only-dev maturin develop"
        );
        assert_eq!(python_build_precondition_tool(&tools), "uv");
    }

    #[test]
    fn poetry_runs_the_build_through_poetry_run() {
        let tools = tools_with(Some("poetry"));
        assert_eq!(
            run_through_python_package_manager("maturin develop".to_string(), &tools),
            "poetry run maturin develop"
        );
        assert_eq!(python_build_precondition_tool(&tools), "poetry");
    }

    /// `pip` selects an interpreter, not a runner: there is nothing to prefix, so the command and
    /// its readiness check must stay exactly what an unset key produces.
    #[test]
    fn pip_has_no_run_wrapper() {
        let tools = tools_with(Some("pip"));
        assert_eq!(python_tool_runner(&tools), None);
        assert_eq!(
            run_through_python_package_manager("maturin develop".to_string(), &tools),
            "maturin develop"
        );
        assert_eq!(python_build_precondition_tool(&tools), "maturin");
    }

    #[test]
    fn feature_line_matches_the_flag_the_build_passes() {
        assert_eq!(
            extension_module_feature_line(),
            "extension-module = [\"pyo3/extension-module\", \"pyo3/abi3-py311\"]"
        );
        assert!(extension_module_feature_line().starts_with(PYO3_EXTENSION_MODULE_FEATURE));
    }

    #[test]
    fn declared_feature_is_read_from_the_generated_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            format!(
                "[package]\nname = \"sample-lib-py\"\nversion = \"0.1.0\"\n\n[features]\n{}\n",
                extension_module_feature_line()
            ),
        )
        .unwrap();

        assert_eq!(
            declared_extension_module_feature(&manifest),
            Some(PYO3_EXTENSION_MODULE_FEATURE)
        );
        assert_eq!(extension_module_feature_flag(&manifest), " --features extension-module");
    }

    #[test]
    fn a_crate_without_the_feature_gets_no_flag() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"sample-lib-py\"\nversion = \"0.1.0\"\n").unwrap();

        assert_eq!(declared_extension_module_feature(&manifest), None);
        assert_eq!(extension_module_feature_flag(&manifest), "");
    }

    #[test]
    fn a_missing_manifest_gets_no_flag() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(extension_module_feature_flag(&dir.path().join("Cargo.toml")), "");
    }
}
