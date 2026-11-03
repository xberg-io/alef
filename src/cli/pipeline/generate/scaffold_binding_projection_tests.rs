//! Coverage for extension-driven binding projection in scaffold generation.

use super::*;
use crate::core::extension::{Extension, ExtensionConfig};

struct SurfaceProbe;

impl Extension for SurfaceProbe {
    fn name(&self) -> &str {
        "surface-probe"
    }

    fn transform_scaffold_files(
        &self,
        api: &ApiSurface,
        _cfg: &ExtensionConfig,
        _language: Language,
        files: &mut Vec<GeneratedFile>,
        _env: &crate::core::template_env::TemplateEnv,
    ) -> anyhow::Result<()> {
        files.push(GeneratedFile {
            path: "surface.txt".into(),
            content: api
                .types
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>()
                .join(","),
            generated_header: true,
        });
        Ok(())
    }
}

#[test]
fn scaffold_extension_receives_only_the_binding_projection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("alef.toml");
    std::fs::write(&config_path, "").expect("write config");
    let api = ApiSurface {
        types: vec![
            crate::core::ir::TypeDef {
                name: "Visible".to_string(),
                ..Default::default()
            },
            crate::core::ir::TypeDef {
                name: "RustOnly".to_string(),
                binding_excluded: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let extensions: Vec<Box<dyn Extension>> = vec![Box::new(SurfaceProbe)];

    let files = scaffold_with_extensions(
        &api,
        &ResolvedCrateConfig::default(),
        &[Language::Rust],
        &config_path,
        &extensions,
    )
    .expect("scaffold with extension");
    let probe = files
        .iter()
        .find(|file| file.path == Path::new("surface.txt"))
        .expect("probe output");
    assert_eq!(probe.content, "Visible");
}
