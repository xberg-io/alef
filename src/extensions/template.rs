//! Built-in `TemplateExtension` — consumes `[[extensions.template]]` blocks.

use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::extension::{Extension, ExtensionConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_env::TemplateEnv;
use anyhow::{Context as _, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One `[[extensions.template]]` block in `alef.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateBlock {
    pub name: String,
    pub language: Language,
    pub template: PathBuf,
    pub output: PathBuf,
}

/// Parsed config for [`TemplateExtension`].
#[derive(Debug, Clone, Default)]
pub struct TemplateExtensionConfig {
    pub blocks: Vec<TemplateBlock>,
}

/// Built-in extension that renders `[[extensions.template]]` blocks.
pub struct TemplateExtension;

impl Extension for TemplateExtension {
    fn name(&self) -> &str {
        "template"
    }

    fn parse_config(&self, raw: Option<&toml::Value>) -> Result<ExtensionConfig> {
        let Some(raw) = raw else {
            return Ok(ExtensionConfig::empty());
        };
        let blocks: Vec<TemplateBlock> = raw
            .clone()
            .try_into()
            .context("failed to parse [[extensions.template]] blocks")?;
        Ok(ExtensionConfig::with_typed(
            TemplateExtensionConfig { blocks },
            Some(raw.clone()),
        ))
    }

    fn emit_for_language(
        &self,
        api: &ApiSurface,
        cfg: &ExtensionConfig,
        language: Language,
        _env: &TemplateEnv,
    ) -> Result<Vec<GeneratedFile>> {
        let Some(typed) = cfg.downcast::<TemplateExtensionConfig>() else {
            return Ok(vec![]);
        };

        let mut files = Vec::new();
        for block in typed.blocks.iter().filter(|b| b.language == language) {
            let template_src = read_template(&block.template)?;
            let mut local_env = minijinja::Environment::new();
            crate::template::configure_env(&mut local_env);
            local_env
                .add_template_owned(block.name.clone(), template_src)
                .with_context(|| format!("failed to load template `{}`", block.template.display()))?;

            let ctx = crate::alef_context! {
                crate_name => &api.crate_name,
                version => &api.version,
                language => language.to_string(),
            };

            let content = local_env
                .get_template(&block.name)
                .unwrap()
                .render(ctx)
                .with_context(|| format!("failed to render template `{}`", block.template.display()))?;

            files.push(GeneratedFile {
                path: block.output.clone(),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }
}

fn read_template(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("failed to read template file `{}`", path.display()))
}
