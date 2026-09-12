use std::fs;
use std::path::Path;
use std::process::Command;

fn fixture(root: &Path, ffi: bool) {
    for directory in ["src", "aux/src"] {
        fs::create_dir_all(root.join(directory)).expect("create source directory");
        fs::write(root.join(directory).join("lib.rs"), "pub fn value() -> u8 { 1 }\n").expect("write source");
    }
    fs::write(root.join("Cargo.toml"), "[package]\nname = 'fixture'\nversion = '1.2.3'\nedition = '2024'\n[workspace]\nmembers = ['aux']\nexclude = ['packages/ffi']\n").expect("write root manifest");
    fs::write(
        root.join("aux/Cargo.toml"),
        "[package]\nname = 'aux'\nversion = '1.0.0'\nedition = '2024'\n",
    )
    .expect("write member manifest");
    let language = if ffi { "ffi" } else { "rust" };
    fs::write(root.join("alef.toml"), format!("[workspace]\nlanguages = ['{language}']\n[[crates]]\nname = 'fixture'\nsources = ['src/lib.rs']\nversion_from = 'Cargo.toml'\n[crates.output]\nffi = 'packages/ffi'\n")).expect("write config");
}

fn run(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_alef"))
        .args(["sync-versions", "--skip-swift-checksum"])
        .current_dir(root)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .expect("run CLI")
}

#[test]
fn failed_existing_ffi_build_stops_cli_before_sync_marker() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path();
    fixture(root, true);
    fs::create_dir_all(root.join("packages/ffi/src")).expect("FFI directory");
    fs::write(root.join("packages/ffi/Cargo.toml"), "[package]\nname = 'custom-bridge'\nversion = '1.2.3'\nedition = '2024'\n[dependencies]\nmissing = { path = '../../missing' }\n").expect("FFI manifest");
    fs::write(root.join("packages/ffi/src/lib.rs"), "").expect("FFI source");
    let output = run(root);
    assert!(!output.status.success(), "CLI ignored failed FFI build: {output:?}");
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expected_command = format!(
        "cargo build --manifest-path {}",
        Path::new("packages/ffi").join("Cargo.toml").display()
    );
    assert!(diagnostics.contains(&expected_command), "{diagnostics}");
    assert!(!root.join(".alef/last_synced_version").exists());
    let retry = run(root);
    assert!(
        !retry.status.success(),
        "retry ignored pending failed FFI build: {retry:?}"
    );
    assert!(!root.join(".alef/last_synced_version").exists());
    fs::write(
        root.join("packages/ffi/Cargo.toml"),
        "[package]\nname = 'custom-bridge'\nversion = '1.2.3'\nedition = '2024'\n",
    )
    .expect("repair FFI dependency");
    let recovered = run(root);
    assert!(recovered.status.success(), "{recovered:?}");
    assert!(root.join("target/debug/libcustom_bridge.rlib").is_file());
    assert_eq!(
        fs::read_to_string(root.join(".alef/last_synced_version")).expect("marker"),
        "1.2.3"
    );
}

#[test]
fn fresh_ffi_and_non_ffi_consumers_sync_without_generated_bridge() {
    for ffi in [true, false] {
        let temporary = tempfile::tempdir().expect("fixture");
        fixture(temporary.path(), ffi);
        let output = run(temporary.path());
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            fs::read_to_string(temporary.path().join(".alef/last_synced_version")).expect("marker"),
            "1.2.3"
        );
    }
}

#[test]
fn existing_non_workspace_ffi_manifest_builds_successfully() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path();
    fixture(root, true);
    fs::create_dir_all(root.join("packages/ffi/src")).expect("FFI directory");
    fs::write(
        root.join("packages/ffi/Cargo.toml"),
        "[package]\nname = 'custom-bridge'\nversion = '1.2.3'\nedition = '2024'\n",
    )
    .expect("FFI manifest");
    fs::write(root.join("packages/ffi/src/lib.rs"), "pub fn bridge() {}\n").expect("FFI source");
    let output = run(root);
    assert!(output.status.success(), "{output:?}");
    assert!(
        root.join("target/debug/libcustom_bridge.rlib").is_file(),
        "configured bridge was not built"
    );
    assert_eq!(
        fs::read_to_string(root.join(".alef/last_synced_version")).expect("marker"),
        "1.2.3"
    );
}

#[test]
fn preceding_crate_cannot_satisfy_another_crates_ffi_rebuild() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path();
    fixture(root, false);
    let config = root.join("alef.toml");
    let mut content = fs::read_to_string(&config).expect("config");
    content.push_str("\n[[crates]]\nname = 'aux'\nsources = ['aux/src/lib.rs']\nversion_from = 'aux/Cargo.toml'\nlanguages = ['ffi']\n[crates.output]\nffi = 'packages/ffi'\n");
    fs::write(config, content).expect("two-crate config");
    fs::create_dir_all(root.join("packages/ffi")).expect("FFI directory");
    fs::write(root.join("packages/ffi/Cargo.toml"), "invalid = [\n").expect("broken FFI manifest");
    for attempt in 1..=2 {
        let output = run(root);
        assert!(
            !output.status.success(),
            "attempt {attempt} skipped required FFI build: {output:?}"
        );
        let expected_command = format!(
            "cargo build --manifest-path {}",
            Path::new("packages/ffi").join("Cargo.toml").display()
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&expected_command),
            "{output:?}"
        );
    }
}

#[test]
fn failed_same_version_rebuild_invalidates_previous_success() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path();
    fixture(root, true);
    fs::create_dir_all(root.join("packages/ffi/src")).expect("FFI directory");
    fs::write(
        root.join("packages/ffi/Cargo.toml"),
        "[package]\nname = 'custom-bridge'\nversion = '1.2.3'\nedition = '2024'\n",
    )
    .expect("FFI manifest");
    fs::write(root.join("packages/ffi/src/lib.rs"), "pub fn bridge() {}\n").expect("FFI source");
    assert!(run(root).status.success());
    fs::write(
        root.join("aux/Cargo.toml"),
        "[package]\nname = 'aux'\nversion = '1.0.0'\nedition = '2024'\n",
    )
    .expect("stale manifest");
    fs::write(
        root.join("packages/ffi/src/lib.rs"),
        "compile_error!(\"broken bridge\");\n",
    )
    .expect("broken FFI source");
    for attempt in 1..=2 {
        let output = run(root);
        assert!(
            !output.status.success(),
            "attempt {attempt} reused stale success: {output:?}"
        );
    }
}

/// A bridge edit with no version change must still rebuild.
///
/// `updated` lists version-stamped files only, so it can never mention the FFI sources. Using it
/// as the whole notion of "nothing changed" let a recorded success survive an edit that broke the
/// bridge: `sync-versions` exited 0 reporting "Version sync complete" while the bridge no longer
/// compiled. This is the defect Windows CI surfaced through
/// `failed_same_version_rebuild_invalidates_previous_success`, but it reproduces on every platform
/// — Windows only differed in that its `updated` happened to come back empty. ~keep
#[test]
fn changed_ffi_source_rebuilds_even_when_no_version_moves() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path();
    fixture(root, true);
    fs::create_dir_all(root.join("packages/ffi/src")).expect("FFI directory");
    fs::write(
        root.join("packages/ffi/Cargo.toml"),
        "[package]\nname = 'custom-bridge'\nversion = '1.2.3'\nedition = '2024'\n",
    )
    .expect("FFI manifest");
    fs::write(root.join("packages/ffi/src/lib.rs"), "pub fn bridge() {}\n").expect("FFI source");
    assert!(run(root).status.success(), "initial sync should build the bridge");

    // Leave every version alone, so `updated` is empty, and break only the bridge itself.
    fs::write(
        root.join("packages/ffi/src/lib.rs"),
        "compile_error!(\"broken bridge\");\n",
    )
    .expect("broken FFI source");

    let output = run(root);
    assert!(
        !output.status.success(),
        "a broken bridge was reported as a successful sync: {output:?}"
    );
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let expected_command = format!(
        "cargo build --manifest-path {}",
        Path::new("packages/ffi").join("Cargo.toml").display()
    );
    assert!(
        diagnostics.contains("broken bridge")
            && diagnostics.contains("compile_error!")
            && diagnostics.contains(&expected_command),
        "the bridge edit did not trigger the expected failed compilation: {diagnostics}"
    );

    // And it must stay failed: a retry cannot resurrect the invalidated success.
    assert!(
        !run(root).status.success(),
        "a retry reused the invalidated success over a broken bridge"
    );
}

/// An untouched tree must still skip the rebuild, so the fingerprint does not turn every
/// `sync-versions` into a cargo spawn. ~keep
#[test]
fn unchanged_ffi_source_still_skips_the_rebuild() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path();
    fixture(root, true);
    fs::create_dir_all(root.join("packages/ffi/src")).expect("FFI directory");
    fs::write(
        root.join("packages/ffi/Cargo.toml"),
        "[package]\nname = 'custom-bridge'\nversion = '1.2.3'\nedition = '2024'\n",
    )
    .expect("FFI manifest");
    fs::write(root.join("packages/ffi/src/lib.rs"), "pub fn bridge() {}\n").expect("FFI source");
    assert!(run(root).status.success());
    let artifact = root.join("target/debug/libcustom_bridge.rlib");
    assert!(artifact.is_file(), "initial sync did not build the bridge");
    fs::remove_file(&artifact).expect("remove only the compiled artifact");

    let second = run(root);
    assert!(second.status.success(), "{second:?}");
    assert!(
        !artifact.exists(),
        "an unchanged bridge was rebuilt despite its recorded source fingerprint"
    );
}
