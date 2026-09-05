use super::validation::validate_generation_api;
use crate::cli::{cache, registry};
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::info;

/// `write_cache` controls whether a freshly generated language's output paths are
/// recorded to `.alef/<crate>/hashes/<lang>.{hash,manifest}`. Read-only callers that
/// regenerate in memory only to inspect the result — `alef verify`'s missing-file
/// check is the motivating case — must pass `false`: a command named `verify`
/// writing cache state as a side effect is surprising, and measurement showed the
/// cache buys nothing on this path (cold and warm regeneration both land around
/// 3.4s). Callers that actually write the generated files to disk (`alef generate`,
/// `alef all`, `alef init`) must pass `true` so the cache stays authoritative for
/// subsequent runs. ~keep
pub fn generate(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    clean: bool,
    config_path: &Path,
    write_cache: bool,
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    // Report against the extracted source surface, then project the documentation used by every
    // backend so it cannot advertise a foreign cfg-only variant that none of them emit. ~keep
    crate::codegen::foreign_cfg_variants::warn_foreign_cfg_gated_variants(api, config, languages);
    let projected_api = crate::codegen::foreign_cfg_variants::project_docs_without_unreachable_foreign_variants(
        api, config, languages,
    )?;
    let api = &projected_api;
    let validated_api = validate_generation_api(api, config, languages)?;

    // Every host-language backend derives its error dispatch from the same explicit
    // `#[alef(error_code = N)]` taxonomy (see `ApiSurface::validate_error_taxonomy`), not just
    // FFI's. Checking it once here — instead of only inside `FfiBackend::generate_bindings` —
    // means a duplicate or reserved-domain (0-4) error code is a generation-time failure even
    // for a `languages` list that omits `"ffi"`. ~keep
    api.validate_error_taxonomy()?;

    for lang in languages_missing_ffi(&config.languages, languages) {
        tracing::warn!("Language {:?} requires FFI in the languages list", lang);
    }

    let ir_json = serde_json::to_string(api)?;
    // Route through `canonical_toml_string`, not a plain `toml::to_string`: `ResolvedCrateConfig`
    // carries several `HashMap` fields, and serde serializes a `HashMap` in that map's own
    // randomly-seeded-per-process order, so an unsorted serialization makes this cache key
    // differ on every run even when nothing changed -- silently defeating the cache. ~keep
    let config_value =
        toml::Value::try_from(config).with_context(|| "failed to serialize resolved crate config for cache key")?;
    let mut config_toml = crate::core::hash::canonical_toml_string(config_value)
        .with_context(|| "failed to serialize resolved crate config for cache key")?;
    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
    config_toml.push_str("\n# raw alef.toml\n");
    config_toml.push_str(&String::from_utf8_lossy(&alef_toml_bytes));

    let to_generate: Vec<_> = languages
        .par_iter()
        .filter_map(|&lang| {
            let lang_str = lang.to_string();

            // `try_get_backend`, not `get_backend`: the latter panics for docs-only/
            // consumer-only targets (Rust, C). A language like C configured in
            // `[workspace] languages` (e.g. as an e2e consumer target) must be skipped
            // gracefully here rather than crashing generation — mirrors the same guard
            // in the build pipeline (`build.rs`). ~keep
            if registry::try_get_backend(lang).is_none() {
                info!("No binding backend for {lang_str}, skipping");
                return None;
            }

            let lang_hash = cache::compute_lang_hash(&ir_json, &lang_str, &config_toml);

            if !clean && cache::is_lang_cached(&config.name, &lang_str, &lang_hash) {
                // `info`, not `debug`: a skipped language contributes nothing to the run's
                // "Generated N files" line, so at the default verbosity a fully cached run was
                // reported as an empty one. The count and the skip are the same fact seen from
                // two sides and both have to be visible, or "0 files" reads as "nothing needed
                // changing" when it means "nothing was looked at". ~keep
                info!("  {lang_str}: unchanged since the last run by this alef build, skipping");
                return None;
            }

            Some((lang, lang_str, lang_hash))
        })
        .collect();

    let results: Vec<(Language, Vec<GeneratedFile>)> = to_generate
        .par_iter()
        .map(|(lang, lang_str, lang_hash)| {
            // Guarded above: every entry in `to_generate` already passed `try_get_backend`.
            let backend = registry::get_backend(*lang);
            info!("  {}: generating...", lang_str);

            let mut files = backend
                .generate_bindings_checked(validated_api, config)
                .with_context(|| format!("failed to generate bindings for {lang_str}"))?;

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

            if write_cache {
                let base_dir = std::env::current_dir().unwrap_or_default();
                let output_paths: Vec<std::path::PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                cache::write_lang_hash(&config.name, lang_str, lang_hash, &output_paths)
                    .with_context(|| format!("failed to write language hash for {lang_str}"))?;

                // Read-only callers (`write_cache = false`, e.g. `alef verify`'s in-memory
                // regeneration) must not gain this as a side effect either, for the same
                // reason `write_lang_hash` above is gated: a command not meant to touch the
                // cache still must not touch it just because it happens to also regenerate.
                // ~keep
                let current_signatures = backend.public_function_signatures(validated_api.api(), config);
                crate::cli::breaking_changes::check_signature_breakage(
                    *lang,
                    &config.name,
                    &base_dir,
                    &current_signatures,
                );
            }
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
    let projected_api = crate::codegen::foreign_cfg_variants::project_docs_without_unreachable_foreign_variants(
        api, config, languages,
    )?;
    let validated_api = validate_generation_api(&projected_api, config, languages)?;

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
    let projected_api = crate::codegen::foreign_cfg_variants::project_docs_without_unreachable_foreign_variants(
        api, config, languages,
    )?;
    let validated_api = validate_generation_api(&projected_api, config, languages)?;
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
/// appended (e.g. Python's `__init__.py`, Ruby's `<gem>.rb`, PHP's facade class
/// file). Some conventions are dynamic — the Ruby gem entry is named after the
/// gem, the PHP facade after the extension — so this resolves against the crate
/// config rather than returning a fixed string. Languages with no recognized
/// entry-file convention (or whose entry file is produced outside this
/// public-API pass) return an empty list, making the additions a silent no-op.
/// New languages are added here only as their entry file is produced within this
/// pass — Go, Dart, and Node emit their entry file (`binding.go`, the Dart
/// barrel, the JS/TS surface) in a different pass, so wiring them here would be a
/// no-op and is deliberately omitted.
fn package_entry_filenames(language: Language, config: &ResolvedCrateConfig) -> Vec<String> {
    match language {
        Language::Python => vec!["__init__.py".to_string()],
        Language::Ruby => vec![format!("{}.rb", config.ruby_gem_name().replace('-', "_"))],
        Language::Php => {
            use heck::ToPascalCase;
            vec![format!("{}.php", config.php_extension_name().to_pascal_case())]
        }
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
    let projected_api = crate::codegen::foreign_cfg_variants::project_docs_without_unreachable_foreign_variants(
        api, config, languages,
    )?;
    let validated_api = validate_generation_api(&projected_api, config, languages)?;

    let results: Vec<(Language, Vec<GeneratedFile>)> = languages
        .par_iter()
        .map(|&lang| {
            let Some(backend) = registry::try_get_backend(lang) else {
                return Ok((lang, Vec::new()));
            };
            let mut files = backend.generate_public_api_checked(validated_api, config)?;

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

/// The requested languages whose bindings are generated against the FFI crate while the crate does
/// not configure one.
///
/// ~keep `configured` is the crate's declared language set; `requested` is what this run was asked
/// to generate. The check used to test `requested` for FFI, but that is the `--lang`-filtered set,
/// so every deliberate single-language regen (`alef generate --lang csharp`) warned that FFI was
/// missing even when the FFI crate was configured, generated and committed. The condition the
/// message describes is a property of the configuration, not of one invocation's scope.
fn languages_missing_ffi(configured: &[Language], requested: &[Language]) -> Vec<Language> {
    if configured.contains(&Language::Ffi) {
        return Vec::new();
    }
    requested
        .iter()
        .copied()
        .filter(|lang| matches!(lang, Language::Go | Language::Java | Language::Csharp))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::extension::{Extension, ExtensionConfig};

    /// A `--lang`-filtered run must not be reported as a missing-FFI configuration.
    #[test]
    fn languages_missing_ffi_table() {
        let go_java_csharp = [Language::Go, Language::Java, Language::Csharp];
        struct Case {
            name: &'static str,
            configured: Vec<Language>,
            requested: Vec<Language>,
            expected: Vec<Language>,
        }
        let cases = [
            Case {
                name: "a single-language regen of a crate that configures ffi warns about nothing",
                configured: vec![Language::Ffi, Language::Csharp, Language::Go],
                requested: vec![Language::Csharp],
                expected: Vec::new(),
            },
            Case {
                name: "a crate that configures no ffi still warns for every ffi-dependent language",
                configured: go_java_csharp.to_vec(),
                requested: go_java_csharp.to_vec(),
                expected: go_java_csharp.to_vec(),
            },
            Case {
                name: "languages that do not bind through ffi are never reported",
                configured: vec![Language::Python, Language::Ruby],
                requested: vec![Language::Python, Language::Ruby],
                expected: Vec::new(),
            },
            Case {
                name: "a full regen of a crate that configures ffi warns about nothing",
                configured: vec![Language::Ffi, Language::Go],
                requested: vec![Language::Ffi, Language::Go],
                expected: Vec::new(),
            },
        ];

        for case in cases {
            assert_eq!(
                languages_missing_ffi(&case.configured, &case.requested),
                case.expected,
                "case `{}`",
                case.name
            );
        }
    }

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
        assert_eq!(files[1].content, "# native\n");

        append_public_api_additions(&mut files, Language::Ruby, &test_cfg(), &additions);
        assert_eq!(files[0].content.matches("require_relative 'pkg/app'").count(), 1);
    }

    #[test]
    fn public_api_additions_ruby_normalizes_dashed_name_to_snake_entry() {
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
    fn public_api_additions_php_facade_entry_appended_and_idempotent() {
        let additions = vec!["require_once __DIR__ . '/Extra.php';".to_string()];
        let mut files = vec![
            GeneratedFile {
                path: std::path::PathBuf::from("packages/php/src/Pkg.php"),
                content: "<?php\n\nnamespace Pkg;\n\nclass Pkg\n{\n}\n".to_string(),
                generated_header: false,
            },
            GeneratedFile {
                path: std::path::PathBuf::from("packages/php/src/SomeType.php"),
                content: "<?php\n// opaque\n".to_string(),
                generated_header: false,
            },
        ];

        append_public_api_additions(&mut files, Language::Php, &test_cfg(), &additions);
        assert!(files[0].content.contains("require_once __DIR__ . '/Extra.php';"));
        assert_eq!(files[1].content, "<?php\n// opaque\n");

        append_public_api_additions(&mut files, Language::Php, &test_cfg(), &additions);
        assert_eq!(
            files[0].content.matches("require_once __DIR__ . '/Extra.php';").count(),
            1
        );
    }

    #[test]
    fn public_api_additions_php_noop_when_facade_absent() {
        let additions = vec!["require_once 'x';".to_string()];
        let mut files = vec![GeneratedFile {
            path: std::path::PathBuf::from("packages/php/src/SomeType.php"),
            content: "<?php\n// opaque\n".to_string(),
            generated_header: false,
        }];
        append_public_api_additions(&mut files, Language::Php, &test_cfg(), &additions);
        assert_eq!(files[0].content, "<?php\n// opaque\n");
    }

    #[test]
    fn public_api_additions_php_resolves_dashed_name_to_pascal_facade() {
        let config = ResolvedCrateConfig {
            name: "my-ext".to_string(),
            ..Default::default()
        };
        let additions = vec!["require_once 'x';".to_string()];
        let mut files = vec![GeneratedFile {
            path: std::path::PathBuf::from("packages/php/src/MyExt.php"),
            content: "<?php\n".to_string(),
            generated_header: false,
        }];
        append_public_api_additions(&mut files, Language::Php, &config, &additions);
        assert!(files[0].content.contains("require_once 'x';"));
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

    // Regression test mirroring `c_language_is_skipped_gracefully_instead_of_panicking`
    // in the build pipeline: `registry::get_backend` panics for `C` (it has no binding
    // backend — it's an e2e/consumer-only target). `generate()`'s `to_generate` filter
    // used to call `get_backend` unconditionally for every language reaching the second
    // pass, so a `[workspace] languages` list including "c" (a documented, valid e2e
    // target) would crash generation instead of skipping C gracefully. ~keep
    #[test]
    fn c_language_is_skipped_gracefully_instead_of_panicking() {
        let api = ApiSurface::default();
        let config = test_cfg();
        let config_path = std::path::Path::new("does-not-exist-alef.toml");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generate(&api, &config, &[Language::C], true, config_path, true)
        }));

        match result {
            Ok(generate_result) => match generate_result {
                Ok(files) => assert!(
                    files.is_empty(),
                    "C has no binding backend and must be skipped cleanly: {files:?}"
                ),
                Err(e) => panic!("generating for an unsupported binding target must not error: {e}"),
            },
            Err(_) => panic!("generating for an unsupported binding target must not panic"),
        }
    }

    /// Regression for making `alef verify` read-only again: its missing-file
    /// check regenerates bindings in memory only to look for absent files, and
    /// must not leave `.alef/<crate>/hashes/<lang>.{hash,manifest}` behind —
    /// acquiring a cache-write side effect under a command named `verify` was
    /// the bug. `write_cache = true` (what `alef generate`/`alef all`/`alef init`
    /// pass) must still record the cache normally. ~keep
    #[test]
    fn write_cache_flag_gates_the_lang_hash_cache_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(dir.path());

        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "cache-write-test".to_string(),
            ..Default::default()
        };
        let config_path = std::path::Path::new("does-not-exist-alef.toml");
        let hash_path = dir.path().join(".alef/cache-write-test/hashes/ffi.hash");

        let read_only_result = generate(&api, &config, &[Language::Ffi], true, config_path, false);
        let existed_after_read_only = hash_path.exists();

        let writing_result = generate(&api, &config, &[Language::Ffi], true, config_path, true);
        let existed_after_write = hash_path.exists();

        read_only_result.expect("write_cache=false must not turn a successful generate into an error");
        writing_result.expect("write_cache=true must not turn a successful generate into an error");
        assert!(
            !existed_after_read_only,
            "write_cache=false must not create the language hash cache file"
        );
        assert!(
            existed_after_write,
            "write_cache=true must create the language hash cache file"
        );
    }

    /// Reproduces the self-erasing baseline reported against `alef all`'s
    /// binding-orphan sweep. `write_lang_hash` (called right here, at
    /// `generate`'s `write_cache` branch above, on every cache-miss
    /// regeneration) and `cache::read_lang_manifest` (the source of
    /// `previous_paths` at the `alef all` binding-orphan-sweep call site in
    /// `src/bin_cli/all_commands.rs`) both resolve to the identical on-disk
    /// file: `.alef/<crate>/hashes/<lang>.manifest`. That call site always runs
    /// after `generate()` in the same command invocation, so the "previous run"
    /// baseline it reads back is not the previous run's output -- it is this
    /// run's output, written moments earlier by the very call under test here.
    /// A path this run stopped emitting (e.g. a type folded into a capsule
    /// type) was never part of that write, so it can never appear in the read,
    /// and is therefore invisible to `sweep_manifest_orphans` as a candidate --
    /// not because the sweep's own matching logic is wrong (see
    /// `orphans.rs`'s `manifest_sweep_*` tests, which prove it is correct given
    /// a correct baseline), but because the baseline itself was clobbered
    /// before the sweep ever read it. This is why the sweep measured zero
    /// orphans in production regardless of whether any genuinely existed. ~keep
    #[test]
    fn lang_manifest_baseline_self_erases_before_the_orphan_sweep_ever_reads_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(dir.path());

        let result = (|| -> anyhow::Result<()> {
            let package_dir = dir.path().join("packages/python");
            std::fs::create_dir_all(&package_dir)?;
            let dropped_type_file = package_dir.join("dropped_type.py");
            let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
            let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
            std::fs::write(&dropped_type_file, &hashed)?;

            // Run N-1: the type still existed, so generation wrote both the file
            // above (not simulated here -- assumed already on disk from that
            // run) and recorded it in the language manifest via the exact
            // `write_lang_hash` call `generate`'s `write_cache` branch makes. ~keep
            cache::write_lang_hash(
                "sample",
                "python",
                &cache::compute_lang_hash("", "hash-n-minus-1", ""),
                std::slice::from_ref(&dropped_type_file),
            )?;

            // Run N: the type folded into a capsule type, so this run's
            // generated file list no longer includes it. `generate` calls
            // `write_lang_hash` again, unconditionally, with the smaller list --
            // before `all_commands.rs` ever reads the manifest as
            // `previous_paths` for the orphan sweep. ~keep
            cache::write_lang_hash("sample", "python", &cache::compute_lang_hash("", "hash-n", ""), &[])?;

            // Mirrors exactly what the `alef all` binding-orphan sweep does:
            // read the language manifest as the previous-run baseline. ~keep
            let previous_paths = cache::read_lang_manifest("sample", "python");
            assert!(
                !previous_paths.contains(&dropped_type_file),
                "the write above already erased the dropped path from the baseline before this \
                 read -- reproducing the self-erasure"
            );

            let keep = std::collections::HashSet::new();
            let removed = crate::cli::pipeline::sweep_manifest_orphans(&previous_paths, &keep, &[package_dir], &[])?;

            assert_eq!(
                removed, 0,
                "the sweep finds nothing to remove not because the file isn't an orphan, but \
                 because its baseline was clobbered before the sweep ever ran"
            );
            assert!(
                dropped_type_file.exists(),
                "the dropped type's file survives on disk with its now-stale marker, invisible to \
                 the sweep, ready to be re-stamped by finalize_hashes_sweeping"
            );
            Ok(())
        })();

        result.expect("reproduction body");
    }

    /// A minimal but fully resolved Python crate config, mirroring
    /// `backends::pyo3::gen_bindings::tests::python_config` so the tests below exercise the
    /// real, already-proven-working config-resolution path rather than a hand-built
    /// `ResolvedCrateConfig::default()` whose defaults pyo3 codegen does not otherwise see
    /// exercised. ~keep
    fn python_fixture() -> (ApiSurface, ResolvedCrateConfig) {
        let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "test_lib"
"#,
        )
        .expect("parse fixture alef.toml");
        (
            ApiSurface::default(),
            cfg.resolve().expect("resolve fixture config").remove(0),
        )
    }

    /// Reproduces alef#158 for `pyo3`/Python: on a real consumer tree, `python.manifest` held
    /// exactly one path (its native-extension `lib.rs`) against six alef-marked files actually
    /// on disk. `generate()` above writes `<lang>.manifest` unconditionally via
    /// `write_lang_hash`, from `generate_bindings_checked`'s own return value alone --
    /// `bin_cli/all_commands.rs`'s `alef all` generate step runs `generate_public_api`
    /// afterward (to emit the `.py` package: `options.py`, `api.py`, `exceptions.py`,
    /// `__init__.py`) but never reconciles that output back into the manifest, unlike
    /// `bin_cli/core_commands.rs`'s `alef generate`, which accumulates every phase's
    /// alef-marked paths and calls `cache::write_lang_manifest` once at the end. This test runs
    /// the exact `alef all` sequence -- `generate()` then `generate_public_api()`, nothing in
    /// between -- and pins that the manifest is left holding only the binding crate's own
    /// path. ~keep
    #[test]
    fn python_manifest_holds_only_the_binding_crate_when_public_api_is_never_reconciled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(dir.path());

        let (api, config) = python_fixture();
        let config_path = std::path::Path::new("does-not-exist-alef.toml");
        let base_dir = std::env::current_dir().unwrap_or_default();

        generate(&api, &config, &[Language::Python], true, config_path, true).expect("generate bindings");
        let public_api_files =
            generate_public_api(&api, &config, &[Language::Python], config_path).expect("generate public api");

        let public_api_paths: Vec<std::path::PathBuf> = public_api_files
            .iter()
            .flat_map(|(_, files)| files.iter())
            .map(|file| file.path.clone())
            .collect();
        assert_eq!(
            public_api_paths,
            vec![
                std::path::PathBuf::from("packages/python/test_lib/options.py"),
                std::path::PathBuf::from("packages/python/test_lib/api.py"),
                std::path::PathBuf::from("packages/python/test_lib/exceptions.py"),
                std::path::PathBuf::from("packages/python/test_lib/__init__.py"),
            ],
            "generate_public_api must still be the one emitting the python package tree"
        );

        // The binding crate's own source resolves under `crates/test-lib-py/src` for this
        // fixture. The exact location is incidental; the property under test is that the
        // manifest holds exactly ONE path -- the generate_bindings output -- while the four
        // public-api files above are unrecorded. ~keep
        let manifest = cache::read_lang_manifest("test-lib", "python");
        assert_eq!(
            manifest,
            vec![base_dir.join("crates/test-lib-py/src/lib.rs")],
            "on the unfixed `alef all` sequence, `<lang>.manifest` must hold only \
             generate_bindings' own path -- the four public-api files above are unrecorded"
        );
    }

    /// The remedy already implemented for `alef generate`
    /// (`bin_cli/core_commands.rs::Commands::Generate`): once a caller unions every phase's
    /// alef-marked output and writes it back through [`cache::write_lang_manifest`], the
    /// manifest holds the exact full set. Proves the gap pinned above is a missing call at the
    /// `alef all` call site, not a defect in `write_lang_manifest`/`read_lang_manifest`
    /// themselves. ~keep
    #[test]
    fn write_lang_manifest_records_the_full_union_once_every_phase_is_reconciled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(dir.path());

        let (api, config) = python_fixture();
        let config_path = std::path::Path::new("does-not-exist-alef.toml");
        let base_dir = std::env::current_dir().unwrap_or_default();

        let bindings =
            generate(&api, &config, &[Language::Python], true, config_path, true).expect("generate bindings");
        let public_api_files =
            generate_public_api(&api, &config, &[Language::Python], config_path).expect("generate public api");

        let mut full_set: Vec<std::path::PathBuf> = bindings
            .iter()
            .flat_map(|(_, files)| files.iter())
            .filter(|file| file.carries_alef_marker())
            .map(|file| base_dir.join(&file.path))
            .collect();
        full_set.extend(
            public_api_files
                .iter()
                .flat_map(|(_, files)| files.iter())
                .filter(|file| file.carries_alef_marker())
                .map(|file| base_dir.join(&file.path)),
        );

        cache::write_lang_manifest("test-lib", "python", &full_set).expect("write reconciled manifest");

        let mut manifest = cache::read_lang_manifest("test-lib", "python");
        manifest.sort();
        let mut expected = vec![
            base_dir.join("crates/test-lib-py/src/lib.rs"),
            base_dir.join("packages/python/test_lib/options.py"),
            base_dir.join("packages/python/test_lib/api.py"),
            base_dir.join("packages/python/test_lib/exceptions.py"),
            base_dir.join("packages/python/test_lib/__init__.py"),
        ];
        expected.sort();
        assert_eq!(
            manifest, expected,
            "write_lang_manifest, once called with every phase's contribution, already \
             records the exact full set -- the fix is wiring the call, not this function"
        );
    }
}
