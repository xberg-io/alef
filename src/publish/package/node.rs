//! NAPI-RS Node.js native binding packager.
//!
//! Produces a per-platform npm sub-package directory and runs `npm pack` to
//! generate a tarball. The sub-package follows the `@scope/{name}-{platform}`
//! naming convention used by napi-rs.
//!
//! Platform list is read from `[publish.languages.node] npm_subpackage_platforms`
//! in alef.toml. When absent, a sensible default set is used.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions as tv;
use crate::publish::package::BuildProfile;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Default set of NAPI platform identifiers when the config is absent.
const DEFAULT_PLATFORMS: &[&str] = &[
    "linux-x64-gnu",
    "linux-arm64-gnu",
    "linux-x64-musl",
    "linux-arm64-musl",
    "darwin-x64",
    "darwin-arm64",
    "win32-x64-msvc",
    "win32-arm64-msvc",
];

/// Package a NAPI native binding for one target into a per-platform npm sub-package.
///
/// Produces: `{scope}-{name}-{platform}-{version}.tgz`
///
/// Steps:
/// 1. Locate the `.node` binary from `target/{triple}/release/` or `target/release/`.
/// 2. Create `output_dir/npm/{platform}/` with `package.json` + `.node` binary.
/// 3. Run `npm pack` inside that directory and move the `.tgz` to `output_dir`.
pub fn package_node(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let platform = target.platform_for(crate::core::config::extras::Language::Node);
    let node_pkg_name = config.node_package_name();
    let base_name = if let Some(slash_pos) = node_pkg_name.rfind('/') {
        &node_pkg_name[slash_pos + 1..]
    } else {
        node_pkg_name.as_str()
    };
    let scope = if node_pkg_name.starts_with('@') {
        let at_end = node_pkg_name.find('/').map(|i| &node_pkg_name[..i]);
        at_end.map(|s| s.to_string())
    } else {
        None
    };

    let node_crate = crate::publish::crate_name_from_output(config, crate::core::config::extras::Language::Node)
        .unwrap_or_else(|| format!("{}-node", config.name));
    let node_lib_name = format!("{}.{}.node", base_name, platform);
    let node_lib_simple = format!("{}.node", base_name.replace('-', "_"));

    let node_bin = find_node_binary(workspace_root, target, &node_crate, &node_lib_name, &node_lib_simple)?;

    let platform_dir = output_dir.join("npm").join(&platform);
    if platform_dir.exists() {
        fs::remove_dir_all(&platform_dir)?;
    }
    fs::create_dir_all(&platform_dir)?;

    let dest_bin_name = format!("{base_name}.{platform}.node");
    fs::copy(&node_bin, platform_dir.join(&dest_bin_name))
        .with_context(|| format!("copying .node binary to {}", platform_dir.display()))?;

    let sub_pkg_name = match &scope {
        Some(s) => format!("{s}/{base_name}-{platform}"),
        None => format!("{base_name}-{platform}"),
    };
    let (pkg_os, pkg_cpu, pkg_libc) = platform_to_os_cpu_libc(&platform);
    let metadata = package_metadata(config);
    let pkg_json = generate_sub_package_json(
        &sub_pkg_name,
        SubPackageJsonOptions {
            version,
            bin_file: &dest_bin_name,
            os: pkg_os,
            cpu: pkg_cpu,
            libc: pkg_libc,
            metadata: &metadata,
        },
    );
    fs::write(platform_dir.join("package.json"), pkg_json)?;

    let readme = format!("# {sub_pkg_name}\n\nNative binary for {platform}.\n");
    fs::write(platform_dir.join("README.md"), readme)?;

    crate::publish::run_shell_command_in("npm pack", &platform_dir)?;

    let tgz = find_tgz(&platform_dir).context("npm pack: no .tgz found")?;
    let tgz_name = tgz
        .file_name()
        .context("tgz has no name")?
        .to_string_lossy()
        .to_string();
    let tgz_dest = output_dir.join(&tgz_name);
    fs::rename(&tgz, &tgz_dest)?;

    Ok(PackageArtifact {
        path: tgz_dest,
        name: tgz_name,
        checksum: None,
    })
}

/// Return the configured npm subpackage platforms for Node, or the default set.
pub fn npm_platforms(config: &ResolvedCrateConfig) -> Vec<String> {
    if let Some(publish) = &config.publish
        && let Some(lang_cfg) = publish.languages.get("node")
        && let Some(platforms) = &lang_cfg.npm_subpackage_platforms
        && !platforms.is_empty()
    {
        return platforms.clone();
    }
    DEFAULT_PLATFORMS.iter().map(|s| s.to_string()).collect()
}

/// Map a napi platform string to (os, cpu, optional libc) for package.json fields.
fn platform_to_os_cpu_libc(platform: &str) -> (&'static str, &'static str, Option<&'static str>) {
    match platform {
        "linux-x64-gnu" => ("linux", "x64", Some("glibc")),
        "linux-x64-musl" => ("linux", "x64", Some("musl")),
        "linux-arm64-gnu" => ("linux", "arm64", Some("glibc")),
        "linux-arm64-musl" => ("linux", "arm64", Some("musl")),
        "darwin-x64" => ("darwin", "x64", None),
        "darwin-arm64" => ("darwin", "arm64", None),
        "win32-x64-msvc" => ("win32", "x64", None),
        "win32-arm64-msvc" => ("win32", "arm64", None),
        "linux-arm-gnueabihf" => ("linux", "arm", Some("glibc")),
        _ => ("linux", "x64", None),
    }
}

struct PackageMetadata {
    license: Option<String>,
    repository_url: Option<String>,
}

fn package_metadata(config: &ResolvedCrateConfig) -> PackageMetadata {
    let meta = crate::scaffold::scaffold_meta(config);
    let repository_url = meta.configured_repository.map(|repository| {
        if repository.starts_with("git+") {
            repository
        } else {
            format!("git+{}.git", repository.trim_end_matches('/').trim_end_matches(".git"))
        }
    });
    PackageMetadata {
        license: meta.license,
        repository_url,
    }
}

struct SubPackageJsonOptions<'a> {
    version: &'a str,
    bin_file: &'a str,
    os: &'a str,
    cpu: &'a str,
    libc: Option<&'a str>,
    metadata: &'a PackageMetadata,
}

fn generate_sub_package_json(name: &str, options: SubPackageJsonOptions) -> String {
    let SubPackageJsonOptions {
        version,
        bin_file,
        os,
        cpu,
        libc,
        metadata,
    } = options;
    let libc_field = if let Some(l) = libc {
        format!(",\n  \"libc\": [\"{l}\"]")
    } else {
        String::new()
    };
    let repository_field = metadata
        .repository_url
        .as_deref()
        .map(|url| {
            format!(
                r#",
  "repository": {{
    "type": "git",
    "url": "{url}"
  }}"#
            )
        })
        .unwrap_or_default();
    let license_field = metadata
        .license
        .as_deref()
        .map(|license| format!(",\n  \"license\": \"{license}\""))
        .unwrap_or_default();
    format!(
        r#"{{
  "name": "{name}",
  "version": "{version}"{license_field}{repository_field},
  "os": ["{os}"],
  "cpu": ["{cpu}"]{libc_field},
  "main": "{bin_file}",
  "files": ["{bin_file}"],
  "engines": {{ "node": "{node_engine}" }},
  "publishConfig": {{ "access": "public" }}
}}
"#,
        node_engine = tv::npm::NODE_ENGINE,
    )
}

/// Locate the compiled `.node` binary, preferring `primary_name` (the cross-compile-renamed form
/// napi-rs produces, e.g. `mylib.linux-x64-gnu.node`) over `fallback_name` (the simple form a
/// plain `napi build` produces, e.g. `mylib.node`) at each location tier in turn.
///
/// Delegates to [`crate::publish::package::find_built_artifact_any_with_extra_dirs`] for the
/// two canonical `target/{triple}/release/` / `target/release/` locations, plus
/// `crates/{node_crate}/`, which is where `napi build` writes its output directly rather than
/// into `target/` at all.
fn find_node_binary(
    workspace_root: &Path,
    target: &RustTarget,
    node_crate: &str,
    primary_name: &str,
    fallback_name: &str,
) -> Result<PathBuf> {
    let in_crate_dir = workspace_root.join("crates").join(node_crate);
    crate::publish::package::find_built_artifact_any_with_extra_dirs(
        workspace_root,
        target,
        &[primary_name, fallback_name],
        BuildProfile::Release,
        std::slice::from_ref(&in_crate_dir),
    )
    .with_context(|| {
        format!(
            ".node binary not found for target {}. Expected '{primary_name}' or '{fallback_name}' in target \
             dirs or crates/{node_crate}/",
            target.triple
        )
    })
}

fn find_tgz(dir: &Path) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tgz"))
        .collect();
    candidates.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    candidates
        .into_iter()
        .next_back()
        .with_context(|| format!("no .tgz found in {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn minimal_config() -> ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["node"]
[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "@myorg/my-lib"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn platform_to_os_cpu_linux_gnu() {
        let (os, cpu, libc) = platform_to_os_cpu_libc("linux-x64-gnu");
        assert_eq!(os, "linux");
        assert_eq!(cpu, "x64");
        assert_eq!(libc, Some("glibc"));
    }

    #[test]
    fn platform_to_os_cpu_darwin() {
        let (os, cpu, libc) = platform_to_os_cpu_libc("darwin-arm64");
        assert_eq!(os, "darwin");
        assert_eq!(cpu, "arm64");
        assert!(libc.is_none());
    }

    #[test]
    fn sub_package_json_has_required_fields() {
        let json = generate_sub_package_json(
            "@scope/foo-linux-x64-gnu",
            SubPackageJsonOptions {
                version: "1.0.0",
                bin_file: "foo.linux-x64-gnu.node",
                os: "linux",
                cpu: "x64",
                libc: Some("glibc"),
                metadata: &PackageMetadata {
                    license: Some("MIT".to_string()),
                    repository_url: Some("git+https://github.com/scope/foo.git".to_string()),
                },
            },
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], "@scope/foo-linux-x64-gnu");
        assert_eq!(parsed["version"], "1.0.0");
        assert_eq!(parsed["license"], "MIT");
        assert_eq!(parsed["repository"]["url"], "git+https://github.com/scope/foo.git");
        assert_eq!(parsed["publishConfig"]["access"], "public");
        assert!(parsed["os"].is_array());
        assert!(parsed["cpu"].is_array());
        assert!(parsed["libc"].is_array());
    }

    #[test]
    fn platform_to_os_cpu_musl_sets_libc() {
        let (os, cpu, libc) = platform_to_os_cpu_libc("linux-arm64-musl");
        assert_eq!(os, "linux");
        assert_eq!(cpu, "arm64");
        assert_eq!(libc, Some("musl"));
    }

    #[test]
    fn platform_to_os_cpu_win32_arm64() {
        let (os, cpu, libc) = platform_to_os_cpu_libc("win32-arm64-msvc");
        assert_eq!(os, "win32");
        assert_eq!(cpu, "arm64");
        assert!(libc.is_none());
    }

    #[test]
    fn default_npm_platforms_nonempty() {
        let config = minimal_config();
        let platforms = npm_platforms(&config);
        assert!(!platforms.is_empty());
        assert!(platforms.contains(&"win32-arm64-msvc".to_string()));
    }

    #[test]
    fn config_npm_platforms_override() {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["node"]
[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
[crates.publish.languages.node]
npm_subpackage_platforms = ["linux-x64-gnu", "darwin-arm64"]
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let platforms = npm_platforms(&config);
        assert_eq!(platforms, vec!["linux-x64-gnu", "darwin-arm64"]);
    }

    #[test]
    fn find_node_binary_cross_path() {
        let tmp = TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let bin_dir = tmp.path().join("target/x86_64-unknown-linux-gnu/release");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("my-lib.x64-linux-gnu.node"), b"fake").unwrap();

        let fallback_dir = tmp.path().join("target/x86_64-unknown-linux-gnu/release");
        std::fs::write(fallback_dir.join("my_lib.node"), b"fake").unwrap();

        let result = find_node_binary(
            tmp.path(),
            &target,
            "my-lib-node",
            "my-lib.x64-linux-gnu.node",
            "my_lib.node",
        )
        .unwrap();
        assert!(result.exists());
    }

    /// The in-crate fallback this rewrite preserves: `napi build` writes its output directly into
    /// the napi crate's own directory rather than into `target/` at all, so neither canonical
    /// uplifted location will ever contain it. This is the exact case
    /// `find_built_artifact`/`find_built_artifact_with_extra_dirs` with an empty `extra_dirs`
    /// would miss.
    #[test]
    fn find_node_binary_falls_back_to_in_crate_output() {
        let tmp = TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let in_crate_dir = tmp.path().join("crates/my-lib-node");
        std::fs::create_dir_all(&in_crate_dir).unwrap();
        std::fs::write(in_crate_dir.join("my_lib.node"), b"in-crate-bytes").unwrap();

        let result = find_node_binary(
            tmp.path(),
            &target,
            "my-lib-node",
            "my-lib.x64-linux-gnu.node",
            "my_lib.node",
        )
        .unwrap();
        assert_eq!(
            result,
            in_crate_dir.join("my_lib.node"),
            "expected the in-crate fallback at {}, got {}",
            in_crate_dir.join("my_lib.node").display(),
            result.display()
        );
    }

    /// Tier priority must win over name priority: a `fallback_name` artifact at the
    /// higher-priority cross-compile tier must be found even when a `primary_name` artifact also
    /// exists at the lower-priority in-crate tier -- proves the delegation to
    /// `find_built_artifact_any_with_extra_dirs` searches tier-then-name, not name-then-tier
    /// (which would wrongly prefer the in-crate primary_name copy here).
    #[test]
    fn find_node_binary_searches_tier_before_name() {
        let tmp = TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        let cross_dir = tmp.path().join("target/x86_64-unknown-linux-gnu/release");
        std::fs::create_dir_all(&cross_dir).unwrap();
        std::fs::write(cross_dir.join("my_lib.node"), b"cross-fallback-name").unwrap();

        let in_crate_dir = tmp.path().join("crates/my-lib-node");
        std::fs::create_dir_all(&in_crate_dir).unwrap();
        std::fs::write(in_crate_dir.join("my-lib.x64-linux-gnu.node"), b"in-crate-primary-name").unwrap();

        let result = find_node_binary(
            tmp.path(),
            &target,
            "my-lib-node",
            "my-lib.x64-linux-gnu.node",
            "my_lib.node",
        )
        .unwrap();
        assert_eq!(
            result,
            cross_dir.join("my_lib.node"),
            "the higher-priority cross tier must win over a lower-priority tier even under a preferred name; \
             expected {}, got {}",
            cross_dir.join("my_lib.node").display(),
            result.display()
        );
    }

    /// `primary_name` must be tried before `fallback_name` at every location tier -- proves the
    /// name-preference order survived the delegation to `find_built_artifact_any_with_extra_dirs`.
    #[test]
    fn find_node_binary_prefers_primary_name_over_fallback_name() {
        let tmp = TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let release_dir = tmp.path().join("target/x86_64-unknown-linux-gnu/release");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join("my-lib.x64-linux-gnu.node"), b"primary").unwrap();
        std::fs::write(release_dir.join("my_lib.node"), b"fallback").unwrap();

        let result = find_node_binary(
            tmp.path(),
            &target,
            "my-lib-node",
            "my-lib.x64-linux-gnu.node",
            "my_lib.node",
        )
        .unwrap();
        assert_eq!(result, release_dir.join("my-lib.x64-linux-gnu.node"));
    }
}
