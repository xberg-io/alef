use super::{PYREFLY_UNAVAILABLE, PythonValidator};
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel};
use crate::snippets::validators::SnippetValidator;
use std::path::PathBuf;

const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

/// Whether `python3` runs, not merely resolves: a version-manager shim (e.g. pyenv, asdf, uv)
/// spawns fine then exits non-zero, so `PythonValidator::is_available`'s PATH-only check would
/// leave the skip below unreachable and fire the assert everywhere Python is absent. ~keep
fn python3_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("python3")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn python_snippet(code: &str) -> Snippet {
    Snippet {
        id: None,
        path: PathBuf::from("guide.md"),
        language: Language::Python,
        title: None,
        code: code.into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: PathBuf::from("guide.md"),
            line: 1,
            block_index: 0,
        },
    }
}

#[test]
fn batch_declines_run_so_each_snippet_executes_on_its_own() {
    let only = python_snippet("value = 1\n");

    let declined = PythonValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);

    assert!(declined.is_none());
}

#[test]
fn batch_returns_one_result_per_snippet_in_input_order() {
    if !python3_is_runnable() {
        return;
    }
    let first = python_snippet("first = 1\n");
    let second = python_snippet("second = 2\n");
    let third = python_snippet("third = 3\n");

    let results = PythonValidator::validate_batch_with_context(
        &[&first, &second, &third],
        ValidationLevel::Syntax,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(
        results,
        vec![
            (SnippetStatus::Pass, None),
            (SnippetStatus::Pass, None),
            (SnippetStatus::Pass, None)
        ]
    );
}

#[test]
fn batch_syntax_fails_only_the_broken_snippet_and_passes_its_neighbours() {
    if !python3_is_runnable() {
        return;
    }
    let first = python_snippet("value = 1\n");
    let broken = python_snippet("def broken(:\n    pass\n");
    let third = python_snippet("value = 3\n");

    let results = PythonValidator::validate_batch_with_context(
        &[&first, &broken, &third],
        ValidationLevel::Syntax,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], (SnippetStatus::Pass, None));
    assert_eq!(results[2], (SnippetStatus::Pass, None));
    assert_eq!(results[1].0, SnippetStatus::Fail);
    assert!(
        results[1]
            .1
            .as_deref()
            .is_some_and(|message| message.contains("SyntaxError")),
        "the failing snippet must carry its own diagnostic: {:?}",
        results[1].1
    );
}

#[test]
fn batch_compile_fails_only_the_broken_snippet() {
    if !python3_is_runnable() {
        return;
    }
    let first = python_snippet("value = 1\n");
    let broken = python_snippet("return 1\n");
    let third = python_snippet("value = 3\n");

    let results = PythonValidator::validate_batch_with_context(
        &[&first, &broken, &third],
        ValidationLevel::Compile,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results[0], (SnippetStatus::Pass, None));
    assert_eq!(results[1].0, SnippetStatus::Fail);
    assert_eq!(results[2], (SnippetStatus::Pass, None));
}

#[test]
fn batch_type_check_fails_only_the_snippet_pyrefly_names() {
    if which::which("pyrefly").is_err() {
        return;
    }
    let first = python_snippet("value: int = 1\nprint(value)\n");
    let broken = python_snippet("undefined_batch_name()\n");
    let third = python_snippet("other: int = 3\nprint(other)\n");

    let results = PythonValidator::validate_batch_with_context(
        &[&first, &broken, &third],
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
    assert_eq!(results[1].0, SnippetStatus::Fail);
    assert!(
        results[1]
            .1
            .as_deref()
            .is_some_and(|message| message.contains("undefined_batch_name")),
        "{:?}",
        results[1].1
    );
    assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
}

/// Regression for task #463: a published snippet with `level: typecheck` front matter and a
/// hard `IndentationError` (an empty `for` loop body) still passed, because `TypeCheck`
/// validated only through `pyrefly`, whose own parser does not have to reject exactly what
/// CPython's does. Simulates a `pyrefly` batch that reported nothing wrong at all (the shape a
/// lenient/recovering parser produces) alongside a real `py_compile` failure, and asserts the
/// compile precheck wins. ~keep
#[test]
fn compile_precheck_overrides_a_typecheck_pass_pyrefly_never_flagged() {
    let typecheck_results = vec![(SnippetStatus::Pass, None)];
    let compile_results = vec![(
        SnippetStatus::Fail,
        Some("IndentationError: expected an indented block after 'for' statement".to_string()),
    )];

    let merged = PythonValidator::apply_compile_precheck(typecheck_results, compile_results);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].0, SnippetStatus::Fail);
    assert!(
        merged[0]
            .1
            .as_deref()
            .is_some_and(|message| message.contains("IndentationError")),
        "{:?}",
        merged[0]
    );
}

/// Negative control: a genuine `pyrefly` finding on code that compiles cleanly must not be
/// discarded just because the compile precheck exists. ~keep
#[test]
fn compile_precheck_leaves_a_real_typecheck_failure_untouched_when_compile_passes() {
    let typecheck_results = vec![(SnippetStatus::Fail, Some("undefined_batch_name".to_string()))];
    let compile_results = vec![(SnippetStatus::Pass, None)];

    let merged = PythonValidator::apply_compile_precheck(typecheck_results, compile_results);

    assert_eq!(
        merged,
        vec![(SnippetStatus::Fail, Some("undefined_batch_name".to_string()))]
    );
}

/// Negative control: a clean compile precheck alongside a clean `pyrefly` pass must stay a
/// pass -- the override must never fire on a snippet the compile check did not itself fail.
#[test]
fn compile_precheck_leaves_a_clean_pass_untouched() {
    let typecheck_results = vec![(SnippetStatus::Pass, None)];
    let compile_results = vec![(SnippetStatus::Pass, None)];

    let merged = PythonValidator::apply_compile_precheck(typecheck_results, compile_results);

    assert_eq!(merged, vec![(SnippetStatus::Pass, None)]);
}

/// End-to-end confidence when the real toolchain is present: a snippet that does not parse at
/// all must fail `TypeCheck`, even though this validator's own early-return only guards
/// `pyrefly`'s absence, not a construct `pyrefly` might tolerate. ~keep
#[test]
fn batch_type_check_fails_a_snippet_that_does_not_compile() {
    if which::which("pyrefly").is_err() {
        return;
    }
    let broken = python_snippet("for value in range(3):\n");

    let results = PythonValidator::validate_batch_with_context(
        &[&broken],
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results.len(), 1);
    assert_ne!(results[0].0, SnippetStatus::Pass, "{:?}", results[0]);
}

#[test]
fn batch_type_check_reports_every_snippet_unavailable_when_pyrefly_is_missing() {
    if which::which("pyrefly").is_ok() {
        return;
    }
    let first = python_snippet("value = 1\n");
    let second = python_snippet("value = 2\n");

    let results =
        PythonValidator::validate_batch_with_context(&[&first, &second], ValidationLevel::TypeCheck, 10, None)
            .expect("batch validation runs");

    assert_eq!(
        results,
        vec![
            (SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string())),
            (SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string())),
        ]
    );
}

/// A checker that dies before reporting on a snippet must fail that snippet carrying the real
/// output, never leave it passing by default. ~keep
#[test]
fn unreported_snippets_fail_with_the_real_output_when_the_checker_breaks() {
    let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
    let output = concat!(
        r#"{"path": "/tmp/x/snippet_batch_0.py", "ok": true, "error": ""}"#,
        "\nTraceback (most recent call last)\n"
    );

    let results = PythonValidator::checker_results(&file_names, output);

    assert_eq!(results[0], (SnippetStatus::Pass, None));
    assert_eq!(
        results[1],
        (
            SnippetStatus::Fail,
            Some("Traceback (most recent call last)".to_string())
        )
    );
}

#[test]
fn pyrefly_blocks_attach_to_the_file_named_on_their_location_line() {
    let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
    let output = concat!(
        "ERROR Could not find name `missing` [unknown-name]\n",
        " --> /tmp/x/snippet_batch_1.py:1:1\n",
        "  |\n",
        "1 | missing()\n",
        " INFO 1 error\n"
    );

    let results = PythonValidator::typecheck_results(&file_names, false, output);

    assert_eq!(results[0], (SnippetStatus::Pass, None));
    assert_eq!(results[1].0, SnippetStatus::Fail);
    assert!(
        results[1]
            .1
            .as_deref()
            .is_some_and(|message| message.contains("snippet_batch_1.py:1:1")),
        "{:?}",
        results[1].1
    );
}

/// A Windows `pyrefly` location opens with a drive prefix (`C:\…`). Cutting the `:line:col` suffix
/// at the first colon left `C`, which owns no file, and an unattributed block on a failing run is
/// charged to every snippet — so the two passing snippets were reported as broken. Driven with a
/// synthetic Windows location so the parse is exercised on every host, not only on Windows. ~keep
#[test]
fn pyrefly_blocks_naming_a_windows_path_attach_to_the_file_they_name() {
    let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
    let output = concat!(
        "ERROR Could not find name `missing` [unknown-name]\n",
        " --> C:\\Users\\RUNNER~1\\AppData\\Local\\Temp\\x\\snippet_batch_1.py:1:1\n",
        "  |\n",
        "1 | missing()\n",
        " INFO 1 error\n"
    );

    let results = PythonValidator::typecheck_results(&file_names, false, output);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
    assert_eq!(results[1].0, SnippetStatus::Fail);
    assert!(
        results[1]
            .1
            .as_deref()
            .is_some_and(|message| message.contains("unknown-name")),
        "{:?}",
        results[1].1
    );
}

#[test]
fn a_type_checker_failure_naming_no_file_fails_every_snippet_with_the_real_output() {
    let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
    let output = "No `pyrefly.toml` found and the preset could not be resolved\n";

    let results = PythonValidator::typecheck_results(&file_names, false, output);

    assert_eq!(results.len(), 2);
    for result in &results {
        assert_eq!(result.0, SnippetStatus::Fail);
        assert_eq!(
            result.1.as_deref(),
            Some("No `pyrefly.toml` found and the preset could not be resolved")
        );
    }
}

#[test]
fn pyrefly_command_matches_scaffolded_python_tooling() {
    let command = PythonValidator::command(
        ValidationLevel::TypeCheck,
        std::path::Path::new("."),
        "python3",
        "snippet.py",
    )
    .expect("type-check command");
    assert_eq!(command.get_program(), "pyrefly");
    assert_eq!(command.get_args().collect::<Vec<_>>(), ["check", "snippet.py"]);
}

#[test]
fn unavailable_diagnostic_names_only_the_supported_checker() {
    assert_eq!(PYREFLY_UNAVAILABLE, "pyrefly is not available for Python type-checking");
    assert!(!PYREFLY_UNAVAILABLE.contains("mypy"));
}

#[test]
fn preserves_multiline_async_signature_lines() {
    let code = r"class UserServiceHandler:
    async def CreateUsers(
        self, request_iterator
    ) -> CreateUsersResponse:
        created_users = []
        return created_users
";

    let patched = PythonValidator::patch_code(code);
    assert!(patched.contains(") -> CreateUsersResponse:"));
    assert!(patched.contains("created_users = []"));
}

#[test]
fn syntax_validation_rejects_malformed_imports_and_indentation() {
    let path = PathBuf::from("broken.py");
    let snippet = Snippet {
        id: None,
        path: path.clone(),
        language: Language::Python,
        title: None,
        code: "from sample import call    from sample.types import Request\n  result = call()".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path,
            line: 1,
            block_index: 0,
        },
    };

    let (status, _) = PythonValidator
        .validate(&snippet, ValidationLevel::Syntax, 10)
        .expect("syntax validator runs");
    assert_eq!(status, SnippetStatus::Fail);
}

#[test]
fn run_session_resolves_local_binding_from_working_directory() {
    if !python3_is_runnable() {
        return;
    }
    let directory = tempfile::tempdir().expect("temp directory");
    std::fs::write(directory.path().join("local_binding.py"), "VALUE = 42\n").expect("local binding");
    let path = PathBuf::from("local.py");
    let snippet = Snippet {
        id: None,
        path: path.clone(),
        language: Language::Python,
        title: None,
        code: "import local_binding\nassert local_binding.VALUE == 42\n".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path,
            line: 1,
            block_index: 0,
        },
    };
    let session = ValidationSession {
        language: Language::Python,
        working_directory: directory.path().to_path_buf(),
        manifest: None,
        fingerprint: "test-binding".into(),
        env: std::collections::BTreeMap::new(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: std::collections::BTreeMap::new(),
    };

    let (status, message) = PythonValidator
        .validate_in_session(&snippet, ValidationLevel::Run, 10, Some(&session))
        .expect("session validation runs");

    assert_eq!(status, SnippetStatus::Pass, "{message:?}");
}

/// Regression: `validate_with_context` used to create its session-scoped scratch directory
/// directly inside `session.working_directory` via a bare `tempdir_in`, leaving a
/// `.alef-snippet-*/` directory loose in a tracked package source directory after every run.
/// It must nest under the session's own `.alef/snippets/tmp` cache root instead — and stay
/// gone whether the snippet passes or fails. ~keep
#[test]
fn session_scratch_resolves_under_the_cache_root_and_is_removed_on_pass_and_fail() {
    if !python3_is_runnable() {
        return;
    }
    let directory = tempfile::tempdir().expect("temp directory");
    let session = ValidationSession {
        language: Language::Python,
        working_directory: directory.path().to_path_buf(),
        manifest: None,
        fingerprint: "scratch-shape-fixture".into(),
        env: std::collections::BTreeMap::new(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: std::collections::BTreeMap::new(),
    };
    let passing = Snippet {
        id: None,
        path: "passing.py".into(),
        language: Language::Python,
        title: None,
        code: "value = 1\n".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "passing.py".into(),
            line: 1,
            block_index: 0,
        },
    };
    let mut failing = passing.clone();
    failing.code = "def broken(:\n".into();

    let (pass_status, pass_message) = PythonValidator
        .validate_in_session(&passing, ValidationLevel::Syntax, 10, Some(&session))
        .expect("passing snippet validates");
    assert_eq!(pass_status, SnippetStatus::Pass, "{pass_message:?}");
    let (fail_status, _) = PythonValidator
        .validate_in_session(&failing, ValidationLevel::Syntax, 10, Some(&session))
        .expect("failing snippet validates");
    assert_eq!(fail_status, SnippetStatus::Fail);

    let top_level_entries: Vec<_> = std::fs::read_dir(directory.path())
        .expect("read working directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name())
        .filter(|name| name != ".alef")
        .collect();
    assert!(
        top_level_entries.is_empty(),
        "no scratch entry may be left directly in working_directory: {top_level_entries:?}"
    );
    let scratch_root = directory.path().join(".alef/snippets/tmp");
    let remaining = std::fs::read_dir(&scratch_root)
        .map(|entries| entries.filter_map(|entry| entry.ok()).count())
        .unwrap_or(0);
    assert_eq!(
        remaining, 0,
        "scratch left behind under the cache root after a passing and a failing snippet validation"
    );
}
