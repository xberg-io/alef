use std::collections::BTreeSet;
use std::path::Path;

use crate::backends::kotlin::{emit_kdoc_pub, kotlin_type_str_pub, to_lower_camel};
use crate::backends::kotlin_android::template_env;
use crate::backends::kotlin_android::trait_bridge;
use crate::core::backend::GeneratedFile;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use crate::core::jni::bridge_class_name;

use super::assemble_kt_content;

pub(super) fn emit_trait_interfaces(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
) {
    let kotlin_android_excluded_function_names: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let mut effective_excluded_types: std::collections::HashSet<String> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    for bridge in &config.trait_bridges {
        if bridge.exclude_languages.iter().any(|l| l == "kotlin_android") {
            if let Some(alias) = &bridge.type_alias {
                effective_excluded_types.insert(alias.clone());
            }
        }
        if let Some(name) = bridge.param_name.as_deref() {
            if kotlin_android_excluded_function_names.contains(name) {
                if let Some(alias) = &bridge.type_alias {
                    effective_excluded_types.insert(alias.clone());
                }
            }
        }
    }
    for name in api.excluded_type_paths.keys() {
        effective_excluded_types.insert(name.clone());
    }

    for bridge in &config.trait_bridges {
        if bridge
            .exclude_languages
            .iter()
            .any(|language| language == "kotlin_android")
        {
            continue;
        }

        if let Some(param_name) = &bridge.param_name {
            if kotlin_android_excluded_function_names.contains(param_name.as_str()) {
                continue;
            }
        }
        let Some(trait_def) = api
            .types
            .iter()
            .find(|typ| typ.is_trait && typ.name == bridge.trait_name && !typ.binding_excluded)
        else {
            continue;
        };

        let interface_name = format!("I{}", trait_def.name);
        let mut imports = BTreeSet::new();
        let mut body = String::new();
        emit_kdoc_pub(&mut body, &trait_def.doc, "");
        body.push_str(&template_env::render(
            "trait_interface_header.jinja",
            minijinja::context! {
                interface_name => interface_name,
            },
        ));
        if bridge.super_trait.is_some() {
            body.push_str("    fun name(): String\n");
            body.push_str("    fun version(): String\n");
            body.push_str("    fun initialize() {}\n");
            body.push_str("    fun shutdown() {}\n");
            body.push_str("    fun description(): String = \"\"\n");
            body.push_str("    fun author(): String = \"\"\n");
        }
        emit_trait_methods(
            api,
            bridge,
            trait_def,
            &effective_excluded_types,
            &mut imports,
            &mut body,
        );
        body.push_str("}\n");

        let content = assemble_kt_content(package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{interface_name}.kt")),
            content,
            generated_header: false,
        });

        let bridge_class = bridge_class_name(&config.name);
        for (filename, bridge_content) in trait_bridge::gen_trait_bridge_files(
            package,
            &trait_def.name,
            bridge,
            trait_def,
            &bridge_class,
            api,
            &effective_excluded_types,
        ) {
            files.push(GeneratedFile {
                path: kotlin_source_dir.join(filename),
                content: bridge_content,
                generated_header: false,
            });
        }
    }
}

/// Format a trait/interface method signature, wrapping long signatures across
/// multiple lines to avoid AGP parser cascade errors on lines >=115 chars.
///
/// When the full single-line signature would exceed ~110 chars, emits a
/// multi-line form with parameters indented and trailing commas:
///
/// ```kotlin
///     suspend fun extractFile(
///         path: java.nio.file.Path,
///         mimeType: String,
///         config: ExtractionConfig,
///     ): ExtractionResult
/// ```
///
/// Short signatures remain single-line. Empty parameter lists are always
/// single-line even if return type is long.
pub fn format_method_signature(suspend_keyword: &str, method_name: &str, params: &str, return_type: &str) -> String {
    let base_sig = format!("{suspend_keyword}fun {method_name}(");
    let indent = "    ";
    let full_sig_no_newline = format!(
        "{indent}{base_sig}{params}{}{}",
        if return_type == "Unit" { "" } else { "): " },
        return_type
    );

    const THRESHOLD: usize = 110;

    if params.is_empty() || full_sig_no_newline.len() < THRESHOLD {
        if return_type == "Unit" {
            format!("{indent}{base_sig}{params})\n")
        } else {
            format!("{indent}{base_sig}{params}): {return_type}\n")
        }
    } else {
        let mut result = format!("{indent}{base_sig}\n");
        for param in params.split(", ") {
            result.push_str("        ");
            result.push_str(param);
            result.push_str(",\n");
        }
        if return_type == "Unit" {
            result.push_str("    )\n");
        } else {
            result.push_str(&template_env::render(
                "trait_method_return_line.jinja",
                minijinja::context! {
                    return_type => return_type,
                },
            ));
        }
        result
    }
}

fn emit_trait_methods(
    api: &ApiSurface,
    bridge: &TraitBridgeConfig,
    trait_def: &TypeDef,
    excluded_types: &std::collections::HashSet<String>,
    imports: &mut BTreeSet<String>,
    body: &mut String,
) {
    let visible_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.binding_excluded && !excluded_types.contains(&t.name))
        .map(|t| t.name.as_str())
        .chain(api.enums.iter().map(|e| e.name.as_str()))
        .collect();

    for method in &trait_def.methods {
        if method.sanitized || method.is_static {
            continue;
        }
        emit_kdoc_pub(body, &method.doc, "    ");
        let suspend_keyword = if method.is_async { "suspend " } else { "" };
        let method_name = to_lower_camel(&method.name);
        let params = method
            .params
            .iter()
            .map(|param| {
                let name = to_lower_camel(&param.name);
                let ty_ref = substitute_trait_carrier_type(api, bridge, &param.ty);
                let ty = kotlin_type_str_visible(&ty_ref, param.optional, &visible_type_names, imports);
                format!("{name}: {ty}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let return_type_ref = substitute_trait_carrier_type(api, bridge, &method.return_type);
        let return_type = kotlin_type_str_visible(&return_type_ref, false, &visible_type_names, imports);
        body.push_str(&format_method_signature(
            suspend_keyword,
            &method_name,
            &params,
            &return_type,
        ));
    }
}

/// Map a `TypeRef` to its Kotlin representation, substituting `String` for any
/// `Named` type that is not in the set of visible (generated) types.
/// This prevents excluded/internal types like `InternalDocument` from appearing
/// in trait interface signatures where they are not defined.
fn kotlin_type_str_visible(
    ty: &crate::core::ir::TypeRef,
    optional: bool,
    visible_type_names: &std::collections::HashSet<&str>,
    imports: &mut BTreeSet<String>,
) -> String {
    match ty {
        crate::core::ir::TypeRef::Named(name) if !visible_type_names.contains(name.as_str()) => {
            if optional {
                "String?".to_string()
            } else {
                "String".to_string()
            }
        }
        crate::core::ir::TypeRef::Optional(inner) => kotlin_type_str_visible(inner, true, visible_type_names, imports),
        other => kotlin_type_str_pub(other, optional, imports),
    }
}

fn substitute_trait_carrier_type(api: &ApiSurface, bridge: &TraitBridgeConfig, ty: &TypeRef) -> TypeRef {
    match ty {
        TypeRef::Named(name) if should_project_trait_carrier(api, bridge, name) => TypeRef::Named(
            bridge
                .result_type
                .as_ref()
                .expect("checked by should_project_trait_carrier")
                .clone(),
        ),
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_trait_carrier_type(api, bridge, inner))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_trait_carrier_type(api, bridge, inner))),
        TypeRef::Map(key, value) => TypeRef::Map(
            Box::new(substitute_trait_carrier_type(api, bridge, key)),
            Box::new(substitute_trait_carrier_type(api, bridge, value)),
        ),
        other => other.clone(),
    }
}

fn should_project_trait_carrier(api: &ApiSurface, bridge: &TraitBridgeConfig, type_name: &str) -> bool {
    bridge.context_type.as_deref() == Some(type_name)
        && bridge.result_type.is_some()
        && (api.excluded_type_paths.contains_key(type_name)
            || api
                .types
                .iter()
                .any(|typ| typ.name == type_name && (typ.binding_excluded || typ.is_opaque)))
}
