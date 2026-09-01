//! Alef's own built-in `lint`/`setup`/`update`/`clean`/`build` pipeline data carriers.
//!
//! None of these five types are part of the `alef.toml` schema: 0.82.0 removed
//! `[lint.<lang>]`, `[setup.<lang>]`, `[update.<lang>]`, `[clean.<lang>]`, and
//! `[build_commands.<lang>]` (at both `[workspace.<key>.<lang>]` and `[crates.<key>.<lang>]`)
//! entirely -- alef now owns those commands end to end. Each struct here is a plain data
//! carrier returned by its matching `core::config::{lint,setup,update,build,clean}_defaults`
//! module and consumed by the matching `cli::pipeline::commands` handler; none derive
//! `Serialize`/`Deserialize`/`JsonSchema` any more, and none carry `deny_unknown_fields`.
//! `TestConfig` is the sole survivor of the old six-table family and stays in `output.rs`
//! itself alongside the other schema types, since `[test.<lang>]` is still deserialized from a
//! real `alef.toml`.
//!
//! Split out of `output.rs` to keep that file under the repo's 1,000-line cap.

use super::{ArgvRunConfig, StringOrVec};
use std::path::PathBuf;

/// Alef's own built-in lint pipeline for one language.
///
/// No longer part of the `alef.toml` schema (0.82.0 removed the `[lint.<lang>]` /
/// `[workspace.lint.<lang>]` / `[crates.lint.<lang>]` override tables): alef now owns lint end to
/// end, so this is a plain data carrier returned by [`super::lint_defaults::default_lint_config`]
/// and consumed by `cli::pipeline::commands::lint`, never deserialized from a consumer's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintConfig {
    /// Shell command that must exit 0 for lint to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main lint commands; aborts on failure.
    pub before: Option<StringOrVec>,
    pub format: Option<StringOrVec>,
    pub check: Option<StringOrVec>,
    pub typecheck: Option<StringOrVec>,
}

/// Alef's own built-in update pipeline for one language.
///
/// No longer part of the `alef.toml` schema (0.82.0 removed the `[update.<lang>]` /
/// `[workspace.update.<lang>]` / `[crates.update.<lang>]` override tables): alef now owns the
/// update command end to end, so this is a plain data carrier returned by
/// [`super::update_defaults::default_update_config`], never deserialized from a consumer's
/// config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateConfig {
    /// Shell command that must exit 0 for update to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main update commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) for safe dependency updates (compatible versions only).
    pub update: Option<StringOrVec>,
    /// Command(s) for aggressive updates (including incompatible/major bumps).
    pub upgrade: Option<StringOrVec>,
}

/// Alef's own built-in setup pipeline for one language.
///
/// No longer part of the `alef.toml` schema (0.82.0 removed the `[setup.<lang>]` /
/// `[workspace.setup.<lang>]` / `[crates.setup.<lang>]` override tables): alef now owns setup end
/// to end, so this is a plain data carrier returned by
/// [`super::setup_defaults::default_setup_config`], never deserialized from a consumer's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupConfig {
    /// Shell command that must exit 0 for setup to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main setup commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to install dependencies for this language.
    pub install: Option<StringOrVec>,
    /// Timeout in seconds for the complete setup (precondition + before + install).
    pub timeout_seconds: u64,
    /// Optional working directory (relative to repo root) for setup commands.
    ///
    /// When set, install commands run from `base_dir.join(workdir)` instead of
    /// `base_dir`. Required for languages whose manifest does not live at the
    /// workspace root (Swift's `Package.swift`, Kotlin-Android's `gradlew`,
    /// Dart's `pubspec.yaml`, Zig's `build.zig`). Defaults to `None` (run from
    /// repo root).
    pub workdir: Option<PathBuf>,
}

/// Alef's own built-in clean pipeline for one language.
///
/// No longer part of the `alef.toml` schema (0.82.0 removed the `[clean.<lang>]` /
/// `[workspace.clean.<lang>]` / `[crates.clean.<lang>]` override tables): alef now owns clean end
/// to end, so this is a plain data carrier returned by
/// [`super::clean_defaults::default_clean_config`], never deserialized from a consumer's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanConfig {
    /// Shell command that must exit 0 for clean to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main clean commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to clean build artifacts for this language.
    ///
    /// Mutually exclusive with `argv_clean` in practice: a default that must embed a
    /// config-supplied path (a custom package output directory) as a literal argument sets
    /// `argv_clean` instead of this field.
    pub clean: Option<StringOrVec>,
    /// Argv-only alternative to `clean`. See [`ArgvRunConfig`]. When both `clean` and
    /// `argv_clean` are set, the caller runs `argv_clean` and ignores `clean`.
    pub argv_clean: Option<ArgvRunConfig>,
}

/// Alef's own built-in build pipeline for one language.
///
/// No longer part of the `alef.toml` schema for real `alef.toml` files (0.82.0 removed the
/// `[build_commands.<lang>]` / `[workspace.build_commands.<lang>]` / `[crates.build_commands.<lang>]`
/// override tables): alef now owns the build command end to end, so this is a plain data carrier
/// returned by [`super::build_defaults::default_build_config`]. The one exception is this crate's
/// own `#[cfg(test)]` build-orchestration tests, which still populate
/// [`crate::core::config::ResolvedCrateConfig::build_commands`] directly (in-memory, never via
/// TOML) to keep hermetic control over what a test "build" actually runs -- see that field's doc
/// comment. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCommandConfig {
    /// Shell command that must exit 0 for build to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Shell command that must exit 0 for the build to be attempted, checking that this project's
    /// dependencies were fetched. Where `precondition` asks whether the machine can build this
    /// language at all, this asks whether the checkout is prepared — an unmet dependency
    /// precondition is fixable here and now, so it fails the run instead of being skipped
    /// silently. Set `dependency_remediation` alongside it with the command that fixes it.
    pub dependency_precondition: Option<String>,
    /// The command a user runs to satisfy `dependency_precondition`, e.g.
    /// `cd packages/elixir && mix deps.get`. Required whenever `dependency_precondition` is set.
    pub dependency_remediation: Option<String>,
    /// Command(s) to run before the main build commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to build in debug mode.
    pub build: Option<StringOrVec>,
    /// Command(s) to build in release mode.
    pub build_release: Option<StringOrVec>,
    /// Ceiling in seconds for this language's post-build `RunCommand` step (e.g. Swift's
    /// `cargo build --release` for the swift-bridge crate). `None` keeps alef's built-in
    /// ceiling (`RUN_COMMAND_TIMEOUT` in `cli::pipeline::commands::build`, 1800s). No built-in
    /// default ever sets this to `Some` (see `build_defaults::default_build_config`) -- alef #364
    /// added it as a per-language `alef.toml` escape hatch for a Swift `cargo build --release`
    /// that legitimately ran past 30 minutes, and 0.82.0's removal of `[build_commands.<lang>]`
    /// took that escape hatch away: the only remaining way to set this field is this crate's own
    /// `#[cfg(test)]` build-orchestration hook (see
    /// [`crate::core::config::ResolvedCrateConfig::build_commands`]), so every real build is now
    /// bound by the hardcoded 1800s ceiling with no override. Only governs a post-build
    /// `RunCommand` step; the `build`/`build_release` commands above run unbounded. ~keep
    pub timeout_seconds: Option<u64>,
}

impl BuildCommandConfig {
    /// Overlay `other` onto this config field-by-field.
    ///
    /// Test-only: since 0.82.0 removed `[build_commands.<lang>]` from `alef.toml`, the only
    /// caller left is `ResolvedCrateConfig::build_command_config_for_language`'s `#[cfg(test)]`
    /// branch, which composes a built-in default with an in-memory
    /// [`crate::core::config::ResolvedCrateConfig::build_commands`] entry a build-orchestration
    /// test set directly (never through TOML). ~keep
    #[cfg(test)]
    pub(crate) fn merge_overlay(mut self, other: &Self) -> Self {
        if other.precondition.is_some() {
            self.precondition = other.precondition.clone();
        }
        // Plain field overlay. Whether a built-in dependency check *survives* a user's own build
        // command is decided by the caller, not here — see
        // `ResolvedCrateConfig::build_command_config_for_language`. ~keep
        if other.dependency_precondition.is_some() {
            self.dependency_precondition = other.dependency_precondition.clone();
        }
        if other.dependency_remediation.is_some() {
            self.dependency_remediation = other.dependency_remediation.clone();
        }
        if other.before.is_some() {
            self.before = other.before.clone();
        }
        if other.build.is_some() {
            self.build = other.build.clone();
        }
        if other.build_release.is_some() {
            self.build_release = other.build_release.clone();
        }
        if other.timeout_seconds.is_some() {
            self.timeout_seconds = other.timeout_seconds;
        }
        self
    }
}
