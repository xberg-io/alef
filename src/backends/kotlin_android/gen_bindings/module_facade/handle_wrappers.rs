use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use crate::backends::kotlin::{emit_kdoc_pub, to_lower_camel, to_pascal_case};
use crate::backends::kotlin_android::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, TypeDef};

use super::super::assemble_kt_content;

/// Shared, per-crate emission context for [`emit_handle_wrappers`] and [`emit_handle_wrapper`].
/// Bundled purely to keep both functions under poly's `too-many-parameters` limit -- no behavior
/// change, `files` stays a separate `&mut` output parameter since it accumulates across every
/// handle type rather than describing one of them, and `api`/`visible_functions` stay direct
/// parameters on `emit_handle_wrappers` since `emit_handle_wrapper` never needs them.
pub(super) struct HandleWrapperContext<'a> {
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) kotlin_source_dir: &'a Path,
    pub(super) package: &'a str,
    pub(super) bridge_name: &'a str,
}

pub(super) fn emit_handle_wrappers(
    api: &ApiSurface,
    context: &HandleWrapperContext,
    files: &mut Vec<GeneratedFile>,
    visible_functions: &[&FunctionDef],
) {
    let client_types: HashSet<&str> = api
        .types
        .iter()
        .filter(|type_def| has_instance_methods(type_def))
        .map(|type_def| type_def.name.as_str())
        .collect();
    let opaque_type_names: HashSet<&str> = api
        .types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait)
        .map(|type_def| type_def.name.as_str())
        .collect();
    let exclude_functions: HashSet<String> = context
        .config
        .kotlin_android
        .as_ref()
        .map(|android| android.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let capsule_types = context
        .config
        .kotlin_android
        .as_ref()
        .map(|android| android.capsule_types.clone())
        .unwrap_or_default();
    // Same predicate the Bridge object uses to decide which `nativeFree<Type>` externals it
    // declares -- a capsule type's host runtime owns the pointer, so it is excluded from
    // both the destructor declaration and this wrapper-class-with-close() emission. See
    // `crate::backends::kotlin::handle_only_type_names` for why the two must never disagree.
    let capsule_type_names: HashSet<&str> = capsule_types.keys().map(String::as_str).collect();
    let handle_type_names = crate::backends::kotlin::handle_only_type_names(
        api,
        visible_functions,
        &exclude_functions,
        &opaque_type_names,
        &capsule_type_names,
        &client_types,
    );
    let handle_types: BTreeMap<&str, &TypeDef> = api
        .types
        .iter()
        .filter(|type_def| handle_type_names.contains(type_def.name.as_str()))
        .map(|type_def| (type_def.name.as_str(), type_def))
        .collect();
    for (class_name, type_def) in handle_types {
        emit_handle_wrapper(context, files, class_name, type_def);
    }
}

fn has_instance_methods(type_def: &TypeDef) -> bool {
    type_def.is_opaque
        && !type_def.is_trait
        && type_def
            .methods
            .iter()
            .any(|method| !method.sanitized && !method.is_static)
}

fn emit_handle_wrapper(
    context: &HandleWrapperContext,
    files: &mut Vec<GeneratedFile>,
    class_name: &str,
    type_def: &TypeDef,
) {
    let mut body = String::new();
    let mut imports = BTreeSet::new();
    if !type_def.doc.is_empty() {
        emit_kdoc_pub(&mut body, &type_def.doc, "");
    }
    append_handle_header(&mut body, class_name, context.bridge_name);
    let adapters = streaming_adapters(context.config, class_name);
    if !adapters.is_empty() {
        add_streaming_imports(&mut imports);
        append_streaming_mapper(&mut body);
        for adapter in adapters {
            append_streaming_method(&mut body, adapter, class_name, context.bridge_name);
        }
    }
    body.push_str("}\n");
    files.push(GeneratedFile {
        path: context.kotlin_source_dir.join(format!("{class_name}.kt")),
        content: assemble_kt_content(context.package, &imports, &body),
        generated_header: false,
    });
}

fn append_handle_header(body: &mut String, class_name: &str, bridge_name: &str) {
    body.push_str(&template_env::render(
        "handle_wrapper_header.jinja",
        crate::alef_context! {
            class_name => class_name,
            bridge_name => bridge_name,
            free_name => format!("nativeFree{}", to_pascal_case(class_name)),
        },
    ));
}

fn streaming_adapters<'a>(config: &'a ResolvedCrateConfig, class_name: &str) -> Vec<&'a AdapterConfig> {
    config
        .adapters
        .iter()
        .filter(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming))
        .filter(|adapter| {
            !adapter
                .skip_languages
                .iter()
                .any(|language| language == "kotlin_android")
        })
        .filter(|adapter| adapter.owner_type.as_deref() == Some(class_name))
        .collect()
}

fn add_streaming_imports(imports: &mut BTreeSet<String>) {
    for import in [
        "import com.fasterxml.jackson.databind.ObjectMapper",
        "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module",
        "import com.fasterxml.jackson.databind.PropertyNamingStrategies",
        "import kotlinx.coroutines.Dispatchers",
        "import kotlinx.coroutines.flow.Flow",
        "import kotlinx.coroutines.flow.callbackFlow",
        "import kotlinx.coroutines.withContext",
        "import kotlinx.coroutines.channels.awaitClose",
    ] {
        imports.insert(import.to_string());
    }
}

fn append_streaming_mapper(body: &mut String) {
    body.push_str(&template_env::render(
        "android_streaming_mapper.jinja",
        crate::alef_context! {
            duration_millis_module => crate::backends::kotlin::duration_millis_jackson_module(8),
        },
    ));
}

fn append_streaming_method(body: &mut String, adapter: &AdapterConfig, class_name: &str, bridge_name: &str) {
    let owner_pascal = to_pascal_case(class_name);
    let adapter_pascal = to_pascal_case(&adapter.name);
    let first_param_name = adapter
        .params
        .first()
        .map(|param| to_lower_camel(&param.name))
        .unwrap_or_else(|| "request".to_string());
    body.push_str(&template_env::render(
        "android_streaming_method.jinja",
        crate::alef_context! {
            method_name => to_lower_camel(&adapter.name),
            params => streaming_params(adapter),
            item_type => adapter.item_type.as_deref().unwrap_or("Any"),
            bridge_name => bridge_name,
            jni_start => format!("native{owner_pascal}{adapter_pascal}Start"),
            jni_next => format!("native{owner_pascal}{adapter_pascal}Next"),
            jni_free => format!("native{owner_pascal}{adapter_pascal}Free"),
            first_param_name => first_param_name,
        },
    ));
}

fn streaming_params(adapter: &AdapterConfig) -> String {
    adapter
        .params
        .iter()
        .map(|param| {
            let simple_type = param.ty.rsplit("::").next().unwrap_or(&param.ty);
            format!("{}: {simple_type}", to_lower_camel(&param.name))
        })
        .collect::<Vec<_>>()
        .join(", ")
}
