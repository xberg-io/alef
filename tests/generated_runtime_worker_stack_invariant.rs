//! Generator-wide invariant: no tokio runtime this crate *emits* may run on tokio's default
//! worker stack.
//!
//! A stack overflow inside a tokio worker thread is not a catchable panic — the guard page fault
//! aborts the whole process (SIGBUS / `EXC_BAD_ACCESS`, `KERN_PROTECTION_FAILURE` on macOS). A
//! consumer whose async work is deep (a nested archive member, a multi-stage OCR pipeline) will
//! therefore take down an entire test process, not just the one call. Tokio's default worker
//! stack is ~2 MB, which is not enough headroom for that shape of future.
//!
//! This invariant has been re-fixed at least six times, backend by backend, because nothing
//! asserted it across the generator as a whole: each fix was scoped to the one backend whose
//! consumer happened to crash. This test is the generator-wide assertion that was missing. It is
//! deliberately a *source and artifact* scan rather than a per-backend generate-and-inspect,
//! because a per-backend test can only cover the backends someone remembered to enumerate,
//! which is precisely how the defect kept surviving.
//!
//! Two rules are enforced over every generator source file, every code template, and every
//! committed snapshot of generated output:
//!
//! 1. `Runtime::new()` is banned. It builds a multi-thread runtime with default worker stacks
//!    and offers no way to widen them.
//! 2. Every `Builder::new_multi_thread()` must set `.thread_stack_size(...)` nearby.
//!
//! `Builder::new_current_thread()` is intentionally *not* covered. A current-thread runtime
//! spawns no worker threads; `block_on` drives the future on the calling thread's stack, so
//! `thread_stack_size` would not govern the future's depth at all (it would only size the
//! blocking pool). Sites that need headroom there must widen the *calling* thread instead, which
//! is a different fix and cannot be asserted by this scan.

use std::fs;
use std::path::{Path, PathBuf};

/// Constructor that cannot express a worker stack size; banned outright in emitted code.
const BANNED_CONSTRUCTOR: &str = "Runtime::new()";

/// Builder entry point that spawns worker threads, and therefore must be given a stack size.
const MULTI_THREAD_BUILDER: &str = "Builder::new_multi_thread()";

/// The setter that widens the worker stack.
const STACK_SIZE_SETTER: &str = ".thread_stack_size(";

/// How far after a `Builder::new_multi_thread()` the setter may appear. Emitted builder chains
/// are split across source lines by string-literal continuations, so the setter is never more
/// than a few lines away; a generous window still catches an omission.
const STACK_SIZE_LOOKAHEAD_LINES: usize = 8;

/// Floors that prove the scan actually examined something. A walker that silently matched no
/// files would otherwise report a vacuous pass, which is the same failure mode this test exists
/// to catch. These are well under the real counts at the time of writing (4900+ source files,
/// 30+ multi-thread sites, 130+ snapshots, 20+ multi-thread sites in snapshots).
const MIN_SOURCE_FILES_SCANNED: usize = 1_000;
const MIN_SOURCE_MULTI_THREAD_SITES: usize = 20;
const MIN_SNAPSHOT_FILES_SCANNED: usize = 50;
const MIN_SNAPSHOT_MULTI_THREAD_SITES: usize = 10;

/// A line that mentions a runtime constructor inside `.contains(` is a test assertion about
/// emitted code, not emitted code itself. Exempting only assertion lines keeps the scan free of
/// blind spots: a generator that re-introduces the banned constructor still trips on its own
/// line even when a test asserts the same text.
fn is_assertion_line(line: &str) -> bool {
    line.contains(".contains(")
}

fn is_comment_line(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

fn collect_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => panic!("failed to read {}: {e}", dir.display()),
        };
        for entry in entries {
            let path = entry.expect("failed to read directory entry").path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "target" && name != ".git" {
                    stack.push(path);
                }
                continue;
            }
            if extensions.iter().any(|ext| name.ends_with(ext)) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

struct ScanReport {
    files_scanned: usize,
    multi_thread_sites: usize,
    banned_constructor: Vec<String>,
    missing_stack_size: Vec<String>,
}

fn scan(files: &[PathBuf], repo_root: &Path) -> ScanReport {
    let mut report = ScanReport {
        files_scanned: files.len(),
        multi_thread_sites: 0,
        banned_constructor: Vec::new(),
        missing_stack_size: Vec::new(),
    };

    for path in files {
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = content.lines().collect();

        for (index, line) in lines.iter().enumerate() {
            if is_assertion_line(line) || is_comment_line(line) {
                continue;
            }
            let relative = path.strip_prefix(repo_root).unwrap_or(path).display();
            let location = format!("{relative}:{}", index + 1);

            if line.contains(BANNED_CONSTRUCTOR) {
                report.banned_constructor.push(location.clone());
            }

            if line.contains(MULTI_THREAD_BUILDER) {
                report.multi_thread_sites += 1;
                let end = (index + STACK_SIZE_LOOKAHEAD_LINES).min(lines.len());
                let window = lines[index..end].join("\n");
                if !window.contains(STACK_SIZE_SETTER) {
                    report.missing_stack_size.push(location);
                }
            }
        }
    }

    report
}

fn assert_clean(report: &ScanReport, what: &str) {
    assert!(
        report.banned_constructor.is_empty(),
        "{what}: `{BANNED_CONSTRUCTOR}` builds a multi-thread runtime with tokio's default ~2 MB \
         worker stack, which a deep consumer future can overflow — and a worker-stack overflow \
         aborts the process with SIGBUS instead of raising a catchable panic. Use \
         `Builder::new_multi_thread().enable_all().thread_stack_size(<named const>).build()` \
         instead. Offending sites:\n  {}",
        report.banned_constructor.join("\n  ")
    );
    assert!(
        report.missing_stack_size.is_empty(),
        "{what}: `{MULTI_THREAD_BUILDER}` spawns worker threads and must set \
         `{STACK_SIZE_SETTER}<named const>)` within {STACK_SIZE_LOOKAHEAD_LINES} lines. Without \
         it the workers get tokio's default ~2 MB stack and a deep consumer future aborts the \
         process with SIGBUS. Offending sites:\n  {}",
        report.missing_stack_size.join("\n  ")
    );
}

/// Every generator source file and code template must emit runtimes with an explicit worker
/// stack size. This covers all backends at once, including any backend added after this test.
#[test]
fn no_generator_emits_a_runtime_with_tokios_default_worker_stack() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = collect_files(&repo_root.join("src"), &[".rs", ".jinja"]);
    let report = scan(&files, &repo_root);

    assert!(
        report.files_scanned >= MIN_SOURCE_FILES_SCANNED,
        "the scan examined only {} source files, far below the expected floor of \
         {MIN_SOURCE_FILES_SCANNED} — the walker is broken and this test would pass vacuously",
        report.files_scanned
    );
    assert!(
        report.multi_thread_sites >= MIN_SOURCE_MULTI_THREAD_SITES,
        "the scan found only {} `{MULTI_THREAD_BUILDER}` sites, below the expected floor of \
         {MIN_SOURCE_MULTI_THREAD_SITES} — either the emitted runtimes moved to a shape this \
         test no longer recognizes, or the scan is not reading the files it thinks it is",
        report.multi_thread_sites
    );

    assert_clean(&report, "generator source and templates");
}

/// The committed snapshots are the generator's rendered output, so they are the artifact a
/// consumer actually compiles. A source fix that never reached a snapshot is a fix that did not
/// land, which is exactly what this arm catches.
#[test]
fn no_generated_snapshot_contains_a_runtime_with_tokios_default_worker_stack() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = collect_files(&repo_root.join("tests").join("snapshots"), &[".snap"]);
    let report = scan(&files, &repo_root);

    assert!(
        report.files_scanned >= MIN_SNAPSHOT_FILES_SCANNED,
        "the scan examined only {} snapshot files, far below the expected floor of \
         {MIN_SNAPSHOT_FILES_SCANNED} — the walker is broken and this test would pass vacuously",
        report.files_scanned
    );
    assert!(
        report.multi_thread_sites >= MIN_SNAPSHOT_MULTI_THREAD_SITES,
        "the scan found only {} `{MULTI_THREAD_BUILDER}` sites in snapshots, below the expected \
         floor of {MIN_SNAPSHOT_MULTI_THREAD_SITES} — the snapshots no longer capture the \
         generated runtimes, so this arm has stopped proving anything",
        report.multi_thread_sites
    );

    assert_clean(&report, "committed snapshots of generated output");
}

/// No committed snapshot of generated output may carry alef's internal `~keep` marker.
///
/// `~keep` tells `poly`'s uncomment pass to spare a comment. It is meaningful in a source tree
/// `poly` reads, and is noise in a file alef rewrites in full every run — which is why
/// `core::keep_marker::strip_keep_markers` runs on every built-in template render. That pass
/// only covers `.jinja` renders, so a comment emitted from a *Rust string literal* bypasses it
/// and the marker reaches the consumer's tree. Nothing asserted that end state, so the leak was
/// only visible as a snapshot mismatch: a `~keep` written into an emitted comment survived into
/// the Swift crate while the identical text in a template was stripped, and the two backends
/// silently disagreed. The snapshots are the rendered artifact, so they are where the question
/// is actually decidable.
/// Create-only seed files are the documented exception: `core::keep_marker` deliberately leaves
/// their markers intact because nothing ever regenerates over them, so the marker is the only
/// thing keeping the rationale alive in the consumer's tree. Exempting them by name keeps the
/// exemption auditable — a blanket skip would also hide a real leak in a regenerated file.
const KEEP_MARKER_EXEMPT_SNAPSHOTS: &[&str] = &["ScaffoldTest.kt"];

#[test]
fn no_generated_snapshot_leaks_the_internal_keep_marker() {
    const KEEP_MARKER: &str = "~keep";

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = collect_files(&repo_root.join("tests").join("snapshots"), &[".snap"]);

    assert!(
        files.len() >= MIN_SNAPSHOT_FILES_SCANNED,
        "the scan examined only {} snapshot files, far below the expected floor of \
         {MIN_SNAPSHOT_FILES_SCANNED} — the walker is broken and this test would pass vacuously",
        files.len()
    );

    let mut leaks = Vec::new();
    let mut exemptions_used = vec![false; KEEP_MARKER_EXEMPT_SNAPSHOTS.len()];
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if let Some(index) = KEEP_MARKER_EXEMPT_SNAPSHOTS.iter().position(|e| name.contains(e)) {
            exemptions_used[index] = true;
            continue;
        }
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let content = String::from_utf8_lossy(&bytes);
        for (index, line) in content.lines().enumerate() {
            if line.contains(KEEP_MARKER) {
                let relative = path.strip_prefix(&repo_root).unwrap_or(path).display();
                leaks.push(format!("{relative}:{}", index + 1));
            }
        }
    }

    for (index, used) in exemptions_used.iter().enumerate() {
        assert!(
            *used,
            "`{}` is exempted from the keep-marker scan but matched no snapshot — a stale \
             exemption silently widens what this test permits, so remove it",
            KEEP_MARKER_EXEMPT_SNAPSHOTS[index]
        );
    }

    assert!(
        leaks.is_empty(),
        "generated output must not carry the internal `{KEEP_MARKER}` marker. A comment emitted \
         from a Rust string literal bypasses `strip_keep_markers` (which only runs on template \
         renders), so drop the marker from the emitted text — it protects nothing there, because \
         a `//` inside a Rust string literal is not a comment `poly` can see. Leaking sites:\n  {}",
        leaks.join("\n  ")
    );
}

/// The scan must actually reject the shapes it exists to reject. Without this, a typo in
/// `BANNED_CONSTRUCTOR` or a broken window calculation would make both arms above pass on any
/// input at all — the exact "green that examined nothing" this whole test guards against.
#[test]
fn the_scan_rejects_the_shapes_it_exists_to_reject() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Deliberately outside the workspace: the build target dir is shared with concurrent cargo
    // invocations, and this fixture is scratch that must not collide with them. ~keep
    let fixture_dir = std::env::temp_dir().join(format!("alef_runtime_stack_invariant_fixture_{}", std::process::id()));
    let _ = fs::remove_dir_all(&fixture_dir);
    fs::create_dir_all(&fixture_dir).expect("failed to create fixture dir");

    fs::write(
        fixture_dir.join("banned.rs"),
        "fn make() { let rt = tokio::runtime::Runtime::new().unwrap(); }\n",
    )
    .expect("failed to write banned fixture");
    fs::write(
        fixture_dir.join("no_stack.rs"),
        "fn make() {\n    tokio::runtime::Builder::new_multi_thread()\n        .enable_all()\n        .build()\n}\n",
    )
    .expect("failed to write no-stack fixture");
    fs::write(
        fixture_dir.join("ok.rs"),
        "fn make() {\n    const S: usize = 16 * 1024 * 1024;\n    tokio::runtime::Builder::new_multi_thread()\n        .enable_all()\n        .thread_stack_size(S)\n        .build()\n}\n",
    )
    .expect("failed to write ok fixture");

    let files = collect_files(&fixture_dir, &[".rs"]);
    assert_eq!(
        files.len(),
        3,
        "fixture walk must find all three files, found {files:?}"
    );

    let report = scan(&files, &repo_root);
    assert_eq!(
        report.banned_constructor.len(),
        1,
        "the scan must flag exactly the one `{BANNED_CONSTRUCTOR}` fixture, got {:?}",
        report.banned_constructor
    );
    assert_eq!(
        report.missing_stack_size.len(),
        1,
        "the scan must flag exactly the one stack-size-less builder fixture, got {:?}",
        report.missing_stack_size
    );
    assert_eq!(
        report.multi_thread_sites, 2,
        "both builder fixtures must be counted as multi-thread sites"
    );

    fs::remove_dir_all(&fixture_dir).expect("failed to clean up fixture dir");
}

/// An assertion line that quotes a banned constructor is test text, not emitted code, and must
/// not trip the scan — otherwise the negative regression assertions that pin this invariant in
/// the per-backend tests would themselves be reported as violations.
#[test]
fn assertion_lines_quoting_a_banned_constructor_are_not_violations() {
    assert!(is_assertion_line(
        "        !code.contains(\"tokio::runtime::Runtime::new()\"),"
    ));
    assert!(is_assertion_line(
        "        code.contains(\"tokio::runtime::Runtime::new()\"),"
    ));
    assert!(
        !is_assertion_line("    let rt = tokio::runtime::Runtime::new().unwrap();"),
        "real emitted-code lines must never be treated as assertions"
    );
}
