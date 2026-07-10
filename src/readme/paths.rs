use crate::core::config::{Language, ResolvedCrateConfig};
use std::path::PathBuf;

/// Determine the output path for a language README.
pub(super) fn readme_output_path(
    config: &ResolvedCrateConfig,
    lang: Language,
    readme_cfg: &crate::core::config::ReadmeConfig,
    lang_json: &serde_json::Value,
) -> PathBuf {
    if let Some(output) = lang_json
        .get("output_path")
        .or_else(|| lang_json.get("output"))
        .and_then(|v| v.as_str())
    {
        return PathBuf::from(output);
    }

    if let Some(pattern) = &readme_cfg.output_pattern {
        let dir = lang_dir_name(lang);
        return PathBuf::from(pattern.replace("{language}", dir));
    }

    default_readme_path(config, lang)
}

/// Determine the output path for a named README target.
pub(super) fn readme_target_output_path(target_name: &str, target_json: &serde_json::Value) -> anyhow::Result<PathBuf> {
    target_json
        .get("output_path")
        .or_else(|| target_json.get("output"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("README target '{target_name}' requires `output_path` or `output`"))
}

pub(super) fn default_readme_path(config: &ResolvedCrateConfig, lang: Language) -> PathBuf {
    let name = &config.name;
    match lang {
        Language::Ffi => PathBuf::from(format!("crates/{name}-ffi/README.md")),
        Language::Wasm => PathBuf::from(format!("crates/{name}-wasm/README.md")),
        Language::Node => PathBuf::from(format!("crates/{name}-node/README.md")),
        Language::Rust => PathBuf::from(format!("crates/{name}/README.md")),
        _ => PathBuf::from(format!("packages/{}/README.md", lang_dir_name(lang))),
    }
}

/// Return the short directory/key name for a language. This is the canonical
/// `packages/<dir>/` directory name used when no explicit `output_path` is
/// configured. For Language::Node we return `"node"` (matching the alef-scaffold
/// directory convention); the YAML/TOML config key remains `"typescript"`
/// (see [`lang_code`]).
pub(super) fn lang_dir_name(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "node",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "ffi",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::KotlinAndroid => "kotlin-android",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
        Language::C | Language::Jni => "c",
    }
}

/// Return the YAML config key for a language.
pub(super) fn lang_code(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "ffi",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::KotlinAndroid => "kotlin_android",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
        Language::C | Language::Jni => "c",
    }
}
