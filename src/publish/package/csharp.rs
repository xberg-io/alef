//! C# NuGet RID-keyed native asset packager.
//!
//! Stages the FFI shared library under the NuGet runtime layout:
//! `runtimes/{rid}/native/{libname}`, then invokes `dotnet pack` to produce a
//! `.nupkg`. The RID is derived from the Rust target triple using the same
//! mapping as `RustTarget::platform_for(Language::Csharp)`.
//!
//! RID examples: `linux-x64`, `linux-arm64`, `osx-x64`, `osx-arm64`, `win-x64`,
//! `linux-musl-x64`, `linux-musl-arm64`.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::publish::platform::RustTarget;
use crate::scaffold::render_csharp_csproj;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package the C# NuGet native asset for the given target.
///
/// Produces: `{namespace}.{version}.nupkg` (moved to `output_dir`).
pub fn package_csharp(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);
    let rid = csharp_rid(config, target);
    let namespace = config.csharp_namespace();

    let lib_src = crate::publish::package::find_built_artifact(workspace_root, target, &shared_lib)?;

    let pkg_dir_str = config.package_dir(crate::core::config::extras::Language::Csharp);
    let runtimes_dir = workspace_root
        .join(&pkg_dir_str)
        .join(&namespace)
        .join("runtimes")
        .join(&rid)
        .join("native");
    fs::create_dir_all(&runtimes_dir).with_context(|| format!("creating runtimes dir {}", runtimes_dir.display()))?;

    let staged = runtimes_dir.join(&shared_lib);
    fs::copy(&lib_src, &staged).with_context(|| format!("staging {} to {}", lib_src.display(), staged.display()))?;

    let csproj = find_csproj(workspace_root, &pkg_dir_str, &namespace)?;
    let proj_dir = csproj.parent().context("csproj has no parent")?.to_path_buf();

    let csproj_content = render_csharp_csproj(config, version);
    fs::write(&csproj, &csproj_content).with_context(|| format!("regenerating csproj at {}", csproj.display()))?;
    tracing::debug!(path = %csproj.display(), "regenerated csproj from scaffold template");

    let csproj_name = csproj
        .file_name()
        .context("csproj has no file name")?
        .to_string_lossy()
        .to_string();
    let abs_output_dir = output_dir.canonicalize().unwrap_or_else(|_| output_dir.to_path_buf());
    let cmd = format!(
        "dotnet pack {proj} --configuration Release -p:Version={version} --output {out}",
        proj = csproj_name,
        out = abs_output_dir.display()
    );
    crate::publish::run_shell_command_in(&cmd, &proj_dir)?;

    let nupkg = find_nupkg(&abs_output_dir, &namespace, version)?;
    let nupkg_name = nupkg
        .file_name()
        .context("nupkg has no name")?
        .to_string_lossy()
        .to_string();

    Ok(PackageArtifact {
        path: nupkg,
        name: nupkg_name,
        checksum: None,
    })
}

/// Return the NuGet RID for this target.
fn csharp_rid(config: &ResolvedCrateConfig, target: &RustTarget) -> String {
    if let Some(publish) = &config.publish {
        if let Some(lang_cfg) = publish.languages.get("csharp") {
            if let Some(override_rid) = &lang_cfg.csharp_rid {
                return override_rid.clone();
            }
        }
    }
    target.platform_for(crate::core::config::extras::Language::Csharp)
}

fn find_csproj(workspace_root: &Path, pkg_dir: &str, namespace: &str) -> Result<PathBuf> {
    let candidate = workspace_root
        .join(pkg_dir)
        .join(namespace)
        .join(format!("{namespace}.csproj"));
    if candidate.exists() {
        return Ok(candidate);
    }
    let scan_dir = workspace_root.join(pkg_dir);
    if scan_dir.exists() {
        for entry in fs::read_dir(&scan_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                for inner in fs::read_dir(&path)? {
                    let inner = inner?;
                    let ip = inner.path();
                    if ip.extension().is_some_and(|e| e == "csproj") {
                        return Ok(ip);
                    }
                }
            }
            if path.extension().is_some_and(|e| e == "csproj") {
                return Ok(path);
            }
        }
    }
    anyhow::bail!("No .csproj found under {}", scan_dir.display())
}

fn find_nupkg(output_dir: &Path, namespace: &str, version: &str) -> Result<PathBuf> {
    let expected = output_dir.join(format!("{namespace}.{version}.nupkg"));
    if expected.exists() {
        return Ok(expected);
    }
    let candidates: Vec<PathBuf> = fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "nupkg"))
        .collect();
    candidates
        .into_iter()
        .next()
        .with_context(|| format!("no .nupkg for {namespace}-{version} found in {}", output_dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    fn minimal_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
[crates.csharp]
namespace = "MyLib"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn rid_linux_x64() {
        let config = minimal_config();
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(csharp_rid(&config, &t), "linux-x64");
    }

    #[test]
    fn rid_osx_arm64() {
        let config = minimal_config();
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(csharp_rid(&config, &t), "osx-arm64");
    }

    #[test]
    fn rid_win_x64() {
        let config = minimal_config();
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(csharp_rid(&config, &t), "win-x64");
    }

    #[test]
    fn rid_linux_musl_x64() {
        let config = minimal_config();
        let t = RustTarget::parse("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(csharp_rid(&config, &t), "linux-musl-x64");
    }

    #[test]
    fn rid_config_override() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
[crates.publish.languages.csharp]
csharp_rid = "linux-x64-custom"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(csharp_rid(&config, &t), "linux-x64-custom");
    }

    #[test]
    fn find_nupkg_expected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg = tmp.path().join("MyLib.1.0.0.nupkg");
        std::fs::write(&pkg, b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0").unwrap();
        assert_eq!(result, pkg);
    }

    #[test]
    fn find_nupkg_fallback_scan() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg = tmp.path().join("SomeOtherName.1.0.0.nupkg");
        std::fs::write(&pkg, b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0").unwrap();
        assert!(result.extension().unwrap() == "nupkg");
    }
}
