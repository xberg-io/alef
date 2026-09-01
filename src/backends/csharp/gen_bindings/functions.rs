//! C# NativeMethods (P/Invoke) code generation.

use super::pinvoke::{gen_pinvoke_for_func, gen_pinvoke_for_method, is_bytes_result_func, is_bytes_result_method};
use super::{HANDLE_PINVOKE_TYPE, StreamingMethodMeta};
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::TraitBridgeConfig;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::{ApiSurface, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

fn emits_registered_trait_bridge(has_visitor_callbacks: bool, config: &TraitBridgeConfig) -> bool {
    !has_visitor_callbacks
        || config.bind_via != crate::core::config::BridgeBinding::OptionsField
        || config.register_fn.is_some()
}

fn ffi_handle_type_names(api: &ApiSurface) -> HashSet<&str> {
    let mut names: HashSet<&str> = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait)
        .map(|typ| typ.name.as_str())
        .collect();
    // Every enum boxes as `AlefHandle` on return, fieldless or data-carrying alike — see
    // `enum_names_with_data_variants` in `marshalling.rs` for the FFI-side evidence. Filtering
    // this by variant shape (as a prior revision did) silently dropped the `{Pascal}ToJson`/
    // `{Pascal}Free` P/Invoke declarations for fieldless-only enum returns. ~keep
    names.extend(api.enums.iter().map(|enum_def| enum_def.name.as_str()));
    names
}

/// Map a Rust FFI type string to the C# P/Invoke parameter declaration.
///
/// String parameters use explicit UTF-8 marshalling to match the C `const char*` ABI.
fn ffi_ty_to_pinvoke_param(rust_ty: &str, param_name: &str) -> String {
    let normalized = rust_ty.trim();
    let cs_name = param_name.to_lower_camel_case();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return format!("[MarshalAs(UnmanagedType.LPUTF8Str)] string {cs_name}");
    }
    let cs_type = match normalized {
        "bool" => "bool",
        "u8" | "uint8_t" => "byte",
        "u16" | "uint16_t" => "ushort",
        "u32" | "uint32_t" => "uint",
        "u64" | "uint64_t" | "usize" => "ulong",
        "i8" | "int8_t" => "sbyte",
        "i16" | "int16_t" => "short",
        "i32" | "int32_t" | "c_int" => "int",
        "i64" | "int64_t" | "isize" => "long",
        "f32" | "float" => "float",
        "f64" | "double" => "double",
        _ => "IntPtr",
    };
    format!("{cs_type} {cs_name}")
}

/// Reduces a P/Invoke parameter list to its ABI-relevant shape by dropping each parameter's
/// trailing identifier — a P/Invoke call binds arguments positionally by type, so the local
/// variable name carries no marshalling meaning. Used by `emit_streaming_pinvoke` to tell a
/// genuine signature divergence apart from mere renaming. ~keep
fn pinvoke_param_shape(params: &str) -> Vec<String> {
    params
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.rsplit_once(char::is_whitespace)
                .map_or_else(|| p.to_string(), |(ty, _)| ty.trim().to_string())
        })
        .collect()
}

/// Emits one streaming P/Invoke declaration (`_start`/`_next`/`_free`), deduplicating by C entry
/// point across the two independent emitters in `gen_native_methods` that both cover streaming
/// symbols: the literal `typ.methods` walk and the `[[crates.adapters]]` walk. A method that is
/// both a real IR method AND configured as a `pattern = "streaming"` adapter with a matching
/// `owner_type` is walked by both — see `tests::e2e_csharp_opaque_streaming_wrapper` for that
/// sanctioned overlap. Rather than letting whichever emitter runs first silently win, this
/// tracks the ABI shape already emitted per entry point and hard-fails on a genuine divergence
/// (different return type or parameter types). A divergence limited to a parameter's local name
/// (e.g. `client` vs `engine`) is not an ABI difference, so that case is left to coalesce
/// silently — see `streaming_pinvoke_dedup_rejects_real_signature_divergence` and
/// `streaming_pinvoke_dedup_allows_cosmetic_name_divergence`. ~keep
#[allow(clippy::too_many_arguments)]
fn emit_streaming_pinvoke(
    out: &mut String,
    emitted: &mut HashSet<String>,
    signatures: &mut HashMap<String, (Vec<String>, String)>,
    entry_point: String,
    cs_name: &str,
    return_type: &str,
    params: &str,
) -> anyhow::Result<()> {
    use crate::backends::csharp::template_env::render;

    let mut shape = pinvoke_param_shape(params);
    shape.insert(0, return_type.to_string());
    let declared_as = format!("{return_type} {cs_name}({params})");

    if let Some((existing_shape, existing_declared_as)) = signatures.get(&entry_point) {
        anyhow::ensure!(
            existing_shape == &shape,
            "csharp NativeMethods: the streaming P/Invoke symbol `{entry_point}` is emitted \
             twice with disagreeing signatures — one emitter declared `{existing_declared_as}`, \
             another declared `{declared_as}`. A method that exists both as a real IR method and \
             as a `[[crates.adapters]] pattern = \"streaming\"` entry must agree on its shape \
             with the adapter config; fix whichever side is stale.",
        );
    } else {
        signatures.insert(entry_point.clone(), (shape, declared_as));
    }

    if emitted.insert(entry_point.clone()) {
        out.push_str(&render(
            "dll_import_attr.jinja",
            minijinja::context! { entry_point => &entry_point },
        ));
        out.push_str(&render(
            "streaming_pinvoke_declaration.jinja",
            minijinja::context! { return_type, cs_name, params },
        ));
        out.push('\n');
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_native_methods(
    api: &ApiSurface,
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_callbacks: bool,
    trait_bridges: &[TraitBridgeConfig],
    streaming_methods: &HashSet<String>,
    streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    exclude_functions: &HashSet<String>,
    client_constructors: &HashMap<String, ClientConstructorConfig>,
    adapters: &[crate::core::config::AdapterConfig],
    capsule_types: &HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> anyhow::Result<String> {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let scalar_named_types = crate::backends::ffi::type_map::scalar_c_abi_named_types(api);

    let mut out = render(
        "native_methods_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "lib_name": lib_name,
        })),
    );
    out.push('\n');

    let mut emitted: HashSet<String> = HashSet::new();
    let mut streaming_signatures: HashMap<String, (Vec<String>, String)> = HashMap::new();

    let mut opaque_param_types: HashSet<String> = HashSet::new();
    let mut opaque_return_types: HashSet<String> = HashSet::new();

    fn inner_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    // Enum-named returns are NOT excluded at insertion time below (in any of the three sites
    // that feed `opaque_return_types`): every enum boxes as `AlefHandle` exactly like a struct
    // (`gen_owned_value_to_c` in the FFI crate has no enum-ness branch for owned return
    // conversion, and no fieldless-vs-data-carrying branch either) and needs the same
    // `{Pascal}ToJson`/`{Pascal}Free` P/Invoke declarations. The
    // `retain(|name| ffi_handle_type_names.contains(name))` below is the single place that keeps
    // only genuine handle types (structs plus every enum, per `ffi_handle_type_names`) — it must
    // not filter enums by variant shape, since that previously dropped these declarations for
    // fieldless-only enum returns (e.g. a consumer's `RefreshOutcome`), which is the CS1503
    // root cause. ~keep
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        for param in &func.params {
            if let TypeRef::Named(name) = &param.ty {
                opaque_param_types.insert(name.clone());
            }
        }
        if let Some(name) = inner_named(&func.return_type) {
            opaque_return_types.insert(name.to_string());
        }
    }
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                for param in &method.params {
                    if let TypeRef::Named(name) = &param.ty {
                        opaque_param_types.insert(name.clone());
                    }
                }
                if let Some(meta) = streaming_methods_meta.get(&method.name) {
                    opaque_return_types.insert(meta.item_type.clone());
                }
                continue;
            }
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    opaque_param_types.insert(name.clone());
                }
            }
            if let Some(name) = inner_named(&method.return_type) {
                opaque_return_types.insert(name.to_string());
            }
            if method.receiver.is_some() {
                opaque_param_types.insert(typ.name.clone());
                opaque_return_types.insert(typ.name.clone());
            }
        }
    }

    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|typ| typ.is_opaque && !typ.is_trait)
        .map(|t| t.name.clone())
        .collect();
    let ffi_handle_type_names = ffi_handle_type_names(api);
    opaque_param_types.retain(|name| ffi_handle_type_names.contains(name.as_str()));
    opaque_return_types.retain(|name| ffi_handle_type_names.contains(name.as_str()));
    opaque_param_types.retain(|name| !bridge_type_aliases.contains(name));
    opaque_return_types.retain(|name| !bridge_type_aliases.contains(name));

    let mut sorted_true_opaque_types: Vec<&String> = true_opaque_types.iter().collect();
    sorted_true_opaque_types.sort();
    for type_name in sorted_true_opaque_types {
        let snake = type_name.to_snake_case();
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

    let mut sorted_ctor_types: Vec<&String> = client_constructors.keys().collect();
    sorted_ctor_types.sort();
    for type_name in sorted_ctor_types {
        let ctor = &client_constructors[type_name];
        let snake = type_name.to_snake_case();
        let new_entry = format!("{prefix}_{snake}_new");
        let new_cs = format!("{}New", csharp_type_name(type_name));
        if emitted.insert(new_entry.clone()) {
            let params_str: String = ctor
                .params
                .iter()
                .map(|p| ffi_ty_to_pinvoke_param(&p.ty, &p.name))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &new_entry },
            ));
            out.push_str(&render(
                "client_constructor_pinvoke.jinja",
                minijinja::context! { new_cs, params_str },
            ));
            out.push('\n');
        }
    }

    let mut sorted_param_types: Vec<&String> = opaque_param_types.iter().collect();
    sorted_param_types.sort();
    for type_name in sorted_param_types {
        let snake = type_name.to_snake_case();
        // A scalar-crossing named type (a `Copy` enum or struct) is passed as `int32_t`, never as
        // an `AlefHandle`, so it has no handle lifecycle in a parameter position: the C FFI
        // backend emits `from_i32`/`from_str` for it and neither `from_json` nor a param-driven
        // `free`. Declaring either here would bind a symbol the native library does not export.
        // A scalar type that is also *returned* still gets its `free` from the return loop below,
        // which mirrors the FFI's own returned-enum condition. ~keep
        if scalar_named_types.contains(type_name.as_str()) {
            continue;
        }
        if !true_opaque_types.contains(type_name) {
            let from_json_entry = format!("{prefix}_{snake}_from_json");
            let from_json_cs = format!("{}FromJson", csharp_type_name(type_name));
            if emitted.insert(from_json_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &from_json_entry },
                ));
                out.push_str(&render(
                    "extern_ptr_from_json.jinja",
                    minijinja::context! { cs_name => &from_json_cs },
                ));
                out.push('\n');
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

    let mut sorted_return_types: Vec<&String> = opaque_return_types.iter().collect();
    sorted_return_types.sort();
    for type_name in sorted_return_types {
        let snake = type_name.to_snake_case();
        if !true_opaque_types.contains(type_name) {
            let to_json_entry = format!("{prefix}_{snake}_to_json");
            let to_json_cs = format!("{}ToJson", csharp_type_name(type_name));
            if emitted.insert(to_json_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &to_json_entry },
                ));
                out.push_str(&render(
                    "extern_ptr_to_json.jinja",
                    minijinja::context! { cs_name => &to_json_cs },
                ));
                out.push('\n');
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

    for func in api.functions.iter().filter(|f| {
        !exclude_functions.contains(&f.name)
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, trait_bridges)
    }) {
        let c_func_name = format!("{}_{}", prefix, func.name.to_lowercase());
        if emitted.insert(c_func_name.clone()) {
            out.push_str(&gen_pinvoke_for_func(
                &c_func_name,
                func,
                bridge_param_names,
                bridge_type_aliases,
                capsule_types,
                &scalar_named_types,
            ));
        }
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                continue;
            }
            if method.returns_ref_to_owner(&typ.name) {
                continue;
            }
            let c_method_name = format!("{}_{}_{}", prefix, type_snake, method.name.to_lowercase());
            let cs_method_name = format!("{}{}", csharp_type_name(&typ.name), to_csharp_name(&method.name));
            if emitted.insert(c_method_name.clone()) {
                out.push_str(&gen_pinvoke_for_method(
                    &c_method_name,
                    &cs_method_name,
                    method,
                    capsule_types,
                    &scalar_named_types,
                ));
            }
        }
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if !streaming_methods.contains(&method.name) {
                continue;
            }
            let cs_type = csharp_type_name(&typ.name);
            let cs_method = to_csharp_name(&method.name);

            let start_entry = format!("{}_{}_{}_start", prefix, type_snake, method.name.to_lowercase());
            let start_cs = format!("{cs_type}{cs_method}Start");
            emit_streaming_pinvoke(
                &mut out,
                &mut emitted,
                &mut streaming_signatures,
                start_entry,
                &start_cs,
                HANDLE_PINVOKE_TYPE,
                &format!("{HANDLE_PINVOKE_TYPE} client, {HANDLE_PINVOKE_TYPE} req"),
            )?;

            let next_entry = format!("{}_{}_{}_next", prefix, type_snake, method.name.to_lowercase());
            let next_cs = format!("{cs_type}{cs_method}Next");
            emit_streaming_pinvoke(
                &mut out,
                &mut emitted,
                &mut streaming_signatures,
                next_entry,
                &next_cs,
                HANDLE_PINVOKE_TYPE,
                &format!("{HANDLE_PINVOKE_TYPE} handle"),
            )?;

            let free_entry = format!("{}_{}_{}_free", prefix, type_snake, method.name.to_lowercase());
            let free_cs = format!("{cs_type}{cs_method}Free");
            emit_streaming_pinvoke(
                &mut out,
                &mut emitted,
                &mut streaming_signatures,
                free_entry,
                &free_cs,
                "void",
                &format!("{HANDLE_PINVOKE_TYPE} handle"),
            )?;
        }
    }

    for adapter in adapters {
        if matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
            let Some(owner_type) = adapter.owner_type.as_deref() else {
                continue;
            };
            let owner_snake = owner_type.to_snake_case();
            let owner_cs = csharp_type_name(owner_type);
            let adapter_snake = adapter.name.to_snake_case();
            let adapter_cs = to_csharp_name(&adapter.name);

            let start_entry = format!("{}_{}_{}_start", prefix, owner_snake, adapter_snake);
            let start_cs = format!("{owner_cs}{adapter_cs}Start");
            emit_streaming_pinvoke(
                &mut out,
                &mut emitted,
                &mut streaming_signatures,
                start_entry,
                &start_cs,
                HANDLE_PINVOKE_TYPE,
                &format!("{HANDLE_PINVOKE_TYPE} engine, {HANDLE_PINVOKE_TYPE} req"),
            )?;

            let next_entry = format!("{}_{}_{}_next", prefix, owner_snake, adapter_snake);
            let next_cs = format!("{owner_cs}{adapter_cs}Next");
            emit_streaming_pinvoke(
                &mut out,
                &mut emitted,
                &mut streaming_signatures,
                next_entry,
                &next_cs,
                HANDLE_PINVOKE_TYPE,
                &format!("{HANDLE_PINVOKE_TYPE} handle"),
            )?;

            let free_entry = format!("{}_{}_{}_free", prefix, owner_snake, adapter_snake);
            let free_cs = format!("{owner_cs}{adapter_cs}Free");
            emit_streaming_pinvoke(
                &mut out,
                &mut emitted,
                &mut streaming_signatures,
                free_entry,
                &free_cs,
                "void",
                &format!("{HANDLE_PINVOKE_TYPE} handle"),
            )?;
        }
    }

    let last_error_code_entry = format!("{prefix}_last_error_code");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &last_error_code_entry },
    ));
    out.push_str("    internal static extern int LastErrorCode();\n\n");

    let last_error_context_entry = format!("{prefix}_last_error_context");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &last_error_context_entry },
    ));
    out.push_str("    internal static extern IntPtr LastErrorContext();\n\n");

    let free_string_entry = format!("{prefix}_free_string");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &free_string_entry },
    ));
    out.push_str("    internal static extern void FreeString(IntPtr ptr);\n\n");

    let has_bytes_results = api.functions.iter().any(is_bytes_result_func)
        || api
            .types
            .iter()
            .any(|typ| typ.methods.iter().any(is_bytes_result_method));
    if has_bytes_results {
        let free_bytes_entry = format!("{prefix}_free_bytes");
        out.push_str(&render(
            "dll_import_attr.jinja",
            minijinja::context! { entry_point => &free_bytes_entry },
        ));
        out.push_str("    internal static extern void FreeBytes(IntPtr ptr, UIntPtr len, UIntPtr cap);\n");
    }

    if has_visitor_callbacks {
        out.push('\n');
        // Every options-field bridge gets a setter declaration, not just the first one: the FFI
        // crate loops over `config.trait_bridges` and emits `{prefix}_options_set_{field}` for
        // each (`backends::ffi::gen_bindings::lib_rs.rs:558`), and
        // `bridge_field_inject.jinja` emits a `{Options}Set{Field}` call for each too, so a
        // `find`-style single pick left every bridge after the first calling an undeclared
        // member (CS0117). `visitor_create` / `visitor_free` stay singular because the FFI side
        // is singular there (`lib_rs.rs:588` is a `find_map`). ~keep
        let visitor_bridges: Vec<_> = trait_bridges
            .iter()
            .filter(|b| {
                b.bind_via == crate::core::config::BridgeBinding::OptionsField
                    && b.is_active_for(&crate::core::config::Language::Csharp.to_string())
            })
            .collect();

        if let Some(first_bridge) = visitor_bridges.first() {
            let mut options_setters: Vec<(String, String)> = Vec::with_capacity(visitor_bridges.len());
            for bridge in &visitor_bridges {
                // Both names below are load-bearing ABI, not cosmetics: the emitted declaration is
                // `[DllImport(EntryPoint = "{prefix}_options_set_{field}")] ... {OptionsType}Set{Field}`,
                // and the FFI crate emits that setter ONLY for a bridge that resolves both keys
                // (`backends::ffi::gen_bindings::lib_rs` skips the bridge when either is `None`).
                // Guessing `options_type` declares a P/Invoke for a symbol the native library never
                // exports; guessing the field name additionally desyncs the declaration from the call
                // `bridge_field_inject.jinja` emits off the real IR field, which is a C# compile error.
                // `options_type` is documented as required under `bind_via = "options_field"`, so
                // absence is a config defect. `function_param` bridges are filtered out above and are
                // unaffected; `[crates.<name>.ffi] visitor_callbacks = false` skips this block whole. ~keep
                let Some(options_type) = bridge.options_type.as_deref() else {
                    anyhow::bail!(
                        "csharp NativeMethods: trait bridge `{trait_name}` sets `bind_via = \"options_field\"` but \
                         no `options_type`. Set `options_type` on its `[[crates.trait_bridges]]` entry, or set \
                         `[crates.<name>.ffi] visitor_callbacks = false`",
                        trait_name = bridge.trait_name,
                    );
                };
                let Some(options_field) = bridge.resolved_options_field() else {
                    anyhow::bail!(
                        "csharp NativeMethods: trait bridge `{trait_name}` sets `bind_via = \"options_field\"` but \
                         neither `options_field` nor `param_name`, so `{prefix}_options_set_<field>` cannot be \
                         derived. Set one on its `[[crates.trait_bridges]]` entry, or set \
                         `[crates.<name>.ffi] visitor_callbacks = false`",
                        trait_name = bridge.trait_name,
                    );
                };
                options_setters.push((options_type.to_owned(), options_field.to_owned()));
            }
            out.push_str(&crate::backends::csharp::gen_visitor::gen_native_methods_visitor(
                namespace,
                lib_name,
                prefix,
                &first_bridge.trait_name,
                HANDLE_PINVOKE_TYPE,
                &options_setters,
            ));
        }
    }

    if !trait_bridges.is_empty() {
        let trait_defs: Vec<_> = api.types.iter().filter(|t| t.is_trait).collect();

        let bridges: Vec<_> = trait_bridges
            .iter()
            .filter(|config| emits_registered_trait_bridge(has_visitor_callbacks, config))
            .filter_map(|config| {
                let trait_name = config.trait_name.clone();
                trait_defs
                    .iter()
                    .find(|t| t.name == trait_name)
                    .map(|trait_def| (trait_name, config, *trait_def))
            })
            .collect();

        if !bridges.is_empty() {
            let visible_type_names: std::collections::HashSet<&str> = api
                .types
                .iter()
                .filter(|t| !t.is_trait)
                .map(|t| t.name.as_str())
                .collect();
            out.push('\n');
            out.push_str(
                &crate::backends::csharp::trait_bridge::gen_native_methods_trait_bridges(
                    namespace,
                    prefix,
                    &bridges,
                    &visible_type_names,
                    has_visitor_callbacks,
                    HANDLE_PINVOKE_TYPE,
                ),
            );
        }
    }

    out.push_str("}\n");

    // `NativeMethods.cs` spells every handle type by hand via `is_handle_type` — nothing here
    // reads the cbindgen header, so a drift between this match and the FFI crate's own
    // `Named -> AlefHandle` mapping in `backends::ffi::type_map` would compile cleanly and only
    // misbehave at runtime. The `alef:handle-abi:1` stamp below asserts that this file was
    // regenerated by a version of this backend that emits the scalar mapping; it does NOT
    // verify the emitted body actually matches the FFI crate's ABI — nothing in this pipeline
    // diffs the two sides, so a future regression here would still compile and stamp cleanly.
    // `native_methods_declares_scalar_handles_not_pointers` below is what actually pins the
    // shape of the emitted text. Stamped inside the backend body so the marker is part of the
    // content `finalize_hashes` hashes; stamping after `inject_hash_line` would leave every
    // generated C# file permanently stale. ~keep
    Ok(crate::core::hash::inject_stamp_line(
        &out,
        crate::core::hash::HANDLE_ABI_STAMP_KEY,
        crate::core::template_versions::abi::HANDLE_ABI_VERSION,
    ))
}

#[cfg(test)]
mod tests {
    use super::{HANDLE_PINVOKE_TYPE, emit_streaming_pinvoke, emits_registered_trait_bridge, ffi_handle_type_names};
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef};

    /// Reproduces the collision `gen_native_methods` hits when a streaming method is walked by
    /// both the `typ.methods` emitter and the `[[crates.adapters]]` emitter (see
    /// `tests::e2e_csharp_opaque_streaming_wrapper::test_opaque_streaming_static_wrapper` for the
    /// full end-to-end scenario). The two emitters here disagree on the `_start` signature's
    /// second parameter type (`ulong req` vs `IntPtr req`) — a genuine ABI divergence, not just a
    /// renamed identifier — so the second call must hard-fail instead of the first emitter's
    /// declaration silently winning.
    #[test]
    fn streaming_pinvoke_dedup_rejects_real_signature_divergence() {
        use std::collections::{HashMap, HashSet};

        let mut out = String::new();
        let mut emitted = HashSet::new();
        let mut signatures = HashMap::new();

        emit_streaming_pinvoke(
            &mut out,
            &mut emitted,
            &mut signatures,
            "sample_stream_engine_stream_items_start".to_string(),
            "StreamEngineStreamItemsStart",
            "ulong",
            "ulong client, ulong req",
        )
        .expect("first emission establishes the signature");

        let error = emit_streaming_pinvoke(
            &mut out,
            &mut emitted,
            &mut signatures,
            "sample_stream_engine_stream_items_start".to_string(),
            "StreamEngineStreamItemsStart",
            "ulong",
            "ulong engine, IntPtr req",
        )
        .expect_err("a real parameter-type divergence on the same C symbol must not be silently dropped");

        let message = error.to_string();
        assert!(
            message.contains("sample_stream_engine_stream_items_start"),
            "error must name the colliding entry point: {message}"
        );
        assert!(
            message.contains("ulong client, ulong req") && message.contains("ulong engine, IntPtr req"),
            "error must name both disagreeing signatures: {message}"
        );
    }

    /// The sanctioned overlap (same scenario as the divergence test above, but with the two
    /// emitters' hardcoded shapes as they exist in `gen_native_methods` today): both declare
    /// `ulong`-typed parameters and only the local parameter identifier differs (`client` vs
    /// `engine`). That is not an ABI difference — P/Invoke binds positionally by type — so the
    /// second call must coalesce silently rather than erroring, and only one declaration must
    /// reach the output.
    #[test]
    fn streaming_pinvoke_dedup_allows_cosmetic_name_divergence() {
        use std::collections::{HashMap, HashSet};

        let mut out = String::new();
        let mut emitted = HashSet::new();
        let mut signatures = HashMap::new();

        emit_streaming_pinvoke(
            &mut out,
            &mut emitted,
            &mut signatures,
            "sample_stream_engine_stream_items_start".to_string(),
            "StreamEngineStreamItemsStart",
            "ulong",
            "ulong client, ulong req",
        )
        .expect("first emission succeeds");

        emit_streaming_pinvoke(
            &mut out,
            &mut emitted,
            &mut signatures,
            "sample_stream_engine_stream_items_start".to_string(),
            "StreamEngineStreamItemsStart",
            "ulong",
            "ulong engine, ulong req",
        )
        .expect("a parameter-name-only divergence carries no ABI meaning and must not error");

        assert_eq!(
            out.matches("StreamEngineStreamItemsStart").count(),
            1,
            "only one declaration for the shared entry point may reach the output:\n{out}"
        );
    }

    /// Traits are excluded (no C ABI handle for a vtable-only type); every enum is included
    /// regardless of variant shape, fieldless (`NodeKind`) or data-carrying (`CrawlEvent`) alike
    /// — see `ffi_handle_type_names`'s doc comment for why filtering enums by variant shape was
    /// the CS1503 root cause for `liter-llm`'s fieldless-only `RefreshOutcome`. ~keep
    #[test]
    fn ffi_handle_types_exclude_traits_but_include_every_enum() {
        let api = ApiSurface {
            types: vec![type_def("RenderOptions", false), type_def("MarkupVisitor", true)],
            enums: vec![
                EnumDef {
                    name: "NodeKind".to_string(),
                    ..EnumDef::default()
                },
                EnumDef {
                    name: "CrawlEvent".to_string(),
                    variants: vec![EnumVariant {
                        name: "Progress".to_string(),
                        fields: vec![FieldDef::default()],
                        ..EnumVariant::default()
                    }],
                    ..EnumDef::default()
                },
            ],
            ..ApiSurface::default()
        };

        let names = ffi_handle_type_names(&api);

        assert!(names.contains("RenderOptions"));
        assert!(!names.contains("MarkupVisitor"));
        assert!(
            names.contains("NodeKind"),
            "a fieldless-only enum return must still get its ToJson/Free P/Invoke declarations"
        );
        assert!(names.contains("CrawlEvent"));
    }

    /// Regression for the CS1503/CS0117 root cause: a data-carrying enum returned by a free
    /// function boxes as `AlefHandle` exactly like a struct return (see
    /// `enum_names_with_data_variants` in `marshalling.rs`), so it needs the same
    /// `{Pascal}ToJson`/`{Pascal}Free` P/Invoke declarations a plain data struct gets. Before the
    /// fix, `gen_native_methods` blanket-excluded every enum name (fieldless or data-carrying)
    /// from `opaque_return_types` at insertion time, so `ffi_handle_type_names`'s correct
    /// data-carrying-enum retain step (commit `420504797`) never got a chance to run — these
    /// declarations were silently never emitted, and the wrapper body (see
    /// `wrappers::tests::async_and_sync_data_carrying_enum_returns_both_use_to_json_round_trip`)
    /// called a `NativeMethods` member that does not exist. ~keep
    #[test]
    fn data_carrying_enum_return_gets_to_json_and_free_pinvoke_declarations() {
        use crate::core::ir::{FunctionDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let api = ApiSurface {
            crate_name: "sample".to_string(),
            enums: vec![EnumDef {
                name: "RefreshOutcome".to_string(),
                has_serde: true,
                variants: vec![EnumVariant {
                    name: "Skipped".to_string(),
                    fields: vec![FieldDef::default()],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            }],
            functions: vec![FunctionDef {
                name: "refresh_catalog".to_string(),
                rust_path: "sample::refresh_catalog".to_string(),
                return_type: TypeRef::Named("RefreshOutcome".to_string()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };

        let native_methods = super::gen_native_methods(
            &api,
            "Sample",
            "sample",
            "sample",
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .expect("no trait bridges configured, so generation cannot fail");

        assert!(
            native_methods.contains("internal static extern ulong RefreshCatalog();"),
            "an enum-returning function must declare the scalar AlefHandle, not IntPtr:\n{native_methods}"
        );
        assert!(
            native_methods.contains("internal static extern IntPtr RefreshOutcomeToJson(ulong ptr);"),
            "a data-carrying enum return must get a ToJson P/Invoke declaration, matching a \
             plain data struct return:\n{native_methods}"
        );
        assert!(
            native_methods.contains("internal static extern void RefreshOutcomeFree(ulong ptr);"),
            "a data-carrying enum return must get a Free P/Invoke declaration, matching a \
             plain data struct return:\n{native_methods}"
        );
    }

    /// Regression for alef task #155: `liter-llm` v1.17.3's real `RefreshOutcome` is a
    /// *fieldless-only* enum (`Disabled`, `FromCache`, `Fetched` — no variant carries data), not
    /// the data-carrying fixture the earlier `data_carrying_enum_return_gets_to_json_and_free_
    /// pinvoke_declarations` test used. The FFI crate's `enum_pointer_return`/`gen_enum_to_json`
    /// (`backends::ffi::gen_bindings::lib_rs`) gate `_to_json`/`_free` emission on "is this enum
    /// ever returned by value" and `has_serde`, never on variant shape, so
    /// `literllm_refresh_outcome_to_json`/`literllm_refresh_outcome_free` exist in the real FFI
    /// header regardless. Filtering `ffi_handle_type_names` by variant shape (the pre-fix
    /// behavior) silently dropped these C# P/Invoke declarations for the fieldless case, which is
    /// exactly what shipped broken. ~keep
    #[test]
    fn fieldless_enum_return_gets_to_json_and_free_pinvoke_declarations() {
        use crate::core::ir::{FunctionDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let api = ApiSurface {
            crate_name: "sample".to_string(),
            enums: vec![EnumDef {
                name: "RefreshOutcome".to_string(),
                has_serde: true,
                variants: vec![
                    EnumVariant {
                        name: "Disabled".to_string(),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "FromCache".to_string(),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "Fetched".to_string(),
                        ..EnumVariant::default()
                    },
                ],
                ..EnumDef::default()
            }],
            functions: vec![FunctionDef {
                name: "refresh_catalog".to_string(),
                rust_path: "sample::refresh_catalog".to_string(),
                is_async: true,
                return_type: TypeRef::Named("RefreshOutcome".to_string()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };

        let native_methods = super::gen_native_methods(
            &api,
            "Sample",
            "sample",
            "sample",
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .expect("no trait bridges configured, so generation cannot fail");

        assert!(
            native_methods.contains("internal static extern ulong RefreshCatalog();"),
            "a fieldless enum-returning function must declare the scalar AlefHandle, not IntPtr:\n{native_methods}"
        );
        assert!(
            native_methods.contains("internal static extern IntPtr RefreshOutcomeToJson(ulong ptr);"),
            "a fieldless-only enum return must still get a ToJson P/Invoke declaration:\n{native_methods}"
        );
        assert!(
            native_methods.contains("internal static extern void RefreshOutcomeFree(ulong ptr);"),
            "a fieldless-only enum return must still get a Free P/Invoke declaration:\n{native_methods}"
        );
    }

    #[test]
    fn visitor_callbacks_preserve_registered_options_field_pinvoke() {
        let config: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "MarkupVisitor"
bind_via = "options_field"
options_type = "RenderOptions"
options_field = "visitor"
register_fn = "register_markup_visitor"
registry_getter = "markup_visitor_registry"
"#,
        )
        .unwrap();
        let resolved = config.resolve().unwrap();
        let bridge = &resolved[0].trait_bridges[0];

        assert!(emits_registered_trait_bridge(true, bridge));
    }

    fn type_def(name: &str, is_trait: bool) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("sample::{name}"),
            original_rust_path: format!("sample::{name}"),
            is_trait,
            ..TypeDef::default()
        }
    }

    #[test]
    fn native_methods_carry_the_handle_abi_stamp() {
        use crate::core::ir::{FunctionDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let api = ApiSurface {
            crate_name: "sample".to_string(),
            types: vec![TypeDef {
                is_opaque: true,
                ..type_def("Thing", false)
            }],
            functions: vec![FunctionDef {
                name: "make_thing".to_string(),
                rust_path: "sample::make_thing".to_string(),
                return_type: TypeRef::Named("Thing".to_string()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };

        let native_methods = super::gen_native_methods(
            &api,
            "Sample",
            "sample",
            "sample",
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .expect("no trait bridges configured, so generation cannot fail");

        // `assert_stamped_before_hashing` only checks the marker string is present in the
        // hashed body — it proves nothing about what type the handle itself is declared as.
        // `native_methods.contains("IntPtr")` would pass here even on the pre-fix `IntPtr`
        // handle (and passes regardless, since `LastErrorContext`/`FreeString` are always
        // real pointers): the actual handle-typed declaration is asserted precisely below,
        // by `native_methods_declares_scalar_handles_not_pointers`. ~keep
        crate::backends::ffi::handle_abi_stamp::assert_stamped_before_hashing(
            &native_methods,
            "csharp NativeMethods.cs",
        );
    }

    /// Regression for the handle-ABI migration gap: a handle-typed return (opaque
    /// constructor) and its paired `_free` must be declared `ulong`, matching the FFI
    /// crate's `AlefHandle` (`uint64_t`), not `IntPtr`. Exact-string assertions so a future
    /// backslide back to `IntPtr` fails loudly instead of vacuously passing a `contains`
    /// check that a real pointer elsewhere in the file (e.g. `LastErrorContext`) would also
    /// satisfy.
    #[test]
    fn native_methods_declares_scalar_handles_not_pointers() {
        use crate::core::ir::{FunctionDef, MethodDef, ReceiverKind, TypeRef};
        use std::collections::{HashMap, HashSet};

        let api = ApiSurface {
            crate_name: "sample".to_string(),
            types: vec![TypeDef {
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "poke".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    return_type: TypeRef::Unit,
                    ..MethodDef::default()
                }],
                ..type_def("Thing", false)
            }],
            functions: vec![FunctionDef {
                name: "make_thing".to_string(),
                rust_path: "sample::make_thing".to_string(),
                return_type: TypeRef::Named("Thing".to_string()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };

        let native_methods = super::gen_native_methods(
            &api,
            "Sample",
            "sample",
            "sample",
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .expect("no trait bridges configured, so generation cannot fail");

        assert!(
            native_methods.contains("internal static extern ulong MakeThing();"),
            "an opaque-type-returning function must return the scalar AlefHandle:\n{native_methods}"
        );
        assert!(
            native_methods.contains("internal static extern void ThingFree(ulong ptr);"),
            "the paired free function must take the scalar AlefHandle, not IntPtr:\n{native_methods}"
        );
        // No trailing comma in the expectation: the receiver is pushed as `ulong handle,`
        // and the final `,\n` is truncated again before the closing paren, so a receiver
        // that is also the last parameter carries no comma. C# rejects a trailing comma in
        // a parameter list, so asserting one here would pin invalid output. ~keep
        assert!(
            native_methods.contains("ulong handle"),
            "a method receiver must be declared ulong, matching ReceiverKind -> AlefHandle:\n{native_methods}"
        );
        assert!(
            !native_methods.contains("IntPtr handle"),
            "no handle-typed declaration may remain IntPtr:\n{native_methods}"
        );
    }

    fn visitor_api() -> ApiSurface {
        use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

        ApiSurface {
            crate_name: "htm".to_string(),
            types: vec![type_def("ConversionOptions", false), type_def("HtmlVisitor", true)],
            functions: vec![FunctionDef {
                name: "convert".to_string(),
                rust_path: "htm::convert".to_string(),
                params: vec![ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ConversionOptions".to_string()),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::String,
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        }
    }

    fn native_methods_with_visitor_bridge(bridge: crate::core::config::TraitBridgeConfig) -> anyhow::Result<String> {
        native_methods_with_visitor_bridges(std::slice::from_ref(&bridge))
    }

    fn native_methods_with_visitor_bridges(
        bridges: &[crate::core::config::TraitBridgeConfig],
    ) -> anyhow::Result<String> {
        use std::collections::{HashMap, HashSet};

        let api = visitor_api();
        super::gen_native_methods(
            &api,
            "Htm",
            "htm_ffi",
            "htm",
            &HashSet::new(),
            &HashSet::new(),
            true,
            bridges,
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
    }

    fn options_field_bridge() -> crate::core::config::TraitBridgeConfig {
        crate::core::config::TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            type_alias: Some("VisitorHandle".to_string()),
            param_name: Some("visitor".to_string()),
            bind_via: crate::core::config::BridgeBinding::OptionsField,
            options_type: Some("ConversionOptions".to_string()),
            ..crate::core::config::TraitBridgeConfig::default()
        }
    }

    #[test]
    fn options_field_bridge_without_options_type_fails_generation() {
        let bridge = crate::core::config::TraitBridgeConfig {
            options_type: None,
            ..options_field_bridge()
        };

        let error = native_methods_with_visitor_bridge(bridge)
            .expect_err("a visitor bridge with no `options_type` must not silently fabricate one");
        let message = error.to_string();

        assert!(
            message.contains("HtmlVisitor") && message.contains("no `options_type`"),
            "error must name the offending bridge and the missing key: {message}"
        );
        assert!(
            message.contains("visitor_callbacks = false"),
            "error must name the sanctioned opt-out: {message}"
        );
    }

    #[test]
    fn options_field_bridge_without_a_resolvable_field_fails_generation() {
        let bridge = crate::core::config::TraitBridgeConfig {
            param_name: None,
            options_field: None,
            ..options_field_bridge()
        };

        let error = native_methods_with_visitor_bridge(bridge)
            .expect_err("a visitor bridge with no resolvable options field must not default to `visitor`");
        let message = error.to_string();

        assert!(
            message.contains("neither `options_field` nor `param_name`"),
            "error must name both keys that can supply the field: {message}"
        );
        assert!(
            message.contains("htm_options_set_<field>"),
            "error must show the entry point that cannot be derived: {message}"
        );
    }

    #[test]
    fn options_field_bridge_with_options_type_emits_the_configured_names() {
        let native_methods = native_methods_with_visitor_bridge(options_field_bridge())
            .expect("a fully configured options-field bridge generates");

        let declaration = format!(
            "internal static extern void ConversionOptionsSetVisitor({HANDLE_PINVOKE_TYPE} options, {HANDLE_PINVOKE_TYPE} visitor);"
        );
        assert!(
            native_methods.contains(&declaration),
            "the declaration must carry the configured `options_type`:\n{native_methods}"
        );
        assert!(
            native_methods.contains(r#"EntryPoint = "htm_options_set_visitor""#),
            "the entry point must be derived from the resolved options field:\n{native_methods}"
        );
        // `ConversionOptionsSetVisitor` contains the fabricated `Options` as a substring, so the
        // regression guard has to match the whole declaration, not the bare type name. ~keep
        assert!(
            !native_methods.contains("void OptionsSetVisitor("),
            "the fabricated `Options` type name must not reach the emitted declaration:\n{native_methods}"
        );
    }

    /// The FFI crate emits `{prefix}_options_set_{field}` once per options-field bridge
    /// (`ffi::gen_bindings::lib_rs.rs:558`) and `bridge_field_inject.jinja` emits a call per
    /// bridge, so a single `find` left the second bridge's call undeclared (CS0117). ~keep
    #[test]
    fn every_options_field_bridge_gets_its_own_setter_declaration() {
        let second = crate::core::config::TraitBridgeConfig {
            trait_name: "OutlineVisitor".to_string(),
            param_name: Some("outliner".to_string()),
            options_type: Some("OutlineOptions".to_string()),
            ..options_field_bridge()
        };
        let native_methods = native_methods_with_visitor_bridges(&[options_field_bridge(), second])
            .expect("two options-field bridges generate");

        for expected in [
            "ConversionOptionsSetVisitor(",
            "OutlineOptionsSetOutliner(",
            r#"EntryPoint = "htm_options_set_visitor""#,
            r#"EntryPoint = "htm_options_set_outliner""#,
        ] {
            assert!(
                native_methods.contains(expected),
                "`{expected}` must be declared once per bridge:\n{native_methods}"
            );
        }
        assert_eq!(
            native_methods.matches("VisitorCreate(").count(),
            1,
            "the FFI crate exports exactly one `visitor_create`:\n{native_methods}"
        );
    }

    /// Two bridges resolving to the same `{Options}Set{Field}` member must declare it once —
    /// a second declaration of the same member name is CS0111. ~keep
    #[test]
    fn duplicate_options_setters_collapse_to_one_declaration() {
        let duplicate = crate::core::config::TraitBridgeConfig {
            trait_name: "OutlineVisitor".to_string(),
            ..options_field_bridge()
        };
        let native_methods = native_methods_with_visitor_bridges(&[options_field_bridge(), duplicate])
            .expect("two bridges sharing an options field generate");

        assert_eq!(
            native_methods.matches("void ConversionOptionsSetVisitor(").count(),
            1,
            "the shared setter must be declared exactly once:\n{native_methods}"
        );
    }

    #[test]
    fn function_param_bridge_without_options_type_still_generates() {
        // Absence of `options_type` is legitimate — and the documented default — for
        // `bind_via = "function_param"`; only the options-field shape needs it. ~keep
        let bridge = crate::core::config::TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            param_name: Some("visitor".to_string()),
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            ..crate::core::config::TraitBridgeConfig::default()
        };

        let native_methods =
            native_methods_with_visitor_bridge(bridge).expect("function-param bridges never need `options_type`");

        assert!(
            !native_methods.contains("htm_options_set_") && !native_methods.contains("OptionsSetVisitor("),
            "no options setter belongs in a function-param bridge's declarations:\n{native_methods}"
        );
    }
}

#[cfg(test)]
mod handle_predicate_agreement_tests {
    use super::ffi_handle_type_names;
    use crate::backends::csharp::gen_bindings::marshalling::enum_names_with_data_variants;
    use crate::codegen::naming::csharp_type_name;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef};
    use std::collections::HashSet;

    /// AGREEMENT GUARD. "Which enums box as an `AlefHandle`" is one fact, and it is derived in
    /// two places that never meet: `ffi_handle_type_names` here decides which names survive into
    /// `opaque_return_types` (and therefore which `{Pascal}ToJson`/`{Pascal}Free` P/Invoke
    /// declarations are *emitted*), while `enum_names_with_data_variants` in `marshalling.rs`
    /// decides which returns `errors.rs` routes *through* that round trip. Each half is
    /// well-formed on its own, so a divergence produces no local failure — it produces C# that
    /// calls a `NativeMethods` member nobody declared (CS0117), which is exactly the half-landed
    /// state a prior fix (which filtered both predicates by variant shape) left behind for
    /// fieldless-only enums such as `liter-llm`'s `RefreshOutcome` (alef task #155). Both
    /// predicates now agree that *every* enum boxes as a handle, fieldless or data-carrying
    /// alike — comparing them is the only thing that can catch a future re-divergence. If this
    /// fails, change both predicates or neither. ~keep
    #[test]
    fn the_two_enum_handle_predicates_select_the_same_enums() {
        let api = ApiSurface {
            crate_name: "sample".to_string(),
            types: vec![
                TypeDef {
                    name: "RenderOptions".to_string(),
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "MarkupVisitor".to_string(),
                    is_trait: true,
                    ..TypeDef::default()
                },
            ],
            enums: vec![
                EnumDef {
                    name: "NodeKind".to_string(),
                    variants: vec![EnumVariant {
                        name: "Text".to_string(),
                        ..EnumVariant::default()
                    }],
                    ..EnumDef::default()
                },
                EnumDef {
                    name: "CrawlEvent".to_string(),
                    variants: vec![EnumVariant {
                        name: "Progress".to_string(),
                        fields: vec![FieldDef::default()],
                        ..EnumVariant::default()
                    }],
                    ..EnumDef::default()
                },
                EnumDef {
                    name: "GraphQlOutcome".to_string(),
                    variants: vec![
                        EnumVariant {
                            name: "Skipped".to_string(),
                            ..EnumVariant::default()
                        },
                        EnumVariant {
                            name: "Refreshed".to_string(),
                            fields: vec![FieldDef::default()],
                            ..EnumVariant::default()
                        },
                    ],
                    ..EnumDef::default()
                },
            ],
            ..ApiSurface::default()
        };

        let declared = enum_names_with_data_variants(&api);
        let emitted: HashSet<String> = ffi_handle_type_names(&api)
            .into_iter()
            .filter(|name| api.enums.iter().any(|enum_def| enum_def.name == *name))
            .map(csharp_type_name)
            .collect();

        assert_eq!(
            declared, emitted,
            "marshalling::enum_names_with_data_variants and functions::ffi_handle_type_names \
             disagree about which enums box as a handle; the C# emitted for the difference will \
             call an undeclared NativeMethods member"
        );
        assert!(
            declared.contains("CrawlEvent"),
            "the shared fixture must actually exercise a data-carrying enum, or this test compares \
             two empty sets and proves nothing: {declared:?}"
        );
        assert!(
            declared.contains("NodeKind"),
            "a fieldless-only enum must be in both sets too — it boxes as `AlefHandle` exactly \
             like a data-carrying enum: {declared:?}"
        );
        assert!(
            declared.contains("GraphQLOutcome") && !declared.contains("GraphQlOutcome"),
            "a mixed enum is data-carrying, and both sets must be keyed by the C# spelling \
             `errors.rs` actually looks up (`csharp_type_name`, which rewrites the `GraphQL` \
             initialism) rather than the raw IR name — otherwise the sets agree only because every \
             fixture name happens to map to itself: {declared:?}"
        );
    }
}
