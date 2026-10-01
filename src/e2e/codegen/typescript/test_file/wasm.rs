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

pub(in crate::e2e::codegen::typescript::test_file) fn wasm_excluded_class_names(
    surface: &crate::backends::wasm::gen_bindings::EffectiveWasmSurface,
    prefix: &str,
) -> std::collections::BTreeSet<String> {
    surface
        .exclude_types
        .iter()
        .map(|name| wasm_class_name(name, prefix))
        .collect()
}

pub(in crate::e2e::codegen::typescript::test_file) fn effective_wasm_e2e_surface(
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    errors: &[crate::core::ir::ErrorDef],
    config: &crate::core::config::ResolvedCrateConfig,
) -> crate::backends::wasm::gen_bindings::EffectiveWasmSurface {
    let api = crate::core::ir::ApiSurface {
        types: type_defs.to_vec(),
        enums: enums.to_vec(),
        errors: errors.to_vec(),
        ..Default::default()
    };
    crate::backends::wasm::gen_bindings::effective_wasm_surface(&api, config)
}

pub(in crate::e2e::codegen::typescript::test_file) fn without_excluded_wasm_nested_types(
    config: &E2eConfig,
    excluded_classes: &std::collections::BTreeSet<String>,
) -> E2eConfig {
    let mut filtered = config.clone();
    retain_exported_nested_types(&mut filtered.call, excluded_classes);
    for call in filtered.calls.values_mut() {
        retain_exported_nested_types(call, excluded_classes);
    }
    filtered
}

fn retain_exported_nested_types(
    call: &mut crate::core::config::e2e::CallConfig,
    excluded_classes: &std::collections::BTreeSet<String>,
) {
    if let Some(wasm) = call.overrides.get_mut("wasm") {
        wasm.nested_types
            .retain(|_, class_name| wasm_import_is_exported(class_name, excluded_classes));
    }
}

pub(in crate::e2e::codegen::typescript::test_file) fn wasm_call_roots_are_exported(
    fixture: &Fixture,
    call: &crate::core::config::e2e::CallConfig,
    fallback_options_type: Option<&str>,
    excluded_classes: &std::collections::BTreeSet<String>,
    prefix: &str,
) -> bool {
    first_unexported_wasm_call_root(fixture, call, fallback_options_type, excluded_classes, prefix).is_none()
}

pub(crate) fn first_unexported_wasm_call_root(
    fixture: &Fixture,
    call: &crate::core::config::e2e::CallConfig,
    fallback_options_type: Option<&str>,
    excluded_classes: &std::collections::BTreeSet<String>,
    prefix: &str,
) -> Option<String> {
    let wasm_override = call.overrides.get("wasm");
    let options_type = wasm_override
        .and_then(|value| value.options_type.as_deref())
        .or(fallback_options_type)
        .or(call.options_type.as_deref());
    let roots = options_type
        .into_iter()
        .chain(wasm_override.and_then(|value| value.handle_config_type.as_deref()))
        .chain(
            fixture
                .resolved_args(call)
                .iter()
                .filter_map(|arg| arg.element_type.as_deref()),
        );
    roots.into_iter().find_map(|name| {
        let class_name = if name.starts_with(prefix) {
            name.to_string()
        } else {
            wasm_class_name(name, prefix)
        };
        excluded_classes.contains(&class_name).then_some(class_name)
    })
}

pub(in crate::e2e::codegen::typescript::test_file) fn wasm_import_is_exported(
    import_name: &str,
    excluded_classes: &std::collections::BTreeSet<String>,
) -> bool {
    // Auto-derived class names always come from `type_defs`, so unknown references already fail
    // closed before reaching this filter. Unrecognised explicit names remain allowed because a
    // consumer may export them from `[crates.wasm].custom_rust_modules`. ~keep
    let class_name = import_name.strip_prefix("type ").unwrap_or(import_name);
    !excluded_classes.contains(class_name)
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
/// every struct-typed field. Returns the set of every transitively-reachable
/// nested wasm class name.
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
///
/// Returns a `BTreeSet<String>` of class names rather than a field-name-keyed
/// map. Two distinct classes can share a field name (e.g. both
/// `WasmTesseractConfig` and `WasmConversionOptions` expose a field named
/// `preprocessing`, with different nested class types); keying by field name
/// let one collide with and silently drop the other, and which one survived
/// depended on `HashMap` iteration order, making generated output
/// non-deterministic across runs. The only consumer of this return value is
/// the import-statement builder in `render.rs`, which needs the set of class
/// names to import, not a field-to-class mapping, so collapsing to a set of
/// class names is both correct and order-independent. Keep this a `BTreeSet`
/// (not `HashSet`) — the generated `e2e/` output is byte-compared by CI, so
/// iteration order must be deterministic.
///
/// `override_nested_types` is the raw `[[crates.e2e.call.overrides.<lang>]].nested_types`
/// call-override map, merged over the IR-derived fields at every BFS step — the same merge
/// `ts_builder_expression_inner` performs at every
/// recursion depth via its own `effective_nested_types` (see `handle_values.rs`'s
/// `HandleConfigContext::nested_types` doc, which threads the identical raw map unchanged to
/// every depth for the same reason). A fixture author's override key is not required to name a
/// field the IR actually declares on the owning struct — that's the point of the override,
/// covering a case the IR shape does not fit — so a class reachable ONLY through an
/// override-introduced key (e.g. `nested_types.auth = "WasmAuthConfig"` naming a field the IR
/// has no `auth`-typed entry for) was never visited by a walk that consulted `type_defs` alone.
/// Anything reachable ONLY beyond that edge — a further-nested class the override's own target
/// type *does* declare as a real IR field (e.g. `AuthConfig.ssrf: SsrfPolicy`) — was built into
/// the body by the emitter's own recursion but never reached by this collector, and therefore
/// never imported: `ReferenceError: WasmSsrfPolicy is not defined` at runtime, surviving even a
/// correctly-configured `nested_types.auth` entry because that entry names the WRONG hop. ~keep
pub(in crate::e2e::codegen::typescript::test_file) fn collect_transitive_nested_types_for_wasm(
    seed_wasm_types: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    wasm_type_prefix: &str,
    override_nested_types: &std::collections::HashMap<String, String>,
) -> std::collections::BTreeSet<String> {
    let mut result: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut queue: Vec<String> = seed_wasm_types.iter().cloned().collect();
    let mut seen: std::collections::HashSet<String> = queue.iter().cloned().collect();
    while let Some(wasm_type) = queue.pop() {
        let mut derived = derive_nested_types_for_wasm(&wasm_type, type_defs, wasm_type_prefix);
        for (key, value) in override_nested_types {
            derived.insert(key.clone(), value.clone());
        }
        for v in derived.into_values() {
            if seen.insert(v.clone()) {
                queue.push(v.clone());
            }
            result.insert(v);
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
    // Two types extracted from different modules can share a bare `name`
    // (e.g. two distinct `Config` structs). `type_defs` is the IR type
    // registry built upstream of e2e codegen; its slice order is not a
    // contract this code should depend on. A plain `.find()` would return
    // whichever same-named entry happens to come first in that slice,
    // which silently swaps one imported wasm class for another between
    // otherwise-identical `alef e2e generate` runs if the upstream order
    // ever shifts. Break ties deterministically on `rust_path` (the fully
    // qualified module path, guaranteed unique) so the result is stable
    // regardless of `type_defs` ordering.
    let Some(type_def) = type_defs
        .iter()
        .filter(|t| t.name == ir_name)
        .min_by(|a, b| a.rust_path.cmp(&b.rust_path))
    else {
        return std::collections::HashMap::new();
    };
    let mut map = std::collections::HashMap::new();
    for field in crate::codegen::shared::binding_fields(&type_def.fields) {
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
