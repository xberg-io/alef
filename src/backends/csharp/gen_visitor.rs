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

/// Returns `(filename, content)` pairs for required configured visitor files.
///
/// IVisitor.cs and VisitorCallbacks.cs are superseded by IVisitor and VisitorCallbacks
/// in TraitBridges.cs which use the configured trait bridge approach. They are intentionally
/// excluded here; stale committed copies are reported (never removed) by
/// `gen_bindings::files::report_unemitted_visitor_files`.
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
                tracing::warn!(
                    "gen_visitor(csharp): skip context file — configured context_type `{context_type}` is absent from IR"
                );
            }
        }
    } else if trait_def.methods.iter().any(|method| method.trait_source.is_none()) {
        tracing::warn!(
            "gen_visitor(csharp): skip context file — trait bridge `{}` has no context_type metadata",
            bridge_cfg.trait_name
        );
    }

    if let Some(result_type) = bridge_cfg.result_type.as_deref() {
        if trait_returns_named_type(trait_def, result_type) {
            if let Some(enum_def) = api.enums.iter().find(|enum_def| enum_def.name == result_type) {
                match gen_visit_result(namespace, enum_def, &bridge_cfg.trait_name, &api.crate_name) {
                    Ok(content) => files.push((
                        format!("{}.cs", crate::codegen::naming::csharp_type_name(result_type)),
                        content,
                    )),
                    Err(err) => tracing::warn!(
                        "gen_visitor(csharp): skip result file — configured result_type `{result_type}` is invalid: {err}"
                    ),
                }
            } else {
                tracing::warn!(
                    "gen_visitor(csharp): skip result file — configured result_type `{result_type}` is absent from IR"
                );
            }
        }
    } else if trait_def.methods.iter().any(|method| method.trait_source.is_none()) {
        tracing::warn!(
            "gen_visitor(csharp): skip result file — trait bridge `{}` has no result_type metadata",
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
/// - `trait_name`: Name of the visitor trait, used only for the emitted section comment
/// - `handle_pinvoke_type`: the caller's `HANDLE_PINVOKE_TYPE` — the single spelling of
///   `AlefHandle` this backend declares. Threaded in rather than restated so the visitor
///   declarations cannot drift from every other handle-carrying `extern`.
/// - `options_setters`: one `(options_type, options_field)` pair per options-field bridge that
///   reaches this backend. The FFI crate emits `{prefix}_options_set_{field}` once **per
///   bridge** (`ffi::gen_bindings::lib_rs.rs:558` loops), while `{prefix}_visitor_create` /
///   `_free` are emitted once for the whole crate (`lib_rs.rs:588` `find_map`), so the setter
///   is the only part of this block that repeats.
///
/// Every parameter and return position below is `AlefHandle` on the FFI side —
/// `ffi/gen_visitor/binding_emission.rs:311-313,340` for create/free and
/// `ffi/templates/options_field_bridge_setter.rs.jinja:14` for the setter — with the sole
/// exception of `visitor_create`'s `callbacks` argument, which really is a
/// `*const {Prefix}VisitorCallbacks` and so stays `IntPtr`. ~keep
pub fn gen_native_methods_visitor(
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    trait_name: &str,
    handle_pinvoke_type: &str,
    options_setters: &[(String, String)],
) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let fn_visitor_create = format!("{prefix}_visitor_create");
    let fn_visitor_free = format!("{prefix}_visitor_free");
    let bridge_name = crate::codegen::naming::csharp_type_name(trait_name) + "Bridge";

    let mut declared_members = std::collections::BTreeSet::new();
    let setters: Vec<serde_json::Value> = options_setters
        .iter()
        .map(|(options_type, options_field)| {
            let member = format!(
                "{}Set{}",
                crate::codegen::naming::csharp_type_name(options_type),
                crate::codegen::naming::to_csharp_name(options_field)
            );
            (member, format!("{prefix}_options_set_{options_field}"))
        })
        .filter(|(member, _)| declared_members.insert(member.clone()))
        .map(|(member, entry_point)| serde_json::json!({ "member": member, "entry_point": entry_point }))
        .collect();

    let mut out = String::from("\n");
    out.push_str(&render(
        "native_methods_visitor.jinja",
        Value::from_serialize(serde_json::json!({
            "fn_visitor_create": fn_visitor_create,
            "fn_visitor_free": fn_visitor_free,
            "bridge_name": bridge_name,
            "handle_type": handle_pinvoke_type,
            "setters": setters,
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

fn gen_visit_result(
    namespace: &str,
    enum_def: &crate::core::ir::EnumDef,
    trait_name: &str,
    host_crate_name: &str,
) -> anyhow::Result<String> {
    use crate::backends::csharp::template_env::render;
    use crate::codegen::naming::{csharp_type_name, to_csharp_name, wire_variant_value};
    use minijinja::Value;

    let result_metadata = crate::codegen::visitor_result::visitor_result_metadata_from_enum_checked(
        enum_def,
        trait_name,
        host_crate_name,
    )?;
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

#[cfg(test)]
mod tests {
    use super::{gen_native_methods_visitor, gen_visitor_files};
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

    /// `node_context.jinja` closes each record parameter with
    /// `{% if not loop.last %},{% endif +%}` — the `+` on `endif` is load bearing:
    /// minijinja's `trim_blocks(true)` (set in `template_env::make_env`) strips the
    /// newline following *any* block tag, including an inline `{% endif %}` that
    /// closes a same-line conditional, so without `+%}` every field's line ending
    /// is eaten and the whole parameter list collapses onto one line. The prior
    /// test above only asserts substrings via `.contains`, which passes whether or
    /// not the fields share a line, so it cannot catch that regression. Assert the
    /// exact per-field line layout instead so reintroducing the bug (dropping the
    /// `+`, or reintroducing the dash on the loop's `endfor` — see `node_context.jinja`
    /// for why that dash independently defeats the `+`) fails this test.
    #[test]
    fn node_context_places_each_field_on_its_own_line() {
        let api = api();
        let bridge_cfg = bridge_cfg(Some("RenderContext"), Some("FlowDecision"));
        let trait_def = api.types.iter().find(|typ| typ.name == "MarkupVisitor").unwrap();
        let files = gen_visitor_files("Sample", &api, &bridge_cfg, trait_def);
        let context = &files[0].1;

        let record_lines: Vec<&str> = context
            .lines()
            .skip_while(|line| !line.starts_with("public record RenderContext("))
            .skip(1)
            .take_while(|line| *line != ");")
            .collect();

        assert_eq!(
            record_lines,
            vec![
                "    [property: JsonPropertyName(\"node_kind\")] string Kind,",
                "    [property: JsonPropertyName(\"depth\")] ulong Depth",
            ],
            "each record parameter must render on its own line; got:\n{context}"
        );
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
    fn native_visitor_methods_use_configured_ffi_symbols() {
        let output = gen_native_methods_visitor(
            "Sample",
            "sample_native",
            "sample",
            "MarkupVisitor",
            "ulong",
            &[("RenderOptions".to_owned(), "renderer".to_owned())],
        );

        assert!(output.contains("EntryPoint = \"sample_visitor_create\""));
        assert!(output.contains("EntryPoint = \"sample_visitor_free\""));
        assert!(output.contains("EntryPoint = \"sample_options_set_renderer\""));
        assert!(!output.contains("htm_"));
        assert!(!output.contains("register_markup_visitor"));
    }

    /// The FFI crate emits one `{prefix}_options_set_{field}` per options-field bridge
    /// (`ffi::gen_bindings::lib_rs.rs:558`) but only one `visitor_create`/`_free` pair
    /// (`lib_rs.rs:588`). Assert the emitted block has exactly that cardinality, and that two
    /// bridges sharing an options field collapse to one member rather than emitting CS0111. ~keep
    #[test]
    fn one_setter_per_options_field_bridge_deduped_by_member_name() {
        let output = gen_native_methods_visitor(
            "Sample",
            "sample_native",
            "sample",
            "MarkupVisitor",
            "ulong",
            &[
                ("RenderOptions".to_owned(), "renderer".to_owned()),
                ("ParseOptions".to_owned(), "inspector".to_owned()),
                ("RenderOptions".to_owned(), "renderer".to_owned()),
            ],
        );

        assert_eq!(output.matches("VisitorCreate(").count(), 1, "{output}");
        assert_eq!(output.matches("VisitorFree(").count(), 1, "{output}");
        assert_eq!(output.matches("RenderOptionsSetRenderer(").count(), 1, "{output}");
        assert_eq!(output.matches("ParseOptionsSetInspector(").count(), 1, "{output}");
    }

    /// Every visitor position is `AlefHandle` on the FFI side, so every one of them must be
    /// declared with the caller's handle spelling — the only `IntPtr` left is
    /// `visitor_create`'s `*const {Prefix}VisitorCallbacks` argument. ~keep
    #[test]
    fn visitor_declarations_carry_handles_with_the_callers_handle_spelling() {
        let output = gen_native_methods_visitor(
            "Sample",
            "sample_native",
            "sample",
            "MarkupVisitor",
            "ulong",
            &[("RenderOptions".to_owned(), "renderer".to_owned())],
        );

        assert!(
            output.contains("internal static extern ulong VisitorCreate(IntPtr callbacksPtr);"),
            "{output}"
        );
        assert!(
            output.contains("internal static extern void VisitorFree(ulong visitor);"),
            "{output}"
        );
        assert!(
            output.contains("internal static extern void RenderOptionsSetRenderer(ulong options, ulong visitor);"),
            "{output}"
        );
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
                serde_content: None,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: Some("snake_case".to_string()),
                rename_all_fields: None,
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
            version: Default::default(),
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
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
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
            cfg: None,
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
