use crate::core::config::ResolvedCrateConfig;
use ahash::AHashSet;
use std::path::Path;
use tracing::debug;

pub fn ensure_gitignore(base_dir: &Path, config: &ResolvedCrateConfig) {
    use crate::core::config::Language;

    let gitignore_path = base_dir.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    let existing_lines: AHashSet<&str> = existing.lines().map(str::trim).collect();

    let mut entries: Vec<&str> = vec![".alef/"];

    for lang in &config.languages {
        match lang {
            Language::Python => {
                entries.extend_from_slice(&["__pycache__/", "*.so", "*.pyd", ".venv/", "*.egg-info/", "dist/"])
            }
            Language::Node => entries.extend_from_slice(&["node_modules/", "*.node"]),
            Language::Ruby => entries.extend_from_slice(&[".gems/", "vendor/bundle/"]),
            Language::Php => entries.extend_from_slice(&["vendor/"]),
            Language::Ffi => entries.push("*.h.bak"),
            Language::Go => entries.push("*.test"),
            Language::Java => entries.extend_from_slice(&["target/", "*.class"]),
            Language::Csharp => entries.extend_from_slice(&["bin/", "obj/", "*.nupkg"]),
            // pkg/ intentionally NOT gitignored — npm publish needs it for WASM artifacts
            Language::Wasm => {}
            _ => {}
        }
    }

    let mut to_add = Vec::new();
    for entry in &entries {
        if !existing_lines.contains(entry) {
            to_add.push(*entry);
        }
    }

    if to_add.is_empty() {
        return;
    }

    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let additions = to_add.join("\n");
    let new_content = format!("{existing}{separator}{additions}\n");

    if let Err(e) = std::fs::write(&gitignore_path, new_content) {
        debug!("Could not update .gitignore: {e}");
    } else {
        debug!("Updated .gitignore with {} entries", to_add.len());
    }
}
