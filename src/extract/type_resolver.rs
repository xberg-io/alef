use crate::core::ir::{PrimitiveType, TypeRef};
use crate::extract::extractor::helpers::result_alias_scope::scope_for_result_type_path;
use std::cell::RefCell;

/// Which `Result` type alias the module currently being extracted resolves `Result` to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultAliasScope {
    /// `Result` names a crate-local alias declared in this module path (`""` = crate root).
    Crate(String),
    /// `Result` names a foreign crate's alias (e.g. `anyhow::Result`), so no crate-local
    /// error type applies.
    Foreign,
}

thread_local! {
    /// Thread-local storage for Result type alias error hints.
    ///
    /// Maps the module path a `Result` alias is *declared* in (`""` = crate root) to the error
    /// type it carries (e.g. `"SampleCrateError"`). Keying by declaring module — rather than by
    /// the alias name — is what keeps a module-private `Result` (a format-specific `error.rs`,
    /// say) from overwriting the crate's canonical alias. ~keep
    static RESULT_ERROR_HINTS: RefCell<ahash::AHashMap<String, String>> = RefCell::new(ahash::AHashMap::new());

    /// The `Result` alias in scope for the module whose items are being extracted right now.
    /// `None` means the module neither declares nor imports one, so lookup falls back to the
    /// crate's canonical alias.
    static RESULT_ALIAS_SCOPE: RefCell<Option<ResultAliasScope>> = const { RefCell::new(None) };
}

/// Drop every Result error hint collected so far.
///
/// Hints accumulate across a crate's modules, so they must be dropped when extraction moves on to
/// the next crate — otherwise a crate with no `Result` alias of its own inherits the previous
/// crate's error type. ~keep
pub fn reset_result_error_hints() {
    RESULT_ERROR_HINTS.with(|h| {
        h.borrow_mut().clear();
    });
    RESULT_ALIAS_SCOPE.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Record the error type of a `Result` alias declared in `module_path`.
///
/// Extraction walks one file at a time, but a crate's `Result` alias is declared in one module
/// (`error.rs`) and used from others (`convert_api.rs`). Replacing the map per file would drop the
/// alias before the functions that return it are resolved, so hints must accumulate. ~keep
pub fn record_result_error_hint(module_path: &str, error_type: String) {
    RESULT_ERROR_HINTS.with(|h| {
        h.borrow_mut().insert(module_path.to_string(), error_type);
    });
}

/// Install `scope` as the `Result` alias in scope, returning the value it replaced.
pub fn set_result_alias_scope(scope: Option<ResultAliasScope>) -> Option<ResultAliasScope> {
    RESULT_ALIAS_SCOPE.with(|s| s.replace(scope))
}

/// Number of path segments in a module path (`""` — the crate root — has none).
fn module_depth(module_path: &str) -> usize {
    if module_path.is_empty() {
        0
    } else {
        module_path.split("::").count()
    }
}

/// The crate's canonical `Result` alias error type: the one declared nearest the crate root.
///
/// A crate that exports `Result` declares it at (or one module below) the root and re-exports it
/// from `lib.rs`; aliases buried deeper are private to a subsystem and are never the type the
/// crate's public API returns. Ties break lexicographically so codegen stays deterministic. ~keep
fn canonical_result_error_hint() -> Option<String> {
    RESULT_ERROR_HINTS.with(|hints| {
        hints
            .borrow()
            .iter()
            .min_by(|(left, _), (right, _)| {
                module_depth(left.as_str())
                    .cmp(&module_depth(right.as_str()))
                    .then_with(|| left.cmp(right))
            })
            .map(|(_, error_type)| error_type.clone())
    })
}

/// Get the error type hint for the `Result` alias in scope for the current module.
fn get_result_error_hint() -> Option<String> {
    hint_for_scope(RESULT_ALIAS_SCOPE.with(|s| s.borrow().clone()))
}

/// Get the error type hint carried by a specific `Result` alias scope.
fn hint_for_scope(scope: Option<ResultAliasScope>) -> Option<String> {
    match scope {
        Some(ResultAliasScope::Foreign) => None,
        // The declaring module may not have been walked yet (module order is source order), so an
        // unresolved crate-local alias still falls back to the canonical one. ~keep
        Some(ResultAliasScope::Crate(module_path)) => RESULT_ERROR_HINTS
            .with(|h| h.borrow().get(&module_path).cloned())
            .or_else(canonical_result_error_hint),
        None => canonical_result_error_hint(),
    }
}

/// Restores the enclosing `Result` alias scope when extraction leaves a module.
pub struct ResultAliasScopeGuard(Option<ResultAliasScope>);

impl ResultAliasScopeGuard {
    /// Enter `scope`, remembering the scope it replaced.
    pub fn enter(scope: Option<ResultAliasScope>) -> Self {
        Self(set_result_alias_scope(scope))
    }
}

impl Drop for ResultAliasScopeGuard {
    fn drop(&mut self) {
        set_result_alias_scope(self.0.take());
    }
}

/// Isolates the `Result` alias hints collected so far for the duration of a foreign-crate walk.
///
/// Re-exported items from a workspace sibling are extracted inline, and that sibling's own
/// `Result` alias must neither be resolved against the host crate's hints nor leak back into
/// them once the walk finishes. ~keep
pub struct IsolatedResultHintsGuard {
    hints: ahash::AHashMap<String, String>,
    scope: Option<ResultAliasScope>,
}

impl IsolatedResultHintsGuard {
    /// Swap in an empty hint set, remembering the current one.
    pub fn enter() -> Self {
        let hints = RESULT_ERROR_HINTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
        let scope = set_result_alias_scope(None);
        Self { hints, scope }
    }
}

impl Drop for IsolatedResultHintsGuard {
    fn drop(&mut self) {
        RESULT_ERROR_HINTS.with(|h| {
            *h.borrow_mut() = std::mem::take(&mut self.hints);
        });
        set_result_alias_scope(self.scope.take());
    }
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
        syn::Type::Array(array) => resolve_array_type(array, ty),
        syn::Type::TraitObject(trait_obj) => {
            if let Some(syn::TypeParamBound::Trait(trait_bound)) = trait_obj.bounds.first()
                && let Some(seg) = trait_bound.path.segments.last()
            {
                return TypeRef::Named(seg.ident.to_string());
            }
            TypeRef::Named("DynObject".to_string())
        }
        syn::Type::ImplTrait(impl_trait) => {
            if let Some(syn::TypeParamBound::Trait(trait_bound)) = impl_trait.bounds.first()
                && let Some(seg) = trait_bound.path.segments.last()
            {
                let trait_name = seg.ident.to_string();
                if (trait_name == "Into" || trait_name == "AsRef")
                    && let Some(inner_ty) = extract_single_generic_arg(seg)
                {
                    return inner_ty;
                }
                return TypeRef::Named(trait_name);
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
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n);
    let is_punct = |b: u8| matches!(b, b'<' | b'>' | b'[' | b']' | b'(' | b')' | b',' | b'*' | b'&' | b':');

    let mut i = 0;
    while i < n {
        let c = bytes[i];
        if c == b' ' {
            let prev_is_punct = out.as_bytes().last().copied().map(is_punct).unwrap_or(false);
            let mut j = i + 1;
            while j < n && bytes[j] == b' ' {
                j += 1;
            }
            let next_is_punct = j < n && is_punct(bytes[j]);
            let prev_ends_lifetime = ends_with_lifetime(&out);
            if (prev_is_punct || next_is_punct) && !prev_ends_lifetime {
            } else {
                out.push(' ');
            }
        } else if c.is_ascii() {
            out.push(c as char);
        } else {
            let mut j = i + 1;
            while j < n && (bytes[j] & 0b1100_0000) == 0b1000_0000 {
                j += 1;
            }
            if let Ok(slice) = std::str::from_utf8(&bytes[i..j]) {
                out.push_str(slice);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// Returns `true` if `s` ends with a lifetime token such as `'static` or `'a`.
fn ends_with_lifetime(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    i > 0 && bytes[i - 1] == b'\''
}

/// Resolve a path-based type like `String`, `Vec<T>`, `Option<T>`, etc.
fn resolve_path_type(type_path: &syn::TypePath) -> TypeRef {
    let segment = match type_path.path.segments.last() {
        Some(seg) => seg,
        None => return TypeRef::Named(String::new()),
    };

    let ident = segment.ident.to_string();

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

        "String" | "str" => TypeRef::String,
        "char" => TypeRef::Char,

        "PathBuf" | "Path" => TypeRef::Path,

        "Bytes" => TypeRef::Bytes,

        "JsonValue" | "Value" => TypeRef::Named(ident),

        "Vec" => {
            let inner = extract_single_generic_arg(segment);
            match inner {
                Some(inner_ty) => {
                    if matches!(inner_ty, TypeRef::Primitive(PrimitiveType::U8)) {
                        TypeRef::Bytes
                    } else {
                        TypeRef::Vec(Box::new(inner_ty))
                    }
                }
                None => TypeRef::Vec(Box::new(TypeRef::Named("unknown".into()))),
            }
        }

        "Option" => {
            let inner = extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into()));
            TypeRef::Optional(Box::new(inner))
        }

        "HashMap" | "BTreeMap" | "AHashMap" | "IndexMap" | "FxHashMap" => {
            let (k, v) = extract_two_generic_args(segment);
            TypeRef::Map(Box::new(k), Box::new(v))
        }

        "HashSet" | "BTreeSet" | "AHashSet" | "IndexSet" | "FxHashSet" => {
            let inner = extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into()));
            TypeRef::Vec(Box::new(inner))
        }

        "Result" => extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into())),

        "Box" | "Arc" | "Rc" | "Mutex" | "RwLock" => {
            extract_single_generic_arg(segment).unwrap_or(TypeRef::Named("unknown".into()))
        }

        "Duration" => TypeRef::Duration,
        "Cow" => extract_single_generic_arg(segment).unwrap_or(TypeRef::String),

        other => TypeRef::Named(other.to_string()),
    }
}

/// Resolve a reference type like `&str`, `&Path`, `&[u8]`.
fn resolve_reference_type(type_ref: &syn::TypeReference) -> TypeRef {
    let inner = &*type_ref.elem;
    match inner {
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

/// Resolve a fixed-size array type `[T; N]`.
///
/// `[u8; N]` mirrors the existing `Vec<u8>` / `&[u8]` special case in [`resolve_slice_type`]: a
/// fixed-length byte buffer is exactly what `Bytes` represents, and the declared length is not
/// information the binding layer needs to keep. Other primitive element types (`[f64; 3]`,
/// `[u32; 4]`, ...) have no fixed-length list counterpart in the binding layer either, so they
/// lower losslessly to `Vec<T>` -- the declared length is the one fact this drops, the same
/// trade-off `sanitize_unknown_types` already makes for a fixed-size array of a known struct or
/// enum type. A non-primitive element (a named struct/enum, tuple, or unsupported shape) still
/// falls back to the stringified whole-array `TypeRef::Named`, so the sanitizer can recognize a
/// known type name and lower it there, or fall back to a lossy placeholder for anything it
/// cannot. ~keep
fn resolve_array_type(array: &syn::TypeArray, whole: &syn::Type) -> TypeRef {
    match resolve_type(&array.elem) {
        TypeRef::Primitive(PrimitiveType::U8) => TypeRef::Bytes,
        element @ TypeRef::Primitive(_) => TypeRef::Vec(Box::new(element)),
        _ => TypeRef::Named(type_to_string(whole)),
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
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Option"
    {
        return extract_single_generic_arg(segment);
    }
    None
}

/// Extract the error type from a `pub type Result<T> = std::result::Result<T, E>` alias definition.
/// Returns the string representation of the error type E.
///
/// `generics` are the alias's own parameters: an alias may be generic over its *error* parameter
/// (`type Result<T, E = MyError>`), in which case the right-hand side names the parameter and the
/// concrete type lives in the parameter's default.
pub fn extract_result_error_type_from_alias(ty: &syn::Type, generics: &syn::Generics) -> Option<String> {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
    {
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
        if type_args.len() == 2 {
            return resolve_alias_error_parameter(&type_to_string(type_args[1]), generics);
        }
    }
    None
}

/// Resolve an alias's right-hand-side error type against the alias's own generic parameters.
///
/// A name that is not one of them is already concrete. A name that *is* one of them resolves to
/// that parameter's default, and yields no hint at all when the parameter has none — recording the
/// bare parameter name would put a type no crate exports into the IR. ~keep
fn resolve_alias_error_parameter(error_type: &str, generics: &syn::Generics) -> Option<String> {
    let parameter = generics.params.iter().find_map(|param| match param {
        syn::GenericParam::Type(type_param) if type_param.ident == error_type => Some(type_param),
        _ => None,
    });
    match parameter {
        None => Some(error_type.to_string()),
        Some(type_param) => type_param.default.as_ref().map(|(_, default)| type_to_string(default)),
    }
}

/// Extract the error type string from a `Result<T, E>` return type.
pub fn extract_result_error_type(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
    {
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
        if !type_args.is_empty() {
            // A qualified `crate::Result<T>` names its alias outright, which beats whatever the
            // module's `use` statements bound the bare name to — qualifying is usually done
            // precisely because a foreign `Result` already holds the bare name. ~keep
            let hint = match scope_for_result_type_path(&type_path.path) {
                Some(qualified) => hint_for_scope(Some(qualified)),
                None => get_result_error_hint(),
            };
            if let Some(hint) = hint {
                return Some(hint);
            }
            return Some("anyhow::Error".to_string());
        }
    }
    None
}

/// Check if a return type is `Result<T, E>` and return the inner T type.
pub fn unwrap_result_type(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
    {
        for arg in &args.args {
            if let syn::GenericArgument::Type(inner_ty) = arg {
                return Some(inner_ty);
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

    // Regression coverage for #395: fixed-size arrays of primitives have a lossless lowering
    // (`Vec<T>`, or `Bytes` for `u8`) and must not fall through to the sanitizer's lossy
    // `TypeRef::String` placeholder path. Before this fix `resolve_type` had no arm for
    // `syn::Type::Array` at all, so every one of these stringified to `TypeRef::Named("[u8 ; 32]")`
    // / `TypeRef::Named("[f64 ; 3]")` / `TypeRef::Named("[u32 ; 4]")` -- an unknown-type-shaped
    // string the sanitizer cannot lower, since `u8`/`f64`/`u32` are never in `known_types` or
    // `known_enums`.

    #[test]
    fn test_fixed_size_array_of_u8_resolves_to_bytes() {
        assert_eq!(resolve_type(&parse_type("[u8; 32]")), TypeRef::Bytes);
    }

    #[test]
    fn test_fixed_size_array_of_f64_resolves_to_vec() {
        assert_eq!(
            resolve_type(&parse_type("[f64; 3]")),
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F64)))
        );
    }

    #[test]
    fn test_fixed_size_array_of_u32_resolves_to_vec() {
        assert_eq!(
            resolve_type(&parse_type("[u32; 4]")),
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)))
        );
    }

    #[test]
    fn test_fixed_size_array_of_a_named_type_still_stringifies_for_the_sanitizer() {
        // Unaffected by the primitive-array fix: a fixed-size array of a type this module cannot
        // recognize (a user-defined struct/enum) must keep stringifying to `TypeRef::Named` so
        // `sanitize_unknown_types` can still look it up against `known_types`/`known_enums`.
        assert_eq!(
            resolve_type(&parse_type("[Point; 4]")),
            TypeRef::Named("[Point ; 4]".to_string())
        );
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
    fn test_nested_option_preserved() {
        assert_eq!(
            resolve_type(&parse_type("Option<Option<u64>>")),
            TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::Primitive(
                PrimitiveType::U64
            )))))
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
    fn test_ahashmap_resolves_as_map() {
        assert_eq!(
            resolve_type(&parse_type("AHashMap<String, MyType>")),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("MyType".into())))
        );
    }

    #[test]
    fn test_indexmap_resolves_as_map() {
        assert_eq!(
            resolve_type(&parse_type("IndexMap<String, u64>")),
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::U64))
            )
        );
    }

    #[test]
    fn test_fxhashmap_resolves_as_map() {
        assert_eq!(
            resolve_type(&parse_type("FxHashMap<String, bool>")),
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::Bool))
            )
        );
    }

    #[test]
    fn test_hashset_resolves_as_vec() {
        assert_eq!(
            resolve_type(&parse_type("HashSet<String>")),
            TypeRef::Vec(Box::new(TypeRef::String))
        );
    }

    #[test]
    fn test_btreeset_resolves_as_vec() {
        assert_eq!(
            resolve_type(&parse_type("BTreeSet<u32>")),
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)))
        );
    }

    #[test]
    fn test_ahashset_resolves_as_vec() {
        assert_eq!(
            resolve_type(&parse_type("AHashSet<String>")),
            TypeRef::Vec(Box::new(TypeRef::String))
        );
    }

    #[test]
    fn test_indexset_resolves_as_vec() {
        assert_eq!(
            resolve_type(&parse_type("IndexSet<MyType>")),
            TypeRef::Vec(Box::new(TypeRef::Named("MyType".into())))
        );
    }

    #[test]
    fn test_fxhashset_resolves_as_vec() {
        assert_eq!(
            resolve_type(&parse_type("FxHashSet<u64>")),
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64)))
        );
    }

    #[test]
    fn test_path_types() {
        assert_eq!(resolve_type(&parse_type("PathBuf")), TypeRef::Path);
        assert_eq!(resolve_type(&parse_type("&Path")), TypeRef::Path);
        assert_eq!(resolve_type(&parse_type("Path")), TypeRef::Path);
        assert_eq!(resolve_type(&parse_type("impl AsRef<Path>")), TypeRef::Path);
        assert_eq!(resolve_type(&parse_type("impl AsRef<PathBuf>")), TypeRef::Path);
    }

    #[test]
    fn test_unit() {
        assert_eq!(resolve_type(&parse_type("()")), TypeRef::Unit);
    }

    #[test]
    fn test_json() {
        assert_eq!(resolve_type(&parse_type("serde_json::Value")), TypeRef::Json);
        assert_eq!(
            resolve_type(&parse_type("JsonValue")),
            TypeRef::Named("JsonValue".to_string())
        );
        assert_eq!(resolve_type(&parse_type("Value")), TypeRef::Named("Value".to_string()));
        assert_eq!(
            resolve_type(&parse_type("HashMap<String, Value>")),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Value".to_string())))
        );
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
    fn test_secret_string_is_name_agnostic() {
        assert_eq!(
            resolve_type(&parse_type("SecretString")),
            TypeRef::Named("SecretString".to_string())
        );
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
        let alias: syn::ItemType =
            syn::parse_str("pub type Result<T> = std::result::Result<T, SampleCrateError>;").expect("alias must parse");
        assert_eq!(
            extract_result_error_type_from_alias(&alias.ty, &alias.generics),
            Some("SampleCrateError".into())
        );
    }

    #[test]
    fn test_alias_error_parameter_resolves_to_its_default() {
        let alias: syn::ItemType =
            syn::parse_str("pub type Result<T, E = SampleCrateError> = std::result::Result<T, E>;")
                .expect("alias must parse");
        assert_eq!(
            extract_result_error_type_from_alias(&alias.ty, &alias.generics),
            Some("SampleCrateError".into())
        );
    }

    #[test]
    fn test_alias_error_parameter_without_a_default_yields_no_hint() {
        let alias: syn::ItemType =
            syn::parse_str("pub type Result<T, E> = std::result::Result<T, E>;").expect("alias must parse");
        assert_eq!(extract_result_error_type_from_alias(&alias.ty, &alias.generics), None);
    }

    #[test]
    fn test_extract_result_error_with_hint() {
        reset_result_error_hints();
        record_result_error_hint("error", "SampleCrateError".to_string());

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(extract_result_error_type(&ty), Some("SampleCrateError".into()));
    }

    #[test]
    fn test_extract_result_error_fallback_without_hint() {
        reset_result_error_hints();

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(extract_result_error_type(&ty), Some("anyhow::Error".into()));
    }

    #[test]
    fn test_canonical_hint_wins_over_a_deeper_module_private_alias() {
        reset_result_error_hints();
        // Declaration order must not matter: the deeper alias is recorded last on purpose.
        record_result_error_hint("error", "SampleCrateError".to_string());
        record_result_error_hint("extraction::binary::error", "BinaryFormatError".to_string());

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(
            extract_result_error_type(&ty),
            Some("SampleCrateError".into()),
            "a module-private alias must never displace the crate's canonical Result alias"
        );
    }

    #[test]
    fn test_module_private_alias_applies_inside_its_own_module() {
        reset_result_error_hints();
        record_result_error_hint("error", "SampleCrateError".to_string());
        record_result_error_hint("extraction::binary::error", "BinaryFormatError".to_string());
        let _scope =
            ResultAliasScopeGuard::enter(Some(ResultAliasScope::Crate("extraction::binary::error".to_string())));

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(extract_result_error_type(&ty), Some("BinaryFormatError".into()));
    }

    #[test]
    fn test_foreign_result_alias_falls_back_to_anyhow() {
        reset_result_error_hints();
        record_result_error_hint("error", "SampleCrateError".to_string());
        let _scope = ResultAliasScopeGuard::enter(Some(ResultAliasScope::Foreign));

        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(
            extract_result_error_type(&ty),
            Some("anyhow::Error".into()),
            "a module using anyhow::Result must not claim the crate's own error type"
        );
    }

    #[test]
    fn test_alias_scope_guard_restores_the_enclosing_scope() {
        reset_result_error_hints();
        record_result_error_hint("error", "SampleCrateError".to_string());
        record_result_error_hint("extraction::binary::error", "BinaryFormatError".to_string());

        let outer = ResultAliasScopeGuard::enter(Some(ResultAliasScope::Crate(String::new())));
        {
            let _inner =
                ResultAliasScopeGuard::enter(Some(ResultAliasScope::Crate("extraction::binary::error".to_string())));
            let ty = parse_type("Result<ExtractionResult>");
            assert_eq!(extract_result_error_type(&ty), Some("BinaryFormatError".into()));
        }
        let ty = parse_type("Result<ExtractionResult>");
        assert_eq!(extract_result_error_type(&ty), Some("SampleCrateError".into()));
        drop(outer);
    }

    #[test]
    fn test_normalize_type_string_static_str() {
        assert_eq!(normalize_type_string("& 'static str"), "&'static str");
    }

    #[test]
    fn test_normalize_type_string_static_slice_of_static_str() {
        assert_eq!(
            normalize_type_string("& 'static [& 'static str]"),
            "&'static [&'static str]"
        );
    }

    #[test]
    fn test_normalize_type_string_generic_no_spaces() {
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

    #[test]
    fn test_arc_mutex_inner_resolved_through_unwrap() {
        assert_eq!(resolve_type(&parse_type("Arc<Mutex<String>>")), TypeRef::String);
    }

    #[test]
    fn test_arc_rwlock_inner_resolved_through_unwrap() {
        assert_eq!(resolve_type(&parse_type("Arc<RwLock<Vec<u8>>>")), TypeRef::Bytes);
    }

    #[test]
    fn test_arc_hashmap_string_string_inner_resolved() {
        assert_eq!(
            resolve_type(&parse_type("Arc<HashMap<String, String>>")),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String))
        );
    }
}
