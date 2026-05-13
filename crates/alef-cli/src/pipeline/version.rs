use alef_core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::sync::LazyLock;
use tracing::{debug, info, warn};

use super::helpers::run_command;
use super::{extract, readme};

/// Regex for matching version field in Cargo.toml format files.
static CARGO_VERSION_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"(?m)^(version\s*=\s*)"[^"]*""#).expect("valid regex"));

/// Regex for matching semantic version strings.
static SEMVER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\d+\.\d+\.\d+(-[a-zA-Z0-9._]+)*").expect("valid regex"));

/// Read the version from a Cargo.toml file (workspace or regular package).
pub(crate) fn read_version(version_from: &str) -> anyhow::Result<String> {
    let content =
        std::fs::read_to_string(version_from).with_context(|| format!("failed to read version file {version_from}"))?;
    let value: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse TOML in {version_from}"))?;
    if let Some(v) = value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    if let Some(v) = value
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    anyhow::bail!("Could not find version in {version_from}")
}

/// Bump a semver version string by the given component (major, minor, patch).
fn bump_version(version: &str, component: &str) -> anyhow::Result<String> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("Invalid semver version: {version}");
    }
    let mut major: u64 = parts[0]
        .parse()
        .with_context(|| format!("Invalid major version component: {}", parts[0]))?;
    let mut minor: u64 = parts[1]
        .parse()
        .with_context(|| format!("Invalid minor version component: {}", parts[1]))?;
    let mut patch: u64 = parts[2]
        .parse()
        .with_context(|| format!("Invalid patch version component: {}", parts[2]))?;

    match component {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "patch" => {
            patch += 1;
        }
        other => anyhow::bail!("Unknown bump component '{other}': expected major, minor, or patch"),
    }

    Ok(format!("{major}.{minor}.{patch}"))
}

/// Write a bumped version back into a Cargo.toml (workspace or regular package).
fn write_version_to_cargo_toml(cargo_toml_path: &str, new_version: &str) -> anyhow::Result<()> {
    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("Failed to read {cargo_toml_path}"))?;

    // Match `version = "..."` as a standalone line (covers both [package] and [workspace.package])
    let new_content = CARGO_VERSION_RE
        .replace(&content, format!(r#"version = "{new_version}""#).as_str())
        .to_string();

    if new_content == content {
        anyhow::bail!("Could not find a `version = \"...\"` field to update in {cargo_toml_path}");
    }

    std::fs::write(cargo_toml_path, new_content)
        .with_context(|| format!("Failed to write updated version to {cargo_toml_path}"))?;

    Ok(())
}

/// Convert a semver pre-release version to PEP 440 format for Python/PyPI.
/// e.g., "0.1.0-rc.1" → "0.1.0rc1", "0.1.0-alpha.2" → "0.1.0a2", "0.1.0-beta.3" → "0.1.0b3"
/// Non-pre-release versions are returned unchanged.
///
/// Single-pass implementation: builds the result into one pre-allocated
/// `String` instead of chaining five `.replace()` calls (each of which
/// allocates a new intermediate `String`).
fn to_pep440(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };
    let mut out = String::with_capacity(base.len() + pre.len());
    out.push_str(base);
    let pre_norm = if let Some(rest) = pre.strip_prefix("alpha.").or_else(|| pre.strip_prefix("alpha")) {
        out.push('a');
        rest
    } else if let Some(rest) = pre.strip_prefix("beta.").or_else(|| pre.strip_prefix("beta")) {
        out.push('b');
        rest
    } else if let Some(rest) = pre.strip_prefix("rc.").or_else(|| pre.strip_prefix("rc")) {
        out.push_str("rc");
        rest
    } else {
        pre
    };
    for c in pre_norm.chars() {
        if c != '.' {
            out.push(c);
        }
    }
    out
}

use alef_core::version::{to_r_version, to_rubygems_prerelease};

/// Patch intra-workspace `version = "..."` pins inside a Cargo.toml dep table,
/// preserving all formatting and comments via `toml_edit`.
///
/// Only dep entries whose key is in `workspace_members` are touched. External
/// crates (e.g. `serde`, `tokio`) are left intact.
///
/// Handles these dep-table shapes:
/// - `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`
/// - `[target.'cfg(...)'.dependencies]` and the dev/build variants
/// - `[workspace.dependencies]` (root manifest only, included when present)
///
/// Returns `true` when at least one version pin was updated.
pub(crate) fn patch_workspace_dep_versions(
    cargo_toml_path: &str,
    new_version: &str,
    workspace_members: &std::collections::HashSet<String>,
) -> anyhow::Result<bool> {
    use toml_edit::{DocumentMut, Item};

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse TOML in {cargo_toml_path}"))?;

    let mut changed = false;

    // Patch a single dep-table item in-place. Returns true when any version was
    // updated. `dep_table` must be a `&mut Item` pointing at an inline or
    // regular TOML table of `{ dep-name = { version = "...", ... } }` entries.
    fn patch_dep_table(
        dep_table: &mut Item,
        new_version: &str,
        workspace_members: &std::collections::HashSet<String>,
    ) -> bool {
        let Some(table) = dep_table.as_table_like_mut() else {
            return false;
        };
        let mut any = false;
        for (key, item) in table.iter_mut() {
            // Only touch deps whose name is a workspace member.
            if !workspace_members.contains(key.get()) {
                continue;
            }
            // The dep value can be an inline table `{ path = "...", version = "X" }`.
            if let Some(inline) = item.as_table_like_mut() {
                if let Some(ver_item) = inline.get_mut("version") {
                    if ver_item.as_str() != Some(new_version) {
                        *ver_item = toml_edit::value(new_version);
                        any = true;
                    }
                }
            }
        }
        any
    }

    // Walk standard top-level dep tables.
    for table_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(item) = doc.get_mut(table_key) {
            if patch_dep_table(item, new_version, workspace_members) {
                changed = true;
            }
        }
    }

    // Walk [workspace.dependencies] (root manifest only).
    // We use a two-level path so we don't accidentally touch
    // `[workspace.package]` or other sibling keys.
    if let Some(workspace) = doc.get_mut("workspace") {
        if let Some(ws_table) = workspace.as_table_like_mut() {
            if let Some(deps) = ws_table.get_mut("dependencies") {
                if patch_dep_table(deps, new_version, workspace_members) {
                    changed = true;
                }
            }
        }
    }

    // Walk [target.'cfg(...)'.{dependencies,dev-dependencies,build-dependencies}].
    if let Some(target_item) = doc.get_mut("target") {
        if let Some(target_table) = target_item.as_table_like_mut() {
            // Collect the keys first to avoid borrow conflicts.
            let cfg_keys: Vec<String> = target_table.iter().map(|(k, _)| k.to_string()).collect();
            for cfg_key in cfg_keys {
                if let Some(cfg_item) = target_table.get_mut(&cfg_key) {
                    if let Some(cfg_table) = cfg_item.as_table_like_mut() {
                        for dep_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
                            if let Some(dep_item) = cfg_table.get_mut(dep_key) {
                                if patch_dep_table(dep_item, new_version, workspace_members) {
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if changed {
        std::fs::write(cargo_toml_path, doc.to_string())
            .with_context(|| format!("failed to write updated dep versions to {cargo_toml_path}"))?;
    }

    Ok(changed)
}

/// Verify that all package manifest versions match the Cargo.toml source of truth.
/// Returns a list of mismatches (empty = all consistent).
pub fn verify_versions(config: &ResolvedCrateConfig) -> anyhow::Result<Vec<String>> {
    let expected = read_version(&config.version_from)?;
    let expected_pep440 = to_pep440(&expected);
    let expected_rubygems = to_rubygems_prerelease(&expected);
    let mut mismatches = Vec::new();

    // Cache compiled regexes across calls within this verify pass — the same
    // ~15 patterns get reused on every invocation, and `Regex::new` is the
    // dominant cost when the function is called from a tight loop.
    fn extract_version(path: &str, pattern: &str) -> Option<String> {
        use std::collections::HashMap;
        use std::sync::Mutex;
        use std::sync::OnceLock;
        static CACHE: OnceLock<Mutex<HashMap<String, regex::Regex>>> = OnceLock::new();
        let content = std::fs::read_to_string(path).ok()?;
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().ok()?;
        let re = match guard.get(pattern) {
            Some(re) => re.clone(),
            None => {
                let re = regex::Regex::new(pattern).ok()?;
                guard.insert(pattern.to_string(), re.clone());
                re
            }
        };
        drop(guard);
        re.captures(&content)?.get(1).map(|m| m.as_str().to_string())
    }

    // Python (PEP 440 format)
    if let Some(found) = extract_version("packages/python/pyproject.toml", r#"version\s*=\s*"([^"]*)""#) {
        if found != expected_pep440 {
            mismatches.push(format!(
                "packages/python/pyproject.toml: found {found}, expected {expected_pep440}"
            ));
        }
    }

    // Node
    if let Some(found) = extract_version("packages/typescript/package.json", r#""version"\s*:\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/typescript/package.json: found {found}, expected {expected}"
            ));
        }
    }

    // Java
    if let Some(found) = extract_version("packages/java/pom.xml", r"<version>([^<]*)</version>") {
        if found != expected {
            mismatches.push(format!("packages/java/pom.xml: found {found}, expected {expected}"));
        }
    }

    // Elixir — check both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Some(found) = extract_version("packages/elixir/mix.exs", r#"version:\s*"([^"]*)""#)
        .or_else(|| extract_version("packages/elixir/mix.exs", r#"@version\s*"([^"]*)""#))
    {
        if found != expected {
            mismatches.push(format!("packages/elixir/mix.exs: found {found}, expected {expected}"));
        }
    }

    // Ruby gemspec (compare normalized form)
    if let Ok(entries) = std::fs::read_dir("packages/ruby") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gemspec") {
                if let Some(found) = extract_version(
                    &path.to_string_lossy(),
                    r"spec\.version\s*=\s*['\x22]([^'\x22]*)['\x22]",
                ) {
                    if found != expected_rubygems {
                        mismatches.push(format!(
                            "{}: found {found}, expected {expected_rubygems}",
                            path.display()
                        ));
                    }
                }
            }
        }
    }

    // Ruby version.rb files (packages/ruby/{lib/*/,ext/*/src/*/,ext/*/native/src/*/}version.rb) (compare normalized form)
    for pattern in &[
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        if let Ok(entries) = glob::glob(pattern) {
            for entry in entries.flatten() {
                if let Some(found) = extract_version(&entry.to_string_lossy(), r#"VERSION\s*=\s*["']([^"']*)["']"#) {
                    if found != expected_rubygems {
                        mismatches.push(format!(
                            "{}: found {found}, expected {expected_rubygems}",
                            entry.display()
                        ));
                    }
                }
            }
        }
    }

    // C# csproj
    if let Some(found) = extract_version(
        "packages/csharp/Kreuzcrawl/Kreuzcrawl.csproj",
        r"<Version>([^<]*)</Version>",
    ) {
        if found != expected {
            mismatches.push(format!("packages/csharp: found {found}, expected {expected}"));
        }
    }

    // PHP composer.json
    if let Some(found) = extract_version("packages/php/composer.json", r#""version"\s*:\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/php/composer.json: found {found}, expected {expected}"
            ));
        }
    }

    Ok(mismatches)
}

/// Set an explicit version in the Cargo.toml (supports pre-release versions like 0.1.0-rc.1).
pub fn set_version(config: &ResolvedCrateConfig, version: &str) -> anyhow::Result<()> {
    write_version_to_cargo_toml(&config.version_from, version)
        .with_context(|| format!("failed to set version to {version}"))?;
    info!("Set version to {version} in {}", config.version_from);
    Ok(())
}

/// Sync version from Cargo.toml to all package manifest files.
pub fn sync_versions(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
    bump: Option<&str>,
) -> anyhow::Result<()> {
    // If bump is requested, read current version, bump it, and write it back to Cargo.toml.
    if let Some(component) = bump {
        let current = read_version(&config.version_from)?;
        let bumped = bump_version(&current, component)?;
        info!("Bumping version {current} -> {bumped} ({component})");
        write_version_to_cargo_toml(&config.version_from, &bumped).context("failed to sync versions")?;
        info!("Updated {} with bumped version {bumped}", config.version_from);
    }

    let version = read_version(&config.version_from)?;

    // Always do the manifest scan. The previous warm-path short-circuit
    // checked `.alef/last_synced_version` and returned early when the
    // canonical version matched, which silently masked real drift:
    // a manifest hand-edited to the wrong version, a newly-added manifest
    // file (e.g. `e2e/rust/Cargo.toml` introduced after the last sync), or
    // a stale `alef:hash:` line all looked the same as "already synced"
    // because the cache key was only the version string. CI runs without
    // the cache, so it produced a different result and the alef-sync-versions
    // hook failed for downstream consumers. The scan is fast (sub-second
    // on kreuzberg-sized repos) and the work is idempotent when nothing
    // is actually stale.
    let last_path = std::path::Path::new(".alef").join("last_synced_version");
    info!("Syncing version {version}");

    let mut updated = vec![];

    // Workspace Cargo.toml files: sync [package] version in both members and excluded crates.
    // After updating [package] version, also patch intra-workspace dep version pins so that
    // entries like `kreuzberg = { path = "...", version = "X.Y.Z" }` get bumped to match.
    if let Ok(root_content) = std::fs::read_to_string("Cargo.toml") {
        if let Ok(root_toml) = root_content.parse::<toml::Table>() {
            let empty_vec = vec![];
            let members = root_toml
                .get("workspace")
                .and_then(|w| w.get("members"))
                .and_then(|m| m.as_array())
                .unwrap_or(&empty_vec);
            let excludes = root_toml
                .get("workspace")
                .and_then(|w| w.get("exclude"))
                .and_then(|m| m.as_array())
                .unwrap_or(&empty_vec);

            // Collect all workspace-member crate names so we can identify
            // which dep entries to bump. A crate name is the `name` field in
            // its [package] table; we derive it from the Cargo.toml path.
            let mut workspace_member_names: std::collections::HashSet<String> = std::collections::HashSet::new();
            for pattern_val in members.iter().chain(excludes.iter()) {
                if let Some(pattern) = pattern_val.as_str() {
                    if let Ok(paths) = glob::glob(&format!("{pattern}/Cargo.toml")) {
                        for entry in paths.flatten() {
                            if let Ok(member_content) = std::fs::read_to_string(&entry) {
                                if let Ok(member_toml) = member_content.parse::<toml::Table>() {
                                    if let Some(name) = member_toml
                                        .get("package")
                                        .and_then(|p| p.get("name"))
                                        .and_then(|n| n.as_str())
                                    {
                                        workspace_member_names.insert(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Collect all matching Cargo.toml paths for the member/exclude update pass.
            let mut cargo_toml_paths: Vec<String> = vec![];
            for pattern_val in members.iter().chain(excludes.iter()) {
                if let Some(pattern) = pattern_val.as_str() {
                    if let Ok(paths) = glob::glob(&format!("{pattern}/Cargo.toml")) {
                        for entry in paths.flatten() {
                            cargo_toml_paths.push(entry.to_string_lossy().to_string());
                        }
                    }
                }
            }

            for path_str in &cargo_toml_paths {
                // Update [package] version (regex-anchored to start-of-line).
                // Skip crates that use workspace version inheritance or have no version.
                if write_version_to_cargo_toml(path_str, &version).is_ok() && !updated.contains(path_str) {
                    updated.push(path_str.clone());
                }
                // Also patch intra-workspace dep version pins in all dep tables.
                if !workspace_member_names.is_empty() {
                    match patch_workspace_dep_versions(path_str, &version, &workspace_member_names) {
                        Ok(true) => {
                            if !updated.contains(path_str) {
                                updated.push(path_str.clone());
                            }
                        }
                        Ok(false) => {}
                        Err(e) => {
                            debug!("Could not patch dep versions in {path_str}: {e}");
                        }
                    }
                }
            }

            // Patch [workspace.dependencies] in the root Cargo.toml.
            if !workspace_member_names.is_empty() {
                match patch_workspace_dep_versions("Cargo.toml", &version, &workspace_member_names) {
                    Ok(true) => {
                        if !updated.contains(&"Cargo.toml".to_string()) {
                            updated.push("Cargo.toml".to_string());
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        debug!("Could not patch workspace dep versions in root Cargo.toml: {e}");
                    }
                }
            }
        }
    }

    // Python: pyproject.toml — convert semver pre-release to PEP 440 format
    // e.g., "0.1.0-rc.1" → "0.1.0rc1", "0.1.0-alpha.2" → "0.1.0a2", "0.1.0-beta.3" → "0.1.0b3"
    let python_version = to_pep440(&version);
    if let Ok(content) = std::fs::read_to_string("packages/python/pyproject.toml") {
        if let Some(new_content) = replace_version_pattern(&content, r#"version = "[^"]*""#, &python_version) {
            std::fs::write("packages/python/pyproject.toml", &new_content)
                .context("failed to write packages/python/pyproject.toml")?;
            updated.push("packages/python/pyproject.toml".to_string());
        }
    }

    // Node: package.json — use the configured Node package_dir, falling back
    // to "packages/node" (the modern default) and "packages/typescript" (legacy
    // path retained so older repos that still use the old default keep syncing).
    let node_pkg_dir = config.package_dir(Language::Node);
    let mut node_paths: Vec<String> = vec![format!("{node_pkg_dir}/package.json")];
    if node_pkg_dir != "packages/typescript" {
        node_paths.push("packages/typescript/package.json".to_string());
    }
    for node_path in node_paths {
        if let Ok(content) = std::fs::read_to_string(&node_path) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
                std::fs::write(&node_path, &new_content).with_context(|| format!("failed to write {node_path}"))?;
                updated.push(node_path);
            }
        }
    }

    // Ruby: *.gemspec (convert to RubyGems prerelease format)
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

    // Ruby: {lib/*/,ext/*/src/*/,ext/*/native/src/*/}version.rb (convert to RubyGems prerelease format)
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

    // Ruby: Gemfile.lock — update the path-gem version entries so bundler does not
    // reject the lockfile with "frozen mode" errors on the next CI run.
    // The lockfile contains the gem version in two places:
    //   1. Under PATH > specs: `    <name> (<version>)` (4-space indent)
    //   2. Under CHECKSUMS:    `  <name> (<version>)` (2-space indent, no sha256)
    // We replace both textually, reusing the already-computed ruby_version.
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

    // PHP: composer.json
    if let Ok(content) = std::fs::read_to_string("packages/php/composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
            std::fs::write("packages/php/composer.json", &new_content)?;
            updated.push("packages/php/composer.json".to_string());
        }
    }

    // Elixir: mix.exs — handle both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Ok(content) = std::fs::read_to_string("packages/elixir/mix.exs") {
        if let Some(new_content) = replace_version_pattern(&content, r#"version: "[^"]*""#, &version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
        } else if let Some(new_content) = replace_version_pattern(&content, r#"@version "[^"]*""#, &version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
        }
    }

    // Go: go.mod (no version field, skip)

    // Java: pom.xml
    if let Ok(content) = std::fs::read_to_string("packages/java/pom.xml") {
        if let Some(new_content) = replace_version_pattern(&content, r#"<version>[^<]*</version>"#, &version) {
            std::fs::write("packages/java/pom.xml", &new_content)?;
            updated.push("packages/java/pom.xml".to_string());
        }
    }

    // C#: *.csproj (recursive under packages/csharp)
    for entry in glob::glob("packages/csharp/**/*.csproj")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            if let Some(new_content) = replace_version_pattern(&content, r#"<Version>[^<]*</Version>"#, &version) {
                std::fs::write(&entry, &new_content)?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }

    // WASM: package.json
    for wasm_pkg in glob::glob("crates/*-wasm/package.json").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&wasm_pkg) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
                std::fs::write(&wasm_pkg, &new_content)?;
                updated.push(wasm_pkg.to_string_lossy().to_string());
            }
        }
    }

    // Node binding crate manifest: crates/*-node/package.json. Some repos keep
    // a `package.json` next to the NAPI-RS binding crate (alongside the Cargo
    // manifest) so `npm publish --workspace` can resolve the prebuilt binary.
    // `validate-versions` already checks this file; sync must write to it too.
    for node_pkg in glob::glob("crates/*-node/package.json").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&node_pkg) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
                std::fs::write(&node_pkg, &new_content)?;
                updated.push(node_pkg.to_string_lossy().to_string());
            }
        }
    }

    // Root package.json (if present): typically a private "root" manifest
    // bookkeeping pnpm workspaces alongside the published bindings. Without
    // this, `validate-versions` flags a mismatch every release because the
    // root manifest carries its own `"version"` that nothing else writes to.
    if let Ok(content) = std::fs::read_to_string("package.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("package.json", &new_content)?;
            updated.push("package.json".to_string());
        }
    }

    // Root composer.json (if present)
    if let Ok(content) = std::fs::read_to_string("composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("composer.json", &new_content)?;
            updated.push("composer.json".to_string());
        }
    }

    // R: DESCRIPTION file — CRAN rejects SemVer dash prereleases.
    if let Ok(content) = std::fs::read_to_string("packages/r/DESCRIPTION") {
        let r_version = to_r_version(&version);
        if let Some(new_content) = replace_version_pattern(&content, r"Version:\s*[^\n]*", &r_version) {
            std::fs::write("packages/r/DESCRIPTION", &new_content)?;
            updated.push("packages/r/DESCRIPTION".to_string());
        }
    }

    // Python: __init__.py
    if let Ok(content) = std::fs::read_to_string("packages/python/__init__.py") {
        if let Some(new_content) = replace_version_pattern(&content, r#"__version__\s*=\s*"[^"]*""#, &version) {
            std::fs::write("packages/python/__init__.py", &new_content)?;
            updated.push("packages/python/__init__.py".to_string());
        }
    }

    // Go: ffi_loader.go
    if let Ok(content) = std::fs::read_to_string("packages/go/ffi_loader.go") {
        if let Some(new_content) = replace_version_pattern(&content, r#"defaultFFIVersion\s*=\s*"[^"]*""#, &version) {
            std::fs::write("packages/go/ffi_loader.go", &new_content)?;
            updated.push("packages/go/ffi_loader.go".to_string());
        }
    }

    // Process extra_paths from config [sync] section (glob patterns)
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
                                        // For package.json files, only update the top-level
                                        // "version" field to avoid clobbering dependency versions.
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
                                        // Cargo.toml: only update [package] version (line-anchored).
                                        // Never use replace_all — it corrupts dependency version specs.
                                        let path_str = path.to_string_lossy().to_string();
                                        if write_version_to_cargo_toml(&path_str, &version).is_ok() {
                                            updated.push(path_str);
                                        }
                                    } else if file_name == "pyproject.toml" {
                                        // pyproject.toml: only update the `version = "..."` field.
                                        // Never do blanket regex replace — it corrupts requires-python
                                        // and dependency version specifiers.
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
                                        // Ruby version.rb: gem-formatted, replace VERSION constant only.
                                        // Never use SEMVER_RE — `0.3.0` in `0.3.0.pre.rc.2` would re-acquire
                                        // a dash-form prerelease, corrupting the gem version.
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
                                        // gemspec: gem-formatted, replace spec.version only.
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

        // Process text_replacements from config [sync] section
        for replacement in &sync_config.text_replacements {
            match glob::glob(&replacement.path) {
                Ok(paths) => {
                    for entry in paths {
                        match entry {
                            Ok(path) => {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    let search = replacement.search.replace("{version}", &version);
                                    let replace = replacement.replace.replace("{version}", &version);
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

    // Finalize alef:hash lines in every file that carries the alef header and
    // was rewritten by this sync. Without this, alef-verify would see a stale
    // hash because the version string changed but the hash was not updated.
    let updated_paths: std::collections::HashSet<std::path::PathBuf> =
        updated.iter().map(std::path::PathBuf::from).collect();
    if !updated_paths.is_empty() {
        match super::super::cache::sources_hash(&config.sources) {
            Ok(sources_hash) => match super::generate::finalize_hashes(&updated_paths, &sources_hash) {
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

    for file in &updated {
        info!("  Updated: {file}");
    }

    // Rebuild FFI to refresh C headers (cbindgen) if FFI language is configured
    // AND something actually changed. Skip when versions were already in sync —
    // a warm rerun should not invoke cargo at all.
    if !updated.is_empty() && config.languages.contains(&Language::Ffi) {
        let ffi_crate = config
            .explicit_output
            .ffi
            .as_ref()
            .and_then(|p| {
                // Output path is like "crates/html-to-markdown-ffi/src/" — get the crate dir name
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

    // Stamp the last-synced version so the next warm run can skip the entire
    // glob+regex pass without re-stat'ing every manifest.
    let _ = std::fs::create_dir_all(".alef");
    let _ = std::fs::write(&last_path, &version);

    // If no manifest actually changed, nothing else needs refreshing — the
    // generated README/docs/binding hashes still match. This is the warm-path
    // fast exit: hundreds of files were already on disk with the right
    // version, so we skip the cache wipe + README regeneration entirely.
    if updated.is_empty() {
        debug!("Versions already in sync — skipping README regeneration");
        return Ok(());
    }

    // Selective cache invalidation: only README (and stage caches that embed
    // version strings) are stale after a sync. Leave the IR cache and the
    // per-language binding hashes in place so the next `alef generate` does
    // not have to re-extract or re-emit unchanged backends.
    let hashes_dir = std::path::Path::new(".alef").join("hashes");
    for stem in ["readme", "docs", "scaffold"] {
        for ext in [".hash", ".manifest", ".output_hashes"] {
            let p = hashes_dir.join(format!("{stem}{ext}"));
            if p.exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    // Regenerate READMEs with the new version.
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

/// Internal helper to regenerate READMEs after a version sync.
/// Extracts IR, computes README files, and writes them to disk.
fn regenerate_readmes(config: &ResolvedCrateConfig, config_path: &std::path::Path) -> anyhow::Result<usize> {
    let api = extract(config, config_path, false)?;
    let languages = config.languages.clone();
    let readme_files = readme(&api, config, &languages)?;
    let base_dir = std::path::PathBuf::from(".");
    let _ = config_path; // unused now that the embedded hash is per-file content-derived
    let sources_hash = super::super::cache::sources_hash(&config.sources)?;
    let count = super::generate::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
    let paths: std::collections::HashSet<std::path::PathBuf> =
        readme_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&paths, &sources_hash)?;
    Ok(count)
}

/// Update all `<gem-name> (<old-version>)` entries in a Gemfile.lock to `new_ruby_version`.
///
/// Gemfile.lock records the path-gem version in two places:
///
/// 1. Under `PATH > specs:` — four-space indent, may include dependency lines below it.
/// 2. Under `CHECKSUMS` — two-space indent, no sha256 suffix (path gems are not downloaded).
///
/// Both patterns look like `  <name> (<version>)` with varying indentation. We replace
/// every occurrence of `<name> (<old>)` with `<name> (<new>)` regardless of indent, so
/// the function handles any future Gemfile.lock layout changes automatically.
///
/// Returns `Some(new_content)` when at least one substitution was made, `None` when the
/// lockfile already contains the target version everywhere (idempotent).
fn sync_gemfile_lock(content: &str, new_ruby_version: &str) -> Option<String> {
    // Build a regex that matches `<gem-name> (<any-version>)` on a word boundary
    // so we never accidentally match a gem whose name is a prefix of another.
    // The gem name is captured from the first occurrence we find in the file
    // (the PATH > specs block always appears first).
    use std::sync::LazyLock;
    static GEM_VERSION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        // Matches: optional leading whitespace + gem-name + space + (version)
        // Capture group 1 = gem name, group 2 = version inside parens.
        regex::Regex::new(r"(?m)^([ \t]*)([A-Za-z0-9_-]+) \(([^)]+)\)$").expect("valid regex")
    });

    // Collect the set of gem names that appear in the PATH block (path gems).
    // PATH block starts with "^PATH" and ends at the next blank line or new section.
    let path_gem_names: std::collections::HashSet<String> = {
        let mut names = std::collections::HashSet::new();
        let mut in_specs = false;
        for line in content.lines() {
            if line.trim_start().starts_with("specs:") {
                // Only enter specs-tracking mode when we are in a PATH block, which
                // always appears before GEM. A simple heuristic: the PATH section
                // starts with "^PATH" (no indent). Track whether we saw PATH before
                // seeing GEM.
            }
            if line == "PATH" {
                in_specs = true;
                continue;
            }
            if in_specs && line.starts_with("  specs:") {
                continue;
            }
            if in_specs && line.starts_with("    ") {
                // Four-space indent — these are gem entries in the PATH specs block.
                if let Some(caps) = GEM_VERSION_RE.captures(line) {
                    let indent = &caps[1];
                    let name = &caps[2];
                    if indent.len() == 4 {
                        names.insert(name.to_string());
                    }
                }
                continue;
            }
            // A line without four-space indent ends the PATH > specs block.
            if in_specs
                && !line.starts_with("    ")
                && !line.trim().is_empty()
                && line != "PATH"
                && !line.starts_with("  ")
            {
                // Top-level section header — PATH block is done.
                in_specs = false;
            }
        }
        names
    };

    if path_gem_names.is_empty() {
        return None;
    }

    let mut changed = false;
    let new_content = content
        .lines()
        .map(|line| {
            if let Some(caps) = GEM_VERSION_RE.captures(line) {
                let gem_name = &caps[2];
                let current_version = &caps[3];
                if path_gem_names.contains(gem_name) && current_version != new_ruby_version {
                    changed = true;
                    // Reconstruct the line with the new version, preserving indent.
                    let indent = &caps[1];
                    return format!("{indent}{gem_name} ({new_ruby_version})");
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Preserve trailing newline if the original had one.
    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };

    if changed { Some(new_content) } else { None }
}

/// Replace version pattern in content. Returns `Some(new_content)` only when
/// the regex match exists *and* the captured version string actually differs
/// from the target. This is the idempotency guard against:
///   1. backend codegen that emits a manifest with the right value but in a
///      slightly different syntactic form (e.g. Magnus emits `VERSION =
///      "4.10.0.pre.rc.9"` while the regex's replacement template uses
///      single-quotes); without this guard the two paths ping-pong and every
///      warm `alef generate` rewrites the manifest, triggers README regen,
///      and looks like real drift to downstream tooling.
///   2. trivial round-trips where new content == old content despite the
///      regex matching.
fn replace_version_pattern(content: &str, pattern: &str, version: &str) -> Option<String> {
    let regex = regex::Regex::new(pattern).ok()?;
    let captures = regex.captures(content)?;
    let matched = captures.get(0)?.as_str();
    // Extract the version literal (text between the first pair of quotes or
    // angle/colon delimiters) and short-circuit when it already equals the
    // target. This way `VERSION = "x"` and `VERSION = 'x'` both count as
    // "already in sync" when x matches, regardless of quote style.
    if matched_version_equals(matched, version) {
        return None;
    }

    let replacement = match pattern {
        p if p.contains("version =") && !p.contains("spec") && !p.contains("VERSION") => {
            format!(r#"version = "{version}""#)
        }
        p if p.contains("\"version\"") && p.contains("\"") => format!(r#""version": "{version}""#),
        p if p.contains("spec") => format!("spec.version = '{version}'"),
        p if p.contains("<version>") => format!("<version>{version}</version>"),
        p if p.contains("<Version>") => format!("<Version>{version}</Version>"),
        p if p.contains("@version") => format!(r#"@version "{version}""#),
        p if p.contains("version:") && p.contains(":") => format!(r#"version: "{version}""#),
        p if p.contains("__version__") => format!(r#"__version__ = "{version}""#),
        p if p.contains("defaultFFIVersion") => format!(r#"defaultFFIVersion = "{version}""#),
        p if p.contains("Version:") => format!("Version: {version}"),
        p if p.contains("VERSION") => format!("VERSION = '{version}'"),
        _ => return None,
    };

    let new_content = regex.replace(content, replacement.as_str()).to_string();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Extract the version-literal substring from a regex match string and decide
/// whether it already equals `target`. The match string is something like
/// `VERSION = "1.2.3"`, `version = "1.2.3"`, `<version>1.2.3</version>`,
/// `Version: 1.2.3`. We look for the first chunk after the delimiter and
/// compare it to `target`; quote style is irrelevant.
fn matched_version_equals(matched: &str, target: &str) -> bool {
    extract_version_literal(matched).is_some_and(|v| v == target)
}

fn extract_version_literal(matched: &str) -> Option<&str> {
    // Try paired-quote form first ("..." or '...').
    if let Some(start) = matched.find(['"', '\'']) {
        let quote = matched.as_bytes()[start];
        let rest = &matched[start + 1..];
        if let Some(end) = rest.find(quote as char) {
            return Some(&rest[..end]);
        }
    }
    // Try angle-bracket form (<version>...</version> or <Version>...</Version>).
    if let Some(close) = matched.find('>') {
        let rest = &matched[close + 1..];
        if let Some(end) = rest.find('<') {
            return Some(&rest[..end]);
        }
    }
    // Try colon-delimited form (`Version: 1.2.3`).
    if let Some(colon) = matched.find(':') {
        return Some(matched[colon + 1..].trim());
    }
    // Try `=` delimited unquoted form.
    if let Some(eq) = matched.find('=') {
        return Some(matched[eq + 1..].trim());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::generate;

    #[test]
    fn to_pep440_no_prerelease_passthrough() {
        assert_eq!(to_pep440("1.2.3"), "1.2.3");
        assert_eq!(to_pep440("0.1.0"), "0.1.0");
    }

    #[test]
    fn to_pep440_rc_prerelease() {
        assert_eq!(to_pep440("0.1.0-rc.1"), "0.1.0rc1");
        assert_eq!(to_pep440("4.10.0-rc.9"), "4.10.0rc9");
    }

    #[test]
    fn to_pep440_alpha_beta_prerelease() {
        assert_eq!(to_pep440("1.0.0-alpha.2"), "1.0.0a2");
        assert_eq!(to_pep440("1.0.0-beta.3"), "1.0.0b3");
    }

    #[test]
    fn to_pep440_strips_internal_dots() {
        assert_eq!(to_pep440("0.1.0-rc.1.2"), "0.1.0rc12");
    }

    #[test]
    fn matched_version_equals_treats_quote_style_uniformly() {
        assert!(matched_version_equals("VERSION = '1.0.0'", "1.0.0"));
        assert!(matched_version_equals("VERSION = \"1.0.0\"", "1.0.0"));
        assert!(!matched_version_equals("VERSION = '1.0.0'", "2.0.0"));
        assert!(matched_version_equals("<version>1.0.0</version>", "1.0.0"));
        assert!(matched_version_equals("Version: 1.0.0", "1.0.0"));
    }

    #[test]
    fn test_replace_version_pattern_ruby_version() {
        let content = r#"# This file is auto-generated by alef
module Kreuzberg
  VERSION = "1.0.0"
end
"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert_eq!(
            new_content,
            r#"# This file is auto-generated by alef
module Kreuzberg
  VERSION = '2.0.0'
end
"#
        );
    }

    #[test]
    fn test_replace_version_pattern_ruby_version_single_quotes() {
        let content = "VERSION = '1.5.2'";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        // rubocop Style/StringLiterals: Ruby VERSION constants use single quotes
        assert_eq!(new_content, "VERSION = '2.0.0'");
    }

    #[test]
    fn test_replace_version_pattern_ruby_version_double_quotes() {
        let content = "VERSION = \"1.5.2\"";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "3.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        // rubocop Style/StringLiterals: output normalised to single quotes regardless of input
        assert_eq!(new_content, "VERSION = '3.0.0'");
    }

    #[test]
    fn test_replace_version_pattern_ruby_in_module() {
        let content = r#"module MyGem
  VERSION = "0.5.0"
end"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "1.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert!(new_content.contains("VERSION = '1.0.0'"));
        assert!(!new_content.contains("0.5.0"));
    }

    #[test]
    fn test_replace_version_pattern_no_match() {
        let content = "NOTHING = \"1.0.0\"";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_none());
    }

    #[test]
    fn test_replace_version_pattern_preserves_other_content() {
        let content = r#"# frozen_string_literal: true
module Kreuzberg
  VERSION = "1.0.0"
  # Other stuff
  CONST = "something"
end"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert!(new_content.contains("# frozen_string_literal: true"));
        assert!(new_content.contains("CONST = \"something\""));
        assert!(new_content.contains("VERSION = '2.0.0'"));
    }

    /// Verify that `finalize_hashes` updates the alef:hash line in a file that
    /// carries the alef header marker. This exercises the mechanism used by
    /// `sync_versions` to refresh stale hashes after rewriting version strings.
    #[test]
    fn test_finalize_hashes_updates_alef_hash_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("version.rb");

        // Simulate a version.rb that was written by alef with an alef header
        // but no hash line yet (as written by write_scaffold_files_with_overwrite
        // before finalize_hashes runs).
        let content = "# This file is auto-generated by alef — do not edit manually.\n# frozen_string_literal: true\n\nmodule MyGem\n  VERSION = '2.0.0'\nend\n";
        std::fs::write(&path, content).expect("write");

        let paths: std::collections::HashSet<std::path::PathBuf> = std::iter::once(path.clone()).collect();
        let n = generate::finalize_hashes(&paths, "test-sources-hash").expect("finalize ok");
        assert_eq!(n, 1, "finalize_hashes must update the file with the alef:hash line");

        let updated = std::fs::read_to_string(&path).expect("read");
        assert!(
            updated.contains("alef:hash:"),
            "file must contain alef:hash: after finalize_hashes, got:\n{updated}"
        );
    }

    /// Verify that `finalize_hashes` is idempotent: running it twice on the same
    /// file must not change the hash line on the second run.
    #[test]
    fn test_finalize_hashes_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("version.rb");

        let content =
            "# This file is auto-generated by alef — do not edit manually.\n\nmodule MyGem\n  VERSION = '2.0.0'\nend\n";
        std::fs::write(&path, content).expect("write");

        let paths: std::collections::HashSet<std::path::PathBuf> = std::iter::once(path.clone()).collect();

        let _ = generate::finalize_hashes(&paths, "sources").expect("first finalize");
        let after_first = std::fs::read_to_string(&path).expect("read after first");

        let n2 = generate::finalize_hashes(&paths, "sources").expect("second finalize");
        assert_eq!(n2, 0, "second finalize_hashes must be a no-op (same hash)");

        let after_second = std::fs::read_to_string(&path).expect("read after second");
        assert_eq!(after_first, after_second, "content must not change on second finalize");
    }

    const GEMFILE_LOCK_SAMPLE: &str = "\
PATH
  remote: .
  specs:
    kreuzberg (4.10.0.pre.rc.13)
      rb_sys (~> 0.9)

GEM
  remote: https://rubygems.org/
  specs:
    rake (13.4.2)

PLATFORMS
  ruby

DEPENDENCIES
  kreuzberg!

CHECKSUMS
  kreuzberg (4.10.0.pre.rc.13)
  rake (13.4.2) sha256=abcdef

BUNDLED WITH
  4.0.7
";

    #[test]
    fn sync_gemfile_lock_updates_both_occurrences() {
        let result = sync_gemfile_lock(GEMFILE_LOCK_SAMPLE, "4.10.0.pre.rc.14");
        assert!(result.is_some(), "expected Some when version changes");
        let new = result.unwrap();
        // PATH > specs entry updated
        assert!(
            new.contains("    kreuzberg (4.10.0.pre.rc.14)"),
            "PATH specs entry not updated:\n{new}"
        );
        // CHECKSUMS entry updated
        assert!(
            new.contains("  kreuzberg (4.10.0.pre.rc.14)"),
            "CHECKSUMS entry not updated:\n{new}"
        );
        // Other gem versions are unchanged
        assert!(
            new.contains("rake (13.4.2)"),
            "non-path gem version must not change:\n{new}"
        );
        // Old version must be gone
        assert!(!new.contains("4.10.0.pre.rc.13"), "old version must be removed:\n{new}");
    }

    #[test]
    fn sync_gemfile_lock_is_idempotent() {
        let first = sync_gemfile_lock(GEMFILE_LOCK_SAMPLE, "4.10.0.pre.rc.14").unwrap();
        let second = sync_gemfile_lock(&first, "4.10.0.pre.rc.14");
        assert!(
            second.is_none(),
            "second call with same version must return None (already in sync)"
        );
    }

    #[test]
    fn sync_gemfile_lock_preserves_trailing_newline() {
        let with_newline = format!("{GEMFILE_LOCK_SAMPLE}\n");
        let result = sync_gemfile_lock(&with_newline, "4.10.0.pre.rc.99").unwrap();
        assert!(result.ends_with('\n'), "trailing newline must be preserved");
    }

    #[test]
    fn sync_gemfile_lock_no_path_gem_returns_none() {
        // A Gemfile.lock with no PATH block — nothing to sync.
        let content = "GEM\n  remote: https://rubygems.org/\n  specs:\n    rake (13.4.2)\n";
        let result = sync_gemfile_lock(content, "1.0.0");
        assert!(result.is_none(), "no PATH gem means nothing to update");
    }

    /// Root `package.json` is a private "root" pnpm-workspace bookkeeping
    /// manifest. It carries its own top-level `"version"` that must track the
    /// canonical Cargo.toml version so `validate-versions` does not flag a
    /// drift on every release. The replacement must not touch nested
    /// `"version"` fields inside `devDependencies` / `pnpm.overrides` / etc.
    #[test]
    fn test_replace_version_pattern_root_package_json_only_top_level() {
        let content = r#"{
  "name": "kreuzberg-root",
  "version": "4.9.5",
  "private": true,
  "devDependencies": {
    "@vitest/coverage-v8": "^4.1.5",
    "tsx": "^4.21.0",
    "typescript": "^6.0.3"
  },
  "pnpm": {
    "overrides": {
      "glob": "10.5.0"
    }
  }
}
"#;
        let new_content = replace_version_pattern(content, r#""version":\s*"[^"]*""#, "5.0.0-rc.1")
            .expect("root package.json version must update");
        assert!(
            new_content.contains(r#""version": "5.0.0-rc.1""#),
            "top-level version must be rewritten, got:\n{new_content}"
        );
        assert!(
            !new_content.contains(r#""version": "4.9.5""#),
            "old version must be removed, got:\n{new_content}"
        );
        // Nested dependency versions and pnpm overrides must remain intact.
        assert!(
            new_content.contains("\"@vitest/coverage-v8\": \"^4.1.5\""),
            "devDependency version specs must not be touched, got:\n{new_content}"
        );
        assert!(
            new_content.contains("\"glob\": \"10.5.0\""),
            "pnpm overrides must not be touched, got:\n{new_content}"
        );
    }

    /// Serialize tests that mutate process-global CWD. `std::env::set_current_dir`
    /// is shared across the test binary, so concurrent tempdir-based `sync_versions`
    /// tests would race without this guard.
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// End-to-end: `sync_versions` must rewrite both `package.json` (root) and
    /// every `crates/*-node/package.json` file alongside the existing manifests.
    /// Regression test for the kreuzberg publish.yaml dry-run failure where the
    /// root manifest stayed at 4.9.5 while Cargo.toml jumped to 5.0.0-rc.1.
    #[test]
    fn sync_versions_writes_root_and_node_crate_package_json() {
        use alef_core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Minimal workspace: Cargo.toml at canonical "1.0.0", root package.json
        // and crates/mylib-node/package.json both stale at "0.9.0".
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(
            root.join("package.json"),
            "{\n  \"name\": \"mylib-root\",\n  \"version\": \"0.9.0\",\n  \"private\": true\n}\n",
        )
        .expect("write root package.json");
        std::fs::create_dir_all(root.join("crates/mylib-node")).expect("mkdir crates/mylib-node");
        std::fs::write(
            root.join("crates/mylib-node/package.json"),
            "{\n  \"name\": \"mylib\",\n  \"version\": \"0.9.0\"\n}\n",
        )
        .expect("write crates/mylib-node/package.json");

        // Drop a minimal alef.toml so we can resolve a config.
        let alef_toml = format!(
            "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display()
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        // Switch into the tempdir for the duration of the call — sync_versions
        // resolves relative paths against CWD.
        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None);
        // Always restore the CWD before unwrapping, so a panic doesn't leave
        // the test runner in a broken directory.
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let root_pkg = std::fs::read_to_string(root.join("package.json")).expect("read root package.json");
        assert!(
            root_pkg.contains(r#""version": "1.0.0""#),
            "root package.json must be bumped to canonical version, got:\n{root_pkg}"
        );
        assert!(
            !root_pkg.contains("0.9.0"),
            "old version must be gone from root package.json, got:\n{root_pkg}"
        );

        let node_pkg = std::fs::read_to_string(root.join("crates/mylib-node/package.json"))
            .expect("read crates/mylib-node/package.json");
        assert!(
            node_pkg.contains(r#""version": "1.0.0""#),
            "crates/*-node/package.json must be bumped to canonical version, got:\n{node_pkg}"
        );
    }

    // -----------------------------------------------------------------------
    // patch_workspace_dep_versions unit tests
    // -----------------------------------------------------------------------

    /// patch_workspace_dep_versions updates [dependencies], [dev-dependencies],
    /// [build-dependencies], [target.*.dependencies], and [workspace.dependencies]
    /// but leaves external crate pins intact.
    #[test]
    fn patch_workspace_dep_versions_all_dep_table_shapes() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().expect("tempdir");

        let cargo_toml = r#"[package]
name = "crate-a"
version = "5.0.0-rc.1"

[dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1", optional = true }
serde = "1.0"

[dev-dependencies]
crate-c = { path = "../crate-c", version = "5.0.0-rc.1" }
tempfile = "3"

[build-dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1" }

[target.'cfg(unix)'.dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1", optional = true }
libc = "0.2"

[workspace.dependencies]
crate-c = { path = "../crate-c", version = "5.0.0-rc.1", default-features = false }
tokio = { version = "1.0", features = ["full"] }
"#;

        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, cargo_toml).expect("write");

        let members: HashSet<String> = ["crate-b", "crate-c"].iter().map(|s| s.to_string()).collect();

        let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "5.0.0-rc.2", &members).expect("patch ok");

        assert!(changed, "at least one version pin must have been updated");

        let result = std::fs::read_to_string(&path).expect("read");

        // [package] version is NOT touched by patch_workspace_dep_versions — only dep tables.
        // All workspace member dep-table pins must be bumped to rc.2.
        // crate-b appears in [dependencies], [build-dependencies], and [target.*.dependencies].
        let crate_b_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.contains("crate-b") && l.contains("version"))
            .collect();
        assert!(
            !crate_b_lines.is_empty(),
            "expected crate-b dep lines with version=:\n{result}"
        );
        for line in &crate_b_lines {
            assert!(
                line.contains("5.0.0-rc.2"),
                "crate-b pin not bumped:\n  {line}\nfull:\n{result}"
            );
        }
        // crate-c appears in [dev-dependencies] and [workspace.dependencies].
        let crate_c_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.contains("crate-c") && l.contains("version"))
            .collect();
        assert!(
            !crate_c_lines.is_empty(),
            "expected crate-c dep lines with version=:\n{result}"
        );
        for line in &crate_c_lines {
            assert!(
                line.contains("5.0.0-rc.2"),
                "crate-c pin not bumped:\n  {line}\nfull:\n{result}"
            );
        }

        // External crates must be untouched.
        assert!(
            result.contains(r#"serde = "1.0""#),
            "serde must not be touched:\n{result}"
        );
        assert!(
            result.contains(r#"tempfile = "3""#),
            "tempfile must not be touched:\n{result}"
        );
        assert!(
            result.contains(r#"libc = "0.2""#),
            "libc must not be touched:\n{result}"
        );
        assert!(
            result.contains(r#"tokio = { version = "1.0", features = ["full"] }"#),
            "tokio must not be touched:\n{result}"
        );
    }

    /// patch_workspace_dep_versions is idempotent: calling it twice with the
    /// same target version returns false and does not rewrite the file.
    #[test]
    fn patch_workspace_dep_versions_is_idempotent() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().expect("tempdir");

        let cargo_toml = "[package]\nname = \"crate-a\"\nversion = \"5.0.0-rc.2\"\n\n[dependencies]\ncrate-b = { path = \"../crate-b\", version = \"5.0.0-rc.2\" }\n";

        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, cargo_toml).expect("write");

        let members: HashSet<String> = std::iter::once("crate-b".to_string()).collect();

        let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "5.0.0-rc.2", &members).expect("patch ok");
        assert!(!changed, "no change expected when already at target version");
    }

    /// patch_workspace_dep_versions does not touch path-only deps (no version= key).
    #[test]
    fn patch_workspace_dep_versions_skips_path_only_deps() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().expect("tempdir");

        let cargo_toml = "[package]\nname = \"crate-a\"\nversion = \"1.0.0\"\n\n[dependencies]\ncrate-b = { path = \"../crate-b\" }\n";

        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, cargo_toml).expect("write");

        let members: HashSet<String> = std::iter::once("crate-b".to_string()).collect();

        let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "2.0.0", &members).expect("patch ok");
        assert!(!changed, "path-only deps without version= must not be touched");
    }

    // -----------------------------------------------------------------------
    // sync_versions dep-table end-to-end test
    // -----------------------------------------------------------------------

    /// Full workspace e2e: after sync_versions the version bump propagates from
    /// [workspace.package] into [workspace.dependencies] and all dep-table shapes
    /// in member crates. External pins must be untouched.
    #[test]
    fn sync_versions_patches_dep_tables_on_version_change() {
        use alef_core::config::NewAlefConfig;

        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fn write_file(dir: &std::path::Path, rel: &str, content: &str) {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(path, content).expect("write");
        }

        // Root Cargo.toml: canonical version already at rc.2 (simulates task version:set).
        write_file(
            root,
            "Cargo.toml",
            "[workspace.package]\nversion = \"5.0.0-rc.2\"\n\n[workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n\n[workspace.dependencies]\nalpha = { path = \"crates/alpha\", version = \"5.0.0-rc.1\", default-features = false }\nserde = \"1.0\"\n",
        );

        // crates/alpha: upstream crate, no intra-workspace deps.
        write_file(
            root,
            "crates/alpha/Cargo.toml",
            "[package]\nname = \"alpha\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\nserde = \"1.0\"\n",
        );

        // crates/beta: all four dep-table shapes referencing alpha.
        write_file(
            root,
            "crates/beta/Cargo.toml",
            "[package]\nname = \"beta\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\", optional = true }\nserde = \"1.0\"\n\n[dev-dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\" }\ntempfile = \"3\"\n\n[build-dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\" }\n\n[target.'cfg(unix)'.dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\", features = [\"unix\"] }\nlibc = \"0.2\"\n",
        );

        let alef_toml_content = format!(
            "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"alpha\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display()
        );
        write_file(root, "alef.toml", &alef_toml_content);
        let alef_toml_path = root.join("alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml_content).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        // Root [workspace.dependencies] alpha pin must be bumped to rc.2.
        let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).expect("read root");
        assert!(
            root_cargo.contains(r#"alpha = { path = "crates/alpha", version = "5.0.0-rc.2""#),
            "root [workspace.dependencies] alpha must be bumped to rc.2:\n{root_cargo}"
        );
        assert!(
            root_cargo.contains(r#"serde = "1.0""#),
            "root serde must be untouched:\n{root_cargo}"
        );

        // crates/alpha [package] version must be rc.2.
        let alpha_cargo = std::fs::read_to_string(root.join("crates/alpha/Cargo.toml")).expect("read alpha");
        assert!(
            alpha_cargo.contains("version = \"5.0.0-rc.2\""),
            "alpha [package] must be bumped:\n{alpha_cargo}"
        );

        // crates/beta: all four dep-table shapes must reference rc.2.
        let beta_cargo = std::fs::read_to_string(root.join("crates/beta/Cargo.toml")).expect("read beta");
        let alpha_version_lines: Vec<&str> = beta_cargo
            .lines()
            .filter(|l| l.contains("alpha") && l.contains("version"))
            .collect();
        assert!(
            !alpha_version_lines.is_empty(),
            "expected alpha dep lines with version= in beta:\n{beta_cargo}"
        );
        for line in &alpha_version_lines {
            assert!(
                line.contains("5.0.0-rc.2"),
                "alpha pin not bumped to rc.2 in beta:\n  {line}\nfull:\n{beta_cargo}"
            );
        }
        assert!(
            !beta_cargo.contains("5.0.0-rc.1"),
            "old rc.1 must be gone from beta:\n{beta_cargo}"
        );

        // External deps in beta must be untouched.
        assert!(
            beta_cargo.contains(r#"serde = "1.0""#),
            "serde must not be touched:\n{beta_cargo}"
        );
        assert!(
            beta_cargo.contains(r#"tempfile = "3""#),
            "tempfile must not be touched:\n{beta_cargo}"
        );
        assert!(
            beta_cargo.contains(r#"libc = "0.2""#),
            "libc must not be touched:\n{beta_cargo}"
        );
    }
}
