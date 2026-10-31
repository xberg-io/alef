use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeDef, TypeRef};
use alef::scaffold::scaffold;
use std::path::Path;
use std::process::{Command, Output};

#[path = "support/mise_task.rs"]
mod mise_task;

fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "sample-core"
sources = ["crates/sample-core/src/lib.rs"]
[crates.node.capsule_types.Handle]
type = "Handle"
from_module = "capsule-probe"
property_name = "__parser"
[crates.node.capsule_types.Handle.type_tag]
lower = "0x1"
upper = "0x2"
"#,
    )
    .expect("fixture configuration");
    let api = ApiSurface {
        crate_name: "sample-core".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Handle".into(),
            rust_path: "sample_core::Handle".into(),
            is_opaque: true,
            is_return_type: true,
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "get_handle".into(),
            rust_path: "sample_core::get_handle".into(),
            return_type: TypeRef::Named("Handle".into()),
            error_type: Some("std::io::Error".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    (api, config.resolve().expect("resolve configuration").remove(0))
}

fn write(root: &Path, relative: &Path, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directories");
    std::fs::write(path, content).expect("fixture file");
}

fn prepare(root: &Path) {
    let (api, config) = fixture();
    for file in scaffold(&api, &config, &[Language::Node]).expect("Node scaffold") {
        if file.path.starts_with("crates/sample-core-node") {
            write(root, &file.path, &file.content);
        }
    }
    let files = NapiBackend.generate_bindings(&api, &config).expect("Node bindings");
    assert_eq!(files.len(), 1, "exactly one generated native source");
    let source = format!("{}\n{}", files[0].content, include_str!("napi_capsule_native/probe.rs"));
    write(root, &files[0].path, &source);
    write(
        root,
        Path::new("crates/sample-core/src/lib.rs"),
        include_str!("napi_capsule_native/core.rs"),
    );
    write(
        root,
        Path::new("crates/sample-core/Cargo.toml"),
        "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n",
    );
    write(
        root,
        Path::new("crates/sample-core-node/build.rs"),
        "fn main() { napi_build::setup(); }\n",
    );
    write(
        root,
        Path::new("crates/sample-core-node/mise.toml"),
        "[tasks.build]\nrun='cargo build --jobs 2'\n",
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires Cargo NAPI dependencies, Node and mise; run by the native capsule CI gate"]
fn generated_tagged_capsule_builds_and_loads_in_node() {
    let directory = tempfile::tempdir().expect("native fixture");
    prepare(directory.path());
    let binding = directory.path().join("crates/sample-core-node");
    let built = mise_task::mise_run(&binding, "build")
        .env_remove("CARGO_TARGET_DIR")
        .env("RUSTC_WRAPPER", "")
        .output()
        .expect("build native capsule");
    assert_success(&built);
    let library = format!(
        "target/debug/{}sample_core_node{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    std::fs::copy(binding.join(library), binding.join("index.node")).expect("loadable native addon");
    std::fs::write(binding.join("probe.cjs"), include_str!("napi_capsule_native/probe.cjs")).expect("Node probe");
    let node = Command::new("node")
        .arg("probe.cjs")
        .current_dir(&binding)
        .output()
        .expect("Node must be installed for the native capsule gate");
    assert_success(&node);
    assert_eq!(
        String::from_utf8_lossy(&node.stdout),
        "capsule native: 3 tag checks passed\n"
    );
}

#[test]
fn native_capsule_ci_gate_collects_the_runtime_test_and_installs_node() {
    let collected = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--list",
            "--ignored",
            "--exact",
            "generated_tagged_capsule_builds_and_loads_in_node",
        ])
        .output()
        .expect("list native capsule tests");
    assert_success(&collected);
    assert_eq!(
        String::from_utf8_lossy(&collected.stdout).replace("\r\n", "\n"),
        "generated_tagged_capsule_builds_and_loads_in_node: test\n\n1 test, 0 benchmarks\n"
    );
    let workflow = serde_saphyr::from_str(include_str!("../.github/workflows/ci.yml")).expect("CI workflow YAML");
    assert!(
        node_ci_requirement_is_enforced(&workflow),
        "required tools and the collected native gate must run on every OS"
    );
}

fn node_ci_step_runs(step: &serde_json::Value) -> bool {
    (step["continue-on-error"].is_null() || step["continue-on-error"] == false)
        && (step["if"].is_null() || step["if"] == true || matches!(step["if"].as_str(), Some("true" | "${{ true }}")))
}

fn node_ci_selected_tools<'a>(step: &'a serde_json::Value, os: &str) -> Option<&'a str> {
    let value = step["with"]["tools"]
        .as_str()
        .filter(|tools| !tools.trim().is_empty())
        .or_else(|| step["with"]["install-args"].as_str())?
        .trim();
    if !value.contains("${{") {
        return Some(value);
    }
    let expression = value.strip_prefix("${{")?.strip_suffix("}}")?.trim();
    let (condition, alternatives) = expression.split_once("&&")?;
    if condition.split_whitespace().collect::<Vec<_>>() != ["runner.os", "==", "'Windows'"] {
        return None;
    }
    let (windows, other) = alternatives.split_once("||")?;
    let selected = if os == "Windows" { windows } else { other };
    let literal = selected.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    (!literal.contains(['\'', '"', '&', '|', '$', '{', '}'])).then_some(literal)
}

fn node_ci_tools_cover_platforms(steps: &[serde_json::Value]) -> bool {
    ["Windows", "Linux", "macOS"].into_iter().all(|os| {
        let required = ["python", "uv", "node", "go", "github:wasm-bindgen/wasm-bindgen"];
        required
            .into_iter()
            .chain((os != "Windows").then_some("bats"))
            .all(|required| {
                steps.iter().any(|step| {
                    node_ci_step_runs(step)
                        && step["uses"]
                            .as_str()
                            .is_some_and(|action| action.starts_with("xberg-io/actions/setup-mise@"))
                        && step["with"]["install"] != false
                        && step["with"]["install"] != "false"
                        && node_ci_selected_tools(step, os).is_some_and(|tools| {
                            tools
                                .split_whitespace()
                                .any(|tool| tool.split(['=', '@']).next() == Some(required))
                        })
                })
            })
    })
}

fn node_ci_gate_command_matches(step: &serde_json::Value) -> bool {
    let expected = [
        "mise",
        "run",
        "test:run",
        "--",
        "--test",
        "napi_capsule_native",
        "--",
        "--ignored",
        "--exact",
        "generated_tagged_capsule_builds_and_loads_in_node",
        "--nocapture",
    ];
    step["run"]
        .as_str()
        .is_some_and(|command| command.trim().lines().count() == 1 && command.split_whitespace().eq(expected))
}

fn node_ci_platforms_are_executed(job: &serde_json::Value) -> bool {
    let Some(matrix) = job["strategy"]["matrix"].as_object() else {
        return false;
    };
    let Some(operating_systems) = job["strategy"]["matrix"]["os"].as_array() else {
        return false;
    };
    let expected = ["ubuntu-latest", "macos-26", "windows-latest"];
    matrix.len() == 1
        && operating_systems.len() == expected.len()
        && expected
            .into_iter()
            .all(|os| operating_systems.iter().filter(|value| **value == os).count() == 1)
        && job["runs-on"] == "${{ matrix.os }}"
}

fn node_ci_requirement_is_enforced(workflow: &serde_json::Value) -> bool {
    let job = &workflow["jobs"]["test"];
    let Some(steps) = job["steps"].as_array() else {
        return false;
    };
    let Some(run) = steps.iter().position(|step| step["name"] == "Run tests") else {
        return false;
    };
    let gates: Vec<_> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step["name"] == "Verify generated Node capsules load natively")
        .collect();
    node_ci_platforms_are_executed(job)
        && node_ci_step_runs(job)
        && node_ci_step_runs(&steps[run])
        && steps[run]["run"].as_str().is_some_and(|run| {
            run.trim().lines().count() == 1
                && run
                    .split_whitespace()
                    .eq(["mise", "run", "test:run", "--", "--no-fail-fast"])
        })
        && node_ci_tools_cover_platforms(&steps[..run])
        && gates.len() == 1
        && gates[0].0 > run
        && node_ci_step_runs(gates[0].1)
        && node_ci_gate_command_matches(gates[0].1)
}

fn node_ci_workflow() -> serde_json::Value {
    serde_saphyr::from_str(include_str!("../.github/workflows/ci.yml")).expect("CI workflow YAML")
}

#[test]
fn node_ci_guard_rejects_a_missing_tool_on_either_platform_branch() {
    for tool in ["python", "uv", "node", "go", "github:wasm-bindgen/wasm-bindgen", "bats"] {
        let mut mutant = node_ci_workflow();
        let steps = mutant["jobs"]["test"]["steps"].as_array_mut().unwrap();
        let step = steps
            .iter_mut()
            .find(|step| {
                node_ci_selected_tools(step, "Windows")
                    .into_iter()
                    .chain(node_ci_selected_tools(step, "Linux"))
                    .any(|tools| tools.split_whitespace().any(|value| value == tool))
            })
            .expect("the actual setup provisions this tool");
        let key = if step["with"]["tools"].is_string() {
            "tools"
        } else {
            "install-args"
        };
        let old = step["with"][key].as_str().unwrap();
        let removed = old.replacen(tool, "", 1);
        assert_ne!(removed, old);
        step["with"][key] = serde_json::json!(removed);
        assert!(!node_ci_requirement_is_enforced(&mutant), "accepted missing {tool}");
    }
    let mut equivalent = node_ci_workflow();
    let steps = equivalent["jobs"]["test"]["steps"].as_array_mut().unwrap();
    let setup = steps
        .iter_mut()
        .find(|step| step["with"]["install-args"].is_string())
        .unwrap();
    setup["with"]["install-args"] =
        serde_json::json!("${{ runner.os == 'Windows' && 'node python uv' || 'node bats python uv' }}");
    assert!(node_ci_requirement_is_enforced(&equivalent));
}

#[test]
fn node_ci_guard_rejects_displaced_and_disabled_setup_and_runtime_steps() {
    for mode in [
        "setup-other-job",
        "setup-after-tests",
        "setup-disabled",
        "setup-no-install",
        "gate-other-job",
        "gate-disabled",
        "gate-narrowed",
    ] {
        let mut mutant = node_ci_workflow();
        let steps = mutant["jobs"]["test"]["steps"].as_array_mut().unwrap();
        let index = if mode.starts_with("setup-") {
            steps
                .iter()
                .position(|step| step["with"]["install-args"].is_string())
                .unwrap()
        } else {
            steps
                .iter()
                .position(|step| step["name"] == "Verify generated Node capsules load natively")
                .unwrap()
        };
        if mode.ends_with("other-job") {
            let moved = steps.remove(index);
            mutant["jobs"]["unrelated"]["steps"] = serde_json::json!([moved]);
        } else if mode == "setup-after-tests" {
            let moved = steps.remove(index);
            steps.push(moved);
        } else if mode == "setup-no-install" {
            steps[index]["with"]["install"] = serde_json::json!(false);
        } else if mode == "gate-narrowed" {
            let command = steps[index]["run"].as_str().unwrap().replace("--ignored", "");
            steps[index]["run"] = serde_json::json!(command);
        } else {
            steps[index]["if"] = serde_json::json!(false);
        }
        assert!(!node_ci_requirement_is_enforced(&mutant), "accepted {mode}");
    }
}

#[test]
fn node_ci_guard_rejects_compound_selector_branches() {
    let workflow = node_ci_workflow();
    for expression in [
        "${{ runner.os == 'Windows' && 'python uv node unused' && 'not-a-tool' || 'python uv bats node' }}",
        "${{ runner.os == 'Windows' && 'python uv node' && 'uv' || 'python uv bats node' }}",
        "${{ runner.os == 'Windows' && 'python uv node' || 'python uv bats node' && 'uv' }}",
        "${{ runner.os == 'Windows' && 'python uv node' || 'python uv bats node' || 'uv' }}",
    ] {
        let mut mutant = workflow.clone();
        let setup = mutant["jobs"]["test"]["steps"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|step| step["with"]["install-args"].is_string())
            .unwrap();
        setup["with"]["install-args"] = serde_json::json!(expression);
        assert_ne!(mutant, workflow);
        assert!(
            !node_ci_requirement_is_enforced(&mutant),
            "accepted compound selector: {expression}"
        );
    }
}

#[test]
fn node_ci_guard_rejects_narrowed_full_suite() {
    let workflow = node_ci_workflow();
    let mut mutant = workflow.clone();
    let run = mutant["jobs"]["test"]["steps"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|step| step["name"] == "Run tests")
        .unwrap();
    run["run"] = serde_json::json!("mise run test:run -- --lib no_such_test");
    assert_ne!(mutant, workflow);
    assert!(!node_ci_requirement_is_enforced(&mutant));
}

#[test]
fn node_ci_guard_rejects_ignored_failures_and_conditional_execution() {
    let workflow: serde_json::Value = serde_saphyr::from_str(include_str!("../.github/workflows/ci.yml")).unwrap();
    for name in [
        "job",
        "Run tests",
        "Verify generated Node capsules load natively",
        "Install Python, uv and Bats",
        "Set up Go",
        "Install the wasm-bindgen CLI",
    ] {
        for (field, value) in [
            ("continue-on-error", serde_json::json!(true)),
            ("if", serde_json::json!(false)),
            ("if", serde_json::json!("${{ runner.os == 'Linux' }}")),
        ] {
            let mut mutant = workflow.clone();
            let job = &mut mutant["jobs"]["test"];
            let target = if name == "job" {
                job
            } else {
                job["steps"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|step| step["name"] == name)
                    .unwrap()
            };
            target[field] = value;
            assert_ne!(mutant, workflow);
            assert!(!node_ci_requirement_is_enforced(&mutant), "accepted {name}: {field}");
        }
    }
}

#[test]
fn node_ci_guard_rejects_retired_or_rebound_platform_coverage() {
    let workflow = node_ci_workflow();
    for mode in [
        "static-runner",
        "dynamic-runner",
        "exclude",
        "include",
        "duplicate",
        "unsupported-runner",
        "missing-os",
        "extra-axis",
    ] {
        let mut mutant = workflow.clone();
        let job = &mut mutant["jobs"]["test"];
        match mode {
            "static-runner" => job["runs-on"] = serde_json::json!("ubuntu-latest"),
            "dynamic-runner" => job["runs-on"] = serde_json::json!("${{ false && matrix.os || 'ubuntu-latest' }}"),
            "exclude" => job["strategy"]["matrix"]["exclude"] = serde_json::json!([{"os": "windows-latest"}]),
            "include" => job["strategy"]["matrix"]["include"] = serde_json::json!([{"os": "ubuntu-latest"}]),
            "extra-axis" => job["strategy"]["matrix"]["other"] = serde_json::json!(["extra"]),
            _ => {
                let systems = job["strategy"]["matrix"]["os"].as_array_mut().unwrap();
                let windows = systems.iter().position(|os| *os == "windows-latest").unwrap();
                if mode == "missing-os" {
                    systems.remove(windows);
                } else {
                    systems[windows] = serde_json::json!(if mode == "duplicate" {
                        "ubuntu-latest"
                    } else {
                        "windows-imaginary"
                    });
                }
            }
        }
        assert_ne!(mutant, workflow);
        assert!(
            !node_ci_requirement_is_enforced(&mutant),
            "accepted coverage change: {mode}"
        );
    }
}
