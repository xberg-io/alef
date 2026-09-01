//! Documents a residual, out-of-scope false-negative in the pending-publish exemption --
//! deliberately NOT fixed here (Phase 3 of the lockfile-freshness work; this lives in the shared
//! release-gate discovery, `version_manifests.rs`, which is out of that phase's scope).
//!
//! `check_generated_lock_freshness_tolerating_pending_publish`'s exemption
//! (`super::explained_by_pending_publish`, defined in this same file, `version_lockfiles.rs`)
//! trusts `discover_cargo_locks`'s `blocked_on_publish`, which `unpublished_dependency`
//! (`crate::cli::commands::version_manifests.rs:192-218`) derives from
//! `registry_dependencies_on_local_crates` (same file, :222-253). That function keys "is this
//! lock waiting on our own pending release" purely on (dependency name, exact version) matching
//! ANY git-tracked in-tree `[package]` -- not specifically the crate this run is releasing. If
//! some OTHER in-tree crate happens to share both a name and a version with the dependency a
//! lock has genuinely drifted on, the drift is wrongly exempted.
//!
//! The real xberg incident this module's sibling tests were built for did NOT hit this path --
//! `git ls-files '*Cargo.toml' | xargs grep -l '^name = "crawlberg"'` in that repo returns
//! nothing, so no such collision existed there. This is a latent defect proven on a synthetic
//! fixture, not a reproduction of the field incident.

use super::*;

const NATIVE_RELATIVE_DIR: &str = "packages/elixir/native/demo_nif";
const CANONICAL: &str = "1.5.0";
const THIRD_PARTY_DEPENDENCY: &str = "demo-upstream-crate";
const STALE_PIN: &str = "1.4.2";

fn write_native_manifest(root: &Path, requirement: &str) -> PathBuf {
    let dir = root.join(NATIVE_RELATIVE_DIR);
    std::fs::create_dir_all(&dir).expect("create native dir");
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        format!(
            "[package]\nname = \"demo_nif\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\n{THIRD_PARTY_DEPENDENCY} = \"{requirement}\"\n"
        ),
    )
    .expect("write native Cargo.toml");
    manifest
}

fn write_native_lock(root: &Path, pin: &str) {
    std::fs::write(
        root.join(NATIVE_RELATIVE_DIR).join("Cargo.lock"),
        format!(
            "version = 3\n\n\
             [[package]]\nname = \"demo_nif\"\nversion = \"0.1.0\"\n\n\
             [[package]]\nname = \"{THIRD_PARTY_DEPENDENCY}\"\nversion = \"{pin}\"\nsource = \
             \"registry+https://github.com/rust-lang/crates.io-index\"\n"
        ),
    )
    .expect("write native Cargo.lock");
}

/// Control: with no colliding in-tree package, the drift is reported correctly. Without this,
/// the "documents a defect" test below would be meaningless -- it needs to show the SAME input
/// passes clean once collision is removed, not merely that the checker never fires.
#[test]
fn without_a_colliding_in_tree_package_the_drift_is_reported() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_native_manifest(root, CANONICAL);
    write_native_lock(root, STALE_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    let result = check_generated_lock_freshness_tolerating_pending_publish(&generated, root, Some(CANONICAL));
    assert!(
        result.is_some(),
        "a genuine third-party drift with nothing else in the tree must fail: {result:?}"
    );
}

/// The residual defect: an unrelated in-tree crate that happens to share the drifting
/// dependency's NAME and the run's canonical VERSION makes `unpublished_dependency`
/// (`version_manifests.rs:192-218`) mark the lock `blocked_on_publish`, and
/// `explained_by_pending_publish` then exempts the genuinely stale third-party pin as if it were
/// this crate's own not-yet-published self-dependency. Nothing about `THIRD_PARTY_DEPENDENCY`
/// here is this run's own package -- the collision is purely coincidental on name + version,
/// which is exactly what `registry_dependencies_on_local_crates` fails to rule out.
#[test]
fn an_unrelated_in_tree_package_sharing_name_and_canonical_version_wrongly_exempts_the_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    // Some OTHER crate in the same tree happens to be named after the third-party dependency,
    // at exactly the version this run is releasing -- pure coincidence, unrelated to the native
    // extension manifest's own drift below.
    std::fs::create_dir_all(root.join("some-other-crate")).expect("mkdir colliding crate");
    std::fs::write(
        root.join("some-other-crate/Cargo.toml"),
        format!("[package]\nname = \"{THIRD_PARTY_DEPENDENCY}\"\nversion = \"{CANONICAL}\"\nedition = \"2024\"\n"),
    )
    .expect("write colliding Cargo.toml");

    let manifest = write_native_manifest(root, CANONICAL);
    write_native_lock(root, STALE_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    let result = check_generated_lock_freshness_tolerating_pending_publish(&generated, root, Some(CANONICAL));
    assert!(
        result.is_none(),
        "KNOWN GAP (not fixed by design, see module doc): a coincidental in-tree name+version \
         collision wrongly exempts a real third-party drift. If this assertion ever starts \
         failing, `unpublished_dependency` has been made release-identity-aware and this whole \
         file's module doc should be revisited: {result:?}"
    );
}
