//! Build provenance rendered into `alef --version`.
//!
//! `build.rs` stamps the commit sha, working-tree state, and build time into `rustc-env` vars at
//! compile time; this module turns them into the long version string clap prints for `--version`
//! and into the [`build_identity`] salt every cache that can skip work is keyed on. Two renderings
//! of one set of constants, never two independent answers to "which alef is this".
//!
//! The bare semver stays alone on the first line. Existing consumers — release gates, the
//! `alef_version` pin in `alef.toml`, `expect_contains` checks in generated Homebrew test apps —
//! read that line, and provenance must not cost them anything. ~keep

use std::sync::LazyLock;

/// Full commit sha, or `"unknown"` when git could not answer at build time.
pub(crate) const COMMIT: &str = env!("ALEF_BUILD_COMMIT");

/// First 12 characters of [`COMMIT`], or `"unknown"`.
pub(crate) const COMMIT_SHORT: &str = env!("ALEF_BUILD_COMMIT_SHORT");

/// `"clean"`, `"dirty"`, or `"unknown"`. See `build.rs` for why `clean` is the weakest of the
/// three claims: it can be stale, while `dirty` and the sha cannot. ~keep
pub(crate) const TREE_STATE: &str = env!("ALEF_BUILD_TREE_STATE");

/// Build time as seconds since the Unix epoch, or `"unknown"`.
pub(crate) const TIMESTAMP: &str = env!("ALEF_BUILD_TIMESTAMP");

/// The value `build.rs` emits when git cannot answer.
const UNKNOWN: &str = "unknown";

const TREE_CLEAN: &str = "clean";
const TREE_DIRTY: &str = "dirty";

/// The token an operator or a script greps for to reject a non-reproducible binary. Uppercase and
/// unhyphenated so it cannot be confused with the lowercase `tree:` state token. ~keep
const DIRTY_MARKER: &str = "DIRTY";

static LONG_VERSION: LazyLock<String> =
    LazyLock::new(|| render_long_version(env!("CARGO_PKG_VERSION"), COMMIT_SHORT, COMMIT, TREE_STATE, TIMESTAMP));

static BUILD_IDENTITY: LazyLock<String> =
    LazyLock::new(|| render_build_identity(env!("CARGO_PKG_VERSION"), COMMIT, TREE_STATE, TIMESTAMP));

/// This binary's build identity: the cache-salting form of exactly the provenance
/// [`long_version`] prints.
///
/// Both are rendered from the same four stamped constants, so a cache key and the `--version`
/// banner can never disagree about which alef is running. That is the whole reason this lives
/// here rather than beside a cache: during a release cycle many candidate binaries report the
/// same semver while differing by commit, and a cache that salts on the semver alone replays one
/// candidate's verdicts under another. A consumer agent observed exactly that — the binary on its
/// PATH changed commit mid-investigation while `alef 0.72.0` never moved.
///
/// The build time is folded in **only for a `dirty` tree**, and the asymmetry is load-bearing in
/// both directions:
///
/// * `clean` — the commit fully determines the source, so two builds of one tag on two machines
///   at two times must produce the *same* identity. Including the timestamp here would give every
///   released binary a private key space and turn every release run into a cold-cache run.
/// * `unknown` — a crates.io tarball has no `.git` but is byte-identical source for every user at
///   a given version, so the semver already identifies it completely and the timestamp would
///   again only destroy sharing. The residual this accepts: two *git* checkouts built at
///   different commits on a machine without git collide. `--version` already shouts that such a
///   build is "not attributable to any commit"; nothing here can recover what was never stamped.
/// * `dirty` — the commit does *not* determine the source, which is precisely the local
///   development case where a stale verdict does the most damage. The timestamp moves whenever
///   `build.rs` re-runs, i.e. whenever a watched compile input changed, so a rebuilt binary gets
///   a cold cache while repeated runs of one dirty binary stay warm. That is narrower than
///   bypassing the cache on dirty, which would make every local run of an expensive validation
///   pass start from nothing.
///
/// This inherits `build.rs`'s documented gap: an uncommitted edit confined to an unwatched
/// tracked path leaves the stamp in place. `dirty` and the sha are trustworthy; `clean` is only
/// as fresh as the last watched path to move. ~keep
pub(crate) fn build_identity() -> &'static str {
    BUILD_IDENTITY.as_str()
}

/// Field separator for [`build_identity`]. Not a hash — the string is read by humans in cache
/// diagnostics, and none of the four components can contain it. ~keep
const IDENTITY_SEPARATOR: char = '/';

/// Render [`build_identity`] from explicit provenance, so a test can compare two builds without
/// producing two real binaries. Production reaches this only through [`BUILD_IDENTITY`].
pub(crate) fn render_build_identity(semver: &str, commit: &str, tree_state: &str, timestamp: &str) -> String {
    let mut identity = format!("{semver}{IDENTITY_SEPARATOR}{commit}{IDENTITY_SEPARATOR}{tree_state}");
    if tree_state == TREE_DIRTY {
        identity.push(IDENTITY_SEPARATOR);
        identity.push_str(timestamp);
    }
    identity
}

/// Whether this binary's own working tree was stamped clean at compile time.
///
/// The one signal `version_pin::maybe_update_alef_toml_version_pin` requires before it will ever
/// rewrite the `alef_version` pin -- see that function's doc for why a dirty build must never
/// drive the pin. `TREE_STATE`'s own doc already notes `clean` is the weakest of the three build
/// claims (it can be stale); this helper does not strengthen that, it only names the comparison
/// once instead of repeating the string literal at every call site. ~keep
pub(crate) fn running_build_is_clean() -> bool {
    TREE_STATE == TREE_CLEAN
}

/// The multi-line body clap prints after the binary name for `--version`.
///
/// `-V` keeps printing the short, single-line `alef <semver>`. ~keep
pub(crate) fn long_version() -> &'static str {
    LONG_VERSION.as_str()
}

fn render_long_version(
    semver: &str,
    commit_short: &str,
    commit_full: &str,
    tree_state: &str,
    timestamp: &str,
) -> String {
    let mut out = String::with_capacity(320);
    out.push_str(semver);

    out.push_str("\ncommit:  ");
    if commit_short == UNKNOWN || commit_full == UNKNOWN {
        out.push_str(UNKNOWN);
    } else {
        out.push_str(commit_short);
        out.push_str(" (");
        out.push_str(commit_full);
        out.push(')');
    }

    out.push_str("\nbuilt:   ");
    out.push_str(&render_timestamp(timestamp));

    out.push_str("\ntree:    ");
    match tree_state {
        TREE_CLEAN => out.push_str(TREE_CLEAN),
        TREE_DIRTY => {
            out.push_str(DIRTY_MARKER);
            out.push_str("\nWARNING: built from a ");
            out.push_str(DIRTY_MARKER);
            out.push_str(" working tree — this binary is not reproducible from ");
            if commit_short == UNKNOWN {
                out.push_str("any commit.\n         ");
            } else {
                out.push_str("commit\n         ");
                out.push_str(commit_short);
                out.push_str(". ");
            }
            out.push_str("Do not attribute a measurement made with it to a source revision.");
        }
        _ => {
            out.push_str(UNKNOWN);
            out.push_str("\nWARNING: build provenance is UNKNOWN — no git metadata was available at build time.");
            out.push_str("\n         This binary is not attributable to any commit.");
        }
    }

    // No trailing newline: clap appends one when it renders `{name} {long_version}`. ~keep
    out
}

/// Render epoch seconds as an RFC 3339 UTC instant, degrading to `"unknown"` rather than to an
/// empty field: the point of the timestamp is telling apart same-version binaries built hours
/// apart, and a blank one tells you nothing while looking like it did. ~keep
fn render_timestamp(epoch_seconds: &str) -> String {
    epoch_seconds
        .parse::<i64>()
        .ok()
        .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
        .map_or_else(
            || UNKNOWN.to_string(),
            |instant| instant.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_SHA: &str = "964c552a267ccfb50a0e6f1d3b2c4a8e7f019d24";
    const SHORT_SHA: &str = "964c552a267c";
    /// 2001-09-09T01:46:40Z — a well-known instant, so the exact rendering below is checkable by
    /// inspection rather than by trusting a date library to agree with itself.
    const EPOCH: &str = "1000000000";

    fn render(tree_state: &str) -> String {
        render_long_version("0.61.0", SHORT_SHA, FULL_SHA, tree_state, EPOCH)
    }

    /// The whole point of the first line staying bare: `hooks/alef_hook.py`, release gates, and
    /// generated Homebrew `--version` checks all read the semver out of it.
    #[test]
    fn first_line_is_the_bare_semver_for_every_tree_state() {
        for state in [TREE_CLEAN, TREE_DIRTY, UNKNOWN] {
            let rendered = render(state);
            let first_line = rendered.lines().next().expect("rendered version has a first line");
            assert_eq!(first_line, "0.61.0", "first line changed for tree state {state}");
        }
    }

    /// The load-bearing assertion: the marker appears when and only when the build was stamped
    /// dirty. A clean or unattributable build must never print it, and a dirty one must never
    /// omit it.
    #[test]
    fn dirty_marker_appears_if_and_only_if_the_tree_state_is_dirty() {
        for (state, expected) in [(TREE_CLEAN, false), (TREE_DIRTY, true), (UNKNOWN, false)] {
            let rendered = render(state);
            assert_eq!(
                rendered.contains(DIRTY_MARKER),
                expected,
                "tree state {state} rendered the wrong dirty marker presence:\n{rendered}"
            );
        }
    }

    #[test]
    fn dirty_build_names_the_commit_it_cannot_be_reproduced_from() {
        let rendered = render(TREE_DIRTY);
        assert!(rendered.contains("not reproducible from commit"), "{rendered}");
        assert!(rendered.contains(SHORT_SHA), "{rendered}");
    }

    #[test]
    fn clean_build_reports_both_short_and_full_sha() {
        let rendered = render(TREE_CLEAN);
        assert!(
            rendered.contains(&format!("commit:  {SHORT_SHA} ({FULL_SHA})")),
            "{rendered}"
        );
        assert!(rendered.contains("tree:    clean"), "{rendered}");
        assert!(!rendered.contains("WARNING"), "{rendered}");
        assert!(!rendered.ends_with('\n'), "clap appends the trailing newline itself");
    }

    /// A tarball build with no `.git` must say so out loud. An empty or absent field would read as
    /// a clean, attributable build, which is the failure this stamp exists to prevent.
    #[test]
    fn missing_git_metadata_renders_explicit_unknown_never_an_empty_field() {
        let rendered = render_long_version("0.61.0", UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN);
        assert!(rendered.contains("commit:  unknown"), "{rendered}");
        assert!(rendered.contains("built:   unknown"), "{rendered}");
        assert!(rendered.contains("tree:    unknown"), "{rendered}");
        assert!(rendered.contains("not attributable to any commit"), "{rendered}");
        assert!(!rendered.contains(DIRTY_MARKER), "{rendered}");
    }

    /// `running_build_is_clean` must track whatever `TREE_STATE` this binary was actually
    /// compiled with -- not hardcode an assumption about the test environment's own tree state,
    /// which this suite does not control. ~keep
    #[test]
    fn running_build_is_clean_matches_the_compiled_in_tree_state() {
        assert_eq!(running_build_is_clean(), TREE_STATE == TREE_CLEAN);
    }

    #[test]
    fn timestamp_renders_as_utc_rfc3339() {
        assert_eq!(render_timestamp("0"), "1970-01-01T00:00:00Z");
        assert_eq!(render_timestamp(EPOCH), "2001-09-09T01:46:40Z");
        assert_eq!(render_timestamp(UNKNOWN), UNKNOWN);
        assert_eq!(render_timestamp(""), UNKNOWN);
        assert_eq!(render_timestamp(&i64::MAX.to_string()), UNKNOWN);
    }

    /// Guards the vars `build.rs` actually stamped into this build: whatever the environment was,
    /// none of them may be empty, because an empty field is the shape of a clean build.
    #[test]
    fn stamped_constants_are_never_empty() {
        for (name, value) in [
            ("ALEF_BUILD_COMMIT", COMMIT),
            ("ALEF_BUILD_COMMIT_SHORT", COMMIT_SHORT),
            ("ALEF_BUILD_TREE_STATE", TREE_STATE),
            ("ALEF_BUILD_TIMESTAMP", TIMESTAMP),
        ] {
            assert!(!value.trim().is_empty(), "{name} was stamped empty");
        }
        assert!(
            matches!(TREE_STATE, TREE_CLEAN | TREE_DIRTY | UNKNOWN),
            "unexpected tree state {TREE_STATE}"
        );
    }

    /// A second sha, differing from [`FULL_SHA`] in every position so a truncating bug cannot
    /// make the two look equal by accident. ~keep
    const OTHER_SHA: &str = "1d3b5f7902e4c6a8b0d2f4061e3c5a7b9d0f2e48";

    /// The defect: during a release cycle many candidate binaries self-report one semver while
    /// being built from different commits, so a cache salted on the semver alone replays one
    /// candidate's verdicts under the next. Everything but the commit is held identical here. ~keep
    #[test]
    fn two_clean_builds_of_one_version_differ_when_only_the_commit_differs() {
        assert_ne!(
            render_build_identity("0.72.0", FULL_SHA, TREE_CLEAN, EPOCH),
            render_build_identity("0.72.0", OTHER_SHA, TREE_CLEAN, EPOCH),
            "two candidate binaries reporting the same semver from different commits must not \
             share a cache identity, or a fix shipped between them is masked by a stale verdict"
        );
    }

    /// The half that proves the identity did not simply become unique per run. A released binary
    /// is built from a tag with a clean tree: two builds of it, on two machines at two times, must
    /// agree — otherwise every release run is a cold-cache run of an expensive validation pass. ~keep
    #[test]
    fn two_clean_builds_of_one_commit_agree_regardless_of_build_time() {
        assert_eq!(
            render_build_identity("0.72.0", FULL_SHA, TREE_CLEAN, EPOCH),
            render_build_identity("0.72.0", FULL_SHA, TREE_CLEAN, "1999999999"),
            "a clean build's identity must depend on the commit, not on when it was compiled, or \
             released binaries never share a warm cache"
        );
    }

    /// A crates.io tarball has no `.git`, so its commit is `unknown` for everyone — but its source
    /// is byte-identical at a given version, so the semver already identifies it and the identity
    /// must stay shareable. This is what keeps a consumer's CI cache warm across `cargo install
    /// alef` re-installs. ~keep
    #[test]
    fn builds_without_git_metadata_stay_shareable_across_install_times() {
        assert_eq!(
            render_build_identity("0.72.0", UNKNOWN, UNKNOWN, EPOCH),
            render_build_identity("0.72.0", UNKNOWN, UNKNOWN, "1999999999"),
            "a tarball build's identity must not vary with install time, or every consumer CI run \
             that re-installs alef discards its restored snippet cache"
        );
    }

    /// The dirty case a commit sha cannot cover: two different working trees at one commit are two
    /// different binaries. The build time is the discriminator because it moves exactly when
    /// `build.rs` re-runs, which is when a watched compile input changed. ~keep
    #[test]
    fn dirty_builds_at_one_commit_are_separated_by_build_time() {
        assert_ne!(
            render_build_identity("0.72.0", FULL_SHA, TREE_DIRTY, EPOCH),
            render_build_identity("0.72.0", FULL_SHA, TREE_DIRTY, "1999999999"),
            "two different dirty trees at one commit must not share a cache identity"
        );
        assert_eq!(
            render_build_identity("0.72.0", FULL_SHA, TREE_DIRTY, EPOCH),
            render_build_identity("0.72.0", FULL_SHA, TREE_DIRTY, EPOCH),
            "one dirty binary must stay warm across repeated runs; this is a salt, not a bypass"
        );
    }

    /// A dirty tree must never be mistaken for the clean build at the same commit. ~keep
    #[test]
    fn tree_state_alone_separates_a_dirty_build_from_the_clean_one_beneath_it() {
        assert_ne!(
            render_build_identity("0.72.0", FULL_SHA, TREE_CLEAN, EPOCH),
            render_build_identity("0.72.0", FULL_SHA, TREE_DIRTY, EPOCH)
        );
    }

    #[test]
    fn version_bump_alone_still_moves_the_identity() {
        assert_ne!(
            render_build_identity("0.72.0", FULL_SHA, TREE_CLEAN, EPOCH),
            render_build_identity("0.72.1", FULL_SHA, TREE_CLEAN, EPOCH)
        );
    }

    /// The wiring assertion: `build_identity` must render the constants this binary was actually
    /// stamped with, not merely expose a pure helper that nothing calls with real provenance. ~keep
    #[test]
    fn build_identity_renders_the_constants_this_binary_was_stamped_with() {
        assert_eq!(
            build_identity(),
            render_build_identity(env!("CARGO_PKG_VERSION"), COMMIT, TREE_STATE, TIMESTAMP)
        );
        assert_eq!(
            build_identity(),
            build_identity(),
            "identity must be stable within a run"
        );
        assert!(
            build_identity().starts_with(env!("CARGO_PKG_VERSION")),
            "{}",
            build_identity()
        );
        assert_ne!(
            build_identity(),
            env!("CARGO_PKG_VERSION"),
            "the identity must carry more than the semver, which is the entire defect"
        );
    }

    #[test]
    fn long_version_is_stable_across_calls_and_contains_the_package_version() {
        let first = long_version();
        assert_eq!(first, long_version());
        assert!(first.starts_with(env!("CARGO_PKG_VERSION")), "{first}");
    }
}
