use crate::core::ir::TypeRef;
use minijinja::context;

/// Generate a safe stub return expression for a sanitized function that cannot be auto-delegated.
///
/// When `has_error` is true the function wraps its return in `PhpResult<T>`, so we emit
/// `Err(PhpException::default(...))`. When `has_error` is false the function returns `T`
/// directly, so we emit a type-appropriate default value instead.
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
        TypeRef::Optional(_) => "None".to_string(),
        TypeRef::Vec(_) => "Vec::new()".to_string(),
        TypeRef::String => "String::new()".to_string(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "false".to_string(),
                PrimitiveType::F32 | PrimitiveType::F64 => "0.0".to_string(),
                _ => "0".to_string(),
            }
        }
        TypeRef::Map(_, _) => "Default::default()".to_string(),
        _ => "Default::default()".to_string(),
    }
}
