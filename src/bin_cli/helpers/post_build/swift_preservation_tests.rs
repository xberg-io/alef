use super::run_required_post_builds;
use crate::backends::swift::gen_bindings::bridge_artifacts::materialize_from_build_output;
use crate::core::backend::CompilePolicy;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::test_support::{ClearedCargoTargetDirGuard, CwdGuard};
use std::path::{Path, PathBuf};

const CRATE_NAME: &str = "preservation-probe-swift";
const PRESERVED_HEADER: &str = "// __swift_bridge__$preserved\n";

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn fixture(root: &Path) -> ResolvedCrateConfig {
    write(root, "Cargo.lock", "");
    write(
        root,
        "packages/swift/Sources/RustBridgeC/RustBridgeC.h",
        PRESERVED_HEADER,
    );
    write(
        root,
        "packages/swift/Sources/RustBridge/SwiftBridgeCore.swift",
        "// preserved core\n",
    );
    write(
        root,
        "packages/swift/Sources/RustBridge/preservation-probe-swift.swift",
        "// preserved crate\n",
    );
    let mut config = ResolvedCrateConfig {
        name: "preservation-probe".into(),
        ..ResolvedCrateConfig::default()
    };
    config
        .output_paths
        .insert("swift".into(), PathBuf::from("packages/swift/Sources/Probe"));
    config.explicit_output.swift = Some(PathBuf::from("packages/swift/Sources/Probe"));
    config
}

fn bridge_contents(root: &Path) -> Vec<Vec<u8>> {
    [
        "Sources/RustBridgeC/RustBridgeC.h",
        "Sources/RustBridge/SwiftBridgeCore.swift",
        "Sources/RustBridge/preservation-probe-swift.swift",
    ]
    .into_iter()
    .map(|path| std::fs::read(root.join("packages/swift").join(path)).unwrap())
    .collect()
}

fn fresh_output(root: &Path) -> PathBuf {
    let out = root.join(format!("target/debug/build/{CRATE_NAME}-fixture/out"));
    write(&out, "SwiftBridgeCore.swift", "// fresh core\n");
    out
}

fn fixture_without_lock(root: &Path, header: &str, redirected: bool) -> (ResolvedCrateConfig, PathBuf) {
    let config = fixture(root);
    std::fs::remove_file(root.join("Cargo.lock")).unwrap();
    write(
        root,
        "Cargo.toml",
        "[package]\nname = \"no-lock-swift-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n",
    );
    write(root, "src/lib.rs", "pub fn probe() {}\n");
    write(root, "packages/swift/Sources/RustBridgeC/RustBridgeC.h", header);
    let target = root.join(if redirected { "configured-target" } else { "target" });
    if redirected {
        write(
            root,
            ".cargo/config.toml",
            &crate::test_support::cargo_target_dir_config(&target),
        );
    }
    assert!(!root.join("Cargo.lock").exists());
    (config, target)
}

#[test]
fn skipped_compilation_preserves_populated_bridge_without_build_output() {
    let directory = tempfile::tempdir().unwrap();
    let _target = ClearedCargoTargetDirGuard::clear();
    let _cwd = CwdGuard::enter(directory.path());
    let config = fixture(directory.path());
    let before = bridge_contents(directory.path());

    run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped)
        .expect("source-only generation must preserve committed bridge artifacts");

    assert_eq!(bridge_contents(directory.path()), before);
    assert!(!directory.path().join("target").exists());
}

#[test]
fn skipped_compilation_still_materializes_all_three_fresh_bridge_files() {
    let directory = tempfile::tempdir().unwrap();
    let _target = ClearedCargoTargetDirGuard::clear();
    let _cwd = CwdGuard::enter(directory.path());
    let config = fixture(directory.path());
    let out = fresh_output(directory.path());
    write(&out, "SwiftBridgeCore.h", "// fresh core header\n");
    write(&out, &format!("{CRATE_NAME}/{CRATE_NAME}.swift"), "// fresh crate\n");
    write(&out, &format!("{CRATE_NAME}/{CRATE_NAME}.h"), "// fresh crate header\n");

    run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped).unwrap();

    let actual = bridge_contents(directory.path());
    assert_eq!(actual.len(), 3);
    assert_eq!(
        actual[1],
        b"// swift-format-ignore-file\nimport RustBridgeC\n\n// fresh core\n"
    );
    assert_eq!(
        actual[2],
        b"// swift-format-ignore-file\nimport RustBridgeC\n\n// fresh crate\n"
    );
    let header = String::from_utf8(actual[0].clone()).unwrap();
    assert!(header.contains("// fresh core header\n"));
    assert!(header.contains("// fresh crate header\n"));
    assert!(!header.contains("preserved"));
}

#[test]
fn required_materialization_rejects_absent_build_output_and_preserves_existing_files() {
    let directory = tempfile::tempdir().unwrap();
    let _target = ClearedCargoTargetDirGuard::clear();
    let _cwd = CwdGuard::enter(directory.path());
    fixture(directory.path());
    let before = bridge_contents(directory.path());

    let error = materialize_from_build_output(CRATE_NAME, &directory.path().join("packages/swift"), directory.path())
        .expect_err("build-required materialization must still require actual output");

    assert!(error.to_string().contains("no usable swift-bridge build output"));
    assert_eq!(bridge_contents(directory.path()), before);
}

#[test]
fn skipped_compilation_rejects_incomplete_fresh_output_and_preserves_existing_files() {
    let directory = tempfile::tempdir().unwrap();
    let _target = ClearedCargoTargetDirGuard::clear();
    let _cwd = CwdGuard::enter(directory.path());
    let config = fixture(directory.path());
    fresh_output(directory.path());
    let before = bridge_contents(directory.path());

    let error = run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped)
        .expect_err("a partial fresh build must not be mistaken for an unbuilt consumer");

    assert!(error.to_string().contains("no usable swift-bridge build output"));
    assert_eq!(bridge_contents(directory.path()), before);
}

#[test]
fn skipped_compilation_bootstraps_only_a_placeholder_when_no_header_exists() {
    let directory = tempfile::tempdir().unwrap();
    let _target = ClearedCargoTargetDirGuard::clear();
    let _cwd = CwdGuard::enter(directory.path());
    let config = fixture(directory.path());
    std::fs::remove_dir_all(directory.path().join("packages/swift/Sources")).unwrap();

    run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped).unwrap();

    let header = std::fs::read_to_string(
        directory
            .path()
            .join("packages/swift/Sources/RustBridgeC/RustBridgeC.h"),
    )
    .unwrap();
    assert!(header.contains("Placeholder header"));
    assert!(!header.contains("__swift_bridge__$"));
    assert!(!directory.path().join("target").exists());
}

#[test]
fn skipped_compilation_rejects_matching_output_without_the_swift_marker() {
    for header in [PRESERVED_HEADER, "// Placeholder header\n"] {
        let directory = tempfile::tempdir().unwrap();
        let _target = ClearedCargoTargetDirGuard::clear();
        let _cwd = CwdGuard::enter(directory.path());
        let config = fixture(directory.path());
        write(
            directory.path(),
            "packages/swift/Sources/RustBridgeC/RustBridgeC.h",
            header,
        );
        let before = bridge_contents(directory.path());
        let out = format!("target/debug/build/{CRATE_NAME}-fixture/out");
        write(
            directory.path(),
            &format!("{out}/SwiftBridgeCore.h"),
            "// partial fresh header\n",
        );
        assert!(directory.path().join(&out).is_dir());
        assert!(!directory.path().join(&out).join("SwiftBridgeCore.swift").exists());

        let error = run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped)
            .expect_err("a matching output directory without its Swift marker is incomplete");

        assert!(error.to_string().contains("no usable swift-bridge build output"));
        assert_eq!(bridge_contents(directory.path()), before);
    }
}

#[test]
fn skipped_compilation_rejects_partial_output_without_a_lockfile() {
    for header in [PRESERVED_HEADER, "// Placeholder header\n"] {
        for redirected in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let _target = ClearedCargoTargetDirGuard::clear();
            let _cwd = CwdGuard::enter(directory.path());
            let (config, target) = fixture_without_lock(directory.path(), header, redirected);
            let before = bridge_contents(directory.path());
            let out = target.join(format!("debug/build/{CRATE_NAME}-fixture/out"));
            write(&out, "SwiftBridgeCore.h", "// partial fresh header\n");
            assert!(out.is_dir());
            assert!(!out.join("SwiftBridgeCore.swift").exists());

            let error = run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped)
                .expect_err("a missing lockfile must not hide existing partial output");

            assert!(error.to_string().contains("no usable swift-bridge build output"));
            assert_eq!(bridge_contents(directory.path()), before);
        }
    }
}

#[test]
fn skipped_compilation_materializes_complete_output_without_a_lockfile() {
    for redirected in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let _target = ClearedCargoTargetDirGuard::clear();
        let _cwd = CwdGuard::enter(directory.path());
        let (config, target) = fixture_without_lock(directory.path(), PRESERVED_HEADER, redirected);
        let out = target.join(format!("debug/build/{CRATE_NAME}-fixture/out"));
        write(&out, "SwiftBridgeCore.swift", "// fresh core\n");
        write(&out, "SwiftBridgeCore.h", "// fresh core header\n");
        write(&out, &format!("{CRATE_NAME}/{CRATE_NAME}.swift"), "// fresh crate\n");
        write(&out, &format!("{CRATE_NAME}/{CRATE_NAME}.h"), "// fresh crate header\n");

        run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped).unwrap();

        let actual = bridge_contents(directory.path());
        assert_eq!(actual.len(), 3);
        assert_eq!(
            actual[1],
            b"// swift-format-ignore-file\nimport RustBridgeC\n\n// fresh core\n"
        );
        assert_eq!(
            actual[2],
            b"// swift-format-ignore-file\nimport RustBridgeC\n\n// fresh crate\n"
        );
        let header = String::from_utf8(actual[0].clone()).unwrap();
        assert!(header.contains("// fresh core header\n"));
        assert!(header.contains("// fresh crate header\n"));
        assert!(!header.contains("preserved"));
    }
}

#[test]
fn skipped_compilation_preserves_bootstrap_behavior_without_a_lockfile_or_output() {
    for header in [PRESERVED_HEADER, "// Placeholder header\n"] {
        for redirected in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let _target = ClearedCargoTargetDirGuard::clear();
            let _cwd = CwdGuard::enter(directory.path());
            let (config, target) = fixture_without_lock(directory.path(), header, redirected);
            let before = bridge_contents(directory.path());

            run_required_post_builds(&[Language::Swift], &config, directory.path(), CompilePolicy::Skipped).unwrap();

            let actual = bridge_contents(directory.path());
            assert_eq!(actual[1..], before[1..]);
            if header == PRESERVED_HEADER {
                assert_eq!(actual, before);
            } else {
                let header = String::from_utf8(actual[0].clone()).unwrap();
                assert!(header.contains("Placeholder header"));
                assert!(!header.contains("__swift_bridge__$"));
            }
            assert!(!target.exists());
        }
    }
}
