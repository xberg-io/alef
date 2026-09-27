use std::collections::HashSet;
use std::path::Path;

use crate::codegen::naming::kotlin_android_wrapper_object_name;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FunctionDef};

#[cfg(test)]
use crate::core::config::HostCapsuleTypeConfig;

mod capsule_functions;
mod facade_functions;
mod facade_types;
mod handle_wrappers;
mod helpers;

use self::facade_types::jni_param_type_str;

#[cfg(test)]
use self::capsule_functions::emit_capsule_function_wrapper;

/// Emit `<Module>.kt`, its opaque handle wrappers, and streaming adapters.
pub(super) fn emit_module_kt(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
) {
    let module_name = kotlin_android_wrapper_object_name(&config.name);
    let bridge_name = crate::core::jni::bridge_class_name(&config.name);
    let opaque_type_names = opaque_type_names(api);
    let visible_functions = visible_functions(api, config);

    handle_wrappers::emit_handle_wrappers(
        api,
        &handle_wrappers::HandleWrapperContext {
            config,
            kotlin_source_dir,
            package,
            bridge_name: &bridge_name,
        },
        files,
        &visible_functions,
    );
    if visible_functions.is_empty() {
        return;
    }
    facade_functions::emit_facade(
        facade_functions::FacadeParams {
            config,
            kotlin_source_dir,
            package,
            module_name: &module_name,
            bridge_name: &bridge_name,
            opaque_types: &opaque_type_names,
            functions: &visible_functions,
        },
        files,
    );
}

fn opaque_type_names(api: &ApiSurface) -> HashSet<String> {
    api.types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait)
        .map(|type_def| type_def.name.clone())
        .collect()
}

fn visible_functions<'a>(api: &'a ApiSurface, config: &ResolvedCrateConfig) -> Vec<&'a FunctionDef> {
    let excluded: HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|android| android.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();
    api.functions
        .iter()
        .filter(|function| {
            !excluded.contains(function.name.as_str())
                && !trait_bridge_manages_android_function(function.name.as_str(), config)
        })
        .collect()
}

fn trait_bridge_manages_android_function(function_name: &str, config: &ResolvedCrateConfig) -> bool {
    config.trait_bridges.iter().any(|bridge| {
        !bridge.exclude_languages.iter().any(|lang| lang == "kotlin_android")
            && (bridge.register_fn.as_deref() == Some(function_name)
                || bridge.unregister_fn.as_deref() == Some(function_name)
                || bridge.clear_fn.as_deref() == Some(function_name))
    })
}

#[cfg(test)]
mod tests;
