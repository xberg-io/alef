use crate::backends::napi::gen_bindings::errors;
use crate::core::backend::GeneratedFile;
use crate::core::config::{
    AdapterPattern, Language, NodeCapsuleTypeConfig, OutputLayout, ResolvedCrateConfig, resolve_output_dir,
};
use crate::core::ir::ApiSurface;
use std::collections::HashMap;

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
    let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Node)?;
    // The overlay must declare exactly the variants `gen_bindings`'s emitted Rust enum declares,
    // so it is given the same two inputs that emitter resolves them from -- `gen_enum` and
    // `gen_dts` then reach one verdict from one authority instead of two independent readings of
    // the IR. See `codegen::conversions::enum_variant_declaration`. ~keep
    let core_import = config.core_import_name();
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Node);
    let configured_features: std::collections::HashSet<&str> = enabled_features.iter().map(String::as_str).collect();
    let mut content = errors::gen_dts(
        api,
        &prefix,
        &exclude_functions,
        &config.trait_bridges,
        &capsule_types,
        &streaming_item_types,
        &default_types,
        &adapter_bodies,
        &core_import,
        Some(&configured_features),
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
        path: OutputLayout::from_output_dir(&src_dir).root.join("index.d.ts"),
        content,
        generated_header: false,
    }])
}
