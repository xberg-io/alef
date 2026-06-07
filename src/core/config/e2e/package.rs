use super::defaults::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Controls whether generated e2e test projects reference the package under
/// test via a local path (for development) or a registry version string
/// (for standalone `test_apps` that consumers can run without the monorepo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DependencyMode {
    /// Local path dependency (default) — used during normal e2e development.
    #[default]
    Local,
    /// Registry dependency — generates standalone test apps that pull the
    /// package from its published registry (PyPI, npm, crates.io, etc.).
    Registry,
}
/// Configuration for registry-mode e2e generation (`alef e2e generate --registry`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegistryConfig {
    /// Output directory for registry-mode test apps (default: "test_apps").
    #[serde(default = "default_test_apps_dir")]
    pub output: String,
    /// Per-language package overrides used only in registry mode.
    /// Merged on top of the base `[e2e.packages]` entries.
    #[serde(default)]
    pub packages: HashMap<String, PackageRef>,
    /// When non-empty, only fixture categories in this list are included in
    /// registry-mode generation (useful for shipping a curated subset).
    #[serde(default)]
    pub categories: Vec<String>,
    /// GitHub repository URL for downloading prebuilt artifacts (e.g., FFI
    /// shared libraries) from GitHub Releases.
    ///
    /// Falls back to `[scaffold] repository` when not set. Registry generators
    /// that need a concrete release host should fail when no repository is
    /// configured instead of inventing publishable metadata.
    #[serde(default)]
    pub github_repo: Option<String>,
    /// Per-language commands that install the published package into the
    /// registry-mode test app and exercise it, executed by `alef test-apps run`.
    /// Languages omitted here fall back to `test_apps_run_defaults`.
    #[serde(default)]
    pub run: HashMap<String, crate::core::config::output::TestAppRunConfig>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            output: default_test_apps_dir(),
            packages: HashMap::new(),
            categories: Vec::new(),
            github_repo: None,
            run: HashMap::new(),
        }
    }
}
/// A single CLI test entry for the Homebrew test_app generator.
///
/// Each entry describes one test step in `run_tests.sh`.  The `command`
/// field is a verbatim shell fragment; the following variables are already
/// exported by `run_tests.sh` when the fragment is evaluated:
///
/// - `$CLI_FORMULA` — the CLI binary name (from `cli_formula`)
/// - `$VERSION` — the package version
/// - `$TAP` — the Homebrew tap (e.g. `"myorg/tap"`)
/// - `$SCRIPT_DIR` — absolute directory of `run_tests.sh`
///
/// If `expect_contains` is set the test passes only when the command's
/// combined stdout+stderr contains the given substring; otherwise a zero
/// exit code is sufficient.
///
/// Example `alef.toml`:
///
/// ```toml
/// [[crates.e2e.registry.packages.homebrew.cli_tests]]
/// name = "version"
/// command = "$CLI_FORMULA --version"
/// expect_contains = "$VERSION"
///
/// [[crates.e2e.registry.packages.homebrew.cli_tests]]
/// name = "convert-h1"
/// command = "echo '<h1>Hi</h1>' | $CLI_FORMULA"
/// expect_contains = "# Hi"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HomebrewCliTest {
    /// Short identifier used in `PASS:` / `FAIL:` reporting.
    pub name: String,
    /// Shell fragment executed verbatim; variables `$CLI_FORMULA`, `$VERSION`,
    /// `$TAP`, and `$SCRIPT_DIR` are in scope.
    pub command: String,
    /// Substring that must appear in the combined stdout+stderr output.
    /// When `None` a zero exit code is sufficient.
    #[serde(default)]
    pub expect_contains: Option<String>,
}

/// Per-language package reference configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct PackageRef {
    /// Package/crate/gem/module name.
    #[serde(default)]
    pub name: Option<String>,
    /// Relative path from e2e/{lang}/ to the package.
    #[serde(default)]
    pub path: Option<String>,
    /// Go module path.
    #[serde(default)]
    pub module: Option<String>,
    /// Package version (e.g., for go.mod require directives).
    #[serde(default)]
    pub version: Option<String>,
    /// SHA-256 hash of the published tarball (Zig registry mode).
    ///
    /// When present without `platform_hashes`, emitted for the generic package tarball.
    /// Multi-platform Zig release assets must use `platform_hashes` because Zig hashes are
    /// content-specific.
    #[serde(default)]
    pub hash: Option<String>,
    /// Platform-specific Zig package hashes keyed by platform suffix
    /// (`linux-x86_64`, `linux-aarch64`, `macos-arm64`, `macos-x86_64`, `windows-x86_64`).
    ///
    /// When present in registry mode, alef emits one lazy dependency per platform.
    #[serde(default)]
    pub platform_hashes: BTreeMap<String, String>,
    /// Homebrew tap name (e.g., `"sample_core-dev/tap"`).
    ///
    /// Used by the `homebrew` test_app generator.
    #[serde(default)]
    pub tap: Option<String>,
    /// Homebrew CLI formula name (e.g., `"sample-markdown"`).
    ///
    /// Used by the `homebrew` test_app generator.
    #[serde(default)]
    pub cli_formula: Option<String>,
    /// Homebrew FFI shared-library formula name (e.g., `"libsample-markdown"`).
    ///
    /// When not set, FFI-related sections (Brewfile entry, compile/run block,
    /// `ffi_smoke.c`) are omitted from the generated test_app.
    ///
    /// Used by the `homebrew` test_app generator.
    #[serde(default)]
    pub ffi_formula: Option<String>,
    /// CLI test steps for the Homebrew test_app generator.
    ///
    /// When empty (the default) a single default `--version` check is emitted
    /// that asserts `$VERSION` appears in the output.  Provide explicit entries
    /// to replace the default entirely.
    ///
    /// Used by the `homebrew` test_app generator.
    #[serde(default)]
    pub cli_tests: Vec<HomebrewCliTest>,
}
