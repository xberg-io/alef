use super::tests::{TOOLCHAIN_TEST_TIMEOUT_SECS, sample_package, sample_project, zig_snippet};
use super::*;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const EXPECTED_RECEIPTS: usize = 8;

#[derive(Default, serde::Serialize)]
struct CacheMetrics {
    files: u64,
    bytes: u64,
}

fn cache_metrics(root: &Path) -> CacheMetrics {
    let mut metrics = CacheMetrics::default();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        if !path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&path).expect("read fixture cache") {
            let entry = entry.expect("cache entry");
            let kind = entry.file_type().expect("cache entry type");
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                metrics.files += 1;
                metrics.bytes += entry.metadata().expect("cache entry metadata").len();
            }
        }
    }
    metrics
}

fn global_cache(session: &ValidationSession) -> PathBuf {
    let mut command = std::process::Command::new("zig");
    session.apply_environment(&mut command);
    let path = command
        .get_envs()
        .find_map(|(name, value)| (name == "ZIG_GLOBAL_CACHE_DIR").then_some(value).flatten())
        .map(PathBuf::from)
        .expect("session sets its Zig global cache");
    assert!(path.is_absolute() && path.starts_with(&session.working_directory));
    path
}

struct Case {
    _root: tempfile::TempDir,
    session: ValidationSession,
    name: &'static str,
    module: &'static str,
    expected: SnippetStatus,
}

fn cases() -> Vec<Case> {
    [
        (
            sample_project(true),
            "include_declared",
            "sample_binding",
            SnippetStatus::Pass,
        ),
        (
            sample_project(false),
            "include_missing",
            "sample_binding",
            SnippetStatus::Fail,
        ),
        (
            sample_package(true),
            "cwd_rebased",
            "sample_package",
            SnippetStatus::Pass,
        ),
        (sample_package(false), "cwd_raw", "sample_package", SnippetStatus::Fail),
    ]
    .into_iter()
    .map(|((root, session), name, module, expected)| Case {
        _root: root,
        session,
        name,
        module,
        expected,
    })
    .collect()
}

fn cache_before(case: &Case, phase: &str) -> (PathBuf, CacheMetrics) {
    let cache = global_cache(&case.session);
    let before = cache_metrics(&cache);
    if phase == "cold" {
        assert_eq!(
            before.files,
            0,
            "cold fixture cache must start empty: {}",
            cache.display()
        );
    } else {
        assert!(
            before.files > 0,
            "warm fixture cache must contain the cold run's artifacts"
        );
    }
    (cache, before)
}

#[allow(clippy::print_stdout, reason = "Diagnostic receipts are this test harness's output.")]
fn run_case(case: &Case, phase: &str) -> bool {
    let (cache, before) = cache_before(case, phase);
    let snippet = zig_snippet(&format!(
        "const package = @import(\"{}\");\npub fn main() void {{ _ = package.value(); }}\n",
        case.module
    ));
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    println!(
        "ALEF_ZIG_DIAGNOSTIC_START {}",
        serde_json::json!({
            "phase": phase, "case": case.name, "started_ms": started, "cache": cache,
            "expected_receipts": EXPECTED_RECEIPTS, "budget_seconds": TOOLCHAIN_TEST_TIMEOUT_SECS,
        })
    );
    let timer = Instant::now();
    let result = ZigValidator.validate_in_session(
        &snippet,
        ValidationLevel::Compile,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        Some(&case.session),
    );
    let (status, detail) = match result {
        Ok((status, output)) => (status, output),
        Err(error) => (SnippetStatus::Error, Some(error.to_string())),
    };
    let control_fired = case.expected == SnippetStatus::Pass
        || detail
            .as_deref()
            .is_some_and(|text| text.contains("'fixture.h' not found"));
    let qualified = status == case.expected && control_fired;
    println!(
        "ALEF_ZIG_DIAGNOSTIC_RECEIPT {}",
        serde_json::json!({
            "phase": phase, "case": case.name, "started_ms": started,
            "elapsed_ms": timer.elapsed().as_millis(), "budget_seconds": TOOLCHAIN_TEST_TIMEOUT_SECS,
            "cache": cache, "cache_before": before, "cache_after": cache_metrics(&cache),
            "expected": case.expected, "actual": status, "qualified": qualified, "detail": detail,
        })
    );
    qualified
}

fn assert_qualified(receipts: &[bool]) {
    assert_eq!(
        receipts.len(),
        EXPECTED_RECEIPTS,
        "every cold/warm fixture must execute"
    );
    assert_eq!(
        receipts.iter().filter(|qualified| **qualified).count(),
        EXPECTED_RECEIPTS
    );
}

#[test]
#[ignore = "Run explicitly under a Windows process-tree CPU/RSS sampler; each command retains its 300-second budget."]
fn qualify_cold_and_warm_real_consumer_cases() {
    if !cfg!(windows) {
        panic!("this diagnostic qualifies Windows runs only");
    }
    assert!(zig_is_runnable(), "the diagnostic must compile real Zig consumers");
    let cases = cases();
    assert_eq!(cases.len(), EXPECTED_RECEIPTS / 2);
    let receipts = ["cold", "warm"]
        .into_iter()
        .flat_map(|phase| cases.iter().map(move |case| run_case(case, phase)))
        .collect::<Vec<_>>();
    assert_qualified(&receipts);
}

#[test]
fn should_reject_empty_and_unqualified_diagnostic_receipts() {
    assert!(std::panic::catch_unwind(|| assert_qualified(&[])).is_err());
    let mut receipts = [true; EXPECTED_RECEIPTS];
    receipts[0] = false;
    assert!(std::panic::catch_unwind(|| assert_qualified(&receipts)).is_err());
    assert_qualified(&[true; EXPECTED_RECEIPTS]);
}

#[test]
fn should_measure_exact_fixture_cache_counts_and_bytes() {
    let directory = tempfile::tempdir().expect("cache fixture");
    std::fs::create_dir(directory.path().join("nested")).expect("nested cache");
    std::fs::write(directory.path().join("first"), b"abc").expect("first cache entry");
    std::fs::write(directory.path().join("nested/second"), b"de").expect("second cache entry");
    let metrics = cache_metrics(directory.path());
    assert_eq!(metrics.files, 2);
    assert_eq!(metrics.bytes, 5);
}
