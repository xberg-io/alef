//! Zig package — archives the source code + FFI shared library for distribution.

use super::PackageArtifact;
use super::util::copy_dir_recursive;
use crate::core::config::ResolvedCrateConfig;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Package Zig bindings as a source distribution with bundled FFI library.
///
/// Produces: `{name}-zig-v{version}-{platform}.tar.gz` containing:
/// - `src/` — Zig source code
/// - `lib/` — FFI shared library (.so/.dylib)
/// - `include/` — C header
/// - `build.zig`, `build.zig.zon` — Zig build files
pub fn package_zig(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let header_name = config.ffi_header_name();
    let module_name = config.zig_module_name();
    let crate_name = &config.name;
    let pkg_dir = config.package_dir(crate::core::config::extras::Language::Zig);
    let platform = target.platform_for(crate::core::config::extras::Language::Zig);

    let pkg_name = format!("{crate_name}-zig-v{version}-{platform}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    let pkg_src = workspace_root.join(&pkg_dir);
    if !pkg_src.exists() {
        anyhow::bail!("Zig package directory not found: {}", pkg_dir);
    }

    copy_dir_recursive(&pkg_src, &staging).context("copying Zig package")?;

    let lib_dir = staging.join("lib");
    let include_dir = staging.join("include");
    fs::create_dir_all(&lib_dir)?;
    fs::create_dir_all(&include_dir)?;

    let shared_lib = target.shared_lib_name(&lib_name);
    let shared_src = super::find_built_artifact(workspace_root, target, &shared_lib)
        .with_context(|| format!("locating built FFI artifact `{shared_lib}` for Zig package"))?;
    let shared_dst = lib_dir.join(&shared_lib);
    fs::copy(&shared_src, &shared_dst).context("copying FFI .so into Zig package")?;

    super::util::fix_macos_dylib_id(target, &shared_dst, &shared_lib)?;

    let ffi_crate_dir = crate::publish::ffi_stage::find_ffi_crate_dir_pub(config, workspace_root);
    let header_src = ffi_crate_dir.join("include").join(&header_name);
    if !header_src.exists() {
        anyhow::bail!(
            "FFI C header not found at {} — run `alef build --lang=ffi` first",
            header_src.display()
        );
    }
    fs::copy(&header_src, include_dir.join(&header_name)).context("copying FFI header into Zig package")?;

    add_bundled_paths_to_manifest(&staging.join("build.zig.zon"))?;

    fs::write(
        staging.join("build.zig"),
        render_distributable_build_zig(&module_name, &lib_name, config),
    )
    .context("writing distributable build.zig into Zig package")?;

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

/// Render the `build.zig` shipped inside the distributed Zig tarball.
///
/// Unlike the in-tree `packages/zig/build.zig` (which links the FFI library from
/// the Cargo workspace `target/` dir for local development), this build script
/// links the prebuilt shared library and C header bundled in the package's own
/// `lib/` and `include/` directories — resolved package-relative via `b.path`,
/// so they work from the global Zig cache when consumed via `zig fetch`. It
/// exports the `{module_name}` module; a consumer links it with
/// `b.dependency("<pkg>", .{ ... }).module("{module_name}")`.
fn render_distributable_build_zig(module_name: &str, ffi_lib_name: &str, config: &ResolvedCrateConfig) -> String {
    let capsule_imports_block: String = config
        .zig
        .as_ref()
        .map(|c| {
            let import_names = crate::core::config::languages::zig_capsule_import_names(&c.capsule_types);
            let mut block = String::new();
            for name in &import_names {
                block.push_str(&format!(
                    "    const {name}_dep = b.dependency(\"{name}\", .{{\n        \
                     .target = target,\n        .optimize = optimize,\n    }});\n    \
                     module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n"
                ));
            }
            block
        })
        .unwrap_or_default();
    format!(
        r#"const std = @import("std");

// alef-generated for distribution. The prebuilt FFI library (lib/) and C header
// (include/) ship inside this package; link them package-relative so consumers
// resolve the native library from the fetched package itself.
pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    const module = b.addModule("{module_name}", .{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    module.addLibraryPath(b.path("lib"));
    module.addIncludePath(b.path("include"));
    module.linkSystemLibrary("{ffi_lib_name}", .{{}});
{capsule_imports_block}}}
"#
    )
}

/// Insert the bundled `lib` and `include` directories into a `build.zig.zon`
/// `.paths` allowlist so a fetched consumer can resolve the prebuilt FFI library
/// and header via `b.path("lib")` / `b.path("include")`.
///
/// Idempotent for a freshly staged manifest: the scaffolded source never lists
/// these directories, so the entries are added exactly once per package run.
fn add_bundled_paths_to_manifest(manifest: &Path) -> Result<()> {
    let zon = fs::read_to_string(manifest).context("reading staged build.zig.zon")?;
    const MARKER: &str = ".paths = .{";
    let Some(pos) = zon.find(MARKER) else {
        anyhow::bail!("build.zig.zon is missing a `.paths` block: {}", manifest.display());
    };
    if zon.contains("\"lib\"") && zon.contains("\"include\"") {
        return Ok(());
    }
    let insert_at = pos + MARKER.len();
    let mut patched = String::with_capacity(zon.len() + 32);
    patched.push_str(&zon[..insert_at]);
    patched.push_str("\n        \"lib\",\n        \"include\",");
    patched.push_str(&zon[insert_at..]);
    fs::write(manifest, patched).context("writing patched build.zig.zon")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn config_no_capsule() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample-lib"
sources = []
"#,
        )
    }

    #[test]
    fn distributable_build_zig_links_bundled_lib() {
        let s = render_distributable_build_zig("sample_router", "sample_router_ffi", &config_no_capsule());
        assert!(
            s.contains("b.addModule(\"sample_router\""),
            "must export the module:\n{s}"
        );
        assert!(
            s.contains("module.addLibraryPath(b.path(\"lib\"))"),
            "must link bundled lib/:\n{s}"
        );
        assert!(
            s.contains("module.addIncludePath(b.path(\"include\"))"),
            "must add bundled include/:\n{s}"
        );
        assert!(
            s.contains("module.linkSystemLibrary(\"sample_router_ffi\""),
            "must link the FFI lib:\n{s}"
        );
        assert!(
            s.contains(".link_libc = true"),
            "must link libc for FFI header symbols:\n{s}"
        );
        assert!(!s.contains("cwd_relative"), "must not use cwd_relative paths:\n{s}");
        assert!(
            !s.contains("../../target/release"),
            "must not reference the workspace target dir:\n{s}"
        );
    }

    #[test]
    fn bundled_paths_added_to_manifest_idempotently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = dir.path().join("build.zig.zon");
        fs::write(
            &manifest,
            ".{\n    .name = .sample_router,\n    .paths = .{\n        \"build.zig\",\n        \"src\",\n    },\n}\n",
        )
        .expect("write manifest");

        add_bundled_paths_to_manifest(&manifest).expect("first patch");
        let once = fs::read_to_string(&manifest).expect("read");
        assert!(once.contains("\"lib\""), "lib added:\n{once}");
        assert!(once.contains("\"include\""), "include added:\n{once}");

        add_bundled_paths_to_manifest(&manifest).expect("second patch");
        let twice = fs::read_to_string(&manifest).expect("read");
        assert_eq!(
            once.matches("\"lib\"").count(),
            twice.matches("\"lib\"").count(),
            "second call must be a no-op"
        );
    }

    #[test]
    fn packaged_tarball_includes_rewritten_build_zig_and_ffis() {
        let s = render_distributable_build_zig("sample_lib", "sample_lib_ffi", &config_no_capsule());

        assert!(
            !s.contains("../../target/release"),
            "rewritten build.zig must not reference workspace target dir:\n{s}"
        );
        assert!(
            !s.contains("../../crates/sample-lib-ffi"),
            "rewritten build.zig must not reference workspace crate dir:\n{s}"
        );
        assert!(
            !s.contains("cwd_relative"),
            "rewritten build.zig must use package-relative paths only:\n{s}"
        );

        assert!(s.contains("b.path(\"lib\")"), "must link bundled lib/ directory:\n{s}");
        assert!(
            s.contains("b.path(\"include\")"),
            "must link bundled include/ directory:\n{s}"
        );

        assert!(s.contains(".link_libc = true"), "must enable libc linking:\n{s}");
    }

    #[test]
    fn distributable_build_zig_wires_capsule_imports() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample-lib"
sources = []

[crates.zig.capsule_types.Language]
host_type = "?*const tree_sitter.Language"
package = "https://github.com/tree-sitter/zig-tree-sitter/archive/refs/tags/v0.26.0.tar.gz"
package_version = "tree_sitter-0.26.0-deadbeef"
"#,
        );
        let s = render_distributable_build_zig("sample_lib", "sample_lib_ffi", &config);

        assert!(
            s.contains("b.dependency(\"tree_sitter\""),
            "distributable build.zig must resolve the capsule dependency:\n{s}"
        );
        assert!(
            s.contains("module.addImport(\"tree_sitter\", tree_sitter_dep.module(\"tree_sitter\"))"),
            "distributable build.zig must import the capsule module:\n{s}"
        );
        assert!(
            !s.contains("test_module"),
            "distributable build.zig must not reference a test module:\n{s}"
        );
    }
}
