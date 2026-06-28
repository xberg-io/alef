/// Generate C# visitor support for configured callback bridges.
///
/// # P/Invoke delegate callback strategy
///
/// C# uses `[UnmanagedFunctionPointer]` delegate types to create `IntPtr` function pointers
/// that can be passed through the generated visitor callback C struct.
///
/// - configured context type: a `record` used when the configured bridge methods require it.
/// - configured result type: a discriminated union emitted when the configured bridge methods return it.
/// - `IVisitor`: an interface with default no-op implementations for all 40 callbacks.
/// - `VisitorCallbacks`: an internal class that allocates `GCHandle`s for all delegate
///   instances and writes them into a marshalled struct layout matching the C struct.
/// - `ConvertWithVisitor`: static method on the wrapper class that creates the delegate
///   struct, calls `htm_visitor_create`, `htm_convert_with_visitor`, deserialises JSON.
use crate::core::hash::{self, CommentStyle};

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
    /// Extra parameters beyond the configured context type in the C# interface.
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
pub(crate) fn callback_specs_from_trait(trait_def: &crate::core::ir::TypeDef, context_type: &str) -> Vec<CallbackSpec> {
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
            if matches!(&p.ty, TypeRef::Named(name) if name == context_type) {
                // Configured context parameter — skip, handled separately.
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

/// Returns `(filename, content)` pairs for required configured visitor files.
///
/// IVisitor.cs and VisitorCallbacks.cs are superseded by IVisitor and VisitorCallbacks
/// in TraitBridges.cs which use the configured trait bridge approach. They are intentionally
/// excluded here; stale committed copies are removed by delete_superseded_visitor_files.
pub fn gen_visitor_files(
    namespace: &str,
    api: &crate::core::ir::ApiSurface,
    bridge_cfg: &crate::core::config::TraitBridgeConfig,
    trait_def: &crate::core::ir::TypeDef,
) -> Vec<(String, String)> {
    let mut files = Vec::new();

    if let Some(context_type) = bridge_cfg.context_type.as_deref() {
        if trait_requires_named_param(trait_def, context_type) {
            if let Some(context_def) = api.types.iter().find(|typ| typ.name == context_type && !typ.is_trait) {
                files.push((
                    format!("{}.cs", crate::codegen::naming::csharp_type_name(context_type)),
                    gen_node_context(namespace, context_def),
                ));
            } else {
                eprintln!(
                    "[alef] gen_visitor(csharp): skip context file — configured context_type `{context_type}` is absent from IR"
                );
            }
        }
    } else if trait_def.methods.iter().any(|method| method.trait_source.is_none()) {
        eprintln!(
            "[alef] gen_visitor(csharp): skip context file — trait bridge `{}` has no context_type metadata",
            bridge_cfg.trait_name
        );
    }

    if let Some(result_type) = bridge_cfg.result_type.as_deref() {
        if trait_returns_named_type(trait_def, result_type) {
            if let Some(enum_def) = api.enums.iter().find(|enum_def| enum_def.name == result_type) {
                match gen_visit_result(namespace, enum_def, &bridge_cfg.trait_name) {
                    Ok(content) => files.push((
                        format!("{}.cs", crate::codegen::naming::csharp_type_name(result_type)),
                        content,
                    )),
                    Err(err) => eprintln!(
                        "[alef] gen_visitor(csharp): skip result file — configured result_type `{result_type}` is invalid: {err}"
                    ),
                }
            } else {
                eprintln!(
                    "[alef] gen_visitor(csharp): skip result file — configured result_type `{result_type}` is absent from IR"
                );
            }
        }
    } else if trait_def.methods.iter().any(|method| method.trait_source.is_none()) {
        eprintln!(
            "[alef] gen_visitor(csharp): skip result file — trait bridge `{}` has no result_type metadata",
            bridge_cfg.trait_name
        );
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

    // Canonical visitor FFI functions (Path 1: callbacks struct approach used by Go/Java):
    // - htm_visitor_create(HtmVisitorCallbacks* callbacks) -> HtmVisitor*
    // - htm_visitor_free(HtmVisitor* visitor)
    // - htm_options_set_visitor(HtmConversionOptions* opts, HtmVisitor* visitor)
    let fn_options_set = format!("{prefix}_options_set_{options_field}");
    let bridge_name = crate::codegen::naming::csharp_type_name(trait_name) + "Bridge";
    let options_name = crate::codegen::naming::csharp_type_name(options_type);
    let field_name = crate::codegen::naming::to_csharp_name(options_field);

    let mut out = String::from("\n");
    out.push_str(&render(
        "native_methods_visitor.jinja",
        Value::from_serialize(serde_json::json!({
            "fn_options_set": fn_options_set,
            "bridge_name": bridge_name,
            "options_name": options_name,
            "field_name": field_name,
        })),
    ));

    let _ = namespace;
    let _ = lib_name;
    let _ = prefix;
    let _ = trait_name;
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

fn gen_node_context(namespace: &str, context_def: &crate::core::ir::TypeDef) -> String {
    use crate::backends::csharp::template_env::render;
    use crate::backends::csharp::type_map::csharp_type_for_dto_field;
    use crate::codegen::naming::{csharp_type_name, to_csharp_name, wire_field_name};
    use minijinja::Value;

    let fields = crate::codegen::shared::binding_fields(&context_def.fields)
        .map(|field| {
            let mut cs_type = csharp_type_for_dto_field(&field.ty).to_string();
            if field.optional && !cs_type.ends_with('?') {
                cs_type.push('?');
            }
            serde_json::json!({
                "cs_name": to_csharp_name(&field.name),
                "cs_type": cs_type,
                "wire_name": wire_field_name(
                    &field.name,
                    field.serde_rename.as_deref(),
                    context_def.serde_rename_all.as_deref(),
                ),
            })
        })
        .collect::<Vec<_>>();

    render(
        "node_context.jinja",
        Value::from_serialize(serde_json::json!({
            "header": hash::header(CommentStyle::DoubleSlash),
            "namespace": namespace,
            "context_type": csharp_type_name(&context_def.name),
            "fields": fields,
        })),
    )
}

fn gen_visit_result(namespace: &str, enum_def: &crate::core::ir::EnumDef, trait_name: &str) -> anyhow::Result<String> {
    use crate::backends::csharp::template_env::render;
    use crate::codegen::naming::{csharp_type_name, to_csharp_name, wire_variant_value};
    use minijinja::Value;

    let result_metadata =
        crate::codegen::visitor_result::visitor_result_metadata_from_enum_checked(enum_def, trait_name)?;
    let result_type = csharp_type_name(&enum_def.name);
    let unit_variants = enum_def
        .variants
        .iter()
        .filter(|variant| variant.fields.is_empty() && !variant.originally_had_data_fields)
        .map(|variant| {
            serde_json::json!({
                "cs_name": to_csharp_name(&variant.name),
                "wire_name": wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ),
            })
        })
        .collect::<Vec<_>>();
    let payload_variants = enum_def
        .variants
        .iter()
        .filter(|variant| variant.fields.len() == 1 && matches!(variant.fields[0].ty, crate::core::ir::TypeRef::String))
        .map(|variant| {
            let field = &variant.fields[0];
            let payload_property = if field.name.starts_with('_') {
                "Value".to_string()
            } else {
                to_csharp_name(field.name.trim_start_matches('_'))
            };
            serde_json::json!({
                "cs_name": to_csharp_name(&variant.name),
                "payload_property": payload_property,
                "wire_name": wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ),
            })
        })
        .collect::<Vec<_>>();
    let default_wire_name = result_metadata.default_variant.wire_name;

    Ok(render(
        "visit_result.jinja",
        Value::from_serialize(serde_json::json!({
            "header": hash::header(CommentStyle::DoubleSlash),
            "namespace": namespace,
            "result_type": result_type,
            "unit_variants": unit_variants,
            "payload_variants": payload_variants,
            "default_wire_name": default_wire_name,
        })),
    ))
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
    use super::{callback_specs_from_trait, gen_visitor_files};
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{
        ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
    };

    #[test]
    fn emits_configured_context_and_result_files_from_metadata() {
        let api = api();
        let bridge_cfg = bridge_cfg(Some("RenderContext"), Some("FlowDecision"));
        let trait_def = api.types.iter().find(|typ| typ.name == "MarkupVisitor").unwrap();
        let files = gen_visitor_files("Sample", &api, &bridge_cfg, trait_def);

        let filenames = files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
        assert_eq!(filenames, vec!["RenderContext.cs", "FlowDecision.cs"]);
        let context = &files[0].1;
        assert!(context.contains("public record RenderContext("));
        assert!(context.contains("[property: JsonPropertyName(\"node_kind\")] string Kind"));
        assert!(context.contains("[property: JsonPropertyName(\"depth\")] ulong Depth"));

        let result = &files[1].1;
        assert!(result.contains("public abstract record FlowDecision"));
        assert!(result.contains("public sealed record Proceed : FlowDecision;"));
        assert!(result.contains("public sealed record DropNode : FlowDecision;"));
        assert!(result.contains("public sealed record ReplaceWith(string Markdown) : FlowDecision;"));
        assert!(result.contains("FlowDecision.Proceed => \"\\\"go_on\\\"\""));
        assert!(result.contains("FlowDecision.ReplaceWith c => \"{\\\"swap\\\":\""));
        assert!(result.contains("_ => \"\\\"go_on\\\"\""));
        assert!(!result.contains("VisitResult"));
        assert!(!result.contains("Continue"));
        assert!(!result.contains("PreserveHtml"));
        assert!(!result.contains("Custom"));
    }

    #[test]
    fn skips_visitor_files_when_metadata_is_absent() {
        let api = api();
        let bridge_cfg = bridge_cfg(None, None);
        let trait_def = api.types.iter().find(|typ| typ.name == "MarkupVisitor").unwrap();
        let files = gen_visitor_files("Sample", &api, &bridge_cfg, trait_def);

        assert!(files.is_empty());
    }

    #[test]
    fn callback_specs_skip_only_configured_context_named_type() {
        let trait_def = trait_def(
            "MarkupVisitor",
            vec![
                method(
                    "visit_node",
                    vec![
                        param("context", TypeRef::Named("RenderContext".to_string())),
                        param("label", TypeRef::String),
                    ],
                    TypeRef::Named("FlowDecision".to_string()),
                ),
                method(
                    "visit_with_payload",
                    vec![
                        param("context", TypeRef::Named("RenderContext".to_string())),
                        param("payload", TypeRef::Named("Payload".to_string())),
                    ],
                    TypeRef::Named("FlowDecision".to_string()),
                ),
            ],
        );

        let callbacks = callback_specs_from_trait(&trait_def, "RenderContext");

        assert_eq!(callbacks.len(), 1);
        assert_eq!(callbacks[0].c_field, "visit_node");
        assert_eq!(callbacks[0].extra[0].cs_name, "label");
    }

    fn api() -> ApiSurface {
        ApiSurface {
            crate_name: "sample".to_string(),
            version: "0.1.0".to_string(),
            types: vec![
                TypeDef {
                    name: "RenderContext".to_string(),
                    fields: vec![
                        field("kind", TypeRef::String, Some("node_kind")),
                        field("depth", TypeRef::Primitive(PrimitiveType::U64), None),
                    ],
                    serde_rename_all: Some("camelCase".to_string()),
                    ..type_def("RenderContext", vec![])
                },
                trait_def(
                    "MarkupVisitor",
                    vec![method(
                        "visit_node",
                        vec![param("context", TypeRef::Named("RenderContext".to_string()))],
                        TypeRef::Named("FlowDecision".to_string()),
                    )],
                ),
            ],
            functions: vec![],
            enums: vec![EnumDef {
                name: "FlowDecision".to_string(),
                rust_path: "sample::FlowDecision".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Proceed".to_string(),
                        is_default: true,
                        serde_rename: Some("go_on".to_string()),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "DropNode".to_string(),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "ReplaceWith".to_string(),
                        fields: vec![field("markdown", TypeRef::String, None)],
                        serde_rename: Some("swap".to_string()),
                        ..EnumVariant::default()
                    },
                ],
                methods: vec![],
                doc: String::new(),
                cfg: None,
                is_copy: false,
                has_serde: true,
                has_default: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: Some("snake_case".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
            }],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: Default::default(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    fn bridge_cfg(context_type: Option<&str>, result_type: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "MarkupVisitor".to_string(),
            context_type: context_type.map(str::to_string),
            result_type: result_type.map(str::to_string),
            ..TraitBridgeConfig::default()
        }
    }

    fn field(name: &str, ty: TypeRef, serde_rename: Option<&str>) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: crate::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: serde_rename.map(str::to_string),
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    fn trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            is_trait: true,
            ..type_def(name, methods)
        }
    }

    fn type_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
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
            is_trait: false,
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
            has_private_fields: false,
            version: Default::default(),
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
            version: Default::default(),
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
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }
}
