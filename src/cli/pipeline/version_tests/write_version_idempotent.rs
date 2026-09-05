use super::*;

/// `write_version_to_cargo_toml` must report `Ok(true)` (and actually rewrite the
/// file) when the requested version differs from what's on disk.
#[test]
fn write_version_to_cargo_toml_reports_changed_on_new_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");

    std::fs::write(&path, "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n").expect("write");

    let changed = write_version_to_cargo_toml(path.to_str().unwrap(), "0.2.0").expect("write ok");
    assert!(changed, "a genuinely new version must report changed = true");

    let after = std::fs::read_to_string(&path).expect("read");
    assert!(
        after.contains(r#"version = "0.2.0""#),
        "the new version must actually be written:\n{after}"
    );
}

/// Regression test for a defect hit during a downstream release: re-running
/// `alef sync-versions --set X` when Cargo.toml is already at X must succeed
/// idempotently, not bail with "could not find a version field". Before this fix,
/// `changed` alone gated both the write AND the error path, so "found the field,
/// it already matches" and "never found the field at all" were indistinguishable —
/// a release engineer re-running a bump after a partial failure was told their
/// manifest was malformed when it was perfectly fine. `write_version_to_cargo_toml`
/// must now report `Ok(false)` for this case: found, no-op, no error, and — just as
/// importantly — the file's bytes must come out byte-identical, proving no spurious
/// rewrite happened even though nothing needed to change. ~keep
#[test]
fn write_version_to_cargo_toml_is_idempotent_when_already_at_target_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");

    let cargo_toml = "[package]\nname = \"mylib\"\nversion = \"0.3.0\"\nedition = \"2024\"\n";
    std::fs::write(&path, cargo_toml).expect("write");

    let changed = write_version_to_cargo_toml(path.to_str().unwrap(), "0.3.0").expect("no-op must not error");
    assert!(
        !changed,
        "setting the version already on disk must report changed = false"
    );

    let after = std::fs::read_to_string(&path).expect("read");
    assert_eq!(
        after, cargo_toml,
        "a no-op must leave the file byte-for-byte untouched, not just semantically equal"
    );
}

/// Same idempotency guarantee for the `[workspace.package]` shape, since
/// `write_version_to_cargo_toml` checks both `[package].version` and
/// `[workspace.package].version` independently.
#[test]
fn write_version_to_cargo_toml_is_idempotent_for_workspace_package() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");

    let cargo_toml = "[workspace.package]\nversion = \"1.2.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n";
    std::fs::write(&path, cargo_toml).expect("write");

    let changed = write_version_to_cargo_toml(path.to_str().unwrap(), "1.2.3").expect("no-op must not error");
    assert!(
        !changed,
        "setting the workspace version already on disk must report changed = false"
    );

    let after = std::fs::read_to_string(&path).expect("read");
    assert_eq!(after, cargo_toml, "a no-op must leave the file byte-for-byte untouched");
}

/// A manifest with no `[package].version` or `[workspace.package].version` at all
/// must still error — the no-op fast path above must not swallow a genuinely
/// malformed/versionless manifest.
#[test]
fn write_version_to_cargo_toml_errors_when_no_version_field_exists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");

    std::fs::write(
        &path,
        "[package]\nname = \"mylib\"\nedition = \"2024\"\n\n[dependencies]\nserde = \"1.0\"\n",
    )
    .expect("write");

    let result = write_version_to_cargo_toml(path.to_str().unwrap(), "1.0.0");
    assert!(
        result.is_err(),
        "a manifest with no [package]/[workspace.package] version field must still error"
    );
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("Could not find a `[package]`/`[workspace.package]` version field"),
        "error message must accurately describe a genuinely missing version field: {message}"
    );
}
