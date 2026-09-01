use super::*;

/// `compute_sha256_hex` must return the correct SHA-256 digest for a known
/// input. The expected value was computed independently with:
///
/// ```sh
/// printf '' | shasum -a 256  # → e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
/// printf 'abc' | shasum -a 256  # → ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469f26f5816a7fef1500
/// ```
#[test]
fn compute_sha256_hex_empty_input() {
    let hex = compute_sha256_hex(b"");
    assert_eq!(
        hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "SHA-256 of empty input must match reference"
    );
}

#[test]
fn compute_sha256_hex_abc() {
    let hex = compute_sha256_hex(b"abc");
    assert_eq!(
        hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "SHA-256 of 'abc' must match reference"
    );
}

/// `precompute_swift_checksum` must substitute `__ALEF_SWIFT_CHECKSUM__` in
/// `Package.swift` when a pre-built `.artifactbundle.zip` exists in
/// `dist/swift-artifactbundle/` and the current config has swift configured.
///
/// This test does not shell out to `swift package compute-checksum`; it uses
/// the in-process SHA-256 fallback because `swift` may not be on PATH in CI.
#[test]
fn precompute_swift_checksum_substitutes_when_zip_present() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let pkg_content = concat!(
        "// swift-tools-version: 6.0\n",
        "import PackageDescription\n",
        "let package = Package(name: \"TestLib\", targets: [\n",
        "  .binaryTarget(\n",
        "    name: \"RustBridge\",\n",
        "    url: \"https://example.com/testlib/releases/download/v2.0.0/TestLib-rs.artifactbundle.zip\",\n",
        "    checksum: \"__ALEF_SWIFT_CHECKSUM__\"\n",
        "  ),\n",
        "])\n",
    );
    std::fs::write(root.join("Package.swift"), pkg_content).expect("write Package.swift");

    let swift_crate_dir = root.join("crates/testlib-swift");
    std::fs::create_dir_all(&swift_crate_dir).expect("mkdir swift crate");
    std::fs::write(
        swift_crate_dir.join("Cargo.toml"),
        "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n",
    )
    .expect("write swift Cargo.toml");

    let bundle_dir = root.join("dist/swift-artifactbundle");
    std::fs::create_dir_all(&bundle_dir).expect("mkdir bundle dir");
    let zip_content = b"fake-artifactbundle-zip-content-for-testing";
    std::fs::write(bundle_dir.join("TestLib-rs.artifactbundle.zip"), zip_content).expect("write fake zip");

    let expected_checksum = compute_sha256_hex(zip_content);

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("chdir");
    let result = precompute_swift_checksum(&resolved_cfg);
    let _ = std::env::set_current_dir(&original_cwd);

    let checksum = result
        .expect("precompute_swift_checksum must succeed")
        .expect("must return Some(checksum) when zip is present");

    assert_eq!(
        checksum, expected_checksum,
        "returned checksum must equal in-process SHA-256 of the fake zip"
    );

    let pkg_result = std::fs::read_to_string(root.join("Package.swift")).expect("read");
    assert!(
        !pkg_result.contains("__ALEF_SWIFT_CHECKSUM__"),
        "Package.swift must not retain the placeholder after precompute, got:\n{pkg_result}"
    );
    assert!(
        pkg_result.contains(&expected_checksum),
        "Package.swift must contain the computed checksum, got:\n{pkg_result}"
    );

    let sidecar =
        std::fs::read_to_string(root.join("target/alef-swift-checksum.txt")).expect("sidecar file must exist");
    assert_eq!(
        sidecar.trim(),
        expected_checksum,
        "sidecar must contain the computed checksum"
    );
}

/// `precompute_swift_checksum` must skip gracefully when no zip is found and
/// the cargo build fails (missing Apple targets on non-macOS CI).
#[test]
fn precompute_swift_checksum_skips_when_no_zip_and_build_fails() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let pkg_content = concat!(
        "// swift-tools-version: 6.0\n",
        "let package = Package(name: \"TestLib\", targets: [\n",
        "  .binaryTarget(name: \"RustBridge\",\n",
        "    url: \"https://example.com/v2.0.0/TestLib-rs.artifactbundle.zip\",\n",
        "    checksum: \"__ALEF_SWIFT_CHECKSUM__\"\n",
        "  ),\n",
        "])\n",
    );
    std::fs::write(root.join("Package.swift"), pkg_content).expect("write Package.swift");

    let swift_crate_dir = root.join("crates/testlib-swift");
    std::fs::create_dir_all(&swift_crate_dir).expect("mkdir swift crate");
    std::fs::write(
        swift_crate_dir.join("Cargo.toml"),
        "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n[lib]\nname = \"nonexistent_guaranteed_fail\"\n",
    )
    .expect("write swift Cargo.toml");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("chdir");
    let result = precompute_swift_checksum(&resolved_cfg);
    let _ = std::env::set_current_dir(&original_cwd);

    assert!(
        result.is_ok(),
        "precompute_swift_checksum must not propagate build errors, got: {:?}",
        result
    );
    assert!(result.unwrap().is_none(), "must return None when build fails");

    let pkg_result = std::fs::read_to_string(root.join("Package.swift")).expect("read");
    assert!(
        pkg_result.contains("__ALEF_SWIFT_CHECKSUM__"),
        "Package.swift must retain placeholder when build fails, got:\n{pkg_result}"
    );
}

/// Regression test for the one-way substitution defect: once a real release commit
/// substitutes `__ALEF_SWIFT_CHECKSUM__` with a literal checksum, `precompute_swift_checksum`
/// must still rewrite that field on the next release rather than treating the literal's mere
/// presence as "already done, nothing to do". Without the fix, this test fails because the
/// function returns `Ok(None)` and leaves the stale checksum from the previous release in
/// place, exactly as observed in crawlberg after commit `e1d2b2293`.
#[test]
fn precompute_swift_checksum_rewrites_existing_literal_checksum() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let stale_checksum = "a".repeat(64);
    let pkg_content = format!(
        concat!(
            "// swift-tools-version: 6.0\n",
            "import PackageDescription\n",
            "let package = Package(name: \"TestLib\", targets: [\n",
            "  .binaryTarget(\n",
            "    name: \"RustBridge\",\n",
            "    url: \"https://example.com/testlib/releases/download/v1.0.0/TestLib-rs.artifactbundle.zip\",\n",
            "    checksum: \"{}\"\n",
            "  ),\n",
            "])\n",
        ),
        stale_checksum
    );
    std::fs::write(root.join("Package.swift"), &pkg_content).expect("write Package.swift");

    let swift_crate_dir = root.join("crates/testlib-swift");
    std::fs::create_dir_all(&swift_crate_dir).expect("mkdir swift crate");
    std::fs::write(
        swift_crate_dir.join("Cargo.toml"),
        "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n",
    )
    .expect("write swift Cargo.toml");

    let bundle_dir = root.join("dist/swift-artifactbundle");
    std::fs::create_dir_all(&bundle_dir).expect("mkdir bundle dir");
    let zip_content = b"fake-artifactbundle-zip-content-for-a-new-release";
    std::fs::write(bundle_dir.join("TestLib-rs.artifactbundle.zip"), zip_content).expect("write fake zip");

    let expected_checksum = compute_sha256_hex(zip_content);
    assert_ne!(
        expected_checksum, stale_checksum,
        "test fixture must exercise an actual change, not a coincidental no-op"
    );

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("chdir");
    let result = precompute_swift_checksum(&resolved_cfg);
    let _ = std::env::set_current_dir(&original_cwd);

    let checksum = result
        .expect("precompute_swift_checksum must succeed")
        .expect("must return Some(checksum) even when Package.swift already holds a literal checksum");

    assert_eq!(
        checksum, expected_checksum,
        "returned checksum must equal the freshly computed SHA-256, not the stale one"
    );

    let pkg_result = std::fs::read_to_string(root.join("Package.swift")).expect("read");
    assert!(
        !pkg_result.contains(&stale_checksum),
        "Package.swift must not retain the stale checksum after precompute, got:\n{pkg_result}"
    );
    assert!(
        pkg_result.contains(&expected_checksum),
        "Package.swift must contain the freshly computed checksum, got:\n{pkg_result}"
    );
}

/// `precompute_swift_checksum` must fail loudly, not silently skip, when `Package.swift`
/// has neither the placeholder nor a literal checksum field to substitute into -- that
/// state means the file's format has drifted underneath this function, and returning
/// `Ok(None)` would report success while doing nothing.
#[test]
fn precompute_swift_checksum_errors_when_no_checksum_field_present() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let pkg_content = concat!(
        "// swift-tools-version: 6.0\n",
        "import PackageDescription\n",
        "let package = Package(name: \"TestLib\", targets: [\n",
        "  .target(name: \"TestLib\"),\n",
        "])\n",
    );
    std::fs::write(root.join("Package.swift"), pkg_content).expect("write Package.swift");

    let swift_crate_dir = root.join("crates/testlib-swift");
    std::fs::create_dir_all(&swift_crate_dir).expect("mkdir swift crate");
    std::fs::write(
        swift_crate_dir.join("Cargo.toml"),
        "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n",
    )
    .expect("write swift Cargo.toml");

    let bundle_dir = root.join("dist/swift-artifactbundle");
    std::fs::create_dir_all(&bundle_dir).expect("mkdir bundle dir");
    let zip_content = b"fake-artifactbundle-zip-content-for-testing-missing-field";
    std::fs::write(bundle_dir.join("TestLib-rs.artifactbundle.zip"), zip_content).expect("write fake zip");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("chdir");
    let result = precompute_swift_checksum(&resolved_cfg);
    let _ = std::env::set_current_dir(&original_cwd);

    let error = result.expect_err("must fail loudly when no checksum field is present to substitute into");
    assert!(
        error.to_string().contains("checksum"),
        "error message must mention the missing checksum field, got: {error}"
    );

    let pkg_result = std::fs::read_to_string(root.join("Package.swift")).expect("read");
    assert_eq!(
        pkg_result, pkg_content,
        "Package.swift must be left untouched when the substitution fails"
    );
}
