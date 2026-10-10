//! Mechanical guard against checks that pass because they examined nothing.
//!
//! `tests/` holds ~253 integration test binaries, and `cargo test --lib` runs none of them. Every
//! agent in this repo verifies with `cargo test --lib` and reads the green as "the tests pass".
//! Two things follow from that, and this file closes both.
//!
//! 1. **CI must keep running the whole suite.** [`ci_workflow_runs_the_integration_test_suite`]
//!    pins the `test` job to a whole-suite invocation, so narrowing it to `--lib` fails here
//!    instead of silently retiring the integration binaries. Same shape as
//!    `generated_output_downstream_gate::ci_workflow_runs_the_generated_output_gate`, which pins
//!    that job's `--ignored` flag for the same reason.
//!
//! 2. **A test that runs must actually assert.** Four `#[test]` functions in this repo had empty
//!    bodies — `fn php_config_types_are_namespace_qualified() {}` — under module docs describing
//!    at length the regression they "covered". They were written as comment-only markers, and
//!    `poly`'s uncomment pass later stripped the comments, leaving naked `{}`. They passed for
//!    over a year. [`no_test_may_have_an_empty_body`] makes that unrepresentable.
//!
//! 3. **A disjunction must not have a dead arm.** `content.contains("u32") ||
//!    content.contains("u32_val")` is exactly `content.contains("u32")`: any text containing the
//!    longer needle contains the shorter one, so the shorter arm decides every case and the
//!    longer — the one naming the behaviour under test — can never fail.
//!    [`no_contains_disjunction_may_subsume_its_own_arm`] rejects that shape.
//!
//! Scope is `tests/**/*.rs` only. The same two defects are possible in `src`'s unit tests, but
//! the gap this file exists to close is the integration suite, and a gate that reports a hundred
//! pre-existing violations gets muted rather than fixed. Widening the scope is a follow-up with
//! its own remediation.
//!
//! Where it runs: nowhere new. An ordinary integration test, so CI's `test` job picks it up
//! through `cargo test --workspace` on all three platforms.

#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use std::path::{Path, PathBuf};
use std::process::Command;

use syn::visit::Visit;

#[path = "workflow_job_block/support.rs"]
mod workflow_job_block_support;
use workflow_job_block_support::workflow_job_block;

/// The workflow whose `test` job is the only thing that runs the integration binaries.
const CI_WORKFLOW: &str = ".github/workflows/ci.yml";

/// The job key in [`CI_WORKFLOW`] that runs the Rust test suite.
const TEST_JOB: &str = "test";

/// Invocations that cover every test target in the crate. `cargo test` with no target filter and
/// `--workspace` both build and run `tests/*.rs`; `--lib`, `--bins` and `--test <name>` do not.
const WHOLE_SUITE_INVOCATIONS: [&str; 3] = [
    "cargo test --workspace",
    "cargo test --all",
    "cargo nextest run --workspace",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Repo-relative, `/`-separated paths of every Rust source file under `tests/`.
///
/// `git ls-files` is authoritative because the rule governs committed content. Where git cannot
/// answer (no binary, no repository) this walks the tree instead and says so, rather than
/// reporting "nothing to check" and passing — a check that silently skips is the bug class this
/// file exists to close.
fn test_sources() -> Vec<String> {
    let listed = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z", "--", "tests/*.rs"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| {
            text.split('\0')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|paths: &Vec<String>| !paths.is_empty());

    let mut paths = match listed {
        Some(paths) => paths,
        None => {
            println!("test_vacuity_gate: git ls-files unavailable, falling back to a filesystem walk");
            walk_fallback()
        }
    };
    paths.sort();
    assert!(
        paths.len() > 100,
        "expected the integration suite to hold hundreds of files; found {} — the enumeration is \
         broken, not the tree",
        paths.len()
    );
    paths
}

fn walk_fallback() -> Vec<String> {
    fn visit(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                visit(&entry.path(), &relative, out);
            } else if relative.ends_with(".rs") {
                out.push(relative);
            }
        }
    }

    let mut out = Vec::new();
    visit(&repo_root().join("tests"), "tests", &mut out);
    out
}

fn parse(path: &str) -> syn::File {
    let absolute = repo_root().join(path);
    let source = std::fs::read_to_string(&absolute).unwrap_or_else(|error| panic!("read {path}: {error}"));
    syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {path}: {error}"))
}

fn is_test_fn(function: &syn::ItemFn) -> bool {
    function.attrs.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[derive(Default)]
struct EmptyBodies {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for EmptyBodies {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if is_test_fn(function) && function.block.stmts.is_empty() {
            self.names.push(function.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, function);
    }
}

/// A `#[test]` with no statements asserts nothing and can never fail. It is not coverage; it is a
/// green tick with nothing behind it.
#[test]
fn no_test_may_have_an_empty_body() {
    let mut offenders: Vec<String> = Vec::new();
    for path in test_sources() {
        let mut collector = EmptyBodies::default();
        collector.visit_file(&parse(&path));
        for name in collector.names {
            offenders.push(format!("  {path}: fn {name}()"));
        }
    }

    assert!(
        offenders.is_empty(),
        "{} `#[test]` function(s) have an empty body and can never fail:\n{}\n\n\
         Give the test a real assertion, or delete it. A placeholder that documents an intended \
         check reads as coverage in every report and provides none; if the behaviour genuinely \
         cannot be asserted from Rust, `#[ignore]` it with a reason so it is at least counted as \
         skipped.",
        offenders.len(),
        offenders.join("\n")
    );
}

/// One `x.contains("a") || x.contains("b")` whose needles subsume each other.
struct SubsumedDisjunction {
    receiver: String,
    dead: String,
    live: String,
}

#[derive(Default)]
struct SubsumedDisjunctions {
    found: Vec<SubsumedDisjunction>,
}

/// `x.contains("literal")` — the receiver rendered as source text, and the literal's value.
fn contains_call(expr: &syn::Expr) -> Option<(String, String)> {
    let syn::Expr::MethodCall(call) = expr else { return None };
    if call.method != "contains" || call.args.len() != 1 {
        return None;
    }
    let syn::Expr::Lit(literal) = call.args.first()? else {
        return None;
    };
    let syn::Lit::Str(text) = &literal.lit else { return None };
    let receiver = &call.receiver;
    Some((quote::quote!(#receiver).to_string(), text.value()))
}

/// `syn` keeps a macro body as an opaque token stream, and nearly every assertion in this repo
/// lives inside `assert!(...)`. Re-parsing the body as a comma-separated expression list puts the
/// condition back in reach of the visitor; bodies that are not expression lists (`matches!`, the
/// `json!` DSLs) simply fail to parse and are skipped.
fn macro_argument_expressions(mac: &syn::Macro) -> Vec<syn::Expr> {
    mac.parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
        .map(|args| args.into_iter().collect())
        .unwrap_or_default()
}

impl<'ast> Visit<'ast> for SubsumedDisjunctions {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        for expression in macro_argument_expressions(mac) {
            self.visit_expr(&expression);
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_expr_binary(&mut self, binary: &'ast syn::ExprBinary) {
        if matches!(binary.op, syn::BinOp::Or(_))
            && let Some((left_receiver, left)) = contains_call(&binary.left)
            && let Some((right_receiver, right)) = contains_call(&binary.right)
            && left_receiver == right_receiver
            && left != right
            && (left.contains(&right) || right.contains(&left))
        {
            // The longer needle implies the shorter, so the shorter arm alone decides the
            // disjunction and the longer one is unreachable. ~keep
            let (live, dead) = if left.len() < right.len() {
                (left, right)
            } else {
                (right, left)
            };
            self.found.push(SubsumedDisjunction {
                receiver: left_receiver,
                dead,
                live,
            });
        }
        syn::visit::visit_expr_binary(self, binary);
    }
}

/// `x.contains("u32") || x.contains("u32_val")` is `x.contains("u32")`. The arm that names the
/// behaviour under test cannot influence the outcome, so the assertion silently checks something
/// weaker than it reads as checking.
#[test]
fn no_contains_disjunction_may_subsume_its_own_arm() {
    let mut offenders: Vec<String> = Vec::new();
    for path in test_sources() {
        let mut collector = SubsumedDisjunctions::default();
        collector.visit_file(&parse(&path));
        for hit in collector.found {
            offenders.push(format!(
                "  {path}: `{}.contains({:?}) || …contains({:?})` — always decided by {:?}; {:?} is dead",
                hit.receiver, hit.live, hit.dead, hit.live, hit.dead
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "{} disjunction(s) have an arm that can never decide the result:\n{}\n\n\
         Drop the dead arm and keep the check honest, or — if both spellings are genuinely \
         acceptable output — pick two needles where neither contains the other.",
        offenders.len(),
        offenders.join("\n")
    );
}

/// `cargo test --lib` runs the unit tests in `src/` and none of the ~253 test binaries under
/// `tests/`. CI is the only place the integration suite runs on all three platforms, so narrowing
/// the `test` job's invocation retires that entire suite while every local and CI signal stays
/// green. This pins the invocation so the narrowing fails here first.
#[test]
fn ci_workflow_runs_the_integration_test_suite() {
    let workflow_path = repo_root().join(CI_WORKFLOW);
    let workflow = std::fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    let block = workflow_job_block(&workflow, TEST_JOB).unwrap_or_else(|| {
        panic!(
            "{CI_WORKFLOW} has no `{TEST_JOB}` job. Nothing else in this workflow runs the \
             ~253 integration test binaries under tests/."
        )
    });

    let taskfile = std::fs::read_to_string(repo_root().join("Taskfile.yml")).expect("read Taskfile");
    assert!(
        whole_suite_is_selected(&workflow, &taskfile),
        "{CI_WORKFLOW}'s `{TEST_JOB}` job must run the whole test suite — one of {WHOLE_SUITE_INVOCATIONS:?}. \
         A narrowed invocation such as `cargo test --lib` or `--bins` skips every test binary \
         under tests/ and still exits 0, so the integration suite would stop running with no \
         visible signal anywhere. Job block was:\n{block}"
    );
}

fn whole_suite_is_selected(workflow: &str, taskfile: &str) -> bool {
    let Ok(workflow) = serde_saphyr::from_str::<serde_json::Value>(workflow) else {
        return false;
    };
    let Ok(taskfile) = serde_saphyr::from_str::<serde_json::Value>(taskfile) else {
        return false;
    };
    let task = &taskfile["tasks"]["test:run"];
    let task_runs_suite = task["cmds"].as_array().is_some_and(|commands| {
        commands.len() == 1 && commands[0].as_str() == Some("cargo {{.TEST_CONFIG}} test --workspace {{.CLI_ARGS}}")
    }) && task["vars"]["TEST_CONFIG"].as_str()
        == Some("{{if eq OS \"darwin\"}}--config profile.test.package.alef.opt-level=1{{end}}");
    workflow["jobs"][TEST_JOB]["steps"].as_array().is_some_and(|steps| {
        steps.iter().filter_map(|step| step["run"].as_str()).any(|command| {
            if let Some(arguments) = command.strip_prefix("task test:run -- ") {
                task_runs_suite && unfiltered_arguments(arguments)
            } else {
                WHOLE_SUITE_INVOCATIONS
                    .iter()
                    .any(|invocation| command.strip_prefix(invocation).is_some_and(unfiltered_arguments))
            }
        })
    })
}

fn unfiltered_arguments(arguments: &str) -> bool {
    arguments
        .split_whitespace()
        .all(|argument| matches!(argument, "--no-fail-fast" | "--locked" | "--offline"))
}

#[test]
fn whole_suite_selection_rejects_narrowed_or_unresolved_task_commands() {
    let workflow = std::fs::read_to_string(repo_root().join(CI_WORKFLOW)).expect("read workflow");
    let taskfile = std::fs::read_to_string(repo_root().join("Taskfile.yml")).expect("read Taskfile");
    assert!(whole_suite_is_selected(&workflow, &taskfile));
    for (label, workflow, taskfile) in [
        (
            "narrowed task",
            workflow.clone(),
            taskfile.replace("test --workspace", "test --workspace --lib"),
        ),
        (
            "missing task",
            workflow.clone(),
            taskfile.replace("  test:run:", "  missing:run:"),
        ),
        (
            "narrowed CLI",
            workflow.replace("-- --no-fail-fast", "-- --lib"),
            taskfile.clone(),
        ),
        (
            "named filter",
            workflow.replace("-- --no-fail-fast", "-- one_test"),
            taskfile.clone(),
        ),
        (
            "narrowed config",
            workflow.clone(),
            taskfile.replace("--config profile.test", "--lib --config profile.test"),
        ),
        (
            "comment only",
            format!(
                "# cargo test --workspace\n{}",
                workflow.replace("task test:run -- --no-fail-fast", "cargo test --lib")
            ),
            taskfile.clone(),
        ),
    ] {
        assert!(!whole_suite_is_selected(&workflow, &taskfile), "accepted {label}");
    }
}
