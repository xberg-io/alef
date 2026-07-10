use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::sync::LazyLock;
use tracing::{debug, info, warn};

use super::helpers::{run_command, run_optional};
use super::version_core::{
    bump_version, patch_workspace_dep_versions, read_version, to_pep440, write_version_to_cargo_toml,
};
use super::version_python::sync_python_versions;
use super::version_regen::{regenerate_readmes, regenerate_scaffold_after_sync, regenerate_test_apps_after_sync};
use super::version_registry::sync_registry_package_versions;
use super::version_swift::{precompute_swift_checksum, sync_swift_package_versions};
use super::version_text::{
    read_workspace_license, remove_stale_kotlin_android_plugin, render_citation_cff, replace_citation_version,
    replace_gradle_project_version, replace_version_pattern, restore_gleam_dep_ranges, sync_cargo_lock_path_versions,
    sync_docs_version_badges, sync_e2e_dart_pubspec_lock, sync_e2e_go_mod, sync_e2e_java_pom, sync_gemfile_lock,
    sync_swift_binary_release_url,
};
use super::version_workspace::sync_workspace_cargo_toml_versions;
use crate::core::version::{to_r_version, to_rubygems_prerelease};

/// Regex for matching semantic version strings.
static SEMVER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\d+\.\d+\.\d+(-[a-zA-Z0-9._]+)*").expect("valid regex"));

/// Sync version from Cargo.toml to all package manifest files.
///
/// When `no_regen` is `false` (the default for direct CLI invocations), this
/// function automatically regenerates `test_apps/` scaffold files after updating
/// `[crates.e2e.registry.packages.*].version` in `alef.toml`, so the version
/// pins in generated files (pyproject.toml, mix.exs, build.zig.zon, Package.swift,
/// etc.) always match the workspace version atomically.
///
/// Pass `no_regen = true` to opt out of the automatic regeneration — useful when
/// this function is called from within another codegen pass that owns `test_apps/`
/// itself (e.g. `alef generate`).
pub fn sync_versions(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
    bump: Option<&str>,
    no_regen: bool,
    skip_swift_checksum: bool,
    release_date_override: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(component) = bump {
        let current = read_version(&config.version_from)?;
        let bumped = bump_version(&current, component)?;
        info!("Bumping version {current} -> {bumped} ({component})");
        write_version_to_cargo_toml(&config.version_from, &bumped).context("failed to sync versions")?;
        info!("Updated {} with bumped version {bumped}", config.version_from);
    }

    let version = read_version(&config.version_from)?;

    let last_path = std::path::Path::new(".alef").join("last_synced_version");
    info!("Syncing version {version}");

    let mut updated = vec![];
    let mut any_node_pkg_modified = false;
    let mut any_cargo_toml_modified = false;
    let mut any_composer_json_modified = false;
    let mut any_mix_exs_modified = false;
    let mut text_replacement_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();

    sync_workspace_cargo_toml_versions(&config.name, &version, &mut updated, &mut any_cargo_toml_modified);

    let python_version = to_pep440(&version);
    sync_python_versions(config, &version, &python_version, &mut updated)?;

    let node_pkg_dir = config.package_dir(Language::Node);
    let node_paths: Vec<String> = vec![format!("{node_pkg_dir}/package.json")];
    for node_path in node_paths {
        if let Ok(content) = std::fs::read_to_string(&node_path) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
                std::fs::write(&node_path, &new_content).with_context(|| format!("failed to write {node_path}"))?;
                updated.push(node_path);
                any_node_pkg_modified = true;
            }
        }
    }

    let ruby_version = to_rubygems_prerelease(&version);
    if let Ok(entries) = std::fs::read_dir("packages/ruby") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gemspec") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(new_content) =
                        replace_version_pattern(&content, r#"spec\.version\s*=\s*['"][^'"]*['"]"#, &ruby_version)
                    {
                        std::fs::write(&path, &new_content)?;
                        updated.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    for pattern in &[
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        for entry in glob::glob(pattern).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&entry) {
                if let Some(new_content) =
                    replace_version_pattern(&content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, &ruby_version)
                {
                    std::fs::write(&entry, &new_content)?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    let gemfile_lock_path = std::path::Path::new("packages/ruby/Gemfile.lock");
    if gemfile_lock_path.exists() {
        if let Ok(content) = std::fs::read_to_string(gemfile_lock_path) {
            if let Some(new_content) = sync_gemfile_lock(&content, &ruby_version) {
                std::fs::write(gemfile_lock_path, &new_content)
                    .context("failed to write packages/ruby/Gemfile.lock")?;
                updated.push("packages/ruby/Gemfile.lock".to_string());
            }
        }
    }

    {
        let core_member: std::collections::HashSet<String> = std::iter::once(config.name.clone()).collect();
        for entry in glob::glob("packages/ruby/ext/*/native/Cargo.toml")
            .into_iter()
            .flatten()
            .flatten()
        {
            let path_str = entry.to_string_lossy().to_string();
            match patch_workspace_dep_versions(&path_str, &version, &core_member) {
                Ok(true) => {
                    if !updated.contains(&path_str) {
                        updated.push(path_str);
                    }
                }
                Ok(false) => {}
                Err(e) => debug!("Could not patch core dep pin in {path_str}: {e}"),
            }
        }
    }

    if let Ok(content) = std::fs::read_to_string("packages/php/composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
            std::fs::write("packages/php/composer.json", &new_content)?;
            updated.push("packages/php/composer.json".to_string());
            any_composer_json_modified = true;
        }
    }

    // Elixir: mix.exs — handle both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Ok(content) = std::fs::read_to_string("packages/elixir/mix.exs") {
        if let Some(new_content) = replace_version_pattern(&content, r#"version: "[^"]*""#, &version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
            any_mix_exs_modified = true;
        } else if let Some(new_content) = replace_version_pattern(&content, r#"@version "[^"]*""#, &version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
            any_mix_exs_modified = true;
        }
    }

    {
        let elixir_pkg = config.package_dir(Language::Elixir);
        let nif_lock_glob = format!("{elixir_pkg}/native/*/Cargo.lock");
        for entry in glob::glob(&nif_lock_glob).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&entry) {
                if let Some(new_content) = sync_cargo_lock_path_versions(&content, &version) {
                    std::fs::write(&entry, &new_content)
                        .with_context(|| format!("failed to write {}", entry.display()))?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    if let Ok(content) = std::fs::read_to_string("packages/java/pom.xml") {
        if let Some(new_content) = replace_version_pattern(&content, r#"<version>[^<]*</version>"#, &version) {
            std::fs::write("packages/java/pom.xml", &new_content)?;
            updated.push("packages/java/pom.xml".to_string());
        }
    }

    for entry in glob::glob("packages/csharp/**/*.csproj")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            let mut working = content.clone();
            if let Some(rewritten) = replace_version_pattern(&working, r#"<Version>[^<]*</Version>"#, &version) {
                working = rewritten;
            }
            if let Some(rewritten) = replace_version_pattern(
                &working,
                r#"<InformationalVersion>[^<]*</InformationalVersion>"#,
                &version,
            ) {
                working = rewritten;
            }
            if working != content {
                std::fs::write(&entry, &working)?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }

    let kotlin_gradle = std::path::Path::new(&config.package_dir(Language::Kotlin)).join("build.gradle.kts");
    if let Ok(content) = std::fs::read_to_string(&kotlin_gradle) {
        if let Some(new_content) = replace_gradle_project_version(&content, &version) {
            std::fs::write(&kotlin_gradle, &new_content)
                .with_context(|| format!("failed to write {}", kotlin_gradle.display()))?;
            updated.push(kotlin_gradle.to_string_lossy().to_string());
        }
    }

    let kotlin_android_gradle =
        std::path::Path::new(&config.package_dir(Language::KotlinAndroid)).join("build.gradle.kts");
    if let Ok(content) = std::fs::read_to_string(&kotlin_android_gradle) {
        let version_synced = replace_gradle_project_version(&content, &version).unwrap_or_else(|| content.clone());
        let new_content = remove_stale_kotlin_android_plugin(&version_synced).unwrap_or_else(|| version_synced.clone());
        if new_content != content {
            std::fs::write(&kotlin_android_gradle, &new_content)
                .with_context(|| format!("failed to write {}", kotlin_android_gradle.display()))?;
            updated.push(kotlin_android_gradle.to_string_lossy().to_string());
        }
    }

    for wasm_pkg in glob::glob("crates/*-wasm/package.json").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&wasm_pkg) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
                std::fs::write(&wasm_pkg, &new_content)?;
                updated.push(wasm_pkg.to_string_lossy().to_string());
            }
        }
    }

    for node_pkg in glob::glob("crates/*-node/package.json").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&node_pkg) {
            let mut working = content.clone();
            if let Some(rewritten) = replace_version_pattern(&working, r#""version":\s*"[^"]*""#, &version) {
                working = rewritten;
            }
            if let Ok(pkg_json) = serde_json::from_str::<serde_json::Value>(&working) {
                if let Some(parent_name) = pkg_json.get("name").and_then(|v| v.as_str()) {
                    let pattern = format!(r#""({}-[^"]+)":\s*"[^"]*""#, regex::escape(parent_name));
                    if let Ok(re) = regex::Regex::new(&pattern) {
                        let replacement = format!(r#""$1": "{version}""#);
                        working = re.replace_all(&working, replacement.as_str()).to_string();
                    }
                }
            }
            if working != content {
                std::fs::write(&node_pkg, &working)?;
                updated.push(node_pkg.to_string_lossy().to_string());
                any_node_pkg_modified = true;
            }
        }
    }

    for platform_pkg in glob::glob("crates/*-node/npm/*/package.json")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&platform_pkg) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
                std::fs::write(&platform_pkg, &new_content)?;
                updated.push(platform_pkg.to_string_lossy().to_string());
                any_node_pkg_modified = true;
            }
        }
    }

    if let Ok(content) = std::fs::read_to_string("package.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("package.json", &new_content)?;
            updated.push("package.json".to_string());
            any_node_pkg_modified = true;
        }
    }

    if let Ok(content) = std::fs::read_to_string("composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("composer.json", &new_content)?;
            updated.push("composer.json".to_string());
            any_composer_json_modified = true;
        }
    }

    if let Ok(content) = std::fs::read_to_string("packages/r/DESCRIPTION") {
        let r_version = to_r_version(&version);
        if let Some(new_content) = replace_version_pattern(&content, r"Version:\s*[^\n]*", &r_version) {
            std::fs::write("packages/r/DESCRIPTION", &new_content)?;
            updated.push("packages/r/DESCRIPTION".to_string());
        }
    }

    if let Ok(content) = std::fs::read_to_string("packages/dart/pubspec.yaml") {
        static PUBSPEC_VERSION_RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"(?m)^version:\s*[^\s#\n]+").expect("valid regex"));
        let new_content = PUBSPEC_VERSION_RE
            .replace(&content, format!("version: {version}").as_str())
            .into_owned();
        if new_content != content {
            std::fs::write("packages/dart/pubspec.yaml", &new_content)?;
            updated.push("packages/dart/pubspec.yaml".to_string());
        }
    }

    if let Ok(content) = std::fs::read_to_string("packages/zig/build.zig.zon") {
        static ZON_VERSION_RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r#"(?m)^(\s*)\.version\s*=\s*"[^"]*""#).expect("valid regex"));
        let new_content = ZON_VERSION_RE
            .replace(&content, format!(r#"$1.version = "{version}""#).as_str())
            .into_owned();
        if new_content != content {
            std::fs::write("packages/zig/build.zig.zon", &new_content)?;
            updated.push("packages/zig/build.zig.zon".to_string());
        }
    }

    if let Ok(content) = std::fs::read_to_string("packages/go/ffi_loader.go") {
        if let Some(new_content) = replace_version_pattern(&content, r#"defaultFFIVersion\s*=\s*"[^"]*""#, &version) {
            std::fs::write("packages/go/ffi_loader.go", &new_content)?;
            updated.push("packages/go/ffi_loader.go".to_string());
        }
    }

    for entry in glob::glob("packages/go/cmd/download_ffi/main.go")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            if let Some(new_content) = replace_version_pattern(&content, r#"moduleVersion\s*=\s*"[^"]*""#, &version) {
                std::fs::write(&entry, &new_content).with_context(|| format!("failed to write {}", entry.display()))?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }

    if let Ok(content) = std::fs::read_to_string("Package.swift") {
        let placeholder_applied = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{version}"));
        let new_content = sync_swift_binary_release_url(&placeholder_applied, &version).unwrap_or(placeholder_applied);
        if new_content != content {
            std::fs::write("Package.swift", &new_content)?;
            updated.push("Package.swift".to_string());
        }
    }

    sync_swift_package_versions(config, &version, &mut updated)?;

    for sh_pattern in &["e2e/c/download_ffi.sh", "test_apps/c/download_ffi.sh"] {
        for sh_script in glob::glob(sh_pattern).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&sh_script) {
                if let Some(new_content) = replace_version_pattern(&content, r#"VERSION="[^"]*""#, &version) {
                    std::fs::write(&sh_script, &new_content)
                        .with_context(|| format!("failed to write {}", sh_script.display()))?;
                    updated.push(sh_script.to_string_lossy().to_string());
                }
            }
        }
    }

    let e2e_java_pom = std::path::Path::new("e2e/java/pom.xml");
    if let Ok(content) = std::fs::read_to_string(e2e_java_pom) {
        if let Some(new_content) = sync_e2e_java_pom(&content, &version) {
            std::fs::write(e2e_java_pom, &new_content).context("failed to write e2e/java/pom.xml")?;
            updated.push("e2e/java/pom.xml".to_string());
        }
    }

    let e2e_ruby_lock = std::path::Path::new("e2e/ruby/Gemfile.lock");
    if e2e_ruby_lock.exists() {
        if let Ok(content) = std::fs::read_to_string(e2e_ruby_lock) {
            if let Some(new_content) = sync_gemfile_lock(&content, &ruby_version) {
                std::fs::write(e2e_ruby_lock, &new_content).context("failed to write e2e/ruby/Gemfile.lock")?;
                updated.push("e2e/ruby/Gemfile.lock".to_string());
            }
        }
    }

    for entry in glob::glob("e2e/go/go.mod").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            static GO_MOD_REQUIRE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"(?m)^\s+([\w./\-]+/packages/go)\s+v[\w.\-]+").expect("valid regex")
            });
            if let Some(caps) = GO_MOD_REQUIRE_RE.captures(&content) {
                let fragment = caps[1].to_string();
                if let Some(new_content) = sync_e2e_go_mod(&content, &fragment, &version) {
                    std::fs::write(&entry, &new_content)
                        .with_context(|| format!("failed to write {}", entry.display()))?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    let e2e_dart_lock = std::path::Path::new("e2e/dart/pubspec.lock");
    if e2e_dart_lock.exists() {
        if let Ok(content) = std::fs::read_to_string(e2e_dart_lock) {
            if let Some(new_content) = sync_e2e_dart_pubspec_lock(&content, &version) {
                std::fs::write(e2e_dart_lock, &new_content).context("failed to write e2e/dart/pubspec.lock")?;
                updated.push("e2e/dart/pubspec.lock".to_string());
            }
        }
    }

    if let Some(citation_config) = config.citation.as_ref() {
        let fallback_license = read_workspace_license(&config.version_from);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let owned_config_with_override;
        let effective_citation = if let Some(date) = release_date_override {
            owned_config_with_override = crate::core::config::CitationConfig {
                date_released: Some(date.to_string()),
                ..citation_config.clone()
            };
            &owned_config_with_override
        } else {
            citation_config
        };
        let rendered = render_citation_cff(effective_citation, &version, fallback_license.as_deref(), &today);
        let needs_write = match std::fs::read_to_string("CITATION.cff") {
            Ok(current) => current != rendered,
            Err(_) => true,
        };
        if needs_write {
            std::fs::write("CITATION.cff", &rendered)?;
            updated.push("CITATION.cff".to_string());
        }
    } else if let Ok(content) = std::fs::read_to_string("CITATION.cff") {
        if let Some(new_content) = replace_citation_version(&content, &version) {
            std::fs::write("CITATION.cff", &new_content)?;
            updated.push("CITATION.cff".to_string());
        }
    }

    if let Some(sync_config) = &config.sync {
        for pattern in &sync_config.extra_paths {
            match glob::glob(pattern) {
                Ok(paths) => {
                    for entry in paths {
                        match entry {
                            Ok(path) => {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                                    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                                    if file_name == "package.json" {
                                        if let Some(new_content) =
                                            replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version)
                                        {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if file_name == "Cargo.toml" {
                                        let path_str = path.to_string_lossy().to_string();
                                        if write_version_to_cargo_toml(&path_str, &version).is_ok() {
                                            updated.push(path_str);
                                        }
                                    } else if file_name == "pyproject.toml" {
                                        let py_ver = to_pep440(&version);
                                        if let Some(new_content) =
                                            replace_version_pattern(&content, r#"version = "[^"]*""#, &py_ver)
                                        {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if file_name == "version.rb" {
                                        let rb_ver = to_rubygems_prerelease(&version);
                                        if let Some(new_content) = replace_version_pattern(
                                            &content,
                                            r#"VERSION\s*=\s*['"][^'"]*['"]"#,
                                            &rb_ver,
                                        ) {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if extension == "gemspec" {
                                        let rb_ver = to_rubygems_prerelease(&version);
                                        if let Some(new_content) = replace_version_pattern(
                                            &content,
                                            r#"spec\.version\s*=\s*['"][^'"]*['"]"#,
                                            &rb_ver,
                                        ) {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if file_name == "gleam.toml" {
                                        let mut new_content = content.clone();
                                        if let Some(updated_version) =
                                            replace_version_pattern(&new_content, r#"version = "[^"]*""#, &version)
                                        {
                                            new_content = updated_version;
                                        }
                                        new_content = restore_gleam_dep_ranges(&new_content);
                                        if new_content != content {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else {
                                        let new_content = SEMVER_RE.replace_all(&content, version.as_str()).to_string();
                                        if new_content != content {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Glob entry error for pattern '{pattern}': {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Invalid glob pattern '{pattern}': {e}");
                }
            }
        }

        for replacement in &sync_config.text_replacements {
            match glob::glob(&replacement.path) {
                Ok(paths) => {
                    for entry in paths {
                        match entry {
                            Ok(path) => {
                                text_replacement_paths.insert(path.clone());
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    let pep440 = to_pep440(&version);
                                    let rubygems = to_rubygems_prerelease(&version);
                                    let r_ver = to_r_version(&version);
                                    let search = replacement
                                        .search
                                        .replace("{python_version}", &pep440)
                                        .replace("{ruby_version}", &rubygems)
                                        .replace("{r_version}", &r_ver)
                                        .replace("{version}", &version);
                                    let replace = replacement
                                        .replace
                                        .replace("{python_version}", &pep440)
                                        .replace("{ruby_version}", &rubygems)
                                        .replace("{r_version}", &r_ver)
                                        .replace("{version}", &version);
                                    if let Ok(re) = regex::Regex::new(&search) {
                                        let new_content = re.replace_all(&content, replace.as_str()).to_string();
                                        if new_content != content {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Glob entry error for pattern '{}': {e}", replacement.path);
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Invalid glob pattern '{}': {e}", replacement.path);
                }
            }
        }
    }

    for badge_file in sync_docs_version_badges(std::path::Path::new("docs/reference"), &version) {
        updated.push(badge_file);
    }

    if any_node_pkg_modified {
        run_optional("pnpm", &["install", "--no-frozen-lockfile", "--ignore-scripts", "-w"]);
    }
    if any_cargo_toml_modified {
        run_optional("cargo", &["update", "--workspace", "--offline"]);
    }
    if any_composer_json_modified {
        run_optional("composer", &["update", "--lock", "--no-interaction"]);
    }
    if any_mix_exs_modified {
        run_optional("mix", &["deps.get"]);
    }

    let mut finalize_paths: std::collections::HashSet<std::path::PathBuf> =
        updated.iter().map(std::path::PathBuf::from).collect();
    finalize_paths.extend(text_replacement_paths);
    if !finalize_paths.is_empty() {
        let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
        match super::super::cache::sources_hash(&config.source_hash_paths()) {
            Ok(sources_hash) => {
                match super::generate::finalize_hashes(&finalize_paths, &sources_hash, &alef_toml_bytes) {
                    Ok(n) if n > 0 => {
                        debug!("  Finalized alef:hash in {n} file(s)");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Could not finalize hashes after version sync: {e}");
                    }
                }
            }
            Err(e) => {
                warn!("Could not compute sources hash for finalize_hashes: {e}");
            }
        }
    }

    for file in &updated {
        info!("  Updated: {file}");
    }

    if !updated.is_empty() && config.languages.contains(&Language::Ffi) {
        let ffi_crate = config
            .explicit_output
            .ffi
            .as_ref()
            .and_then(|p| {
                let p = p.to_string_lossy();
                let trimmed = p.trim_end_matches('/');
                let trimmed = trimmed.strip_suffix("/src").unwrap_or(trimmed);
                trimmed.rsplit('/').next().map(|s| s.to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{}-ffi", config.core_crate_dir()));
        info!("Rebuilding FFI ({ffi_crate}) to refresh C headers...");
        let _ = run_command(&format!("cargo build -p {ffi_crate}"));
    }

    let _ = std::fs::create_dir_all(".alef");
    let _ = std::fs::write(&last_path, &version);

    match sync_registry_package_versions(config_path, &version) {
        Ok(true) => {
            info!("Updated registry package versions in {}", config_path.display());
        }
        Ok(false) => {}
        Err(e) => {
            warn!(
                "Could not sync registry package versions in {}: {e}",
                config_path.display()
            );
        }
    }

    if !no_regen {
        if let Some(e2e_config) = config.e2e.as_ref() {
            match regenerate_test_apps_after_sync(config, e2e_config, config_path) {
                Ok(count) if count > 0 => {
                    info!("  Regenerated {count} test_apps file(s) with updated version pins");
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Could not regenerate test_apps after version sync: {e}");
                }
            }
        }

        match regenerate_scaffold_after_sync(config, config_path) {
            Ok(count) if count > 0 => {
                info!("  Regenerated {count} scaffold file(s) with updated version pins");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("Could not regenerate scaffold after version sync: {e}");
            }
        }

        if let Ok(content) = std::fs::read_to_string("Package.swift") {
            let placeholder_applied = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{version}"));
            let new_content =
                sync_swift_binary_release_url(&placeholder_applied, &version).unwrap_or(placeholder_applied);
            if new_content != content {
                std::fs::write("Package.swift", &new_content)?;
                if !updated.iter().any(|p| p == "Package.swift") {
                    updated.push("Package.swift".to_string());
                }
            }
        }

        if !skip_swift_checksum {
            match precompute_swift_checksum(config) {
                Ok(Some(checksum)) => {
                    info!("Swift artifactbundle checksum precomputed: {checksum}");
                    if !updated.iter().any(|p| p == "Package.swift") {
                        updated.push("Package.swift".to_string());
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("Swift checksum precompute failed: {e} — Package.swift retains placeholder");
                }
            }
        }
    }

    if updated.is_empty() {
        debug!("Versions already in sync — skipping README regeneration");
        return Ok(());
    }

    let hashes_dir = std::path::Path::new(".alef").join("hashes");
    for stem in ["readme", "docs", "scaffold"] {
        for ext in [".hash", ".manifest", ".output_hashes"] {
            let p = hashes_dir.join(format!("{stem}{ext}"));
            if p.exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    info!("Regenerating READMEs with updated version");
    match regenerate_readmes(config, config_path) {
        Ok(count) => {
            if count > 0 {
                info!("  Regenerated {count} README(s)");
            } else {
                debug!("  No READMEs updated");
            }
        }
        Err(e) => {
            warn!("Could not regenerate READMEs: {e}");
        }
    }

    Ok(())
}
#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
