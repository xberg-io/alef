use crate::core::ir::ApiSurface;
use ahash::AHashSet;

pub(super) fn warn_thread_bound_async_handles(
    api: &ApiSurface,
    opaque: &AHashSet<String>,
    send_sync: &AHashSet<String>,
    absent: &AHashSet<String>,
    excluded_functions: &AHashSet<String>,
) {
    let mut candidates: Vec<_> = opaque
        .difference(send_sync)
        .filter(|name| !absent.contains(*name))
        .filter(|name| !api.types.iter().any(|ty| ty.name == **name && ty.is_trait))
        .collect();
    candidates.sort();
    for name in candidates {
        let parameter_use = api.functions.iter().any(|function| {
            function.is_async
                && !function.sanitized
                && !function.binding_excluded
                && !excluded_functions.contains(&function.name)
                && function
                    .params
                    .iter()
                    .any(|parameter| parameter.ty.references_named(name))
        });
        let method_use = captured_by_async_method(api, name, absent);
        if parameter_use || method_use {
            tracing::warn!(opaque_type = %name,
                "Python async parameter or receiver is thread-bound; its final reference may be released on a runtime thread. Configure crates.python.send_sync_types only if the Rust handle supports Send + Sync, or remove the opaque handle from the async boundary");
        }
    }
}

fn captured_by_async_method(api: &ApiSurface, name: &str, absent: &AHashSet<String>) -> bool {
    api.types
        .iter()
        .filter(|owner| !owner.is_trait && !owner.binding_excluded && !absent.contains(&owner.name))
        .flat_map(|owner| owner.methods.iter().map(move |method| (owner, method)))
        .filter(|(_, method)| method.is_async && !method.sanitized && !method.binding_excluded)
        .any(|(owner, method)| {
            (owner.name == name && !method.is_static && method.receiver.is_some())
                || method
                    .params
                    .iter()
                    .any(|parameter| parameter.ty.references_named(name))
        })
}
