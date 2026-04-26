use crate::type_map::{java_boxed_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_java_name;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write;

use super::helpers::is_bridge_param_java;
use super::marshal::{
    ffi_param_name, gen_helper_methods, is_ffi_string_return, java_ffi_return_cast, marshal_param_to_ffi,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_main_class(
    api: &ApiSurface,
    _config: &AlefConfig,
    package: &str,
    class_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
) -> String {
    // Build the set of opaque type names so we can distinguish opaque handles from records
    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Generate the class body first, then scan it to determine which imports are needed.
    let mut body = String::with_capacity(4096);

    writeln!(body, "public final class {} {{", class_name).ok();
    writeln!(body, "    private {}() {{ }}", class_name).ok();
    writeln!(body).ok();

    // Generate static methods for free functions
    for func in &api.functions {
        // Always generate sync method (bridge params stripped from signature)
        gen_sync_function_method(
            &mut body,
            func,
            prefix,
            class_name,
            &opaque_types,
            bridge_param_names,
            bridge_type_aliases,
        );
        writeln!(body).ok();

        // Also generate async wrapper if marked as async
        if func.is_async {
            gen_async_wrapper_method(&mut body, func, bridge_param_names, bridge_type_aliases);
            writeln!(body).ok();
        }
    }

    // Inject convertWithVisitor when a visitor bridge is configured.
    if has_visitor_bridge {
        body.push_str(&crate::gen_visitor::gen_convert_with_visitor_method(class_name, prefix));
        writeln!(body).ok();
    }

    // Add helper methods only if they are referenced in the body
    gen_helper_methods(&mut body, prefix, class_name);

    writeln!(body, "}}").ok();

    // Now assemble the file with only the imports that are actually used in the body.
    let mut out = String::with_capacity(body.len() + 512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    if body.contains("Arena") {
        writeln!(out, "import java.lang.foreign.Arena;").ok();
    }
    if body.contains("FunctionDescriptor") {
        writeln!(out, "import java.lang.foreign.FunctionDescriptor;").ok();
    }
    if body.contains("Linker") {
        writeln!(out, "import java.lang.foreign.Linker;").ok();
    }
    if body.contains("MemorySegment") {
        writeln!(out, "import java.lang.foreign.MemorySegment;").ok();
    }
    if body.contains("SymbolLookup") {
        writeln!(out, "import java.lang.foreign.SymbolLookup;").ok();
    }
    if body.contains("ValueLayout") {
        writeln!(out, "import java.lang.foreign.ValueLayout;").ok();
    }
    if body.contains("List<") {
        writeln!(out, "import java.util.List;").ok();
    }
    if body.contains("Map<") {
        writeln!(out, "import java.util.Map;").ok();
    }
    if body.contains("Optional<") {
        writeln!(out, "import java.util.Optional;").ok();
    }
    if body.contains("HashMap<") || body.contains("new HashMap") {
        writeln!(out, "import java.util.HashMap;").ok();
    }
    if body.contains("CompletableFuture") {
        writeln!(out, "import java.util.concurrent.CompletableFuture;").ok();
    }
    if body.contains("CompletionException") {
        writeln!(out, "import java.util.concurrent.CompletionException;").ok();
    }
    // Only import the short name `ObjectMapper` when it's used as a type reference (not just via
    // `createObjectMapper()` which uses fully qualified names internally).
    // Check for " ObjectMapper" (space before) which indicates use as a type, not a method name suffix.
    if body.contains(" ObjectMapper") {
        writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper;").ok();
    }
    writeln!(out).ok();

    out.push_str(&body);

    out
}

pub(crate) fn gen_sync_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    // Exclude bridge params from the public Java signature.
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = java_type(&p.ty);
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = java_type(&func.return_type);

    writeln!(
        out,
        "    public static {} {}({}) throws {}Exception {{",
        return_type,
        to_java_name(&func.name),
        params.join(", "),
        class_name
    )
    .ok();

    writeln!(out, "        try (var arena = Arena.ofConfined()) {{").ok();

    // Collect non-opaque Named params that need FFI pointer cleanup after the call.
    // These are Rust-allocated by _from_json and must be freed with _free.
    // Bridge params are excluded — they are passed as NULL.
    let ffi_ptr_params: Vec<(String, String)> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .filter_map(|p| {
            let inner_name = match &p.ty {
                TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => Some(n.clone()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if !opaque_types.contains(n.as_str()) {
                            Some(n.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            inner_name.map(|type_name| {
                let cname = "c".to_string() + &to_java_name(&p.name);
                let type_snake = type_name.to_snake_case();
                let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
                (cname, free_handle)
            })
        })
        .collect();

    // Marshal non-bridge parameters (use camelCase Java names)
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        marshal_param_to_ffi(out, &to_java_name(&param.name), &param.ty, opaque_types, prefix);
    }

    // Call FFI
    let ffi_handle = format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

    // Build call args: bridge params get MemorySegment.NULL, others are marshalled normally.
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if is_bridge_param_java(p, bridge_param_names, bridge_type_aliases) {
                "MemorySegment.NULL".to_string()
            } else {
                ffi_param_name(&to_java_name(&p.name), &p.ty, opaque_types)
            }
        })
        .collect();

    // Emit a helper closure to free FFI-allocated param pointers (e.g. options created by _from_json)
    let emit_ffi_ptr_cleanup = |out: &mut String| {
        for (cname, free_handle) in &ffi_ptr_params {
            writeln!(out, "            if (!{}.equals(MemorySegment.NULL)) {{", cname).ok();
            writeln!(out, "                {}.invoke({});", free_handle, cname).ok();
            writeln!(out, "            }}").ok();
        }
    };

    if matches!(func.return_type, TypeRef::Unit) {
        writeln!(out, "            {}.invoke({});", ffi_handle, call_args.join(", ")).ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if is_ffi_string_return(&func.return_type) {
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
        writeln!(out, "                checkLastError();").ok();
        writeln!(out, "                return null;").ok();
        writeln!(out, "            }}").ok();
        writeln!(
            out,
            "            String result = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        writeln!(out, "            return result;").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(func.return_type, TypeRef::Named(_)) {
        // Named return types: FFI returns a struct pointer.
        let return_type_name = match &func.return_type {
            TypeRef::Named(name) => name,
            _ => unreachable!(),
        };
        let is_opaque = opaque_types.contains(return_type_name.as_str());

        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
        writeln!(out, "                checkLastError();").ok();
        writeln!(out, "                return null;").ok();
        writeln!(out, "            }}").ok();

        if is_opaque {
            // Opaque handles: wrap the raw pointer directly, caller owns and will close()
            writeln!(out, "            return new {}(resultPtr);", return_type_name).ok();
        } else {
            // Record types: use _to_json to serialize the full struct to JSON, then deserialize.
            // NOTE: _content only returns the markdown string field, not a full JSON object.
            let type_snake = return_type_name.to_snake_case();
            let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
            let to_json_handle = format!(
                "NativeLib.{}_{}_TO_JSON",
                prefix.to_uppercase(),
                type_snake.to_uppercase()
            );
            writeln!(
                out,
                "            var jsonPtr = (MemorySegment) {}.invoke(resultPtr);",
                to_json_handle
            )
            .ok();
            writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
            writeln!(out, "            if (jsonPtr.equals(MemorySegment.NULL)) {{").ok();
            writeln!(out, "                checkLastError();").ok();
            writeln!(out, "                return null;").ok();
            writeln!(out, "            }}").ok();
            writeln!(
                out,
                "            String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);"
            )
            .ok();
            writeln!(
                out,
                "            NativeLib.{}_FREE_STRING.invoke(jsonPtr);",
                prefix.to_uppercase()
            )
            .ok();
            writeln!(
                out,
                "            return createObjectMapper().readValue(json, {}.class);",
                return_type_name
            )
            .ok();
        }

        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(func.return_type, TypeRef::Vec(_)) {
        // Vec return types: FFI returns a JSON string pointer; deserialize into List<T>.
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
        writeln!(out, "                return java.util.List.of();").ok();
        writeln!(out, "            }}").ok();
        writeln!(
            out,
            "            String json = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        // Determine the element type for deserialization (use boxed types for generics)
        let element_type = match &func.return_type {
            TypeRef::Vec(inner) => java_boxed_type(inner),
            _ => unreachable!(),
        };
        writeln!(
            out,
            "            return createObjectMapper().readValue(json, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<{}>>() {{ }});",
            element_type
        )
        .ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else {
        writeln!(
            out,
            "            var primitiveResult = ({}) {}.invoke({});",
            java_ffi_return_cast(&func.return_type),
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            return primitiveResult;").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    }

    writeln!(out, "    }}").ok();
}

pub(crate) fn gen_async_wrapper_method(
    out: &mut String,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = java_type(&p.ty);
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = match &func.return_type {
        TypeRef::Unit => "Void".to_string(),
        other => java_boxed_type(other).to_string(),
    };

    let sync_method_name = to_java_name(&func.name);
    let async_method_name = format!("{}Async", sync_method_name);
    let param_names: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| to_java_name(&p.name))
        .collect();

    writeln!(
        out,
        "    public static CompletableFuture<{}> {}({}) {{",
        return_type,
        async_method_name,
        params.join(", ")
    )
    .ok();
    writeln!(out, "        return CompletableFuture.supplyAsync(() -> {{").ok();
    writeln!(out, "            try {{").ok();
    writeln!(
        out,
        "                return {}({});",
        sync_method_name,
        param_names.join(", ")
    )
    .ok();
    writeln!(out, "            }} catch (Throwable e) {{").ok();
    writeln!(out, "                throw new CompletionException(e);").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }});").ok();
    writeln!(out, "    }}").ok();
}
