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
//! Three rules are enforced over every generator source file, every code template, and every
//! committed snapshot of generated output:
//!
//! 1. `Runtime::new()` is banned. It builds a multi-thread runtime with default worker stacks
//!    and offers no way to widen them.
//! 2. Every `Builder::new_multi_thread()` must set `.thread_stack_size(...)` nearby.
//! 3. Every `Builder::new_current_thread()` must sit on a *widened calling thread*.
//!
//! Rule 3 exists because rules 1 and 2 cannot reach a current-thread runtime at all. Such a
//! runtime spawns no worker threads: `block_on` drives the future on the calling thread's own
//! stack, and `thread_stack_size` sizes only the blocking pool (verified in tokio's
//! `runtime/blocking/pool.rs`, which reads `builder.thread_stack_size`). Adding
//! `.thread_stack_size(...)` to a current-thread builder is therefore *worse than useless* — it
//! reads as protection while governing nothing the deep future runs on. The only real fix is to
//! widen the thread that calls `block_on`, with
//! `std::thread::Builder::new().stack_size(N).spawn(...)`.
//!
//! Leaving this class out of the scan was how the exposure survived the first pass: it was
//! reasoned about correctly and then recorded only as prose, which nothing re-checks. Some
//! current-thread sites genuinely cannot be widened by this generator, because the host owns the
//! thread; those are enumerated by path in `HOST_OWNED_CALLING_THREADS` with a written reason,
//! and each entry must still match a real site, so a stale exemption fails rather than silently
//! widening what the scan permits.

use std::fs;
use std::path::{Path, PathBuf};

/// Constructor that cannot express a worker stack size; banned outright in emitted code.
const BANNED_CONSTRUCTOR: &str = "Runtime::new()";

/// Builder entry point that spawns worker threads, and therefore must be given a stack size.
const MULTI_THREAD_BUILDER: &str = "Builder::new_multi_thread()";

/// The setter that widens the worker stack.
const STACK_SIZE_SETTER: &str = ".thread_stack_size(";

/// Builder entry point that spawns *no* worker threads. `block_on` runs the future on whatever
/// thread called it, so this constructor must be paired with a widened calling thread.
const CURRENT_THREAD_BUILDER: &str = "Builder::new_current_thread()";

/// The only construct that widens a thread this generator itself creates. Note that
/// `THREAD_BUILDER` plus `THREAD_STACK_SIZE_SETTER` cannot be satisfied accidentally by rule 2's
/// `.thread_stack_size(`: that spelling has no `.` immediately before `stack_size`.
const THREAD_BUILDER: &str = "std::thread::Builder::new()";
const THREAD_STACK_SIZE_SETTER: &str = ".stack_size(";

/// How far after a `Builder::new_multi_thread()` the setter may appear. Emitted builder chains
/// are split across source lines by string-literal continuations, so the setter is never more
/// than a few lines away; a generous window still catches an omission.
const STACK_SIZE_LOOKAHEAD_LINES: usize = 8;

/// How far *above* a `Builder::new_current_thread()` the widening spawn may appear. The widening
/// wraps the runtime, so it is always a handful of lines earlier; a generous window still catches
/// a runtime built on a bare `std::thread::spawn` or on a host thread.
const CALLING_THREAD_LOOKBEHIND_LINES: usize = 12;

/// Emitted `Builder::new_current_thread()` sites whose calling thread this generator does not
/// create and must not take over. Each entry carries the reason, because "we cannot fix it" and
/// "we forgot to fix it" are indistinguishable from the outside, and only the first is legitimate.
///
/// Every entry is re-checked by `every_host_owned_exemption_still_matches_a_real_site`: an entry
/// that no longer matches anything is a stale widening of what this scan permits, and fails.
const HOST_OWNED_CALLING_THREADS: &[(&str, &str)] = &[
    (
        "src/backends/magnus/templates/function_async_body.rs.jinja",
        "runs inside `alef_magnus_run_without_gvl`, i.e. inside an `rb_thread_call_without_gvl` \
         callback. Ruby invokes that callback on the *same* OS thread that released the GVL, and \
         the trait-bridge contract requires exactly that: a task on any other thread calling \
         `rb_thread_call_with_gvl` aborts the process. Moving `block_on` onto a widened thread \
         would trade a possible stack overflow for a certain abort. The stack here is Ruby's \
         thread machine stack, which only the host can size (`RUBY_THREAD_MACHINE_STACK_SIZE`).",
    ),
    (
        "src/backends/magnus/templates/service_rs_async_entrypoint_call.rs.jinja",
        "same GVL contract as `function_async_body`: the runtime is built inside this file's own \
         `rb_thread_call_without_gvl` callback so that GVL re-acquisition from the runtime's \
         tasks stays valid. Host-owned thread, host-sized stack.",
    ),
    (
        "src/backends/dart/templates/rust_trait_method_await_plain_spawn_blocking.jinja",
        "unrendered. Registered in `template_env.rs` but named by no `render()` call, so it emits \
         nothing; see `dart_spawn_blocking_trait_method_templates_are_unreachable`. Its runtime \
         would run on a `spawn_blocking` pool thread, which is sized by the *ambient* runtime's \
         `thread_stack_size` and so is not this call site's to widen. Delete rather than revive.",
    ),
    (
        "src/backends/dart/templates/rust_trait_method_await_result_spawn_blocking.jinja",
        "unrendered; see the `await_plain` entry above.",
    ),
    (
        "src/backends/dart/templates/rust_trait_method_default_await_spawn_blocking.jinja",
        "unrendered; see the `await_plain` entry above.",
    ),
    (
        "src/backends/dart/templates/rust_trait_method_ok_await_spawn_blocking.jinja",
        "unrendered; see the `await_plain` entry above.",
    ),
];

/// The dart trait-method templates that build a runtime on a `spawn_blocking` pool thread. They
/// are exempted above only because nothing renders them; this list is what
/// `dart_spawn_blocking_trait_method_templates_are_unreachable` proves that claim over.
const UNREACHABLE_DART_TEMPLATES: &[&str] = &[
    "rust_trait_method_await_plain_spawn_blocking.jinja",
    "rust_trait_method_await_result_spawn_blocking.jinja",
    "rust_trait_method_default_await_spawn_blocking.jinja",
    "rust_trait_method_ok_await_spawn_blocking.jinja",
];

/// The one file allowed to name an unreachable template: the registry that holds its bytes.
const TEMPLATE_REGISTRY: &str = "src/backends/dart/template_env.rs";

/// Floors that prove the scan actually examined something. A walker that silently matched no
/// files would otherwise report a vacuous pass, which is the same failure mode this test exists
/// to catch. These are well under the real counts at the time of writing (4900+ source files,
/// 30+ multi-thread sites, 130+ snapshots, 20+ multi-thread sites in snapshots).
const MIN_SOURCE_FILES_SCANNED: usize = 1_000;
const MIN_SOURCE_MULTI_THREAD_SITES: usize = 20;
const MIN_SNAPSHOT_FILES_SCANNED: usize = 50;
const MIN_SNAPSHOT_MULTI_THREAD_SITES: usize = 10;

/// Below the live current-thread site count (8 at the time of writing: two in the dart trait
/// bridge, two in the magnus GVL callbacks, four in unrendered dart templates). Deliberately low
/// enough to survive deleting the unrendered templates, and high enough that a scan which stopped
/// recognizing the constructor fails instead of passing on an empty match set.
const MIN_SOURCE_CURRENT_THREAD_SITES: usize = 3;

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
    current_thread_sites: usize,
    banned_constructor: Vec<String>,
    missing_stack_size: Vec<String>,
    /// `(relative path, "relative path:line")` for every current-thread runtime whose calling
    /// thread this generator does not widen. The path is kept separately so the exemption table
    /// can be matched against it without re-parsing the line suffix off the location string.
    unwidened_calling_thread: Vec<(String, String)>,
}

fn scan(files: &[PathBuf], repo_root: &Path) -> ScanReport {
    let mut report = ScanReport {
        files_scanned: files.len(),
        multi_thread_sites: 0,
        current_thread_sites: 0,
        banned_constructor: Vec::new(),
        missing_stack_size: Vec::new(),
        unwidened_calling_thread: Vec::new(),
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
                    report.missing_stack_size.push(location.clone());
                }
            }

            if line.contains(CURRENT_THREAD_BUILDER) {
                report.current_thread_sites += 1;
                let start = index.saturating_sub(CALLING_THREAD_LOOKBEHIND_LINES);
                // Comments are dropped from the window on purpose: the rationale comment that
                // belongs beside a widening spawn names the very constructs being searched for,
                // so counting it would let prose about widening pass for widening. ~keep
                let window = lines[start..index]
                    .iter()
                    .filter(|candidate| !is_comment_line(candidate))
                    .copied()
                    .collect::<Vec<&str>>()
                    .join("\n");
                let widened = window.contains(THREAD_BUILDER) && window.contains(THREAD_STACK_SIZE_SETTER);
                if !widened {
                    report.unwidened_calling_thread.push((relative.to_string(), location));
                }
            }
        }
    }

    report
}

/// Split the unwidened current-thread sites into the ones a `HOST_OWNED_CALLING_THREADS` entry
/// covers and the ones nothing covers, and report which entries were used. Returned per-entry
/// usage is what makes a stale exemption detectable.
fn partition_unwidened(report: &ScanReport) -> (Vec<String>, Vec<bool>) {
    let mut unexplained = Vec::new();
    let mut entry_used = vec![false; HOST_OWNED_CALLING_THREADS.len()];

    for (relative, location) in &report.unwidened_calling_thread {
        match HOST_OWNED_CALLING_THREADS
            .iter()
            .position(|(path, _)| relative.contains(path))
        {
            Some(index) => entry_used[index] = true,
            None => unexplained.push(location.clone()),
        }
    }

    (unexplained, entry_used)
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

    let (unexplained, _) = partition_unwidened(report);
    assert!(
        unexplained.is_empty(),
        "{what}: `{CURRENT_THREAD_BUILDER}` spawns no workers, so `block_on` runs the future on \
         the *calling* thread and `{STACK_SIZE_SETTER}` governs nothing here — adding it would \
         look like protection while providing none. Wrap the runtime in \
         `{THREAD_BUILDER}{THREAD_STACK_SIZE_SETTER}<named const>).spawn(...)` so the thread that \
         actually drives the future is wide enough; a bare `std::thread::spawn` gets Rust's 2 MiB \
         default and overflows into a process-killing SIGBUS rather than a catchable panic. If \
         the host owns this thread and the generator genuinely cannot widen it, add the file to \
         `HOST_OWNED_CALLING_THREADS` with the reason. Offending sites:\n  {}",
        unexplained.join("\n  ")
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

    assert!(
        report.current_thread_sites >= MIN_SOURCE_CURRENT_THREAD_SITES,
        "the scan found only {} `{CURRENT_THREAD_BUILDER}` sites, below the expected floor of \
         {MIN_SOURCE_CURRENT_THREAD_SITES} — the current-thread rule would then be passing on an \
         empty match set, which is the same as not running at all",
        report.current_thread_sites
    );

    assert_clean(&report, "generator source and templates");
}

/// Every current-thread runtime this generator emits must run on a thread wide enough for the
/// future it drives — the calling thread, since a current-thread runtime has no workers.
///
/// This is a separate test from the worker-stack arm above because it is a different fix for a
/// different mechanism, and conflating them is how the class went uncovered: the worker-stack
/// sweep correctly concluded `thread_stack_size` was inert here and then simply exempted the
/// shape, leaving the dart trait bridge driving host callbacks on a bare 2 MiB
/// `std::thread::spawn`.
#[test]
fn no_generator_emits_a_current_thread_runtime_on_an_unwidened_calling_thread() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = collect_files(&repo_root.join("src"), &[".rs", ".jinja"]);
    let report = scan(&files, &repo_root);

    assert!(
        report.current_thread_sites >= MIN_SOURCE_CURRENT_THREAD_SITES,
        "the scan found only {} `{CURRENT_THREAD_BUILDER}` sites, below the expected floor of \
         {MIN_SOURCE_CURRENT_THREAD_SITES} — this test would pass vacuously",
        report.current_thread_sites
    );

    let (unexplained, _) = partition_unwidened(&report);
    assert!(
        unexplained.is_empty(),
        "these `{CURRENT_THREAD_BUILDER}` sites drive `block_on` on a thread nobody widened. \
         Wrap the runtime in `{THREAD_BUILDER}{THREAD_STACK_SIZE_SETTER}<named const>).spawn(...)`, \
         or add the file to `HOST_OWNED_CALLING_THREADS` with the reason the host owns the \
         thread:\n  {}",
        unexplained.join("\n  ")
    );
}

/// A `HOST_OWNED_CALLING_THREADS` entry that matches nothing has stopped documenting a real
/// constraint and started silently widening what the scan accepts. Every entry must still name a
/// live unwidened site.
#[test]
fn every_host_owned_exemption_still_matches_a_real_site() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = collect_files(&repo_root.join("src"), &[".rs", ".jinja"]);
    let report = scan(&files, &repo_root);

    let (_, entry_used) = partition_unwidened(&report);
    let stale: Vec<&str> = HOST_OWNED_CALLING_THREADS
        .iter()
        .zip(&entry_used)
        .filter(|(_, used)| !**used)
        .map(|((path, _), _)| *path)
        .collect();

    assert!(
        stale.is_empty(),
        "these `HOST_OWNED_CALLING_THREADS` entries matched no unwidened current-thread site. \
         Either the site was fixed or removed (drop the entry) or it moved (update the path) — a \
         stale entry exempts a path that may later hold a real defect:\n  {}",
        stale.join("\n  ")
    );
}

/// The four dart `*_spawn_blocking` trait-method templates are exempted above on the grounds that
/// nothing renders them. That claim is only worth anything if something re-checks it: the moment
/// an emitter names one, its runtime reaches a consumer on a `spawn_blocking` pool thread this
/// call site cannot widen, and the exemption becomes a hole. `render()` resolves templates by
/// string name, so naming one anywhere outside the registry is the whole signal.
#[test]
fn dart_spawn_blocking_trait_method_templates_are_unreachable() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = collect_files(&repo_root.join("src"), &[".rs", ".jinja"]);

    assert!(
        files.len() >= MIN_SOURCE_FILES_SCANNED,
        "the scan examined only {} source files, far below the expected floor of \
         {MIN_SOURCE_FILES_SCANNED} — this test would pass vacuously",
        files.len()
    );

    let mut references = Vec::new();
    let mut registry_seen = vec![false; UNREACHABLE_DART_TEMPLATES.len()];

    for path in &files {
        let relative = path.strip_prefix(&repo_root).unwrap_or(path).display().to_string();
        // The template file is literally named after itself; only other files can reference it.
        if UNREACHABLE_DART_TEMPLATES.iter().any(|name| relative.ends_with(name)) {
            continue;
        }
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let content = String::from_utf8_lossy(&bytes);
        for (index, line) in content.lines().enumerate() {
            for (slot, name) in UNREACHABLE_DART_TEMPLATES.iter().enumerate() {
                if !line.contains(name) {
                    continue;
                }
                if relative.ends_with(TEMPLATE_REGISTRY) {
                    registry_seen[slot] = true;
                } else {
                    references.push(format!("{relative}:{}", index + 1));
                }
            }
        }
    }

    for (slot, seen) in registry_seen.iter().enumerate() {
        assert!(
            *seen,
            "`{}` is listed in `UNREACHABLE_DART_TEMPLATES` but `{TEMPLATE_REGISTRY}` never names \
             it — the template was renamed or removed, so this test is checking nothing for it",
            UNREACHABLE_DART_TEMPLATES[slot]
        );
    }

    assert!(
        references.is_empty(),
        "a dart `*_spawn_blocking` trait-method template is now named outside \
         `{TEMPLATE_REGISTRY}`, so it can be rendered. Its `{CURRENT_THREAD_BUILDER}` runs on a \
         `spawn_blocking` pool thread whose stack comes from the *ambient* runtime's \
         `{STACK_SIZE_SETTER}` — nothing this call site controls. Either delete the template or \
         remove its `HOST_OWNED_CALLING_THREADS` exemption and give it a real widening. Referencing \
         sites:\n  {}",
        references.join("\n  ")
    );
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

/// The current-thread rule must actually reject the shapes it exists to reject, including the
/// deceptive one: a current-thread runtime carrying `.thread_stack_size(...)`. That setter sizes
/// only the blocking pool, so it leaves `block_on`'s own stack at the default while reading, to a
/// reviewer, exactly like the multi-thread fix. A scan that accepted it would launder the defect.
#[test]
fn the_scan_rejects_a_current_thread_runtime_whose_calling_thread_is_not_widened() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = std::env::temp_dir().join(format!("alef_current_thread_widening_fixture_{}", std::process::id()));
    let _ = fs::remove_dir_all(&fixture_dir);
    fs::create_dir_all(&fixture_dir).expect("failed to create fixture dir");

    fs::write(
        fixture_dir.join("bare_spawn.rs"),
        "fn make() {\n    std::thread::spawn(move || {\n        tokio::runtime::Builder::new_current_thread()\n            .build()\n            .unwrap()\n            .block_on(fut)\n    });\n}\n",
    )
    .expect("failed to write bare-spawn fixture");
    fs::write(
        fixture_dir.join("thread_stack_size_only.rs"),
        "fn make() {\n    const S: usize = 16 * 1024 * 1024;\n    std::thread::spawn(move || {\n        tokio::runtime::Builder::new_current_thread()\n            .thread_stack_size(S)\n            .build()\n            .unwrap()\n            .block_on(fut)\n    });\n}\n",
    )
    .expect("failed to write thread-stack-size-only fixture");
    fs::write(
        fixture_dir.join("widened.rs"),
        "fn make() {\n    const S: usize = 32 * 1024 * 1024;\n    std::thread::Builder::new()\n        .stack_size(S)\n        .spawn(move || {\n            tokio::runtime::Builder::new_current_thread()\n                .build()\n                .unwrap()\n                .block_on(fut)\n        })\n        .unwrap();\n}\n",
    )
    .expect("failed to write widened fixture");

    let files = collect_files(&fixture_dir, &[".rs"]);
    assert_eq!(
        files.len(),
        3,
        "fixture walk must find all three files, found {files:?}"
    );

    let report = scan(&files, &repo_root);
    assert_eq!(
        report.current_thread_sites, 3,
        "all three fixtures must be counted as current-thread sites"
    );
    assert_eq!(
        report.missing_stack_size.len(),
        0,
        "no fixture builds a multi-thread runtime, so the worker-stack rule must stay silent, got {:?}",
        report.missing_stack_size
    );

    let flagged: Vec<&str> = report
        .unwidened_calling_thread
        .iter()
        .map(|(relative, _)| relative.as_str())
        .collect();
    assert_eq!(
        flagged.len(),
        2,
        "exactly the bare-spawn and thread_stack_size-only fixtures must be flagged, got {flagged:?}"
    );
    assert!(
        flagged.iter().any(|p| p.ends_with("bare_spawn.rs")),
        "a current-thread runtime on a bare `std::thread::spawn` must be flagged, got {flagged:?}"
    );
    assert!(
        flagged.iter().any(|p| p.ends_with("thread_stack_size_only.rs")),
        "`{STACK_SIZE_SETTER}` on a current-thread runtime sizes only the blocking pool and must \
         not be accepted as widening the `block_on` stack, got {flagged:?}"
    );
    assert!(
        !flagged.iter().any(|p| p.ends_with("widened.rs")),
        "a runtime wrapped in `{THREAD_BUILDER}{THREAD_STACK_SIZE_SETTER}...)` is correctly \
         widened and must not be flagged, got {flagged:?}"
    );

    fs::remove_dir_all(&fixture_dir).expect("failed to clean up fixture dir");
}

/// The exemption table must match by path and only by path: an entry must cover its own file and
/// nothing else, or one host-owned site would silently license every other unwidened one.
#[test]
fn host_owned_exemptions_cover_only_their_own_paths() {
    let exempt_path = HOST_OWNED_CALLING_THREADS[0].0;
    let report = ScanReport {
        files_scanned: 2,
        multi_thread_sites: 0,
        current_thread_sites: 2,
        banned_constructor: Vec::new(),
        missing_stack_size: Vec::new(),
        unwidened_calling_thread: vec![
            (exempt_path.to_string(), format!("{exempt_path}:2")),
            (
                "src/backends/somewhere/else.rs".to_string(),
                "src/backends/somewhere/else.rs:7".to_string(),
            ),
        ],
    };

    let (unexplained, entry_used) = partition_unwidened(&report);
    assert_eq!(
        unexplained,
        vec!["src/backends/somewhere/else.rs:7".to_string()],
        "only the unlisted path may be reported as unexplained"
    );
    assert!(entry_used[0], "the matching exemption entry must be marked used");
    assert!(
        entry_used[1..].iter().all(|used| !used),
        "one site must not mark any other exemption entry as used"
    );
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
