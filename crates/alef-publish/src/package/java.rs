//! Java JNI classifier-staged JAR packaging.
//!
//! Takes the FFI shared library for a target and stages it under
//! `src/main/resources/natives/{classifier}/` ready for Maven to package as a
//! classifier JAR. Optionally invokes `mvn package` to produce the JAR.
//!
//! JNI classifier convention (from Maven's JNI classifier standards):
//! - `linux-x86_64`
//! - `linux-aarch_64`
//! - `osx-x86_64`
//! - `osx-aarch_64`
//! - `windows-x86_64`

use super::PackageArtifact;
use crate::platform::{Arch, Os, RustTarget};
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package the Java JNI native library for the given target.
///
/// Produces a staged directory layout and optionally a classified JAR.
pub fn package_java(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);
    let classifier = jni_classifier(config, target);

    // Locate the built FFI library.
    let lib_src = crate::package::find_built_artifact(workspace_root, target, &shared_lib)?;

    // Stage: packages/java/src/main/resources/natives/{classifier}/
    let pkg_dir_str = config.package_dir(alef_core::config::extras::Language::Java);
    let natives_dir = workspace_root
        .join(&pkg_dir_str)
        .join("src/main/resources/natives")
        .join(&classifier);
    fs::create_dir_all(&natives_dir).with_context(|| format!("creating natives dir {}", natives_dir.display()))?;

    let staged = natives_dir.join(&shared_lib);
    fs::copy(&lib_src, &staged).with_context(|| format!("staging {} to {}", lib_src.display(), staged.display()))?;

    // Invoke mvn package to produce a classified JAR.
    let jar_path = build_maven_jar(config, workspace_root, output_dir, version, &classifier)?;

    Ok(PackageArtifact {
        path: jar_path.clone(),
        name: jar_path
            .file_name()
            .context("jar has no filename")?
            .to_string_lossy()
            .to_string(),
        checksum: None,
    })
}

/// Return the JNI classifier string for this target.
///
/// Tries the per-language config override first, then derives from the target triple.
fn jni_classifier(config: &AlefConfig, target: &RustTarget) -> String {
    // Check for override in publish config.
    if let Some(publish) = &config.publish {
        if let Some(lang_cfg) = publish.languages.get("java") {
            if let Some(override_cls) = &lang_cfg.jni_classifier {
                return override_cls.clone();
            }
        }
    }
    derive_jni_classifier(target)
}

/// Derive the JNI classifier from a Rust target triple.
///
/// Maps to the standard JNA/JNI naming convention:
/// `{os}-{arch}` where arch uses Java's convention (`aarch_64` not `aarch64`).
pub fn derive_jni_classifier(target: &RustTarget) -> String {
    let os = match target.os {
        Os::Linux => "linux",
        Os::MacOs => "osx",
        Os::Windows => "windows",
        Os::Unknown => "unknown",
    };
    let arch = match target.arch {
        Arch::X86_64 => "x86_64",
        Arch::Aarch64 => "aarch_64", // JNI uses underscore, not "aarch64"
        Arch::Arm => "arm",
        Arch::Wasm32 => "wasm32",
    };
    format!("{os}-{arch}")
}

fn build_maven_jar(
    config: &AlefConfig,
    workspace_root: &Path,
    output_dir: &Path,
    _version: &str,
    classifier: &str,
) -> Result<PathBuf> {
    let pkg_dir_str = config.package_dir(alef_core::config::extras::Language::Java);
    let pkg_dir = workspace_root.join(&pkg_dir_str);

    if !pkg_dir.join("pom.xml").exists() {
        anyhow::bail!("pom.xml not found in {}", pkg_dir.display());
    }

    let cmd = format!("mvn --batch-mode package -Dclassifier={classifier} -DskipTests");
    crate::run_shell_command_in(&cmd, &pkg_dir)?;

    // Find the produced JAR in target/.
    let jar = find_jar(&pkg_dir.join("target"), classifier)?;
    let jar_name = jar
        .file_name()
        .context("jar has no filename")?
        .to_string_lossy()
        .to_string();
    let dest = output_dir.join(&jar_name);
    fs::copy(&jar, &dest)?;
    Ok(dest)
}

fn find_jar(target_dir: &Path, classifier: &str) -> Result<PathBuf> {
    if !target_dir.exists() {
        anyhow::bail!("Maven target/ directory not found: {}", target_dir.display());
    }
    let candidates: Vec<PathBuf> = fs::read_dir(target_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "jar")
                && p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n.contains(classifier) && !n.contains("sources") && !n.contains("javadoc")
                })
        })
        .collect();
    candidates
        .into_iter()
        .next()
        .with_context(|| format!("no classified JAR for '{classifier}' in {}", target_dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jni_classifier_linux_x86_64() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(derive_jni_classifier(&t), "linux-x86_64");
    }

    #[test]
    fn jni_classifier_linux_aarch64() {
        let t = RustTarget::parse("aarch64-unknown-linux-gnu").unwrap();
        assert_eq!(derive_jni_classifier(&t), "linux-aarch_64");
    }

    #[test]
    fn jni_classifier_macos_arm64() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(derive_jni_classifier(&t), "osx-aarch_64");
    }

    #[test]
    fn jni_classifier_macos_x64() {
        let t = RustTarget::parse("x86_64-apple-darwin").unwrap();
        assert_eq!(derive_jni_classifier(&t), "osx-x86_64");
    }

    #[test]
    fn jni_classifier_windows_x86_64() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(derive_jni_classifier(&t), "windows-x86_64");
    }

    #[test]
    fn jni_classifier_config_override() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["java"]
[crate]
name = "mylib"
sources = ["src/lib.rs"]
[publish.languages.java]
jni_classifier = "linux-x86_64-custom"
"#,
        )
        .unwrap();
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(jni_classifier(&config, &t), "linux-x86_64-custom");
    }

    #[test]
    fn find_jar_returns_classified() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target_dir = tmp.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("mylib-1.0.0-linux-x86_64.jar"), b"fake").unwrap();
        fs::write(target_dir.join("mylib-1.0.0-sources.jar"), b"src").unwrap();

        let result = find_jar(&target_dir, "linux-x86_64").unwrap();
        assert!(result.file_name().unwrap().to_str().unwrap().contains("linux-x86_64"));
    }
}
