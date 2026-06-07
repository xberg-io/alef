use crate::core::ir::FunctionDef;
use ahash::AHashSet;

/// Whether a free function's return type involves an opaque type that requires the
/// tokio variant of `Mutex` (because every `&mut self` method on that type is async).
pub(super) fn returns_tokio_mutex_type(func: &FunctionDef, tokio_mutex_types: &AHashSet<String>) -> bool {
    use crate::core::ir::TypeRef;
    fn check(ty: &TypeRef, set: &AHashSet<String>) -> bool {
        match ty {
            TypeRef::Named(n) => set.contains(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => check(inner, set),
            _ => false,
        }
    }
    check(&func.return_type, tokio_mutex_types)
}

pub(super) fn rewrite_to_tokio_mutex_struct(struct_code: &str) -> String {
    struct_code.replace("Arc<std::sync::Mutex<", "Arc<tokio::sync::Mutex<")
}

pub(super) fn rewrite_to_tokio_mutex_impl(impl_code: &str) -> String {
    impl_code
        .replace("Arc<std::sync::Mutex<", "Arc<tokio::sync::Mutex<")
        .replace("Arc::new(std::sync::Mutex::new(", "Arc::new(tokio::sync::Mutex::new(")
        .replace(".lock().unwrap()", ".lock().await")
}
