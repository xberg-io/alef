use crate::type_map::{java_boxed_type, java_return_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_java_name;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write;

use super::OptionsFieldBridgeInfo;
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
    options_field_bridges: &[OptionsFieldBridgeInfo],
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
        // Detect whether any options-field bridge applies to this function.
        // A bridge applies when the function has a parameter whose Named type matches
        // `bridge.options_type` (the options struct that carries the bridge field).
        let opts_bridge: Option<&OptionsFieldBridgeInfo> = options_field_bridges.iter().find(|bridge| {
            func.params.iter().any(|p| {
                let inner = match &p.ty {
                    TypeRef::Named(n) => n.as_str(),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            n.as_str()
                        } else {
                            ""
                        }
                    }
                    _ => "",
                };
                inner == bridge.options_type.as_str()
            })
        });

        if let Some(bridge) = opts_bridge {
            // Options-field bridge mode: emit a wrapper that reads the visitor from the
            // options object, attaches it via the FFI setter, then delegates to the main
            // 2-arg FFI. No separate `convertWithVisitor` method is emitted.
            gen_sync_function_method_with_options_field_bridge(
                &mut body,
                func,
                prefix,
                class_name,
                &opaque_types,
                bridge,
            );
        } else {
            // Legacy path: generate sync method, stripping any bridge params.
            gen_sync_function_method(
                &mut body,
                func,
                prefix,
                class_name,
                &opaque_types,
                bridge_param_names,
                bridge_type_aliases,
            );
        }
        writeln!(body).ok();

        // Also generate async wrapper if marked as async
        if func.is_async {
            gen_async_wrapper_method(&mut body, func, bridge_param_names, bridge_type_aliases);
            writeln!(body).ok();
        }
    }

    // Inject convertWithVisitor only for the legacy visitor_callbacks pattern.
    // Options-field bridges surface the visitor via ConversionOptions — no extra method needed.
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
    // Exclude bridge params from the public Java signature. Optional params
    // take the boxed Java type (Integer/Long/Boolean/...) so callers can pass
    // `null` to skip them.
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = if p.optional {
                java_boxed_type(&p.ty)
            } else {
                java_type(&p.ty)
            };
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = java_return_type(&func.return_type);

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
        // When a parameter is optional (Option<T> in Rust), wrap the TypeRef so that
        // marshal_param_to_ffi generates a null-safe allocation path.
        let effective_ty = if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
            TypeRef::Optional(Box::new(param.ty.clone()))
        } else {
            param.ty.clone()
        };
        marshal_param_to_ffi(out, &to_java_name(&param.name), &effective_ty, opaque_types, prefix);
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
                // Apply the same optional-wrapping logic used when marshalling.
                let effective_ty = if p.optional && !matches!(p.ty, TypeRef::Optional(_)) {
                    TypeRef::Optional(Box::new(p.ty.clone()))
                } else {
                    p.ty.clone()
                };
                ffi_param_name(&to_java_name(&p.name), &effective_ty, opaque_types)
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

    // Unwrap Optional<T> to determine the actual dispatch type and whether we're optional.
    let (is_optional_return, dispatch_return_type) = match &func.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    if matches!(dispatch_return_type, TypeRef::Unit) {
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
    } else if is_ffi_string_return(&dispatch_return_type) {
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
        if is_optional_return {
            writeln!(out, "                return Optional.empty();").ok();
        } else {
            writeln!(out, "                return null;").ok();
        }
        writeln!(out, "            }}").ok();
        writeln!(
            out,
            "            String str = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        let return_expr = if matches!(dispatch_return_type, TypeRef::Path) {
            "java.nio.file.Path.of(str)"
        } else {
            "str"
        };
        if is_optional_return {
            writeln!(out, "            return Optional.of({});", return_expr).ok();
        } else {
            writeln!(out, "            return {};", return_expr).ok();
        }
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(dispatch_return_type, TypeRef::Named(_)) {
        // Named return types: FFI returns a struct pointer.
        let return_type_name = match &dispatch_return_type {
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
        if is_optional_return {
            writeln!(out, "                return Optional.empty();").ok();
        } else {
            writeln!(out, "                return null;").ok();
        }
        writeln!(out, "            }}").ok();

        if is_opaque {
            // Opaque handles: wrap the raw pointer directly, caller owns and will close()
            if is_optional_return {
                writeln!(
                    out,
                    "            return Optional.of(new {}(resultPtr));",
                    return_type_name
                )
                .ok();
            } else {
                writeln!(out, "            return new {}(resultPtr);", return_type_name).ok();
            }
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
            if is_optional_return {
                writeln!(out, "                return Optional.empty();").ok();
            } else {
                writeln!(out, "                return null;").ok();
            }
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
            if is_optional_return {
                writeln!(
                    out,
                    "            return Optional.of(createObjectMapper().readValue(json, {}.class));",
                    return_type_name
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "            return createObjectMapper().readValue(json, {}.class);",
                    return_type_name
                )
                .ok();
            }
        }

        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(dispatch_return_type, TypeRef::Vec(_)) {
        // Vec return types: FFI returns a JSON string pointer; deserialize into List<T>.
        // The body is delegated to a single `readJsonList` helper emitted by
        // `gen_helper_methods` so the JSON-deserialize boilerplate isn't duplicated
        // at every call site (which CPD flagged as copy-paste duplication).
        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        let element_type = match &dispatch_return_type {
            TypeRef::Vec(inner) => java_boxed_type(inner),
            _ => unreachable!(),
        };
        let type_ref = format!(
            "new com.fasterxml.jackson.core.type.TypeReference<java.util.List<{}>>() {{ }}",
            element_type
        );
        if is_optional_return {
            writeln!(
                out,
                "            return Optional.of(readJsonList(resultPtr, {}));",
                type_ref
            )
            .ok();
        } else {
            writeln!(out, "            return readJsonList(resultPtr, {});", type_ref).ok();
        }
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(dispatch_return_type, TypeRef::Bytes) {
        // Bytes return types: FFI returns an opaque pointer to allocated bytes; deserialize as byte array.
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
        if is_optional_return {
            writeln!(out, "                return Optional.empty();").ok();
        } else {
            writeln!(out, "                return null;").ok();
        }
        writeln!(out, "            }}").ok();
        writeln!(out, "            long byteLen = resultPtr.byteSize();").ok();
        writeln!(
            out,
            "            byte[] result = resultPtr.reinterpret(byteLen).toArray(ValueLayout.JAVA_BYTE);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        if is_optional_return {
            writeln!(out, "            return Optional.of(result);").ok();
        } else {
            writeln!(out, "            return result;").ok();
        }
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else {
        // Primitive return types (including boxed types for Optional)
        writeln!(
            out,
            "            var primitiveResult = ({}) {}.invoke({});",
            java_ffi_return_cast(&dispatch_return_type),
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        if is_optional_return {
            writeln!(out, "            return Optional.of(primitiveResult);").ok();
        } else {
            writeln!(out, "            return primitiveResult;").ok();
        }
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

/// Generate a sync method that uses the options-field bridge pattern.
///
/// Instead of receiving the visitor as an extra function argument, it is embedded as a field
/// on the options record. The generated wrapper:
///   1. Takes the same public signature as the normal method (no extra visitor param).
///   2. Reads `options.<field>()` (e.g. `options.visitor()`).
///   3. If non-null, creates the bridge object via `new <Bridge>(options.visitor())` and
///      attaches it via the `{PU}_OPTIONS_SET_{FIELD}` handle before the main FFI call.
///   4. Calls the main FFI function with the (now-mutated) options pointer.
pub(crate) fn gen_sync_function_method_with_options_field_bridge(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge: &OptionsFieldBridgeInfo,
) {
    // Build the public Java parameter list — same as normal path (no bridge param to strip).
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ptype = if p.optional {
                java_boxed_type(&p.ty)
            } else {
                java_type(&p.ty)
            };
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = java_return_type(&func.return_type);

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

    // Find the options param (whose Named type == bridge.options_type).
    let opts_param = func.params.iter().find(|p| {
        let inner = match &p.ty {
            TypeRef::Named(n) => n.as_str(),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    n.as_str()
                } else {
                    ""
                }
            }
            _ => "",
        };
        inner == bridge.options_type.as_str()
    });

    // Collect non-opaque Named params that need FFI pointer cleanup after the call.
    // The options param is excluded here because we pass JSON directly (no _from_json pointer to free).
    let ffi_ptr_params: Vec<(String, String)> = func
        .params
        .iter()
        .filter(|p| {
            // Exclude the options param — it is serialized to JSON and passed as a C string,
            // not allocated by _from_json, so there is no pointer to free.
            if let Some(opts_p) = opts_param {
                p.name != opts_p.name
            } else {
                true
            }
        })
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

    // Marshal all parameters.
    // The options param is serialized to JSON + arena.allocateFrom (no _from_json call —
    // htm_conversion_options_from_json does not exist in the FFI surface).
    // All other params use the standard marshal_param_to_ffi path.
    for param in &func.params {
        let param_java_name = to_java_name(&param.name);
        let is_options_param = opts_param.is_some_and(|op| op.name == param.name);
        if is_options_param {
            let cname = format!("c{param_java_name}");
            writeln!(
                out,
                "            var {cname}Json = {param_java_name} != null ? createObjectMapper().writeValueAsString({param_java_name}) : null;"
            )
            .ok();
            writeln!(
                out,
                "            var {cname}JsonSeg = {cname}Json != null ? arena.allocateFrom({cname}Json) : MemorySegment.NULL;"
            )
            .ok();
        } else {
            let effective_ty = if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
                TypeRef::Optional(Box::new(param.ty.clone()))
            } else {
                param.ty.clone()
            };
            marshal_param_to_ffi(out, &param_java_name, &effective_ty, opaque_types, prefix);
        }
    }

    // After marshalling, if there is an options param with a bridge field, attach the bridge.
    if let Some(opts_p) = opts_param {
        let opts_java_name = to_java_name(&opts_p.name);
        // The JSON segment variable name matches what we emitted above.
        let opts_ffi_name = format!("c{opts_java_name}JsonSeg");
        let field_getter = to_java_name(&bridge.field_name);
        let bridge_java_type = &bridge.bridge_java_type;
        let set_handle = format!(
            "NativeLib.{}_OPTIONS_SET_{}",
            prefix.to_uppercase(),
            bridge.field_name.to_uppercase()
        );
        let is_opaque_bridge = opaque_types.contains(bridge_java_type.as_str());
        // Emit bridge attachment only when handle and visitor are both non-null.
        writeln!(
            out,
            "            if ({set_handle} != null && {opts_java_name}.{field_getter}() != null) {{"
        )
        .ok();
        if is_opaque_bridge {
            // Opaque handle: pass the raw MemorySegment directly via .handle()
            writeln!(
                out,
                "                {set_handle}.invoke({opts_ffi_name}, {opts_java_name}.{field_getter}().handle());"
            )
            .ok();
        } else {
            // Non-opaque / interface type: create a bridge wrapper and use callbacksStruct()
            let bridge_class_name = if bridge_java_type.ends_with("Handle") {
                format!("{}Bridge", &bridge_java_type[..bridge_java_type.len() - "Handle".len()])
            } else {
                format!("{bridge_java_type}Bridge")
            };
            writeln!(
                out,
                "                var bridge = new {bridge_class_name}({opts_java_name}.{field_getter}());"
            )
            .ok();
            writeln!(out, "                var bridgeSeg = bridge.callbacksStruct();").ok();
            writeln!(out, "                {set_handle}.invoke({opts_ffi_name}, bridgeSeg);").ok();
        }
        writeln!(out, "            }}").ok();
    }

    // Build call args — all parameters are included (no bridge param to skip).
    // For the options param, pass the JSON segment variable name directly.
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let param_java_name = to_java_name(&p.name);
            let is_options_param = opts_param.is_some_and(|op| op.name == p.name);
            if is_options_param {
                format!("c{param_java_name}JsonSeg")
            } else {
                let effective_ty = if p.optional && !matches!(p.ty, TypeRef::Optional(_)) {
                    TypeRef::Optional(Box::new(p.ty.clone()))
                } else {
                    p.ty.clone()
                };
                ffi_param_name(&param_java_name, &effective_ty, opaque_types)
            }
        })
        .collect();

    let ffi_handle = format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

    let emit_ffi_ptr_cleanup = |out: &mut String| {
        for (cname, free_handle) in &ffi_ptr_params {
            writeln!(out, "            if (!{}.equals(MemorySegment.NULL)) {{", cname).ok();
            writeln!(out, "                {}.invoke({});", free_handle, cname).ok();
            writeln!(out, "            }}").ok();
        }
    };

    let (is_optional_return, dispatch_return_type) = match &func.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    if matches!(dispatch_return_type, TypeRef::Unit) {
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
    } else if is_ffi_string_return(&dispatch_return_type) {
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
        if is_optional_return {
            writeln!(out, "                return Optional.empty();").ok();
        } else {
            writeln!(out, "                return null;").ok();
        }
        writeln!(out, "            }}").ok();
        writeln!(
            out,
            "            String str = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        let return_expr = if matches!(dispatch_return_type, TypeRef::Path) {
            "java.nio.file.Path.of(str)"
        } else {
            "str"
        };
        if is_optional_return {
            writeln!(out, "            return Optional.of({});", return_expr).ok();
        } else {
            writeln!(out, "            return {};", return_expr).ok();
        }
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(dispatch_return_type, TypeRef::Named(_)) {
        let return_type_name = match &dispatch_return_type {
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
        if is_optional_return {
            writeln!(out, "                return Optional.empty();").ok();
        } else {
            writeln!(out, "                return null;").ok();
        }
        writeln!(out, "            }}").ok();
        if is_opaque {
            if is_optional_return {
                writeln!(
                    out,
                    "            return Optional.of(new {}(resultPtr));",
                    return_type_name
                )
                .ok();
            } else {
                writeln!(out, "            return new {}(resultPtr);", return_type_name).ok();
            }
        } else {
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
            if is_optional_return {
                writeln!(out, "                return Optional.empty();").ok();
            } else {
                writeln!(out, "                return null;").ok();
            }
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
            if is_optional_return {
                writeln!(
                    out,
                    "            return Optional.of(createObjectMapper().readValue(json, {}.class));",
                    return_type_name
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "            return createObjectMapper().readValue(json, {}.class);",
                    return_type_name
                )
                .ok();
            }
        }
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(dispatch_return_type, TypeRef::Vec(_)) {
        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        let element_type = match &dispatch_return_type {
            TypeRef::Vec(inner) => java_boxed_type(inner),
            _ => unreachable!(),
        };
        let type_ref = format!(
            "new com.fasterxml.jackson.core.type.TypeReference<java.util.List<{}>>() {{ }}",
            element_type
        );
        if is_optional_return {
            writeln!(
                out,
                "            return Optional.of(readJsonList(resultPtr, {}));",
                type_ref
            )
            .ok();
        } else {
            writeln!(out, "            return readJsonList(resultPtr, {});", type_ref).ok();
        }
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
            java_ffi_return_cast(&dispatch_return_type),
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        if is_optional_return {
            writeln!(out, "            return Optional.of(primitiveResult);").ok();
        } else {
            writeln!(out, "            return primitiveResult;").ok();
        }
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
    if matches!(func.return_type, TypeRef::Unit) {
        writeln!(out, "                {}({});", sync_method_name, param_names.join(", ")).ok();
        writeln!(out, "                return null;").ok();
    } else {
        writeln!(
            out,
            "                return {}({});",
            sync_method_name,
            param_names.join(", ")
        )
        .ok();
    }
    writeln!(out, "            }} catch (Throwable e) {{").ok();
    writeln!(out, "                throw new CompletionException(e);").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }});").ok();
    writeln!(out, "    }}").ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_opaque_types() -> AHashSet<String> {
        AHashSet::new()
    }

    fn create_test_bridge_sets() -> (HashSet<String>, HashSet<String>) {
        (HashSet::new(), HashSet::new())
    }

    fn create_test_function(name: &str, return_type: TypeRef) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("test::{}", name),
            original_rust_path: String::new(),
            params: vec![],
            return_type,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }
    }

    #[test]
    fn test_optional_string_return_emits_optional_empty() {
        let func = create_test_function("get_name", TypeRef::Optional(Box::new(TypeRef::String)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        assert!(out.contains("return Optional.empty();"));
        assert!(out.contains("return Optional.of(str);"));
    }

    #[test]
    fn test_optional_named_return_emits_optional_wrappers() {
        let func = create_test_function(
            "get_preset",
            TypeRef::Optional(Box::new(TypeRef::Named("EmbeddingPreset".to_string()))),
        );

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        assert!(out.contains("return Optional.empty();"));
        assert!(out.contains("return Optional.of(createObjectMapper().readValue(json, EmbeddingPreset.class));"));
    }

    #[test]
    fn test_optional_vec_return_emits_optional_list() {
        let func = create_test_function(
            "list_items",
            TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
        );

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        // Vec returns now go through the readJsonList helper to deduplicate
        // the JSON-deserialize boilerplate (CPD was flagging multiple inline
        // copies). The empty-list-on-null path lives inside the helper.
        assert!(out.contains(
            "return Optional.of(readJsonList(resultPtr, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<String>>()"
        ));
    }

    #[test]
    fn test_optional_bytes_return_emits_optional_array() {
        let func = create_test_function("get_data", TypeRef::Optional(Box::new(TypeRef::Bytes)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        assert!(out.contains("return Optional.empty();"));
        assert!(out.contains("return Optional.of(result);"));
    }

    #[test]
    fn test_non_optional_string_return_no_optional_wrapper() {
        let func = create_test_function("get_name", TypeRef::String);

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        assert!(out.contains("return null;"));
        assert!(out.contains("return str;"));
        assert!(!out.contains("Optional.empty()"));
        assert!(!out.contains("Optional.of(str)"));
    }

    #[test]
    fn test_path_return_wraps_with_path_of() {
        let func = create_test_function("cache_dir", TypeRef::Path);

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        assert!(out.contains("return java.nio.file.Path.of(str);"));
        assert!(!out.contains("return str;"));
    }

    #[test]
    fn test_optional_path_return_wraps_with_path_of() {
        let func = create_test_function("maybe_cache_dir", TypeRef::Optional(Box::new(TypeRef::Path)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        assert!(out.contains("return Optional.of(java.nio.file.Path.of(str));"));
    }

    #[test]
    fn test_non_optional_vec_return_no_optional_wrapper() {
        let func = create_test_function("list_items", TypeRef::Vec(Box::new(TypeRef::String)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        // The Vec dispatch path now delegates to the readJsonList helper.
        // Optional<List<T>> wrapping is added by the caller; non-optional
        // is a bare call.
        assert!(out.contains(
            "return readJsonList(resultPtr, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<String>>()"
        ));
        assert!(!out.contains("Optional.of(readJsonList"));
    }

    #[test]
    fn vec_return_uses_helper_not_inline_json_deserialize() {
        // CPD regression: every Vec-returning method previously inlined a
        // ~15-line null-check + reinterpret + free + readValue block, which
        // CPD (rightly) flagged as duplication. The helper extraction means
        // the call site is one line and `readJsonList` appears exactly once
        // in the helper section.
        let func = create_test_function("list_items", TypeRef::Vec(Box::new(TypeRef::String)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        // The previously-duplicated JSON-deserialize line must NOT appear at
        // the call site any more (it now lives only in the helper, which is
        // emitted by gen_helper_methods at the bottom of the class).
        assert!(!out.contains(
            "createObjectMapper().readValue(json, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<"
        ));
    }
}
