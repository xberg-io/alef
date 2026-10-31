#[path = "support/mise_task.rs"]
mod mise_task;

use std::path::{Path, PathBuf};
use std::process::Command;

fn canonical_tools() -> toml::Value {
    toml::from_str(include_str!("../mise.toml")).expect("committed Mise configuration")
}

#[test]
fn fixture_tool_declarations_preserve_tasks_and_only_copy_requested_committed_pins() {
    let tasks = "[tasks.probe]\nrun = 'literal fixture command'\n";
    let config = mise_task::fixture_config_with_tools(tasks, &["go", "zig"]);
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    let canonical = canonical_tools();
    assert_eq!(parsed["tools"].as_table().unwrap().len(), 2);
    for tool in ["go", "zig"] {
        assert_eq!(parsed["tools"][tool], canonical["tools"][tool]);
    }
    assert_eq!(
        parsed["tasks"]["probe"]["run"].as_str(),
        Some("literal fixture command")
    );
    let directory = tempfile::tempdir().unwrap();
    let command = mise_task::mise_run(directory.path(), "probe");
    let variable = |name: &str| command.get_envs().find(|(key, _)| *key == name).map(|(_, value)| value);
    assert_eq!(
        variable("MISE_TASK_RUN_AUTO_INSTALL"),
        Some(Some(std::ffi::OsStr::new("false")))
    );
    assert_eq!(variable("MISE_ENV"), Some(None));
    assert_eq!(
        variable("MISE_GLOBAL_CONFIG_FILE"),
        Some(Some(directory.path().join(".no-global-mise.toml").as_os_str()))
    );
}

fn canonical_executable(tool: &str) -> PathBuf {
    let canonical = canonical_tools();
    let pin = canonical["tools"][tool].as_str().expect("exact committed fixture pin");
    let output = Command::new(which::which("mise").expect("Mise executable"))
        .args(["which", tool, "--tool", &format!("{tool}@{pin}")])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("MISE_TASK_RUN_AUTO_INSTALL", "false")
        .env_remove("MISE_ENV")
        .output()
        .expect("resolve the already-installed committed tool");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Path::new(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .expect("installed canonical executable")
}

#[test]
fn isolated_go_and_zig_tasks_execute_the_exact_committed_mise_binaries() {
    let python = which::which("python3")
        .or_else(|_| which::which("python"))
        .expect("Python identity probe");
    for tool in ["go", "zig"] {
        let expected = canonical_executable(tool);
        let directory = tempfile::tempdir().unwrap();
        let tasks = "[tasks.probe]\nrun = '\"$FIXTURE_PYTHON\" identity.py'\n";
        std::fs::write(
            directory.path().join("mise.toml"),
            mise_task::fixture_config_with_tools(tasks, &[tool]),
        )
        .unwrap();
        std::fs::write(
            directory.path().join("identity.py"),
            concat!(
                "import json, os, shutil, subprocess\n",
                "tool = os.environ['FIXTURE_TOOL']\n",
                "executable = shutil.which(tool)\n",
                "assert executable is not None\n",
                "result = subprocess.run([executable, 'version'], check=True, capture_output=True, text=True)\n",
                "print(json.dumps({'executable': executable, 'version': result.stdout.strip()}))\n"
            ),
        )
        .unwrap();
        let output = mise_task::mise_run(directory.path(), "probe")
            .env("FIXTURE_PYTHON", &python)
            .env("FIXTURE_TOOL", tool)
            .output()
            .expect("run isolated tool identity probe");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout).expect("real tool identity");
        assert_eq!(
            Path::new(actual["executable"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            expected
        );
        assert_ne!(actual["version"].as_str().unwrap(), "");
    }
}
