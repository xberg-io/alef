use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

pub(crate) fn emit_cargo_toml(
    rust_dir: &str,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    source_crate_name: &str,
) -> GeneratedFile {
    let crate_name = config.name.as_str();
    let version = &api_version(config);
    let frb_version = crate::naming::dart_frb_version(config);
    let core_crate_dir = config.core_crate_for_language(alef_core::config::extras::Language::Dart);
    let dart_override = config.dart.as_ref().and_then(|c| c.core_crate_override.as_deref());
    // Cargo dep KEY: when an override is set, use it as-is; otherwise preserve
    // the historical behaviour (`source_crate_name`, the Rust-ident form of
    // the umbrella crate name) so existing configs produce identical output.
    let core_dep_key: String = match dart_override {
        Some(name) => name.to_string(),
        None => source_crate_name.to_string(),
    };
    let same_as_workspace = dart_override.is_none() && core_crate_dir == *crate_name && config.workspace_root.is_none();
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

    // Trait bridge impl methods use tokio::runtime::Handle::current().block_on(...) and
    // async-trait for async trait impls. Add these only when trait bridges are configured.
    // Note: anyhow is NOT included — bridge impls use source_crate::Result directly.
    let has_trait_bridges = config.trait_bridges.iter().any(|b| {
        !b.exclude_languages.iter().any(|l| l == "dart")
            && api.types.iter().any(|t| t.name == b.trait_name && t.is_trait)
    });
    let trait_bridge_deps = if has_trait_bridges {
        "tokio = { version = \"1\", features = [\"rt\"] }\nasync-trait = \"0.1\"\n"
    } else {
        ""
    };

    // Merge [crate.extra_dependencies] from alef.toml — required for multi-crate
    // workspaces where the bindings codegen emits qualified paths from sibling
    // crates (e.g. mylib_extra::QueryOnlyConfig). The umbrella crate is
    // already listed above; these are the additional sibling crates.
    let workspace_extra = config.extra_deps_for_language(alef_core::config::extras::Language::Dart);
    let mut workspace_dep_lines: Vec<String> = workspace_extra
        .iter()
        .map(|(name, value)| {
            if let Some(s) = value.as_str() {
                format!("{name} = \"{s}\"")
            } else {
                format!("{name} = {value}")
            }
        })
        .collect();
    workspace_dep_lines.sort();
    let workspace_deps_block = if workspace_dep_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", workspace_dep_lines.join("\n"))
    };
    let extra_deps = format!("{trait_bridge_deps}{workspace_deps_block}");

    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.as_deref())
        .unwrap_or("MIT");

    // Build the cargo-machete ignored list: the umbrella crate plus every sibling
    // crate from [crate.extra_dependencies]. flutter_rust_bridge resolves types
    // across all of them, but the generated Rust wrapper only `use`s a subset —
    // cargo-machete would otherwise flag the rest.
    let mut machete_ignored: Vec<String> = std::iter::once(core_dep_key.clone())
        .chain(workspace_extra.keys().cloned())
        .collect();
    machete_ignored.sort();
    machete_ignored.dedup();
    let machete_ignored_list = machete_ignored
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let content = format!(
        r#"[package]
name = "{crate_name}-dart"
version = "{version}"
edition = "2024"
license = "{license}"

[package.metadata.cargo-machete]
# Umbrella + sibling crates are pulled in so flutter_rust_bridge can resolve
# every referenced type, but the generated Rust wrapper only `use`s a subset.
ignored = [{machete_ignored_list}]

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
{core_dep_key} = {{ path = "{core_path}"{features_block} }}
flutter_rust_bridge = "{frb_version}"
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

fn api_version(config: &ResolvedCrateConfig) -> String {
    // Use the resolved version from Cargo.toml if available, otherwise fall back to "0.1.0"
    // as a safe default (the real version is resolved from Cargo.toml at publish time).
    config.resolved_version().unwrap_or_else(|| "0.1.0".to_string())
}
