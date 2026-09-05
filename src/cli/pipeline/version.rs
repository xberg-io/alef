use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::sync::LazyLock;
use tracing::{debug, info, warn};

use super::helpers::run_optional;
use super::version_core::{
    bump_version, package_json_is_private, patch_workspace_dep_versions, read_version, to_pep440,
    write_version_to_cargo_toml,
};
use super::version_csharp::sync_csharp_project_versions;
use super::version_lockfiles::{relock_cargo_lockfiles, retry_blocked_lockfiles};
use super::version_python::sync_python_versions;
use super::version_regen::{regenerate_readmes, regenerate_scaffold_after_sync, regenerate_test_apps_after_sync};
use super::version_registry::sync_registry_package_versions;
use super::version_swift::{precompute_swift_checksum, sync_swift_package_versions};
use super::version_text::{
    read_workspace_license, remove_stale_kotlin_android_plugin, render_citation_cff, replace_citation_version,
    replace_gradle_project_version, replace_version_pattern, restore_gleam_dep_ranges, sync_cargo_lock_path_versions,
    sync_docs_version_badges, sync_e2e_dart_pubspec_lock, sync_e2e_go_mod, sync_e2e_java_pom, sync_gemfile_lock,
    sync_go_cmd_setup_version_ident, sync_go_native_setup_sentinel, sync_swift_binary_release_url,
};
use super::version_workspace::{sync_rust_test_app_version, sync_workspace_cargo_toml_versions};
use crate::core::version::{to_go_version_ident, to_r_version, to_rubygems_prerelease};

/// Regex for matching semantic version strings.
static SEMVER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\d+\.\d+\.\d+(-[a-zA-Z0-9._]+)*").expect("valid regex"));

/// Whether version-sync's *catch-all* rewrites may touch this file.
///
/// The named-filename branches above (`package.json`, `Cargo.toml`, `pyproject.toml`,
/// `version.rb`, `.gemspec`, `gleam.toml`) are a declared contract: a consumer listing
/// one is asking for that specific field to be rewritten, and they stay unguarded. The
/// two catch-alls are different — a blanket `SEMVER_RE.replace_all` and a user-supplied
/// regex will rewrite *whatever the glob happens to match*, so a slightly wide pattern
/// silently edits hand-written files.
///
/// Only marker-bearing extensions can be judged. Per `marker_comment_style`, a file alef
/// cannot stamp never carries a marker even when alef wrote every byte, so for those
/// (`.md`, `.json`, `.lock`, `Makefile`, …) absence proves nothing and they stay
/// permitted — refusing there would freeze legitimate regeneration, and it is why
/// `--regen` still replaces a generated README.
///
/// ~keep alef #A1: `sync.text_replacements`'s `search`/`replace` pair is consumer-authored and
/// path-named, unlike `sync.extra_paths`'s blanket `SEMVER_RE` -- one could argue that is
/// explicit-enough consent to skip this guard entirely. Deliberately kept anyway: a
/// `text_replacements` glob (`packages/*/README.md`, `docs/**/*.mdx`) can still reach a file the
/// consumer did not mean to hand alef write access to, and the regex itself only constrains
/// *what* gets replaced inside a match, not *which* files the glob touches -- the same "glob
/// wider than intended" risk `sync.extra_paths` has, just with a narrower blast radius per file.
/// The defect this fix actually closes is the CALL ORDER (below), not this guard's scope: the
/// caller now proves the substitution would change something before asking this guard whether
/// the write is even permitted, so an already-correct file is never refused in the first place.
fn catch_all_rewrite_is_permitted(path: &std::path::Path, content: &str) -> bool {
    if super::generate::marker_comment_style(path).is_none() {
        return true;
    }
    if crate::core::hash::content_has_alef_marker(content) {
        return true;
    }
    // A refusal here changed nothing on disk if the file never had a semver-shaped
    // substring to begin with (e.g. a Cargo.toml member using `version.workspace =
    // true`, which has no literal version field for either the catch-all or the
    // named-filename branches to touch). That case is expected and not worth an
    // alarming `warn!` — only a file that actually had a rewrite candidate and got
    // refused anyway is the surprising, worth-a-warning case. ~keep
    if !SEMVER_RE.is_match(content) {
        debug!(
            path = %path.display(),
            "version-sync: skipping a catch-all rewrite; this stampable file carries no alef marker and \
             no semver-shaped substring"
        );
        return false;
    }
    warn!(
        path = %path.display(),
        "version-sync: skipping a catch-all rewrite; this stampable file carries no alef marker, so alef \
         treats it as hand-written. If alef does own it, give it a provenance marker or move it to a \
         named-filename branch"
    );
    false
}

/// Report, once per consumer-supplied pattern, that its expansion reached files git ignores.
///
/// `sync.extra_paths` and `sync.text_replacements` are the only rewrite surfaces whose pattern
/// alef does not author, so a refusal here means the consumer's pattern is wider than they meant
/// — worth a warning, unlike alef's own globs whose refusals are routine and logged at `debug`.
/// Counted per pattern rather than per path because a `**` that reaches a staging tree reaches
/// every file in it at once, and a line per file would bury the one fact that matters. ~keep
fn warn_refused_matches(surface: &str, pattern: &str, refused: usize) {
    if refused == 0 {
        return;
    }
    warn!(
        "version-sync: {surface} pattern '{pattern}' matched {refused} git-ignored file(s), which were not \
         rewritten. Narrow the pattern if that is a surprise"
    );
}

/// Warn that a *declared* version-sync target -- a `sync.text_replacements` entry a consumer
/// named explicitly -- was not rewritten to the new version.
///
/// This is a stronger claim than an ordinary skipped rewrite: a consumer that lists a path here
/// is asking alef to keep that file's version pin current, so a refused write leaves the repo
/// internally inconsistent (this file stale, everything else current) with nothing at regen time
/// connecting the refusal to the sync contract it just broke. The break only surfaces later, at
/// `alef validate versions`, far from the cause -- so this stays a WARN emitted right here rather
/// than a hard error: failing `sync_versions` itself would turn one unwritable file into a stop
/// for the whole `alef all` pipeline, which is worse than the staleness it prevents. `alef
/// validate versions` remains the authoritative hard gate for the drift this leaves behind. ~keep
fn warn_sync_target_not_updated(path: &std::path::Path, version: &str, reason: &str) {
    warn!(
        path = %path.display(),
        expected_version = version,
        "version-sync: declared sync.text_replacements target was not updated to {version} -- {reason}. It \
         stays pinned to a stale version until the write succeeds or the entry is removed from \
         sync.text_replacements"
    );
}

/// Warn that a declared `sync.text_replacements` `search` pattern matched nothing at all in a
/// file alef successfully opened.
///
/// ~keep alef #A2: a pattern that never matches is indistinguishable on disk from a file that is
/// already current -- both leave the file byte-for-byte unchanged -- so nothing here signalled
/// the difference before this warning existed. A real downstream incident this closes: a
/// `text_replacements` entry searched for a downstream repo's `.git` URL with `from:`, but the target file
/// had moved to a non-`.git` URL with `branch:`; the pattern matched nothing for three releases in
/// a row and every one of those runs looked identical to "already correct." The same rule applies
/// here as everywhere else this fix touches: a substitution must prove it changed something, or
/// it must say so.
fn warn_sync_target_pattern_matched_nothing(path: &std::path::Path, pattern: &str) {
    warn!(
        path = %path.display(),
        pattern,
        "version-sync: declared sync.text_replacements search pattern '{pattern}' matched nothing in this \
         file. If the file's shape changed (a different URL scheme, a renamed field), update the pattern -- \
         a silent non-match is indistinguishable from an already-current file"
    );
}

/// Write-tracking state threaded through every version-sync step: which files were rewritten,
/// which package ecosystems saw a manifest change (so the post-write side effects in
/// `run_post_write_side_effects` know which package managers to re-run), and which
/// `sync.text_replacements` paths were touched (folded into `finalize_hashes_for_updated`'s
/// hash-stamping set alongside `updated`). ~keep
struct SyncState {
    updated: Vec<String>,
    any_node_pkg_modified: bool,
    any_cargo_toml_modified: bool,
    any_composer_json_modified: bool,
    any_mix_exs_modified: bool,
    text_replacement_paths: std::collections::HashSet<std::path::PathBuf>,
}

impl SyncState {
    fn new() -> Self {
        Self {
            updated: vec![],
            any_node_pkg_modified: false,
            any_cargo_toml_modified: false,
            any_composer_json_modified: false,
            any_mix_exs_modified: false,
            text_replacement_paths: std::collections::HashSet::new(),
        }
    }
}

/// Sync version from Cargo.toml to all package manifest files.
///
/// When `no_regen` is `false` (the default for direct CLI invocations), this
/// function automatically regenerates `test_apps/` scaffold files after updating
/// `[crates.e2e.registry.packages.*].version` in `alef.toml`, so the version
/// pins in generated files (pyproject.toml, mix.exs, build.zig.zon, Package.swift,
/// etc.) always match the workspace version atomically.
///
/// Pass `no_regen = true` to opt out of the automatic regeneration. `alef generate`
/// passes it not because it owns `test_apps/` — it does not write that tree at all —
/// but because regenerating test apps is deliberately excluded from that command.
/// `alef all` and `alef test-apps generate` are the two commands that write there. ~keep
pub fn sync_versions(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
    bump: Option<&str>,
    no_regen: bool,
    skip_swift_checksum: bool,
    release_date_override: Option<&str>,
) -> anyhow::Result<()> {
    apply_version_bump(config, bump)?;

    let version = read_version(&config.version_from)?;
    info!("Syncing version {version}");

    // Every rewrite below that finds its target by expanding a glob goes through this filter.
    // The name-based skip lists this replaces could never be complete — `tmp`, `stage` and
    // `build` are per-tool names, and a gem-packaging stage under `packages/ruby/tmp/` was the
    // third such directory to be discovered the hard way — whereas "git ignores it" is the
    // property that actually describes the damage: a version bump written into build staging is
    // an edit no review sees and the next clean destroys, while leaving it genuinely ambiguous
    // whether the tracked original was bumped too. ~keep
    let writable = crate::cli::git::IgnoreFilter::for_current_dir();
    warn_if_ignore_filter_degraded(&writable);

    let mut state = SyncState::new();
    let ruby_version = to_rubygems_prerelease(&version);

    sync_rust_and_python(config, &version, &writable, &mut state)?;
    sync_node_and_ruby_packages(config, &version, &ruby_version, &writable, &mut state)?;
    sync_remaining_package_manifests(config, &version, &writable, &mut state)?;
    sync_go_and_swift(config, &version, &writable, &mut state)?;
    sync_e2e_harness_files(&version, &ruby_version, &writable, &mut state)?;
    sync_citation_and_extra_paths(config, &version, release_date_override, &writable, &mut state)?;

    finalize_version_sync(config, config_path, &version, no_regen, skip_swift_checksum, &mut state)
}

fn apply_version_bump(config: &ResolvedCrateConfig, bump: Option<&str>) -> anyhow::Result<()> {
    let Some(component) = bump else {
        return Ok(());
    };
    let current = read_version(&config.version_from)?;
    let bumped = bump_version(&current, component)?;
    info!("Bumping version {current} -> {bumped} ({component})");
    write_version_to_cargo_toml(&config.version_from, &bumped).context("failed to sync versions")?;
    info!("Updated {} with bumped version {bumped}", config.version_from);
    Ok(())
}

fn warn_if_ignore_filter_degraded(writable: &crate::cli::git::IgnoreFilter) {
    if !writable.is_degraded() {
        return;
    }
    warn!(
        "version-sync cannot read git's ignore rules (not a git work tree, or `git` is unavailable); \
         glob-discovered rewrites fall back to an unfiltered disk walk and may write into \
         build-staging copies"
    );
}

fn sync_rust_and_python(
    config: &ResolvedCrateConfig,
    version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    sync_workspace_cargo_toml_versions(
        &config.name,
        version,
        writable,
        &mut state.updated,
        &mut state.any_cargo_toml_modified,
    );
    sync_rust_test_app_version(config, version, &mut state.updated, &mut state.any_cargo_toml_modified);

    let python_version = to_pep440(version);
    sync_python_versions(config, version, &python_version, writable, &mut state.updated)?;
    Ok(())
}

fn sync_node_pkg_json(node_pkg_dir: &str, version: &str, state: &mut SyncState) -> anyhow::Result<()> {
    let node_paths: Vec<String> = vec![format!("{node_pkg_dir}/package.json")];
    for node_path in node_paths {
        if let Ok(content) = std::fs::read_to_string(&node_path) {
            if package_json_is_private(&content) {
                continue;
            }
            if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, version) {
                std::fs::write(&node_path, &new_content).with_context(|| format!("failed to write {node_path}"))?;
                state.updated.push(node_path);
                state.any_node_pkg_modified = true;
            }
        }
    }
    Ok(())
}

fn sync_ruby_gemspecs(
    writable: &crate::cli::git::IgnoreFilter,
    ruby_version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    if let Ok(entries) = std::fs::read_dir("packages/ruby") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gemspec")
                && writable.allows(&path)
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Some(new_content) =
                    replace_version_pattern(&content, r#"spec\.version\s*=\s*['"][^'"]*['"]"#, ruby_version)
            {
                std::fs::write(&path, &new_content)?;
                updated.push(path.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

fn sync_ruby_version_rb_files(
    writable: &crate::cli::git::IgnoreFilter,
    ruby_version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    for pattern in &[
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        for entry in writable.glob(pattern) {
            if let Ok(content) = std::fs::read_to_string(&entry)
                && let Some(new_content) =
                    replace_version_pattern(&content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, ruby_version)
            {
                std::fs::write(&entry, &new_content)?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

/// Sync a `Gemfile.lock` at `path_str` to `ruby_version`. Shared by the main
/// `packages/ruby/Gemfile.lock` and the `e2e/ruby/Gemfile.lock` harness copy, which carry the
/// same rewrite rule but live at different paths. ~keep
fn sync_ruby_gemfile_lock_at(path_str: &str, ruby_version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    let path = std::path::Path::new(path_str);
    if path.exists()
        && let Ok(content) = std::fs::read_to_string(path)
        && let Some(new_content) = sync_gemfile_lock(&content, ruby_version)
    {
        std::fs::write(path, &new_content).with_context(|| format!("failed to write {path_str}"))?;
        updated.push(path_str.to_string());
    }
    Ok(())
}

fn sync_ruby_native_cargo_tomls(
    writable: &crate::cli::git::IgnoreFilter,
    config: &ResolvedCrateConfig,
    version: &str,
    updated: &mut Vec<String>,
) {
    let core_member: std::collections::HashSet<String> = std::iter::once(config.name.clone()).collect();
    for entry in writable.glob("packages/ruby/ext/*/native/Cargo.toml") {
        let path_str = entry.to_string_lossy().to_string();
        // This manifest is not a workspace member, so `sync_workspace_cargo_toml_versions`
        // never reaches it: its own `[package].version` needs the same direct write the
        // dep-pin patch below gets, or the crate's declared version drifts from the
        // workspace version on every bump. ~keep
        if write_version_to_cargo_toml(&path_str, version).unwrap_or(false) && !updated.contains(&path_str) {
            updated.push(path_str.clone());
        }
        match patch_workspace_dep_versions(&path_str, version, &core_member) {
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

fn sync_node_and_ruby_packages(
    config: &ResolvedCrateConfig,
    version: &str,
    ruby_version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    let node_pkg_dir = config.package_dir(Language::Node);
    sync_node_pkg_json(&node_pkg_dir, version, state)?;

    sync_ruby_gemspecs(writable, ruby_version, &mut state.updated)?;
    sync_ruby_version_rb_files(writable, ruby_version, &mut state.updated)?;
    sync_ruby_gemfile_lock_at("packages/ruby/Gemfile.lock", ruby_version, &mut state.updated)?;
    sync_ruby_native_cargo_tomls(writable, config, version, &mut state.updated);
    Ok(())
}

fn sync_php_composer_json(path: &str, version: &str, state: &mut SyncState) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string(path)
        && let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, version)
    {
        std::fs::write(path, &new_content)?;
        state.updated.push(path.to_string());
        state.any_composer_json_modified = true;
    }
    Ok(())
}

fn sync_elixir_mix_exs(version: &str, state: &mut SyncState) -> anyhow::Result<()> {
    // Elixir: mix.exs — handle both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Ok(content) = std::fs::read_to_string("packages/elixir/mix.exs") {
        if let Some(new_content) = replace_version_pattern(&content, r#"version: "[^"]*""#, version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            state.updated.push("packages/elixir/mix.exs".to_string());
            state.any_mix_exs_modified = true;
        } else if let Some(new_content) = replace_version_pattern(&content, r#"@version "[^"]*""#, version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            state.updated.push("packages/elixir/mix.exs".to_string());
            state.any_mix_exs_modified = true;
        }
    }
    Ok(())
}

fn sync_elixir_nif_cargo_locks(
    config: &ResolvedCrateConfig,
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    let elixir_pkg = config.package_dir(Language::Elixir);
    let nif_lock_glob = format!("{elixir_pkg}/native/*/Cargo.lock");
    for entry in writable.glob(&nif_lock_glob) {
        if let Ok(content) = std::fs::read_to_string(&entry)
            && let Some(new_content) = sync_cargo_lock_path_versions(&content, version)
        {
            std::fs::write(&entry, &new_content).with_context(|| format!("failed to write {}", entry.display()))?;
            updated.push(entry.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn sync_java_pom(path: &str, version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string(path)
        && let Some(new_content) = replace_version_pattern(&content, r#"<version>[^<]*</version>"#, version)
    {
        std::fs::write(path, &new_content)?;
        updated.push(path.to_string());
    }
    Ok(())
}

fn sync_csharp_csproj_files(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    for entry in writable.glob("packages/csharp/**/*.csproj") {
        if let Ok(content) = std::fs::read_to_string(&entry)
            && let Some(rewritten) = sync_csharp_project_versions(&content, version)
        {
            std::fs::write(&entry, &rewritten)?;
            updated.push(entry.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn sync_kotlin_gradle(config: &ResolvedCrateConfig, version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    let kotlin_gradle = std::path::Path::new(&config.package_dir(Language::Kotlin)).join("build.gradle.kts");
    if let Ok(content) = std::fs::read_to_string(&kotlin_gradle)
        && let Some(new_content) = replace_gradle_project_version(&content, version)
    {
        std::fs::write(&kotlin_gradle, &new_content)
            .with_context(|| format!("failed to write {}", kotlin_gradle.display()))?;
        updated.push(kotlin_gradle.to_string_lossy().to_string());
    }
    Ok(())
}

fn sync_kotlin_android_gradle(
    config: &ResolvedCrateConfig,
    version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    let kotlin_android_gradle =
        std::path::Path::new(&config.package_dir(Language::KotlinAndroid)).join("build.gradle.kts");
    if let Ok(content) = std::fs::read_to_string(&kotlin_android_gradle) {
        let version_synced = replace_gradle_project_version(&content, version).unwrap_or_else(|| content.clone());
        let new_content = remove_stale_kotlin_android_plugin(&version_synced).unwrap_or_else(|| version_synced.clone());
        if new_content != content {
            std::fs::write(&kotlin_android_gradle, &new_content)
                .with_context(|| format!("failed to write {}", kotlin_android_gradle.display()))?;
            updated.push(kotlin_android_gradle.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn sync_wasm_package_json_files(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    for wasm_pkg in writable.glob("crates/*-wasm/package.json") {
        if let Ok(content) = std::fs::read_to_string(&wasm_pkg) {
            if package_json_is_private(&content) {
                continue;
            }
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, version) {
                std::fs::write(&wasm_pkg, &new_content)?;
                updated.push(wasm_pkg.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

fn sync_node_crate_package_json_files(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    for node_pkg in writable.glob("crates/*-node/package.json") {
        if let Ok(content) = std::fs::read_to_string(&node_pkg) {
            if package_json_is_private(&content) {
                continue;
            }
            let mut working = content.clone();
            if let Some(rewritten) = replace_version_pattern(&working, r#""version":\s*"[^"]*""#, version) {
                working = rewritten;
            }
            if let Ok(pkg_json) = serde_json::from_str::<serde_json::Value>(&working)
                && let Some(parent_name) = pkg_json.get("name").and_then(|v| v.as_str())
            {
                let pattern = format!(r#""({}-[^"]+)":\s*"[^"]*""#, regex::escape(parent_name));
                if let Ok(re) = regex::Regex::new(&pattern) {
                    let replacement = format!(r#""$1": "{version}""#);
                    working = re.replace_all(&working, replacement.as_str()).to_string();
                }
            }
            if working != content {
                std::fs::write(&node_pkg, &working)?;
                state.updated.push(node_pkg.to_string_lossy().to_string());
                state.any_node_pkg_modified = true;
            }
        }
    }
    Ok(())
}

fn sync_node_platform_package_json_files(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    for platform_pkg in writable.glob("crates/*-node/npm/*/package.json") {
        if let Ok(content) = std::fs::read_to_string(&platform_pkg) {
            if package_json_is_private(&content) {
                continue;
            }
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, version) {
                std::fs::write(&platform_pkg, &new_content)?;
                state.updated.push(platform_pkg.to_string_lossy().to_string());
                state.any_node_pkg_modified = true;
            }
        }
    }
    Ok(())
}

fn sync_root_package_json(version: &str, state: &mut SyncState) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("package.json")
        && let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, version)
    {
        std::fs::write("package.json", &new_content)?;
        state.updated.push("package.json".to_string());
        state.any_node_pkg_modified = true;
    }
    Ok(())
}

fn sync_root_composer_json(version: &str, state: &mut SyncState) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("composer.json")
        && let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, version)
    {
        std::fs::write("composer.json", &new_content)?;
        state.updated.push("composer.json".to_string());
        state.any_composer_json_modified = true;
    }
    Ok(())
}

fn sync_r_description(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("packages/r/DESCRIPTION") {
        let r_version = to_r_version(version);
        if let Some(new_content) = replace_version_pattern(&content, r"Version:\s*[^\n]*", &r_version) {
            std::fs::write("packages/r/DESCRIPTION", &new_content)?;
            updated.push("packages/r/DESCRIPTION".to_string());
        }
    }
    Ok(())
}

fn sync_dart_pubspec(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
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
    Ok(())
}

fn sync_zig_build_zon(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
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
    Ok(())
}

fn sync_remaining_package_manifests(
    config: &ResolvedCrateConfig,
    version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    sync_php_composer_json("packages/php/composer.json", version, state)?;
    sync_elixir_mix_exs(version, state)?;
    sync_elixir_nif_cargo_locks(config, writable, version, &mut state.updated)?;
    sync_java_pom("packages/java/pom.xml", version, &mut state.updated)?;
    sync_csharp_csproj_files(writable, version, &mut state.updated)?;
    sync_kotlin_gradle(config, version, &mut state.updated)?;
    sync_kotlin_android_gradle(config, version, &mut state.updated)?;
    sync_wasm_package_json_files(writable, version, &mut state.updated)?;
    sync_node_crate_package_json_files(writable, version, state)?;
    sync_node_platform_package_json_files(writable, version, state)?;
    sync_root_package_json(version, state)?;
    sync_root_composer_json(version, state)?;
    sync_r_description(version, &mut state.updated)?;
    sync_dart_pubspec(version, &mut state.updated)?;
    sync_zig_build_zon(version, &mut state.updated)?;
    Ok(())
}

fn sync_go_ffi_loader(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("packages/go/ffi_loader.go")
        && let Some(new_content) = replace_version_pattern(&content, r#"defaultFFIVersion\s*=\s*"[^"]*""#, version)
    {
        std::fs::write("packages/go/ffi_loader.go", &new_content)?;
        updated.push("packages/go/ffi_loader.go".to_string());
    }
    Ok(())
}

fn sync_go_cmd_setup_main(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    go_version_ident: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    for entry in writable.glob("packages/go/cmd/setup/main.go") {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            let mut new_content = content.clone();
            let mut changed = false;
            if let Some(c) = replace_version_pattern(&new_content, r#"moduleVersion\s*=\s*"[^"]*""#, version) {
                new_content = c;
                changed = true;
            }
            if let Some(c) = sync_go_cmd_setup_version_ident(&new_content, go_version_ident) {
                new_content = c;
                changed = true;
            }
            if changed {
                std::fs::write(&entry, &new_content).with_context(|| format!("failed to write {}", entry.display()))?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

fn sync_go_native_setup(go_version_ident: &str, version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("packages/go/native_setup.go")
        && let Some(new_content) = sync_go_native_setup_sentinel(&content, go_version_ident, version)
    {
        std::fs::write("packages/go/native_setup.go", &new_content)
            .context("failed to write packages/go/native_setup.go")?;
        updated.push("packages/go/native_setup.go".to_string());
    }
    Ok(())
}

fn sync_package_swift_placeholder(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("Package.swift") {
        let placeholder_applied = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{version}"));
        let new_content = sync_swift_binary_release_url(&placeholder_applied, version).unwrap_or(placeholder_applied);
        if new_content != content {
            std::fs::write("Package.swift", &new_content)?;
            updated.push("Package.swift".to_string());
        }
    }
    Ok(())
}

fn sync_go_and_swift(
    config: &ResolvedCrateConfig,
    version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    sync_go_ffi_loader(version, &mut state.updated)?;

    // `cmd/setup/main.go`'s `versionIdent` const and `native_setup.go`'s
    // `RequireNativeSetup_<ident>` sentinel must always carry the identical identifier --
    // both are derived from this single `to_go_version_ident` call, so a sync-versions run
    // can never move one file's identifier without moving the other's. See
    // `sync_go_native_setup_sentinel`'s doc for the alef#159 incident this
    // single-source computation closes.
    let go_version_ident = to_go_version_ident(version);
    sync_go_cmd_setup_main(writable, version, &go_version_ident, &mut state.updated)?;
    sync_go_native_setup(&go_version_ident, version, &mut state.updated)?;

    sync_package_swift_placeholder(version, &mut state.updated)?;
    sync_swift_package_versions(config, version, writable, &mut state.updated)?;
    Ok(())
}

fn sync_download_ffi_scripts(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    for sh_pattern in &["e2e/c/download_ffi.sh", "test_apps/c/download_ffi.sh"] {
        for sh_script in writable.glob(sh_pattern) {
            if let Ok(content) = std::fs::read_to_string(&sh_script)
                && let Some(new_content) = replace_version_pattern(&content, r#"VERSION="[^"]*""#, version)
            {
                std::fs::write(&sh_script, &new_content)
                    .with_context(|| format!("failed to write {}", sh_script.display()))?;
                updated.push(sh_script.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

fn sync_e2e_java_pom_file(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    let e2e_java_pom = std::path::Path::new("e2e/java/pom.xml");
    if let Ok(content) = std::fs::read_to_string(e2e_java_pom)
        && let Some(new_content) = sync_e2e_java_pom(&content, version)
    {
        std::fs::write(e2e_java_pom, &new_content).context("failed to write e2e/java/pom.xml")?;
        updated.push("e2e/java/pom.xml".to_string());
    }
    Ok(())
}

fn sync_e2e_go_mod_file(
    writable: &crate::cli::git::IgnoreFilter,
    version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    for entry in writable.glob("e2e/go/go.mod") {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            static GO_MOD_REQUIRE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"(?m)^\s+([\w./\-]+/packages/go)\s+v[\w.\-]+").expect("valid regex")
            });
            if let Some(caps) = GO_MOD_REQUIRE_RE.captures(&content) {
                let fragment = caps[1].to_string();
                if let Some(new_content) = sync_e2e_go_mod(&content, &fragment, version) {
                    std::fs::write(&entry, &new_content)
                        .with_context(|| format!("failed to write {}", entry.display()))?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }
    Ok(())
}

fn sync_e2e_dart_pubspec_lock_file(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    let e2e_dart_lock = std::path::Path::new("e2e/dart/pubspec.lock");
    if e2e_dart_lock.exists()
        && let Ok(content) = std::fs::read_to_string(e2e_dart_lock)
        && let Some(new_content) = sync_e2e_dart_pubspec_lock(&content, version)
    {
        std::fs::write(e2e_dart_lock, &new_content).context("failed to write e2e/dart/pubspec.lock")?;
        updated.push("e2e/dart/pubspec.lock".to_string());
    }
    Ok(())
}

fn sync_e2e_harness_files(
    version: &str,
    ruby_version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    sync_download_ffi_scripts(writable, version, &mut state.updated)?;
    sync_e2e_java_pom_file(version, &mut state.updated)?;
    sync_ruby_gemfile_lock_at("e2e/ruby/Gemfile.lock", ruby_version, &mut state.updated)?;
    sync_e2e_go_mod_file(writable, version, &mut state.updated)?;
    sync_e2e_dart_pubspec_lock_file(version, &mut state.updated)?;
    Ok(())
}

fn sync_citation_file(
    config: &ResolvedCrateConfig,
    version: &str,
    release_date_override: Option<&str>,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
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
        let rendered = render_citation_cff(effective_citation, version, fallback_license.as_deref(), &today);
        let needs_write = match std::fs::read_to_string("CITATION.cff") {
            Ok(current) => current != rendered,
            Err(_) => true,
        };
        if needs_write {
            std::fs::write("CITATION.cff", &rendered)?;
            updated.push("CITATION.cff".to_string());
        }
    } else if let Ok(content) = std::fs::read_to_string("CITATION.cff")
        && let Some(new_content) = replace_citation_version(&content, version)
    {
        std::fs::write("CITATION.cff", &new_content)?;
        updated.push("CITATION.cff".to_string());
    }
    Ok(())
}

fn write_extra_path_rewrite(path: &std::path::Path, new_content: Option<String>, updated: &mut Vec<String>) {
    let Some(new_content) = new_content else {
        return;
    };
    if let Err(e) = std::fs::write(path, &new_content) {
        debug!("Could not write {}: {e}", path.display());
    } else {
        updated.push(path.to_string_lossy().to_string());
    }
}

/// Apply a single `sync.extra_paths` glob match. The writability check happens in the caller
/// (glob entries are filtered before this runs); this dispatches on the well-known filenames
/// (contract, unguarded) before falling back to the marker-gated catch-all. ~keep
fn apply_extra_path_rewrite(path: &std::path::Path, version: &str, updated: &mut Vec<String>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if file_name == "package.json" {
        let new_content = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, version);
        write_extra_path_rewrite(path, new_content, updated);
        return;
    }
    if file_name == "Cargo.toml" {
        let path_str = path.to_string_lossy().to_string();
        if let Ok(true) = write_version_to_cargo_toml(&path_str, version) {
            updated.push(path_str);
        }
        return;
    }
    if file_name == "pyproject.toml" {
        let py_ver = to_pep440(version);
        let new_content = replace_version_pattern(&content, r#"version = "[^"]*""#, &py_ver);
        write_extra_path_rewrite(path, new_content, updated);
        return;
    }
    if file_name == "version.rb" {
        let rb_ver = to_rubygems_prerelease(version);
        let new_content = replace_version_pattern(&content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, &rb_ver);
        write_extra_path_rewrite(path, new_content, updated);
        return;
    }
    if extension == "gemspec" {
        let rb_ver = to_rubygems_prerelease(version);
        let new_content = replace_version_pattern(&content, r#"spec\.version\s*=\s*['"][^'"]*['"]"#, &rb_ver);
        write_extra_path_rewrite(path, new_content, updated);
        return;
    }
    if file_name == "gleam.toml" {
        let mut new_content = content.clone();
        if let Some(updated_version) = replace_version_pattern(&new_content, r#"version = "[^"]*""#, version) {
            new_content = updated_version;
        }
        new_content = restore_gleam_dep_ranges(&new_content);
        write_extra_path_rewrite(path, (new_content != content).then_some(new_content), updated);
        return;
    }
    if catch_all_rewrite_is_permitted(path, &content) {
        let new_content = SEMVER_RE.replace_all(&content, version).to_string();
        write_extra_path_rewrite(path, (new_content != content).then_some(new_content), updated);
    }
}

fn apply_sync_extra_paths(
    extra_paths: &[String],
    version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    updated: &mut Vec<String>,
) {
    for pattern in extra_paths {
        let paths = match glob::glob(pattern) {
            Ok(paths) => paths,
            Err(e) => {
                debug!("Invalid glob pattern '{pattern}': {e}");
                continue;
            }
        };
        let mut refused = 0usize;
        for entry in paths {
            let path = match entry {
                Ok(path) => path,
                Err(e) => {
                    debug!("Glob entry error for pattern '{pattern}': {e}");
                    continue;
                }
            };
            if !writable.allows(&path) {
                refused += 1;
                continue;
            }
            apply_extra_path_rewrite(&path, version, updated);
        }
        warn_refused_matches("sync.extra_paths", pattern, refused);
    }
}

/// Apply one declared `sync.text_replacements` rule to a single glob-matched path. Writability
/// is checked by the caller before this runs. ~keep
fn apply_text_replacement_to_path(
    path: &std::path::Path,
    replacement: &crate::core::config::TextReplacement,
    version: &str,
    updated: &mut Vec<String>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => {
            warn_sync_target_not_updated(path, version, &format!("could not read file: {e}"));
            return;
        }
    };

    let pep440 = to_pep440(version);
    let rubygems = to_rubygems_prerelease(version);
    let r_ver = to_r_version(version);
    let search = replacement
        .search
        .replace("{python_version}", &pep440)
        .replace("{ruby_version}", &rubygems)
        .replace("{r_version}", &r_ver)
        .replace("{version}", version);
    let replace = replacement
        .replace
        .replace("{python_version}", &pep440)
        .replace("{ruby_version}", &rubygems)
        .replace("{r_version}", &r_ver)
        .replace("{version}", version);

    let re = match regex::Regex::new(&search) {
        Ok(re) => re,
        Err(e) => {
            warn_sync_target_not_updated(path, version, &format!("the configured search regex is invalid: {e}"));
            return;
        }
    };

    // ~keep A substitution must prove it changed something before either the ownership guard or
    // the write is even considered. Checking `is_match` first (not just `new_content != content`)
    // separately catches the pattern matching nothing at all, rather than folding it into the
    // same silence as "the file already has the target text" -- see
    // `warn_sync_target_pattern_matched_nothing`'s doc.
    if !re.is_match(&content) {
        warn_sync_target_pattern_matched_nothing(path, &replacement.search);
        return;
    }

    let new_content = re.replace_all(&content, replace.as_str()).to_string();
    if new_content == content {
        // Matched, but the file already holds the target text -- nothing to write, nothing to
        // warn about.
        return;
    }
    if !catch_all_rewrite_is_permitted(path, &content) {
        warn_sync_target_not_updated(
            path,
            version,
            "alef's ownership guard refused the write (no alef marker on a stampable file, so it reads as \
             hand-written)",
        );
        return;
    }
    if let Err(e) = std::fs::write(path, &new_content) {
        warn_sync_target_not_updated(path, version, &format!("the write failed: {e}"));
        return;
    }
    updated.push(path.to_string_lossy().to_string());
}

fn apply_sync_text_replacements(
    text_replacements: &[crate::core::config::TextReplacement],
    version: &str,
    writable: &crate::cli::git::IgnoreFilter,
    updated: &mut Vec<String>,
    text_replacement_paths: &mut std::collections::HashSet<std::path::PathBuf>,
) {
    for replacement in text_replacements {
        let paths = match glob::glob(&replacement.path) {
            Ok(paths) => paths,
            Err(e) => {
                debug!("Invalid glob pattern '{}': {e}", replacement.path);
                continue;
            }
        };
        let mut refused = 0usize;
        for entry in paths {
            let path = match entry {
                Ok(path) => path,
                Err(e) => {
                    debug!("Glob entry error for pattern '{}': {e}", replacement.path);
                    continue;
                }
            };
            if !writable.allows(&path) {
                refused += 1;
                warn_sync_target_not_updated(
                    &path,
                    version,
                    "the path is git-ignored (build staging or another disposable copy)",
                );
                continue;
            }
            text_replacement_paths.insert(path.clone());
            apply_text_replacement_to_path(&path, replacement, version, updated);
        }
        warn_refused_matches("sync.text_replacements", &replacement.path, refused);
    }
}

fn sync_citation_and_extra_paths(
    config: &ResolvedCrateConfig,
    version: &str,
    release_date_override: Option<&str>,
    writable: &crate::cli::git::IgnoreFilter,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    sync_citation_file(config, version, release_date_override, &mut state.updated)?;

    if let Some(sync_config) = &config.sync {
        apply_sync_extra_paths(&sync_config.extra_paths, version, writable, &mut state.updated);
        apply_sync_text_replacements(
            &sync_config.text_replacements,
            version,
            writable,
            &mut state.updated,
            &mut state.text_replacement_paths,
        );
    }

    // Resolved from `[docs] reference_output`, not the `docs/reference` default this used to
    // hardcode: a consumer that publishes its reference pages elsewhere (one such tree renders
    // them into `docs-site/src/content/docs/reference`) got its READMEs bumped and its
    // API-reference badges left pinned to the previous release. ~keep
    for badge_file in sync_docs_version_badges(&crate::docs::reference_output_dir(config), version, writable) {
        state.updated.push(badge_file);
    }
    Ok(())
}

fn run_post_write_side_effects(state: &SyncState, version: &str) {
    if state.any_node_pkg_modified {
        run_optional("pnpm", &["install", "--no-frozen-lockfile", "--ignore-scripts", "-w"]);
    }
    if state.any_cargo_toml_modified {
        // Relocks every `Cargo.lock` `alef validate versions` will check (not just the root
        // workspace's), sharing that command's exact discovery so the write set and the
        // validate set can never diverge again. See alef #148.
        relock_cargo_lockfiles(version);
    }
    // Unconditional, unlike the gated call above: alef #1528 found that once a lock is left
    // `blocked_on_publish` by the run that first bumped its manifest, nothing ever revisits it on
    // a LATER `sync_versions` call that changes no manifest bytes at all -- which is every call
    // after the first, since the version is already correct on disk. `blocked_on_publish` is
    // re-derived fresh each time from whatever the lock and manifest currently disagree on, so it
    // cannot tell "still pending" from "published weeks ago, never relocked since"; only an actual
    // retry can. See `retry_blocked_lockfiles`'s doc for the full incident.
    retry_blocked_lockfiles(version);
    if state.any_composer_json_modified {
        run_optional("composer", &["update", "--lock", "--no-interaction"]);
    }
    if state.any_mix_exs_modified {
        run_optional("mix", &["deps.get"]);
    }
}

fn sync_registry_and_log(config_path: &std::path::Path, version: &str) {
    match sync_registry_package_versions(config_path, version) {
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
}

fn finalize_hashes_for_updated(config: &ResolvedCrateConfig, config_path: &std::path::Path, state: &mut SyncState) {
    let mut finalize_paths: std::collections::HashSet<std::path::PathBuf> =
        state.updated.iter().map(std::path::PathBuf::from).collect();
    finalize_paths.extend(std::mem::take(&mut state.text_replacement_paths));
    if finalize_paths.is_empty() {
        return;
    }
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    match super::super::cache::sources_hash(&config.source_hash_paths()) {
        Ok(sources_hash) => match super::generate::finalize_hashes(&finalize_paths, &sources_hash, &alef_toml_bytes) {
            Ok(n) if n > 0 => {
                debug!("  Finalized alef:hash in {n} file(s)");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("Could not finalize hashes after version sync: {e}");
            }
        },
        Err(e) => {
            warn!("Could not compute sources hash for finalize_hashes: {e}");
        }
    }
}

fn rebuild_ffi_if_needed(config: &ResolvedCrateConfig, updated: &[String]) {
    if updated.is_empty() || !config.languages.contains(&Language::Ffi) {
        return;
    }
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
    let _ = std::process::Command::new("cargo")
        .args(["build", "-p", &ffi_crate])
        .status();
}

fn resync_package_swift_placeholder_after_regen(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    if let Ok(content) = std::fs::read_to_string("Package.swift") {
        let placeholder_applied = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{version}"));
        let new_content = sync_swift_binary_release_url(&placeholder_applied, version).unwrap_or(placeholder_applied);
        if new_content != content {
            std::fs::write("Package.swift", &new_content)?;
            if !updated.iter().any(|p| p == "Package.swift") {
                updated.push("Package.swift".to_string());
            }
        }
    }
    Ok(())
}

fn run_regen_after_sync(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
    version: &str,
    skip_swift_checksum: bool,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
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

    resync_package_swift_placeholder_after_regen(version, updated)?;

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

    Ok(())
}

fn clear_readme_related_hashes() {
    let hashes_dir = std::path::Path::new(".alef").join("hashes");
    for stem in ["readme", "docs", "scaffold"] {
        for ext in [".hash", ".manifest", ".output_hashes"] {
            let p = hashes_dir.join(format!("{stem}{ext}"));
            if p.exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

fn regenerate_readmes_after_sync(config: &ResolvedCrateConfig, config_path: &std::path::Path) {
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
}

fn finalize_version_sync(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
    version: &str,
    no_regen: bool,
    skip_swift_checksum: bool,
    state: &mut SyncState,
) -> anyhow::Result<()> {
    run_post_write_side_effects(state, version);

    // `sync_registry_package_versions` must run *before* `finalize_hashes` below, not after it.
    // `finalize_hashes` re-derives `inputs_hash` by reading `config_path` (`alef.toml`) off disk
    // right before stamping — so whatever bytes are on disk at that moment become the hash every
    // file in `finalize_paths` is stamped against. `[crates.e2e.registry.packages.*].version` is a
    // real generation input (it feeds registry-mode test_apps content via
    // `E2eConfig::effective_package_for`), so it is correctly folded into `inputs_hash` — but only
    // if the bump has already landed on disk before that hash is computed. Running this after
    // `finalize_hashes` stamped every rewritten file against the *pre-bump* `inputs_hash`, so the
    // files this very run just wrote were born stale: the next `alef verify` recomputes
    // `inputs_hash` from the (by-then bumped) `alef.toml` and finds a mismatch immediately. ~keep
    sync_registry_and_log(config_path, version);

    finalize_hashes_for_updated(config, config_path, state);

    for file in &state.updated {
        info!("  Updated: {file}");
    }

    rebuild_ffi_if_needed(config, &state.updated);

    let last_path = std::path::Path::new(".alef").join("last_synced_version");
    let _ = crate::core::cache_dir::ensure_cache_dir(std::path::Path::new(".alef"));
    let _ = std::fs::write(&last_path, version);

    if !no_regen {
        run_regen_after_sync(config, config_path, version, skip_swift_checksum, &mut state.updated)?;
    }

    if state.updated.is_empty() {
        debug!("Versions already in sync — skipping README regeneration");
        return Ok(());
    }

    clear_readme_related_hashes();

    if !no_regen {
        regenerate_readmes_after_sync(config, config_path);
    }

    Ok(())
}
#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
