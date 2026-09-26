//! Compiles the generated mock-server source (both the standalone binary's runtime and the
//! in-process `MockServer`) into a real, throwaway cargo project alongside a driver of
//! `#[test]`/`#[tokio::test]` functions, and runs `cargo test` against it -- the same pattern
//! `truncation_tests.rs` already uses. The request counter and origin substitution are real
//! runtime logic (a `Mutex<HashMap<..>>`, byte-level substring replace, a DNS loopback probe),
//! not parameterized templates, so a string-`.contains()` assertion on the generated source
//! would prove only that some text is present -- it says nothing about whether the counter
//! actually counts once per request, keys on the right thing, or substitutes into headers as
//! well as bodies. Compiling and running the real code is the only assertion strong enough to
//! catch a broken counter or a substitution that never fires.
use super::{render_mock_server_binary, render_mock_server_module};

#[test]
fn request_counter_and_origin_substitution_work_in_the_generated_servers() {
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let directory = tempfile::tempdir().expect("temporary generated server project");
    std::fs::create_dir(directory.path().join("src")).expect("create generated server source directory");
    std::fs::write(directory.path().join("Cargo.toml"), MANIFEST).expect("write server probe manifest");
    let source = format!(
        "{}\nmod per_test {{\n{}\n}}\n{}",
        render_mock_server_binary(),
        render_mock_server_module(),
        include_str!("testdata/request_counter_and_origins_probe.rs")
    );
    std::fs::write(directory.path().join("src/main.rs"), source).expect("write generated server probe");
    let output = std::process::Command::new("cargo")
        .args(["test", "--quiet", "--", "--nocapture", "--test-threads=1"])
        .current_dir(directory.path())
        .output()
        .expect("run generated server request-counter and origins probes");
    assert!(
        output.status.success(),
        "generated server probe failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const MANIFEST: &str = r#"[package]
name = "mock-server-request-counter-origins-probe"
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
