use super::{render_mock_server_binary, render_mock_server_module};

#[test]
fn truncated_mock_response_reaches_native_and_node_body_readers() {
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let directory = tempfile::tempdir().expect("temporary generated server project");
    std::fs::create_dir(directory.path().join("src")).expect("create generated server source directory");
    std::fs::write(directory.path().join("Cargo.toml"), MANIFEST).expect("write server probe manifest");
    let source = format!(
        "{}\nmod per_test {{\n{}\n}}\n{}",
        render_mock_server_binary(),
        render_mock_server_module(),
        include_str!("testdata/truncation_probe.rs")
    );
    std::fs::write(directory.path().join("src/main.rs"), source).expect("write generated server probe");
    let output = std::process::Command::new("cargo")
        .args(["test", "--quiet", "--", "--nocapture"])
        .current_dir(directory.path())
        .output()
        .expect("run generated server wire and client probes");
    assert!(
        output.status.success(),
        "generated server probe failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const MANIFEST: &str = r#"[package]
name = "mock-server-truncation-probe"
version = "0.0.0"
edition = "2024"
[dependencies]
axum = "0.8"
brotli = "9"
flate2 = "1"
reqwest = { version = "0.13", default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
walkdir = "2"
"#;
