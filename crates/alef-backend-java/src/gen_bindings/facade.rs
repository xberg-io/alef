use crate::type_map::java_type;
use alef_codegen::naming::to_java_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, TypeRef};
use std::collections::HashSet;
use std::fmt::Write;

use super::helpers::is_bridge_param_java;

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_facade_class(
    api: &ApiSurface,
    package: &str,
    public_class: &str,
    raw_class: &str,
    _prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    let mut body = String::with_capacity(4096);

    writeln!(body, "public final class {} {{", public_class).ok();
    writeln!(body, "    private {}() {{ }}", public_class).ok();
    writeln!(body).ok();

    // Generate static methods for free functions
    for func in &api.functions {
        // Sync method — bridge params stripped from public signature.
        // Optional params take the Java boxed type (Integer/Long/Boolean/...)
        // so callers can pass `null` to skip them.
        let params: Vec<String> = func
            .params
            .iter()
            .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
            .map(|p| {
                let ptype = if p.optional {
                    crate::type_map::java_boxed_type(&p.ty)
                } else {
                    java_type(&p.ty)
                };
                format!("final {} {}", ptype, to_java_name(&p.name))
            })
            .collect();

        let return_type = java_type(&func.return_type);

        if !func.doc.is_empty() {
            writeln!(body, "    /**").ok();
            for line in func.doc.lines() {
                writeln!(body, "     * {}", line).ok();
            }
            writeln!(body, "     */").ok();
        }

        writeln!(
            body,
            "    public static {} {}({}) throws {}Exception {{",
            return_type,
            to_java_name(&func.name),
            params.join(", "),
            raw_class
        )
        .ok();

        // Null checks for required non-bridge parameters
        for param in &func.params {
            if !param.optional && !is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
                let pname = to_java_name(&param.name);
                writeln!(
                    body,
                    "        java.util.Objects.requireNonNull({}, \"{} must not be null\");",
                    pname, pname
                )
                .ok();
            }
        }

        // Delegate to raw FFI class — bridge params are stripped from the raw class
        // signature, so we must exclude them entirely (not pass null) to match the
        // raw class's parameter count.
        let call_args: Vec<String> = func
            .params
            .iter()
            .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
            .map(|p| to_java_name(&p.name))
            .collect();

        if matches!(func.return_type, TypeRef::Unit) {
            writeln!(
                body,
                "        {}.{}({});",
                raw_class,
                to_java_name(&func.name),
                call_args.join(", ")
            )
            .ok();
        } else if matches!(func.return_type, TypeRef::Optional(_)) {
            writeln!(
                body,
                "        return {}.{}({}).orElseThrow();",
                raw_class,
                to_java_name(&func.name),
                call_args.join(", ")
            )
            .ok();
        } else {
            writeln!(
                body,
                "        return {}.{}({});",
                raw_class,
                to_java_name(&func.name),
                call_args.join(", ")
            )
            .ok();
        }

        writeln!(body, "    }}").ok();
        writeln!(body).ok();

        // Generate overload without optional params (convenience method).
        // Only non-bridge params are considered here.
        let has_optional = func
            .params
            .iter()
            .any(|p| p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases));
        if has_optional {
            let required_params: Vec<String> = func
                .params
                .iter()
                .filter(|p| !p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    let ptype = java_type(&p.ty);
                    format!("final {} {}", ptype, to_java_name(&p.name))
                })
                .collect();

            writeln!(
                body,
                "    public static {} {}({}) throws {}Exception {{",
                return_type,
                to_java_name(&func.name),
                required_params.join(", "),
                raw_class
            )
            .ok();

            // Build call to raw class: bridge params are excluded (stripped from raw
            // class signature), optional params passed as default values or null.
            let full_args: Vec<String> = func
                .params
                .iter()
                .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    if p.optional {
                        match &p.ty {
                            TypeRef::Primitive(prim) => match prim {
                                alef_core::ir::PrimitiveType::I8 => "0".to_string(),
                                alef_core::ir::PrimitiveType::I16 => "0".to_string(),
                                alef_core::ir::PrimitiveType::I32 => "0".to_string(),
                                alef_core::ir::PrimitiveType::I64 => "0L".to_string(),
                                alef_core::ir::PrimitiveType::Isize => "0L".to_string(),
                                alef_core::ir::PrimitiveType::U8 => "0".to_string(),
                                alef_core::ir::PrimitiveType::U16 => "0".to_string(),
                                alef_core::ir::PrimitiveType::U32 => "0".to_string(),
                                alef_core::ir::PrimitiveType::U64 => "0L".to_string(),
                                alef_core::ir::PrimitiveType::Usize => "0L".to_string(),
                                alef_core::ir::PrimitiveType::F32 => "0.0f".to_string(),
                                alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
                                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                            },
                            _ => "null".to_string(),
                        }
                    } else {
                        to_java_name(&p.name)
                    }
                })
                .collect();

            if matches!(func.return_type, TypeRef::Unit) {
                writeln!(
                    body,
                    "        {}.{}({});",
                    raw_class,
                    to_java_name(&func.name),
                    full_args.join(", ")
                )
                .ok();
            } else if matches!(func.return_type, TypeRef::Optional(_)) {
                // FFI returns Optional<T>, but facade declares T (unwrapped).
                // Unwrap the Optional and throw if empty.
                writeln!(
                    body,
                    "        return {}.{}({}).orElseThrow();",
                    raw_class,
                    to_java_name(&func.name),
                    full_args.join(", ")
                )
                .ok();
            } else {
                writeln!(
                    body,
                    "        return {}.{}({});",
                    raw_class,
                    to_java_name(&func.name),
                    full_args.join(", ")
                )
                .ok();
            }

            writeln!(body, "    }}").ok();
            writeln!(body).ok();
        }
    }

    writeln!(body, "}}").ok();

    // Now assemble the file with imports
    let mut out = String::with_capacity(body.len() + 512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();

    // Check what imports are needed based on content
    let has_list = body.contains("List<");
    let has_map = body.contains("Map<");
    let has_optional = body.contains("Optional<");
    let has_imports = has_list || has_map || has_optional;

    if has_imports {
        writeln!(out).ok();
        if has_list {
            writeln!(out, "import java.util.List;").ok();
        }
        if has_map {
            writeln!(out, "import java.util.Map;").ok();
        }
        if has_optional {
            writeln!(out, "import java.util.Optional;").ok();
        }
    }

    writeln!(out).ok();
    out.push_str(&body);

    out
}
