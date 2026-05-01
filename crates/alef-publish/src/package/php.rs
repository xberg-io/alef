//! PHP PIE binary package builder.
//!
//! Produces a single flat archive containing the compiled PHP extension,
//! named according to the PIE convention so that `pie install <vendor>/<pkg>`
//! resolves to a pre-built binary instead of compiling from source.
//!
//! **Unix archive** (`{ext}.so` at archive root, `.tgz`):
//! ```text
//! php_{ext}-{ver}_php{phpVer}-{arch}-{os}-{libc}-{ts}.tgz
//! ```
//!
//! **Windows archive** (`{ext}.dll` at archive root, `.zip`):
//! ```text
//! php_{ext}-{ver}-{phpVer}-{ts}-{compiler}-{arch}.zip
//! ```

use super::PackageArtifact;
use crate::platform::{Os, RustTarget};
use alef_core::config::AlefConfig;
use anyhow::{Result, bail};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Thread-safety mode for the PHP extension binary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TsMode {
    /// Non-thread-safe (NTS) — the common default.
    Nts,
    /// Zend Thread Safe (ZTS).
    Ts,
}

impl TsMode {
    /// Short tag used in PIE filenames: `"nts"` or `"ts"` (Unix) / `"nts"` or `"ts"` (Windows).
    ///
    /// Note: The Unix PIE convention uses `"zts"` for thread-safe mode.
    pub fn as_short(&self) -> &'static str {
        match self {
            Self::Nts => "nts",
            Self::Ts => "ts",
        }
    }

    /// Unix-specific suffix used in the tarball filename: `"nts"` or `"zts"`.
    fn as_unix_suffix(&self) -> &'static str {
        match self {
            Self::Nts => "nts",
            Self::Ts => "zts",
        }
    }

    /// Parse from a string (`"nts"` or `"ts"`, case-insensitive).
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "nts" => Ok(Self::Nts),
            "ts" | "zts" => Ok(Self::Ts),
            other => bail!("unknown ts-mode '{other}': expected 'nts' or 'ts'"),
        }
    }
}

/// Options that control PIE-conventional PHP packaging.
pub struct PiePackageOptions<'a> {
    /// PHP minor version string, e.g. `"8.5"`. Required.
    pub php_version: &'a str,
    /// Thread-safety mode. Defaults to `Nts`.
    pub ts_mode: TsMode,
    /// Override the libc tag (e.g. `"musl"`). Auto-detected from the target triple when `None`.
    pub libc_override: Option<&'a str>,
    /// Windows compiler tag, e.g. `"vs17"`. Required when `target.os == Windows`.
    pub windows_compiler: Option<&'a str>,
}

/// Package a PHP extension binary as a PIE-conventional archive.
///
/// Returns a single `PackageArtifact` whose `name` follows the PIE filename
/// convention and whose `checksum` is set to the SHA-256 hex digest of the
/// archive (written as `{archive_name}.sha256` next to the archive).
pub fn package_php(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
    options: &PiePackageOptions<'_>,
) -> Result<PackageArtifact> {
    // PHP extension name comes from `[php].extension_name` in alef.toml (which is
    // also the value emitted into composer.json's `php-ext.extension-name`). PIE
    // installs the binary using this name, so it MUST match composer.json — never
    // derive from the crate name (which carries a `-php` binding suffix).
    let ext_name = config.php_extension_name();

    // Cargo's compiled artifact filename comes from the crate name, not the PHP
    // extension name — for `html-to-markdown-php` cargo emits `html_to_markdown_php.{so,dylib,dll}`.
    let cargo_lib_stem = crate::crate_name_from_output(config, alef_core::config::extras::Language::Php)
        .map(|n| n.replace('-', "_"))
        .unwrap_or_else(|| ext_name.clone());

    let archive_name = pie_archive_name(&ext_name, version, target, options)?;
    let archive_path = output_dir.join(&archive_name);

    // Staging directory (cleaned up after archive creation).
    let staging = output_dir.join(format!("_pie_stage_{ext_name}_{}", target.triple));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    if target.os == Os::Windows {
        // Locate the cargo-produced .dll and rename it to {ext_name}.dll inside the archive.
        let cargo_dll_name = format!("{cargo_lib_stem}.dll");
        let dll_src = find_php_ext(workspace_root, target, &cargo_dll_name)?;
        let staged_name = format!("{ext_name}.dll");
        fs::copy(&dll_src, staging.join(&staged_name))?;
        create_zip(&staging, &archive_path)?;
    } else {
        // Locate cargo's .so/.dylib and rename to {ext_name}.so inside the archive.
        // PIE always looks for {ext_name}.so on Unix — even on macOS where cargo emits .dylib.
        let cargo_lib_file = target.shared_lib_name(&cargo_lib_stem);
        let lib_src = find_php_ext(workspace_root, target, &cargo_lib_file)?;
        let staged_name = format!("{ext_name}.so");
        fs::copy(&lib_src, staging.join(&staged_name))?;
        super::create_tar_gz(&staging, &archive_path)?;
    }

    fs::remove_dir_all(&staging).ok();

    // Compute and write SHA-256 sidecar.
    let checksum = sha256_file(&archive_path)?;
    let sidecar_path = output_dir.join(format!("{archive_name}.sha256"));
    fs::write(&sidecar_path, format!("{checksum}  {archive_name}\n"))?;

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: Some(checksum),
    })
}

/// Generate the PIE-conventional archive filename.
///
/// All components are lowercased per the PIE spec.
fn pie_archive_name(
    ext_name: &str,
    version: &str,
    target: &RustTarget,
    options: &PiePackageOptions<'_>,
) -> Result<String> {
    let lower = |s: &str| s.to_lowercase();
    if target.os == Os::Windows {
        let compiler = options
            .windows_compiler
            .ok_or_else(|| anyhow::anyhow!("windows PHP packaging requires --windows-compiler (e.g. vs17)"))?;
        Ok(lower(&format!(
            "php_{ext}-{ver}-{php}-{ts}-{cc}-{arch}.zip",
            ext = ext_name,
            ver = version,
            php = options.php_version,
            ts = options.ts_mode.as_short(),
            cc = compiler,
            arch = target.pie_arch()?,
        )))
    } else {
        let libc = options
            .libc_override
            .map(|s| s.to_string())
            .map_or_else(|| target.pie_libc().map(|s| s.to_string()), Ok)?;
        Ok(lower(&format!(
            "php_{ext}-{ver}_php{php}-{arch}-{os}-{libc}-{ts}.tgz",
            ext = ext_name,
            ver = version,
            php = options.php_version,
            arch = target.pie_arch()?,
            os = target.pie_os_family()?,
            libc = libc,
            ts = options.ts_mode.as_unix_suffix(),
        )))
    }
}

/// Locate the compiled PHP extension (`.so`, `.dylib`, or `.dll`).
///
/// Searches `target/{triple}/release/` then `target/release/`.
fn find_php_ext(workspace_root: &Path, target: &RustTarget, lib_file: &str) -> Result<PathBuf> {
    let cross = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(lib_file);
    if cross.exists() {
        return Ok(cross);
    }
    let native = workspace_root.join("target/release").join(lib_file);
    if native.exists() {
        return Ok(native);
    }
    bail!(
        "PHP extension '{lib_file}' not found in target/{}/release/ or target/release/",
        target.triple
    )
}

/// Create a zip archive from a staging directory.
///
/// Adds every file at the top level of `staging_dir` into the zip at the archive root.
fn create_zip(staging_dir: &Path, output_path: &Path) -> Result<()> {
    use std::io::Write;
    let file = fs::File::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for entry in fs::read_dir(staging_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let mut source = fs::File::open(&path)?;
        let mut buf = Vec::new();
        source.read_to_end(&mut buf)?;
        zip.start_file(&file_name, options)?;
        zip.write_all(&buf)?;
    }
    zip.finish()?;
    Ok(())
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
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::RustTarget;
    use tempfile::TempDir;

    fn make_config(name: &str) -> alef_core::config::AlefConfig {
        toml::from_str(&format!(
            r#"
languages = ["php"]
[crate]
name = "{name}"
sources = ["src/lib.rs"]
"#
        ))
        .unwrap()
    }

    fn nts_options(php_version: &str) -> PiePackageOptions<'_> {
        PiePackageOptions {
            php_version,
            ts_mode: TsMode::Nts,
            libc_override: None,
            windows_compiler: None,
        }
    }

    // --- TsMode ---

    #[test]
    fn ts_mode_from_str_nts() {
        assert_eq!(TsMode::parse("nts").unwrap(), TsMode::Nts);
    }

    #[test]
    fn ts_mode_from_str_ts() {
        assert_eq!(TsMode::parse("ts").unwrap(), TsMode::Ts);
    }

    #[test]
    fn ts_mode_from_str_zts_accepted() {
        assert_eq!(TsMode::parse("zts").unwrap(), TsMode::Ts);
    }

    #[test]
    fn ts_mode_from_str_case_insensitive() {
        assert_eq!(TsMode::parse("NTS").unwrap(), TsMode::Nts);
        assert_eq!(TsMode::parse("TS").unwrap(), TsMode::Ts);
    }

    #[test]
    fn ts_mode_from_str_unknown_errors() {
        assert!(TsMode::parse("thread").is_err());
    }

    // --- pie_archive_name ---

    #[test]
    fn pie_filename_linux_x86_64_glibc_nts() {
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let opts = nts_options("8.5");
        let name = pie_archive_name("html_to_markdown", "3.4.0", &target, &opts).unwrap();
        assert_eq!(name, "php_html_to_markdown-3.4.0_php8.5-x86_64-linux-glibc-nts.tgz");
    }

    #[test]
    fn pie_filename_linux_aarch64_musl_zts() {
        let target = RustTarget::parse("aarch64-unknown-linux-musl").unwrap();
        let opts = PiePackageOptions {
            php_version: "8.4",
            ts_mode: TsMode::Ts,
            libc_override: None,
            windows_compiler: None,
        };
        let name = pie_archive_name("myext", "1.0.0", &target, &opts).unwrap();
        assert_eq!(name, "php_myext-1.0.0_php8.4-arm64-linux-musl-zts.tgz");
    }

    #[test]
    fn pie_filename_macos_arm64_nts() {
        let target = RustTarget::parse("aarch64-apple-darwin").unwrap();
        let opts = nts_options("8.5");
        let name = pie_archive_name("html_to_markdown", "3.4.0-rc.22", &target, &opts).unwrap();
        assert_eq!(
            name,
            "php_html_to_markdown-3.4.0-rc.22_php8.5-arm64-darwin-bsdlibc-nts.tgz"
        );
    }

    #[test]
    fn pie_filename_windows_x86_64_vs17_nts() {
        let target = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        let opts = PiePackageOptions {
            php_version: "8.5",
            ts_mode: TsMode::Nts,
            libc_override: None,
            windows_compiler: Some("vs17"),
        };
        let name = pie_archive_name("html_to_markdown", "3.4.0", &target, &opts).unwrap();
        assert_eq!(name, "php_html_to_markdown-3.4.0-8.5-nts-vs17-x86_64.zip");
    }

    #[test]
    fn pie_filename_windows_missing_compiler_errors() {
        let target = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        let opts = nts_options("8.5");
        let result = pie_archive_name("myext", "1.0.0", &target, &opts);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("windows-compiler"),
            "error message should mention --windows-compiler, got: {msg}"
        );
    }

    #[test]
    fn pie_filename_lowercase_invariant() {
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let opts = nts_options("8.5");
        // Uppercase ext_name should produce all-lowercase output.
        let name = pie_archive_name("MyExt", "1.0.0", &target, &opts).unwrap();
        assert_eq!(name, name.to_lowercase(), "archive name must be all-lowercase");
    }

    #[test]
    fn pie_libc_override_wins() {
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let opts = PiePackageOptions {
            php_version: "8.4",
            ts_mode: TsMode::Nts,
            libc_override: Some("musl"),
            windows_compiler: None,
        };
        let name = pie_archive_name("ext", "1.0.0", &target, &opts).unwrap();
        // Override should win over the auto-detected "glibc".
        assert!(name.contains("-musl-"), "expected musl in name, got: {name}");
    }

    // --- Archive layout ---

    #[test]
    fn pie_archive_contains_only_extension_at_root() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let output_dir = tmp.path().join("dist");
        fs::create_dir_all(&output_dir).unwrap();

        // Create a fake .so in the expected location.
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let release_dir = workspace.join("target/x86_64-unknown-linux-gnu/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join("libhtml_to_markdown.so"), b"ELF fake so content").unwrap();

        let config = make_config("html-to-markdown");
        let opts = nts_options("8.4");
        let artifact = package_php(&config, &target, &workspace, &output_dir, "3.4.0", &opts).unwrap();

        // Verify the archive exists.
        assert!(artifact.path.exists(), "archive file must exist");
        assert!(artifact.checksum.is_some(), "checksum must be set");

        // Untar and check the single entry at root.
        let output = std::process::Command::new("tar")
            .arg("-tzf")
            .arg(&artifact.path)
            .output()
            .expect("tar must be available");
        let listing = String::from_utf8_lossy(&output.stdout);
        let entries: Vec<&str> = listing
            .lines()
            // tar on macOS may produce the directory entry first; strip it
            .filter(|l| !l.ends_with('/'))
            .collect();

        assert_eq!(
            entries.len(),
            1,
            "expected exactly one file in archive, got: {entries:?}"
        );
        assert!(
            entries[0].ends_with("html_to_markdown.so"),
            "expected html_to_markdown.so at archive root, got: {}",
            entries[0]
        );

        // Sidecar must exist.
        let sidecar = output_dir.join(format!("{}.sha256", artifact.name));
        assert!(sidecar.exists(), "SHA-256 sidecar must be written");
    }
}
