use super::*;

/// Return the WASM binding class name for an IR type name.
///
/// wasm-bindgen emits each exported Rust type as a JS class named
/// `<prefix><TypeName>`.  For example, with prefix "Wasm", the IR type
/// `ChatMessage` is exposed as `WasmChatMessage`.  This mirrors the
/// `wasm_class_name` helper used elsewhere in the wasm-bindgen backend.
pub(in crate::e2e::codegen::typescript::test_file) fn wasm_class_name(ir_type_name: &str, prefix: &str) -> String {
    format!("{prefix}{ir_type_name}")
}

/// Derive `nested_types` entries from the IR type registry for a given
/// WASM class name.
///
/// For each field in the named IR type whose `TypeRef` is (or contains) a
/// `Named` variant, map `field.name → wasm_class_name(ir_named_type)`.
/// This eliminates the need for manual `nested_types` entries in alef.toml
/// call overrides.
///
/// Rules:
/// - `TypeRef::Named(n)` → field is a direct struct instance; map it.
/// - `TypeRef::Vec(Named(n))` → field is a slice of struct instances; map it
///   (the array-element wrapping path uses the same key).
/// - `TypeRef::Option(inner)` → unwrap recursively; if inner is class-typed,
///   the field should still be mapped.
/// - Everything else (primitives, strings, maps, etc.) → skip.
///
/// BFS over the wasm class graph starting from each `seed_wasm_type` and walking
/// every struct-typed field. Returns a flat field-name → wasm-class-name map
/// covering EVERY transitively-reachable nested class.
///
/// The single-level [`derive_nested_types_for_wasm`] only inspects the seed
/// type's immediate fields. That's insufficient for the import block, because
/// the test body's builder expressions construct nested classes recursively:
/// `WasmChatCompletionRequest.tools[].function = new WasmFunctionDefinition()`.
/// Without this transitive walk, `WasmFunctionDefinition` was emitted in the
/// test body but missing from the import statement, causing
/// `ReferenceError: WasmFunctionDefinition is not defined` at runtime.
///
/// Termination is guaranteed by a `seen` set on wasm class names.
pub(in crate::e2e::codegen::typescript::test_file) fn collect_transitive_nested_types_for_wasm(
    seed_wasm_types: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    wasm_type_prefix: &str,
) -> std::collections::HashMap<String, String> {
    let mut result: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut queue: Vec<String> = seed_wasm_types.iter().cloned().collect();
    let mut seen: std::collections::HashSet<String> = queue.iter().cloned().collect();
    while let Some(wasm_type) = queue.pop() {
        let derived = derive_nested_types_for_wasm(&wasm_type, type_defs, wasm_type_prefix);
        for (k, v) in derived {
            if seen.insert(v.clone()) {
                queue.push(v.clone());
            }
            result.entry(k).or_insert(v);
        }
    }
    result
}

pub(in crate::e2e::codegen::typescript::test_file) fn derive_nested_types_for_wasm(
    wasm_type_name: &str,
    type_defs: &[TypeDef],
    wasm_type_prefix: &str,
) -> std::collections::HashMap<String, String> {
    // Strip the prefix to get the IR type name.
    let ir_name = wasm_type_name.strip_prefix(wasm_type_prefix).unwrap_or(wasm_type_name);
    let Some(type_def) = type_defs.iter().find(|t| t.name == ir_name) else {
        return std::collections::HashMap::new();
    };
    let mut map = std::collections::HashMap::new();
    for field in &type_def.fields {
        if let Some(class_name) = class_name_from_type_ref(&field.ty) {
            // Only map fields whose IR type is a struct (TypeDef). Sealed-union
            // enums (EnumDef) don't expose a constructible wasm-bindgen class
            // — wasm-bindgen serialises them via discriminator from a plain
            // object literal, so wrapping them with `new <prefix><Enum>()` fails
            // with `<prefix>Foo is not a constructor`. Looking up the name in
            // type_defs filters enums out (they're carried in EnumDef, not here).
            if type_defs.iter().any(|t| t.name == class_name) {
                map.insert(field.name.clone(), wasm_class_name(&class_name, wasm_type_prefix));
            }
        }
    }
    map
}

/// Prefix a bare IR type name with `wasm_type_prefix` when it names a
/// wasm-wrapped struct or enum, leaving primitives / host types untouched.
///
/// The wasm-bindgen backend exposes every wrapped Rust struct/enum under the
/// `wasm_type_prefix` (e.g. `ExtractInput` -> `WasmExtractInput`). Config
/// option types are already prefixed via per-language `options_type`
/// overrides in `alef.toml`, but a bare `element_type` on a `json_object`
/// arg (e.g. the `extract` call's `ExtractInput` input) is not. Both the
/// generated constructor call (`WasmExtractInput.default()`) and the import
/// statement must reference the prefixed name, or the test throws
/// `ReferenceError: WasmExtractInput is not defined` at runtime.
///
/// Returns `type_name` unchanged when it is already prefixed, when it is not
/// a known wrapped type (TS primitives, `Uint8Array`, ...), or when `lang`
/// is not `"wasm"`.
pub(in crate::e2e::codegen::typescript::test_file) fn wasm_prefixed_wrapped_type(
    lang: &str,
    type_name: &str,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
) -> String {
    if lang != "wasm" || type_name.starts_with(wasm_type_prefix) {
        return type_name.to_string();
    }
    let is_wrapped = type_defs.iter().any(|t| t.name == type_name) || enums.iter().any(|e| e.name == type_name);
    if is_wrapped {
        format!("{wasm_type_prefix}{type_name}")
    } else {
        type_name.to_string()
    }
}

/// Recursively inspect a `TypeRef` to find the innermost named type, if any.
///
/// Returns the IR type name (without the `Wasm` prefix) when the type
/// resolves to a struct/class, or `None` for primitives and other scalars.
fn class_name_from_type_ref(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(name) => Some(name.clone()),
        TypeRef::Vec(inner) => class_name_from_type_ref(inner),
        TypeRef::Optional(inner) => class_name_from_type_ref(inner),
        _ => None,
    }
}
