//! Extension trait and supporting types for alef.

use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::ir::ApiSurface;
use crate::core::template_env::TemplateEnv;
use anyhow::{Context as _, Result};
use std::any::Any;
use std::path::Path;

/// Opaque per-extension configuration.
pub struct ExtensionConfig {
    pub inner: Option<Box<dyn Any + Send + Sync>>,
    pub raw: Option<toml::Value>,
}

impl ExtensionConfig {
    /// Construct an empty config.
    pub fn empty() -> Self {
        Self { inner: None, raw: None }
    }

    /// Construct from a raw TOML value.
    pub fn from_raw(raw: toml::Value) -> Self {
        Self {
            inner: None,
            raw: Some(raw),
        }
    }

    /// Construct with typed inner config and raw TOML value.
    pub fn with_typed<T: Any + Send + Sync>(typed: T, raw: Option<toml::Value>) -> Self {
        Self {
            inner: Some(Box::new(typed)),
            raw,
        }
    }

    /// Downcast the typed inner config to `T`.
    pub fn downcast<T: Any>(&self) -> Option<&T> {
        self.inner.as_ref().and_then(|b| b.downcast_ref::<T>())
    }
}

impl std::fmt::Debug for ExtensionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionConfig")
            .field("has_inner", &self.inner.is_some())
            .field("has_raw", &self.raw.is_some())
            .finish()
    }
}

/// Extension point for alef's code generation pipeline.
///
/// All three methods have default no-op implementations; override only what
/// you need.
pub trait Extension: Send + Sync {
    /// Stable, unique slug for this extension. Used as the TOML config key.
    fn name(&self) -> &str;

    /// Parse this extension's TOML section.
    ///
    /// Default: returns [`ExtensionConfig::empty`].
    fn parse_config(&self, raw: Option<&toml::Value>) -> Result<ExtensionConfig> {
        let _ = raw;
        Ok(ExtensionConfig::empty())
    }

    /// Augment the API surface after extraction and before generation.
    ///
    /// Default: no-op.
    fn augment_surface(&self, _api: &mut ApiSurface, _cfg: &ExtensionConfig) -> Result<()> {
        Ok(())
    }

    /// Emit extra files for one language.
    ///
    /// Default: returns an empty list.
    fn emit_for_language(
        &self,
        _api: &ApiSurface,
        _cfg: &ExtensionConfig,
        _language: Language,
        _env: &TemplateEnv,
    ) -> Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    /// Transform the complete file list for one language after backend generation
    /// and after [`Extension::emit_for_language`] has appended any extension-owned files.
    ///
    /// Use this hook to rewrite, filter, or replace backend-generated files —
    /// for example to patch a generated type definition or inject content into
    /// an existing file. The `files` slice includes both backend-generated files
    /// and any files previously appended by other extensions for this language.
    ///
    /// Default: no-op.
    fn transform_emitted_files(
        &self,
        _api: &ApiSurface,
        _cfg: &ExtensionConfig,
        _language: Language,
        _files: &mut Vec<GeneratedFile>,
        _env: &TemplateEnv,
    ) -> Result<()> {
        Ok(())
    }
}

/// Read the `[extensions.<name>]` section from `alef.toml` at `config_path`.
///
/// Returns `None` when the file has no `[extensions]` table or no key matching
/// `name`.  Returns the raw [`toml::Value`] of that sub-table when present.
///
/// Called by the extract and generation pipeline stages so every extension
/// receives its own TOML section rather than `None`.  Backwards compatible:
/// consumers that have no `[extensions.<name>]` block continue to receive
/// `None` in [`Extension::parse_config`].
pub fn read_extension_config(config_path: &Path, name: &str) -> Result<Option<toml::Value>> {
    let content = std::fs::read_to_string(config_path).with_context(|| {
        format!(
            "failed to read alef.toml for extension config ({})",
            config_path.display()
        )
    })?;
    let doc: toml::Value = toml::from_str(&content).with_context(|| {
        format!(
            "failed to parse alef.toml for extension config ({})",
            config_path.display()
        )
    })?;
    let raw = doc.get("extensions").and_then(|ext| ext.get(name)).cloned();
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopExtension;
    impl Extension for NoopExtension {
        fn name(&self) -> &str {
            "noop"
        }
    }

    #[test]
    fn extension_config_empty_round_trip() {
        let cfg = ExtensionConfig::empty();
        assert!(cfg.inner.is_none());
        assert!(cfg.raw.is_none());
        assert!(cfg.downcast::<u32>().is_none());
    }

    #[test]
    fn extension_config_with_typed_downcasts() {
        let cfg = ExtensionConfig::with_typed(42u32, None);
        assert_eq!(cfg.downcast::<u32>(), Some(&42u32));
        assert!(cfg.downcast::<String>().is_none());
    }

    #[test]
    fn noop_extension_parse_config_returns_empty_for_none() {
        let ext = NoopExtension;
        let cfg = ext.parse_config(None).expect("parse_config failed");
        assert!(cfg.inner.is_none());
        assert!(cfg.raw.is_none());
    }

    #[test]
    fn noop_extension_parse_config_accepts_raw_value() {
        let ext = NoopExtension;
        let raw = toml::Value::String("some_value".to_string());
        // Default impl ignores the raw value — this tests that it compiles and
        // returns empty regardless of what value is passed.
        let cfg = ext.parse_config(Some(&raw)).expect("parse_config failed");
        assert!(cfg.inner.is_none());
    }

    #[test]
    fn read_extension_config_missing_section_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        std::fs::write(&path, "[workspace]\nalef_version = \"0.1.0\"\n").unwrap();
        let result = read_extension_config(&path, "my_ext").expect("read failed");
        assert!(result.is_none());
    }

    #[test]
    fn read_extension_config_present_section_returns_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        std::fs::write(
            &path,
            "[workspace]\nalef_version = \"0.1.0\"\n\n[extensions.my_ext]\nfoo = \"bar\"\n",
        )
        .unwrap();
        let result = read_extension_config(&path, "my_ext").expect("read failed");
        let val = result.expect("expected Some");
        assert_eq!(val.get("foo").and_then(|v| v.as_str()), Some("bar"));
    }

    #[test]
    fn read_extension_config_unknown_extension_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        std::fs::write(
            &path,
            "[workspace]\nalef_version = \"0.1.0\"\n\n[extensions.other_ext]\nfoo = \"bar\"\n",
        )
        .unwrap();
        let result = read_extension_config(&path, "my_ext").expect("read failed");
        assert!(result.is_none());
    }

    #[test]
    fn transform_emitted_files_default_is_noop() {
        use crate::core::backend::GeneratedFile;
        let ext = NoopExtension;
        let api = ApiSurface::default();
        let cfg = ExtensionConfig::empty();
        let env = crate::core::template_env::TemplateEnv::new();
        let mut files = vec![GeneratedFile {
            path: std::path::PathBuf::from("test.rs"),
            content: "fn main() {}".to_string(),
            generated_header: false,
        }];
        ext.transform_emitted_files(&api, &cfg, Language::Python, &mut files, &env)
            .expect("transform failed");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "fn main() {}");
    }
}
