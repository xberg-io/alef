//! Swift plugin marshaling helpers for Box class generation.
//!
//! This module provides type-conversion and marshaling utilities for emitting FFI shim methods
//! in the `Swift{Trait}Box` classes. Each Box class bridges between:
//! - The **FFI layer**: raw `RustString`, `RustVec<UInt8>`, primitive types (FFI types)
//! - The **user-facing bridge protocol**: typed Swift structs (Codable), String, Data, [String], enums, etc.
//!
//! The helpers cover all TypeRef variants that appear in plugin trait methods:
//! user DTOs, primitive values, byte buffers, string collections, and enums.

use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};

/// Returns the Swift FFI type for a Box `alef_*` shim parameter.
///
/// FFI types are what the Rust side passes via `extern "Swift"` shim declarations.
/// They correspond to swift-bridge's native bridging types.
///
/// Examples:
/// - `String` → `"RustString"`
/// - `[UInt8]` / `Bytes` → `"RustVec<UInt8>"`
/// - `Bool` → `"Bool"`
/// - `u32` → `"UInt32"`
/// - `Vec<String>` → `"RustVec<RustString>"`
/// - Named (Codable struct) → `"RustString"` (JSON-encoded)
/// - enum → `"RustString"` (JSON-encoded)
pub fn swift_shim_param_ffi_type(ty: &TypeRef, optional: bool) -> String {
    use crate::core::ir::PrimitiveType;
    let inner = match ty {
        TypeRef::String | TypeRef::Named(_) | TypeRef::Path | TypeRef::Json | TypeRef::Map(_, _) => {
            "RustString".to_string()
        }
        TypeRef::Optional(inner) => return format!("{}?", swift_shim_param_ffi_type(inner, false)),
        TypeRef::Vec(inner) => format!("RustVec<{}>", swift_shim_param_ffi_type(inner, false)),
        TypeRef::Primitive(PrimitiveType::Usize) | TypeRef::Primitive(PrimitiveType::Isize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Bytes => "RustVec<UInt8>".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Duration => "Double".to_string(),
        TypeRef::Unit => "Void".to_string(),
    };
    if optional { format!("{inner}?") } else { inner }
}

/// Returns the Swift expression that converts an FFI parameter to the typed value
/// the bridge protocol method expects.
///
/// For simple types (String, Bool, primitives), this is a direct conversion or passthrough.
/// For complex types (Codable structs, enums, `Vec<String>`), this involves JSON decoding.
///
/// Returns `ParamDecode` with:
/// - `setup`: Vec of setup lines to emit before the bridge call (e.g., `let cfg = try JSONDecoder...`)
/// - `expr`: The expression to pass as the bridge argument
/// - `is_throwing`: Whether the decode itself can throw (wrapped in try/catch at call site)
pub fn swift_shim_param_decode(
    param_name: &str,
    ty: &TypeRef,
    _optional: bool,
    excluded_types: &std::collections::HashSet<String>,
) -> ParamDecode {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => ParamDecode {
            setup: vec![],
            expr: param_name.to_string(),
            is_throwing: false,
        },
        TypeRef::Primitive(PrimitiveType::U32)
        | TypeRef::Primitive(PrimitiveType::U64)
        | TypeRef::Primitive(PrimitiveType::I32)
        | TypeRef::Primitive(PrimitiveType::I64)
        | TypeRef::Primitive(PrimitiveType::U8)
        | TypeRef::Primitive(PrimitiveType::I8)
        | TypeRef::Primitive(PrimitiveType::U16)
        | TypeRef::Primitive(PrimitiveType::I16)
        | TypeRef::Primitive(PrimitiveType::Usize)
        | TypeRef::Primitive(PrimitiveType::Isize)
        | TypeRef::Primitive(PrimitiveType::F32)
        | TypeRef::Primitive(PrimitiveType::F64) => ParamDecode {
            setup: vec![],
            expr: param_name.to_string(),
            is_throwing: false,
        },
        TypeRef::String => ParamDecode {
            setup: vec![],
            expr: format!("{}.toString()", param_name),
            is_throwing: false,
        },
        TypeRef::Bytes => ParamDecode {
            setup: vec![],
            expr: format!("Data({})", param_name),
            is_throwing: false,
        },
        TypeRef::Vec(inner_ty) => {
            if matches!(inner_ty.as_ref(), TypeRef::String) {
                ParamDecode {
                    setup: vec![format!(
                        "var {}_list: [String] = []\n\
                         let {}_count = {}.len()\n\
                         var {}_idx: UInt = 0\n\
                         while {}_idx < {}_count {{\n\
                         \x20   {}_list.append({}.get(index: {}_idx)!.as_str().toString())\n\
                         \x20   {}_idx += 1\n\
                         }}",
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name
                    )],
                    expr: format!("{}_list", param_name),
                    is_throwing: false,
                }
            } else {
                ParamDecode {
                    setup: vec![],
                    expr: format!("{}.toString()", param_name),
                    is_throwing: false,
                }
            }
        }
        TypeRef::Named(type_name) => {
            if excluded_types.contains(type_name) {
                ParamDecode {
                    setup: vec![],
                    expr: format!("{}.toString()", param_name),
                    is_throwing: false,
                }
            } else {
                let setup = format!(
                    "let {}_decoded = try JSONDecoder().decode({}.self, from: Data({}.toString().utf8))",
                    param_name, type_name, param_name
                );
                ParamDecode {
                    setup: vec![setup],
                    expr: format!("{}_decoded", param_name),
                    is_throwing: true,
                }
            }
        }
        TypeRef::Char => ParamDecode {
            setup: vec![],
            expr: format!("Character({}.toString().first ?? \" \")", param_name),
            is_throwing: false,
        },
        TypeRef::Duration => ParamDecode {
            setup: vec![],
            expr: param_name.to_string(),
            is_throwing: false,
        },
        TypeRef::Unit => ParamDecode {
            setup: vec![],
            expr: "()".to_string(),
            is_throwing: false,
        },
        TypeRef::Optional(inner) => {
            let inner_decode = swift_shim_param_decode(param_name, inner, false, excluded_types);
            if inner_decode.is_throwing {
                let try_expr = format!("try? {}", inner_decode.expr);
                ParamDecode {
                    setup: inner_decode.setup,
                    expr: try_expr,
                    is_throwing: false,
                }
            } else if inner_decode.expr.ends_with("()") {
                let expr = format!(
                    "{}?.{}",
                    param_name,
                    &inner_decode.expr[format!("{}.", param_name).len()..]
                );
                ParamDecode {
                    setup: inner_decode.setup,
                    expr,
                    is_throwing: false,
                }
            } else {
                ParamDecode {
                    setup: inner_decode.setup,
                    expr: inner_decode.expr,
                    is_throwing: false,
                }
            }
        }
        TypeRef::Path => ParamDecode {
            setup: vec![],
            expr: format!("URL(fileURLWithPath: {}.toString())", param_name),
            is_throwing: false,
        },
        TypeRef::Json | TypeRef::Map(_, _) => ParamDecode {
            setup: vec![],
            expr: format!("{}.toString()", param_name),
            is_throwing: false,
        },
    }
}

/// Result of parameter decode that can be passed to a bridge method.
pub struct ParamDecode {
    /// Lines to emit before the bridge call (declarations, JSON decode, etc.)
    pub setup: Vec<String>,
    /// The expression to pass as the bridge argument.
    pub expr: String,
    /// Whether this decode sequence can throw (requires try/catch wrapping).
    pub is_throwing: bool,
}

/// Returns the Swift FFI return type for the Box `alef_*` shim.
///
/// Rules:
/// - If method has an error_type (throws): always `"String"` (JSON envelope).
/// - If method returns Unit and no error: `"Void"`.
/// - If method returns Bool and no error: `"Bool"`.
/// - If method returns primitive int and no error: the mapped type (UInt32, Int64, etc.).
/// - If method returns `Vec<String>` and no error: `"RustVec<RustString>"`.
/// - If method returns [other complex] and no error: `"RustString"` (envelope).
pub fn swift_shim_return_ffi_type(method: &MethodDef) -> String {
    if method.error_type.is_some() {
        return "String".to_string();
    }

    match &method.return_type {
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => "RustVec<RustString>".to_string(),
        _ => "RustString".to_string(),
    }
}

/// Returns the Swift body lines that wrap the bridge call result into the FFI return type.
///
/// Handles:
/// - Throwing methods returning Unit: encode `{"ok":null}` on success, `{"err": "..."}` on error
/// - Throwing methods returning T: encode `{"ok": <T>}` / `{"err": "..."}`
/// - Non-throwing methods: passthrough the result or build RustVec for `Vec<String>`
/// - String return types are wrapped in RustString for FFI boundary
///
/// The `bridge_call_expr` is the expression that calls the inner bridge method
/// (e.g., `bridge.processImage(imageBytes: imageBytes, config: config)`).
///
/// Returns lines to emit as the method body (from opening brace to closing brace).
pub fn swift_shim_return_marshal(method: &MethodDef, bridge_call_expr: &str) -> Vec<String> {
    if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => vec![
                "do {".to_string(),
                format!("  try {}", bridge_call_expr),
                "  return encodeOkVoidEnvelope()".to_string(),
                "} catch { return encodeErrEnvelope(\"\\(error)\") }".to_string(),
            ],
            _ => {
                vec![
                    "do {".to_string(),
                    format!("  let result = try {}", bridge_call_expr),
                    "  return encodeOkEnvelope(result)".to_string(),
                    "} catch { return encodeErrEnvelope(\"\\(error)\") }".to_string(),
                ]
            }
        }
    } else {
        match &method.return_type {
            TypeRef::Unit => vec!["return ()".to_string()],
            TypeRef::String => {
                vec![format!("return RustString({})", bridge_call_expr)]
            }
            TypeRef::Named(_) => vec![format!("return RustString({})", bridge_call_expr)],
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                vec![
                    format!("let strings = {}", bridge_call_expr),
                    "let vec = RustVec<RustString>()".to_string(),
                    "for s in strings { vec.push(value: RustString(s)) }".to_string(),
                    "return vec".to_string(),
                ]
            }
            TypeRef::Primitive(PrimitiveType::Usize) | TypeRef::Primitive(PrimitiveType::Isize) => {
                vec![format!("return UInt({})", bridge_call_expr)]
            }
            _ => vec![format!("return {}", bridge_call_expr)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_method(
        name: &str,
        params: Vec<(String, TypeRef, bool)>,
        return_type: TypeRef,
        error_type: Option<String>,
    ) -> MethodDef {
        use crate::core::ir::ParamDef;
        MethodDef {
            name: name.to_string(),
            params: params
                .into_iter()
                .map(|(name, ty, optional)| ParamDef {
                    name,
                    ty,
                    optional,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                })
                .collect(),
            return_type,
            error_type,
            is_async: false,
            is_static: false,
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn test_shim_param_ffi_type_string() {
        assert_eq!(swift_shim_param_ffi_type(&TypeRef::String, false), "RustString");
    }

    #[test]
    fn test_shim_param_ffi_type_bytes() {
        assert_eq!(swift_shim_param_ffi_type(&TypeRef::Bytes, false), "RustVec<UInt8>");
    }

    #[test]
    fn test_shim_param_ffi_type_bool() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Primitive(PrimitiveType::Bool), false),
            "Bool"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_u32() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Primitive(PrimitiveType::U32), false),
            "UInt32"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_vec_string() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Vec(Box::new(TypeRef::String)), false),
            "RustVec<RustString>"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_optional_string() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Optional(Box::new(TypeRef::String)), false),
            "RustString?"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_named() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Named("ParseConfig".to_string()), false),
            "RustString"
        );
    }

    #[test]
    fn test_param_decode_string() {
        let decode = swift_shim_param_decode("config", &TypeRef::String, false, &std::collections::HashSet::new());
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "config.toString()");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_bytes() {
        let decode = swift_shim_param_decode("image_bytes", &TypeRef::Bytes, false, &std::collections::HashSet::new());
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "Data(image_bytes)");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_bool() {
        let decode = swift_shim_param_decode(
            "flag",
            &TypeRef::Primitive(PrimitiveType::Bool),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "flag");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_u32() {
        let decode = swift_shim_param_decode(
            "count",
            &TypeRef::Primitive(PrimitiveType::U32),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "count");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_vec_string() {
        let decode = swift_shim_param_decode(
            "langs",
            &TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(!decode.setup.is_empty());
        assert!(decode.setup[0].contains("langs_list"));
        assert_eq!(decode.expr, "langs_list");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_named_codable() {
        let decode = swift_shim_param_decode(
            "cfg",
            &TypeRef::Named("ParseConfig".to_string()),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(!decode.setup.is_empty());
        assert!(decode.setup[0].contains("JSONDecoder"));
        assert!(decode.setup[0].contains("ParseConfig"));
        assert_eq!(decode.expr, "cfg_decoded");
        assert!(decode.is_throwing);
    }

    #[test]
    fn test_param_decode_optional_string() {
        let decode = swift_shim_param_decode(
            "opt_str",
            &TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "opt_str?.toString()");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_optional_named() {
        let decode = swift_shim_param_decode(
            "opt_cfg",
            &TypeRef::Optional(Box::new(TypeRef::Named("Config".to_string()))),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(!decode.setup.is_empty());
        assert!(decode.expr.contains("try?"));
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_return_ffi_type_throwing_unit() {
        let method = make_method("initialize", vec![], TypeRef::Unit, Some("Error".to_string()));
        assert_eq!(swift_shim_return_ffi_type(&method), "String");
    }

    #[test]
    fn test_return_ffi_type_throwing_string() {
        let method = make_method("process", vec![], TypeRef::String, Some("Error".to_string()));
        assert_eq!(swift_shim_return_ffi_type(&method), "String");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_unit() {
        let method = make_method("get_value", vec![], TypeRef::Unit, None);
        assert_eq!(swift_shim_return_ffi_type(&method), "Void");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_bool() {
        let method = make_method("supports_lang", vec![], TypeRef::Primitive(PrimitiveType::Bool), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "Bool");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_u64() {
        let method = make_method("get_size", vec![], TypeRef::Primitive(PrimitiveType::U64), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "UInt64");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_vec_string() {
        let method = make_method("languages", vec![], TypeRef::Vec(Box::new(TypeRef::String)), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "RustVec<RustString>");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_named() {
        let method = make_method("process", vec![], TypeRef::Named("ParseResult".to_string()), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "RustString");
    }

    #[test]
    fn test_return_marshal_throwing_unit() {
        let method = make_method("initialize", vec![], TypeRef::Unit, Some("Error".to_string()));
        let lines = swift_shim_return_marshal(&method, "try inner.initialize()");
        assert_eq!(lines[0], "do {");
        assert!(lines.join("\n").contains("encodeOkVoidEnvelope"));
        assert!(lines.join("\n").contains("encodeErrEnvelope"));
    }

    #[test]
    fn test_return_marshal_throwing_string() {
        let method = make_method("process", vec![], TypeRef::String, Some("Error".to_string()));
        let lines = swift_shim_return_marshal(&method, "try inner.process()");
        assert_eq!(lines[0], "do {");
        assert!(lines.join("\n").contains("encodeOkEnvelope"));
        assert!(lines.join("\n").contains("encodeErrEnvelope"));
    }

    #[test]
    fn test_return_marshal_non_throwing_unit() {
        let method = make_method("get_value", vec![], TypeRef::Unit, None);
        let lines = swift_shim_return_marshal(&method, "inner.getValue()");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "return ()");
    }

    #[test]
    fn test_return_marshal_non_throwing_bool() {
        let method = make_method("supports_lang", vec![], TypeRef::Primitive(PrimitiveType::Bool), None);
        let lines = swift_shim_return_marshal(&method, "inner.supportsLanguage(lang)");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("return"));
    }

    #[test]
    fn test_return_marshal_non_throwing_vec_string() {
        let method = make_method("languages", vec![], TypeRef::Vec(Box::new(TypeRef::String)), None);
        let lines = swift_shim_return_marshal(&method, "inner.languages()");
        assert!(lines.join("\n").contains("RustVec<RustString>"));
        assert!(lines.join("\n").contains("vec.push"));
    }

    #[test]
    fn test_param_decode_path_url() {
        let decode = swift_shim_param_decode("path", &TypeRef::Path, false, &std::collections::HashSet::new());
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "URL(fileURLWithPath: path.toString())");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_return_marshal_non_throwing_named() {
        let method = make_method(
            "backend_type",
            vec![],
            TypeRef::Named("TextBackendType".to_string()),
            None,
        );
        let lines = swift_shim_return_marshal(&method, "bridge.backendType()");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "return RustString(bridge.backendType())");
    }

    #[test]
    fn test_return_marshal_non_throwing_usize() {
        let method = make_method("dimensions", vec![], TypeRef::Primitive(PrimitiveType::Usize), None);
        let lines = swift_shim_return_marshal(&method, "bridge.dimensions()");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "return UInt(bridge.dimensions())");
    }

    #[test]
    fn test_return_marshal_vec_vec_f32_with_error() {
        let method = make_method(
            "embed",
            vec![],
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32))))),
            Some("Error".to_string()),
        );
        let lines = swift_shim_return_marshal(&method, "try inner.embed(texts)");
        assert!(lines.join("\n").contains("encodeOkEnvelope"));
    }
}
