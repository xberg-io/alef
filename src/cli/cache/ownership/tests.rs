use super::*;
use crate::cli::cache::{CACHE_DIR, LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST, OWNERSHIP_MANIFEST};

/// THE DEFECT: a path whose ownership predates #80 and has never been rewritten since
/// lives only under the gitignored `.alef/` cache -- see
/// [`LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST`]'s doc. A consumer that clears `.alef` (a
/// clean, an upgrade, a fresh CI runner) used to lose that record outright: alef had
/// no durable proof left that it ever wrote the file, and refused to overwrite it. One
/// consumer hit this for 48 of its own outputs, including every README.
///
/// The fix: any query against the legacy record -- exactly what `alef verify`'s
/// frozen-file scan and every ownership-gated write already perform, unconditionally,
/// for every unmarkable managed path -- promotes it into the committed
/// [`OWNERSHIP_MANIFEST`] immediately, not only on the next authorised write. Once
/// promoted, the entry lives at the repo root, outside `.alef/`, so clearing the cache
/// afterward is harmless.
///
/// Red without the fix: the first query is a plain read with no side effect, so after
/// `.alef` is removed the second query finds nothing in either record and answers
/// `false`. ~keep
#[test]
fn legacy_only_ownership_survives_the_cache_being_cleared_once_queried() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = Path::new("packages/java/pom.xml");

    std::fs::create_dir_all(base.join(CACHE_DIR)).expect("create legacy cache dir");
    std::fs::write(
        base.join(CACHE_DIR).join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST),
        "packages/java/pom.xml\n",
    )
    .expect("seed legacy record");
    assert!(
        !base.join(OWNERSHIP_MANIFEST).exists(),
        "ownership starts recorded only in the legacy cache"
    );

    assert!(
        is_scaffold_owned_path(base, &base.join(relative)),
        "sanity: the legacy record alone must already answer true"
    );

    std::fs::remove_dir_all(base.join(CACHE_DIR)).expect("clear the cache, as `alef clean` / a fresh CI runner would");
    assert!(!base.join(CACHE_DIR).exists(), "sanity: the cache is now wholly absent");

    assert!(
        is_scaffold_owned_path(base, &base.join(relative)),
        "ownership queried once while the legacy cache still existed must survive the cache being cleared"
    );
}

/// The companion guard for the fix above: a path that was never recorded anywhere --
/// not the legacy cache, not the committed record -- must stay refused even though the
/// cache is also wholly absent. Without this, "the cache is absent, so forgive it"
/// would be indistinguishable from disabling the ownership check outright: a file alef
/// genuinely did not author must still be refused. ~keep
#[test]
fn path_never_recorded_stays_refused_even_with_no_cache_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = Path::new("packages/java/pom.xml");

    assert!(!base.join(CACHE_DIR).exists(), "sanity: no cache at all");
    assert!(
        !base.join(OWNERSHIP_MANIFEST).exists(),
        "sanity: no committed record either"
    );

    assert!(
        !is_scaffold_owned_path(base, &base.join(relative)),
        "a path alef never recorded must stay refused, cache absent or not"
    );
}

/// A record written by a pre-#80 alef, which exists only under the
/// gitignored cache, must keep working -- otherwise upgrading turns every
/// unmarkable file in every existing consumer repo into a refusal at once.
#[test]
fn legacy_gitignored_record_is_still_honoured_for_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = Path::new("packages/java/pom.xml");

    std::fs::create_dir_all(base.join(CACHE_DIR)).expect("create legacy cache dir");
    std::fs::write(
        base.join(CACHE_DIR).join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST),
        "packages/java/pom.xml\n",
    )
    .expect("seed legacy record");

    assert!(!base.join(OWNERSHIP_MANIFEST).exists(), "no committed record yet");
    assert!(is_scaffold_owned_path(base, &base.join(relative)));
}

/// Content verification may rely only on the committed ownership record: a legacy cache entry
/// is accepted for guarded writes but is not reproducible evidence in a fresh clone. ~keep
#[test]
fn committed_ownership_query_excludes_legacy_only_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = Path::new("test_apps/kotlin/gradlew");
    std::fs::create_dir_all(base.join(CACHE_DIR)).expect("create legacy cache dir");
    std::fs::write(
        base.join(CACHE_DIR).join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST),
        "test_apps/kotlin/gradlew\n",
    )
    .expect("seed legacy record");

    assert!(!is_committed_scaffold_owned_path(base, &base.join(relative)));

    crate::cli::cache::record_scaffold_owned_path(base, &base.join(relative)).expect("record committed ownership");
    assert!(is_committed_scaffold_owned_path(base, &base.join(relative)));
}
