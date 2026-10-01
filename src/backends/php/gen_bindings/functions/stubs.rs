use crate::core::ir::TypeRef;
use minijinja::context;

/// Generate a safe stub return expression for a sanitized function that cannot be auto-delegated.
///
/// When `has_error` is true the function wraps its return in `PhpResult<T>`, so we emit
/// `Err(PhpException::from_message(...))`. When `has_error` is false and the return type is
/// `TypeRef::Unit`, `()` is the only correct value. Every other `has_error: false` case has
/// no safe fabricated value, so we emit `compile_error!` and fail the generated crate's build
/// instead of silently shipping fake data.
pub(super) fn gen_stub_return(ty: &TypeRef, has_error: bool, func_name: &str) -> String {
    if has_error {
        return crate::backends::php::template_env::render(
            "php_stub_error_body.jinja",
            context! {
                func_name => func_name,
            },
        );
    }

    match ty {
        TypeRef::Unit => "()".to_string(),
        _ => crate::backends::php::template_env::render(
            "php_stub_unsupported_return.jinja",
            context! {
                func_name => func_name,
            },
        ),
    }
}

#[cfg(test)]
mod gen_stub_return_tests {
    use super::gen_stub_return;
    use crate::core::ir::TypeRef;

    #[test]
    fn gen_stub_return_string_return_fails_loudly() {
        let stub = gen_stub_return(&TypeRef::String, false, "process");
        assert!(stub.contains("compile_error!"), "expected compile_error!, got: {stub}");
        assert!(
            !stub.contains("String::new()"),
            "fabricated value leaked through: {stub}"
        );
    }

    #[test]
    fn gen_stub_return_unit_return_stays_void() {
        let stub = gen_stub_return(&TypeRef::Unit, false, "process");
        assert_eq!(stub, "()");
    }

    #[test]
    fn gen_stub_return_with_error_type_raises_runtime_error() {
        let stub = gen_stub_return(&TypeRef::String, true, "process");
        assert!(
            !stub.contains("compile_error!"),
            "error branch must not emit compile_error!: {stub}"
        );
        assert!(
            stub.contains("PhpException"),
            "expected PHP exception raise, got: {stub}"
        );
    }
}
