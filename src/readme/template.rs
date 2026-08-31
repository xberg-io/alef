use super::paths::{lang_code, readme_output_path, readme_target_output_path};
use super::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef};
use minijinja::{Environment, Value};
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SnippetMapping {
    Path(String),
    Detailed { path: String, root: PathBuf },
}

#[derive(Serialize)]
struct ReadmeFunction {
    name: String,
    rust_name: String,
    is_async: bool,
    documentation: String,
}

impl SnippetMapping {
    fn into_parts(self) -> (String, Option<PathBuf>) {
        match self {
            Self::Path(path) => (path, None),
            Self::Detailed { path, root } => (path, Some(root)),
        }
    }
}

/// The `crates.readme.languages.<lang_code>` entry for a language, if one is configured.
///
/// Prefers the inline TOML `languages` map and falls back to the deprecated external YAML file
/// named by `crates.readme.config`. Shared with the hardcoded fallback generator, which needs
/// the very same entry to honour a configured `output_path`: resolving "the language's config
/// entry" separately in each route is how the fallback came to consult neither source and
/// silently derive its own path instead. ~keep
pub(super) fn language_entry(
    readme_cfg: &crate::core::config::ReadmeConfig,
    workspace_root: &Path,
    lang_code: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !readme_cfg.languages.is_empty() {
        return Ok(readme_cfg.languages.get(lang_code).cloned());
    }
    let Some(config_path) = &readme_cfg.config else {
        return Ok(None);
    };
    let abs_config = workspace_root.join(config_path);
    if !abs_config.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&abs_config)
        .map_err(|e| anyhow::anyhow!("Failed to read readme config {:?}: {}", abs_config, e))?;
    let as_json: serde_json::Value =
        serde_saphyr::from_str(&content).map_err(|e| anyhow::anyhow!("Failed to parse readme config YAML: {}", e))?;
    Ok(as_json.get("languages").and_then(|l| l.get(lang_code)).cloned())
}

/// Attempt to render a README using a minijinja template. Returns `None` when no
/// language-specific template entry is found in the config (signals caller to fall back).
pub(super) fn try_template_readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
    readme_cfg: &crate::core::config::ReadmeConfig,
    workspace_root: &Path,
    abs_template_dir: &Path,
) -> anyhow::Result<Option<GeneratedFile>> {
    let lang_code = lang_code(lang);

    let Some(lang_json) = language_entry(readme_cfg, workspace_root, lang_code)? else {
        return Ok(None);
    };

    let path = readme_output_path(config, lang, readme_cfg, &lang_json);

    render_template_readme(
        api,
        config,
        readme_cfg,
        workspace_root,
        abs_template_dir,
        &lang_json,
        "language_package.md",
        lang_code,
        path,
        false,
        Some(lang),
    )
}

/// Render a configured non-language README target.
pub(super) fn render_target_readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    target_name: &str,
    target_json: &serde_json::Value,
    readme_cfg: &crate::core::config::ReadmeConfig,
    workspace_root: &Path,
    abs_template_dir: &Path,
) -> anyhow::Result<GeneratedFile> {
    let path = readme_target_output_path(target_name, target_json)?;
    render_template_readme(
        api,
        config,
        readme_cfg,
        workspace_root,
        abs_template_dir,
        target_json,
        "root.md",
        target_name,
        path,
        true,
        None,
    )?
    .ok_or_else(|| anyhow::anyhow!("README target '{target_name}' could not be rendered"))
}

#[allow(clippy::too_many_arguments)]
fn render_template_readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    readme_cfg: &crate::core::config::ReadmeConfig,
    workspace_root: &Path,
    abs_template_dir: &Path,
    entry_json: &serde_json::Value,
    default_template: &str,
    language_context: &str,
    path: PathBuf,
    require_template: bool,
    lang: Option<Language>,
) -> anyhow::Result<Option<GeneratedFile>> {
    let discord_url = readme_cfg.discord_url.as_deref().unwrap_or("").to_string();
    let banner_url = readme_cfg.banner_url.as_deref().unwrap_or("").to_string();

    let template_name = entry_json
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or(default_template)
        .to_string();

    let template_file = abs_template_dir.join(&template_name);
    if !template_file.exists() {
        if require_template {
            anyhow::bail!("README template '{}' does not exist", template_file.display());
        }
        return Ok(None);
    }

    let abs_template_dir_owned = abs_template_dir.to_path_buf();
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
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

    let snippets_dir = entry_json
        .get("snippets_dir")
        .and_then(|value| value.as_str())
        .map(|path| workspace_root.join(path))
        .or_else(|| readme_cfg.snippets_dir.as_ref().map(|path| workspace_root.join(path)));
    let snippets_dir_clone = snippets_dir.clone();
    let workspace_root_for_snippets = workspace_root.to_path_buf();

    // A README language may borrow its code snippets from a differently-named
    // snippet directory (e.g. an `ffi` README pulling examples from a `c/`
    // snippet root, because the FFI binding *is* a C API and consumer repos
    // already maintain a single `c/` snippet set rather than a duplicate
    // `ffi/` one). `crates.readme.languages.<name>.snippet_language` names
    // that source directory; it defaults to the language's own code so every
    // existing config is unaffected. Only applies when a template calls
    // `include_snippet(language)` with the current README's own language
    // variable — an explicit literal (e.g. `include_snippet("python")`) is
    // passed through unchanged. ~keep
    let snippet_language_alias: Option<String> = entry_json
        .get("snippet_language")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let language_context_owned = language_context.to_string();

    env.add_filter(
        "include_snippet",
        move |mapping: Value, language: String| -> Result<String, minijinja::Error> {
            // Undefined must be caught before struct deserialization: minijinja's
            // `Deserializer for &Value` visits an undefined value as unit, which
            // fails both `SnippetMapping` variants and collapses into serde's
            // generic untagged-enum "did not match any variant" message —
            // discarding the actionable, config-key-naming error a caller needs
            // when `snippets.<key>` was never populated (e.g. no snippets are
            // configured for this README language at all). ~keep
            if mapping.is_undefined() {
                return Err(minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!(
                        "cannot include snippet for language `{language}`: the piped snippet reference is undefined — neither a `snippets.<key>` entry is configured for this README language nor does `crates.readme.snippets_dir` provide a fallback root"
                    ),
                ));
            }
            let mapping: SnippetMapping = SnippetMapping::deserialize(&mapping).map_err(|err| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!("cannot include snippet for language `{language}`: {err}"),
                )
            })?;
            let (path, mapping_root) = mapping.into_parts();
            let resolved_root = mapping_root.map(|root| workspace_root_for_snippets.join(root));
            let dir = resolved_root.as_ref().or(snippets_dir_clone.as_ref());
            let Some(dir) = dir else {
                return Err(minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!(
                        "cannot include snippet `{language}/{path}`: neither this snippet mapping nor the README language nor `crates.readme.snippets_dir` configures a snippet root"
                    ),
                ));
            };
            let resolved_language = if language == language_context_owned {
                snippet_language_alias.as_deref().unwrap_or(&language)
            } else {
                &language
            };
            include_snippet(dir, resolved_language, &path)
                .map_err(|err| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, err.to_string()))
        },
    );

    env.add_filter(
        "render_performance_table",
        |benchmarks: Value, name: String| -> String { render_performance_table(&benchmarks, &name) },
    );

    let workspace_root_clone = workspace_root.to_path_buf();
    env.add_function("has_migration", move |_lang: String, _version: String| -> bool {
        let path = workspace_root_clone
            .join("docs")
            .join("migrations")
            .join(&_lang)
            .join(format!("{_version}.md"));
        path.exists()
    });

    let name = &config.name;
    let description = config
        .scaffold
        .as_ref()
        .and_then(|s| s.description.clone())
        .map(|s| s.trim_end().to_string())
        .unwrap_or_else(|| format!("Bindings for {name}"));
    let repository = config.github_repo();
    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.clone())
        .unwrap_or_else(|| "MIT".to_string());

    let mut ctx: HashMap<&str, Value> = HashMap::new();
    ctx.insert("version", Value::from(api.version.clone()));
    ctx.insert("name", Value::from(escape_markdown_heading_text(name)));
    ctx.insert("description", Value::from(description));
    ctx.insert("license", Value::from(license));
    ctx.insert("repository", Value::from(repository));
    ctx.insert("discord_url", Value::from(discord_url));
    ctx.insert("banner_url", Value::from(banner_url));
    ctx.insert("language", Value::from(language_context.to_string()));
    ctx.insert("functions", Value::from_serialize(readme_functions(api, config, lang)));

    ctx.insert(
        "csharp_wrapper_class",
        Value::from(crate::codegen::naming::csharp_wrapper_class_name(
            &api.crate_name,
            &config.csharp_namespace(),
        )),
    );

    if let serde_json::Value::Object(map) = entry_json {
        for (key, val) in map {
            let rendered_val = if let serde_json::Value::String(s) = val {
                if s.contains("{{") {
                    let rendered = env.render_str(s, &ctx).unwrap_or_else(|_| s.clone());
                    Value::from(rendered)
                } else if key == "name" {
                    // Every shipped README template renders `{{ name }}` as the H1
                    // heading (`# {{ name }}`). A configured display name ending in a
                    // markdown-significant character -- most concretely `#`, as in
                    // "C#" -- is genuinely ambiguous per CommonMark's ATX heading
                    // closing-sequence rule, and downstream markdown formatters that
                    // apply that rule liberally (e.g. treating any trailing `#` as a
                    // closing sequence, not just one preceded by a space) strip it,
                    // silently truncating "C#" to "C" on every regen (#555). Escaping
                    // it here protects every template that interpolates `{{ name }}`
                    // without requiring template authors to remember a filter. ~keep
                    Value::from(escape_markdown_heading_text(s))
                } else {
                    json_to_minijinja_value(val)
                }
            } else {
                json_to_minijinja_value(val)
            };
            ctx.insert(
                // SAFETY: we leak the string to get a &'static str for the HashMap key.
                Box::leak(key.clone().into_boxed_str()),
                rendered_val,
            );
        }
    }

    ctx.entry("snippets")
        .or_insert_with(|| json_to_minijinja_value(&serde_json::Value::Object(Default::default())));

    let tmpl = env
        .get_template(&template_name)
        .map_err(|e| anyhow::anyhow!("Failed to load template '{}': {}", template_name, e))?;

    let mut content = tmpl.render(ctx).map_err(|e| {
        anyhow::anyhow!(
            "Failed to render template '{}': {}",
            template_name,
            describe_minijinja_error(&e)
        )
    })?;

    if !content.ends_with('\n') {
        content.push('\n');
    }

    // `.md` has no `CommentStyle` (`write::marker_comment_style` returns `None`
    // for it), so the generic `ensure_generated_header` stamping pass is a
    // no-op for README output — without a self-embedded marker, the write-time
    // ownership guard could only ever prove ownership via a local `.alef/`
    // record, which does not survive a fresh clone. Embedding the same
    // HTML-comment marker docs pages use makes README ownership provable from
    // content alone, identically in CI and on a warm dev machine. ~keep
    let content = crate::docs::with_html_header(content, "alef readme");

    Ok(Some(GeneratedFile {
        path,
        content,
        generated_header: true,
    }))
}

fn readme_functions(api: &ApiSurface, config: &ResolvedCrateConfig, lang: Option<Language>) -> Vec<ReadmeFunction> {
    let Some(lang) = lang else {
        return Vec::new();
    };
    let features = crate::docs::language_pages::effective_docs_features(api, config, lang);
    let enabled_features = features.iter().map(String::as_str).collect();
    let filtered_api = api.with_cfg_filtered_deep(&enabled_features);
    let (excluded_functions, _) = crate::docs::language_pages::excludes::language_excludes(config, lang);

    filtered_api
        .functions
        .iter()
        .filter(|function| {
            !excluded_functions.contains(&function.name) && (lang == Language::Rust || !function.binding_excluded)
        })
        .map(|function| ReadmeFunction {
            name: readme_function_name(&filtered_api, config, lang, function),
            rust_name: function.name.clone(),
            is_async: function.is_async,
            documentation: function.doc.clone(),
        })
        .collect()
}

fn readme_function_name(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
    function: &FunctionDef,
) -> String {
    use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};

    if matches!(lang, Language::Ffi | Language::C | Language::Jni) {
        return crate::codegen::naming::abi_symbol(&config.ffi_prefix(), &function.name);
    }

    if lang == Language::Go {
        let (_, excluded_types) = crate::docs::language_pages::excludes::language_excludes(config, lang);
        let reserved_type_names = api
            .types
            .iter()
            .filter(|item| !item.is_trait && !excluded_types.contains(&item.name))
            .map(|item| crate::codegen::naming::go_type_name(&item.name))
            .chain(
                api.enums
                    .iter()
                    .filter(|item| !excluded_types.contains(&item.name))
                    .map(|item| crate::codegen::naming::go_type_name(&item.name)),
            )
            .collect();
        return crate::codegen::naming::go_free_function_name(&function.name, &reserved_type_names);
    }

    let source_name = if lang == Language::Ruby {
        crate::backends::magnus::ruby_public_function_name(function)
    } else {
        &function.name
    };
    public_host_identifier(lang, PublicIdentifierKind::Function, source_name)
}

/// Render a minijinja render error together with its full cause chain.
///
/// minijinja wraps errors raised inside `{% include %}`'d partials (such as an
/// `include_snippet` filter failure) in an outer "could not render include"
/// error whose `Display` alone drops the actionable inner message. Walking
/// `source()` surfaces the underlying `include_snippet` failure text (snippet
/// path, language, and roots searched) instead of just the include wrapper. ~keep
fn describe_minijinja_error(err: &minijinja::Error) -> String {
    let mut out = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

/// Load a snippet file. For `.md` files, extract the first fenced code block.
/// For other files, wrap the content in a fenced code block.
///
/// Returns an error naming the language, the requested path, the resolved
/// file path, and the snippets root that was searched when the file does not
/// exist. A silently-substituted placeholder here previously let unresolvable
/// README snippet references ship into published packages with `alef readme`
/// reporting success (0 files skipped). ~keep
pub(super) fn include_snippet(snippets_dir: &Path, lang_code: &str, path: &str) -> anyhow::Result<String> {
    let file = snippets_dir.join(lang_code).join(path);
    if !file.exists() {
        anyhow::bail!(
            "snippet not found: language `{lang_code}`, path `{path}` (resolved to `{}`, searched under snippets root `{}`)",
            file.display(),
            snippets_dir.display()
        );
    }
    let content = fs::read_to_string(&file)
        .map_err(|err| anyhow::anyhow!("failed to read snippet `{}`: {err}", file.display()))?;
    if path.ends_with(".md") {
        Ok(extract_code_block(&content))
    } else {
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("");
        Ok(format!("```{ext}\n{}\n```", content.trim()))
    }
}

/// Extract the first fenced code block from a Markdown string.
/// Returns the original content (including fence markers) if no block is found.
pub(super) fn extract_code_block(md: &str) -> String {
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
pub(super) fn render_performance_table(perf: &Value, _name: &str) -> String {
    use minijinja::value::ValueKind;

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

    let benchmarks = match perf.get_attr("benchmarks") {
        Ok(v) if v.kind() == ValueKind::Seq => v,
        _ => return String::new(),
    };
    let Ok(iter) = benchmarks.try_iter() else {
        return String::new();
    };

    let mut out = String::new();
    if !platform.is_empty() {
        out.push_str(&template_env::render(
            "performance_context.jinja",
            minijinja::context! { platform => platform, function => function, note => note },
        ));
        out.push('\n');
    }

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
            out.push_str(&template_env::render(
                "performance_throughput_row.jinja",
                minijinja::context! { name => name, size => size, latency => latency, throughput => throughput },
            ));
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
            out.push_str(&template_env::render(
                "performance_ops_row.jinja",
                minijinja::context! { name => name, size => size, ops => ops },
            ));
        }
    }
    out
}

/// Convert a `serde_json::Value` into a `minijinja::Value` via serde serialization.
pub(super) fn json_to_minijinja_value(json: &serde_json::Value) -> Value {
    Value::from_serialize(json)
}

/// Backslash-escape a trailing run of `#` characters so a display name can be safely
/// interpolated into a Markdown ATX heading (`# {{ name }}`).
///
/// Per CommonMark, an ATX heading may end in an optional closing sequence of `#`s,
/// but only when that sequence is preceded by a space (`# Title #` closes; `# C#`
/// does not, since the `#` immediately follows `C` with no space). Some downstream
/// Markdown formatters implement the closing-sequence rule more liberally --
/// stripping any trailing `#`s regardless of the preceding character -- which
/// silently truncates a heading like `# C#` to `# C` on every reformat (#555).
/// `\#` renders as a literal `#` in CommonMark and is unambiguous under either
/// interpretation, so escaping only the trailing run (not `#`s in the middle of the
/// name, which are never ambiguous) fixes the general case: any display name ending
/// in one or more `#` characters, not just the literal string "C#".
pub(super) fn escape_markdown_heading_text(name: &str) -> String {
    let trailing_hashes = name.chars().rev().take_while(|&c| c == '#').count();
    if trailing_hashes == 0 {
        return name.to_string();
    }
    let split_at = name.len() - trailing_hashes;
    let (prefix, hashes) = name.split_at(split_at);
    let mut escaped = String::with_capacity(name.len() + trailing_hashes);
    escaped.push_str(prefix);
    for _ in hashes.chars() {
        escaped.push_str("\\#");
    }
    escaped
}
