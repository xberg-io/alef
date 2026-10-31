use alef::core::config::{Language, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use alef::scaffold::scaffold;
use std::path::Path;
use std::process::Command;

#[test]
fn emitted_node_loader_dispatches_to_the_matching_host_libc() {
    let api = ApiSurface {
        crate_name: "musl-probe".to_owned(),
        version: "0.1.0".to_owned(),
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: "musl-probe".to_owned(),
        languages: vec![Language::Node],
        ..ResolvedCrateConfig::default()
    };
    let files = scaffold(&api, &config, &[Language::Node]).expect("emit the actual Node scaffold");
    let indexes: Vec<_> = files
        .iter()
        .filter(|file| file.path == Path::new("crates/musl-probe-node/index.js"))
        .collect();
    assert_eq!(indexes.len(), 1, "exactly one loader must be exercised");
    let seed = &indexes[0].content;
    assert_ne!(seed.as_str(), "", "an empty seed must never pass the VM controls");
    assert!(seed.contains("const isMusl ="));
    assert!(seed.contains("linux-arm64-musl"));
    assert!(seed.contains("linux-arm64-gnu"));
    let directory = tempfile::tempdir().expect("create emitted-seed fixture directory");
    let path = directory.path().join("index.cjs");
    std::fs::write(&path, seed).expect("write the actual emitted loader");
    let checker = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/node_musl_dispatch.cjs");
    let output = Command::new("node")
        .arg(checker)
        .arg(path)
        .output()
        .expect("Node from the managed toolchain is required; do not silently skip this regression");
    assert!(
        output.status.success(),
        "actual emitted loader failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse nonempty dispatch census");
    assert_eq!(report["total"], 16);
    assert_eq!(report["passed"], 16);
    assert_eq!(report["failures"], serde_json::json!([]));
}
