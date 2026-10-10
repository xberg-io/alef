//! Warn when an async pyo3 binding carries a thread-bound opaque type across the runtime
//! boundary (alef #526). ~keep
//!
//! A `#[pyclass]` opaque type is bound to the thread that created it unless the project lists
//! it in `python.send_sync_types`. An async call hands such a value to a Tokio worker, which
//! can release the last reference on that worker; pyo3 then refuses the drop ("is unsendable,
//! but is being dropped on another thread") and leaks the Rust value. The generator cannot make
//! the type sendable on its own — it cannot prove `Send + Sync`, so a bad default becomes a
//! compile error — so it names the option instead.

use ahash::AHashSet;

use crate::core::ir::{ApiSurface, ParamDef, TypeRef};

/// Whether the thread-bound type reached the async call as a parameter or as the method receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThreadBoundRole {
    Parameter,
    Receiver,
}

impl ThreadBoundRole {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            ThreadBoundRole::Parameter => "parameter",
            ThreadBoundRole::Receiver => "receiver",
        }
    }
}

/// One async call that passes a thread-bound opaque type across the runtime boundary.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ThreadBoundAsyncUse {
    pub item: String,
    pub type_name: String,
    pub role: ThreadBoundRole,
}

/// Every async free function or method whose parameter or receiver is an opaque wrapper that is
/// not in `send_sync_types`. An empty result means every async call is safe to schedule.
pub(super) fn thread_bound_async_types(
    api: &ApiSurface,
    opaque_types: &AHashSet<String>,
    send_sync_types: &AHashSet<String>,
) -> Vec<ThreadBoundAsyncUse> {
    let trait_types: AHashSet<&str> = api
        .types
        .iter()
        .filter(|typ| typ.is_trait)
        .map(|typ| typ.name.as_str())
        .collect();
    let thread_bound: AHashSet<&str> = opaque_types
        .iter()
        .map(String::as_str)
        .filter(|name| !send_sync_types.contains(*name) && !trait_types.contains(*name))
        .collect();
    let mut uses = Vec::new();

    for function in &api.functions {
        if !function.is_async || function.sanitized {
            continue;
        }
        collect_parameter_uses(&function.name, &function.params, &thread_bound, &mut uses);
    }

    for typ in &api.types {
        for method in &typ.methods {
            if !method.is_async || method.sanitized {
                continue;
            }
            let item = format!("{}::{}", typ.name, method.name);
            if method.receiver.is_some() && thread_bound.contains(typ.name.as_str()) {
                uses.push(ThreadBoundAsyncUse {
                    item: item.clone(),
                    type_name: typ.name.clone(),
                    role: ThreadBoundRole::Receiver,
                });
            }
            collect_parameter_uses(&item, &method.params, &thread_bound, &mut uses);
        }
    }

    uses
}

fn collect_parameter_uses(
    item: &str,
    params: &[ParamDef],
    thread_bound: &AHashSet<&str>,
    uses: &mut Vec<ThreadBoundAsyncUse>,
) {
    for param in params {
        if let Some(name) = named_type_name(&param.ty)
            && thread_bound.contains(name)
        {
            uses.push(ThreadBoundAsyncUse {
                item: item.to_string(),
                type_name: name.to_string(),
                role: ThreadBoundRole::Parameter,
            });
        }
    }
}

/// The inner named type through `Option`/`Vec`/`Map` wrappers.
fn named_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => named_type_name(inner),
        TypeRef::Map(key, value) => named_type_name(key).or_else(|| named_type_name(value)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ahash::AHashSet;

    use super::{ThreadBoundAsyncUse, ThreadBoundRole, thread_bound_async_types};
    use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

    fn names(items: &[&str]) -> AHashSet<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn opaque_handle() -> TypeDef {
        TypeDef {
            name: "SessionHandle".to_string(),
            rust_path: "sample::SessionHandle".to_string(),
            is_opaque: true,
            methods: vec![
                MethodDef {
                    name: "fetch".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    is_async: true,
                    ..Default::default()
                },
                MethodDef {
                    name: "len".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    is_async: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn async_takes_handle() -> FunctionDef {
        FunctionDef {
            name: "run".to_string(),
            rust_path: "sample::run".to_string(),
            is_async: true,
            params: vec![ParamDef {
                name: "session".to_string(),
                ty: TypeRef::Named("SessionHandle".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn async_trait() -> TypeDef {
        let mut typ = opaque_handle();
        typ.is_trait = true;
        typ
    }

    #[test]
    fn warns_for_a_thread_bound_receiver_and_parameter() {
        let api = ApiSurface {
            types: vec![opaque_handle()],
            functions: vec![async_takes_handle()],
            ..Default::default()
        };

        let uses = thread_bound_async_types(&api, &names(&["SessionHandle"]), &names(&[]));

        assert_eq!(
            uses,
            vec![
                ThreadBoundAsyncUse {
                    item: "run".to_string(),
                    type_name: "SessionHandle".to_string(),
                    role: ThreadBoundRole::Parameter,
                },
                ThreadBoundAsyncUse {
                    item: "SessionHandle::fetch".to_string(),
                    type_name: "SessionHandle".to_string(),
                    role: ThreadBoundRole::Receiver,
                },
            ],
            "both the async free function's parameter and the async method's receiver must warn"
        );
    }

    #[test]
    fn send_sync_types_silences_the_warning() {
        let api = ApiSurface {
            types: vec![opaque_handle()],
            functions: vec![async_takes_handle()],
            ..Default::default()
        };

        let uses = thread_bound_async_types(&api, &names(&["SessionHandle"]), &names(&["SessionHandle"]));

        assert!(
            uses.is_empty(),
            "a type listed in send_sync_types is sendable and must not warn: {uses:?}"
        );
    }

    #[test]
    fn async_trait_receivers_and_parameters_do_not_warn() {
        let api = ApiSurface {
            types: vec![async_trait()],
            functions: vec![async_takes_handle()],
            ..Default::default()
        };

        let uses = thread_bound_async_types(&api, &names(&["SessionHandle"]), &names(&[]));

        assert!(
            uses.is_empty(),
            "trait markers use separately generated sendable bridges and must not warn: {uses:?}"
        );
    }

    #[test]
    fn a_sync_method_on_a_thread_bound_type_does_not_warn() {
        let api = ApiSurface {
            types: vec![opaque_handle()],
            ..Default::default()
        };

        let uses = thread_bound_async_types(&api, &names(&["SessionHandle"]), &names(&[]));

        assert_eq!(
            uses.len(),
            1,
            "only the async method warns; the sync `len` must not: {uses:?}"
        );
        assert_eq!(uses[0].item, "SessionHandle::fetch");
    }
}
