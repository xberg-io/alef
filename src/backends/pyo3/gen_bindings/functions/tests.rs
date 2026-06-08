mod tests {
    use super::super::{classify_param_type, emit_param_conversion};
    use crate::core::ir::TypeRef;

    /// classify_param_type returns Plain for a bare Named type.
    #[test]
    fn classify_param_type_returns_plain_for_named() {
        let ty = TypeRef::Named("Foo".to_string());
        let result = classify_param_type(&ty);
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "Foo");
    }

    /// classify_param_type returns None for a primitive type.
    #[test]
    fn classify_param_type_returns_none_for_primitive() {
        let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool);
        assert!(classify_param_type(&ty).is_none());
    }

    /// emit_param_conversion emits a guarded None check when optional.
    #[test]
    fn emit_param_conversion_guards_optional() {
        let mut out = String::new();
        emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", true);
        assert!(out.contains("if x is not None else None"));
    }

    /// emit_param_conversion emits a direct assignment when not optional.
    #[test]
    fn emit_param_conversion_direct_when_required() {
        let mut out = String::new();
        emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", false);
        assert!(!out.contains("if x is not None"));
        assert!(out.contains("_rust_x = convert(x)"));
    }
}
