use alef_core::config::{AlefConfig, Language};
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
fn to_pep440(version: &str) -> String {
    if let Some((base, pre)) = version.split_once('-') {
        let pep = pre
            .replace("alpha.", "a")
            .replace("alpha", "a")
            .replace("beta.", "b")
            .replace("beta", "b")
            .replace("rc.", "rc")
            .replace('.', "");
        format!("{base}{pep}")
    } else {
        version.to_string()
    }
}

use alef_core::version::to_rubygems_prerelease;

/// Verify that all package manifest versions match the Cargo.toml source of truth.
/// Returns a list of mismatches (empty = all consistent).
pub fn verify_versions(config: &AlefConfig) -> anyhow::Result<Vec<String>> {
    let expected = read_version(&config.crate_config.version_from)?;
    let expected_pep440 = to_pep440(&expected);
    let expected_rubygems = to_rubygems_prerelease(&expected);
    let mut mismatches = Vec::new();

    fn extract_version(path: &str, pattern: &str) -> Option<String> {
        let content = std::fs::read_to_string(path).ok()?;
        let re = regex::Regex::new(pattern).ok()?;
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
pub fn set_version(config: &AlefConfig, version: &str) -> anyhow::Result<()> {
    write_version_to_cargo_toml(&config.crate_config.version_from, version)
        .with_context(|| format!("failed to set version to {version}"))?;
    info!("Set version to {version} in {}", config.crate_config.version_from);
    Ok(())
}

/// Sync version from Cargo.toml to all package manifest files.
pub fn sync_versions(config: &AlefConfig, config_path: &std::path::Path, bump: Option<&str>) -> anyhow::Result<()> {
    // If bump is requested, read current version, bump it, and write it back to Cargo.toml.
    if let Some(component) = bump {
        let current = read_version(&config.crate_config.version_from)?;
        let bumped = bump_version(&current, component)?;
        info!("Bumping version {current} -> {bumped} ({component})");
        write_version_to_cargo_toml(&config.crate_config.version_from, &bumped).context("failed to sync versions")?;
        info!(
            "Updated {} with bumped version {bumped}",
            config.crate_config.version_from
        );
    }

    let version = read_version(&config.crate_config.version_from)?;
    info!("Syncing version {version}");

    let mut updated = vec![];

    // Workspace Cargo.toml files: sync [package] version in both members and excluded crates.
    // Excluded crates (e.g. Ruby ext) have their own version field that needs updating.
    // Uses write_version_to_cargo_toml which only replaces the [package] version field
    // (anchored to start-of-line), so dependency version specs are never touched.
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

            for pattern_val in members.iter().chain(excludes.iter()) {
                if let Some(pattern) = pattern_val.as_str() {
                    if let Ok(paths) = glob::glob(&format!("{pattern}/Cargo.toml")) {
                        for entry in paths.flatten() {
                            let path_str = entry.to_string_lossy().to_string();
                            // Skip crates that use workspace version inheritance or have no version
                            if write_version_to_cargo_toml(&path_str, &version).is_ok() {
                                updated.push(path_str);
                            }
                        }
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

    // Node: package.json
    if let Ok(content) = std::fs::read_to_string("packages/typescript/package.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
            std::fs::write("packages/typescript/package.json", &new_content)
                .context("failed to write packages/typescript/package.json")?;
            updated.push("packages/typescript/package.json".to_string());
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

    // Root composer.json (if present)
    if let Ok(content) = std::fs::read_to_string("composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("composer.json", &new_content)?;
            updated.push("composer.json".to_string());
        }
    }

    // R: DESCRIPTION file
    if let Ok(content) = std::fs::read_to_string("packages/r/DESCRIPTION") {
        if let Some(new_content) = replace_version_pattern(&content, r"Version:\s*[^\n]*", &version) {
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

    for file in updated {
        info!("  Updated: {file}");
    }

    // Rebuild FFI to refresh C headers (cbindgen) if FFI language is configured.
    if config.languages.contains(&Language::Ffi) {
        let ffi_crate = config
            .output
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

    // Invalidate the IR cache so that subsequent readme/docs generation picks up the new version.
    // This ensures READMEs (which embed version strings) are regenerated with the new version.
    info!("Invalidating IR cache to refresh version in documentation");
    if let Err(e) = std::fs::remove_dir_all(".alef") {
        // Log at debug level if cache doesn't exist yet (not an error)
        debug!("Could not remove cache directory: {e}");
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
fn regenerate_readmes(config: &AlefConfig, config_path: &std::path::Path) -> anyhow::Result<usize> {
    let api = extract(config, config_path, false)?;
    let languages = config.languages.clone();
    let readme_files = readme(&api, config, &languages)?;
    let base_dir = std::path::PathBuf::from(".");
    super::generate::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)
}

/// Replace version pattern in content. Returns Some(new_content) if replaced, None if pattern not found.
fn replace_version_pattern(content: &str, pattern: &str, version: &str) -> Option<String> {
    let regex = regex::Regex::new(pattern).ok()?;
    if !regex.is_match(content) {
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
        p if p.contains("VERSION") => format!(r#"VERSION = "{version}""#),
        _ => return None,
    };

    Some(regex.replace(content, replacement.as_str()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
  VERSION = "2.0.0"
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
        assert_eq!(new_content, "VERSION = \"2.0.0\"");
    }

    #[test]
    fn test_replace_version_pattern_ruby_version_double_quotes() {
        let content = "VERSION = \"1.5.2\"";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "3.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert_eq!(new_content, "VERSION = \"3.0.0\"");
    }

    #[test]
    fn test_replace_version_pattern_ruby_in_module() {
        let content = r#"module MyGem
  VERSION = "0.5.0"
end"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "1.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert!(new_content.contains("VERSION = \"1.0.0\""));
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
        assert!(new_content.contains("VERSION = \"2.0.0\""));
    }
}
