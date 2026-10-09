//! Go FFI package — archives the shared library + header for GitHub Release upload.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::publish::platform::RustTarget;
use anyhow::Result;
use std::fs;
use std::io::Read;
use std::path::Path;

/// Package Go FFI artifacts into a distributable tarball.
///
/// Produces: `{name}-go-v{version}-{platform}.tar.gz` containing:
/// - `lib/` — shared library, or a static library for musl targets, with
///   `native-static-libs.txt` listing the native libraries it needs when
///   `alef publish build` recorded them
/// - `include/` — C header
///
/// Uses a `-go-` infix (not `-ffi-`) so that Go and C FFI tarballs do not
/// collide in shared release-asset prefix matchers — the C FFI packager
/// emits `{crate_name}-ffi-v{version}-{rust-triple}.tar.gz` and the two
/// asset families need to be distinguishable for downstream verifiers
/// (asset-prefix probes, verify-release-assets pattern lists).
pub fn package_go_ffi(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let header_name = config.ffi_header_name();
    let crate_name = &config.name;
    let platform = target.platform_for(crate::core::config::extras::Language::Go);

    let pkg_name = format!("{crate_name}-go-v{version}-{platform}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let lib_dir = staging.join("lib");
    let include_dir = staging.join("include");
    fs::create_dir_all(&lib_dir)?;
    fs::create_dir_all(&include_dir)?;

    let static_lib = target.static_lib_name(&lib_name);
    let static_result = super::find_built_artifact(workspace_root, target, &static_lib, super::BuildProfile::Release);
    let static_only = target.triple.contains("-musl");

    // Packaging always ships a `--release` build -- nothing here is publishable in `debug`. ~keep
    if !static_only {
        let shared_lib = target.shared_lib_name(&lib_name);
        let shared_src = super::find_built_artifact(workspace_root, target, &shared_lib, super::BuildProfile::Release)?;
        let shared_dst = lib_dir.join(&shared_lib);
        fs::copy(&shared_src, &shared_dst)?;
        super::util::fix_macos_dylib_id(target, &shared_dst, &shared_lib)?;
    }

    let static_src = if static_only {
        Some(static_result?)
    } else {
        static_result.ok()
    };
    if let Some(static_src) = static_src {
        fs::copy(&static_src, lib_dir.join(&static_lib))?;
        match crate::publish::native_libs::find_recorded(workspace_root, target) {
            Some(recorded) => {
                fs::copy(
                    &recorded,
                    lib_dir.join(crate::publish::native_libs::NATIVE_STATIC_LIBS_FILE),
                )?;
            }
            None => tracing::warn!(
                "no {} recorded by `alef publish build`; the Go package ships the static library without the \
                 list of native libraries it links against",
                crate::publish::native_libs::NATIVE_STATIC_LIBS_FILE
            ),
        }
    }

    let ffi_crate_dir = crate::publish::ffi_stage::find_ffi_crate_dir_pub(config, workspace_root);
    let header_src = ffi_crate_dir.join("include").join(&header_name);
    if header_src.exists() {
        fs::copy(&header_src, include_dir.join(&header_name))?;
    }

    let archive_name = format!("{pkg_name}.tar.gz");
    let archive_path = output_dir.join(&archive_name);
    super::create_tar_gz(&staging, &archive_path)?;

    let _ = fs::remove_dir_all(&staging);

    let checksum = sha256_file(&archive_path)?;
    let sidecar_path = output_dir.join(format!("{archive_name}.sha256"));
    fs::write(&sidecar_path, format!("{checksum}  {archive_name}\n"))?;

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: Some(checksum),
    })
}

/// Compute the SHA-256 hex digest of a file.
fn sha256_file(path: &Path) -> Result<String> {
    use anyhow::Context as _;
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").expect("writing to String never fails");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::platform::RustTarget;
    use tempfile::TempDir;

    fn make_config(name: &str) -> crate::core::config::ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["go"]

[[crates]]
name = "{name}"
sources = ["src/lib.rs"]
"#
        ))
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn package_go_ffi_writes_checksum_and_sidecar() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let output_dir = tmp.path().join("dist");
        fs::create_dir_all(&output_dir).unwrap();

        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let release_dir = workspace.join("target/x86_64-unknown-linux-gnu/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join("libdemo_go_ffi.so"), b"ELF fake so content").unwrap();

        let config = make_config("demo_go");
        let artifact = package_go_ffi(&config, &target, &workspace, &output_dir, "1.2.3").unwrap();

        assert!(artifact.path.exists(), "archive file must exist");
        assert_eq!(
            artifact.name, "demo_go-go-v1.2.3-linux-x86_64.tar.gz",
            "archive name must embed the version, matching the asset name the setup template requests"
        );

        let expected_checksum = sha256_file(&artifact.path).unwrap();
        assert_eq!(
            artifact.checksum.as_deref(),
            Some(expected_checksum.as_str()),
            "PackageArtifact.checksum must be the archive's real SHA-256 digest"
        );

        let sidecar_path = output_dir.join(format!("{}.sha256", artifact.name));
        assert!(
            sidecar_path.exists(),
            "SHA-256 sidecar must be written next to the archive"
        );
        let sidecar_contents = fs::read_to_string(&sidecar_path).unwrap();
        assert_eq!(
            sidecar_contents,
            format!("{expected_checksum}  {}\n", artifact.name),
            "sidecar contents must be '<digest>  <archive name>\\n'"
        );
    }
    #[cfg(not(target_os = "windows"))]
    fn archive_entry(archive: &Path, entry: &str) -> Option<String> {
        let out = std::process::Command::new("tar")
            .arg("xzOf")
            .arg(archive)
            .arg(entry)
            .output()
            .unwrap();
        out.status.success().then(|| String::from_utf8(out.stdout).unwrap())
    }

    #[cfg(not(target_os = "windows"))]
    fn package_with_static_lib(record: Option<&str>) -> (TempDir, PackageArtifact) {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let output_dir = tmp.path().join("dist");
        fs::create_dir_all(&output_dir).unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let release_dir = workspace.join("target/x86_64-unknown-linux-gnu/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join("libdemo_go_ffi.so"), b"so").unwrap();
        fs::write(release_dir.join("libdemo_go_ffi.a"), b"a").unwrap();
        if let Some(record) = record {
            fs::write(
                release_dir.join(crate::publish::native_libs::NATIVE_STATIC_LIBS_FILE),
                record,
            )
            .unwrap();
        }
        let artifact = package_go_ffi(&make_config("demo_go"), &target, &workspace, &output_dir, "1.2.3").unwrap();
        (tmp, artifact)
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn package_go_ffi_ships_the_recorded_native_static_libs_beside_the_static_library() {
        let (_tmp, artifact) = package_with_static_lib(Some("-lm -lc\n"));
        let base = "demo_go-go-v1.2.3-linux-x86_64";
        assert_eq!(
            archive_entry(&artifact.path, &format!("{base}/lib/native-static-libs.txt")).as_deref(),
            Some("-lm -lc\n")
        );
        assert!(archive_entry(&artifact.path, &format!("{base}/lib/libdemo_go_ffi.a")).is_some());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn package_go_ffi_without_a_record_ships_no_native_static_libs_file() {
        let (_tmp, artifact) = package_with_static_lib(None);
        let base = "demo_go-go-v1.2.3-linux-x86_64";
        assert!(archive_entry(&artifact.path, &format!("{base}/lib/libdemo_go_ffi.a")).is_some());
        assert!(archive_entry(&artifact.path, &format!("{base}/lib/native-static-libs.txt")).is_none());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn package_go_ffi_musl_ships_static_library_without_shared_library() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let output_dir = tmp.path().join("dist");
        fs::create_dir_all(&output_dir).unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-musl").unwrap();
        let release_dir = workspace.join("target/x86_64-unknown-linux-musl/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join("libdemo_go_ffi.a"), b"a").unwrap();
        fs::write(
            release_dir.join(crate::publish::native_libs::NATIVE_STATIC_LIBS_FILE),
            "-lunwind -lc\n",
        )
        .unwrap();

        let artifact = package_go_ffi(&make_config("demo_go"), &target, &workspace, &output_dir, "1.2.3").unwrap();
        let base = "demo_go-go-v1.2.3-linux-x86_64-musl";

        assert!(archive_entry(&artifact.path, &format!("{base}/lib/libdemo_go_ffi.a")).is_some());
        assert!(archive_entry(&artifact.path, &format!("{base}/lib/libdemo_go_ffi.so")).is_none());
        assert_eq!(
            archive_entry(&artifact.path, &format!("{base}/lib/native-static-libs.txt")).as_deref(),
            Some("-lunwind -lc\n")
        );
    }

    #[test]
    fn package_go_ffi_musl_requires_static_library() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let output_dir = tmp.path().join("dist");
        fs::create_dir_all(&output_dir).unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-musl").unwrap();

        let error = package_go_ffi(&make_config("demo_go"), &target, &workspace, &output_dir, "1.2.3")
            .expect_err("musl packaging must reject a missing static library");

        assert!(error.to_string().contains("libdemo_go_ffi.a not found"));
    }
}
