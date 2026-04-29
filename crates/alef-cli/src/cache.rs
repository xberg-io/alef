use std::fs;
use std::path::{Path, PathBuf};

const CACHE_DIR: &str = ".alef";
const PER_FILE_CACHE_NAME: &str = "sources_hash.cache";

/// Compute the per-run sources hash that drives both the IR cache and the
/// embedded `alef:hash:` value. Pure function of the rust source files
/// (paths + content); independent of `alef.toml` and the alef CLI version, so
/// that `alef verify` is idempotent across alef upgrades.
///
/// Warm-run optimisation: stat every source and check `(mtime_nanos, size)`
/// against an on-disk memo (`.alef/sources_hash.cache`). When **every** file's
/// stat is unchanged we return the cached aggregate hash directly — no file
/// reads, no blake3 work. Any change to any file falls back to the canonical
/// [`alef_core::hash::compute_sources_hash`] (which reads + hashes everything)
/// and refreshes the memo. The output is always equivalent to the canonical
/// function; the memo only elides redundant reads on no-change runs.
pub fn sources_hash(sources: &[PathBuf]) -> anyhow::Result<String> {
    let mut sorted: Vec<&PathBuf> = sources.iter().collect();
    sorted.sort();

    let memo = read_per_file_memo();
    let mut current: Vec<(String, u64, u64)> = Vec::with_capacity(sorted.len());
    let mut all_match = !memo.entries.is_empty() && memo.aggregate.is_some();
    for source in &sorted {
        let metadata = fs::metadata(source)
            .map_err(|e| anyhow::anyhow!("failed to stat source {}: {e}", source.display()))?;
        let mtime_nanos = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let size = metadata.len();
        let path_str = source.to_string_lossy().to_string();
        if all_match {
            match memo.entries.get(&path_str) {
                Some((m, s)) if *m == mtime_nanos && *s == size => {}
                _ => all_match = false,
            }
        }
        current.push((path_str, mtime_nanos, size));
    }

    // If the memo also tracks the same number of files (no rename / removal),
    // the cached aggregate is valid.
    if all_match && current.len() == memo.entries.len() {
        if let Some(agg) = memo.aggregate {
            return Ok(agg);
        }
    }

    // Cold path / change detected: read+hash every file via the canonical
    // function so the result remains bit-identical with what existing
    // alef:hash lines were derived from.
    let aggregate = alef_core::hash::compute_sources_hash(sources)?;
    let _ = write_per_file_memo(&current, &aggregate);
    Ok(aggregate)
}

struct PerFileMemo {
    aggregate: Option<String>,
    entries: std::collections::HashMap<String, (u64, u64)>,
}

fn read_per_file_memo() -> PerFileMemo {
    let path = Path::new(CACHE_DIR).join(PER_FILE_CACHE_NAME);
    let Ok(content) = fs::read_to_string(&path) else {
        return PerFileMemo {
            aggregate: None,
            entries: std::collections::HashMap::new(),
        };
    };
    let mut aggregate: Option<String> = None;
    let mut entries = std::collections::HashMap::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("aggregate\t") {
            aggregate = Some(rest.to_string());
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let mtime_nanos = parts[1].parse::<u64>().unwrap_or(0);
        let size = parts[2].parse::<u64>().unwrap_or(0);
        entries.insert(parts[0].to_string(), (mtime_nanos, size));
    }
    PerFileMemo { aggregate, entries }
}

fn write_per_file_memo(entries: &[(String, u64, u64)], aggregate: &str) -> anyhow::Result<()> {
    let dir = Path::new(CACHE_DIR);
    fs::create_dir_all(dir)?;
    let mut content = format!("aggregate\t{aggregate}\n");
    for (path, mtime, size) in entries {
        content.push_str(&format!("{path}\t{mtime}\t{size}\n"));
    }
    fs::write(dir.join(PER_FILE_CACHE_NAME), content)?;
    Ok(())
}

/// Check if cached IR is still valid.
pub fn is_ir_cached(source_hash: &str) -> bool {
    let hash_path = Path::new(CACHE_DIR).join("ir.hash");
    let ir_path = Path::new(CACHE_DIR).join("ir.json");
    if !ir_path.exists() {
        return false;
    }
    match fs::read_to_string(&hash_path) {
        Ok(cached) => cached.trim() == source_hash,
        Err(_) => false,
    }
}

/// Read cached IR.
pub fn read_cached_ir() -> anyhow::Result<alef_core::ir::ApiSurface> {
    let ir_path = Path::new(CACHE_DIR).join("ir.json");
    let content = fs::read_to_string(&ir_path)?;
    Ok(serde_json::from_str(&content)?)
}

/// Write IR to cache.
pub fn write_ir_cache(api: &alef_core::ir::ApiSurface, source_hash: &str) -> anyhow::Result<()> {
    let cache_dir = Path::new(CACHE_DIR);
    fs::create_dir_all(cache_dir)?;
    fs::write(cache_dir.join("ir.json"), serde_json::to_string_pretty(api)?)?;
    fs::write(cache_dir.join("ir.hash"), source_hash)?;
    Ok(())
}

/// Compute hash for a language's output (IR + language-specific config).
pub fn compute_lang_hash(ir_json: &str, lang: &str, config_toml: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(lang.as_bytes());
    hasher.update(config_toml.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Check if a language's output is cached.
/// Returns false if the hash doesn't match OR if any previously-generated
/// output files are missing from disk.
pub fn is_lang_cached(lang: &str, lang_hash: &str) -> bool {
    let hash_path = Path::new(CACHE_DIR).join("hashes").join(format!("{lang}.hash"));
    let manifest_path = Path::new(CACHE_DIR).join("hashes").join(format!("{lang}.manifest"));
    match fs::read_to_string(&hash_path) {
        Ok(cached) => {
            if cached.trim() != lang_hash {
                return false;
            }
            // Verify all output files from the manifest still exist on disk
            outputs_exist(&manifest_path)
        }
        Err(_) => false,
    }
}

/// Write language hash and output file manifest.
pub fn write_lang_hash(lang: &str, lang_hash: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let hashes_dir = Path::new(CACHE_DIR).join("hashes");
    fs::create_dir_all(&hashes_dir)?;
    fs::write(hashes_dir.join(format!("{lang}.hash")), lang_hash)?;
    write_manifest(&hashes_dir.join(format!("{lang}.manifest")), output_paths)?;
    Ok(())
}

/// Compute hash for a generation stage (stubs, docs, readme, scaffold, e2e).
/// `extra` allows including additional content (e.g., fixture files for e2e).
pub fn compute_stage_hash(ir_json: &str, stage: &str, config_toml: &str, extra: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(stage.as_bytes());
    hasher.update(config_toml.as_bytes());
    if !extra.is_empty() {
        hasher.update(extra);
    }
    hasher.finalize().to_hex().to_string()
}

/// Check if a stage's output is cached.
/// Returns false if the hash doesn't match OR if any previously-generated
/// output files are missing from disk.
pub fn is_stage_cached(stage: &str, stage_hash: &str) -> bool {
    let hash_path = Path::new(CACHE_DIR).join("hashes").join(format!("{stage}.hash"));
    let manifest_path = Path::new(CACHE_DIR).join("hashes").join(format!("{stage}.manifest"));
    match fs::read_to_string(&hash_path) {
        Ok(cached) => {
            if cached.trim() != stage_hash {
                return false;
            }
            // Verify all output files from the manifest still exist on disk
            outputs_exist(&manifest_path)
        }
        Err(_) => false,
    }
}

/// Write stage hash and output file manifest.
pub fn write_stage_hash(stage: &str, stage_hash: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let hashes_dir = Path::new(CACHE_DIR).join("hashes");
    fs::create_dir_all(&hashes_dir)?;
    fs::write(hashes_dir.join(format!("{stage}.hash")), stage_hash)?;
    write_manifest(&hashes_dir.join(format!("{stage}.manifest")), output_paths)?;
    Ok(())
}

/// Write a manifest of output file paths (one per line).
fn write_manifest(manifest_path: &Path, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let content: String = output_paths
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(manifest_path, content)?;
    Ok(())
}

/// Check that all files listed in a manifest exist on disk.
/// Returns true if the manifest is missing (backwards compat with old caches)
/// or if all listed files exist. Returns false if any file is missing.
fn outputs_exist(manifest_path: &Path) -> bool {
    match fs::read_to_string(manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .all(|line| Path::new(line).exists()),
        // No manifest means old-style cache entry; treat as valid to avoid
        // breaking existing caches on upgrade. The next write will create one.
        Err(_) => true,
    }
}

/// Hash all files in a directory recursively (for e2e fixture hashing).
pub fn hash_directory(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();
    if dir.exists() {
        let mut entries: Vec<_> = walkdir(dir)?;
        entries.sort();
        for path in entries {
            let content = fs::read(&path)?;
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(&content);
        }
    }
    Ok(hasher.finalize().as_bytes().to_vec())
}

fn walkdir(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

// ---------------------------------------------------------------------------
// Generation content hashing — used by `alef verify` for idempotent staleness
// checking.  We blake3-hash the raw codegen output strings and store
// `path\thash` entries in `.alef/hashes/<name>.output_hashes`.  During verify
// we regenerate in-memory, hash the new content, and compare against stored
// hashes.  Both sides are pure codegen output — on-disk state is never
// consulted.  Formatter/linter autofixes cannot cause false positives.
// ---------------------------------------------------------------------------

/// Blake3 hash of a content string.
pub fn hash_content(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// Store generation content hashes: Vec of (path_display, content_hash).
///
/// Call this with pre-computed hashes — use [`hash_content`] on each file's
/// content string before calling.  Stored before writing to disk so hashes
/// reflect pure codegen output, independent of any on-disk formatter.
pub fn write_generation_hashes(name: &str, hashes: &[(String, String)]) -> anyhow::Result<()> {
    let dir = Path::new(CACHE_DIR).join("hashes");
    fs::create_dir_all(&dir)?;
    let lines: Vec<String> = hashes.iter().map(|(p, h)| format!("{p}\t{h}")).collect();
    fs::write(dir.join(format!("{name}.output_hashes")), lines.join("\n"))?;
    Ok(())
}

/// Load stored generation hashes as `HashMap<path, hash>`.
pub fn read_generation_hashes(name: &str) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let path = Path::new(CACHE_DIR)
        .join("hashes")
        .join(format!("{name}.output_hashes"));
    let content = fs::read_to_string(&path)?;
    Ok(content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(p, h)| (p.to_string(), h.to_string()))
        .collect())
}

/// Clear cache.
pub fn clear_cache() -> anyhow::Result<()> {
    let cache_dir = Path::new(CACHE_DIR);
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

/// Show cache status information.
pub fn show_status() {
    let cache_dir = Path::new(CACHE_DIR);
    if !cache_dir.exists() {
        println!("No cache directory.");
        return;
    }

    println!("Cache directory: .alef/");

    let ir_path = cache_dir.join("ir.json");
    if ir_path.exists() {
        if let Ok(meta) = fs::metadata(&ir_path) {
            println!("  ir.json: {} bytes", meta.len());
        }
    } else {
        println!("  ir.json: not cached");
    }

    let hashes_dir = cache_dir.join("hashes");
    if hashes_dir.exists() {
        if let Ok(entries) = fs::read_dir(&hashes_dir) {
            let langs: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str().map(String::from)))
                .collect();
            if langs.is_empty() {
                println!("  language hashes: none");
            } else {
                println!("  language hashes: {}", langs.join(", "));
            }
        }
    } else {
        println!("  language hashes: none");
    }
}
