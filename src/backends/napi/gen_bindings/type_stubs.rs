mod esm;
mod native_exports;

use crate::backends::napi::gen_bindings::errors;
use crate::codegen::naming::{node_type_name, to_node_name};
use crate::codegen::shared::split_bytes_field;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{
    AdapterPattern, Language, NodeCapsuleTypeConfig, OutputLayout, ResolvedCrateConfig, resolve_output_dir,
};
use crate::core::ir::ApiSurface;
use esm::{gen_esm_wrapper, runtime_export_names};
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
        &config.trait_bridges_for(Language::Node).cloned().collect::<Vec<_>>(),
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

    let crate_root = OutputLayout::from_output_dir(&src_dir).root;
    let entrypoint = if api.services.is_empty() {
        "./index.js"
    } else {
        "./index-wrapper.cjs"
    };
    let names = collect_runtime_exports(api, config, &mut content)?;
    let esm_wrapper = gen_esm_wrapper(&names, entrypoint);

    Ok(vec![
        GeneratedFile {
            path: crate_root.join("index.d.ts"),
            content,
            generated_header: false,
        },
        // Both files are plain JavaScript with a leading do-not-edit comment: `.cjs` and `.mjs`
        // carry no provenance marker, so the comment is the only ownership signal on the file.
        GeneratedFile {
            path: crate_root.join("binding.cjs"),
            content: BINDING_SHIM.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: crate_root.join("index.mjs"),
            content: esm_wrapper,
            generated_header: false,
        },
    ])
}

fn collect_runtime_exports(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    content: &mut String,
) -> anyhow::Result<Vec<String>> {
    let declared_names = runtime_export_names(content);
    let mut names = declared_names.clone();
    for binding in super::NapiBackend.generate_bindings(api, config)? {
        names.extend(native_exports::runtime_export_names(&binding.content)?);
    }
    names.sort();
    names.dedup();
    append_split_bytes_declarations(api, &names, &declared_names, content);
    Ok(names)
}

fn append_split_bytes_declarations(
    api: &ApiSurface,
    names: &[String],
    declared_names: &[String],
    content: &mut String,
) {
    for typ in &api.types {
        if split_bytes_field(typ).is_none() {
            continue;
        }
        let name = format!("{}FromJsonWithBytes", to_node_name(&typ.name));
        if names.binary_search(&name).is_err() || declared_names.contains(&name) {
            continue;
        }
        let return_type = node_type_name(&typ.name);
        content.push_str(&format!(
            "\nexport declare function {name}(bytes: Uint8Array, metadataJson: string): {return_type};\n"
        ));
    }
}

/// The CommonJS shim that moves the error properties from the `cause` payload onto each thrown
/// error. `index.js` passes the native binding through it.
const BINDING_SHIM: &str = include_str!("../templates/binding.cjs");
