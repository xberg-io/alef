use std::fs;
use std::path::{Path, PathBuf};

const CACHE_DIR: &str = ".alef";

/// Hash a list of files + config to determine if extraction is needed.
pub fn compute_source_hash(sources: &[PathBuf], config_path: &Path) -> anyhow::Result<String> {
    let mut hasher = blake3::Hasher::new();
    for source in sources {
        let content = fs::read(source)?;
        hasher.update(&content);
    }
    let config_content = fs::read(config_path)?;
    hasher.update(&config_content);
    Ok(hasher.finalize().to_hex().to_string())
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

/// Read the manifest for a given name and return the list of file paths.
pub fn read_manifest_paths(name: &str) -> anyhow::Result<Vec<PathBuf>> {
    let manifest_path = Path::new(CACHE_DIR).join("hashes").join(format!("{name}.manifest"));
    let content = fs::read_to_string(&manifest_path)?;
    Ok(content.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
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
