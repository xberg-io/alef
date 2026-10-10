//! Thin wrapper over `minijinja::Environment` for use in extensions.

use anyhow::{Context as _, Result};
use minijinja::Environment;

/// A handle over a Jinja2 template environment.
pub struct TemplateEnv {
    env: Environment<'static>,
}

impl TemplateEnv {
    /// Create an empty template environment.
    pub fn new() -> Self {
        let mut env = Environment::new();
        crate::template::configure_env(&mut env);
        Self { env }
    }

    /// Register a template by name and source string.
    pub fn register_template(&mut self, name: &'static str, source: &'static str) -> Result<()> {
        self.env
            .add_template(name, source)
            .with_context(|| format!("failed to register template `{name}`"))
    }

    /// Render a template by name with the provided context.
    pub fn render<S: serde::Serialize>(&self, name: &str, ctx: S) -> Result<String> {
        let tmpl = self
            .env
            .get_template(name)
            .with_context(|| format!("template `{name}` not registered in TemplateEnv"))?;
        let rendered = tmpl
            .render(crate::template::to_value(&ctx))
            .with_context(|| format!("failed to render template `{name}`"))?;
        Ok(rendered)
    }
}

impl Default for TemplateEnv {
    fn default() -> Self {
        Self::new()
    }
}
