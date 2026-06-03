/// Generate C# visitor support for compatibility callback bridges.
///
/// # P/Invoke delegate callback strategy
///
/// C# uses `[UnmanagedFunctionPointer]` delegate types to create `IntPtr` function pointers
/// that can be passed through the generated visitor callback C struct.
///
/// - `NodeContext`: a `record` used when the configured bridge methods require it.
/// - `VisitResult`: a discriminated union emitted when the configured bridge methods return it.
/// - `IVisitor`: an interface with default no-op implementations for all 40 callbacks.
/// - `VisitorCallbacks`: an internal class that allocates `GCHandle`s for all delegate
///   instances and writes them into a marshalled struct layout matching the C struct.
/// - `ConvertWithVisitor`: static method on the wrapper class that creates the delegate
///   struct, calls `htm_visitor_create`, `htm_convert_with_visitor`, deserialises JSON.
use crate::core::hash::{self, CommentStyle};
use heck::ToSnakeCase;

const COMPAT_CONTEXT_TYPE: &str = "NodeContext";
const COMPAT_RESULT_TYPE: &str = "VisitResult";

// ---------------------------------------------------------------------------
// Callback specification table
// ---------------------------------------------------------------------------

pub struct CallbackSpec {
    /// Field name in the generated visitor callback C struct.
    pub c_field: String,
    /// C# interface method name (PascalCase).
    pub cs_method: String,
    /// XML doc summary.
    pub doc: String,
    /// Extra parameters beyond `NodeContext` in the C# interface.
    pub extra: Vec<ExtraParam>,
    /// If true, add `bool isHeader` (only visit_table_row).
    pub has_is_header: bool,
}

pub struct ExtraParam {
    /// C# parameter name in the interface.
    pub cs_name: String,
    /// C# type in the interface method signature.
    pub cs_type: String,
    /// P/Invoke types for each raw C parameter (one or more per Java param).
    pub pinvoke_types: Vec<String>,
    /// C# expression to decode the raw P/Invoke args (vars named `raw<CsName>N`).
    pub decode: String,
}

// ---------------------------------------------------------------------------
// IR-driven callback spec builder
// ---------------------------------------------------------------------------

/// Convert snake_case to lowerCamelCase for C# parameter names.
/// E.g. "tag_name" → "tagName", "inputType" → "inputType" (passthrough).
#[allow(dead_code)]
fn snake_to_lower_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut next_upper = false;
    for ch in s.chars() {
        if ch == '_' {
            next_upper = true;
        } else if next_upper {
            result.extend(ch.to_uppercase());
            next_upper = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Build a `Vec<CallbackSpec>` from a trait's IR definition for the C# backend.
///
/// Derives all language-specific C# fields (method names, P/Invoke types, decode
/// expressions) from `TypeRef` + `optional` flag. Methods with unsupported parameter
/// types are skipped with a warning.
#[allow(dead_code)]
pub(crate) fn callback_specs_from_trait(trait_def: &crate::core::ir::TypeDef) -> Vec<CallbackSpec> {
    use crate::codegen::naming::to_csharp_name;
    use crate::core::ir::{PrimitiveType, TypeRef};

    let mut specs = Vec::with_capacity(trait_def.methods.len());
    'methods: for m in &trait_def.methods {
        if m.trait_source.is_some() {
            continue;
        }
        let cs_method = to_csharp_name(&m.name);
        let first_line = m.doc.lines().next().unwrap_or("").trim().to_string();
        let doc = if first_line.is_empty() {
            format!("Called for {} elements.", m.name.replace('_', " "))
        } else {
            first_line
        };

        let mut extra = Vec::new();
        let mut has_is_header = false;

        for p in &m.params {
            if matches!(&p.ty, TypeRef::Named(_)) {
                // Context parameter — skip, handled separately
                continue;
            }
            let raw_name = p.name.trim_start_matches('_').to_string();
            let cs_name = snake_to_lower_camel(&raw_name);
            // Capitalise for the raw var names (e.g. "text" → "Text", "inputType" → "InputType")
            let cs_name_pascal: String = {
                let mut chars = cs_name.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            };

            match (&p.ty, p.optional) {
                (TypeRef::String, false) => {
                    let raw_var = format!("raw{cs_name_pascal}0");
                    extra.push(ExtraParam {
                        cs_name,
                        cs_type: "string".to_string(),
                        pinvoke_types: vec!["IntPtr".to_string()],
                        decode: format!("global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8({raw_var})!"),
                    });
                }
                (TypeRef::String, true) => {
                    let raw_var = format!("raw{cs_name_pascal}0");
                    extra.push(ExtraParam {
                        cs_name,
                        cs_type: "string?".to_string(),
                        pinvoke_types: vec!["IntPtr".to_string()],
                        decode: format!("{raw_var} == IntPtr.Zero ? null : global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8({raw_var})"),
                    });
                }
                (TypeRef::Primitive(PrimitiveType::Bool), false) => {
                    let raw_var = format!("raw{cs_name_pascal}0");
                    extra.push(ExtraParam {
                        cs_name,
                        cs_type: "bool".to_string(),
                        pinvoke_types: vec!["int".to_string()],
                        decode: format!("{raw_var} != 0"),
                    });
                }
                (
                    TypeRef::Primitive(
                        PrimitiveType::U32
                        | PrimitiveType::I32
                        | PrimitiveType::U16
                        | PrimitiveType::I16
                        | PrimitiveType::U8
                        | PrimitiveType::I8,
                    ),
                    false,
                ) => {
                    let raw_var = format!("raw{cs_name_pascal}0");
                    extra.push(ExtraParam {
                        cs_name,
                        cs_type: "uint".to_string(),
                        pinvoke_types: vec!["uint".to_string()],
                        decode: raw_var,
                    });
                }
                (TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::I64), false) => {
                    let raw_var = format!("raw{cs_name_pascal}0");
                    extra.push(ExtraParam {
                        cs_name,
                        cs_type: "ulong".to_string(),
                        pinvoke_types: vec!["UIntPtr".to_string()],
                        decode: format!("(ulong){raw_var}"),
                    });
                }
                (TypeRef::Vec(inner), false) => match inner.as_ref() {
                    TypeRef::String => {
                        let raw_ptr = format!("raw{cs_name_pascal}0");
                        let raw_len = format!("raw{cs_name_pascal}1");
                        extra.push(ExtraParam {
                            cs_name,
                            cs_type: "string[]".to_string(),
                            pinvoke_types: vec!["IntPtr".to_string(), "UIntPtr".to_string()],
                            decode: format!("DecodeCells({raw_ptr}, (long)(ulong){raw_len})"),
                        });
                        has_is_header = true;
                        break;
                    }
                    _ => {
                        eprintln!(
                            "[alef] gen_visitor(csharp): skip method `{}` — unsupported Vec param `{}`",
                            m.name, p.name
                        );
                        continue 'methods;
                    }
                },
                _ => {
                    eprintln!(
                        "[alef] gen_visitor(csharp): skip method `{}` — unsupported param `{}: {:?}`",
                        m.name, p.name, p.ty
                    );
                    continue 'methods;
                }
            }
        }

        specs.push(CallbackSpec {
            c_field: m.name.clone(),
            cs_method,
            doc,
            extra,
            has_is_header,
        });
    }
    specs
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns `(filename, content)` pairs for required compatibility visitor files.
///
/// IVisitor.cs and VisitorCallbacks.cs are superseded by IVisitor and VisitorCallbacks
/// in TraitBridges.cs which use the configured trait bridge approach. They are intentionally
/// excluded here; stale committed copies are removed by delete_superseded_visitor_files.
pub fn gen_visitor_files(namespace: &str, trait_def: &crate::core::ir::TypeDef) -> Vec<(String, String)> {
    let mut files = Vec::new();
    if trait_requires_named_param(trait_def, COMPAT_CONTEXT_TYPE) {
        files.push(("NodeContext.cs".to_string(), gen_node_context(namespace)));
    }
    if trait_returns_named_type(trait_def, COMPAT_RESULT_TYPE) {
        files.push(("VisitResult.cs".to_string(), gen_visit_result(namespace)));
    }
    files
}

/// Generate the P/Invoke declarations needed in NativeMethods.cs for visitor FFI.
///
/// Parameters:
/// - `namespace`: C# namespace (unused, kept for compatibility)
/// - `lib_name`: Native library name (unused, kept for compatibility)
/// - `prefix`: C FFI function name prefix (e.g., "htm")
/// - `trait_name`: Name of the visitor trait for bridge function names
/// - `options_field`: Field name in options to set visitor on (e.g., "visitor")
pub fn gen_native_methods_visitor(
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    trait_name: &str,
    options_type: &str,
    options_field: &str,
) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    // Generate function names:
    // htm_htm_html_visitor_bridge_new, htm_htm_html_visitor_bridge_free, htm_options_set_visitor
    let trait_snake = trait_name.to_snake_case();
    let bridge_snake = format!("{prefix}_{trait_snake}_bridge");
    let fn_bridge_new = format!("{prefix}_{bridge_snake}_new");
    let fn_bridge_free = format!("{prefix}_{bridge_snake}_free");
    let fn_options_set = format!("{prefix}_options_set_{options_field}");
    let bridge_name = crate::codegen::naming::csharp_type_name(trait_name) + "Bridge";
    let options_name = crate::codegen::naming::csharp_type_name(options_type);
    let field_name = crate::codegen::naming::to_csharp_name(options_field);

    let mut out = String::from("\n");
    out.push_str(&render(
        "native_methods_visitor.jinja",
        Value::from_serialize(serde_json::json!({
            "fn_bridge_new": fn_bridge_new,
            "fn_bridge_free": fn_bridge_free,
            "fn_options_set": fn_options_set,
            "bridge_name": bridge_name,
            "options_name": options_name,
            "field_name": field_name,
        })),
    ));

    let _ = namespace;
    let _ = lib_name;
    out
}

/// DEPRECATED: gen_convert_with_visitor_method is no longer used.
/// The visitor logic is now integrated into the main Convert() method in gen_wrapper_function,
/// which creates the configured bridge and uses the configured options setter instead.
#[allow(dead_code)]
pub fn gen_convert_with_visitor_method(exception_name: &str, prefix: &str) -> String {
    let _ = exception_name;
    let _ = prefix;
    String::new()
}

// ---------------------------------------------------------------------------
// Individual file generators
// ---------------------------------------------------------------------------

fn gen_node_context(namespace: &str) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let mut out = String::with_capacity(1024);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("#nullable enable\n");
    out.push('\n');
    out.push_str("using System;\n");
    out.push('\n');
    out.push_str(&render(
        "namespace_decl.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
        })),
    ));
    out.push_str("/// <summary>Context passed to every visitor callback.</summary>\n");
    out.push_str("public record NodeContext(\n");
    out.push_str("    /// <summary>Coarse-grained node type tag.</summary>\n");
    out.push_str("    NodeType NodeType,\n");
    out.push_str("    /// <summary>Element tag name.</summary>\n");
    out.push_str("    string TagName,\n");
    out.push_str("    /// <summary>Traversal depth (0 = root).</summary>\n");
    out.push_str("    ulong Depth,\n");
    out.push_str("    /// <summary>0-based sibling index.</summary>\n");
    out.push_str("    ulong IndexInParent,\n");
    out.push_str("    /// <summary>Parent element tag name, or null at the root.</summary>\n");
    out.push_str("    string? ParentTag,\n");
    out.push_str("    /// <summary>True when this element is treated as inline.</summary>\n");
    out.push_str("    bool IsInline\n");
    out.push_str(");\n");
    out
}

fn gen_visit_result(namespace: &str) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let mut out = String::with_capacity(2048);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("#nullable enable\n");
    out.push('\n');
    out.push_str("using System;\n");
    out.push('\n');
    out.push_str(&render(
        "namespace_decl.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
        })),
    ));
    out.push_str("/// <summary>Controls how the visitor affects the conversion pipeline.</summary>\n");
    out.push_str("public abstract record VisitResult\n");
    out.push_str("{\n");
    out.push_str("    private VisitResult() {}\n");
    out.push('\n');
    out.push_str("    /// <summary>Proceed with default conversion.</summary>\n");
    out.push_str("    public sealed record Continue : VisitResult;\n");
    out.push('\n');
    out.push_str("    /// <summary>Omit this element from output entirely.</summary>\n");
    out.push_str("    public sealed record Skip : VisitResult;\n");
    out.push('\n');
    out.push_str("    /// <summary>Keep original content verbatim.</summary>\n");
    out.push_str("    public sealed record PreserveHtml : VisitResult;\n");
    out.push('\n');
    out.push_str("    /// <summary>Replace with custom output.</summary>\n");
    out.push_str("    public sealed record Custom(string Output) : VisitResult;\n");
    out.push('\n');
    out.push_str("    /// <summary>Abort conversion with an error message.</summary>\n");
    out.push_str("    public sealed record Error(string Message) : VisitResult;\n");
    out.push('\n');
    out.push_str("    internal string ToFfiJson() => this switch {\n");
    out.push_str("        VisitResult.Continue => \"\\\"Continue\\\"\",\n");
    out.push_str("        VisitResult.Skip => \"\\\"Skip\\\"\",\n");
    out.push_str("        VisitResult.PreserveHtml => \"\\\"PreserveHtml\\\"\",\n");
    out.push_str("        VisitResult.Custom c => \"{\\\"Custom\\\":\" + System.Text.Json.JsonSerializer.Serialize(c.Output) + \"}\",\n");
    out.push_str("        VisitResult.Error e => \"{\\\"Error\\\":\" + System.Text.Json.JsonSerializer.Serialize(e.Message) + \"}\",\n");
    out.push_str("        _ => \"\\\"Continue\\\"\"\n");
    out.push_str("    };\n");
    out.push_str("}\n");
    out
}

fn trait_requires_named_param(trait_def: &crate::core::ir::TypeDef, type_name: &str) -> bool {
    trait_def.methods.iter().any(|method| {
        method.trait_source.is_none()
            && method
                .params
                .iter()
                .any(|param| named_type_name(&param.ty) == Some(type_name))
    })
}

fn trait_returns_named_type(trait_def: &crate::core::ir::TypeDef, type_name: &str) -> bool {
    trait_def
        .methods
        .iter()
        .any(|method| method.trait_source.is_none() && named_type_name(&method.return_type) == Some(type_name))
}

fn named_type_name(ty: &crate::core::ir::TypeRef) -> Option<&str> {
    match ty {
        crate::core::ir::TypeRef::Named(name) => Some(name.as_str()),
        crate::core::ir::TypeRef::Optional(inner) => named_type_name(inner),
        _ => None,
    }
}

// gen_ivisitor and gen_visitor_callbacks were removed: IVisitor and VisitorCallbacks
// are now emitted by TraitBridges.cs. Generating them here produced dead code that
// conflicted with those implementations.

#[cfg(test)]
mod tests {
    use super::gen_visitor_files;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

    #[test]
    fn emits_compat_files_only_when_trait_methods_require_them() {
        let files = gen_visitor_files(
            "Sample",
            &trait_def(
                "Renderer",
                vec![method(
                    "visit_text",
                    vec![param("_ctx", TypeRef::Named("NodeContext".to_string()))],
                    TypeRef::Named("VisitResult".to_string()),
                )],
            ),
        );

        let filenames = files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
        assert_eq!(filenames, vec!["NodeContext.cs", "VisitResult.cs"]);
    }

    #[test]
    fn skips_fixed_compat_files_for_generic_traits() {
        let files = gen_visitor_files(
            "Sample",
            &trait_def(
                "Renderer",
                vec![method(
                    "render",
                    vec![param("_input", TypeRef::String)],
                    TypeRef::String,
                )],
            ),
        );

        assert!(files.is_empty());
    }

    fn trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("sample::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::RefMut),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: true,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            newtype_wrapper: None,
            is_ref: false,
            is_mut: false,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
        }
    }
}
