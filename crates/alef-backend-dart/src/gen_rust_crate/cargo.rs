use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::ir::ApiSurface;
use alef_core::template_versions::cargo as tv;
use std::path::PathBuf;

pub(crate) fn emit_cargo_toml(
    rust_dir: &str,
    api: &ApiSurface,
    config: &AlefConfig,
    source_crate_name: &str,
) -> GeneratedFile {
    let crate_name = config.crate_config.name.as_str();
    let version = &api_version(config);
    let frb_version = tv::FLUTTER_RUST_BRIDGE;
    let core_crate_dir = config.core_crate_dir();
    let same_as_workspace = core_crate_dir == *crate_name && config.crate_config.workspace_root.is_none();
    let core_path = if same_as_workspace {
        "../../..".to_string()
    } else {
        format!("../../../crates/{core_crate_dir}")
    };

    let features = config.features_for_language(alef_core::config::extras::Language::Dart);
    let features_block = if features.is_empty() {
        String::new()
    } else {
        let list = features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(", features = [{list}]")
    };

    // Trait bridge impl methods use tokio::runtime::Handle::current().block_on(...),
    // anyhow for error conversion, and async-trait for async trait impls.
    // Add these dependencies only when trait bridges are configured and emitted.
    let has_trait_bridges = config.trait_bridges.iter().any(|b| {
        !b.exclude_languages.iter().any(|l| l == "dart")
            && api.types.iter().any(|t| t.name == b.trait_name && t.is_trait)
    });
    let extra_deps = if has_trait_bridges {
        "tokio = { version = \"1\", features = [\"rt\"] }\nanyhow = \"1\"\nasync-trait = \"0.1\"\n"
    } else {
        ""
    };

    let content = format!(
        r#"[package]
name = "{crate_name}-dart"
version = "{version}"
edition = "2024"

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
{source_crate_name} = {{ path = "{core_path}"{features_block} }}
flutter_rust_bridge = "{frb_version}"
serde_json = "1"
{extra_deps}
[lints.rust]
# flutter_rust_bridge uses #[cfg(frb_expand)] internally during macro expansion.
# Declare it as a known cfg so rustc does not emit unexpected_cfgs warnings.
unexpected_cfgs = {{ level = "warn", check-cfg = ['cfg(frb_expand)'] }}"#
    );

    GeneratedFile {
        path: PathBuf::from(rust_dir).join("Cargo.toml"),
        content,
        generated_header: false,
    }
}

pub(crate) fn emit_build_rs(rust_dir: &str) -> GeneratedFile {
    let content = "fn main() {\n    // FRB codegen runs as a separate command (`flutter_rust_bridge_codegen generate`).\n    // This build.rs is a placeholder for any pre-build steps.\n}\n".to_string();
    GeneratedFile {
        path: PathBuf::from(rust_dir).join("build.rs"),
        content,
        generated_header: false,
    }
}

pub(crate) fn emit_frb_yaml(rust_dir: &str, module_name: &str) -> GeneratedFile {
    // FRB v2 schema: `rust_root` points at the Rust crate dir (not a single file)
    // and `dart_output` is the directory where Dart bindings are written. The v1
    // `rust_input` / `rust_output` keys were removed in v2.
    let content = format!("rust_root: .\ndart_output: ../lib/src/{module_name}_bridge_generated\n");
    GeneratedFile {
        path: PathBuf::from(rust_dir).join("flutter_rust_bridge.yaml"),
        content,
        generated_header: false,
    }
}

fn api_version(config: &AlefConfig) -> String {
    // Use explicit version override if set, otherwise fall back to "0.1.0" as a
    // safe default (the real version is resolved from Cargo.toml at publish time).
    config.version.as_deref().unwrap_or("0.1.0").to_string()
}
