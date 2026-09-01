//! Artifact packaging — creates distributable archives for each language.

pub mod c_ffi;
pub mod cli;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod gleam;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod node;
pub mod php;
pub mod python;
pub mod ruby;
pub mod swift;
pub(crate) mod template_env;
pub mod util;
pub mod wasm;
pub mod zig;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A produced package artifact.
#[derive(Debug)]
pub struct PackageArtifact {
    /// Path to the artifact file.
    pub path: PathBuf,
    /// Human-readable artifact name.
    pub name: String,
    /// SHA256 hex digest (if computed).
    pub checksum: Option<String>,
}

/// Create a tar.gz archive from a staging directory.
///
/// The staging directory's basename becomes the single top-level entry inside
/// the archive — so callers whose consumers expect that wrapper (CLI tarballs,
/// FFI tarballs, language SDK archives) get the conventional `dirname/...`
/// layout. For consumers that need the staging contents at the archive root
/// (PHP PIE, which probes the extracted-source root for the extension `.so`),
/// use [`create_tar_gz_flat`] instead.
pub fn create_tar_gz(staging_dir: &Path, output_path: &Path) -> Result<()> {
    let file_name = staging_dir
        .file_name()
        .context("staging dir has no file name")?
        .to_string_lossy();

    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(output_path)
        .arg("-C")
        .arg(staging_dir.parent().unwrap_or(Path::new(".")))
        .arg(file_name.as_ref())
        .status()?;

    if !status.success() {
        anyhow::bail!("tar failed with exit code {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Create a tar.gz archive whose entries are the contents of `staging_dir`,
/// without the wrapping directory.
///
/// PHP PIE's `UnixBuild` probes the extracted-source root for the extension
/// `.so`; if it sees only a single subdirectory it would `unfoldUnarchivedSourcePaths()`,
/// but only when that subdir contains `config.m4` / `config.w32`. Our PIE
/// archive is a precompiled binary with neither, so PIE never unfolds and the
/// install fails with "extension not found". Archive contents directly so the
/// `.so` lands at the archive root.
///
/// Entries are enumerated explicitly rather than passing `.` to `tar`, because
/// `tar czf out.tgz -C dir .` emits a leading `./` directory entry that PIE's
/// Phar-based `TarDownloader` rejects with `Cannot extract ".", internal error`.
/// Passing each top-level entry by name produces a flat archive with no
/// directory entries at all.
pub fn create_tar_gz_flat(staging_dir: &Path, output_path: &Path) -> Result<()> {
    let mut entries: Vec<String> = std::fs::read_dir(staging_dir)
        .with_context(|| format!("reading staging dir {}", staging_dir.display()))?
        .map(|res| {
            res.map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    if entries.is_empty() {
        anyhow::bail!(
            "staging dir {} is empty; refusing to create empty archive",
            staging_dir.display()
        );
    }
    entries.sort();

    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(output_path)
        .arg("-C")
        .arg(staging_dir)
        .args(&entries)
        .status()?;

    if !status.success() {
        anyhow::bail!("tar failed with exit code {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// The cargo build profile a caller expects an artifact to have been produced under.
///
/// Every caller must name a profile explicitly — there is no default — because guessing has a
/// real failure mode: a `debug`-profile artifact and a `release`-profile artifact for the same
/// crate can legitimately coexist with different symbol sets (a plain `cargo build` and `cargo
/// build --release` are two independent invocations; nothing keeps them in sync), and silently
/// picking one over the other is exactly the "check that passes because it examined nothing"
/// shape this type exists to close off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildProfile {
    Release,
    Debug,
}

impl BuildProfile {
    /// The `target/<this>/` directory name cargo uses for this profile.
    pub(crate) fn dir_name(self) -> &'static str {
        match self {
            BuildProfile::Release => "release",
            BuildProfile::Debug => "debug",
        }
    }

    /// The `cargo build` flag that selects this profile (empty for the debug default).
    pub(crate) fn cargo_flag(self) -> &'static str {
        match self {
            BuildProfile::Release => " --release",
            BuildProfile::Debug => "",
        }
    }
}

impl std::fmt::Display for BuildProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.dir_name())
    }
}

/// Profile search order for callers with no build of their own to point to (`alef generate`'s
/// post-build pass, `alef test`'s e2e FFI staging, Dart's post-build native-library staging).
/// `release` wins because it is what every profile-aware caller (`alef build`, `alef publish`)
/// treats as canonical. This is the single declared ordering every "prefer release, fall back to
/// debug" caller must read instead of re-deciding the order locally — see
/// [`find_built_artifact`]'s doc comment for why `deps/` is never a third option here. ~keep
pub const PREFERRING_RELEASE_ORDER: [BuildProfile; 2] = [BuildProfile::Release, BuildProfile::Debug];

/// Find a built artifact for a specific, caller-named build profile.
///
/// Searches only `target/{triple}/{profile}/` and `target/{profile}/` — the two locations cargo
/// uplifts an unhashed copy into for a package that was an *explicit* root of the `cargo build`
/// invocation that produced it (a `-p`/`--manifest-path` target, or a workspace default-member).
///
/// Deliberately does **not** fall back to either location's `deps/` subdirectory. `deps/`
/// aggregates the unhashed cdylib/staticlib output of *every* cargo invocation that has ever
/// compiled this crate in this profile, including as a transitive dependency of something else's
/// build with its own, possibly narrower, feature selection — there is no way to tell from the
/// directory alone which invocation's bytes are sitting there, so treating its mere presence as
/// "the crate's own build" is a check that passes without verifying what it claims to verify. A
/// deps-only artifact is real cargo output but unattributed cargo output; staging or packaging it
/// as if it were the crate's own dedicated build risks shipping a library whose exported symbols
/// don't match what the bindings were generated against — the caller must build the crate
/// explicitly (`cargo build -p <crate>{profile_flag}`) instead. When rejecting, this function
/// still looks in `deps/` to name what it found there, so the error is diagnosable rather than a
/// bare "not found". ~keep
pub fn find_built_artifact(
    workspace_root: &Path,
    target: &crate::publish::platform::RustTarget,
    filename: &str,
    profile: BuildProfile,
) -> Result<PathBuf> {
    find_built_artifact_impl(workspace_root, target, &[filename], profile, &[])
}

/// As [`find_built_artifact`], but also checks `extra_dirs` (in the given order, after the two
/// canonical uplifted locations and before giving up or falling through to the `deps/`
/// diagnostic) for callers whose toolchain writes output somewhere cargo does not uplift to on
/// its own — e.g. `napi build`'s in-crate output directory, or a Ruby extension crate built from
/// its own directory rather than the workspace root. `extra_dirs` are joined with `filename`
/// literally; they are not profile-scoped, matching how those tools actually place output. This
/// is the one place that logic lives — every caller with such a directory must pass it here
/// rather than re-implementing its own search order alongside a duplicated copy of the
/// two-canonical-locations check this function already does. ~keep
pub fn find_built_artifact_with_extra_dirs(
    workspace_root: &Path,
    target: &crate::publish::platform::RustTarget,
    filename: &str,
    profile: BuildProfile,
    extra_dirs: &[PathBuf],
) -> Result<PathBuf> {
    find_built_artifact_impl(workspace_root, target, &[filename], profile, extra_dirs)
}

/// As [`find_built_artifact_with_extra_dirs`], but for a caller with more than one acceptable
/// filename for the same artifact (e.g. napi-rs's cross-compile-renamed `.node` name and its
/// plain, non-renamed name) — `filenames` are tried in order, and *every* filename is tried at
/// one location tier before moving on to the next tier. That tier-then-name order matters: a
/// fresher artifact under a lower-priority name at a higher-priority tier must still win over a
/// stale artifact under a higher-priority name at a lower-priority tier, so this must not be
/// approximated by calling [`find_built_artifact_with_extra_dirs`] once per filename (which would
/// search name-then-tier instead). ~keep
pub fn find_built_artifact_any_with_extra_dirs(
    workspace_root: &Path,
    target: &crate::publish::platform::RustTarget,
    filenames: &[&str],
    profile: BuildProfile,
    extra_dirs: &[PathBuf],
) -> Result<PathBuf> {
    find_built_artifact_impl(workspace_root, target, filenames, profile, extra_dirs)
}

/// Only `target/{triple}/{profile}/` and `target/{profile}/` (plus `extra_dirs`) are searched:
/// those are the only locations cargo uplifts an explicit build target into. A `deps/`-only copy
/// is named in the error but never returned -- a crate compiled solely as a transitive dependency
/// of something else's build may not carry the feature set the bindings were generated against,
/// so the artifact is untrustworthy. ~keep
fn find_built_artifact_impl(
    workspace_root: &Path,
    target: &crate::publish::platform::RustTarget,
    filenames: &[&str],
    profile: BuildProfile,
    extra_dirs: &[PathBuf],
) -> Result<PathBuf> {
    let cross_dir = workspace_root
        .join("target")
        .join(&target.triple)
        .join(profile.dir_name());
    let native_dir = workspace_root.join("target").join(profile.dir_name());
    for candidate_dir in [&cross_dir, &native_dir].into_iter().chain(extra_dirs.iter()) {
        for filename in filenames {
            let candidate = candidate_dir.join(*filename);
            if candidate.exists() {
                tracing::debug!(path = %candidate.display(), %profile, "found uplifted build artifact");
                return Ok(candidate);
            }
        }
    }

    let untrusted_deps_copy = [cross_dir.join("deps"), native_dir.join("deps")]
        .into_iter()
        .flat_map(|deps_dir| filenames.iter().map(move |filename| deps_dir.join(*filename)))
        .find(|candidate| candidate.exists());

    let filenames_display = filenames.join(" or ");
    let extra_dirs_note = if extra_dirs.is_empty() {
        String::new()
    } else {
        let listed = extra_dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>();
        format!(" or in {}", listed.join(", "))
    };

    match untrusted_deps_copy {
        Some(deps_path) => anyhow::bail!(
            "{filenames_display} not found in target/{}/{profile}/ or target/{profile}/{extra_dirs_note}; an \
             unused deps/-only copy exists at {} — run `cargo build -p <crate>{}` (or `alef build{}`) to build \
             it as an explicit top-level target",
            target.triple,
            deps_path.display(),
            profile.cargo_flag(),
            profile.cargo_flag(),
        ),
        None => anyhow::bail!(
            "{filenames_display} not found in target/{}/{profile}/ or target/{profile}/{extra_dirs_note}",
            target.triple
        ),
    }
}

#[cfg(test)]
mod find_built_artifact_tests {
    use super::{BuildProfile, find_built_artifact};
    use crate::publish::platform::RustTarget;

    /// Baseline positive case: an uplifted `target/{triple}/release/` copy is found for a
    /// `Release` request. Every other test in this module is a variation that must NOT match --
    /// this one proves the happy path still works at all.
    #[test]
    fn finds_uplifted_release_artifact() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let release_dir = root.join("target").join(&target.triple).join("release");
        std::fs::create_dir_all(&release_dir).expect("create release dir");
        std::fs::write(release_dir.join("libsample_ffi.so"), b"uplifted-release").expect("write fixture");

        let found = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release)
            .expect("must find uplifted release artifact");
        assert_eq!(found, release_dir.join("libsample_ffi.so"));
    }

    /// Same shape as above but for the native (no-triple) `target/release/` location, and for
    /// `Debug` rather than `Release` -- proves the profile parameter actually selects the
    /// `target/{profile}/` directory name, not just a hardcoded `release`. Negative control:
    /// `finds_uplifted_release_artifact` above uses the triple-scoped `release` directory and
    /// would not find this fixture, and `debug_profile_does_not_fall_back_to_release_uplift`
    /// below proves a `Release` request does not find this `debug` fixture either.
    #[test]
    fn finds_uplifted_debug_artifact_in_native_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let debug_dir = root.join("target/debug");
        std::fs::create_dir_all(&debug_dir).expect("create debug dir");
        std::fs::write(debug_dir.join("libsample_ffi.so"), b"uplifted-debug").expect("write fixture");

        let found = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Debug)
            .expect("must find uplifted debug artifact");
        assert_eq!(found, debug_dir.join("libsample_ffi.so"));
    }

    /// Negative control for profile-awareness: a `debug`-only artifact must not satisfy a
    /// `Release` request. This is the exact regression this rewrite closes -- staging used to
    /// hardcode `release` regardless of which profile a build actually produced, so a `cargo
    /// build` (no `--release`) that wrote only `target/debug/...` left `target/release/...`
    /// missing and staging silently fell through to an unrelated `deps/` copy instead of naming
    /// the real problem.
    #[test]
    fn release_profile_does_not_fall_back_to_debug_uplift() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let debug_dir = root.join("target/debug");
        std::fs::create_dir_all(&debug_dir).expect("create debug dir");
        std::fs::write(debug_dir.join("libsample_ffi.so"), b"uplifted-debug").expect("write fixture");

        let result = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release);
        assert!(
            result.is_err(),
            "a debug-only artifact must not satisfy a release request"
        );
    }

    /// Mirror of the previous test in the other direction, so profile-selection is proven both
    /// ways rather than by one assertion that could pass if the parameter were ignored entirely.
    #[test]
    fn debug_profile_does_not_fall_back_to_release_uplift() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let release_dir = root.join("target/release");
        std::fs::create_dir_all(&release_dir).expect("create release dir");
        std::fs::write(release_dir.join("libsample_ffi.so"), b"uplifted-release").expect("write fixture");

        let result = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Debug);
        assert!(
            result.is_err(),
            "a release-only artifact must not satisfy a debug request"
        );
    }

    /// Contract change from the prior behaviour (alef #456's follow-up added a `deps/` fallback
    /// that made this case succeed): a crate compiled only because another crate path-depends on
    /// it (e.g. `-ffi` pulled in by a `-swift`/`-jni` binding crate's own build) lands only in
    /// `target/.../deps/`, with a feature set governed by whatever pulled it in rather than the
    /// crate's own defaults. That is now rejected rather than silently used -- see this
    /// function's doc comment for why. Negative control: `finds_uplifted_release_artifact` above
    /// proves the same directory layout succeeds once the artifact is in the trusted uplifted
    /// location instead of only `deps/`, so this test is verifying the `deps/`-rejection
    /// specifically, not a broken lookup in general.
    #[test]
    fn rejects_deps_only_artifact_even_though_it_is_real_cargo_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let deps_dir = root.join("target/release/deps");
        std::fs::create_dir_all(&deps_dir).expect("create deps dir");
        std::fs::write(deps_dir.join("libsample_ffi.so"), b"deps-only-artifact").expect("write fixture");

        let result = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release);
        assert!(result.is_err(), "a deps/-only artifact must never be silently staged");
    }

    /// The rejection above must be diagnosable, not a bare "not found" -- the operator needs to
    /// see that a deps/-only copy exists and why it was not trusted, per this task's requirement
    /// that a rejected fallback still be visible in the log/error. Negative control:
    /// `errors_without_mentioning_deps_when_nothing_exists_anywhere` below proves the deps/
    /// mention only appears when a deps/ copy genuinely exists, not unconditionally.
    #[test]
    fn error_names_the_untrusted_deps_copy_when_rejecting() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let deps_dir = root.join("target/release/deps");
        std::fs::create_dir_all(&deps_dir).expect("create deps dir");
        std::fs::write(deps_dir.join("libsample_ffi.so"), b"deps-only-artifact").expect("write fixture");

        let error = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release)
            .expect_err("deps-only artifact must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("deps"),
            "error should name the rejected deps/ copy, got: {message}"
        );
        // `deps_dir` above was built by joining one already-slashed literal ("target/release/
        // deps"), while the production code under test joins each component separately
        // (`native_dir.join("deps")`) -- on Windows those two constructions render with
        // different mixes of `\` and `/` for the identical file, so the raw `.display()` strings
        // do not substring-match even though both name the same path. Compare through
        // `portable_path_string` instead of `.display()` directly. ~keep
        let portable_message = message.replace('\\', "/");
        assert!(
            portable_message.contains(&crate::test_support::portable_path_string(
                &deps_dir.join("libsample_ffi.so")
            )),
            "error should include the deps/ copy's path so an operator can inspect it, got: {message}"
        );
    }

    /// An uplifted copy must still win over a `deps/` copy when both exist -- `deps/` is never
    /// preferred, and its presence alongside a trusted uplifted copy must not even appear in the
    /// error path (there is no error).
    #[test]
    fn prefers_uplifted_artifact_over_deps_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let release_dir = root.join("target/release");
        std::fs::create_dir_all(&release_dir).expect("create release dir");
        std::fs::write(release_dir.join("libsample_ffi.so"), b"uplifted").expect("write uplifted fixture");

        let deps_dir = release_dir.join("deps");
        std::fs::create_dir_all(&deps_dir).expect("create deps dir");
        std::fs::write(deps_dir.join("libsample_ffi.so"), b"deps-copy").expect("write deps fixture");

        let found = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release)
            .expect("must find uplifted artifact");
        assert_eq!(found, release_dir.join("libsample_ffi.so"));
    }

    /// When nothing exists anywhere (no uplifted copy, no deps/ copy either), the error must not
    /// falsely claim a deps/ copy exists -- negative control for
    /// `error_names_the_untrusted_deps_copy_when_rejecting`.
    #[test]
    fn errors_without_mentioning_deps_when_nothing_exists_anywhere() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let result = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release);
        let error = result.expect_err("must error when nothing exists");
        let message = error.to_string();
        assert!(message.contains("not found"), "got: {message}");
        assert!(
            !message.contains("deps/-only copy exists"),
            "must not claim a deps/ copy exists when none does, got: {message}"
        );
    }

    #[test]
    fn still_errors_when_absent_everywhere_including_deps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let result = find_built_artifact(root, &target, "libsample_ffi.so", BuildProfile::Release);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    /// `PREFERRING_RELEASE_ORDER` is the single declared "release wins, debug is the fallback"
    /// policy every "no build of my own" caller (ffi_stage's `*_preferring_release`, Dart's
    /// post-build staging) must read instead of hardcoding the order itself. If a future edit
    /// reorders or resizes this array without updating every reader in lockstep, this is the one
    /// test that catches it.
    #[test]
    fn preferring_release_order_puts_release_first_and_only_lists_the_two_real_profiles() {
        assert_eq!(
            super::PREFERRING_RELEASE_ORDER,
            [BuildProfile::Release, BuildProfile::Debug],
            "release must be tried before debug, and no third profile may appear here"
        );
    }

    /// An `extra_dirs` entry is checked when the two canonical uplifted locations both miss --
    /// proving `find_built_artifact_with_extra_dirs` actually consults it rather than silently
    /// ignoring the parameter. Callers with a legitimate extra output location (napi-rs's
    /// in-crate `.node` output, a Ruby extension crate's own `target/`) depend on this.
    #[test]
    fn with_extra_dirs_checks_extra_dir_when_canonical_locations_miss() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let extra_dir = root.join("crates/sample-node");
        std::fs::create_dir_all(&extra_dir).expect("create extra dir");
        std::fs::write(extra_dir.join("sample.node"), b"in-crate-output").expect("write fixture");

        let found = super::find_built_artifact_with_extra_dirs(
            root,
            &target,
            "sample.node",
            BuildProfile::Release,
            std::slice::from_ref(&extra_dir),
        )
        .expect("must find artifact in extra_dirs");
        assert_eq!(
            found,
            extra_dir.join("sample.node"),
            "expected the extra_dirs copy at {}, got {}",
            extra_dir.join("sample.node").display(),
            found.display()
        );
    }

    /// The two canonical uplifted locations must still win over `extra_dirs` when both exist --
    /// `extra_dirs` is a fallback for tools that do not uplift to the canonical locations at all,
    /// never a preference over cargo's own uplifted output.
    #[test]
    fn extra_dirs_does_not_shadow_the_canonical_uplifted_location() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let release_dir = root.join("target").join(&target.triple).join("release");
        std::fs::create_dir_all(&release_dir).expect("create release dir");
        std::fs::write(release_dir.join("sample.node"), b"canonical-uplift").expect("write fixture");

        let extra_dir = root.join("crates/sample-node");
        std::fs::create_dir_all(&extra_dir).expect("create extra dir");
        std::fs::write(extra_dir.join("sample.node"), b"in-crate-output").expect("write fixture");

        let found = super::find_built_artifact_with_extra_dirs(
            root,
            &target,
            "sample.node",
            BuildProfile::Release,
            std::slice::from_ref(&extra_dir),
        )
        .expect("must find canonical artifact");
        assert_eq!(
            found,
            release_dir.join("sample.node"),
            "expected the canonical uplifted copy at {}, got {}",
            release_dir.join("sample.node").display(),
            found.display()
        );
    }

    /// Negative control: `find_built_artifact` (no `extra_dirs`) must not accidentally pick up a
    /// file sitting in a directory that only the `_with_extra_dirs` variant is supposed to check.
    #[test]
    fn find_built_artifact_without_extra_dirs_ignores_the_extra_location() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let extra_dir = root.join("crates/sample-node");
        std::fs::create_dir_all(&extra_dir).expect("create extra dir");
        std::fs::write(extra_dir.join("sample.node"), b"in-crate-output").expect("write fixture");

        let result = find_built_artifact(root, &target, "sample.node", BuildProfile::Release);
        assert!(
            result.is_err(),
            "find_built_artifact must not silently check extra_dirs-only locations, got: {result:?}"
        );
    }

    /// `find_built_artifact_any_with_extra_dirs` must search every location tier before moving to
    /// the next filename -- i.e. tier priority beats name priority. A caller with two acceptable
    /// filenames (napi-rs's cross-compile-renamed name and its plain name) depends on this: a
    /// fresh artifact under the second filename at the first tier must still beat a stale artifact
    /// under the first filename at a later tier.
    #[test]
    fn find_built_artifact_any_searches_every_tier_before_the_next_filename() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let cross_dir = root.join("target").join(&target.triple).join("release");
        std::fs::create_dir_all(&cross_dir).expect("create cross dir");
        std::fs::write(cross_dir.join("second.node"), b"cross-tier-second-name").expect("write fixture");

        let extra_dir = root.join("crates/sample-node");
        std::fs::create_dir_all(&extra_dir).expect("create extra dir");
        std::fs::write(extra_dir.join("first.node"), b"extra-tier-first-name").expect("write fixture");

        let found = super::find_built_artifact_any_with_extra_dirs(
            root,
            &target,
            &["first.node", "second.node"],
            BuildProfile::Release,
            std::slice::from_ref(&extra_dir),
        )
        .expect("must find an artifact");
        assert_eq!(
            found,
            cross_dir.join("second.node"),
            "the higher-priority tier must win over a lower-priority tier under a preferred name; expected {}, \
             got {}",
            cross_dir.join("second.node").display(),
            found.display()
        );
    }
}
