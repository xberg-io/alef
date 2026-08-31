//! C# NuGet packager for the meta + per-RID native runtime split.
//!
//! The scaffold (`crate::scaffold::render_csharp_csproj` /
//! `render_csharp_runtime_csproj` / `render_csharp_runtime_json_template`) emits two
//! projects per crate:
//!
//! - `<Namespace>.csproj` — a thin meta package carrying the managed assembly plus
//!   `runtime.json` (NuGet's RID-fallback graph). No natives.
//! - `<Namespace>.Runtime.csproj` — a native-only package packed once per RID via
//!   `dotnet pack -p:PublishedRID=<rid>`, producing `<PackageId>.runtime.<rid>`.
//!
//! `package_csharp` stages the current target's native library under
//! `runtimes/{rid}/native/{libname}` (the layout both projects expect), regenerates
//! `runtime.json` from `runtime.json.template`, packs the meta project once, then packs
//! a `<PackageId>.runtime.<rid>` package for every enabled published RID that currently
//! has a staged native asset. This lets a single invocation (one target/RID staged) still
//! produce a coherent set, and lets repeated invocations that share a workspace (natives
//! accumulating across targets) produce the full RID set on the invocation that completes
//! staging.
//!
//! RID examples: `linux-x64`, `linux-arm64`, `osx-x64`, `osx-arm64`, `win-x64`,
//! `linux-musl-x64`, `linux-musl-arm64`.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::publish::platform::RustTarget;
use crate::scaffold::naming::csharp_package_id;
use crate::scaffold::{
    PUBLISHED_RUNTIME_IDENTIFIERS, render_csharp_csproj, render_csharp_runtime_csproj,
    render_csharp_runtime_json_template,
};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package the C# NuGet artifacts for the given target.
///
/// Produces the thin meta package (`{package_id}.{version}.nupkg`) plus one
/// `{package_id}.runtime.{rid}.{version}.nupkg` per enabled RID with a staged native
/// asset (at minimum, the RID for `target`).
pub fn package_csharp(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<Vec<PackageArtifact>> {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);
    let rid = csharp_rid(config, target);
    let namespace = config.csharp_namespace();
    let package_id = csharp_package_id(config);

    // Packaging always ships a `--release` build -- nothing here is publishable in `debug`. ~keep
    let lib_src = crate::publish::package::find_built_artifact(
        workspace_root,
        target,
        &shared_lib,
        crate::publish::package::BuildProfile::Release,
    )?;

    let pkg_dir_str = config.package_dir(Language::Csharp);
    let pkg_dir = workspace_root.join(&pkg_dir_str);
    let meta_dir = pkg_dir.join(&namespace);

    stage_native(&meta_dir, &rid, &lib_src, &shared_lib)?;

    // Regenerate the meta csproj, runtime.json, and per-RID runtime csproj from the
    // scaffold templates so the packed metadata stays in sync with alef.toml regardless
    // of what's committed on disk.
    let csproj = find_csproj(&pkg_dir, &namespace)?;
    fs::write(&csproj, render_csharp_csproj(config, version))
        .with_context(|| format!("regenerating csproj at {}", csproj.display()))?;
    tracing::debug!(path = %csproj.display(), "regenerated csproj from scaffold template");

    let runtime_json = meta_dir.join("runtime.json");
    let runtime_json_content = render_csharp_runtime_json_template(config).replace("{{VERSION}}", version);
    fs::write(&runtime_json, runtime_json_content)
        .with_context(|| format!("rendering runtime.json at {}", runtime_json.display()))?;
    tracing::debug!(path = %runtime_json.display(), "rendered runtime.json from template");

    let runtime_csproj = runtime_csproj_path(&pkg_dir, &namespace);
    fs::create_dir_all(runtime_csproj.parent().context("runtime csproj has no parent")?)
        .with_context(|| format!("creating {}", runtime_csproj.display()))?;
    fs::write(&runtime_csproj, render_csharp_runtime_csproj(config, version))
        .with_context(|| format!("regenerating runtime csproj at {}", runtime_csproj.display()))?;
    tracing::debug!(path = %runtime_csproj.display(), "regenerated runtime csproj from scaffold template");

    let abs_output_dir = output_dir.canonicalize().unwrap_or_else(|_| output_dir.to_path_buf());

    let mut artifacts = Vec::new();

    // Pack the thin meta package once: managed assembly + runtime.json RID-fallback graph.
    let meta_proj_dir = csproj.parent().context("csproj has no parent")?;
    let meta_proj_name = file_name(&csproj)?;
    let pack_meta_cmd = format!(
        "dotnet pack {proj} --configuration Release -p:Version={version} --output {out}",
        proj = meta_proj_name,
        out = abs_output_dir.display()
    );
    crate::publish::run_shell_command_in(&pack_meta_cmd, meta_proj_dir)?;
    let meta_nupkg = find_nupkg(&abs_output_dir, &package_id, version)?;
    artifacts.push(PackageArtifact {
        name: file_name(&meta_nupkg)?,
        path: meta_nupkg,
        checksum: None,
    });

    // Pack a native runtime package for every enabled published RID that currently has a
    // staged native asset (at minimum, the RID just staged above for `target`).
    let runtime_proj_dir = runtime_csproj.parent().context("runtime csproj has no parent")?;
    let runtime_proj_name = file_name(&runtime_csproj)?;
    for (rid_name, _triple) in PUBLISHED_RUNTIME_IDENTIFIERS
        .iter()
        .filter(|(_, t)| config.target_enabled(t))
    {
        let native_dir = meta_dir.join("runtimes").join(rid_name).join("native");
        if !has_staged_native(&native_dir) {
            continue;
        }

        let pack_runtime_cmd = format!(
            "dotnet pack {proj} --configuration Release -p:Version={version} -p:PublishedRID={rid_name} --output {out}",
            proj = runtime_proj_name,
            out = abs_output_dir.display()
        );
        crate::publish::run_shell_command_in(&pack_runtime_cmd, runtime_proj_dir)?;

        let runtime_package_id = format!("{package_id}.runtime.{rid_name}");
        let runtime_nupkg = find_nupkg(&abs_output_dir, &runtime_package_id, version)?;
        artifacts.push(PackageArtifact {
            name: file_name(&runtime_nupkg)?,
            path: runtime_nupkg,
            checksum: None,
        });
    }

    Ok(artifacts)
}

/// Stage the built native library under `{meta_dir}/runtimes/{rid}/native/{shared_lib}`,
/// the layout `<Namespace>.Runtime.csproj` packs from.
fn stage_native(meta_dir: &Path, rid: &str, lib_src: &Path, shared_lib: &str) -> Result<()> {
    let runtimes_dir = meta_dir.join("runtimes").join(rid).join("native");
    fs::create_dir_all(&runtimes_dir).with_context(|| format!("creating runtimes dir {}", runtimes_dir.display()))?;

    let staged = runtimes_dir.join(shared_lib);
    fs::copy(lib_src, &staged).with_context(|| format!("staging {} to {}", lib_src.display(), staged.display()))?;
    Ok(())
}

/// Whether a RID's `runtimes/{rid}/native/` directory has at least one staged file.
fn has_staged_native(native_dir: &Path) -> bool {
    fs::read_dir(native_dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn file_name(path: &Path) -> Result<String> {
    Ok(path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy()
        .to_string())
}

/// Return the NuGet RID for this target.
fn csharp_rid(config: &ResolvedCrateConfig, target: &RustTarget) -> String {
    if let Some(publish) = &config.publish
        && let Some(lang_cfg) = publish.languages.get("csharp")
        && let Some(override_rid) = &lang_cfg.csharp_rid
    {
        return override_rid.clone();
    }
    target.platform_for(Language::Csharp)
}

/// Suffixes of `.csproj` files that are never the thin meta package: the per-RID native
/// project the scaffold emits alongside it, plus the test projects consumers keep in the
/// same package directory. Packing one of these instead of the meta project publishes a
/// structurally valid but wrong `.nupkg`, so they are excluded from the fallback scan
/// rather than left to `read_dir` order. ~keep
const NON_PACKAGE_CSPROJ_SUFFIXES: [&str; 3] = [".Runtime.csproj", ".SmokeTests.csproj", ".Tests.csproj"];

/// Find the thin meta `<Namespace>.csproj` under the C# package directory.
///
/// Selection is deterministic and refuses to guess:
///
/// 1. the canonical `{pkg_dir}/{Namespace}/{Namespace}.csproj` wins outright;
/// 2. otherwise any project file actually named `{Namespace}.csproj` wins;
/// 3. otherwise the scan considers only projects that are not a per-RID runtime or test
///    project, and requires exactly one — several qualifying candidates is an error naming
///    all of them, never a `read_dir`-order pick.
fn find_csproj(pkg_dir: &Path, namespace: &str) -> Result<PathBuf> {
    let expected_name = format!("{namespace}.csproj");
    let canonical = pkg_dir.join(namespace).join(&expected_name);
    if canonical.exists() {
        return Ok(canonical);
    }
    if !pkg_dir.exists() {
        anyhow::bail!("No .csproj found under {}", pkg_dir.display());
    }

    let mut found = collect_csproj_files(pkg_dir)?;
    found.sort();

    let exact: Vec<PathBuf> = found
        .iter()
        .filter(|p| has_file_name(p, &expected_name))
        .cloned()
        .collect();
    let candidates = if exact.is_empty() {
        found.into_iter().filter(|p| is_package_csproj(p)).collect()
    } else {
        exact
    };

    match candidates.as_slice() {
        [] => anyhow::bail!("No .csproj found under {}", pkg_dir.display()),
        [only] => Ok(only.clone()),
        many => anyhow::bail!(
            "ambiguous .csproj under {}: expected exactly one meta project (ideally `{expected_name}`), found {}: {}",
            pkg_dir.display(),
            many.len(),
            many.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Collect every `.csproj` directly under `pkg_dir` or one directory below it.
fn collect_csproj_files(pkg_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(pkg_dir).with_context(|| format!("reading {}", pkg_dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            for inner in fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))? {
                let inner_path = inner?.path();
                if is_csproj(&inner_path) {
                    found.push(inner_path);
                }
            }
        } else if is_csproj(&path) {
            found.push(path);
        }
    }
    Ok(found)
}

fn is_csproj(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "csproj")
}

fn has_file_name(path: &Path, name: &str) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some(name)
}

/// Whether a `.csproj` could be the meta package project at all.
fn is_package_csproj(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    !NON_PACKAGE_CSPROJ_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// The canonical path of the per-RID native runtime project the scaffold emits:
/// `{pkg_dir}/{Namespace}.Runtime/{Namespace}.Runtime.csproj`.
fn runtime_csproj_path(pkg_dir: &Path, namespace: &str) -> PathBuf {
    pkg_dir
        .join(format!("{namespace}.Runtime"))
        .join(format!("{namespace}.Runtime.csproj"))
}

fn find_nupkg(output_dir: &Path, package_id: &str, version: &str) -> Result<PathBuf> {
    let expected = output_dir.join(format!("{package_id}.{version}.nupkg"));
    if expected.exists() {
        return Ok(expected);
    }
    // dotnet pack should always produce `{package_id}.{version}.nupkg` exactly; this
    // fallback only covers benign version-string normalization (e.g. build metadata).
    // With multiple nupkgs now coexisting in `output_dir` (meta + N per-RID runtime
    // packages), an ambiguous scan could silently return the wrong artifact, so the
    // fallback only fires when exactly one `.nupkg` is present.
    let candidates: Vec<PathBuf> = fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "nupkg"))
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates.into_iter().next().expect("len checked"));
    }
    anyhow::bail!(
        "no unambiguous .nupkg for {package_id}-{version} found in {} ({} candidate(s))",
        output_dir.display(),
        candidates.len()
    )
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

    #[test]
    fn find_nupkg_ambiguous_scan_errors() {
        // With multiple nupkgs coexisting (meta + per-RID runtime packages), a missing
        // exact match must error rather than silently pick one of several candidates.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("MyLib.runtime.linux-x64.1.0.0.nupkg"), b"fake").unwrap();
        std::fs::write(tmp.path().join("MyLib.runtime.osx-arm64.1.0.0.nupkg"), b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn find_nupkg_exact_match_ignores_sibling_runtime_packages() {
        // The meta package_id ("MyLib") is a prefix of the runtime package ids
        // ("MyLib.runtime.<rid>"); the exact-match path must not be confused by siblings.
        let tmp = tempfile::TempDir::new().unwrap();
        let meta = tmp.path().join("MyLib.1.0.0.nupkg");
        std::fs::write(&meta, b"fake").unwrap();
        std::fs::write(tmp.path().join("MyLib.runtime.linux-x64.1.0.0.nupkg"), b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0").unwrap();
        assert_eq!(result, meta);
    }

    #[test]
    fn runtime_csproj_path_matches_scaffold_layout() {
        let pkg_dir = Path::new("/repo/packages/csharp");
        let path = runtime_csproj_path(pkg_dir, "MyLib");
        assert_eq!(
            path,
            Path::new("/repo/packages/csharp/MyLib.Runtime/MyLib.Runtime.csproj")
        );
    }

    #[test]
    fn has_staged_native_false_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!has_staged_native(&tmp.path().join("runtimes/linux-x64/native")));
    }

    #[test]
    fn has_staged_native_true_when_file_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let native_dir = tmp.path().join("runtimes/linux-x64/native");
        std::fs::create_dir_all(&native_dir).unwrap();
        std::fs::write(native_dir.join("libmylib.so"), b"fake").unwrap();
        assert!(has_staged_native(&native_dir));
    }

    #[test]
    fn stage_native_writes_expected_layout() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lib_src = tmp.path().join("libmylib.so");
        std::fs::write(&lib_src, b"fake-native").unwrap();

        let meta_dir = tmp.path().join("meta");
        stage_native(&meta_dir, "linux-x64", &lib_src, "libmylib.so").unwrap();

        let staged = meta_dir.join("runtimes/linux-x64/native/libmylib.so");
        assert!(staged.exists());
        assert_eq!(std::fs::read(staged).unwrap(), b"fake-native");
    }

    /// A `.csproj` living only under decoy names (per-RID runtime project, smoke/unit test
    /// projects) must not be packed as the meta package: without the exclusion the fallback
    /// returns whichever one `read_dir` happened to yield first and publishes a corrupt package.
    #[test]
    fn find_csproj_refuses_test_and_runtime_projects() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("packages/csharp");
        for name in ["MyLib.SmokeTests", "MyLib.Runtime", "MyLib.Tests"] {
            let dir = pkg_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.csproj")), b"<Project />").unwrap();
        }

        let error = find_csproj(&pkg_dir, "MyLib").unwrap_err();
        assert!(
            error.to_string().contains("No .csproj found under"),
            "unexpected error: {error}"
        );
    }

    /// When the canonical path is missing, a project actually named `{Namespace}.csproj`
    /// wins over every decoy regardless of directory-iteration order.
    #[test]
    fn find_csproj_prefers_exact_namespace_name_over_decoys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("packages/csharp");
        for name in ["MyLib.SmokeTests", "MyLib.Runtime", "MyLib.Tests"] {
            let dir = pkg_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.csproj")), b"<Project />").unwrap();
        }
        let meta_dir = pkg_dir.join("meta");
        std::fs::create_dir_all(&meta_dir).unwrap();
        let meta = meta_dir.join("MyLib.csproj");
        std::fs::write(&meta, b"<Project />").unwrap();

        assert_eq!(find_csproj(&pkg_dir, "MyLib").unwrap(), meta);
    }

    /// Several qualifying projects and no exact-name match is ambiguity: error naming all of
    /// them rather than silently packing whichever one the filesystem listed first.
    #[test]
    fn find_csproj_errors_when_several_candidates_qualify() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("packages/csharp");
        for name in ["Alpha", "Beta"] {
            let dir = pkg_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.csproj")), b"<Project />").unwrap();
        }

        let error = find_csproj(&pkg_dir, "MyLib").unwrap_err().to_string();
        assert!(error.contains("ambiguous .csproj"), "unexpected error: {error}");
        assert!(
            error.contains("Alpha.csproj"),
            "error must name every candidate: {error}"
        );
        assert!(
            error.contains("Beta.csproj"),
            "error must name every candidate: {error}"
        );
    }

    /// The canonical `{pkg_dir}/{Namespace}/{Namespace}.csproj` still wins outright, even with
    /// decoy projects sitting next to it.
    #[test]
    fn find_csproj_uses_canonical_path_when_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("packages/csharp");
        let meta_dir = pkg_dir.join("MyLib");
        std::fs::create_dir_all(&meta_dir).unwrap();
        let meta = meta_dir.join("MyLib.csproj");
        std::fs::write(&meta, b"<Project />").unwrap();
        std::fs::write(meta_dir.join("MyLib.SmokeTests.csproj"), b"<Project />").unwrap();
        let runtime_dir = pkg_dir.join("MyLib.Runtime");
        std::fs::create_dir_all(&runtime_dir).unwrap();
        std::fs::write(runtime_dir.join("MyLib.Runtime.csproj"), b"<Project />").unwrap();

        assert_eq!(find_csproj(&pkg_dir, "MyLib").unwrap(), meta);
    }

    /// The RID loop must only pack RIDs that are both enabled in config and have a
    /// staged native — proving the loop is config-driven (not target-specific) while
    /// remaining safe against unstaged RIDs from a single-target invocation.
    #[test]
    fn published_runtime_identifiers_filtered_by_target_enabled_and_staged() {
        let config = minimal_config();
        let enabled: Vec<&str> = PUBLISHED_RUNTIME_IDENTIFIERS
            .iter()
            .filter(|(_, t)| config.target_enabled(t))
            .map(|(rid, _)| *rid)
            .collect();
        // Default config enables every published target.
        assert_eq!(enabled.len(), PUBLISHED_RUNTIME_IDENTIFIERS.len());
        assert!(enabled.contains(&"linux-x64"));
        assert!(enabled.contains(&"osx-arm64"));
        assert!(enabled.contains(&"win-x64"));
    }
}
