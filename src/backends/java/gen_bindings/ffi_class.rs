use crate::core::config::{HostCapsuleTypeConfig, ResolvedCrateConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, FunctionDef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

use super::marshal::gen_helper_methods;

mod async_wrappers;
mod conversion_internals;
mod params_returns;
mod sync_functions;
mod visitor_bridge;

use conversion_internals::gen_convert_with_visitor_internal_method;
use visitor_bridge::visitor_bridge_for_function;

#[cfg(test)]
mod tests;

pub(crate) fn gen_async_wrapper_method(
    out: &mut String,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    async_wrappers::gen_async_wrapper_method(out, func, bridge_param_names, bridge_type_aliases);
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) fn gen_sync_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
    clear_fn_handles: &AHashMap<String, String>,
    capsule_types: &HashMap<String, HostCapsuleTypeConfig>,
) {
    sync_functions::gen_sync_function_method(
        out,
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        has_visitor_bridge,
        clear_fn_handles,
        capsule_types,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_main_class(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    class_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
    capsule_types: &HashMap<String, HostCapsuleTypeConfig>,
) -> String {
    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let clear_fn_handles: AHashMap<String, String> = config
        .trait_bridges
        .iter()
        .filter_map(|b| {
            b.clear_fn.as_ref().map(|clear_fn| {
                let trait_snake_upper = b.trait_name.to_snake_case().to_uppercase();
                let handle = format!("{}_CLEAR_{}", prefix.to_uppercase(), trait_snake_upper);
                (clear_fn.clone(), handle)
            })
        })
        .collect();

    let mut body = String::with_capacity(4096);

    let header_out = crate::backends::java::template_env::render(
        "ffi_main_class_header.jinja",
        minijinja::context! { class_name => class_name },
    );
    body.push_str(&header_out);
    body.push('\n');

    for func in &api.functions {
        sync_functions::gen_sync_function_method_with_visitor(
            &mut body,
            func,
            prefix,
            class_name,
            &opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            has_visitor_bridge,
            &clear_fn_handles,
            visitor_bridge_for_function(func, config).as_ref(),
            capsule_types,
        );
        body.push('\n');

        let generate_config = config.generate_overrides.get("java").unwrap_or(&config.generate);
        if func.is_async && generate_config.async_wrappers {
            gen_async_wrapper_method(&mut body, func, bridge_param_names, bridge_type_aliases);
            body.push('\n');
        }
    }

    if has_visitor_bridge {
        for func in &api.functions {
            if let Some(visitor_bridge) = visitor_bridge_for_function(func, config) {
                body.push_str(&gen_convert_with_visitor_internal_method(
                    func,
                    class_name,
                    prefix,
                    &opaque_types,
                    bridge_param_names,
                    bridge_type_aliases,
                    &visitor_bridge,
                ));
                body.push('\n');
            }
        }
    }

    gen_helper_methods(&mut body, prefix, class_name);

    let footer_out = crate::backends::java::template_env::render("ffi_main_class_footer.jinja", minijinja::context! {});
    body.push_str(&footer_out);

    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = crate::backends::java::template_env::render(
        "ffi_imports.jinja",
        minijinja::context! {
            header => header,
            package => package,
            needs_arena => body.contains("Arena"),
            needs_function_descriptor => body.contains("FunctionDescriptor"),
            needs_linker => body.contains("Linker"),
            needs_memory_segment => body.contains("MemorySegment"),
            needs_symbol_lookup => body.contains("SymbolLookup"),
            needs_value_layout => body.contains("ValueLayout"),
            needs_list => body.contains("List<"),
            needs_map => body.contains("Map<"),
            needs_optional => body.contains("Optional<"),
            needs_hash_map => body.contains("HashMap<") || body.contains("new HashMap"),
            needs_completable_future => body.contains("CompletableFuture"),
            needs_completion_exception => body.contains("CompletionException"),
            needs_object_mapper => body.contains(" ObjectMapper"),
            needs_jackson_json_node => body.contains("JsonNode"),
            needs_nullable => body.contains("@Nullable"),
        },
    );

    out.push_str(&body);

    out
}
