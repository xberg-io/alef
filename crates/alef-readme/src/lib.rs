//! README generator for alef.

use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use minijinja::{Environment, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Generate README files for the given languages.
pub fn generate_readmes(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![];
    for &lang in languages {
        files.push(generate_readme(api, config, lang)?);
    }
    Ok(files)
}

fn generate_readme(api: &ApiSurface, config: &AlefConfig, lang: Language) -> anyhow::Result<GeneratedFile> {
    // Try template-based generation first when readme config is present
    if let Some(readme_cfg) = &config.readme {
        if let Some(template_dir) = &readme_cfg.template_dir {
            let workspace_root = config
                .crate_config
                .workspace_root
                .clone()
                .unwrap_or_else(|| PathBuf::from("."));
            let abs_template_dir = workspace_root.join(template_dir);
            if abs_template_dir.exists() {
                if let Some(file) =
                    try_template_readme(api, config, lang, readme_cfg, &workspace_root, &abs_template_dir)?
                {
                    return Ok(file);
                }
            }
        }
    }

    // Fall back to hardcoded generation
    generate_readme_hardcoded(api, config, lang)
}

/// Attempt to render a README using a minijinja template. Returns `None` when no
/// language-specific template entry is found in the config (signals caller to fall back).
fn try_template_readme(
    api: &ApiSurface,
    config: &AlefConfig,
    lang: Language,
    readme_cfg: &alef_core::config::ReadmeConfig,
    workspace_root: &Path,
    abs_template_dir: &Path,
) -> anyhow::Result<Option<GeneratedFile>> {
    let lang_code = lang_code(lang);

    // Resolve per-language JSON value. Prefer inline `languages` map in alef.toml;
    // fall back to the deprecated external YAML file when `config` is set.
    let lang_json: Option<serde_json::Value> = if !readme_cfg.languages.is_empty() {
        readme_cfg.languages.get(lang_code).cloned()
    } else if let Some(config_path) = &readme_cfg.config {
        let abs_config = workspace_root.join(config_path);
        if abs_config.exists() {
            let content = fs::read_to_string(&abs_config)
                .map_err(|e| anyhow::anyhow!("Failed to read readme config {:?}: {}", abs_config, e))?;
            let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Failed to parse readme config YAML: {}", e))?;
            // Convert the YAML value to JSON so the rest of the function can use a single type.
            let as_json = serde_json::to_value(&yaml)
                .map_err(|e| anyhow::anyhow!("Failed to convert readme YAML to JSON: {}", e))?;
            as_json.get("languages").and_then(|l| l.get(lang_code)).cloned()
        } else {
            None
        }
    } else {
        None
    };

    let Some(lang_json) = lang_json else {
        // No entry for this language — signal caller to fall back
        return Ok(None);
    };

    // Resolve top-level discord_url / banner_url. Prefer inline fields; fall back to
    // what may have been loaded from the external YAML (not re-loaded here — callers
    // using the deprecated path still get the values injected via the JSON block).
    let discord_url = readme_cfg.discord_url.as_deref().unwrap_or("").to_string();
    let banner_url = readme_cfg.banner_url.as_deref().unwrap_or("").to_string();

    // Determine template name: prefer lang config, then default
    let template_name = lang_json
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or("language_package.md")
        .to_string();

    let template_file = abs_template_dir.join(&template_name);
    if !template_file.exists() {
        // Template file missing — fall back to hardcoded
        return Ok(None);
    }

    // Set up minijinja environment
    let abs_template_dir_owned = abs_template_dir.to_path_buf();
    let mut env = Environment::new();
    env.set_loader(move |name: &str| {
        let path = abs_template_dir_owned.join(name);
        match fs::read_to_string(&path) {
            Ok(content) => Ok(Some(content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                format!("Failed to read template {name}: {e}"),
            )),
        }
    });

    // Register include_snippet filter: {{ path | include_snippet(language) }}
    let snippets_dir = readme_cfg.snippets_dir.as_ref().map(|s| workspace_root.join(s));
    let snippets_dir_clone = snippets_dir.clone();
    env.add_filter("include_snippet", move |path: String, language: String| -> String {
        match &snippets_dir_clone {
            Some(dir) => include_snippet(dir, &language, &path),
            None => format!("<!-- snippet not found: {path} -->"),
        }
    });

    // Register render_performance_table filter: {{ perf | render_performance_table(name) }}
    env.add_filter(
        "render_performance_table",
        |benchmarks: Value, name: String| -> String { render_performance_table(&benchmarks, &name) },
    );

    // Register has_migration function
    let workspace_root_clone = workspace_root.to_path_buf();
    env.add_function("has_migration", move |_lang: String, _version: String| -> bool {
        // Check for migration guide file: docs/migrations/{lang}/{version}.md
        let path = workspace_root_clone
            .join("docs")
            .join("migrations")
            .join(&_lang)
            .join(format!("{_version}.md"));
        path.exists()
    });

    // Build template context
    let name = &config.crate_config.name;
    let description = config
        .scaffold
        .as_ref()
        .and_then(|s| s.description.clone())
        .unwrap_or_else(|| format!("Bindings for {name}"));
    let repository = config
        .scaffold
        .as_ref()
        .and_then(|s| s.repository.clone())
        .unwrap_or_else(|| format!("https://github.com/kreuzberg-dev/{name}"));
    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.clone())
        .unwrap_or_else(|| "MIT".to_string());

    let mut ctx: HashMap<&str, Value> = HashMap::new();
    ctx.insert("version", Value::from(api.version.clone()));
    ctx.insert("name", Value::from(name.clone()));
    ctx.insert("description", Value::from(description));
    ctx.insert("license", Value::from(license));
    ctx.insert("repository", Value::from(repository));
    ctx.insert("discord_url", Value::from(discord_url));
    ctx.insert("banner_url", Value::from(banner_url));
    ctx.insert("language", Value::from(lang_code.to_string()));

    // Flatten per-language config fields into top-level context
    // (templates expect snippets, features, performance, etc. at top level)
    //
    // String values may themselves contain template expressions (e.g. `{{ version }}`
    // in Java/Elixir install_command). We render those inline before inserting them
    // so the outer template receives the final text.
    if let serde_json::Value::Object(map) = &lang_json {
        for (key, val) in map {
            let rendered_val = if let serde_json::Value::String(s) = val {
                if s.contains("{{") {
                    let rendered = env.render_str(s, &ctx).unwrap_or_else(|_| s.clone());
                    Value::from(rendered)
                } else {
                    json_to_minijinja_value(val)
                }
            } else {
                json_to_minijinja_value(val)
            };
            ctx.insert(
                // SAFETY: we leak the string to get a &'static str for the HashMap key.
                // This is fine since readmes are generated once per run.
                Box::leak(key.clone().into_boxed_str()),
                rendered_val,
            );
        }
    }

    let tmpl = env
        .get_template(&template_name)
        .map_err(|e| anyhow::anyhow!("Failed to load template '{}': {}", template_name, e))?;

    let mut content = tmpl
        .render(ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render template '{}': {}", template_name, e))?;

    // Ensure POSIX-compliant trailing newline
    if !content.ends_with('\n') {
        content.push('\n');
    }

    // Determine output path
    let path = readme_output_path(config, lang, readme_cfg, &lang_json);

    Ok(Some(GeneratedFile {
        path,
        content,
        generated_header: false,
    }))
}

/// Determine the output path for a language README.
fn readme_output_path(
    config: &AlefConfig,
    lang: Language,
    readme_cfg: &alef_core::config::ReadmeConfig,
    lang_json: &serde_json::Value,
) -> PathBuf {
    // Check for explicit output_path in per-language config
    if let Some(output) = lang_json
        .get("output_path")
        .or_else(|| lang_json.get("output"))
        .and_then(|v| v.as_str())
    {
        return PathBuf::from(output);
    }

    // Check output_pattern in ReadmeConfig (e.g. "packages/{language}/README.md")
    if let Some(pattern) = &readme_cfg.output_pattern {
        let dir = lang_dir_name(lang);
        return PathBuf::from(pattern.replace("{language}", dir));
    }

    // Default to the same paths as the hardcoded generator
    default_readme_path(config, lang)
}

fn default_readme_path(config: &AlefConfig, lang: Language) -> PathBuf {
    let name = &config.crate_config.name;
    match lang {
        Language::Ffi => PathBuf::from(format!("crates/{name}-ffi/README.md")),
        Language::Wasm => PathBuf::from(format!("crates/{name}-wasm/README.md")),
        _ => PathBuf::from(format!("packages/{}/README.md", lang_dir_name(lang))),
    }
}

/// Return the short directory/key name for a language.
fn lang_dir_name(lang: Language) -> &'static str {
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
    }
}

/// Return the YAML config key for a language.
fn lang_code(lang: Language) -> &'static str {
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
    }
}

/// Load a snippet file. For `.md` files, extract the first fenced code block.
/// For other files, wrap the content in a fenced code block.
fn include_snippet(snippets_dir: &Path, lang_code: &str, path: &str) -> String {
    let file = snippets_dir.join(lang_code).join(path);
    if !file.exists() {
        return format!("<!-- snippet not found: {path} -->");
    }
    let content = fs::read_to_string(&file).unwrap_or_default();
    if path.ends_with(".md") {
        extract_code_block(&content)
    } else {
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("");
        format!("```{ext}\n{}\n```", content.trim())
    }
}

/// Extract the first fenced code block from a Markdown string.
/// Returns the original content (including fence markers) if no block is found.
fn extract_code_block(md: &str) -> String {
    let mut in_block = false;
    let mut block_lines: Vec<&str> = vec![];
    let mut fence_marker = "";

    for line in md.lines() {
        if !in_block {
            if line.starts_with("```") || line.starts_with("~~~") {
                in_block = true;
                fence_marker = if line.starts_with("```") { "```" } else { "~~~" };
                block_lines.push(line);
            }
        } else {
            block_lines.push(line);
            if line.trim() == fence_marker {
                break;
            }
        }
    }

    if block_lines.is_empty() {
        md.to_string()
    } else {
        block_lines.join("\n")
    }
}

/// Render a Markdown performance table from a minijinja benchmarks Value.
///
/// Expects the value to be a sequence of mappings with keys:
/// `name`, `value`, `unit` (optional), `notes` (optional).
fn render_performance_table(perf: &Value, _name: &str) -> String {
    use minijinja::value::ValueKind;

    // Extract platform/function/note metadata
    let platform = perf
        .get_attr("platform")
        .ok()
        .and_then(|v: Value| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let function = perf
        .get_attr("function")
        .ok()
        .and_then(|v: Value| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let note = perf
        .get_attr("note")
        .ok()
        .and_then(|v: Value| v.as_str().map(str::to_string))
        .unwrap_or_default();

    // Extract benchmarks array
    let benchmarks = match perf.get_attr("benchmarks") {
        Ok(v) if v.kind() == ValueKind::Seq => v,
        _ => return String::new(),
    };
    let Ok(iter) = benchmarks.try_iter() else {
        return String::new();
    };

    let mut out = String::new();
    if !platform.is_empty() {
        out.push_str(&format!("**{platform}** · `{function}` · {note}\n\n"));
    }

    // Detect table format: latency/throughput or ops/sec
    let items: Vec<Value> = iter.collect();
    let has_throughput = items
        .iter()
        .any(|item| item.get_attr("throughput").ok().is_some_and(|v| !v.is_undefined()));

    if has_throughput {
        out.push_str("| Document | Size | Latency | Throughput |\n");
        out.push_str("|----------|------|---------|------------|\n");
        for item in &items {
            let name = item
                .get_attr("name")
                .ok()
                .and_then(|v: Value| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let size = item
                .get_attr("size")
                .ok()
                .and_then(|v: Value| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let latency = item
                .get_attr("latency")
                .ok()
                .and_then(|v: Value| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let throughput = item
                .get_attr("throughput")
                .ok()
                .and_then(|v: Value| v.as_str().map(str::to_string))
                .unwrap_or_default();
            out.push_str(&format!("| {name} | {size} | {latency} | {throughput} |\n"));
        }
    } else {
        out.push_str("| Document | Size | Ops/sec |\n");
        out.push_str("|----------|------|---------|\n");
        for item in &items {
            let name = item
                .get_attr("name")
                .ok()
                .and_then(|v: Value| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let size = item
                .get_attr("size")
                .ok()
                .and_then(|v: Value| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let ops = item
                .get_attr("ops_sec")
                .ok()
                .map(|v: Value| format!("{v}"))
                .unwrap_or_default();
            out.push_str(&format!("| {name} | {size} | {ops} |\n"));
        }
    }
    out
}

/// Convert a `serde_json::Value` into a `minijinja::Value` via serde serialization.
fn json_to_minijinja_value(json: &serde_json::Value) -> Value {
    Value::from_serialize(json)
}

// ---------------------------------------------------------------------------
// Hardcoded fallback generator (original implementation)
// ---------------------------------------------------------------------------

fn generate_readme_hardcoded(api: &ApiSurface, config: &AlefConfig, lang: Language) -> anyhow::Result<GeneratedFile> {
    let name = &config.crate_config.name;
    let description = config
        .scaffold
        .as_ref()
        .and_then(|s| s.description.clone())
        .unwrap_or_else(|| format!("Bindings for {}", name));
    let repository = config
        .scaffold
        .as_ref()
        .and_then(|s| s.repository.clone())
        .unwrap_or_else(|| format!("https://github.com/kreuzberg-dev/{}", name));

    let (lang_display, install_instructions, example_code, dir_name) = match lang {
        Language::Python => (
            "Python",
            format!("```bash\npip install {name}\n```"),
            format!(
                "```python\nimport {module}\n\n# TODO: add usage example\n```",
                module = config.python_module_name().trim_start_matches('_')
            ),
            "python",
        ),
        Language::Node => (
            "Node.js",
            format!("```bash\nnpm install {}\n```", config.node_package_name()),
            format!(
                "```typescript\nimport {{ /* ... */ }} from '{}';\n\n// TODO: add usage example\n```",
                config.node_package_name()
            ),
            "typescript",
        ),
        Language::Ruby => (
            "Ruby",
            format!("```bash\ngem install {}\n```", config.ruby_gem_name()),
            format!(
                "```ruby\nrequire '{}'\n\n# TODO: add usage example\n```",
                config.ruby_gem_name()
            ),
            "ruby",
        ),
        Language::Php => (
            "PHP",
            format!("```bash\ncomposer require kreuzberg-dev/{name}\n```"),
            format!(
                "```php\n<?php\n\nuse {};\n\n// TODO: add usage example\n```",
                config.php_extension_name()
            ),
            "php",
        ),
        Language::Elixir => (
            "Elixir",
            format!(
                "Add `:{app}` to your `mix.exs` dependencies:\n\n```elixir\ndefp deps do\n  [\n    {{:{app}, \"~> {version}\"}}\n  ]\nend\n```",
                app = config.elixir_app_name(),
                version = api.version,
            ),
            format!(
                "```elixir\n{module}.hello()\n\n# TODO: add usage example\n```",
                module = capitalize_first(&config.elixir_app_name()),
            ),
            "elixir",
        ),
        Language::Go => (
            "Go",
            format!("```bash\ngo get {}\n```", config.go_module()),
            format!(
                "```go\npackage main\n\nimport \"{module}\"\n\nfunc main() {{\n\t// TODO: add usage example\n}}\n```",
                module = config.go_module(),
            ),
            "go",
        ),
        Language::Java => (
            "Java",
            format!(
                "Add to your `pom.xml`:\n\n```xml\n<dependency>\n    <groupId>{package}</groupId>\n    <artifactId>{name}</artifactId>\n    <version>{version}</version>\n</dependency>\n```",
                package = config.java_package(),
                name = name,
                version = api.version,
            ),
            format!(
                "```java\nimport {package}.*;\n\n// TODO: add usage example\n```",
                package = config.java_package(),
            ),
            "java",
        ),
        Language::Csharp => (
            "C#",
            format!("```bash\ndotnet add package {}\n```", config.csharp_namespace()),
            format!(
                "```csharp\nusing {};\n\n// TODO: add usage example\n```",
                config.csharp_namespace()
            ),
            "csharp",
        ),
        Language::Ffi => (
            "FFI (C/C++)",
            format!(
                "Link against `lib{name}_ffi` and include `{header}`.\n\nSee the build instructions in the main repository.",
                name = name,
                header = config.ffi_header_name(),
            ),
            format!(
                "```c\n#include \"{header}\"\n\nint main(void) {{\n    // TODO: add usage example\n    return 0;\n}}\n```",
                header = config.ffi_header_name(),
            ),
            "ffi",
        ),
        Language::Wasm => (
            "WebAssembly",
            format!("```bash\nnpm install {name}-wasm\n```"),
            format!("```javascript\nimport init from '{name}-wasm';\n\nawait init();\n// TODO: add usage example\n```"),
            "wasm",
        ),
        Language::R => (
            "R",
            format!(
                "```r\ninstall.packages('{package}')\n```",
                package = config.r_package_name()
            ),
            format!(
                "```r\nlibrary({})\n\n# TODO: add usage example\n```",
                config.r_package_name()
            ),
            "r",
        ),
        Language::Rust => (
            "Rust",
            format!("```bash\ncargo add {name}\n```"),
            format!(
                "```rust\nuse {};\n\n// TODO: add usage example\n```",
                config.core_import()
            ),
            "rust",
        ),
    };

    let content = format!(
        r#"# {name} - {lang_display} Bindings

{description}

## Installation

{install}

## Quick Start

{example}

## Documentation

For full documentation, see the [{name} repository]({repository}).

## License

See the [LICENSE]({repository}/blob/main/LICENSE) file in the root repository.
"#,
        name = name,
        lang_display = lang_display,
        description = description,
        install = install_instructions,
        example = example_code,
        repository = repository,
    );

    // Use the readme config output pattern if provided, otherwise default
    let path = match lang {
        Language::Ffi => PathBuf::from(format!("crates/{}-ffi/README.md", name)),
        Language::Wasm => PathBuf::from(format!("crates/{}-wasm/README.md", name)),
        _ => PathBuf::from(format!("packages/{}/README.md", dir_name)),
    };

    Ok(GeneratedFile {
        path,
        content,
        generated_header: false,
    })
}

/// Capitalize the first character of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::*;

    fn test_config() -> AlefConfig {
        AlefConfig {
            crate_config: CrateConfig {
                name: "my-lib".to_string(),
                sources: vec![],
                version_from: "Cargo.toml".to_string(),
                core_import: None,
                workspace_root: None,
                skip_core_import: false,
                features: vec![],
                path_mappings: std::collections::HashMap::new(),
                auto_path_mappings: Default::default(),
                extra_dependencies: Default::default(),
                source_crates: vec![],
                error_type: None,
                error_constructor: None,
            },
            languages: vec![Language::Python, Language::Node],
            exclude: ExcludeConfig::default(),
            include: IncludeConfig::default(),
            output: OutputConfig::default(),
            python: None,
            node: None,
            ruby: None,
            php: None,
            elixir: None,
            wasm: None,
            ffi: None,
            go: None,
            java: None,
            csharp: None,
            r: None,
            scaffold: Some(ScaffoldConfig {
                description: Some("Test library".to_string()),
                license: Some("MIT".to_string()),
                repository: Some("https://github.com/test/my-lib".to_string()),
                homepage: None,
                authors: vec![],
                keywords: vec![],
            }),
            readme: None,
            lint: None,
            custom_files: None,
            adapters: vec![],
            custom_modules: CustomModulesConfig::default(),
            custom_registrations: CustomRegistrationsConfig::default(),
            opaque_types: std::collections::HashMap::new(),
            generate: GenerateConfig::default(),
            generate_overrides: std::collections::HashMap::new(),
            dto: Default::default(),
            sync: None,
            test: None,
            e2e: None,
            trait_bridges: vec![],
        }
    }

    fn test_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn test_generate_python_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/python/README.md"));
        assert!(files[0].content.contains("Python"));
        assert!(files[0].content.contains("pip install"));
    }

    #[test]
    fn test_generate_node_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Node]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/typescript/README.md"));
        assert!(files[0].content.contains("Node.js"));
    }

    #[test]
    fn test_generate_multiple_readmes() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python, Language::Node]).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_extract_code_block() {
        let md = "Some text\n\n```python\nprint('hello')\n```\n\nMore text";
        let result = extract_code_block(md);
        assert!(result.contains("```python"));
        assert!(result.contains("print('hello')"));
    }

    #[test]
    fn test_extract_code_block_no_block() {
        let md = "Just plain text";
        let result = extract_code_block(md);
        assert_eq!(result, "Just plain text");
    }

    #[test]
    fn test_render_performance_table_empty() {
        let v = Value::from(Vec::<Value>::new());
        let result = render_performance_table(&v, "test");
        assert!(result.is_empty());
    }

    #[test]
    fn test_include_snippet_missing() {
        let result = include_snippet(Path::new("/nonexistent"), "python", "foo.py");
        assert!(result.contains("snippet not found"));
    }

    #[test]
    fn test_template_version_in_install_command() {
        // Verify that {{ version }} placeholders in per-language config strings
        // (e.g. Java pom.xml snippet, Elixir mix.exs dep) are rendered.
        let tmp = std::env::temp_dir().join("alef_readme_test_version");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Minimal template that just outputs the install_command
        fs::write(tmp.join("test.md"), "{{ install_command }}").unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(
            "java".to_string(),
            serde_json::json!({
                "template": "test.md",
                "install_command": "<version>{{ version }}</version>",
                "output_path": "packages/java/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api(); // version = "0.1.0"
        let files = generate_readmes(&api, &config, &[Language::Java]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.contains("<version>0.1.0</version>"),
            "Expected version placeholder to be rendered, got: {}",
            files[0].content,
        );
        assert!(
            !files[0].content.contains("{{ version }}"),
            "Raw template placeholder should not remain in output",
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_json_to_minijinja_value_primitives() {
        let json: serde_json::Value = serde_json::from_str(r#"{"key": "value", "num": 42, "flag": true}"#).unwrap();
        let mj = json_to_minijinja_value(&json);
        // The value should be an object accessible by attribute
        assert!(mj.get_attr("key").is_ok());
    }

    // --- generate_readmes: empty language list ---

    #[test]
    fn test_generate_readmes_empty_languages() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[]).unwrap();
        assert_eq!(files.len(), 0);
    }

    // --- hardcoded fallback: remaining language variants ---

    #[test]
    fn test_generate_ruby_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Ruby]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/ruby/README.md"));
        assert!(files[0].content.contains("Ruby"));
        assert!(files[0].content.contains("gem install"));
    }

    #[test]
    fn test_generate_php_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Php]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/php/README.md"));
        assert!(files[0].content.contains("PHP"));
        assert!(files[0].content.contains("composer require"));
    }

    #[test]
    fn test_generate_elixir_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Elixir]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/elixir/README.md"));
        assert!(files[0].content.contains("Elixir"));
        assert!(files[0].content.contains("mix.exs"));
    }

    #[test]
    fn test_generate_go_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Go]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/go/README.md"));
        assert!(files[0].content.contains("Go"));
        assert!(files[0].content.contains("go get"));
    }

    #[test]
    fn test_generate_java_readme_hardcoded() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Java]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/java/README.md"));
        assert!(files[0].content.contains("Java"));
        assert!(files[0].content.contains("pom.xml"));
    }

    #[test]
    fn test_generate_csharp_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Csharp]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/csharp/README.md"));
        assert!(files[0].content.contains("C#"));
        assert!(files[0].content.contains("dotnet add package"));
    }

    #[test]
    fn test_generate_ffi_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("crates/my-lib-ffi/README.md"));
        assert!(files[0].content.contains("FFI"));
    }

    #[test]
    fn test_generate_wasm_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Wasm]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("crates/my-lib-wasm/README.md"));
        assert!(files[0].content.contains("WebAssembly"));
    }

    #[test]
    fn test_generate_r_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::R]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/r/README.md"));
        assert!(files[0].content.contains("install.packages"));
    }

    #[test]
    fn test_generate_rust_readme() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Rust]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/rust/README.md"));
        assert!(files[0].content.contains("Rust"));
        assert!(files[0].content.contains("cargo add"));
    }

    // --- hardcoded fallback: no scaffold config (default description/repository) ---

    #[test]
    fn test_generate_readme_without_scaffold_uses_defaults() {
        let mut config = test_config();
        config.scaffold = None;
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.contains("Bindings for my-lib"),
            "Expected default description, got: {}",
            files[0].content
        );
        assert!(
            files[0].content.contains("https://github.com/kreuzberg-dev/my-lib"),
            "Expected default repository URL, got: {}",
            files[0].content
        );
    }

    // --- capitalize_first ---

    #[test]
    fn test_capitalize_first_normal() {
        assert_eq!(capitalize_first("hello"), "Hello");
    }

    #[test]
    fn test_capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_capitalize_first_already_upper() {
        assert_eq!(capitalize_first("World"), "World");
    }

    // --- extract_code_block: tilde fence ---

    #[test]
    fn test_extract_code_block_tilde_fence() {
        let md = "~~~python\nprint('hi')\n~~~\n";
        let result = extract_code_block(md);
        assert!(result.contains("~~~python"), "Got: {result}");
        assert!(result.contains("print('hi')"), "Got: {result}");
    }

    // --- include_snippet: file exists ---

    #[test]
    fn test_include_snippet_non_md_file() {
        let tmp = std::env::temp_dir().join("alef_readme_snippet_test_py");
        let _ = fs::remove_dir_all(&tmp);
        let lang_dir = tmp.join("python");
        fs::create_dir_all(&lang_dir).unwrap();
        fs::write(lang_dir.join("example.py"), "print('hello')").unwrap();

        let result = include_snippet(&tmp, "python", "example.py");
        assert!(result.contains("```py"), "Got: {result}");
        assert!(result.contains("print('hello')"), "Got: {result}");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_include_snippet_md_file_extracts_code_block() {
        let tmp = std::env::temp_dir().join("alef_readme_snippet_test_md");
        let _ = fs::remove_dir_all(&tmp);
        let lang_dir = tmp.join("python");
        fs::create_dir_all(&lang_dir).unwrap();
        fs::write(
            lang_dir.join("example.md"),
            "Some prose\n\n```python\nfoo()\n```\n\nMore prose",
        )
        .unwrap();

        let result = include_snippet(&tmp, "python", "example.md");
        assert!(result.contains("```python"), "Got: {result}");
        assert!(result.contains("foo()"), "Got: {result}");

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- render_performance_table: ops/sec table ---

    #[test]
    fn test_render_performance_table_ops_sec() {
        let perf = serde_json::json!({
            "platform": "Apple M2",
            "function": "parse",
            "note": "single-threaded",
            "benchmarks": [
                {"name": "small.json", "size": "1 KB", "ops_sec": 12345},
                {"name": "large.json", "size": "1 MB", "ops_sec": 42}
            ]
        });
        let v = Value::from_serialize(&perf);
        let result = render_performance_table(&v, "parse");
        assert!(result.contains("Apple M2"), "Got: {result}");
        assert!(result.contains("| Document | Size | Ops/sec |"), "Got: {result}");
        assert!(result.contains("small.json"), "Got: {result}");
        assert!(result.contains("large.json"), "Got: {result}");
    }

    // --- render_performance_table: throughput table ---

    #[test]
    fn test_render_performance_table_throughput() {
        let perf = serde_json::json!({
            "platform": "Linux x86-64",
            "function": "extract",
            "note": "4 threads",
            "benchmarks": [
                {
                    "name": "doc.pdf",
                    "size": "2 MB",
                    "latency": "10ms",
                    "throughput": "100 MB/s"
                }
            ]
        });
        let v = Value::from_serialize(&perf);
        let result = render_performance_table(&v, "extract");
        assert!(
            result.contains("| Document | Size | Latency | Throughput |"),
            "Got: {result}"
        );
        assert!(result.contains("doc.pdf"), "Got: {result}");
        assert!(result.contains("100 MB/s"), "Got: {result}");
    }

    // --- readme_output_path: output_pattern branch ---

    #[test]
    fn test_template_with_output_pattern() {
        let tmp = std::env::temp_dir().join("alef_readme_test_output_pattern");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("lang.md"), "# {{ name }}").unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        // No output_path key — let output_pattern drive the path
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "lang.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: Some("docs/{language}/README.md".to_string()),
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("docs/python/README.md"));

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- try_template_readme: template file missing falls back to hardcoded ---

    #[test]
    fn test_template_readme_missing_template_falls_back() {
        let tmp = std::env::temp_dir().join("alef_readme_test_missing_tmpl");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        // No template file written — the entry points at a file that does not exist

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "nonexistent.md",
                "output_path": "packages/python/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        // Falls back to hardcoded — must contain pip install
        assert!(
            files[0].content.contains("pip install"),
            "Expected hardcoded fallback content, got: {}",
            files[0].content
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- try_template_readme: no language entry falls back to hardcoded ---

    #[test]
    fn test_template_readme_no_lang_entry_falls_back() {
        let tmp = std::env::temp_dir().join("alef_readme_test_no_lang_entry");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut config = test_config();
        // Languages map is empty — no entry for Python
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: std::collections::HashMap::new(),
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].content.contains("pip install"));

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- try_template_readme: deprecated YAML config path ---

    #[test]
    fn test_template_readme_yaml_config() {
        let tmp = std::env::temp_dir().join("alef_readme_test_yaml_cfg");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("tmpl.md"), "version={{ version }}").unwrap();
        let yaml_content = r#"
languages:
  python:
    template: tmpl.md
    output_path: packages/python/README.md
"#;
        fs::write(tmp.join("readme.yaml"), yaml_content).unwrap();

        let mut config = test_config();
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: Some(PathBuf::from("readme.yaml")),
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: std::collections::HashMap::new(), // empty — triggers YAML path
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.contains("version=0.1.0"),
            "Expected rendered version, got: {}",
            files[0].content
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- try_template_readme: discord_url and banner_url in context ---

    #[test]
    fn test_template_readme_discord_and_banner_url() {
        let tmp = std::env::temp_dir().join("alef_readme_test_discord_banner");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("t.md"), "{{ discord_url }}|{{ banner_url }}").unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "t.md",
                "output_path": "packages/python/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: Some("https://discord.gg/test".to_string()),
            banner_url: Some("https://img.example.com/banner.png".to_string()),
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.contains("https://discord.gg/test"),
            "Got: {}",
            files[0].content
        );
        assert!(
            files[0].content.contains("https://img.example.com/banner.png"),
            "Got: {}",
            files[0].content
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- try_template_readme: template without scaffold (default description/repo/license) ---

    #[test]
    fn test_template_readme_no_scaffold_uses_defaults() {
        let tmp = std::env::temp_dir().join("alef_readme_test_no_scaffold");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("t.md"), "{{ description }}|{{ repository }}|{{ license }}").unwrap();

        let mut config = test_config();
        config.scaffold = None;
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "t.md",
                "output_path": "packages/python/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.contains("Bindings for my-lib"),
            "Got: {}",
            files[0].content
        );
        assert!(
            files[0].content.contains("https://github.com/kreuzberg-dev/my-lib"),
            "Got: {}",
            files[0].content
        );
        assert!(files[0].content.contains("MIT"), "Got: {}", files[0].content);

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- try_template_readme: content already ends with newline (no double-newline) ---

    #[test]
    fn test_template_readme_trailing_newline_not_doubled() {
        let tmp = std::env::temp_dir().join("alef_readme_test_trailing_newline");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Template content ends with a literal newline already
        fs::write(tmp.join("t.md"), "hello\n").unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "t.md",
                "output_path": "packages/python/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].content.ends_with('\n'), "Must end with newline");
        assert!(
            !files[0].content.ends_with("\n\n"),
            "Must not have double trailing newline, got: {:?}",
            files[0].content
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- default_readme_path: all language variants ---

    #[test]
    fn test_default_readme_path_ffi() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
        assert_eq!(files[0].path, PathBuf::from("crates/my-lib-ffi/README.md"));
    }

    #[test]
    fn test_default_readme_path_wasm() {
        let config = test_config();
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Wasm]).unwrap();
        assert_eq!(files[0].path, PathBuf::from("crates/my-lib-wasm/README.md"));
    }

    // --- readme_output_path: "output" key alias ---

    #[test]
    fn test_template_output_key_alias() {
        let tmp = std::env::temp_dir().join("alef_readme_test_output_alias");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("t.md"), "hello").unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        // Use "output" instead of "output_path" to exercise the .or_else() branch
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "t.md",
                "output": "custom/path/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("custom/path/README.md"));

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- readme_output_path: falls through to default_readme_path ---

    #[test]
    fn test_template_readme_default_path_fallthrough() {
        let tmp = std::env::temp_dir().join("alef_readme_test_default_path");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("t.md"), "hello").unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        // No output_path, no output key, no output_pattern — falls through to default_readme_path
        lang_map.insert("python".to_string(), serde_json::json!({ "template": "t.md" }));
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("packages/python/README.md"));

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- include_snippet: snippets_dir in template context (filter exercised via template) ---

    #[test]
    fn test_template_include_snippet_filter() {
        let tmp = std::env::temp_dir().join("alef_readme_test_snippet_filter");
        let _ = fs::remove_dir_all(&tmp);
        let snippets_dir = tmp.join("snippets");
        let lang_snippet_dir = snippets_dir.join("python");
        fs::create_dir_all(&lang_snippet_dir).unwrap();
        fs::write(lang_snippet_dir.join("hello.py"), "print('hi')").unwrap();
        // Template exercises the include_snippet filter
        fs::write(tmp.join("t.md"), r#"{{ "hello.py" | include_snippet("python") }}"#).unwrap();

        let mut config = test_config();
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(
            "python".to_string(),
            serde_json::json!({
                "template": "t.md",
                "output_path": "packages/python/README.md"
            }),
        );
        config.readme = Some(ReadmeConfig {
            template_dir: Some(tmp.clone()),
            snippets_dir: Some(PathBuf::from("snippets")),
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: lang_map,
        });
        config.crate_config.workspace_root = Some(tmp.clone());

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.contains("print('hi')"),
            "Expected snippet content, got: {}",
            files[0].content
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
