use alef_core::ir::{PrimitiveType, TypeRef};
use std::cell::RefCell;

thread_local! {
    /// Thread-local storage for Result type alias error hints.
    /// Maps alias name (e.g., "Result") to the error type (e.g., "KreuzbergError").
    static RESULT_ERROR_HINTS: RefCell<ahash::AHashMap<String, String>> = RefCell::new(ahash::AHashMap::new());
}

/// Set the Result error hints for the current extraction context.
pub fn set_result_error_hints(hints: ahash::AHashMap<String, String>) {
    RESULT_ERROR_HINTS.with(|h| {
        *h.borrow_mut() = hints;
    });
}

/// Get the error type hint for a Result type alias.
fn get_result_error_hint(name: &str) -> Option<String> {
    RESULT_ERROR_HINTS.with(|h| h.borrow().get(name).cloned())
}

/// Convert a `syn::Type` into our IR `TypeRef`.
pub fn resolve_type(ty: &syn::Type) -> TypeRef {
    match ty {
        syn::Type::Path(type_path) => resolve_path_type(type_path),
        syn::Type::Reference(type_ref) => resolve_reference_type(type_ref),
        syn::Type::Tuple(tuple) => {
            if tuple.elems.is_empty() {
                TypeRef::Unit
            } else {
                let parts: Vec<String> = tuple.elems.iter().map(type_to_string).collect();
                TypeRef::Named(format!("({})", parts.join(", ")))
            }
        }
        syn::Type::Slice(slice) => resolve_slice_type(&slice.elem),
        // dyn Trait → Named(TraitName), trait objects are opaque
        syn::Type::TraitObject(trait_obj) => {
            if let Some(syn::TypeParamBound::Trait(trait_bound)) = trait_obj.bounds.first() {
                if let Some(seg) = trait_bound.path.segments.last() {
                    return TypeRef::Named(seg.ident.to_string());
                }
            }
            TypeRef::Named("DynObject".to_string())
        }
        // impl Trait → resolve generic if Into<T> or AsRef<T>, otherwise Named(TraitName)
        syn::Type::ImplTrait(impl_trait) => {
            if let Some(syn::TypeParamBound::Trait(trait_bound)) = impl_trait.bounds.first() {
                if let Some(seg) = trait_bound.path.segments.last() {
                    let trait_name = seg.ident.to_string();
                    if trait_name == "Into" || trait_name == "AsRef" {
                        if let Some(inner_ty) = extract_single_generic_arg(seg) {
                            return inner_ty;
                        }
                    }
                    return TypeRef::Named(trait_name);
                }
            }
            TypeRef::Named("ImplTrait".to_string())
        }
        _ => TypeRef::Named(type_to_string(ty)),
    }
}

/// Convert a syn::Type to its string representation.
///
/// Strips cosmetic whitespace that `quote` adds around punctuation, while preserving
/// the space between a lifetime (e.g. `'static`) and the type token that follows it.
/// Without that preservation, `&'static str` would be rendered as `&'staticstr`.
pub fn type_to_string(ty: &syn::Type) -> String {
    use quote::ToTokens;
    let raw = ty.to_token_stream().to_string();
    normalize_type_string(&raw)
}

/// Remove cosmetic spaces added by `quote` around punctuation, but keep the space
/// that separates a lifetime token from the type or bracket that follows it.
///
/// Examples:
/// - `& 'static str`      → `&'static str`
/// - `& 'static [ & 'static str ]` → `&'static [&'static str]`
/// - `Vec < String >`     → `Vec<String>`
fn normalize_type_string(s: &str) -> String {
    let is_punct = |c: char| matches!(c, '<' | '>' | '[' | ']' | '(' | ')' | ',' | '*' | '&' | ':');

    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if c == ' ' {
            let prev_is_punct = out.chars().last().map(is_punct).unwrap_or(false);
            let next_is_punct = chars[i + 1..]
                .iter()
                .find(|&&ch| ch != ' ')
                .copied()
                .map(is_punct)
                .unwrap_or(false);
            // Keep the space when the preceding token is a lifetime — stripping it would
            // concatenate the lifetime with the following type token (e.g. `'staticstr`).
            let prev_ends_lifetime = ends_with_lifetime(&out);
            if (prev_is_punct || next_is_punct) && !prev_ends_lifetime {
                // Drop cosmetic space around punctuation.
            } else {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Returns `true` if `s` ends with a lifetime token such as `'static` or `'a`.
fn ends_with_lifetime(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    // Walk back over the identifier characters of the lifetime name.
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    // The character before the identifier must be a tick.
    i > 0 && bytes[i - 1] == b'\''
}

/// Resolve a path-based type like `String`, `Vec<T>`, `Option<T>`, etc.
fn resolve_path_type(type_path: &syn::TypePath) -> TypeRef {
    // Get the last segment (handles both `std::path::PathBuf` and just `PathBuf`)
    let segment = match type_path.path.segments.last() {
        Some(seg) => seg,
        None => return TypeRef::Named(String::new()),
    };

    let ident = segment.ident.to_string();

    // Check for qualified paths like `serde_json::Value`
    if type_path.path.segments.len() >= 2 {
        let full_path: String = type_path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if full_path == "serde_json::Value" {
            return TypeRef::Json;
        }
    }

    match ident.as_str() {
        // Primitives
        "bool" => TypeRef::Primitive(PrimitiveType::Bool),
        "u8" => TypeRef::Primitive(PrimitiveType::U8),
        "u16" => TypeRef::Primitive(PrimitiveType::U16),
        "u32" => TypeRef::Primitive(PrimitiveType::U32),
        "u64" => TypeRef::Primitive(PrimitiveType::U64),
        "i8" => TypeRef::Primitive(PrimitiveType::I8),
        "i16" => TypeRef::Primitive(PrimitiveType::I16),
        "i32" => TypeRef::Primitive(PrimitiveType::I32),
        "i64" => TypeRef::Primitive(PrimitiveType::I64),
        "f32" => TypeRef::Primitive(PrimitiveType::F32),
        "f64" => TypeRef::Primitive(PrimitiveType::F64),
        "usize" => TypeRef::Primitive(PrimitiveType::Usize),
        "isize" => TypeRef::Primitive(PrimitiveType::Isize),

        // String types
        "String" => TypeRef::String,
        "char" => TypeRef::Char,

        // Path types
        "PathBuf" => TypeRef::Path,

        // Bytes
        "Bytes" => TypeRef::Bytes,

        // JSON
        "JsonValue" => TypeRef::Json,

        // Vec<T>
        "Vec" => {
            let inner = extract_single_generic_arg(segment);
            match inner {
                Some(inner_ty) => {
                    // Vec<u8> → Bytes
                    if matches!(inner_ty, TypeRef::Primitive(PrimitiveType::U8)) {
                        TypeRef::Bytes
                    } else {
                        TypeRef::Vec(Box::new(inner_ty))
                    }
                }
                None => TypeRef::Vec(Box::new(TypeRef::Named("unknown".into()))),
            }
        }

        // Option<T>
        "Option" => {
            let inner = extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into()));
            TypeRef::Optional(Box::new(inner))
        }

        // HashMap<K, V> / BTreeMap<K, V>
        "HashMap" | "BTreeMap" => {
            let (k, v) = extract_two_generic_args(segment);
            TypeRef::Map(Box::new(k), Box::new(v))
        }

        // Result<T, E> → unwrap to T
        "Result" => extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into())),

        // Box<T>, Arc<T> → unwrap to T
        "Box" | "Arc" | "Rc" => extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into())),

        // Well-known std/common types
        "Duration" => TypeRef::Duration,
        "SecretString" => TypeRef::String,
        "Cow" => {
            // Cow<str> → String, Cow<[u8]> → Bytes, etc.
            extract_single_generic_arg(segment).unwrap_or(TypeRef::String)
        }

        // Any other named type
        other => TypeRef::Named(other.to_string()),
    }
}

/// Resolve a reference type like `&str`, `&Path`, `&[u8]`.
fn resolve_reference_type(type_ref: &syn::TypeReference) -> TypeRef {
    let inner = &*type_ref.elem;
    match inner {
        // &str → String
        syn::Type::Path(p) => {
            if let Some(seg) = p.path.segments.last() {
                match seg.ident.to_string().as_str() {
                    "str" => TypeRef::String,
                    "Path" => TypeRef::Path,
                    _ => resolve_type(inner),
                }
            } else {
                resolve_type(inner)
            }
        }
        // &[u8] → Bytes, &[T] → Vec<T>
        syn::Type::Slice(slice) => resolve_slice_type(&slice.elem),
        _ => resolve_type(inner),
    }
}

/// Resolve a slice type `[T]` — `[u8]` becomes Bytes, otherwise Vec<T>.
fn resolve_slice_type(elem: &syn::Type) -> TypeRef {
    let inner = resolve_type(elem);
    if matches!(inner, TypeRef::Primitive(PrimitiveType::U8)) {
        TypeRef::Bytes
    } else {
        TypeRef::Vec(Box::new(inner))
    }
}

/// Extract the first generic type argument from a path segment, e.g., `Vec<T>` → T.
/// Extract the raw syn::Type of the first generic argument (unresolved).
pub fn extract_single_generic_arg_syn(segment: &syn::PathSegment) -> Option<Box<syn::Type>> {
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(ty) = arg {
                return Some(Box::new(ty.clone()));
            }
        }
    }
    None
}

fn extract_single_generic_arg(segment: &syn::PathSegment) -> Option<TypeRef> {
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(ty) = arg {
                return Some(resolve_type(ty));
            }
        }
    }
    None
}

/// Extract two generic type arguments from a path segment, e.g., `HashMap<K, V>`.
fn extract_two_generic_args(segment: &syn::PathSegment) -> (TypeRef, TypeRef) {
    let mut types = Vec::new();
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(ty) = arg {
                types.push(resolve_type(ty));
            }
        }
    }
    let k = types.first().cloned().unwrap_or(TypeRef::Named("unknown".into()));
    let v = types.get(1).cloned().unwrap_or(TypeRef::Named("unknown".into()));
    (k, v)
}

/// Check if a `syn::Type` represents `Option<T>`, and if so return the inner type.
pub fn is_option_type(ty: &syn::Type) -> Option<TypeRef> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Option" {
                return extract_single_generic_arg(segment);
            }
        }
    }
    None
}

/// Extract the error type from a `pub type Result<T> = std::result::Result<T, E>` alias definition.
/// Returns the string representation of the error type E.
pub fn extract_result_error_type_from_alias(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(type_path) = ty {
        // For `std::result::Result<T, E>` or just `Result<T, E>`
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    let type_args: Vec<_> = args
                        .args
                        .iter()
                        .filter_map(|a| {
                            if let syn::GenericArgument::Type(ty) = a {
                                Some(ty)
                            } else {
                                None
                            }
                        })
                        .collect();
                    // For a non-generic type alias, we expect exactly 2 args: T and E
                    if type_args.len() == 2 {
                        return Some(type_to_string(type_args[1]));
                    }
                }
            }
        }
    }
    None
}

/// Extract the error type string from a `Result<T, E>` return type.
pub fn extract_result_error_type(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    let type_args: Vec<_> = args
                        .args
                        .iter()
                        .filter_map(|a| {
                            if let syn::GenericArgument::Type(ty) = a {
                                Some(ty)
                            } else {
                                None
                            }
                        })
                        .collect();
                    if type_args.len() >= 2 {
                        return Some(type_to_string(type_args[1]));
                    }
                    // Result<T> with a single type arg (type alias) — look up the error type hint.
                    if !type_args.is_empty() {
                        if let Some(hint) = get_result_error_hint("Result") {
                            return Some(hint);
                        }
                        // Fallback to anyhow::Error if no hint is available
                        return Some("anyhow::Error".to_string());
                    }
                }
            }
        }
    }
    None
}

/// Check if a return type is `Result<T, E>` and return the inner T type.
pub fn unwrap_result_type(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner_ty) = arg {
                            return Some(inner_ty);
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_type(s: &str) -> syn::Type {
        syn::parse_str(s).unwrap()
    }

    #[test]
    fn test_primitives() {
        assert_eq!(
            resolve_type(&parse_type("bool")),
            TypeRef::Primitive(PrimitiveType::Bool)
        );
        assert_eq!(resolve_type(&parse_type("u32")), TypeRef::Primitive(PrimitiveType::U32));
        assert_eq!(resolve_type(&parse_type("f64")), TypeRef::Primitive(PrimitiveType::F64));
        assert_eq!(
            resolve_type(&parse_type("usize")),
            TypeRef::Primitive(PrimitiveType::Usize)
        );
    }

    #[test]
    fn test_string_types() {
        assert_eq!(resolve_type(&parse_type("String")), TypeRef::String);
        assert_eq!(resolve_type(&parse_type("&str")), TypeRef::String);
    }

    #[test]
    fn test_bytes_types() {
        assert_eq!(resolve_type(&parse_type("Vec<u8>")), TypeRef::Bytes);
        assert_eq!(resolve_type(&parse_type("&[u8]")), TypeRef::Bytes);
        assert_eq!(resolve_type(&parse_type("Bytes")), TypeRef::Bytes);
    }

    #[test]
    fn test_vec() {
        assert_eq!(
            resolve_type(&parse_type("Vec<String>")),
            TypeRef::Vec(Box::new(TypeRef::String))
        );
    }

    #[test]
    fn test_option() {
        assert_eq!(
            resolve_type(&parse_type("Option<u64>")),
            TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64)))
        );
    }

    #[test]
    fn test_map() {
        assert_eq!(
            resolve_type(&parse_type("HashMap<String, u32>")),
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::U32))
            )
        );
    }

    #[test]
    fn test_path_types() {
        assert_eq!(resolve_type(&parse_type("PathBuf")), TypeRef::Path);
        assert_eq!(resolve_type(&parse_type("&Path")), TypeRef::Path);
    }

    #[test]
    fn test_unit() {
        assert_eq!(resolve_type(&parse_type("()")), TypeRef::Unit);
    }

    #[test]
    fn test_json() {
        assert_eq!(resolve_type(&parse_type("serde_json::Value")), TypeRef::Json);
        assert_eq!(resolve_type(&parse_type("JsonValue")), TypeRef::Json);
    }

    #[test]
    fn test_box_arc_unwrap() {
        assert_eq!(resolve_type(&parse_type("Box<String>")), TypeRef::String);
        assert_eq!(
            resolve_type(&parse_type("Arc<u32>")),
            TypeRef::Primitive(PrimitiveType::U32)
        );
    }

    #[test]
    fn test_result_unwrap() {
        assert_eq!(resolve_type(&parse_type("Result<String, Error>")), TypeRef::String);
    }

    #[test]
    fn test_named() {
        assert_eq!(
            resolve_type(&parse_type("MyCustomType")),
            TypeRef::Named("MyCustomType".into())
        );
    }

    #[test]
    fn test_trait_object() {
        assert_eq!(
            resolve_type(&parse_type("dyn MyTrait")),
            TypeRef::Named("MyTrait".into())
        );
    }

    #[test]
    fn test_box_dyn_trait() {
        assert_eq!(
            resolve_type(&parse_type("Box<dyn MyTrait>")),
            TypeRef::Named("MyTrait".into())
        );
    }

    #[test]
    fn test_duration() {
        assert_eq!(resolve_type(&parse_type("Duration")), TypeRef::Duration);
    }

    #[test]
    fn test_secret_string() {
        assert_eq!(resolve_type(&parse_type("SecretString")), TypeRef::String);
    }

    #[test]
    fn test_impl_trait() {
        assert_eq!(resolve_type(&parse_type("impl Into<String>")), TypeRef::String);
    }

    #[test]
    fn test_extract_result_error() {
        let ty = parse_type("Result<String, MyError>");
        assert_eq!(extract_result_error_type(&ty), Some("MyError".into()));
    }

    #[test]
    fn test_extract_result_error_from_alias_definition() {
        // Test extracting error type from `pub type Result<T> = std::result::Result<T, E>`
        let ty = parse_type("std::result::Result<T, KreuzbergError>");
        assert_eq!(extract_result_error_type_from_alias(&ty), Some("KreuzbergError".into()));
    }

    #[test]
    fn test_extract_result_error_with_hint() {
        // Test that when a Result<T> type alias has a registered error hint, it's used
        let hints = {
            let mut m = ahash::AHashMap::new();
            m.insert("Result".to_string(), "KreuzbergError".to_string());
            m
        };
        set_result_error_hints(hints);

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(extract_result_error_type(&ty), Some("KreuzbergError".into()));
    }

    #[test]
    fn test_extract_result_error_fallback_without_hint() {
        // Test that without a hint, it falls back to anyhow::Error
        set_result_error_hints(ahash::AHashMap::new());

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(extract_result_error_type(&ty), Some("anyhow::Error".into()));
    }

    // Regression tests for whitespace between lifetime tokens and following type/bracket.
    // Previously, `type_to_string` called `.replace(' ', "")` which collapsed
    // `&'static str` into `&'staticstr` and `&'static [&'static str]` into
    // `&'static[&'staticstr]`.

    #[test]
    fn test_normalize_type_string_static_str() {
        // quote produces "& 'static str"; must become "&'static str"
        assert_eq!(normalize_type_string("& 'static str"), "&'static str");
    }

    #[test]
    fn test_normalize_type_string_static_slice_of_static_str() {
        // quote produces "& 'static [& 'static str]"; must become "&'static [&'static str]"
        assert_eq!(
            normalize_type_string("& 'static [& 'static str]"),
            "&'static [&'static str]"
        );
    }

    #[test]
    fn test_normalize_type_string_generic_no_spaces() {
        // Cosmetic spaces around angle brackets must be stripped.
        assert_eq!(normalize_type_string("Vec < String >"), "Vec<String>");
    }

    #[test]
    fn test_type_to_string_static_str() {
        let ty = parse_type("&'static str");
        assert_eq!(type_to_string(&ty), "&'static str");
    }

    #[test]
    fn test_type_to_string_static_slice_of_static_str() {
        let ty = parse_type("&'static [&'static str]");
        assert_eq!(type_to_string(&ty), "&'static [&'static str]");
    }
}
