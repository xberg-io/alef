use super::validation::validate_generation_api;
use crate::cli::{cache, registry};
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info};

pub fn generate(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    clean: bool,
    config_path: &Path,
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let validated_api = validate_generation_api(api, config, languages)?;

    // Validate that Go/Java/C# have FFI in the languages list
    let has_ffi = languages.contains(&Language::Ffi);
    for &lang in languages {
        if (lang == Language::Go || lang == Language::Java || lang == Language::Csharp) && !has_ffi {
            tracing::warn!(
                "Language {:?} requires FFI to be in the languages list for proper code generation",
                lang
            );
        }
    }

    let ir_json = serde_json::to_string(api)?;
    let mut config_toml =
        toml::to_string(config).with_context(|| "failed to serialize resolved crate config for cache key")?;
    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
    config_toml.push_str("\n# raw alef.toml\n");
    config_toml.push_str(&String::from_utf8_lossy(&alef_toml_bytes));

    let to_generate: Vec<_> = languages
        .par_iter()
        .filter_map(|&lang| {
            let lang_str = lang.to_string();
            let lang_hash = cache::compute_lang_hash(&ir_json, &lang_str, &config_toml);

            if !clean && cache::is_lang_cached(&config.name, &lang_str, &lang_hash) {
                debug!("  {}: cached, skipping", lang_str);
                return None;
            }

            Some((lang, lang_str, lang_hash))
        })
        .collect();

    let results: Vec<(Language, Vec<GeneratedFile>)> = to_generate
        .par_iter()
        .map(|(lang, lang_str, lang_hash)| {
            let backend = registry::get_backend(*lang);
            info!("  {}: generating...", lang_str);

            let mut files = backend
                .generate_bindings_checked(validated_api, config)
                .with_context(|| format!("failed to generate bindings for {lang_str}"))?;

            // Collect additional files from registered extensions, then let each
            // extension transform the full file list. Both hooks receive the
            // per-extension config from the `[extensions.<name>]` alef.toml section.
            crate::with_extensions(|exts| {
                let env = crate::core::template_env::TemplateEnv::new();
                for ext in exts {
                    let raw = crate::core::extension::read_extension_config(config_path, ext.name())
                        .with_context(|| format!("extension `{}`: failed to read config from alef.toml", ext.name()))?;
                    let cfg = ext
                        .parse_config(raw.as_ref())
                        .with_context(|| format!("extension `{}`: failed to parse config", ext.name()))?;
                    let extra = ext
                        .emit_for_language(validated_api.api(), &cfg, *lang, &env)
                        .with_context(|| format!("extension `{}`: emit_for_language({lang_str}) failed", ext.name()))?;
                    files.extend(extra);
                    ext.transform_emitted_files(validated_api.api(), &cfg, *lang, &mut files, &env)
                        .with_context(|| {
                            format!("extension `{}`: transform_emitted_files({lang_str}) failed", ext.name())
                        })?;
                }
                Ok::<(), anyhow::Error>(())
            })?;

            let base_dir = std::env::current_dir().unwrap_or_default();
            let output_paths: Vec<std::path::PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
            cache::write_lang_hash(&config.name, lang_str, lang_hash, &output_paths)
                .with_context(|| format!("failed to write language hash for {lang_str}"))?;
            Ok((*lang, files))
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(results)
}

/// Generate type stubs for given languages.
pub fn generate_stubs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let validated_api = validate_generation_api(api, config, languages)?;

    let results: Vec<(Language, Vec<GeneratedFile>)> = languages
        .par_iter()
        .map(|&lang| {
            let Some(backend) = registry::try_get_backend(lang) else {
                return Ok((lang, Vec::new()));
            };
            let files = backend.generate_type_stubs_checked(validated_api, config)?;
            Ok((lang, files))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .collect();
    Ok(results)
}

/// Generate service API (idiomatic app object + handler bridge) for backends that
/// declare `supports_service_api`.  Only invoked when `api.services` is non-empty.
/// Fails for languages whose backends do not support service API yet.
pub fn generate_service_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let validated_api = validate_generation_api(api, config, languages)?;
    let api = validated_api.api();

    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let results: Vec<(Language, Vec<GeneratedFile>)> = languages
        .par_iter()
        .copied()
        .filter(|&lang| {
            registry::try_get_backend(lang).is_some_and(|backend| backend.capabilities().supports_service_api)
        })
        .map(|lang| {
            let backend = registry::get_backend(lang);
            let files = backend.generate_service_api_checked(validated_api, config)?;
            Ok((lang, files))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .collect();
    Ok(results)
}

/// Candidate filenames for a language's package public-API entry file.
///
/// The package entry file is where an
/// [`crate::core::extension::Extension::public_api_additions`] contribution is
/// appended (e.g. Python's `__init__.py`, Ruby's `<gem>.rb`). Some conventions
/// are dynamic — the Ruby gem entry is named after the gem — so this resolves
/// against the crate config rather than returning a fixed string. Languages with
/// no recognized entry-file convention (or whose entry file is produced outside
/// this public-API pass) return an empty list, making the additions a silent
/// no-op. New languages are added here as their entry file is produced within
/// this pass.
fn package_entry_filenames(language: Language, config: &ResolvedCrateConfig) -> Vec<String> {
    match language {
        Language::Python => vec!["__init__.py".to_string()],
        // The magnus backend emits the gem entry `lib/<gem_name_snake>.rb` in this
        // pass, where `<gem_name_snake>` is `ruby_gem_name()` with dashes normalized.
        Language::Ruby => vec![format!("{}.rb", config.ruby_gem_name().replace('-', "_"))],
        _ => Vec::new(),
    }
}

/// Append `lines` to the package entry file for `language` within `files`.
///
/// Core stays dumb: it only appends, skipping any line already present so
/// repeated application (or re-runs) is idempotent. The extension owns all
/// language semantics of the appended lines. No-op when `lines` is empty, the
/// language has no known entry-file convention, or no matching file is present.
fn append_public_api_additions(
    files: &mut [GeneratedFile],
    language: Language,
    config: &ResolvedCrateConfig,
    lines: &[String],
) {
    if lines.is_empty() {
        return;
    }
    let names = package_entry_filenames(language, config);
    if names.is_empty() {
        return;
    }
    let Some(init_file) = files.iter_mut().find(|f| {
        f.path
            .file_name()
            .is_some_and(|n| names.iter().any(|t| n == t.as_str()))
    }) else {
        return;
    };

    let mut seen: std::collections::HashSet<String> = init_file.content.lines().map(str::to_string).collect();
    let mut appended = String::new();
    for line in lines {
        if seen.insert(line.clone()) {
            appended.push_str(line);
            appended.push('\n');
        }
    }
    if appended.is_empty() {
        return;
    }
    if !init_file.content.is_empty() && !init_file.content.ends_with('\n') {
        init_file.content.push('\n');
    }
    init_file.content.push_str(&appended);
}

/// Generate public API wrappers for given languages.
pub fn generate_public_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    config_path: &Path,
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let validated_api = validate_generation_api(api, config, languages)?;

    let results: Vec<(Language, Vec<GeneratedFile>)> = languages
        .par_iter()
        .map(|&lang| {
            let Some(backend) = registry::try_get_backend(lang) else {
                return Ok((lang, Vec::new()));
            };
            let mut files = backend.generate_public_api_checked(validated_api, config)?;

            // Let registered extensions contribute raw lines to the package
            // public-API init file. This mirrors the bindings pass: each
            // extension receives its `[extensions.<name>]` config, and core only
            // appends (with exact-line de-dup). The appended content does not
            // feed the generation-inputs hash, so `alef verify` is unaffected.
            crate::with_extensions(|exts| {
                for ext in exts {
                    let raw = crate::core::extension::read_extension_config(config_path, ext.name())
                        .with_context(|| format!("extension `{}`: failed to read config from alef.toml", ext.name()))?;
                    let cfg = ext
                        .parse_config(raw.as_ref())
                        .with_context(|| format!("extension `{}`: failed to parse config", ext.name()))?;
                    let additions = ext
                        .public_api_additions(validated_api.api(), &cfg, lang)
                        .with_context(|| format!("extension `{}`: public_api_additions({lang}) failed", ext.name()))?;
                    append_public_api_additions(&mut files, lang, config, &additions);
                }
                Ok::<(), anyhow::Error>(())
            })?;

            Ok((lang, files))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .collect();
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::extension::{Extension, ExtensionConfig};

    struct AdditionsExtension;
    impl Extension for AdditionsExtension {
        fn name(&self) -> &str {
            "additions"
        }
        fn public_api_additions(
            &self,
            _api: &ApiSurface,
            _cfg: &ExtensionConfig,
            _language: Language,
        ) -> anyhow::Result<Vec<String>> {
            Ok(vec![
                "from ._extra import thing".to_string(),
                "__all__ = [*__all__, \"thing\"]".to_string(),
            ])
        }
    }

    fn init_files(content: &str) -> Vec<GeneratedFile> {
        vec![GeneratedFile {
            path: std::path::PathBuf::from("packages/python/pkg/__init__.py"),
            content: content.to_string(),
            generated_header: true,
        }]
    }

    fn test_cfg() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "pkg".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn public_api_additions_appended_and_idempotent() {
        let ext = AdditionsExtension;
        let api = ApiSurface::default();
        let cfg = ExtensionConfig::empty();
        let additions = ext.public_api_additions(&api, &cfg, Language::Python).unwrap();

        let mut files = init_files("__all__ = [\"Existing\"]\n");
        append_public_api_additions(&mut files, Language::Python, &test_cfg(), &additions);
        let content = &files[0].content;
        assert!(content.contains("from ._extra import thing"));
        assert!(content.contains("__all__ = [*__all__, \"thing\"]"));
        assert!(content.contains("__all__ = [\"Existing\"]"));

        // Applying the same additions again must not duplicate any line.
        append_public_api_additions(&mut files, Language::Python, &test_cfg(), &additions);
        let content = &files[0].content;
        assert_eq!(content.matches("from ._extra import thing").count(), 1);
        assert_eq!(content.matches("__all__ = [*__all__, \"thing\"]").count(), 1);
    }

    #[test]
    fn public_api_additions_noop_without_init_convention() {
        let additions = vec!["some line".to_string()];
        let mut files = vec![GeneratedFile {
            path: std::path::PathBuf::from("packages/go/pkg.go"),
            content: "package pkg\n".to_string(),
            generated_header: true,
        }];
        append_public_api_additions(&mut files, Language::Go, &test_cfg(), &additions);
        assert_eq!(files[0].content, "package pkg\n");
    }

    #[test]
    fn public_api_additions_ruby_gem_entry_appended_and_idempotent() {
        // The gem entry file name is dynamic (`<gem_name_snake>.rb`); the append
        // must resolve it from config and leave sibling files untouched.
        let additions = vec!["require_relative 'pkg/app'".to_string()];
        let mut files = vec![
            GeneratedFile {
                path: std::path::PathBuf::from("packages/ruby/lib/pkg.rb"),
                content: "# frozen_string_literal: true\nrequire_relative 'pkg/native'\n".to_string(),
                generated_header: true,
            },
            GeneratedFile {
                path: std::path::PathBuf::from("packages/ruby/lib/pkg/native.rb"),
                content: "# native\n".to_string(),
                generated_header: true,
            },
        ];

        append_public_api_additions(&mut files, Language::Ruby, &test_cfg(), &additions);
        assert!(files[0].content.contains("require_relative 'pkg/app'"));
        // Sibling (non-entry) file is never touched.
        assert_eq!(files[1].content, "# native\n");

        // Idempotent on re-apply.
        append_public_api_additions(&mut files, Language::Ruby, &test_cfg(), &additions);
        assert_eq!(files[0].content.matches("require_relative 'pkg/app'").count(), 1);
    }

    #[test]
    fn public_api_additions_ruby_normalizes_dashed_name_to_snake_entry() {
        // The gem entry file is `<gem_name_snake>.rb`; `ruby_gem_name()` falls back
        // to the crate name, and the resolver normalizes dashes so it matches the
        // magnus backend's `gem_name_snake` (dashes → underscores).
        let config = ResolvedCrateConfig {
            name: "my-gem".to_string(),
            ..Default::default()
        };
        let additions = vec!["require_relative 'my_gem/app'".to_string()];
        let mut files = vec![GeneratedFile {
            path: std::path::PathBuf::from("packages/ruby/lib/my_gem.rb"),
            content: "# frozen_string_literal: true\n".to_string(),
            generated_header: true,
        }];

        append_public_api_additions(&mut files, Language::Ruby, &config, &additions);
        assert!(files[0].content.contains("require_relative 'my_gem/app'"));
    }

    #[test]
    fn public_api_additions_noop_when_no_matching_file() {
        let additions = vec!["some line".to_string()];
        let mut files = vec![GeneratedFile {
            path: std::path::PathBuf::from("packages/python/pkg/options.py"),
            content: "X = 1\n".to_string(),
            generated_header: true,
        }];
        append_public_api_additions(&mut files, Language::Python, &test_cfg(), &additions);
        assert_eq!(files[0].content, "X = 1\n");
    }

    #[test]
    fn default_public_api_additions_is_empty() {
        struct Noop;
        impl Extension for Noop {
            fn name(&self) -> &str {
                "noop"
            }
        }
        let api = ApiSurface::default();
        let cfg = ExtensionConfig::empty();
        let out = Noop.public_api_additions(&api, &cfg, Language::Python).unwrap();
        assert!(out.is_empty());
    }
}
