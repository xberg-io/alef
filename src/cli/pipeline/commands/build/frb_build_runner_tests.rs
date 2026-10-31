#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]
use super::*;
use crate::core::backend::{BuildConfig, BuildDependency, PostBuildStep};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = "alef-freezed-child-failed-569";
const ORIGINAL: &str =
    "rust_root: .\nrust_input: crate\ndart_output: ../lib/src/sample_bridge_generated\nadd_mod_to_lib: false\n";
const FILTER: &str = "--build-filter=package:overridden_name/src/sample_bridge_generated/**.freezed.dart";

fn run_in_isolated_case(name: &str, body: impl FnOnce()) {
    let full_name = format!("{}::{name}", module_path!().split_once("::").unwrap().1);
    let marker = format!("alef-569-case-completed:{full_name}");
    if std::env::var("ALEF_569_ISOLATED_CASE").ok().as_deref() == Some(full_name.as_str()) {
        body();
        eprintln!("{marker}");
        return;
    }
    let logs = tempfile::tempdir().expect("isolated case logs");
    let stdout_path = logs.path().join("stdout.log");
    let stderr_path = logs.path().join("stderr.log");
    let stdout = std::fs::File::create(&stdout_path).unwrap();
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let result = run_run_command_with_spawn(
        "alef-569-isolated-case",
        &[],
        logs.path(),
        "alef-569-isolation",
        std::time::Duration::from_secs(30),
        &|command| {
            let mut child = Command::new(std::env::current_exe()?);
            child.args(["--exact", &full_name, "--nocapture", "--test-threads=1"]);
            child.current_dir(logs.path());
            child.env_remove("ALEF_SKIP_COMMANDS");
            child.env("ALEF_569_ISOLATED_CASE", &full_name);
            child.stdout(stdout.try_clone()?).stderr(stderr.try_clone()?);
            *command = child;
            Ok(())
        },
    );
    let stdout = std::fs::read_to_string(stdout_path).unwrap();
    let stderr = std::fs::read_to_string(stderr_path).unwrap();
    assert!(
        matches!(result, Ok(true)),
        "isolated {full_name}: {result:?}\n{stdout}\n{stderr}"
    );
    let summary = stdout
        .lines()
        .rev()
        .find(|line| line.starts_with("test result:"))
        .expect("the selected test must produce a final harness summary");
    assert!(
        summary.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;"),
        "one actual selected case is required, not nested helper counts: {summary}\n{stdout}"
    );
    assert_eq!(
        stderr.lines().filter(|line| *line == marker).count(),
        1,
        "the selected case must complete its body exactly once: {stderr}"
    );
    eprintln!("isolated {full_name}: {summary}");
}

#[cfg(unix)]
#[test]
fn should_launch_isolated_cases_from_a_live_owned_directory_after_ambient_cwd_is_removed() {
    const NAME: &str = "should_launch_isolated_cases_from_a_live_owned_directory_after_ambient_cwd_is_removed";
    const FLAG: &str = "ALEF_FRB_REMOVED_CWD_CASE";
    let full_name = format!("{}::{NAME}", module_path!().split_once("::").unwrap().1);
    if std::env::var("ALEF_569_ISOLATED_CASE").ok().as_deref() == Some(full_name.as_str()) {
        run_in_isolated_case(NAME, || {
            let cwd = std::env::current_dir().expect("isolated case must have a live explicit cwd");
            assert!(cwd.is_dir(), "isolated cwd must remain owned until the case ends");
        });
        return;
    }
    if std::env::var(FLAG).ok().as_deref() == Some(full_name.as_str()) {
        let ambient = tempfile::tempdir().expect("disposable isolated-process cwd");
        std::env::set_current_dir(ambient.path()).unwrap();
        drop(ambient);
        assert_eq!(
            std::env::current_dir().unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "the regression must exercise a genuinely unlinked ambient cwd"
        );
        run_in_isolated_case(NAME, || panic!("body must run only in the nested isolated case"));
        eprintln!("alef-frb-removed-cwd-case-completed");
        return;
    }
    assert_removed_cwd_case_completes(&full_name, FLAG);
}

#[cfg(unix)]
fn assert_removed_cwd_case_completes(full_name: &str, flag: &str) {
    let root = tempfile::tempdir().expect("outer isolated case cwd");
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", full_name, "--nocapture", "--test-threads=1"])
        .current_dir(root.path())
        .env_remove("ALEF_569_ISOLATED_CASE")
        .env_remove("ALEF_SKIP_COMMANDS")
        .env(flag, full_name)
        .output()
        .expect("launch exact removed-cwd case without changing the parent cwd");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "removed-cwd child failed:\n{stdout}\n{stderr}");
    let summaries = stdout
        .lines()
        .filter(|line| line.starts_with("test result:"))
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 1, "exactly one outer selected case: {stdout}");
    assert!(
        summaries[0].starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;"),
        "one selected case must actually run: {stdout}"
    );
    assert_eq!(
        stderr
            .lines()
            .filter(|line| *line == "alef-frb-removed-cwd-case-completed")
            .count(),
        1,
        "the removed-cwd case must finish exactly once: {stderr}"
    );
}

struct Fixture {
    root: tempfile::TempDir,
    ledger: PathBuf,
    original: PathBuf,
    bridge: PathBuf,
    child_options: Value,
}

impl Fixture {
    fn new() -> Self {
        Self::with_root(tempfile::tempdir().expect("temporary project"))
    }

    fn with_root(root: tempfile::TempDir) -> Self {
        let dart = root.path().join("packages/dart");
        let rust = dart.join("rust");
        let output = dart.join("lib/src/sample_bridge_generated");
        std::fs::create_dir_all(&rust).unwrap();
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(dart.join("pubspec.yaml"), "name: overridden_name\n").unwrap();
        let original = rust.join("flutter_rust_bridge.yaml");
        std::fs::write(&original, ORIGINAL).unwrap();
        let bridge = output.join("lib.dart");
        let ledger = root.path().join("ledger.jsonl");
        Self {
            root,
            ledger,
            original,
            bridge,
            child_options: json!({}),
        }
    }

    fn run(&self, dart_status: i32) -> anyhow::Result<PostBuildOutcome> {
        let skipped = std::env::var("ALEF_SKIP_COMMANDS").unwrap_or_default();
        assert!(
            !skipped
                .split(',')
                .any(|name| (name.trim() == "flutter_rust_bridge_codegen"
                    && self.child_options["allow_frb_skip"] != true)
                    || (name.trim() == "dart" && self.child_options["allow_dart_skip"] != true)),
            "fixture precondition: parent ALEF_SKIP_COMMANDS explicitly skips a tested tool; not a behavioral RED"
        );
        let mut config = ResolvedCrateConfig {
            name: "sample".to_owned(),
            dart: Some(crate::core::config::DartConfig {
                pubspec_name: Some("overridden_name".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Some(seconds) = self.child_options["timeout_seconds"].as_u64() {
            config.build_commands.insert(
                "dart".to_owned(),
                crate::core::config::BuildCommandConfig {
                    precondition: None,
                    dependency_precondition: None,
                    dependency_remediation: None,
                    before: None,
                    build: None,
                    build_release: None,
                    timeout_seconds: Some(seconds),
                },
            );
        }
        assert!(super::build_command::output_path_for(Language::Dart, &config).is_none());
        let bc = BuildConfig {
            tool: "cargo",
            crate_suffix: "-dart",
            build_dep: BuildDependency::None,
            post_build: vec![
                PostBuildStep::RunCommand {
                    cmd: "flutter_rust_bridge_codegen",
                    args: vec![
                        "generate",
                        "--config-file",
                        "packages/dart/rust/flutter_rust_bridge.yaml",
                    ],
                },
                PostBuildStep::PatchFile {
                    path: "packages/dart/lib/src/sample_bridge_generated/lib.dart",
                    find: "// pending",
                    replace: "// postprocessed",
                },
            ],
        };
        let base = self.child_options["relative_base"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.path().to_path_buf());
        run_post_build_with_spawn(
            Language::Dart,
            &bc,
            &config,
            &base,
            StagingProfile::PreferOnDisk,
            Some(&|command| self.redirect_to_child(command, dart_status)),
        )
    }

    fn redirect_to_child(&self, command: &mut Command, dart_status: i32) -> anyhow::Result<()> {
        let original_program = command.get_program().to_string_lossy().into_owned();
        let args: Vec<String> = command.get_args().map(|v| v.to_string_lossy().into_owned()).collect();
        let cwd = command.get_current_dir().map(Path::to_path_buf).unwrap();
        let envs: Vec<_> = command
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(|v| v.to_owned())))
            .collect();
        let request = json!({"program": original_program, "args": args, "cwd": cwd});
        if original_program == "flutter_rust_bridge_codegen" && self.child_options["missing_frb"] == true {
            *command = Command::new("alef-required-frb-missing-569");
            return Ok(());
        }
        if original_program == "flutter_rust_bridge_codegen" && self.child_options["non_executable_frb"] == true {
            *command = Command::new(self.root.path());
            return Ok(());
        }
        if original_program == "dart" && self.child_options["missing_dart"] == true {
            *command = Command::new("alef-required-dart-missing-569");
            return Ok(());
        }
        let test_name = format!("{}::fake_tool_process", module_path!().split_once("::").unwrap().1);
        let mut replacement = Command::new(std::env::current_exe()?);
        replacement.args(["--ignored", "--exact", &test_name, "--nocapture"]);
        replacement.current_dir(cwd);
        for (key, value) in envs {
            if let Some(value) = value {
                replacement.env(key, value);
            } else {
                replacement.env_remove(key);
            }
        }
        replacement.env("ALEF_569_REQUEST", serde_json::to_string(&request)?);
        replacement.env("ALEF_569_LEDGER", &self.ledger);
        replacement.env("ALEF_569_ROOT", self.root.path());
        replacement.env("ALEF_569_DART_STATUS", dart_status.to_string());
        replacement.env("ALEF_569_OPTIONS", serde_json::to_string(&self.child_options)?);
        if original_program == "alef-explicit-frb-skip-569" {
            replacement.env("ALEF_SKIP_COMMANDS", "flutter_rust_bridge_codegen");
        }
        if original_program == "alef-explicit-dart-skip-569" {
            replacement.env("ALEF_SKIP_COMMANDS", "dart");
        }
        replacement
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        *command = replacement;
        Ok(())
    }

    fn run_neutral(&self, status: i32) -> anyhow::Result<bool> {
        let skipped = std::env::var("ALEF_SKIP_COMMANDS").unwrap_or_default();
        assert!(
            !skipped
                .split(',')
                .any(|name| name.trim() == "alef-neutral-uninstalled-569"),
            "parent skip policy excludes this fixture; not a neutral-seam result"
        );
        run_run_command_with_spawn(
            "alef-neutral-uninstalled-569",
            &["literal*argument"],
            self.root.path(),
            "alef-569-neutral",
            std::time::Duration::from_secs(20),
            &|command| self.redirect_to_child(command, status),
        )
    }

    fn rows(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.ledger)
            .unwrap()
            .lines()
            .map(|row| serde_json::from_str(row).unwrap())
            .collect()
    }
}

#[test]
fn should_execute_substituted_child_without_installed_tool() {
    run_in_isolated_case("should_execute_substituted_child_without_installed_tool", || {
        let f = Fixture::new();
        assert!(matches!(f.run_neutral(0), Ok(true)));
        let rows = f.rows();
        assert_eq!(rows.len(), 1, "the actual fixture child must run exactly once");
        assert_eq!(rows[0]["kind"], "neutral");
        assert_eq!(rows[0]["args"], json!(["literal*argument"]));
        assert_eq!(Path::new(rows[0]["cwd"].as_str().unwrap()), f.root.path());
    });
}

#[test]
fn should_preserve_substituted_child_failure_and_stderr() {
    run_in_isolated_case("should_preserve_substituted_child_failure_and_stderr", || {
        let f = Fixture::new();
        let error = format!("{:#}", f.run_neutral(23).expect_err("child exits 23"));
        assert!(error.contains(MARKER), "actual stderr must survive: {error}");
        assert!(
            error.contains("exited with status 23"),
            "actual status must survive: {error}"
        );
        assert_eq!(f.rows().len(), 1, "the failing fixture child must actually run");
    });
}

#[test]
fn should_keep_missing_tool_as_observable_skip_without_substitution() {
    run_in_isolated_case(
        "should_keep_missing_tool_as_observable_skip_without_substitution",
        || {
            let f = Fixture::new();
            let result = run_run_command(
                "alef-missing-uninstalled-569-no-substitution",
                &[],
                f.root.path(),
                "alef-569-neutral",
                std::time::Duration::from_secs(20),
            );
            assert!(matches!(result, Ok(false)), "actual missing-tool result: {result:?}");
            assert!(!f.ledger.exists(), "no substituted child should have run");
        },
    );
}

#[test]
fn should_reject_failed_freezed_child_before_postprocessing() {
    run_in_isolated_case("should_reject_failed_freezed_child_before_postprocessing", || {
        let f = Fixture::new();
        let original_permissions = std::fs::metadata(&f.original).unwrap().permissions();
        let result = f.run(23);
        let observed = f.rows();
        assert_eq!(observed.first().map(|r| &r["kind"]), Some(&json!("frb")));
        // Current FRB can swallow a nested builder error while returning zero. ~keep
        assert!(
            result.is_err(),
            "a failed required Freezed child must fail generation; actual {result:?}"
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains(MARKER), "actual child stderr must survive: {error}");
        assert!(
            error.contains("exited with status 23"),
            "actual child status must survive: {error}"
        );
        let rows = f.rows();
        assert_eq!(rows.len(), 2, "expected FRB and exactly one checked Dart invocation");
        assert_eq!(
            rows.iter().map(|r| r["kind"].as_str().unwrap()).collect::<Vec<_>>(),
            ["frb", "dart"]
        );
        assert_eq!(
            rows[1]["args"],
            json!(["run", "build_runner", "build", "--delete-conflicting-outputs", FILTER])
        );
        assert_eq!(rows[1]["skip"], "1");
        assert_eq!(
            Path::new(rows[1]["cwd"].as_str().unwrap()).canonicalize().unwrap(),
            f.root.path().join("packages/dart").canonicalize().unwrap()
        );
        assert!(std::fs::read_to_string(&f.bridge).unwrap().contains("// pending"));
        assert_eq!(std::fs::read(&f.original).unwrap(), ORIGINAL.as_bytes());
        assert_eq!(
            std::fs::metadata(&f.original).unwrap().permissions(),
            original_permissions
        );
        let runtime = Path::new(rows[0]["config"].as_str().unwrap());
        assert_eq!(
            runtime.parent().unwrap().canonicalize().unwrap(),
            f.original.parent().unwrap().canonicalize().unwrap()
        );
        assert!(!runtime.exists(), "runtime config must be removed after child failure");
    });
}

#[test]
fn should_run_checked_builder_once_then_postprocess() {
    run_in_isolated_case("should_run_checked_builder_once_then_postprocess", || {
        let f = Fixture::new();
        let original_permissions = std::fs::metadata(&f.original).unwrap().permissions();
        f.run(0).expect("successful checked child");
        let rows = f.rows();
        assert_eq!(rows.len(), 2, "exactly FRB and Dart, no hidden retry");
        assert_eq!(rows[0]["kind"], "frb");
        assert_eq!(rows[1]["kind"], "dart");
        assert_eq!(rows[0]["nested_disabled"], true);
        assert_eq!(
            std::fs::read_to_string(f.bridge.with_file_name("lib.freezed.dart")).unwrap(),
            "fresh-freezed-569\n"
        );
        assert!(std::fs::read_to_string(&f.bridge).unwrap().contains("// postprocessed"));
        assert_eq!(std::fs::read(&f.original).unwrap(), ORIGINAL.as_bytes());
        assert_eq!(
            std::fs::metadata(&f.original).unwrap().permissions(),
            original_permissions
        );
        assert!(!Path::new(rows[0]["config"].as_str().unwrap()).exists());
    });
}

#[test]
fn should_skip_builder_when_bridge_declares_no_generated_parts() {
    run_in_isolated_case("should_skip_builder_when_bridge_declares_no_generated_parts", || {
        let mut f = Fixture::new();
        f.child_options = json!({"body":"// pending\n"});
        f.run(0).expect("no builder required");
        let rows = f.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["kind"], "frb");
        assert!(std::fs::read_to_string(&f.bridge).unwrap().contains("// postprocessed"));
    });
}

#[test]
fn should_add_only_json_filter_for_actual_json_part() {
    run_in_isolated_case("should_add_only_json_filter_for_actual_json_part", || {
        let mut f = Fixture::new();
        f.child_options = json!({"body":"part 'lib.freezed.dart';\npart 'lib.g.dart';\n// pending\n"});
        f.run(0).expect("both generated parts");
        let rows = f.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1]["args"],
            json!([
                "run",
                "build_runner",
                "build",
                "--delete-conflicting-outputs",
                FILTER,
                "--build-filter=package:overridden_name/src/sample_bridge_generated/**.g.dart"
            ])
        );
        assert_eq!(
            std::fs::read_to_string(f.bridge.with_file_name("lib.g.dart")).unwrap(),
            "fresh-json-569\n"
        );
    });
}

#[test]
fn should_reject_json_only_bridge_without_claiming_completion() {
    run_in_isolated_case("should_reject_json_only_bridge_without_claiming_completion", || {
        let mut f = Fixture::new();
        f.child_options = json!({"body":"part 'lib.g.dart';\n// pending\n"});
        let error = format!("{:#}", f.run(0).expect_err("JSON-only is unsupported by this FRB path"));
        assert!(error.contains("JSON-only"), "actual diagnostic: {error}");
        assert_eq!(f.rows().len(), 1);
    });
}

#[test]
fn should_reject_missing_bridge_instead_of_an_empty_check() {
    run_in_isolated_case("should_reject_missing_bridge_instead_of_an_empty_check", || {
        let mut f = Fixture::new();
        f.child_options = json!({"missing_bridge":true});
        let error = format!("{:#}", f.run(0).expect_err("missing generated bridge"));
        assert!(error.contains("generated Dart bridge"), "actual diagnostic: {error}");
        assert_eq!(f.rows().len(), 1);
    });
}

#[test]
fn should_reject_successful_builder_missing_declared_part() {
    run_in_isolated_case("should_reject_successful_builder_missing_declared_part", || {
        let mut f = Fixture::new();
        f.child_options = json!({"missing_part":true});
        let error = format!("{:#}", f.run(0).expect_err("required part absent after child success"));
        assert!(
            error.contains("required generated Dart part"),
            "actual diagnostic: {error}"
        );
        assert_eq!(f.rows().len(), 2);
        assert!(std::fs::read_to_string(&f.bridge).unwrap().contains("// pending"));
    });
}

#[test]
fn should_not_run_builder_after_frb_failure() {
    run_in_isolated_case("should_not_run_builder_after_frb_failure", || {
        let mut f = Fixture::new();
        f.child_options = json!({"frb_failure":true});
        let error = format!("{:#}", f.run(0).expect_err("FRB child failed"));
        assert!(error.contains("exited with status 17"));
        assert!(error.contains("frb-own-failure-569"));
        let rows = f.rows();
        assert_eq!(rows.len(), 1);
        assert!(!Path::new(rows[0]["config"].as_str().unwrap()).exists());
    });
}

#[test]
fn should_fail_if_required_dart_executable_is_missing() {
    run_in_isolated_case("should_fail_if_required_dart_executable_is_missing", || {
        let mut f = Fixture::new();
        f.child_options = json!({"missing_dart":true});
        let error = format!("{:#}", f.run(0).expect_err("required Dart cannot be skipped"));
        assert!(
            error.contains("required Dart build_runner"),
            "actual diagnostic: {error}"
        );
        assert_eq!(f.rows().len(), 1);
    });
}

#[test]
fn should_share_original_deadline_with_checked_builder() {
    run_in_isolated_case("should_share_original_deadline_with_checked_builder", || {
        let mut f = Fixture::new();
        f.child_options = json!({"timeout_seconds":5,"frb_delay_ms":3000,"dart_delay_ms":3000});
        let error = format!(
            "{:#}",
            f.run(0).expect_err("combined work must stay within original deadline")
        );
        assert!(error.contains("timeout"), "actual deadline diagnostic: {error}");
        let rows = f.rows();
        assert_eq!(
            rows.len(),
            2,
            "FRB completed and Dart actually entered the remaining budget"
        );
        assert_eq!(rows[1]["kind"], "dart");
        assert!(std::fs::read_to_string(&f.bridge).unwrap().contains("// pending"));
        assert!(!Path::new(rows[0]["config"].as_str().unwrap()).exists());
    });
}

#[test]
fn should_reject_explicit_dart_skip_in_an_isolated_process() {
    run_in_isolated_case("should_reject_explicit_dart_skip_in_an_isolated_process", || {
        let f = Fixture::new();
        let result = run_run_command_with_spawn(
            "alef-explicit-dart-skip-569",
            &[],
            f.root.path(),
            "alef-569-skip",
            std::time::Duration::from_secs(20),
            &|command| f.redirect_to_child(command, 0),
        );
        assert!(matches!(result, Ok(true)), "isolated skip contract: {result:?}");
        assert_eq!(f.rows(), vec![json!({"kind":"explicit-dart-skip", "frb_rows":1})]);
    });
}

fn private_config_count(fixture: &Fixture) -> usize {
    std::fs::read_dir(fixture.original.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".alef-frb-"))
        .count()
}

#[test]
fn should_preserve_valid_config_missing_frb_as_an_observable_skip() {
    run_in_isolated_case("should_preserve_valid_config_missing_frb_as_an_observable_skip", || {
        let mut f = Fixture::new();
        f.child_options = json!({"missing_frb":true});
        let outcome = f.run(0).expect("missing FRB retains committed-output fallback");
        assert_eq!(outcome.skipped_missing_tools, ["flutter_rust_bridge_codegen"]);
        assert!(!f.ledger.exists(), "no actual fixture executable was spawned");
        assert_eq!(private_config_count(&f), 0);
        assert_eq!(std::fs::read(&f.original).unwrap(), ORIGINAL.as_bytes());
    });
}

#[test]
fn should_reject_non_executable_frb_instead_of_counting_a_missing_tool() {
    run_in_isolated_case(
        "should_reject_non_executable_frb_instead_of_counting_a_missing_tool",
        || {
            let mut f = Fixture::new();
            f.child_options = json!({"non_executable_frb":true});
            let error = format!("{:#}", f.run(0).expect_err("existing non-executable path is an error"));
            assert!(error.contains("failed to spawn"), "actual spawn diagnostic: {error}");
            assert_eq!(private_config_count(&f), 0);
        },
    );
}

#[test]
fn should_reject_invalid_frb_config_before_running_any_tool() {
    run_in_isolated_case("should_reject_invalid_frb_config_before_running_any_tool", || {
        let mut f = Fixture::new();
        f.child_options = json!({"missing_frb":true});
        std::fs::write(&f.original, "dart_output: [\n").unwrap();
        let error = format!(
            "{:#}",
            f.run(0).expect_err("malformed configuration is a preflight error")
        );
        assert!(
            error.contains("parsing FRB generation configuration"),
            "actual config diagnostic: {error}"
        );
        assert!(!f.ledger.exists());
        assert_eq!(private_config_count(&f), 0);
    });
}

#[test]
fn should_reject_missing_frb_config_before_running_any_tool() {
    run_in_isolated_case("should_reject_missing_frb_config_before_running_any_tool", || {
        let mut f = Fixture::new();
        f.child_options = json!({"missing_frb":true});
        std::fs::remove_file(&f.original).unwrap();
        let error = format!(
            "{:#}",
            f.run(0).expect_err("missing configuration is a preflight error")
        );
        assert!(
            error.contains("reading FRB generation configuration"),
            "actual config diagnostic: {error}"
        );
        assert!(!f.ledger.exists());
        assert_eq!(private_config_count(&f), 0);
    });
}

#[test]
fn should_preserve_explicit_frb_skip_with_invalid_config_in_an_isolated_process() {
    run_in_isolated_case(
        "should_preserve_explicit_frb_skip_with_invalid_config_in_an_isolated_process",
        || {
            let f = Fixture::new();
            let result = run_run_command_with_spawn(
                "alef-explicit-frb-skip-569",
                &[],
                f.root.path(),
                "alef-569-skip",
                std::time::Duration::from_secs(20),
                &|command| f.redirect_to_child(command, 0),
            );
            assert!(
                matches!(result, Ok(true)),
                "isolated explicit FRB skip contract: {result:?}"
            );
            assert_eq!(f.rows(), vec![json!({"kind":"explicit-frb-skip", "temp_count":0})]);
        },
    );
}

#[test]
fn should_resolve_private_config_once_with_a_relative_base_directory() {
    run_in_isolated_case(
        "should_resolve_private_config_once_with_a_relative_base_directory",
        || {
            let cwd = std::env::current_dir().unwrap();
            let root = tempfile::Builder::new()
                .prefix(".alef-frb-relative-")
                .tempdir_in(&cwd)
                .unwrap();
            let mut f = Fixture::with_root(root);
            f.child_options = json!({"relative_base": f.root.path().file_name().unwrap().to_str().unwrap()});
            f.run(0)
                .expect("relative base resolves runtime configuration exactly once");
            let rows = f.rows();
            assert_eq!(rows.len(), 2);
            let runtime = Path::new(rows[0]["config"].as_str().unwrap());
            assert!(runtime.is_absolute());
            assert_eq!(
                runtime.parent().unwrap(),
                f.original.parent().unwrap().canonicalize().unwrap()
            );
            assert!(!runtime.exists());
            assert_eq!(rows[1]["args"][4], FILTER);
            assert!(std::fs::read_to_string(&f.bridge).unwrap().contains("// postprocessed"));
        },
    );
}

#[test]
fn should_honor_explicit_relative_dart_root_over_a_nearer_package() {
    run_in_isolated_case("should_honor_explicit_relative_dart_root_over_a_nearer_package", || {
        let f = Fixture::new();
        std::fs::write(&f.original, format!("{ORIGINAL}dart_root: packages/dart\n")).unwrap();
        std::fs::write(
            f.bridge.parent().unwrap().join("pubspec.yaml"),
            "name: nearer_wrong_package\n",
        )
        .unwrap();
        f.run(0).expect("explicit relative root follows FRB process cwd");
        let rows = f.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["args"][4], FILTER);
        assert_eq!(rows[0]["config_value"]["dart_root"], "packages/dart");
        assert_eq!(
            Path::new(rows[1]["cwd"].as_str().unwrap()),
            f.root.path().join("packages/dart").canonicalize().unwrap()
        );
    });
}

#[test]
fn should_honor_explicit_absolute_dart_root_without_rewriting_it() {
    run_in_isolated_case("should_honor_explicit_absolute_dart_root_without_rewriting_it", || {
        let f = Fixture::new();
        let root = f.root.path().join("packages/dart").canonicalize().unwrap();
        let mut config: Value = serde_saphyr::from_str(ORIGINAL).unwrap();
        config["dart_root"] = json!(root);
        std::fs::write(&f.original, serde_json::to_vec(&config).unwrap()).unwrap();
        std::fs::write(
            f.bridge.parent().unwrap().join("pubspec.yaml"),
            "name: nearer_wrong_package\n",
        )
        .unwrap();
        f.run(0).expect("explicit absolute root wins over nested pubspec");
        let rows = f.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["args"][4], FILTER);
        assert_eq!(rows[0]["config_value"]["dart_root"], config["dart_root"]);
        assert_eq!(Path::new(rows[1]["cwd"].as_str().unwrap()), root);
    });
}

#[test]
fn should_reject_output_outside_explicit_root_before_running_a_builder() {
    run_in_isolated_case(
        "should_reject_output_outside_explicit_root_before_running_a_builder",
        || {
            let f = Fixture::new();
            let other = f.root.path().join("another-package");
            std::fs::create_dir_all(&other).unwrap();
            std::fs::write(other.join("pubspec.yaml"), "name: unrelated_package\n").unwrap();
            std::fs::write(&f.original, format!("{ORIGINAL}dart_root: another-package\n")).unwrap();
            let error = format!(
                "{:#}",
                f.run(0).expect_err("never generate into an unrelated Dart package")
            );
            assert!(error.contains("inside package lib"), "actual root diagnostic: {error}");
            assert_eq!(f.rows().len(), 1);
            assert_eq!(private_config_count(&f), 0);
        },
    );
}

#[test]
fn should_escape_literal_filter_path_as_in_published_frb_regression() {
    run_in_isolated_case(
        "should_escape_literal_filter_path_as_in_published_frb_regression",
        || {
            // Same input and expected literal encoding as FRB 2.13's build_runner_output_filters_metacharacter_output test. ~keep
            let encoded = super::frb_build_runner::escaped_filter_path(Path::new("generated[foo] #bar?")).unwrap();
            assert_eq!(encoded, "generated%5C%5Bfoo%5C%5D%20%23bar%5C%3F/");
            assert_eq!(
                format!("--build-filter=package:example/{encoded}**.freezed.dart"),
                "--build-filter=package:example/generated%5C%5Bfoo%5C%5D%20%23bar%5C%3F/**.freezed.dart"
            );
        },
    );
}

#[test]
fn should_preserve_other_original_configuration_values_in_the_runtime_copy() {
    run_in_isolated_case(
        "should_preserve_other_original_configuration_values_in_the_runtime_copy",
        || {
            let f = Fixture::new();
            let original: Value = json!({
                "rust_root":".", "rust_input":"crate", "dart_output":"../lib/src/sample_bridge_generated",
                "add_mod_to_lib":false, "rust_features":["feature_one","feature_two"],
                "dart_preamble":"a literal value", "stop_on_error":true, "build_runner":true
            });
            let bytes = serde_json::to_vec(&original).unwrap();
            std::fs::write(&f.original, &bytes).unwrap();
            f.run(0).expect("only nested build_runner is disabled in runtime copy");
            let mut expected = original;
            expected["build_runner"] = json!(false);
            let rows = f.rows();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0]["config_value"], expected);
            assert_eq!(std::fs::read(&f.original).unwrap(), bytes);
        },
    );
}

#[test]
fn should_preserve_non_generation_frb_commands_without_configuration() {
    run_in_isolated_case(
        "should_preserve_non_generation_frb_commands_without_configuration",
        || {
            for flag in ["--version", "--help"] {
                let f = Fixture::new();
                std::fs::remove_file(&f.original).unwrap();
                let config = BuildConfig {
                    tool: "cargo",
                    crate_suffix: "-dart",
                    build_dep: BuildDependency::None,
                    post_build: vec![PostBuildStep::RunCommand {
                        cmd: "flutter_rust_bridge_codegen",
                        args: vec![flag],
                    }],
                };
                let outcome = run_post_build_with_spawn(
                    Language::Dart,
                    &config,
                    &ResolvedCrateConfig::default(),
                    f.root.path(),
                    StagingProfile::PreferOnDisk,
                    Some(&|command| f.redirect_to_child(command, 0)),
                )
                .expect("non-generation FRB commands do not need a generation configuration");
                assert!(outcome.skipped_missing_tools.is_empty());
                assert_eq!(f.rows(), vec![json!({"kind":"frb-info", "args":[flag]})]);
                assert_eq!(private_config_count(&f), 0);
            }
        },
    );
}

#[test]
#[should_panic(expected = "one actual selected case is required")]
fn should_reject_a_zero_case_child_instead_of_claiming_success() {
    run_in_isolated_case("intentionally_nonexistent_zero_case_569", || {
        panic!("the nonexistent selected case must never execute its body");
    });
}

#[test]
#[ignore = "subprocess fixture; invoked explicitly with its request environment"]
fn fake_tool_process() {
    let request = std::env::var("ALEF_569_REQUEST").expect("subprocess fixture requires its request");
    let request: Value = serde_json::from_str(&request).unwrap();
    let program = request["program"].as_str().unwrap();
    let args: Vec<String> = serde_json::from_value(request["args"].clone()).unwrap();
    let root = PathBuf::from(std::env::var_os("ALEF_569_ROOT").unwrap());
    let ledger = PathBuf::from(std::env::var_os("ALEF_569_LEDGER").unwrap());
    let output = root.join("packages/dart/lib/src/sample_bridge_generated");
    let options: Value = serde_json::from_str(&std::env::var("ALEF_569_OPTIONS").unwrap()).unwrap();
    if program == "alef-explicit-frb-skip-569" {
        let mut fixture = Fixture::new();
        fixture.child_options = json!({"allow_frb_skip":true});
        std::fs::write(&fixture.original, "dart_output: [\n").unwrap();
        let outcome = fixture
            .run(0)
            .expect("explicit FRB skip precedes malformed configuration");
        assert_eq!(outcome.skipped_missing_tools, ["flutter_rust_bridge_codegen"]);
        assert!(!fixture.ledger.exists());
        assert_eq!(private_config_count(&fixture), 0);
        append(&ledger, json!({"kind":"explicit-frb-skip", "temp_count":0}));
    } else if program == "alef-explicit-dart-skip-569" {
        let mut fixture = Fixture::new();
        fixture.child_options = json!({"allow_dart_skip":true});
        let error = format!(
            "{:#}",
            fixture
                .run(0)
                .expect_err("explicit skip cannot complete required parts")
        );
        assert!(
            error.contains("required Dart build_runner"),
            "actual skip diagnostic: {error}"
        );
        assert_eq!(fixture.rows().len(), 1);
        append(&ledger, json!({"kind":"explicit-dart-skip", "frb_rows":1}));
    } else if program == "flutter_rust_bridge_codegen"
        && args.len() == 1
        && matches!(args[0].as_str(), "--version" | "--help")
    {
        append(&ledger, json!({"kind":"frb-info", "args":args}));
    } else if program == "flutter_rust_bridge_codegen" {
        let i = args.iter().position(|v| v == "--config-file").unwrap();
        let config = std::env::current_dir().unwrap().join(&args[i + 1]);
        let text = std::fs::read_to_string(&config).unwrap();
        let parsed: Value = serde_saphyr::from_str(&text).unwrap();
        let disabled = parsed.get("build_runner") == Some(&json!(false));
        append(
            &ledger,
            json!({"kind":"frb", "config":config, "nested_disabled":disabled, "config_value":parsed}),
        );
        if options["missing_bridge"] != true {
            let body = options["body"]
                .as_str()
                .unwrap_or("part 'lib.freezed.dart';\n// pending\n");
            std::fs::write(output.join("lib.dart"), body).unwrap();
        }
        if let Some(delay) = options["frb_delay_ms"].as_u64() {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        if options["frb_failure"] == true {
            eprintln!("frb-own-failure-569");
            std::process::exit(17);
        }
        if !disabled {
            // Emulates the documented FRB boundary, not Pub/network internals. ~keep
            append(&ledger, json!({"kind":"swallowed-nested-dart", "status":23}));
            eprintln!("Fail to execute_build_runner, but continue to run.");
        }
    } else if program == "dart" {
        append(
            &ledger,
            json!({"kind":"dart", "args":args, "cwd":request["cwd"], "skip":std::env::var("FRB_SIMPLE_BUILD_SKIP").unwrap_or_default()}),
        );
        let status: i32 = std::env::var("ALEF_569_DART_STATUS").unwrap().parse().unwrap();
        if status != 0 {
            eprintln!("{MARKER}");
            std::process::exit(status);
        }
        if let Some(delay) = options["dart_delay_ms"].as_u64() {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        if options["missing_part"] != true {
            std::fs::write(output.join("lib.freezed.dart"), "fresh-freezed-569\n").unwrap();
            if options["body"].as_str().is_some_and(|body| body.contains("lib.g.dart")) {
                std::fs::write(output.join("lib.g.dart"), "fresh-json-569\n").unwrap();
            }
        }
    } else if program == "alef-neutral-uninstalled-569" {
        append(&ledger, json!({"kind":"neutral", "args":args, "cwd":request["cwd"]}));
        let status: i32 = std::env::var("ALEF_569_DART_STATUS").unwrap().parse().unwrap();
        if status != 0 {
            eprintln!("{MARKER}");
            std::process::exit(status);
        }
    } else {
        panic!("unexpected own command boundary {program}");
    }
}

fn append(path: &Path, row: Value) {
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(out, "{row}").unwrap();
}
