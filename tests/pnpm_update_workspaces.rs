use alef::core::config::update_defaults::default_update_config;
use alef::core::config::{LangContext, Language, ToolsConfig};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
#[ignore = "requires pnpm and Python; run explicitly through Mise"]
fn latest_default_plans_update_root_and_child_against_local_registry() {
    let pnpm = which::which("pnpm").expect("the runtime regression requires pnpm on PATH");
    let python = which::which("python3")
        .or_else(|_| which::which("python"))
        .expect("the runtime regression requires Python on PATH");
    let tools = ToolsConfig::default();
    let context = LangContext::default(&tools);
    let plans: Vec<_> = [Language::Node, Language::Wasm]
        .into_iter()
        .map(|language| {
            default_update_config(language, "unused", &context)
                .upgrade
                .expect("pnpm latest plan")
                .commands()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut child = Command::new(python)
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pnpm_workspace_update.py"
        ))
        .arg(pnpm)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the local registry fixture");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&plans).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{diagnostics}");
    assert!(diagnostics.contains("plans=2 manifests=4"), "{diagnostics}");
}
