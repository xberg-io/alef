use crate::backends::napi::gen_bindings::errors;
use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, NodeCapsuleTypeConfig, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::ApiSurface;
use std::collections::HashMap;
use std::path::PathBuf;

pub(super) fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let prefix = config.node_type_prefix();
    let exclude_functions: ahash::AHashSet<String> = config
        .node
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let capsule_types: HashMap<String, NodeCapsuleTypeConfig> = config
        .node
        .as_ref()
        .map(|c| c.capsule_types.clone())
        .unwrap_or_default();
    let streaming_item_types: ahash::AHashMap<String, String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| {
            let owner = a.owner_type.as_deref()?;
            let item = a.item_type.as_deref()?;
            Some((format!("{owner}.{}", a.name), item.to_string()))
        })
        .collect();
    let default_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default)
        .map(|t| t.name.clone())
        .collect();
    let mut content = errors::gen_dts(
        api,
        &prefix,
        &exclude_functions,
        &config.trait_bridges,
        &capsule_types,
        &streaming_item_types,
        &default_types,
    );
    if !config.components.is_empty() {
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(
            "export function componentLoad(component: string): Promise<void>\nexport function componentPrefetch(components?: string[] | null): Promise<string[]>\nexport function componentStatus(component: string): string\nexport function componentCachePath(component: string): string\n",
        );
    }
    let src_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");

    Ok(vec![GeneratedFile {
        path: crate_root(&src_dir).join("index.d.ts"),
        content,
        generated_header: false,
    }])
}

fn crate_root(src_dir: &str) -> PathBuf {
    let path = PathBuf::from(src_dir);
    match path.file_name().and_then(|name| name.to_str()) {
        Some("src") => path.parent().map(|parent| parent.to_path_buf()).unwrap_or(path),
        _ => path,
    }
}
