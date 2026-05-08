use ahash::AHashSet;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, TypeRef};
use heck::ToSnakeCase;

use super::marshal::{gen_ffi_layout, gen_function_descriptor, is_bytes_result};

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
                .contains(&alef_core::config::Language::Java.to_string())
        })
        .flat_map(|b| {
            let trait_snake = b.trait_name.to_snake_case();
            let trait_upper = trait_snake.to_uppercase();
            vec![
                format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper),
                format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper),
            ]
        })
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
        // For input Bytes parameters, expand them to (ADDRESS pointer, JAVA_LONG length) pairs.
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
                        layouts.push(gen_ffi_layout(other));
                    }
                }
            }
            layouts.push("ValueLayout.ADDRESS".to_string()); // out_ptr: *mut *mut u8
            layouts.push("ValueLayout.ADDRESS".to_string()); // out_len: *mut usize
            layouts.push("ValueLayout.ADDRESS".to_string()); // out_cap: *mut usize
            ("ValueLayout.JAVA_INT".to_string(), layouts)
        } else {
            let return_layout = gen_ffi_layout(&func.return_type);
            let param_layouts: Vec<String> = func.params.iter().map(|p| gen_ffi_layout(&p.ty)).collect();
            (return_layout, param_layouts)
        };

        let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

        let handle_code = if ffi_excluded.contains(&func.name) {
            // Use orElse(null) for FFI-excluded functions — their native symbol may be absent.
            // Callers must null-check before invoking these handles.
            crate::template_env::render(
                "method_handle_nullable.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            )
        } else {
            crate::template_env::render(
                "method_handle_normal.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            )
        };
        function_handles.push(handle_code);
    }

    // free_string handle for releasing FFI-allocated strings
    {
        let free_name = format!("{}_free_string", prefix);
        let handle_name = format!("{}_FREE_STRING", prefix.to_uppercase());
        let handle_code = crate::template_env::render(
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
        let handle_code = crate::template_env::render(
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

    // Build the set of opaque type names so we can pick the right accessor below.
    let opaque_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
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
            let _is_opaque = opaque_type_names.contains(name.as_str());

            // Emit `_to_json` method handle whenever the FFI exposes one for this type.
            // Both opaque and non-opaque types may have a `_to_json` exporter — the Java
            // wrapper code uses it to serialize for inspection (e.g. `EmbeddingPreset`).
            // We use `LIB.find(...).map(...).orElse(null)` so generation is robust if the
            // function is absent in this build (compile-time presence isn't always guaranteed).
            let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
            let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
            if emitted_to_json_handles.insert(to_json_handle.clone()) {
                let handle_code = crate::template_env::render(
                    "method_handle_to_json.jinja",
                    minijinja::context! {
                        handle_name => to_json_handle,
                        ffi_name => to_json_ffi,
                    },
                );
                accessor_handles.push(handle_code);
            }

            // _free: (struct_ptr) -> void
            let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                let handle_code = crate::template_env::render(
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
                if !opaque_type_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    let type_upper = type_snake.to_uppercase();

                    // _from_json: (char*) -> struct_ptr
                    let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
                    let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
                    if emitted_from_json_handles.insert(from_json_handle.clone()) {
                        let handle_code = crate::template_env::render(
                            "method_handle_from_json.jinja",
                            minijinja::context! {
                                handle_name => from_json_handle,
                                ffi_name => from_json_ffi,
                            },
                        );
                        accessor_handles.push(handle_code);
                    }

                    // _free: (struct_ptr) -> void
                    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                    let free_ffi = format!("{}_{}_free", prefix, type_snake);
                    if emitted_free_handles.insert(free_handle.clone()) {
                        let handle_code = crate::template_env::render(
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

    // Free handles for opaque types (handle pointer → void)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque && !builder_class_names.contains(&typ.name) {
            let type_snake = typ.name.to_snake_case();
            let type_upper = type_snake.to_uppercase();
            let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                let handle_code = crate::template_env::render(
                    "method_handle_free.jinja",
                    minijinja::context! {
                        handle_name => free_handle,
                        ffi_name => free_ffi,
                    },
                );
                builder_handles.push(handle_code);
            }
        }
    }

    // Collect trait handles
    let mut trait_handles = Vec::new();
    let mut emitted_register_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_unregister_handles: AHashSet<String> = AHashSet::new();

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .contains(&alef_core::config::Language::Java.to_string())
        {
            continue;
        }

        let trait_snake = bridge_cfg.trait_name.to_snake_case();
        let trait_upper = trait_snake.to_uppercase();

        // Register handle
        let register_handle_name = format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper);
        let register_ffi_name = format!("{}_register_{}", prefix, trait_snake);
        if emitted_register_handles.insert(register_handle_name.clone()) {
            // Use orElse(null): the register symbol may be absent when the trait bridge
            // is not compiled into the dylib. Callers must null-check before invoking.
            let handle_code = crate::template_env::render(
                "method_handle_register.jinja",
                minijinja::context! {
                    handle_name => register_handle_name,
                    ffi_name => register_ffi_name,
                },
            );
            trait_handles.push(handle_code);
        }

        // Unregister handle
        let unregister_handle_name = format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper);
        let unregister_ffi_name = format!("{}_unregister_{}", prefix, trait_snake);
        if emitted_unregister_handles.insert(unregister_handle_name.clone()) {
            // Use orElse(null): the unregister symbol may be absent when the trait bridge
            // is not compiled into the dylib. Callers must null-check before invoking.
            let handle_code = crate::template_env::render(
                "method_handle_unregister.jinja",
                minijinja::context! {
                    handle_name => unregister_handle_name,
                    ffi_name => unregister_ffi_name,
                },
            );
            trait_handles.push(handle_code);
        }
    }

    // Generate visitor FFI method handles when a trait bridge is configured.
    let visitor_handles = if has_visitor_pattern {
        crate::gen_visitor::gen_native_lib_visitor_handles(prefix)
    } else {
        String::new()
    };

    // Generate the class body first using the template
    let class_body = crate::template_env::render(
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
