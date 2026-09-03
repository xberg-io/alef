//! Non-vacuous regression proving `scripts/publish/commit-scoop-manifest.sh` catches the
//! untracked-first-publish bug that a plain `git diff --quiet <path>` (checked against the
//! working tree, without staging first) misses.
//!
//! On the very first Scoop publish, `bucket/alef.json` does not exist in the bucket repo's
//! history yet -- it is untracked. Plain `git diff <path>` never reports untracked paths, so
//! `git diff --quiet <path>` exits 0 ("no differences") even though the file is brand new,
//! and a commit step built on that check would silently conclude "nothing to publish" on the
//! one run where publishing matters most. The fix stages the file first and diffs the index
//! (`git diff --cached --quiet -- <path>`), which does see a newly-staged file as a change.
//!
//! `first_publish_of_untracked_manifest_is_committed` proves the fixed script commits on that
//! exact scenario. `old_working_tree_diff_form_misses_the_same_scenario` reproduces the old,
//! broken form directly against the identical fixture and asserts it reports "no differences"
//! -- if this test passed either way, it would not be testing the fix.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/publish/commit-scoop-manifest.sh")
}

fn run_git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|error| panic!("git {args:?} must run in {dir:?}: {error}"))
}

fn git_ok(dir: &Path, args: &[&str]) {
    let output = run_git(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed in {dir:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Sets up a bucket checkout with an initial commit (an unrelated file, mirroring a real
/// scoop-bucket repo that already carries manifests for other apps) and a local bare "origin"
/// so `git push origin HEAD` in the script under test succeeds without network access.
struct BucketFixture {
    dir: tempfile::TempDir,
    clone_counter: AtomicUsize,
}

impl BucketFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let bare_dir = dir.path().join("origin.git");
        let bucket_dir = dir.path().join("bucket-checkout");

        git_ok(dir.path(), &["init", "-q", "--bare", bare_dir.to_str().unwrap()]);
        git_ok(
            dir.path(),
            &["clone", "-q", bare_dir.to_str().unwrap(), bucket_dir.to_str().unwrap()],
        );
        git_ok(&bucket_dir, &["config", "user.email", "test@example.invalid"]);
        git_ok(&bucket_dir, &["config", "user.name", "test"]);

        std::fs::create_dir_all(bucket_dir.join("bucket")).expect("create bucket dir");
        std::fs::write(bucket_dir.join("bucket/other-app.json"), "{\"version\":\"1.0.0\"}\n").expect("write fixture");
        git_ok(&bucket_dir, &["add", "bucket/other-app.json"]);
        git_ok(&bucket_dir, &["commit", "-q", "-m", "seed: other-app manifest"]);
        git_ok(&bucket_dir, &["push", "-q", "origin", "HEAD"]);

        Self {
            dir,
            clone_counter: AtomicUsize::new(0),
        }
    }

    fn bucket_dir(&self) -> PathBuf {
        self.dir.path().join("bucket-checkout")
    }

    /// A fresh clone of the same origin, used to read back what was actually pushed without
    /// relying on the working checkout's own (possibly stale) view.
    fn reclone(&self) -> PathBuf {
        let bare_dir = self.dir.path().join("origin.git");
        // ~keep A unique target per call. `no_op_retry_on_unchanged_manifest_makes_no_commit`
        // reclones twice -- once after each script run -- to compare the pushed HEAD before and
        // after; a fixed path made the second `git clone` fail with "destination path already
        // exists and is not an empty directory", which reads as a script failure rather than as
        // the harness colliding with itself. Reusing one path by deleting it first would also
        // invalidate the PathBuf the caller still holds from the first clone.
        let index = self.clone_counter.fetch_add(1, Ordering::Relaxed);
        let target = self.dir.path().join(format!("verify-checkout-{index}"));
        git_ok(
            self.dir.path(),
            &["clone", "-q", bare_dir.to_str().unwrap(), target.to_str().unwrap()],
        );
        target
    }
}

fn run_commit_script(bucket_dir: &Path, manifest_path: &str, version: &str) -> Output {
    Command::new("bash")
        .arg(script_path())
        .env("BUCKET_DIR", bucket_dir)
        .env("MANIFEST_PATH", manifest_path)
        .env("VERSION", version)
        .output()
        .expect("commit-scoop-manifest.sh must run")
}

#[test]
fn first_publish_of_untracked_manifest_is_committed() {
    let fixture = BucketFixture::new();
    let bucket_dir = fixture.bucket_dir();

    // The manifest does not exist yet -- this is the untracked-first-publish scenario.
    std::fs::write(bucket_dir.join("bucket/alef.json"), "{\"version\":\"1.0.0\"}\n").expect("write manifest");

    let output = run_commit_script(&bucket_dir, "bucket/alef.json", "1.0.0");
    assert!(
        output.status.success(),
        "commit-scoop-manifest.sh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim_end().ends_with("committed"),
        "expected the untracked manifest to be committed, got: {stdout}"
    );

    // Verify against a fresh clone of origin, not the working checkout, so this proves the
    // commit was actually pushed, not just made locally.
    let verify_dir = fixture.reclone();
    let content = std::fs::read_to_string(verify_dir.join("bucket/alef.json")).expect("read pushed manifest");
    assert_eq!(content, "{\"version\":\"1.0.0\"}\n");

    let log = run_git(&verify_dir, &["log", "--oneline", "-1"]);
    let log_message = String::from_utf8_lossy(&log.stdout);
    assert!(
        log_message.contains("alef 1.0.0"),
        "unexpected commit message: {log_message}"
    );
}

#[test]
fn no_op_retry_on_unchanged_manifest_makes_no_commit() {
    let fixture = BucketFixture::new();
    let bucket_dir = fixture.bucket_dir();
    std::fs::write(bucket_dir.join("bucket/alef.json"), "{\"version\":\"1.0.0\"}\n").expect("write manifest");

    let first = run_commit_script(&bucket_dir, "bucket/alef.json", "1.0.0");
    assert!(first.status.success());
    let verify_after_first = fixture.reclone();
    let sha_after_first = run_git(&verify_after_first, &["rev-parse", "HEAD"]);

    // Re-render the identical content (as a real no-op retry would) and run again.
    std::fs::write(bucket_dir.join("bucket/alef.json"), "{\"version\":\"1.0.0\"}\n").expect("rewrite manifest");
    let second = run_commit_script(&bucket_dir, "bucket/alef.json", "1.0.0");
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.trim_end().ends_with("skipped"),
        "expected an unchanged manifest to be skipped, got: {stdout}"
    );

    let verify_after_second = fixture.reclone();
    let sha_after_second = run_git(&verify_after_second, &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&sha_after_first.stdout),
        String::from_utf8_lossy(&sha_after_second.stdout),
        "an unchanged manifest must not produce a new commit"
    );
}

#[test]
fn real_version_bump_on_tracked_manifest_is_committed() {
    let fixture = BucketFixture::new();
    let bucket_dir = fixture.bucket_dir();
    std::fs::write(bucket_dir.join("bucket/alef.json"), "{\"version\":\"1.0.0\"}\n").expect("write manifest");
    let first = run_commit_script(&bucket_dir, "bucket/alef.json", "1.0.0");
    assert!(first.status.success());

    std::fs::write(bucket_dir.join("bucket/alef.json"), "{\"version\":\"1.0.1\"}\n").expect("bump manifest");
    let second = run_commit_script(&bucket_dir, "bucket/alef.json", "1.0.1");
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.trim_end().ends_with("committed"),
        "expected a real version bump to be committed, got: {stdout}"
    );

    let verify_dir = fixture.reclone();
    let content = std::fs::read_to_string(verify_dir.join("bucket/alef.json")).expect("read pushed manifest");
    assert_eq!(content, "{\"version\":\"1.0.1\"}\n");
}

#[test]
fn old_working_tree_diff_form_misses_the_same_scenario() {
    // Reproduces the exact bug this script fixes, directly against the identical
    // untracked-first-publish fixture: `git diff --quiet <path>`, checked against the working
    // tree without staging first, reports "no differences" (exit 0) for an untracked file.
    // This is the discriminating control -- it proves the OLD form would have silently
    // skipped the commit on this fixture, so `first_publish_of_untracked_manifest_is_committed`
    // above is actually exercising the fix, not a scenario where both forms happen to agree.
    let fixture = BucketFixture::new();
    let bucket_dir = fixture.bucket_dir();
    std::fs::write(bucket_dir.join("bucket/alef.json"), "{\"version\":\"1.0.0\"}\n").expect("write manifest");

    let status = Command::new("git")
        .args(["diff", "--quiet", "bucket/alef.json"])
        .current_dir(&bucket_dir)
        .status()
        .expect("git diff must run");

    assert!(
        status.success(),
        "the old `git diff --quiet` form was expected to report no differences (exit 0) for an \
         untracked file -- if it did not, this control no longer reproduces the bug it exists to guard against"
    );
}
