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
    let esm_wrapper = gen_esm_wrapper(&runtime_export_names(&content), entrypoint);

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

/// The CommonJS shim that moves the error properties from the `cause` payload onto each thrown
/// error. `index.js` passes the native binding through it.
const BINDING_SHIM: &str = include_str!("../templates/binding.cjs");

/// Names the native module exports at runtime, read from the generated declarations: every
/// `class`, `function`, `enum` and `const`. Interfaces and type aliases have no runtime value.
fn runtime_export_names(dts: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in dts.lines() {
        let rest = line.strip_prefix("export declare ").or_else(|| line.strip_prefix("export "));
        let Some(rest) = rest else { continue };
        let mut words = rest.split_whitespace();
        let mut kind = words.next();
        if kind == Some("const") {
            let mut ahead = words.clone();
            if ahead.next() == Some("enum") {
                words.next();
                kind = Some("enum");
            }
        }
        if !matches!(kind, Some("class" | "function" | "enum" | "const" | "let" | "var")) {
            continue;
        }
        let Some(word) = words.next() else { continue };
        let name: String = word
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if is_esm_export_name(&name) && !names.contains(&name) {
            names.push(name);
        }
    }
    names.sort();
    names
}

/// An export name the ESM wrapper can declare with `export const`.
fn is_esm_export_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "default", "class", "function", "const", "let", "var", "new", "delete", "import", "export", "static", "enum",
        "await", "yield", "null", "true", "false", "void", "this", "super", "with", "in", "of", "if", "else",
        "for", "while", "do", "switch", "case", "return", "break", "continue", "try", "catch", "finally", "throw",
        "typeof", "instanceof", "debugger", "extends", "implements", "interface", "package", "private",
        "protected", "public",
    ];
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && !RESERVED.contains(&name)
}

/// The ESM entry point for `import { name } from "<package>"`.
///
/// Node's CommonJS lexer finds named exports only in `module.exports.name = ...` or
/// `exports.name = ...` statements, so `module.exports = nativeBinding` exposes none. This module
/// imports the CommonJS entry as its default export and re-declares each export by name.
fn gen_esm_wrapper(names: &[String], entrypoint: &str) -> String {
    let mut out = String::from("// DO NOT EDIT - auto-generated by alef\n");
    out.push_str(&format!("import binding from \"{entrypoint}\";\n\n"));
    for name in names {
        out.push_str(&format!("export const {name} = binding.{name};\n"));
    }
    out.push_str("\nexport default binding;\n");
    out
}
