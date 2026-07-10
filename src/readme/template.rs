use super::paths::{lang_code, readme_output_path, readme_target_output_path};
use super::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use minijinja::{Environment, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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

    let lang_json: Option<serde_json::Value> = if !readme_cfg.languages.is_empty() {
        readme_cfg.languages.get(lang_code).cloned()
    } else if let Some(config_path) = &readme_cfg.config {
        let abs_config = workspace_root.join(config_path);
        if abs_config.exists() {
            let content = fs::read_to_string(&abs_config)
                .map_err(|e| anyhow::anyhow!("Failed to read readme config {:?}: {}", abs_config, e))?;
            let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Failed to parse readme config YAML: {}", e))?;
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

    let snippets_dir = readme_cfg.snippets_dir.as_ref().map(|s| workspace_root.join(s));
    let snippets_dir_clone = snippets_dir.clone();
    env.add_filter("include_snippet", move |path: String, language: String| -> String {
        match &snippets_dir_clone {
            Some(dir) => include_snippet(dir, &language, &path),
            None => format!("<!-- snippet not found: {path} -->"),
        }
    });

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
    ctx.insert("name", Value::from(name.clone()));
    ctx.insert("description", Value::from(description));
    ctx.insert("license", Value::from(license));
    ctx.insert("repository", Value::from(repository));
    ctx.insert("discord_url", Value::from(discord_url));
    ctx.insert("banner_url", Value::from(banner_url));
    ctx.insert("language", Value::from(language_context.to_string()));

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

    let mut content = tmpl
        .render(ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render template '{}': {}", template_name, e))?;

    if !content.ends_with('\n') {
        content.push('\n');
    }

    Ok(Some(GeneratedFile {
        path,
        content,
        generated_header: false,
    }))
}

/// Load a snippet file. For `.md` files, extract the first fenced code block.
/// For other files, wrap the content in a fenced code block.
pub(super) fn include_snippet(snippets_dir: &Path, lang_code: &str, path: &str) -> String {
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
