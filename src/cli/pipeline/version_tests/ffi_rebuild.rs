use super::*;

const CASE_ENV: &str = "ALEF_VERSION_FFI_REBUILD_TEST_CASE";
const CHILD_TEST: &str = "cli::pipeline::version::tests::ffi_rebuild::isolated_finalization_child";

fn run_case(case: &str) {
    let fixture = tempfile::tempdir().expect("private version-sync fixture");
    std::fs::write(fixture.path().join("alef.toml"), "crates = []\n").expect("write configuration");
    std::fs::create_dir(fixture.path().join("ffi")).expect("create FFI directory");
    std::fs::write(fixture.path().join("ffi/Cargo.toml"), "invalid = [\n").expect("write invalid FFI manifest");
    std::fs::write(fixture.path().join("changed.txt"), "updated version\n").expect("write changed file");
    std::fs::create_dir(fixture.path().join(".alef")).expect("create marker directory");
    std::fs::write(
        fixture.path().join(".alef/last_synced_version"),
        if case == "unchanged" { "1.2.3" } else { "1.0.0" },
    )
    .expect("write prior marker");
    if case == "unchanged" {
        let marker = fixture
            .path()
            .join(ffi_build_marker(&fixture.path().join("ffi/Cargo.toml")).expect("marker path"));
        std::fs::create_dir_all(marker.parent().expect("marker directory")).expect("create build state directory");
        std::fs::write(marker, "1.2.3").expect("write previous successful FFI version");
    }
    let mut child = std::process::Command::new(std::env::current_exe().expect("test executable"));
    child
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .current_dir(fixture.path())
        .env(CASE_ENV, case);
    if case != "nonzero" {
        child.env("PATH", "");
    }
    let output = child.output().expect("run isolated finalization test");
    assert!(
        output.status.success(),
        "case {case} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn failed_cargo_build_does_not_mark_version_synced() {
    run_case("nonzero");
}

#[test]
fn cargo_spawn_failure_does_not_mark_version_synced() {
    run_case("spawn");
}

#[test]
fn unchanged_versions_do_not_require_cargo() {
    run_case("unchanged");
}

#[test]
fn non_ffi_versions_do_not_require_cargo() {
    run_case("no-ffi");
}

#[test]
fn isolated_finalization_child() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let config = ResolvedCrateConfig {
        name: "fixture".into(),
        languages: if case == "no-ffi" { vec![] } else { vec![Language::Ffi] },
        output_paths: [("ffi".into(), "ffi".into())].into(),
        ..Default::default()
    };
    let mut state = SyncState::new();
    if case != "unchanged" {
        state.updated.push("changed.txt".into());
    }
    let outcome = finalize_version_sync(
        &config,
        std::path::Path::new("alef.toml"),
        "1.2.3",
        true,
        true,
        &mut state,
    );
    let expected_marker = if matches!(case.as_str(), "nonzero" | "spawn") {
        let error = outcome.expect_err("a required FFI build failure must stop finalization");
        let expected_command = format!(
            "cargo build --manifest-path {}",
            std::path::Path::new("ffi").join("Cargo.toml").display()
        );
        assert!(error.to_string().contains(&expected_command), "{error:#}");
        if case == "spawn" {
            assert_eq!(
                error.downcast_ref::<std::io::Error>().expect("spawn I/O cause").kind(),
                std::io::ErrorKind::NotFound
            );
        }
        "1.0.0"
    } else {
        outcome.expect("unneeded FFI build must not require Cargo");
        "1.2.3"
    };
    assert_eq!(
        std::fs::read_to_string(".alef/last_synced_version").expect("read marker"),
        expected_marker
    );
}
