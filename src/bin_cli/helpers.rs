use anyhow::{Context, Result};

/// Returns true when every freshly generated file already matches the file on disk,
/// using the same hash-line-insensitive body comparison as [`crate::cli::pipeline::write_files`].
///
/// The per-run side cache (`.alef/hashes/*.output_hashes`) records what was last
/// generated, but the files on disk can drift from it out-of-band — a `git restore`,
/// a hand-edit, a partial write, or an interrupted run. Treating the cache as the
/// sole authority for an "up to date" skip silently retains that stale output: the
/// generator would emit different bytes, yet the skip fires and `write_files` is
/// never reached. Gating the skip on actual disk agreement closes that gap while
/// staying a no-op for the common clean case.
pub(crate) fn generated_files_match_disk(
    lang_files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> bool {
    lang_files.iter().all(|file| {
        let normalized = crate::cli::pipeline::normalize_content(&file.path, &file.content);
        match std::fs::read_to_string(base_dir.join(&file.path)) {
            Ok(disk) => crate::core::hash::strip_hash_line(&disk) == crate::core::hash::strip_hash_line(&normalized),
            Err(_) => false,
        }
    })
}

pub(crate) fn init_tracing(verbose: u8, quiet: bool, no_color: bool) {
    use tracing_subscriber::EnvFilter;
    let default_level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(!no_color)
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();
}

/// Load and resolve an alef.toml, returning the workspace-level config and
/// the per-crate resolved configs.  Detects legacy schema and returns an error
/// with a migration hint rather than a confusing parse error.
pub(crate) fn load_config(
    path: &std::path::Path,
) -> Result<(
    crate::core::config::WorkspaceConfig,
    Vec<crate::core::config::ResolvedCrateConfig>,
)> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config: {}", path.display()))?;
    crate::core::config::detect_legacy_keys(&content).with_context(|| {
        format!(
            "legacy schema detected in {} — run `alef migrate` to update automatically",
            path.display()
        )
    })?;
    let mut toml_value: toml::Value =
        toml::from_str(&content).with_context(|| format!("Failed to parse alef.toml ({})", path.display()))?;
    let deprecation_warnings = crate::core::config::legacy::strip_deprecated_keys(&mut toml_value);
    for warning in &deprecation_warnings {
        tracing::warn!("{}", warning);
    }
    let cfg: crate::core::config::NewAlefConfig = toml_value
        .try_into()
        .with_context(|| format!("Failed to deserialize alef.toml ({})", path.display()))?;
    let resolved = cfg
        .resolve()
        .with_context(|| format!("failed to resolve crates in {}", path.display()))?;
    for resolved_cfg in &resolved {
        crate::core::config::validation::validate_resolved(resolved_cfg)
            .with_context(|| format!("invalid resolved config for crate `{}`", resolved_cfg.name))?;
    }
    Ok((cfg.workspace, resolved))
}

pub(crate) fn resolve_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, false)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
/// Docs can always be generated for Rust since it's the source language.
pub(crate) fn resolve_doc_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
///
/// Every Rust crate that publishes to crates.io needs a `crates/<lib>/README.md`,
/// so the readme command must regenerate it from the same templates that produce
/// the per-binding READMEs. Configure with `[crates.readme.languages.rust]` in
/// `alef.toml` to opt in.
pub(crate) fn resolve_readme_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Resolve languages for `alef test`.
///
/// Test suites can exist for targets that do not generate host bindings, such
/// as Rust e2e tests for the source crate. Keep binding language resolution
/// strict for generation/build commands, but allow explicit test targets and
/// include e2e-only entries when `alef test --e2e` runs without a filter.
pub(crate) fn resolve_test_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    include_e2e: bool,
) -> Result<Vec<crate::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || config.test.contains_key(&lang.to_string()) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list or test configuration");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if include_e2e {
                let mut extra_test_langs = vec![];
                for (lang_str, test_config) in &config.test {
                    if test_config.e2e.is_none() {
                        continue;
                    }
                    let lang = parse_language(lang_str)
                        .with_context(|| format!("Invalid test language in alef.toml: {lang_str}"))?;
                    if !langs.contains(&lang) {
                        extra_test_langs.push(lang);
                    }
                }
                extra_test_langs.sort_by_key(|lang| lang.to_string());
                for lang in extra_test_langs {
                    if !langs.contains(&lang) {
                        langs.push(lang);
                    }
                }
            }
            Ok(langs)
        }
    }
}

pub(crate) fn resolve_languages_inner(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    allow_rust: bool,
) -> Result<Vec<crate::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || (allow_rust && lang == crate::core::config::Language::Rust) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if allow_rust && !langs.contains(&crate::core::config::Language::Rust) {
                langs.push(crate::core::config::Language::Rust);
            }
            Ok(langs)
        }
    }
}

pub(crate) fn parse_language(lang_str: &str) -> Result<crate::core::config::Language> {
    toml::Value::String(lang_str.to_string())
        .try_into()
        .with_context(|| format!("Unknown language: {lang_str}"))
}

pub(crate) fn format_languages(languages: &[crate::core::config::Language]) -> String {
    languages.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
}

/// A file whose embedded `alef:hash:` did not match any expected inputs hash.
///
/// Returned by [`verify_walk_multi`] and [`verify_walk`] for every stale file
/// found during a verify walk. The `computed` field holds all candidate hashes
/// from the current workspace (one per crate); `embedded` is what was found in
/// the file's header. Pass `--verbose` / `-v` to `alef verify` to print these.
pub(crate) struct StaleMismatch {
    /// Absolute path of the stale generated file.
    pub(crate) path: String,
    /// The `alef:hash:` value embedded in the file's header.
    pub(crate) embedded: String,
    /// All candidate inputs hashes computed from the current workspace.
    /// The file is stale because none of these equals `embedded`.
    pub(crate) computed: Vec<String>,
}

/// Build/cache directories the verify walk never descends into.
const VERIFY_SKIP_DIRS: &[&str] = &[
    ".git",
    ".alef",
    "target",
    "node_modules",
    "_build",
    "deps",
    "parsers",
    "dist",
    "dist-node",
    "vendor",
    ".venv",
    ".cache",
    ".remote-cache",
    "__pycache__",
    "build",
    "tmp",
    "out",
    ".idea",
    ".vscode",
];

/// File extensions the verify walk inspects for an `alef:hash:` header.
const VERIFY_SCAN_EXTENSIONS: &[&str] = &[
    "rs", "py", "pyi", "ts", "tsx", "js", "mjs", "cjs", "rb", "rbs", "php", "phpstub", "go", "java", "cs", "ex", "exs",
    "R", "r", "toml", "json", "md", "h", "c", "yaml", "yml",
];

/// Walk `base_dir` and return every alef-owned file paired with the
/// `alef:hash:<hex>` embedded in its header. Skips build/cache directories and
/// files without the header marker. Shared by [`verify_walk`],
/// [`verify_walk_multi`], and [`detect_hash_inconsistency`] so all three see the
/// same file set.
fn collect_alef_hashes(base_dir: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    let mut found: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![base_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if VERIFY_SKIP_DIRS.contains(&name) || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    VERIFY_SCAN_EXTENSIONS
                        .iter()
                        .any(|allowed| allowed.eq_ignore_ascii_case(e))
                })
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(disk_hash) = crate::core::hash::extract_hash(&content) {
                found.push((path, disk_hash));
            }
        }
    }
    found
}

/// A tree carrying more distinct `alef:hash` values than there are generating
/// crates — the signature of a partial regeneration (some files regenerated,
/// others left with an older hash), independent of what the current host would
/// compute for the inputs hash.
pub(crate) struct HashInconsistency {
    /// Each distinct embedded hash and the files that carry it, sorted by hash.
    pub(crate) groups: Vec<(String, Vec<String>)>,
    /// Number of generating crates (the maximum number of distinct hashes a
    /// consistent tree may legitimately carry — one per crate).
    pub(crate) crate_count: usize,
}

/// Flag a tree whose alef-owned files carry more distinct `alef:hash` values
/// than there are generating crates (#184).
///
/// Each crate stamps all of its generated files with a single inputs hash, so a
/// consistent workspace carries at most one distinct hash per crate. More
/// distinct hashes than crates means at least one crate's output is split across
/// two generations — a partial regen that must not be committed. This check is
/// host-independent: it never recomputes the inputs hash, so it holds even in an
/// environment that would compute a different hash than the one on disk.
pub(crate) fn detect_hash_inconsistency(base_dir: &std::path::Path, crate_count: usize) -> Option<HashInconsistency> {
    let mut by_hash: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for (path, hash) in collect_alef_hashes(base_dir) {
        by_hash.entry(hash).or_default().push(path.display().to_string());
    }
    if by_hash.len() <= crate_count.max(1) {
        return None;
    }
    let mut groups: Vec<(String, Vec<String>)> = by_hash
        .into_iter()
        .map(|(hash, mut paths)| {
            paths.sort();
            (hash, paths)
        })
        .collect();
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    Some(HashInconsistency { groups, crate_count })
}

/// Multi-crate variant of [`verify_walk`].
///
/// Walk the repo from `base_dir`, find every alef-headered file, and return the
/// list of stale ones — where the embedded `alef:hash:<hex>` does not match any
/// of the provided `inputs_hashes`. In a multi-crate workspace each file was
/// generated by exactly one crate, so the file passes verification when it
/// matches its generating crate's inputs hash.
pub(crate) fn verify_walk_multi(
    base_dir: &std::path::Path,
    inputs_hashes: &[String],
) -> anyhow::Result<Vec<StaleMismatch>> {
    if inputs_hashes.is_empty() {
        return Ok(Vec::new());
    }
    if inputs_hashes.len() == 1 {
        return verify_walk(base_dir, &inputs_hashes[0]);
    }

    let mut stale: Vec<StaleMismatch> = collect_alef_hashes(base_dir)
        .into_iter()
        .filter(|(_, disk_hash)| !inputs_hashes.iter().any(|ih| ih == disk_hash))
        .map(|(path, disk_hash)| StaleMismatch {
            path: path.display().to_string(),
            embedded: disk_hash,
            computed: inputs_hashes.to_vec(),
        })
        .collect();

    stale.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(stale)
}

/// Walk the consumer's repo from `base_dir`, find every alef-headered file, and
/// return the list of stale ones — where the embedded `alef:hash:<hex>` does not
/// equal `inputs_hash`.
///
/// Verification is a direct string equality check against the generation-inputs
/// hash (alef rev + sources + alef.toml). File content is never rehashed, so
/// post-generation formatter rewrites cannot cause false-positive staleness.
///
/// Skips obvious build/cache directories (`target/`, `node_modules/`, `_build/`,
/// `.alef/`, `parsers/`, `dist/`, `vendor/`, `.git/`) so verify stays fast on
/// large repos. Files without the alef header marker are skipped silently —
/// those are user-owned (scaffold-once Cargo.toml templates, composer.json,
/// gemspec, package.json, lockfiles, etc.) and alef has no claim.
pub(crate) fn verify_walk(base_dir: &std::path::Path, inputs_hash: &str) -> anyhow::Result<Vec<StaleMismatch>> {
    let mut stale: Vec<StaleMismatch> = collect_alef_hashes(base_dir)
        .into_iter()
        .filter(|(_, disk_hash)| disk_hash != inputs_hash)
        .map(|(path, disk_hash)| StaleMismatch {
            path: path.display().to_string(),
            embedded: disk_hash,
            computed: vec![inputs_hash.to_string()],
        })
        .collect();

    stale.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;

    fn resolved_test_config() -> crate::core::config::ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
command = "pytest"

[crates.test.rust]
e2e = "cargo test"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn resolve_test_languages_allows_explicit_test_only_language() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, Some(&["rust".to_string()]), true).unwrap();
        assert_eq!(langs, vec![Language::Rust]);
    }

    #[test]
    fn resolve_test_languages_appends_e2e_only_languages() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, None, true).unwrap();
        assert_eq!(langs, vec![Language::Python, Language::Rust]);
    }

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn write_hashed(dir: &std::path::Path, name: &str, hash: &str) {
        let path = dir.join(name);
        std::fs::write(&path, format!("// alef:hash:{hash}\n\nfn generated() {{}}\n")).expect("write file");
    }

    #[test]
    fn single_crate_with_two_distinct_hashes_is_flagged_inconsistent() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("packages/python");
        std::fs::create_dir_all(&dir).expect("create dir");
        write_hashed(&dir, "a.rs", HASH_A);
        write_hashed(&dir, "b.rs", HASH_B);

        let inconsistency = detect_hash_inconsistency(tempdir.path(), 1).expect("partial regen must be flagged");
        assert_eq!(inconsistency.groups.len(), 2);
        assert_eq!(inconsistency.crate_count, 1);
    }

    #[test]
    fn single_crate_with_one_hash_is_consistent() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("packages/python");
        std::fs::create_dir_all(&dir).expect("create dir");
        write_hashed(&dir, "a.rs", HASH_A);
        write_hashed(&dir, "b.rs", HASH_A);

        assert!(detect_hash_inconsistency(tempdir.path(), 1).is_none());
    }

    #[test]
    fn multi_crate_allows_one_distinct_hash_per_crate() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("packages");
        std::fs::create_dir_all(&dir).expect("create dir");
        // Two crates legitimately stamp two different hashes. ~keep
        write_hashed(&dir, "a.rs", HASH_A);
        write_hashed(&dir, "b.rs", HASH_B);
        assert!(detect_hash_inconsistency(tempdir.path(), 2).is_none());

        // A third distinct hash exceeds the crate count — one crate is split. ~keep
        write_hashed(&dir, "c.rs", HASH_C);
        assert!(detect_hash_inconsistency(tempdir.path(), 2).is_some());
    }

    #[test]
    fn resolve_test_languages_omits_e2e_only_languages_without_e2e() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, None, false).unwrap();
        assert_eq!(langs, vec![Language::Python]);
    }

    fn gen_file(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
        crate::core::backend::GeneratedFile {
            path: std::path::PathBuf::from(rel),
            content: content.to_string(),
            generated_header: true,
        }
    }

    #[test]
    fn generated_files_match_disk_true_when_bodies_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
        let files = vec![gen_file("binding.go", "package x\n\nvar a = 1\n")];
        assert!(generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn generated_files_match_disk_ignores_embedded_hash_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("binding.go"),
            "// alef:hash:deadbeef\npackage x\n\nvar a = 1\n",
        )
        .unwrap();
        let files = vec![gen_file("binding.go", "package x\n\nvar a = 1\n")];
        assert!(generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn generated_files_match_disk_false_when_body_differs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
        let files = vec![gen_file("binding.go", "package x\n\nimport \"fmt\"\n\nvar a = 1\n")];
        assert!(!generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn generated_files_match_disk_false_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![gen_file("binding.go", "package x\n")];
        assert!(!generated_files_match_disk(&files, dir.path()));
    }
}
