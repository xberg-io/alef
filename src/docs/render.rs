use super::context::{CliCommand, CliSurface, DocsRenderContext, McpItem, McpSurface};
use crate::core::backend::GeneratedFile;
use crate::core::config::{DocsLlmsConfig, DocsSkillsConfig};
use crate::core::hash;
use anyhow::Context as _;
use minijinja::Environment;
use std::fs;
use std::path::{Path, PathBuf};

pub fn generate_cli_doc(surface: &CliSurface, path: PathBuf) -> GeneratedFile {
    let mut out = markdown_header("CLI Reference");
    if surface.commands.is_empty() {
        out.push_str("No CLI commands discovered.\n");
    } else {
        for command in &surface.commands {
            render_command(&mut out, command, 2);
        }
    }
    GeneratedFile {
        path,
        content: out,
        generated_header: true,
    }
}

pub fn generate_mcp_doc(surface: &McpSurface, path: PathBuf) -> GeneratedFile {
    let mut out = markdown_header("MCP Reference");
    render_mcp_items(&mut out, "Tools", &surface.tools);
    render_mcp_items(&mut out, "Prompts", &surface.prompts);
    render_mcp_items(&mut out, "Resources", &surface.resources);
    GeneratedFile {
        path,
        content: out,
        generated_header: true,
    }
}

pub fn render_llms(
    cfg: &DocsLlmsConfig,
    context: &DocsRenderContext,
    workspace_root: &Path,
    snippet_dirs: &[PathBuf],
) -> anyhow::Result<GeneratedFile> {
    let template = cfg
        .template
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("docs.llms.template is required when docs.llms is configured"))?;
    let output = cfg.output.clone().unwrap_or_else(|| PathBuf::from("docs/llms.txt"));
    ensure_managed_or_adopted(workspace_root, &output, cfg.adopt_existing)?;
    let content = render_template_file(workspace_root, template, context, snippet_dirs)?;
    Ok(GeneratedFile {
        path: output,
        content: with_html_header(content),
        generated_header: true,
    })
}

pub fn render_skills(
    cfg: &DocsSkillsConfig,
    context: &DocsRenderContext,
    workspace_root: &Path,
    snippet_dirs: &[PathBuf],
) -> anyhow::Result<Vec<GeneratedFile>> {
    if cfg.outputs.is_empty() {
        anyhow::bail!("docs.skills.outputs must contain at least one skill root when docs.skills is configured");
    }
    let mut rendered = Vec::new();
    let groups = if cfg.templates.is_empty() {
        default_skill_groups(cfg, workspace_root)?
    } else {
        cfg.templates
            .iter()
            .map(|(group, template)| {
                let template_path = template
                    .template
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("docs.skills.templates.{group}.template is required"))?;
                let output = template
                    .output
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(group).join("SKILL.md"));
                Ok((group.clone(), template_path, output))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    };

    for (_, template, relative_output) in groups {
        let template_path = if let Some(template_dir) = &cfg.template_dir {
            template_dir.join(&template)
        } else {
            template
        };
        let content = render_template_file(workspace_root, &template_path, context, snippet_dirs)?;
        for root in &cfg.outputs {
            let output = root.join(&relative_output);
            ensure_managed_or_adopted(workspace_root, &output, cfg.adopt_existing)?;
            rendered.push(GeneratedFile {
                path: output,
                content: with_html_header(content.clone()),
                generated_header: true,
            });
        }
    }

    Ok(rendered)
}

fn default_skill_groups(
    cfg: &DocsSkillsConfig,
    workspace_root: &Path,
) -> anyhow::Result<Vec<(String, PathBuf, PathBuf)>> {
    let Some(template_dir) = &cfg.template_dir else {
        anyhow::bail!("docs.skills.template_dir is required when docs.skills.templates is empty");
    };
    let mut groups = Vec::new();
    for group in ["api", "cli", "mcp"] {
        let template = PathBuf::from(group).join("SKILL.md.jinja");
        let full_template = workspace_root.join(template_dir).join(&template);
        if !full_template.exists() {
            anyhow::bail!("required skill template does not exist: {}", full_template.display());
        }
        groups.push((group.to_string(), template, PathBuf::from(group).join("SKILL.md")));
    }
    Ok(groups)
}

fn render_template_file(
    workspace_root: &Path,
    template: &Path,
    context: &DocsRenderContext,
    snippet_dirs: &[PathBuf],
) -> anyhow::Result<String> {
    let template_path = workspace_root.join(template);
    if !template_path.exists() {
        anyhow::bail!("docs template does not exist: {}", template_path.display());
    }
    let template_dir = template_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("template path has no parent: {}", template_path.display()))?
        .to_path_buf();
    let template_name = template_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("template path is not valid UTF-8: {}", template_path.display()))?
        .to_string();

    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    env.set_loader(move |name: &str| {
        let path = template_dir.join(name);
        match fs::read_to_string(&path) {
            Ok(content) => Ok(Some(content)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                format!("failed to read template {name}: {err}"),
            )),
        }
    });

    let snippet_roots: Vec<PathBuf> = snippet_dirs.iter().map(|dir| workspace_root.join(dir)).collect();
    env.add_filter(
        "include_snippet",
        move |path: String, language: String| -> Result<String, minijinja::Error> {
            include_snippet(&snippet_roots, &language, &path).map_err(|err| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!("failed to include snippet {language}/{path}: {err}"),
                )
            })
        },
    );

    let tmpl = env
        .get_template(&template_name)
        .map_err(|err| anyhow::anyhow!("failed to load template '{}': {err}", template_path.display()))?;
    let mut content = tmpl
        .render(context)
        .map_err(|err| anyhow::anyhow!("failed to render template '{}': {err}", template_path.display()))?;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    Ok(content)
}

fn include_snippet(snippet_roots: &[PathBuf], language: &str, path: &str) -> anyhow::Result<String> {
    for root in snippet_roots {
        let file = root.join(language).join(path);
        if file.exists() {
            let content =
                fs::read_to_string(&file).with_context(|| format!("failed to read snippet {}", file.display()))?;
            return if path.ends_with(".md") {
                Ok(extract_code_block(&content))
            } else {
                let ext = Path::new(path).extension().and_then(|ext| ext.to_str()).unwrap_or("");
                Ok(format!("```{ext}\n{}\n```", content.trim()))
            };
        }
    }
    anyhow::bail!("snippet not found in configured docs.snippets.dirs")
}

fn extract_code_block(md: &str) -> String {
    let mut in_block = false;
    let mut block_lines = Vec::new();
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

pub(super) fn ensure_managed_or_adopted(
    workspace_root: &Path,
    path: &Path,
    adopt_existing: bool,
) -> anyhow::Result<()> {
    let full_path = workspace_root.join(path);
    if !full_path.exists() || adopt_existing {
        return Ok(());
    }
    let content = fs::read_to_string(&full_path).unwrap_or_default();
    if hash::extract_hash(&content).is_some()
        || content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef"))
    {
        return Ok(());
    }
    anyhow::bail!(
        "{} exists but is not Alef-managed; set adopt_existing = true under the matching docs output config to adopt it",
        path.display()
    )
}

fn markdown_header(title: &str) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: \"{title}\"\n"));
    out.push_str("---\n\n");
    out.push_str(&format!("## {title}\n\n"));
    with_html_header(out)
}

pub(super) fn with_html_header(content: String) -> String {
    let content = content.trim_start();
    if content
        .lines()
        .take(10)
        .any(|line| line.contains("auto-generated by alef"))
    {
        return ensure_trailing_newline(content.to_string());
    }

    let header = "<!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
<!-- To regenerate: alef docs -->\n\
<!-- To verify freshness: alef verify --exit-code -->\n\n";
    let mut out = String::with_capacity(content.len() + header.len());
    if let Some(frontmatter_end) = yaml_frontmatter_end(content) {
        out.push_str(&content[..frontmatter_end]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(header);
        out.push_str(content[frontmatter_end..].trim_start());
    } else {
        out.push_str(header);
        out.push_str(content);
    }
    ensure_trailing_newline(out)
}

fn yaml_frontmatter_end(content: &str) -> Option<usize> {
    let mut offset = 0;
    let mut lines = content.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return None;
    }
    offset += first.len();

    for line in lines {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Some(offset);
        }
    }
    None
}

fn ensure_trailing_newline(mut content: String) -> String {
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content
}

fn render_command(out: &mut String, command: &CliCommand, depth: usize) {
    out.push_str(&format!("{} `{}`\n\n", "#".repeat(depth), command.path));
    if !command.about.is_empty() {
        out.push_str(&command.about);
        out.push_str("\n\n");
    }
    render_options(out, "Arguments", &command.positionals);
    render_options(out, "Options", &command.options);
    for child in &command.subcommands {
        render_command(out, child, depth + 1);
    }
}

fn render_options(out: &mut String, title: &str, options: &[crate::docs::context::CliOption]) {
    if options.is_empty() {
        return;
    }
    out.push_str(&format!("### {title}\n\n"));
    out.push_str("| Name | Flags | Type | Default | Description |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for option in options {
        let mut flags = Vec::new();
        if let Some(short) = &option.short {
            flags.push(format!("`-{short}`"));
        }
        if let Some(long) = &option.long {
            flags.push(format!("`--{long}`"));
        }
        out.push_str(&format!(
            "| `{}` | {} | `{}` | {} | {} |\n",
            escape_table(&option.name),
            flags.join(", "),
            escape_table(&option.ty),
            option
                .default
                .as_ref()
                .map(|value| format!("`{}`", escape_table(value)))
                .unwrap_or_default(),
            escape_table(&option.help)
        ));
    }
    out.push('\n');
}

fn render_mcp_items(out: &mut String, title: &str, items: &[McpItem]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("### {title}\n\n"));
    out.push_str("| Name | Title | Parameters | Description |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for item in items {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            escape_table(&item.name),
            escape_table(&item.title),
            item.params_type
                .as_ref()
                .map(|ty| format!("`{}`", escape_table(ty)))
                .unwrap_or_default(),
            escape_table(&item.description)
        ));
    }
    out.push('\n');
}

fn escape_table(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>").trim().to_string()
}
