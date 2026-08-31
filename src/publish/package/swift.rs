//! Swift package — archives the swift-bridge source tree + XCFramework placeholder.

use super::PackageArtifact;
use super::util::{copy_dir_recursive, copy_optional_file};
use crate::core::config::ResolvedCrateConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Linux Swift build instructions for non-Apple targets.
const LINUX_BUILDING_MD: &str = "\
# Building on Linux\n\
\n\
The same `rust/` swift-bridge crate that drives the macOS/iOS XCFramework also\n\
builds on Linux. swift-bridge generates Swift glue files at build time; SwiftPM\n\
on Linux consumes them via the same `Package.swift` checked into this archive.\n\
\n\
## Steps\n\
\n\
1. Build the Rust shared library:\n\
\n\
   ```sh\n\
   cd rust\n\
   cargo build --release --target x86_64-unknown-linux-gnu\n\
   # Or for ARM64 servers:\n\
   cargo build --release --target aarch64-unknown-linux-gnu\n\
   ```\n\
\n\
2. The build script (`rust/build.rs`) writes Swift glue into `OUT_DIR`. Surface it\n\
   to SwiftPM by symlinking or copying into `Sources/<Module>/generated/`.\n\
\n\
3. Build and test the Swift package:\n\
\n\
   ```sh\n\
   swift build -c release\n\
   swift test\n\
   ```\n\
\n\
Linux Swift consumers (e.g., Vapor servers) link the resulting `.so` directly\n\
through SwiftPM — no XCFramework involvement. The `LD_LIBRARY_PATH` export is\n\
required because SwiftPM does not auto-discover Cargo's `target/release/` output\n\
at runtime; without it, `swift test` fails with a dynamic linker error.\n\
";

/// XCFramework build instructions emitted as a placeholder.
const BUILDING_MD: &str = "\
# Building the XCFramework\n\
\n\
Real XCFramework creation requires `xcodebuild` and must be performed on a macOS host\n\
after compiling the Rust crate for all desired Apple targets.\n\
\n\
## Steps\n\
\n\
1. Build the Rust crate for each target slice, e.g.:\n\
\n\
   ```sh\n\
   cargo build --release --target aarch64-apple-ios\n\
   cargo build --release --target x86_64-apple-ios-simulator\n\
   cargo build --release --target aarch64-apple-darwin\n\
   ```\n\
\n\
2. Create a fat library for the simulator slice (optional):\n\
\n\
   ```sh\n\
   lipo -create \\\n\
     target/x86_64-apple-ios-simulator/release/libmy_lib.a \\\n\
     target/aarch64-apple-ios-simulator/release/libmy_lib.a \\\n\
     -output libmy_lib_simulator.a\n\
   ```\n\
\n\
3. Assemble the XCFramework:\n\
\n\
   ```sh\n\
   xcodebuild -create-xcframework \\\n\
     -library target/aarch64-apple-ios/release/libmy_lib.a \\\n\
     -headers include/ \\\n\
     -library libmy_lib_simulator.a \\\n\
     -headers include/ \\\n\
     -output MyLib.xcframework\n\
   ```\n\
\n\
4. Compress and compute checksum:\n\
\n\
   ```sh\n\
   zip -r MyLib.xcframework.zip MyLib.xcframework\n\
   swift package compute-checksum MyLib.xcframework.zip\n\
   ```\n\
\n\
The `Package.swift` in this archive references `XCFramework.xcframework/`; replace\n\
this placeholder with the real framework after completing the above steps.\n\
";

/// Package Swift bindings into a source tarball suitable for Swift Package Manager.
///
/// Produces: `{module}-{version}.tar.gz` containing:
/// - `Package.swift` — copied from `packages/swift/Package.swift`
/// - `Sources/{Module}/` — Swift wrappers (copied from `packages/swift/Sources/`)
/// - `Tests/{Module}Tests/` — e2e tests if present in `packages/swift/Tests/`
/// - `rust/` — Rust-side swift-bridge crate
/// - `xcframework/` — placeholder directory with `BUILDING.md`
/// - `README.md`, `CHANGELOG.md`, `LICENSE` if present in workspace root
///
/// The `xcframework/` placeholder exists so consumers know where the real XCFramework
/// goes; actual XCFramework creation requires `xcodebuild` and is documented in
/// `xcframework/BUILDING.md`.
pub fn package_swift(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let module = config.swift_module();
    let pkg_dir = config.package_dir(crate::core::config::extras::Language::Swift);

    let pkg_name = format!("{module}-{version}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    let pkg_src = workspace_root.join(&pkg_dir);
    if !pkg_src.exists() {
        anyhow::bail!("Swift package directory not found: {}", pkg_dir);
    }
    copy_dir_recursive(&pkg_src, &staging).context("copying Swift package directory")?;

    let root_manifest = workspace_root.join("Package.swift");
    if root_manifest.exists() {
        fs::copy(&root_manifest, staging.join("Package.swift")).context("copying root Swift Package.swift")?;
    }
    // A SwiftPM package with no `Package.swift` is unusable, and `patch_root_package_manifest`
    // treats a missing manifest as "nothing to patch" -- so without this check the tarball ships
    // silently unbuildable. ~keep
    if !staging.join("Package.swift").exists() {
        anyhow::bail!(
            "Swift package manifest not found: neither {} nor {}/Package.swift exists",
            root_manifest.display(),
            pkg_dir
        );
    }

    // Resolved here rather than inside `patch_root_package_manifest` so that function stays a
    // pure transform over its explicit inputs -- see its doc comment. ~keep
    let checksum = std::env::var("ALEF_SWIFT_CHECKSUM")
        .or_else(|_| std::env::var("SWIFT_ARTIFACT_CHECKSUM"))
        .ok();
    patch_root_package_manifest(&staging, version, checksum.as_deref()).context("patching root Swift Package.swift")?;

    let xcframework_dir = staging.join("xcframework");
    fs::create_dir_all(&xcframework_dir).context("creating xcframework placeholder directory")?;
    fs::write(xcframework_dir.join("BUILDING.md"), BUILDING_MD).context("writing xcframework/BUILDING.md")?;

    let linux_dir = staging.join("linux");
    fs::create_dir_all(&linux_dir).context("creating linux build instructions directory")?;
    fs::write(linux_dir.join("BUILDING.md"), LINUX_BUILDING_MD).context("writing linux/BUILDING.md")?;

    for filename in ["README.md", "CHANGELOG.md", "LICENSE"] {
        copy_optional_file(workspace_root, filename, &staging)
            .with_context(|| format!("staging {filename} for Swift package"))?;
    }

    let archive_name = format!("{pkg_name}.tar.gz");
    let archive_path = output_dir.join(&archive_name);
    super::create_tar_gz(&staging, &archive_path)?;

    fs::remove_dir_all(&staging).ok();

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: None,
    })
}

/// Replace the release placeholders in a staged root `Package.swift`.
///
/// `checksum` is resolved by the caller ([`package_swift`], from `ALEF_SWIFT_CHECKSUM` /
/// `SWIFT_ARTIFACT_CHECKSUM`) rather than read from the environment here. Reading it internally
/// would make this function -- and any test calling it directly -- depend on process-global
/// state shared with every other test in the binary, the same "locally serialized, globally not"
/// shape `f968767b6`/`227942b4b` fixed for other process-global env/cwd dependencies in this
/// crate's tests. Threading it through as a parameter keeps this a pure transform over its
/// inputs and lets tests exercise the "checksum present" and "checksum missing" branches with an
/// explicit value instead of mutating `std::env`. ~keep
fn patch_root_package_manifest(staging: &Path, version: &str, checksum: Option<&str>) -> Result<()> {
    let manifest = staging.join("Package.swift");
    if !manifest.exists() {
        return Ok(());
    }
    let mut content = fs::read_to_string(&manifest).context("reading staged Package.swift")?;
    content = content.replace("__ALEF_SWIFT_VERSION__", version);
    if content.contains("__ALEF_SWIFT_CHECKSUM__") {
        let checksum =
            checksum.context("ALEF_SWIFT_CHECKSUM must be set when Package.swift contains __ALEF_SWIFT_CHECKSUM__")?;
        content = content.replace("__ALEF_SWIFT_CHECKSUM__", checksum);
    }
    fs::write(&manifest, content).context("writing staged Package.swift")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
    use std::fs;

    fn minimal_config(name: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "{name}"
sources = []
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).expect("valid config");
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn package_swift_errors_when_pkg_dir_missing() {
        let config = minimal_config("my-lib");
        let tmp = tempfile::tempdir().expect("tempdir");
        let output = tmp.path().join("out");
        fs::create_dir_all(&output).unwrap();

        let err = package_swift(&config, tmp.path(), &output, "0.1.0").unwrap_err();
        assert!(
            err.to_string().contains("Swift package directory not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn package_swift_produces_tarball() {
        let config = minimal_config("my-lib");
        let tmp = tempfile::tempdir().expect("tempdir");

        let swift_pkg = tmp.path().join("packages/swift");
        fs::create_dir_all(swift_pkg.join("Sources/MyLib")).unwrap();
        fs::write(swift_pkg.join("Package.swift"), "// swift-tools-version:5.9\n").unwrap();
        fs::write(swift_pkg.join("Sources/MyLib/MyLib.swift"), "public struct MyLib {}\n").unwrap();

        let output = tmp.path().join("out");
        fs::create_dir_all(&output).unwrap();

        let artifact = package_swift(&config, tmp.path(), &output, "0.1.0").unwrap();
        assert!(artifact.path.exists(), "tarball should exist");
        assert_eq!(artifact.name, "MyLib-0.1.0.tar.gz");
    }

    /// A staged tree with no `Package.swift` is not a SwiftPM package: packaging must fail
    /// loudly instead of producing a tarball SwiftPM cannot open.
    #[test]
    fn package_swift_errors_when_no_manifest_is_staged() {
        let config = minimal_config("my-lib");
        let tmp = tempfile::tempdir().expect("tempdir");

        let swift_pkg = tmp.path().join("packages/swift");
        fs::create_dir_all(swift_pkg.join("Sources/MyLib")).unwrap();
        fs::write(swift_pkg.join("Sources/MyLib/MyLib.swift"), "public struct MyLib {}\n").unwrap();

        let output = tmp.path().join("out");
        fs::create_dir_all(&output).unwrap();

        let error = package_swift(&config, tmp.path(), &output, "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Swift package manifest not found"),
            "unexpected error: {error}"
        );
        assert!(
            !output.join("MyLib-0.1.0.tar.gz").exists(),
            "no tarball may be produced without a manifest"
        );
    }

    #[test]
    fn package_swift_module_name_from_config() {
        let toml = r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []
[crates.swift]
module_name = "AlefCore"
"#;
        let cfg: NewAlefConfig = toml::from_str(toml).expect("valid config");
        let config = cfg.resolve().unwrap().remove(0);
        let tmp = tempfile::tempdir().expect("tempdir");

        let swift_pkg = tmp.path().join("packages/swift");
        fs::create_dir_all(&swift_pkg).unwrap();
        fs::write(swift_pkg.join("Package.swift"), "// swift-tools-version:5.9\n").unwrap();

        let output = tmp.path().join("out");
        fs::create_dir_all(&output).unwrap();

        let artifact = package_swift(&config, tmp.path(), &output, "1.2.3").unwrap();
        assert_eq!(artifact.name, "AlefCore-1.2.3.tar.gz");
    }

    #[test]
    fn patch_root_package_manifest_replaces_release_placeholders() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest = tmp.path().join("Package.swift");
        fs::write(
            &manifest,
            r#"url: "https://example.test/releases/download/v__ALEF_SWIFT_VERSION__/Demo.zip",
checksum: "__ALEF_SWIFT_CHECKSUM__"
"#,
        )
        .unwrap();

        patch_root_package_manifest(tmp.path(), "1.2.3", Some("abc123")).unwrap();

        let content = fs::read_to_string(manifest).unwrap();
        assert!(
            content.contains("v1.2.3"),
            "version placeholder must be replaced: {content}"
        );
        assert!(
            content.contains("abc123"),
            "checksum placeholder must be replaced: {content}"
        );
        assert!(
            !content.contains("__ALEF_SWIFT_"),
            "no Swift placeholders should remain: {content}"
        );
    }

    /// A manifest carrying the checksum placeholder with no checksum supplied must fail with a
    /// clear message, not silently ship a literal `__ALEF_SWIFT_CHECKSUM__` in the release
    /// manifest. Exercised with `checksum: None` directly -- now that
    /// `patch_root_package_manifest` takes the checksum as a parameter rather than reading
    /// `ALEF_SWIFT_CHECKSUM` itself, this branch no longer needs `std::env` at all.
    #[test]
    fn patch_root_package_manifest_errors_when_checksum_placeholder_has_no_checksum() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest = tmp.path().join("Package.swift");
        fs::write(&manifest, r#"checksum: "__ALEF_SWIFT_CHECKSUM__""#).unwrap();

        let error = patch_root_package_manifest(tmp.path(), "1.2.3", None).unwrap_err();
        assert!(
            error.to_string().contains("ALEF_SWIFT_CHECKSUM must be set"),
            "unexpected error: {error}"
        );
    }
}
