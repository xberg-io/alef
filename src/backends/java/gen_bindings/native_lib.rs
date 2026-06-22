use crate::core::config::{AdapterPattern, BridgeBinding, ResolvedCrateConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::BTreeSet;

use super::marshal::{gen_ffi_layout_with_enums, gen_function_descriptor, is_bytes_result, is_ffi_string_return};

/// Returns true if the FFI backend exports a `{type_name}_to_json` symbol for this type.
/// This matches the predicate in `src/backends/ffi/gen_bindings/mod.rs`:
/// - Opaque types do NOT get `_to_json` (they are handles, not serializable values)
/// - Types without serde derives do NOT get `_to_json`
/// - Update types (ending with "Update") do NOT get `_to_json` (deserialize-only)
/// - Types with an existing `to_json` method do NOT get auto `_to_json` (method collision)
fn should_emit_to_json_handle(typ: &TypeDef) -> bool {
    !typ.is_opaque && typ.has_serde && !typ.name.ends_with("Update") && !typ.methods.iter().any(|m| m.name == "to_json")
}

/// Returns true if the FFI backend exports a `{type_name}_from_json` symbol for this type.
/// This matches the predicate in `src/backends/ffi/gen_bindings/mod.rs`:
/// - Opaque types do NOT get `_from_json` (they are handles, not serializable values)
/// - Types without serde derives do NOT get `_from_json`
/// - Types with an existing `from_json` method do NOT get auto `_from_json` (method collision)
fn should_emit_from_json_handle(typ: &TypeDef) -> bool {
    !typ.is_opaque && typ.has_serde && !typ.methods.iter().any(|m| m.name == "from_json")
}

/// Detection mirroring `is_bytes_result` for `MethodDef` — `Result<Vec<u8>>`-returning
/// methods use the (out_ptr, out_len, out_cap) triple FFI ABI.
fn is_bytes_result_method(method: &MethodDef) -> bool {
    if method.error_type.is_none() {
        return false;
    }
    matches!(method.return_type, TypeRef::Bytes)
        || matches!(&method.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes))
}

pub(crate) fn gen_native_lib(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    prefix: &str,
    has_visitor_pattern: bool,
) -> String {
    // Derive the native library name from the FFI output path (directory name with hyphens replaced
    // by underscores), falling back to `{ffi_prefix}_ffi`.
    let lib_name = config.ffi_lib_name();

    // Collect trait bridge handle names that will be emitted later, so we can skip them
    // in the functions loop (prevents duplicate handle emission with wrong descriptors).
    let trait_bridge_handles: AHashSet<String> = config
        .trait_bridges
        .iter()
        .filter(|b| {
            !b.exclude_languages
                .contains(&crate::core::config::Language::Java.to_string())
        })
        .flat_map(|b| {
            let trait_snake = b.trait_name.to_snake_case();
            let trait_upper = trait_snake.to_uppercase();
            let mut handles = vec![
                format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper),
                format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper),
            ];
            // clear_fn is emitted by the trait bridge layer; pre-register its handle name
            // so the functions loop skips it and avoids emitting a duplicate method handle.
            if let Some(clear_fn) = &b.clear_fn {
                handles.push(format!("{}_{}", prefix.to_uppercase(), clear_fn.to_uppercase()));
            }
            handles
        })
        .collect();

    // Collect bridge type aliases (e.g., "VisitorHandle") that should not get _from_json handles.
    // Bridge types are not real FFI types — they're trait wrapper handles that don't have
    // _from_json/_free functions in the FFI layer.
    let bridge_type_aliases: AHashSet<String> = config
        .trait_bridges
        .iter()
        .filter(|b| {
            !b.exclude_languages
                .contains(&crate::core::config::Language::Java.to_string())
        })
        .filter_map(|b| b.type_alias.clone())
        .collect();

    // Collect FFI-excluded function names so we can emit nullable handles for them.
    // Functions excluded from the FFI layer are still present in the IR (and thus appear
    // in the Java facade) but their native symbols are not compiled into the shared library.
    // Using orElse(null) prevents class initialization failure; callers must null-check before
    // invoking these handles.
    let ffi_excluded: AHashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    // Collect enum names so we can emit JAVA_INT layouts for enum params (Wave 2 FFI backend
    // emits enums as i32 discriminants, not pointers).
    let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Collect function handles
    let mut function_handles = Vec::new();

    // Generate method handles for free functions.
    // All functions get handles regardless of is_async — the FFI layer always exposes
    // synchronous C functions, and the Java async wrapper delegates to the sync method.
    for func in &api.functions {
        let handle_name = format!("{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

        // Skip if this function's handle will be emitted by trait bridge code (with correct descriptor).
        if trait_bridge_handles.contains(&handle_name) {
            continue;
        }

        let ffi_name = format!("{}_{}", prefix, func.name.to_lowercase());

        // Bytes-result functions use the out-param convention: JAVA_INT return +
        // 3 trailing ADDRESS/ADDRESS/ADDRESS params for (out_ptr: *mut *mut u8, out_len: *mut usize, out_cap: *mut usize).
        // For input Bytes parameters (in ALL functions), expand them to (ADDRESS pointer, JAVA_LONG length) pairs.
        let (return_layout, param_layouts) = if is_bytes_result(func) {
            let mut layouts: Vec<String> = Vec::new();
            for param in &func.params {
                match &param.ty {
                    TypeRef::Bytes => {
                        // Input byte slice: expand to pointer + length
                        layouts.push("ValueLayout.ADDRESS".to_string()); // pointer
                        layouts.push("ValueLayout.JAVA_LONG".to_string()); // length
                    }
                    TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => {
                        // Optional byte slice: expand to pointer + length
                        layouts.push("ValueLayout.ADDRESS".to_string());
                        layouts.push("ValueLayout.JAVA_LONG".to_string());
                    }
                    other => {
                        layouts.push(gen_ffi_layout_with_enums(other, &enum_names));
                    }
                }
            }
            layouts.push("ValueLayout.ADDRESS".to_string()); // out_ptr: *mut *mut u8
            layouts.push("ValueLayout.ADDRESS".to_string()); // out_len: *mut usize
            layouts.push("ValueLayout.ADDRESS".to_string()); // out_cap: *mut usize
            ("ValueLayout.JAVA_INT".to_string(), layouts)
        } else {
            let return_layout = gen_ffi_layout_with_enums(&func.return_type, &enum_names);
            // For non-bytes-result functions, still expand Bytes params to (ptr, len)
            let mut param_layouts: Vec<String> = Vec::new();
            for param in &func.params {
                match &param.ty {
                    TypeRef::Bytes => {
                        // Input byte slice: expand to pointer + length
                        param_layouts.push("ValueLayout.ADDRESS".to_string()); // pointer
                        param_layouts.push("ValueLayout.JAVA_LONG".to_string()); // length
                    }
                    TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => {
                        // Optional byte slice: expand to pointer + length
                        param_layouts.push("ValueLayout.ADDRESS".to_string());
                        param_layouts.push("ValueLayout.JAVA_LONG".to_string());
                    }
                    other => {
                        param_layouts.push(gen_ffi_layout_with_enums(other, &enum_names));
                    }
                }
            }
            (return_layout, param_layouts)
        };

        let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

        let handle_code = if ffi_excluded.contains(&func.name) {
            // Use orElse(null) for FFI-excluded functions — their native symbol may be absent.
            // Callers must null-check before invoking these handles.
            crate::backends::java::template_env::render(
                "method_handle_nullable.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            )
        } else {
            crate::backends::java::template_env::render(
                "method_handle_normal.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            )
        };
        function_handles.push(handle_code);

        if is_ffi_string_return(&func.return_type) {
            let len_handle_name = format!("{}_{}_LEN", prefix.to_uppercase(), func.name.to_uppercase());
            let len_ffi_name = format!("{}_{}_len", prefix, func.name.to_lowercase());
            let len_layout = gen_function_descriptor("ValueLayout.JAVA_LONG", &param_layouts);
            function_handles.push(crate::backends::java::template_env::render(
                "method_handle_len.jinja",
                minijinja::context! {
                    handle_name => len_handle_name,
                    ffi_name => len_ffi_name,
                    layout => len_layout,
                },
            ));
        }
    }

    // free_string handle for releasing FFI-allocated strings
    {
        let free_name = format!("{}_free_string", prefix);
        let handle_name = format!("{}_FREE_STRING", prefix.to_uppercase());
        let handle_code = crate::backends::java::template_env::render(
            "method_handle_free.jinja",
            minijinja::context! {
                handle_name => handle_name,
                ffi_name => free_name,
            },
        );
        function_handles.push(handle_code);
    }

    // free_bytes handle for releasing byte buffers returned via the out-param convention.
    // Signature: (ptr: ADDRESS, len: JAVA_LONG, cap: JAVA_LONG) -> void
    {
        let free_bytes_name = format!("{}_free_bytes", prefix);
        let handle_name = format!("{}_FREE_BYTES", prefix.to_uppercase());
        let handle_code = crate::backends::java::template_env::render(
            "method_handle_free_bytes.jinja",
            minijinja::context! {
                handle_name => handle_name,
                ffi_name => free_bytes_name,
            },
        );
        function_handles.push(handle_code);
    }

    // Error handling — use the FFI's last_error_code and last_error_context symbols
    // (Note: these are emitted inline in the template, not via function_handles)

    // Track emitted handles to avoid duplicates (a type may appear both as
    // a function return type AND as an opaque type, or as both return and parameter type).
    let mut emitted_free_handles: AHashSet<String> = AHashSet::new();
    // Same dedup for `_to_json` handles — when multiple functions return the
    // same Named type we'd otherwise emit the constant twice.
    let mut emitted_to_json_handles: AHashSet<String> = AHashSet::new();

    // Build maps for type classification to gate handle emission consistently with FFI backend.
    let opaque_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    // Serde-deriving enums are returned from the FFI as heap-allocated `*mut Enum` pointers and
    // expose a `{enum}_to_json` symbol (see `gen_enum_to_json` in the FFI backend, gated on
    // `has_serde` for any enum used as a Named return). Internally-tagged enums like
    // `ChunkingDecision` / `StructuredCallMode` reach the call-site `_TO_JSON` path in
    // `ffi_class::sync_functions` (which fires for every non-opaque Named return), so their
    // downcall handle must be enumerated here too — otherwise the generated Java facade calls a
    // `NativeLib` constant that was never declared, breaking compilation.
    let to_json_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| should_emit_to_json_handle(t))
        .map(|t| t.name.clone())
        .chain(api.enums.iter().filter(|e| e.has_serde).map(|e| e.name.clone()))
        .collect();
    let from_json_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| should_emit_from_json_handle(t))
        .map(|t| t.name.clone())
        .collect();

    // Collect accessor handles
    let mut accessor_handles = Vec::new();

    // Accessor handles for Named return types (struct pointer → field accessor + free).
    // Also handles `Option<Named>` return types — the FFI layer flattens nullable returns
    // to a raw pointer that's NULL when the optional is empty.
    for func in &api.functions {
        let inner_named = match &func.return_type {
            TypeRef::Named(n) => Some(n),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(name) = inner_named {
            let type_snake = name.to_snake_case();
            let type_upper = type_snake.to_uppercase();

            // Emit `_to_json` method handle only if the FFI backend exports one for this type.
            // Opaque types, types without serde, Update types, and types with a to_json method
            // do NOT get _to_json in the C FFI, so we skip the Java MethodHandle to prevent
            // NoSuchElementException at JVM clinit.
            if to_json_type_names.contains(name.as_str()) {
                let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
                let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
                if emitted_to_json_handles.insert(to_json_handle.clone()) {
                    let handle_code = crate::backends::java::template_env::render(
                        "method_handle_to_json.jinja",
                        minijinja::context! {
                            handle_name => to_json_handle,
                            ffi_name => to_json_ffi,
                        },
                    );
                    accessor_handles.push(handle_code);
                }
            }

            // _free: (struct_ptr) -> void
            let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_free.jinja",
                    minijinja::context! {
                        handle_name => free_handle,
                        ffi_name => free_ffi,
                    },
                );
                accessor_handles.push(handle_code);
            }
        }
    }

    // FROM_JSON + FREE handles for non-opaque Named types used as parameters.
    // These allow serializing a Java record to JSON and passing it to the FFI.
    //
    // Note: Even enums need _free here. `{prefix}_{type}_from_json` returns *mut T
    // (a heap-allocated pointer) regardless of whether T is an enum or struct, so the
    // matching _free is required to avoid leaking that allocation.
    //
    // We scan ALL functions (including ffi-excluded ones) because parameter type helpers
    // like _from_json/_free may be needed for the generated wrapper regardless of whether
    // the main function handle uses orElse(null). The dylib always exports these helpers
    // for parameter types that appear in non-excluded functions of the same type.
    let mut emitted_from_json_handles: AHashSet<String> = AHashSet::new();
    for func in &api.functions {
        for param in &func.params {
            // Handle both Named and Optional<Named> params
            let inner_name = match &param.ty {
                TypeRef::Named(n) => Some(n.clone()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(name) = inner_name {
                // Skip opaque types and bridge type aliases — these don't have _from_json/_free in the FFI backend.
                // Emit _from_json/_free for all other Named parameter types, regardless of whether they appear
                // in from_json_type_names. If the wrapper code references a type parameter, the FFI backend has
                // exported _from_json and _free for it.
                if !opaque_type_names.contains(name.as_str()) && !bridge_type_aliases.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    let type_upper = type_snake.to_uppercase();

                    // Emit _from_json: (char*) -> struct_ptr — emit for all non-opaque/non-bridge parameter types,
                    // because ffi_class.rs::gen_sync_function_method will invoke it to marshal parameter values.
                    let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
                    let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
                    if emitted_from_json_handles.insert(from_json_handle.clone()) {
                        let handle_code = crate::backends::java::template_env::render(
                            "method_handle_from_json.jinja",
                            minijinja::context! {
                                handle_name => from_json_handle,
                                ffi_name => from_json_ffi,
                            },
                        );
                        accessor_handles.push(handle_code);
                    }

                    // _free: (struct_ptr) -> void — emit for ALL non-opaque/non-bridge parameter types,
                    // because ffi_class.rs::gen_sync_function_method will invoke it to clean up parameter allocations.
                    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                    let free_ffi = format!("{}_{}_free", prefix, type_snake);
                    if emitted_free_handles.insert(free_handle.clone()) {
                        let handle_code = crate::backends::java::template_env::render(
                            "method_handle_free.jinja",
                            minijinja::context! {
                                handle_name => free_handle,
                                ffi_name => free_ffi,
                            },
                        );
                        accessor_handles.push(handle_code);
                    }
                }
            }
        }
    }

    // Collect builder class names from record types with defaults, so we skip
    // opaque types that are superseded by a pure-Java builder class.
    let builder_class_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !t.fields.is_empty() && t.has_default)
        .map(|t| format!("{}Builder", t.name))
        .collect();

    // Collect builder handles
    let mut builder_handles = Vec::new();

    // Free handles for opaque types (handle pointer → void), plus _new constructor handles
    // for opaque types that have static factory methods.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque && !builder_class_names.contains(&typ.name) {
            let type_snake = typ.name.to_snake_case();
            let type_upper = type_snake.to_uppercase();
            let prefix_upper = prefix.to_uppercase();

            // Free handle
            let free_handle = format!("{}_{}_FREE", prefix_upper, type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_free.jinja",
                    minijinja::context! {
                        handle_name => free_handle,
                        ffi_name => free_ffi,
                    },
                );
                builder_handles.push(handle_code);
            }

            // Note: opaque static `new` constructors emit their MethodHandle via the
            // method-handle loop further below (per-method, per-opaque-type — which
            // already produces `{PREFIX}_{TYPE}_NEW` for a `new` method and includes
            // its parameter layouts). Emitting a separate handle here would duplicate
            // that and ALSO collide on any second static-returning-Self method like
            // `default()` (both would resolve to `{PREFIX}_{TYPE}_NEW`).
        }
    }

    // Collect trait handles
    let mut trait_handles = Vec::new();
    let mut emitted_register_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_unregister_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_clear_handles: AHashSet<String> = AHashSet::new();

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .contains(&crate::core::config::Language::Java.to_string())
        {
            continue;
        }

        let trait_snake = bridge_cfg.trait_name.to_snake_case();
        let trait_upper = trait_snake.to_uppercase();
        // For wide vtables, wrap the field list across multiple lines so the surrounding
        // `LINKER.downcallHandle(...)` line stays under the checkstyle 200-char limit.
        // Always pass the vtable as a pointer (ValueLayout.ADDRESS) rather than by value.
        //
        // On ARM64 the System V / AAPCS64 ABI requires structs larger than 16 bytes to
        // be passed via an invisible pointer inserted by the caller. Java Panama FFM does
        // not apply that invisible-pointer promotion automatically for structs expressed as
        // MemoryLayout.structLayout(...), so passing a large vtable by value causes a
        // SIGSEGV when Rust reads a misaligned / garbage struct on the callee side.
        //
        // The Rust FFI layer now accepts `*const XxxVTable` (a pointer) for all
        // `register_*` functions, making the parameter a single ADDRESS regardless of vtable
        // size. All language bindings were updated accordingly.
        let vtable_layout = "ValueLayout.ADDRESS".to_string();

        // Register handle
        let register_handle_name = format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper);
        let register_ffi_name = format!("{}_register_{}", prefix, trait_snake);
        if emitted_register_handles.insert(register_handle_name.clone()) {
            // Use orElse(null): the register symbol may be absent when the trait bridge
            // is not compiled into the dylib. Callers must null-check before invoking.
            let handle_code = crate::backends::java::template_env::render(
                "method_handle_register.jinja",
                minijinja::context! {
                    handle_name => register_handle_name,
                    ffi_name => register_ffi_name,
                    vtable_layout => &vtable_layout,
                },
            );
            trait_handles.push(handle_code);
        }

        // Unregister handle — only emitted when unregister_fn is configured.
        if bridge_cfg.unregister_fn.is_some() {
            let unregister_handle_name = format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper);
            let unregister_ffi_name = format!("{}_unregister_{}", prefix, trait_snake);
            if emitted_unregister_handles.insert(unregister_handle_name.clone()) {
                // Use orElse(null): the unregister symbol may be absent when the trait bridge
                // is not compiled into the dylib. Callers must null-check before invoking.
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_unregister.jinja",
                    minijinja::context! {
                        handle_name => unregister_handle_name,
                        ffi_name => unregister_ffi_name,
                    },
                );
                trait_handles.push(handle_code);
            }
        }

        // Clear handle — only emitted when clear_fn is configured.
        if bridge_cfg.clear_fn.is_some() {
            let clear_handle_name = format!("{}_CLEAR_{}", prefix.to_uppercase(), trait_upper);
            let clear_ffi_name = format!("{}_clear_{}", prefix, trait_snake);
            if emitted_clear_handles.insert(clear_handle_name.clone()) {
                // Use orElse(null): the clear symbol may be absent when the trait bridge
                // is not compiled into the dylib. Callers must null-check before invoking.
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_clear.jinja",
                    minijinja::context! {
                        handle_name => clear_handle_name,
                        ffi_name => clear_ffi_name,
                    },
                );
                trait_handles.push(handle_code);
            }
        }
    }

    // Collect all stream item and request types from adapters. Even if a type's has_serde
    // is false in the IR (due to cfg gating), if it's an adapter item/request type, the FFI
    // backend MUST export _to_json/_from_json symbols for them — streaming types are always
    // serializable by contract.
    let stream_item_types: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| a.item_type.as_ref())
        .cloned()
        .collect();
    let stream_request_types: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| a.params.first().map(|p| p.ty.as_str()))
        .filter_map(|ty| ty.rsplit("::").next()) // Strip module path
        .map(|s| s.to_string())
        .collect();

    // Streaming-adapter method handles. For each `[[crates.adapters]]` entry with
    // pattern = "streaming", emit three downcall handles for the FFI iterator-handle
    // functions (`_start`, `_next`, `_free`) plus the request `_from_json`/`_free`
    // and the chunk-item `_to_json`/`_free` accessors needed to drive the iterator
    // from Java. The Java public method is emitted on the owner opaque handle class.
    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        let Some(owner_type) = adapter.owner_type.as_deref() else {
            continue;
        };
        let Some(item_type) = adapter.item_type.as_deref() else {
            continue;
        };
        let Some(request_type) = adapter.params.first().map(|p| p.ty.as_str()).filter(|s| !s.is_empty()) else {
            continue;
        };

        let owner_snake = owner_type.to_snake_case();
        let owner_upper = owner_snake.to_uppercase();
        let adapter_snake = adapter.name.to_snake_case();
        let adapter_upper = adapter_snake.to_uppercase();
        let prefix_upper = prefix.to_uppercase();

        // _start: (client_ptr, request_ptr) -> stream_handle_ptr
        let start_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_START");
        let start_ffi = format!("{prefix}_{owner_snake}_{adapter_snake}_start");
        let start_layout =
            "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)".to_string();
        accessor_handles.push(crate::backends::java::template_env::render(
            "method_handle_normal.jinja",
            minijinja::context! {
                handle_name => start_handle,
                ffi_name => start_ffi,
                layout => start_layout,
            },
        ));

        // _next: (stream_handle_ptr) -> item_ptr
        let next_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_NEXT");
        let next_ffi = format!("{prefix}_{owner_snake}_{adapter_snake}_next");
        let next_layout = "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)".to_string();
        accessor_handles.push(crate::backends::java::template_env::render(
            "method_handle_normal.jinja",
            minijinja::context! {
                handle_name => next_handle,
                ffi_name => next_ffi,
                layout => next_layout,
            },
        ));

        // _free: (stream_handle_ptr) -> void
        let free_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_FREE");
        let free_ffi = format!("{prefix}_{owner_snake}_{adapter_snake}_free");
        accessor_handles.push(crate::backends::java::template_env::render(
            "method_handle_free.jinja",
            minijinja::context! {
                handle_name => free_handle,
                ffi_name => free_ffi,
            },
        ));

        // Request type _from_json + _free (used to marshal the request POJO into a
        // pointer the FFI iterator-start expects).
        let request_snake = request_type.to_snake_case();
        let request_upper = request_snake.to_uppercase();

        // Emit if the FFI backend exports a _from_json for this type, OR if it's a stream request type.
        // Stream request types are always serializable by contract, even if has_serde is false in IR
        // (due to cfg gating); the FFI backend guarantees a _from_json symbol for all stream requests.
        if from_json_type_names.contains(request_type) || stream_request_types.contains(request_type) {
            let req_from_json_handle = format!("{prefix_upper}_{request_upper}_FROM_JSON");
            let req_from_json_ffi = format!("{prefix}_{request_snake}_from_json");
            if emitted_from_json_handles.insert(req_from_json_handle.clone()) {
                accessor_handles.push(crate::backends::java::template_env::render(
                    "method_handle_from_json.jinja",
                    minijinja::context! {
                        handle_name => req_from_json_handle,
                        ffi_name => req_from_json_ffi,
                    },
                ));
            }
        }
        let req_free_handle = format!("{prefix_upper}_{request_upper}_FREE");
        let req_free_ffi = format!("{prefix}_{request_snake}_free");
        if emitted_free_handles.insert(req_free_handle.clone()) {
            accessor_handles.push(crate::backends::java::template_env::render(
                "method_handle_free.jinja",
                minijinja::context! {
                    handle_name => req_free_handle,
                    ffi_name => req_free_ffi,
                },
            ));
        }

        // Item type _to_json + _free (used to deserialize each chunk pointer back
        // into a Java record).
        let item_snake = item_type.to_snake_case();
        let item_upper = item_snake.to_uppercase();

        // Emit if the FFI backend exports a _to_json for this type, OR if it's a stream item type.
        // Stream item types are always serializable by contract, even if has_serde is false in IR
        // (due to cfg gating); the FFI backend guarantees a _to_json symbol for all stream items.
        if to_json_type_names.contains(item_type) || stream_item_types.contains(item_type) {
            let item_to_json_handle = format!("{prefix_upper}_{item_upper}_TO_JSON");
            let item_to_json_ffi = format!("{prefix}_{item_snake}_to_json");
            if emitted_to_json_handles.insert(item_to_json_handle.clone()) {
                accessor_handles.push(crate::backends::java::template_env::render(
                    "method_handle_to_json.jinja",
                    minijinja::context! {
                        handle_name => item_to_json_handle,
                        ffi_name => item_to_json_ffi,
                    },
                ));
            }
        }
        let item_free_handle = format!("{prefix_upper}_{item_upper}_FREE");
        let item_free_ffi = format!("{prefix}_{item_snake}_free");
        if emitted_free_handles.insert(item_free_handle.clone()) {
            accessor_handles.push(crate::backends::java::template_env::render(
                "method_handle_free.jinja",
                minijinja::context! {
                    handle_name => item_free_handle,
                    ffi_name => item_free_ffi,
                },
            ));
        }
    }

    // Method handles for instance methods on opaque types (chat, embed, moderate, …).
    //
    // Each FFI export is named `{prefix}_{owner_snake}_{method_snake}` and takes the
    // opaque receiver pointer as its first argument. Streaming-adapter methods are
    // excluded — those use the (`_start`, `_next`, `_free`) iterator-handle trio above.
    // Bytes-result methods use the (out_ptr, out_len, out_cap) triple convention,
    // mirroring `is_bytes_result` for free functions.
    //
    // NOTE: This loop only processes NON-OPAQUE types. Opaque types do NOT have FFI
    // method exports (neither instance nor static), so method handles are not generated
    // for them. Their Java wrappers in gen_opaque_handle_class are pure Java code with
    // no FFI calls.
    let streaming_adapter_method_keys: AHashSet<(String, String)> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| {
            let owner = a.owner_type.clone()?;
            Some((owner, a.name.to_snake_case()))
        })
        .collect();
    for typ in api.types.iter().filter(|t| t.is_opaque && !t.is_trait) {
        for method in &typ.methods {
            if streaming_adapter_method_keys.contains(&(typ.name.clone(), method.name.to_snake_case())) {
                continue;
            }
            // The FFI backend never exports `_default` / `_to_json` / `_from_json` for opaque
            // types — those C functions only exist for non-opaque, serde-derivable, non-Update
            // value types. Emitting a MethodHandle for them here would make `LIB.find(...)`
            // throw `NoSuchElementException` at JVM clinit. Mirror the FFI's omission.
            if matches!(method.name.as_str(), "default" | "to_json" | "from_json") {
                continue;
            }
            // The FFI backend never exports a symbol for a static method that returns a
            // borrowed reference to its own opaque type (e.g. `Registry::global() ->
            // &'static Registry`) — a borrow cannot be boxed into an owned `*mut T` handle.
            // Emitting an eager MethodHandle here would make `LIB.find(...)` throw at JVM
            // clinit (ExceptionInInitializerError). Mirror the FFI's omission.
            if method.returns_ref_to_owner(&typ.name) {
                continue;
            }
            let owner_snake = typ.name.to_snake_case();
            let owner_upper = owner_snake.to_uppercase();
            let method_snake = method.name.to_snake_case();
            let method_upper = method_snake.to_uppercase();
            let handle_name = format!("{}_{}_{}", prefix.to_uppercase(), owner_upper, method_upper);
            let ffi_name = format!("{}_{}_{}", prefix, owner_snake, method_snake);

            // Instance methods carry the receiver pointer as the first FFI param;
            // static factory methods do not. The static-factory case still needs a
            // MethodHandle for the same FFI symbol — see `gen_static_factory_method`
            // in `types.rs` which references this handle.
            let mut param_layouts: Vec<String> = if method.is_static {
                Vec::new()
            } else {
                vec!["ValueLayout.ADDRESS".to_string()]
            };
            // For method parameters, expand Bytes to (ptr, len) pairs
            for p in &method.params {
                match &p.ty {
                    TypeRef::Bytes => {
                        // Input byte slice: expand to pointer + length
                        param_layouts.push("ValueLayout.ADDRESS".to_string()); // pointer
                        param_layouts.push("ValueLayout.JAVA_LONG".to_string()); // length
                    }
                    TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => {
                        // Optional byte slice: expand to pointer + length
                        param_layouts.push("ValueLayout.ADDRESS".to_string());
                        param_layouts.push("ValueLayout.JAVA_LONG".to_string());
                    }
                    other => {
                        param_layouts.push(gen_ffi_layout_with_enums(other, &enum_names));
                    }
                }
            }
            let return_layout = if is_bytes_result_method(method) {
                param_layouts.push("ValueLayout.ADDRESS".to_string()); // out_ptr
                param_layouts.push("ValueLayout.ADDRESS".to_string()); // out_len
                param_layouts.push("ValueLayout.ADDRESS".to_string()); // out_cap
                "ValueLayout.JAVA_INT".to_string()
            } else {
                gen_ffi_layout_with_enums(&method.return_type, &enum_names)
            };
            let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

            let handle_code = crate::backends::java::template_env::render(
                "method_handle_normal.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            );
            function_handles.push(handle_code);

            // For Named return types, ensure the response struct's `_to_json` and `_free`
            // helpers are registered so the instance-method body can deserialize and free.
            let return_named = match &method.return_type {
                TypeRef::Named(n) => Some(n.clone()),
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => Some(n.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(name) = return_named {
                let type_snake = name.to_snake_case();
                let type_upper = type_snake.to_uppercase();

                // Emit `_to_json` handle only if the FFI backend exports one for this type.
                if to_json_type_names.contains(&name) {
                    let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
                    let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
                    if emitted_to_json_handles.insert(to_json_handle.clone()) {
                        accessor_handles.push(crate::backends::java::template_env::render(
                            "method_handle_to_json.jinja",
                            minijinja::context! {
                                handle_name => to_json_handle,
                                ffi_name => to_json_ffi,
                            },
                        ));
                    }
                }
                // Always emit free handle, even if _to_json wasn't emitted
                let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                let free_ffi = format!("{}_{}_free", prefix, type_snake);
                if emitted_free_handles.insert(free_handle.clone()) {
                    accessor_handles.push(crate::backends::java::template_env::render(
                        "method_handle_free.jinja",
                        minijinja::context! {
                            handle_name => free_handle,
                            ffi_name => free_ffi,
                        },
                    ));
                }
            }

            // For Named param types, register their `_from_json` + `_free` helpers.
            for p in &method.params {
                let param_named = match &p.ty {
                    TypeRef::Named(n) => Some(n.clone()),
                    TypeRef::Optional(inner) => match inner.as_ref() {
                        TypeRef::Named(n) => Some(n.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = param_named {
                    // Skip bridge type aliases and types without serde —
                    // these don't have _from_json/_free in the FFI backend.
                    if !bridge_type_aliases.contains(name.as_str()) && from_json_type_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        let type_upper = type_snake.to_uppercase();
                        let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
                        let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
                        if emitted_from_json_handles.insert(from_json_handle.clone()) {
                            accessor_handles.push(crate::backends::java::template_env::render(
                                "method_handle_from_json.jinja",
                                minijinja::context! {
                                    handle_name => from_json_handle,
                                    ffi_name => from_json_ffi,
                                },
                            ));
                        }
                        let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                        let free_ffi = format!("{}_{}_free", prefix, type_snake);
                        if emitted_free_handles.insert(free_handle.clone()) {
                            accessor_handles.push(crate::backends::java::template_env::render(
                                "method_handle_free.jinja",
                                minijinja::context! {
                                    handle_name => free_handle,
                                    ffi_name => free_ffi,
                                },
                            ));
                        }
                    }
                }
            }
        }
    }

    // Generate visitor FFI method handles when a trait bridge is configured.
    let visitor_handles = if has_visitor_pattern {
        let options_fields: Vec<String> = config
            .trait_bridges
            .iter()
            .filter(|bridge| bridge.bind_via == BridgeBinding::OptionsField)
            .filter_map(|bridge| bridge.resolved_options_field().map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        crate::backends::java::gen_visitor::gen_native_lib_visitor_handles(prefix, &options_fields)
    } else {
        String::new()
    };

    // Generate the class body first using the template
    let class_body = crate::backends::java::template_env::render(
        "native_lib.jinja",
        minijinja::context! {
            class_name => "NativeLib",
            lib_name => lib_name,
            prefix => prefix,
            prefix_upper => prefix.to_uppercase(),
            function_handles => function_handles,
            accessor_handles => accessor_handles,
            builder_handles => builder_handles,
            trait_handles => trait_handles,
            visitor_handles => visitor_handles,
        },
    );

    // Now assemble the file with the necessary imports
    let mut out = String::with_capacity(class_body.len() + 512);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("package ");
    out.push_str(package);
    out.push_str(";\n\n");

    // Add imports based on what's in the generated class body
    if class_body.contains("Arena") {
        out.push_str("import java.lang.foreign.Arena;\n");
    }
    if class_body.contains("FunctionDescriptor") {
        out.push_str("import java.lang.foreign.FunctionDescriptor;\n");
    }
    if class_body.contains("Linker") {
        out.push_str("import java.lang.foreign.Linker;\n");
    }
    if class_body.contains("MemoryLayout") {
        out.push_str("import java.lang.foreign.MemoryLayout;\n");
    }
    if class_body.contains("MemorySegment") {
        out.push_str("import java.lang.foreign.MemorySegment;\n");
    }
    if class_body.contains("SymbolLookup") {
        out.push_str("import java.lang.foreign.SymbolLookup;\n");
    }
    if class_body.contains("ValueLayout") {
        out.push_str("import java.lang.foreign.ValueLayout;\n");
    }
    if class_body.contains("MethodHandle") {
        out.push_str("import java.lang.invoke.MethodHandle;\n");
    }
    // Imports required by the JAR-extraction native loader (always present).
    out.push_str("import java.io.File;\n");
    out.push_str("import java.net.URL;\n");
    out.push_str("import java.nio.file.Files;\n");
    out.push_str("import java.nio.file.Path;\n");
    out.push_str("import java.nio.file.Paths;\n");
    out.push_str("import java.nio.file.StandardCopyOption;\n");
    out.push_str("import java.util.ArrayList;\n");
    out.push_str("import java.util.Enumeration;\n");
    out.push_str("import java.util.List;\n");
    out.push_str("import java.util.jar.JarEntry;\n");
    out.push_str("import java.util.jar.JarFile;\n");
    out.push('\n');

    out.push_str(&class_body);

    out
}
