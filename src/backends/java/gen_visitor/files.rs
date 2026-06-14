//! Individual Java file generators: result enum, visitor interface, VisitorBridge.

use crate::codegen::naming::{to_class_name, to_java_name};
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, MethodDef, PrimitiveType, TypeDef, TypeRef};

use super::callbacks::{CallbackSpec, ExtraParam};
use super::helpers::{
    callback_descriptor, callback_method_type, gen_handle_method, iface_param_str, sanitize_callback_doc, stub_var_name,
};

/// Number of callbacks per generated `registerStubsN` Java method.
/// Used by both the stub-call list (constructor body) and the stub-method emitter.
const CHUNK_SIZE: usize = 5;

pub(super) struct VisitorGeneration {
    trait_name: String,
    context_type: String,
    /// Java enum type used for the first (discriminant) field of the context_type record.
    /// Used by the VisitorBridge to convert the raw int discriminant read from the C
    /// context struct into the typed enum expected by the record constructor.
    /// `None` when the first field is not a Named enum (defaults to plain int passthrough).
    node_type_enum: Option<String>,
    result_type: String,
    default_variant: String,
    callbacks: Vec<CallbackSpec>,
    result_variants: Vec<ResultVariant>,
}

struct ResultVariant {
    name: String,
    factory_name: String,
    code_name: String,
    code: usize,
    payload_field: Option<String>,
}

pub(super) fn resolve_visitor_generation(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    _class_name: &str,
) -> Option<VisitorGeneration> {
    let bridge = config.trait_bridges.iter().find(|bridge| {
        bridge.bind_via == BridgeBinding::OptionsField
            && !bridge.exclude_languages.contains(&Language::Java.to_string())
    })?;

    let Some(context_type) = bridge.context_type.as_deref() else {
        eprintln!(
            "Skipping Java visitor generation for trait bridge `{}`: missing context_type metadata",
            bridge.trait_name
        );
        return None;
    };
    let Some(result_type) = bridge.result_type.as_deref() else {
        eprintln!(
            "Skipping Java visitor generation for trait bridge `{}`: missing result_type metadata",
            bridge.trait_name
        );
        return None;
    };
    let Some(context_type_def) = api.types.iter().find(|typ| typ.name == context_type && !typ.is_trait) else {
        eprintln!(
            "Skipping Java visitor generation for trait bridge `{}`: context_type `{context_type}` is absent",
            bridge.trait_name
        );
        return None;
    };
    // The first field of the context record is the node-type discriminant. The C ABI delivers
    // it as a raw `int`; the Java record constructor expects the typed enum, so we capture the
    // enum's Java name here and pass it to the VisitorBridge template for `Enum.values()[i]`
    // conversion. Falls back to None (plain int) when the first field isn't a Named enum.
    let node_type_enum = context_type_def.fields.first().and_then(|field| match &field.ty {
        TypeRef::Named(name) if api.enums.iter().any(|e| e.name == *name) => Some(name.clone()),
        _ => None,
    });
    let Some(result_enum) = api.enums.iter().find(|enum_def| enum_def.name == result_type) else {
        eprintln!(
            "Skipping Java visitor generation for trait bridge `{}`: result_type `{result_type}` is absent",
            bridge.trait_name
        );
        return None;
    };
    let Some(trait_def) = api
        .types
        .iter()
        .find(|typ| typ.name == bridge.trait_name && typ.is_trait)
    else {
        eprintln!(
            "Skipping Java visitor generation for trait bridge `{}`: trait definition is absent",
            bridge.trait_name
        );
        return None;
    };

    let metadata = crate::codegen::visitor_result::visitor_result_metadata(api, bridge)?;
    let callbacks = callbacks_from_trait(trait_def, context_type, result_type);
    if callbacks.is_empty() {
        eprintln!(
            "Skipping Java visitor generation for trait bridge `{}`: no methods use `{context_type}` -> `{result_type}`",
            bridge.trait_name
        );
        return None;
    }

    Some(VisitorGeneration {
        trait_name: bridge.trait_name.clone(),
        context_type: context_type.to_string(),
        node_type_enum,
        result_type: result_type.to_string(),
        default_variant: metadata.default_variant.name,
        callbacks,
        result_variants: result_variants_from_enum(result_enum),
    })
}

pub(super) fn gen_visitor_files(package: &str, visitor: &VisitorGeneration) -> Vec<(String, String)> {
    vec![
        (
            format!("{}.java", visitor.result_type),
            gen_visit_result(package, visitor),
        ),
        (
            format!("{}.java", visitor.trait_name),
            gen_visitor_interface(package, visitor),
        ),
        ("VisitorBridge.java".to_string(), gen_visitor_bridge(package, visitor)),
    ]
}

fn gen_visit_result(package: &str, visitor: &VisitorGeneration) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let permits: Vec<String> = visitor
        .result_variants
        .iter()
        .map(|variant| format!("{}.{}", visitor.result_type, variant.name))
        .collect();
    let variants: Vec<_> = visitor
        .result_variants
        .iter()
        .map(|variant| {
            minijinja::context! {
                name => &variant.name,
                factory_name => &variant.factory_name,
                payload_field => variant.payload_field.as_deref(),
            }
        })
        .collect();
    crate::backends::java::template_env::render(
        "visit_result.jinja",
        minijinja::context! {
            header => header,
            package => package,
            result_type => &visitor.result_type,
            default_variant => &visitor.default_variant,
            permits => permits,
            variants => variants,
        },
    )
}

fn gen_visitor_interface(package: &str, visitor: &VisitorGeneration) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    // Scan callback parameter types so we know which java.util.* imports the interface needs.
    // Generic markers (`List<`, `Map<`) on `ExtraParam.java_type` are the simplest reliable
    // detector — Rust trait params like `&[String]` lower to `List<String>` and `&BTreeMap<K, V>`
    // to `Map<K, V>` in the generated Java signature. Missing imports produce javac
    // `cannot find symbol: class List` / `class Map`.
    let mut needs_list_import = false;
    let mut needs_map_import = false;
    for spec in &visitor.callbacks {
        for ep in &spec.extra {
            if ep.java_type.contains("List<") {
                needs_list_import = true;
            }
            if ep.java_type.contains("Map<") {
                needs_map_import = true;
            }
        }
    }
    let callbacks: Vec<_> = visitor
        .callbacks
        .iter()
        .map(|spec| {
            minijinja::context! {
                doc => sanitize_callback_doc(&spec.doc),
                java_method => spec.java_method,
                params => iface_param_str(spec, &visitor.context_type),
            }
        })
        .collect();
    crate::backends::java::template_env::render(
        "visitor_interface.jinja",
        minijinja::context! {
            header => header,
            package => package,
            trait_name => &visitor.trait_name,
            result_type => &visitor.result_type,
            default_variant => &visitor.default_variant,
            callbacks => callbacks,
            needs_list_import => needs_list_import,
            needs_map_import => needs_map_import,
        },
    )
}

/// Wrap arbitrary Java file content with package declaration and imports using the visitor_files template.
/// This demonstrates the usage of visitor_files.jinja template for generic file wrapping.
#[allow(dead_code)]
fn wrap_java_file(package: &str, imports: Vec<String>, content: String) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::backends::java::template_env::render(
        "visitor_files.jinja",
        minijinja::context! {
            header => header,
            package => package,
            imports => imports,
            content => content,
        },
    )
}

/// Generate `VisitorBridge.java` — builds Panama upcall stubs for all 40 callbacks
/// and exposes a `MemorySegment callbacksStruct()` pointing to the C struct.
fn gen_visitor_bridge(package: &str, visitor: &VisitorGeneration) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    let num_fields = visitor.callbacks.len() + 1; // +1 for user_data
    let num_callbacks = visitor.callbacks.len();

    // Build stub_calls list: which registerStubsN method to call at each step
    let num_chunks = visitor.callbacks.chunks(CHUNK_SIZE).count();
    let mut stub_calls = Vec::new();
    for i in 1..=num_chunks {
        stub_calls.push(format!("registerStubs{i}(offset)"));
    }

    // Build stub_methods: the actual method implementations as a list of strings
    let mut stub_methods = Vec::new();
    for (chunk_idx, chunk) in visitor.callbacks.chunks(CHUNK_SIZE).enumerate() {
        let method_num = chunk_idx + 1;
        let mut method = String::new();
        method.push_str("    private long registerStubs");
        method.push_str(&method_num.to_string());
        method.push_str("(\n            final long offset)\n            throws ReflectiveOperationException {\n");
        method.push_str("        long off = offset;\n");
        for spec in chunk {
            let descriptor = callback_descriptor(spec);
            let method_type = callback_method_type(spec);
            let stub_var = stub_var_name(&spec.java_method);
            method.push_str("        // ");
            method.push_str(&spec.c_field);
            method.push('\n');
            method.push_str("        var ");
            method.push_str(&stub_var);
            method.push_str(" = LINKER.upcallStub(\n");
            method.push_str("                LOOKUP.bind(\n");
            method.push_str("                    this, \"");
            method.push_str(&super::helpers::handle_method_name(&spec.java_method));
            method.push_str("\",\n");
            method.push_str("                    ");
            method.push_str(&method_type);
            method.push_str("),\n");
            method.push_str("                ");
            method.push_str(&descriptor);
            method.push_str(",\n");
            method.push_str("                arena);\n");
            method.push_str("        struct.set(ValueLayout.ADDRESS, off, ");
            method.push_str(&stub_var);
            method.push_str(");\n");
            method.push_str("        off += ValueLayout.ADDRESS.byteSize();\n");
        }
        method.push_str("        return off;\n");
        method.push_str("    }\n");
        stub_methods.push(method);
    }

    // Build handle_methods: one per callback as a list of strings
    let mut handle_methods = Vec::new();
    for spec in &visitor.callbacks {
        let mut method = String::new();
        gen_handle_method(&mut method, spec, &visitor.context_type);
        handle_methods.push(method);
    }
    let result_constants: Vec<_> = visitor
        .result_variants
        .iter()
        .map(|variant| {
            minijinja::context! {
                code_name => &variant.code_name,
                code => variant.code,
            }
        })
        .collect();
    let result_cases: Vec<_> = visitor
        .result_variants
        .iter()
        .map(|variant| {
            minijinja::context! {
                result_type => &visitor.result_type,
                variant_name => &variant.name,
                code_name => &variant.code_name,
                payload_field => variant.payload_field.as_deref(),
            }
        })
        .collect();

    crate::backends::java::template_env::render(
        "visitor_bridge.jinja",
        minijinja::context! {
            header => header,
            package => package,
            trait_name => &visitor.trait_name,
            context_type => &visitor.context_type,
            node_type_enum => visitor.node_type_enum.as_deref(),
            result_type => &visitor.result_type,
            num_callbacks => num_callbacks,
            num_fields => num_fields,
            stub_calls => stub_calls,
            stub_methods => stub_methods,
            handle_methods => handle_methods,
            result_constants => result_constants,
            result_cases => result_cases,
        },
    )
}

fn callbacks_from_trait(trait_def: &TypeDef, context_type: &str, result_type: &str) -> Vec<CallbackSpec> {
    trait_def
        .methods
        .iter()
        .filter(|method| method_returns(method, result_type) && method_has_context(method, context_type))
        .filter_map(|method| callback_from_method(method, context_type))
        .collect()
}

fn method_returns(method: &MethodDef, result_type: &str) -> bool {
    matches!(&method.return_type, TypeRef::Named(name) if name == result_type)
}

fn method_has_context(method: &MethodDef, context_type: &str) -> bool {
    method
        .params
        .iter()
        .any(|param| matches!(&param.ty, TypeRef::Named(name) if name == context_type))
}

fn callback_from_method(method: &MethodDef, context_type: &str) -> Option<CallbackSpec> {
    let extra: Option<Vec<_>> = method
        .params
        .iter()
        .filter(|param| !matches!(&param.ty, TypeRef::Named(name) if name == context_type))
        .map(extra_param_from_param)
        .collect();
    let Some(extra) = extra else {
        eprintln!(
            "[alef] gen_visitor(java): skip method `{}` — unsupported callback parameter",
            method.name
        );
        return None;
    };
    Some(CallbackSpec {
        c_field: method.name.clone(),
        java_method: to_java_name(&method.name),
        doc: method.doc.clone(),
        extra,
    })
}

fn extra_param_from_param(param: &crate::core::ir::ParamDef) -> Option<ExtraParam> {
    let name = param.name.as_str();
    let java_name = to_java_name(name);
    match (&param.ty, param.optional) {
        (TypeRef::Vec(inner), false) if matches!(inner.as_ref(), TypeRef::String) => Some(ExtraParam {
            java_name,
            java_type: "List<String>".to_string(),
            c_layouts: vec!["ValueLayout.ADDRESS".to_string(), "ValueLayout.JAVA_LONG".to_string()],
            decode: format!(
                "decodeStringList({}, {})",
                super::helpers::raw_var_name(&to_java_name(name), 0),
                super::helpers::raw_var_name(&to_java_name(name), 1)
            ),
        }),
        (TypeRef::String, _) => Some(ExtraParam {
            java_name,
            java_type: crate::backends::java::type_map::java_type(&param.ty).into_owned(),
            c_layouts: vec!["ValueLayout.ADDRESS".to_string()],
            decode: format!(
                "{}.equals(MemorySegment.NULL) ? null : {}.reinterpret(Long.MAX_VALUE).getString(0)",
                super::helpers::raw_var_name(&to_java_name(name), 0),
                super::helpers::raw_var_name(&to_java_name(name), 0)
            ),
        }),
        (TypeRef::Primitive(PrimitiveType::Bool), false) => Some(ExtraParam {
            java_name,
            java_type: "boolean".to_string(),
            c_layouts: vec!["ValueLayout.JAVA_INT".to_string()],
            decode: format!("{} != 0", super::helpers::raw_var_name(&to_java_name(name), 0)),
        }),
        (TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize), false) => {
            Some(ExtraParam {
                java_name,
                java_type: "long".to_string(),
                c_layouts: vec!["ValueLayout.JAVA_LONG".to_string()],
                decode: super::helpers::raw_var_name(&to_java_name(name), 0),
            })
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
        ) => Some(ExtraParam {
            java_name,
            java_type: "int".to_string(),
            c_layouts: vec!["ValueLayout.JAVA_INT".to_string()],
            decode: format!("(int) {}", super::helpers::raw_var_name(&to_java_name(name), 0)),
        }),
        _ => None,
    }
}

fn result_variants_from_enum(enum_def: &EnumDef) -> Vec<ResultVariant> {
    enum_def
        .variants
        .iter()
        .enumerate()
        .filter_map(|(code, variant)| {
            if variant.fields.is_empty() && !variant.originally_had_data_fields {
                Some(ResultVariant {
                    name: variant.name.clone(),
                    factory_name: java_factory_name(&variant.name),
                    code_name: to_class_name(&variant.name).to_uppercase(),
                    code,
                    payload_field: None,
                })
            } else if variant.fields.len() == 1 && matches!(variant.fields[0].ty, TypeRef::String) {
                Some(ResultVariant {
                    name: variant.name.clone(),
                    factory_name: java_factory_name(&variant.name),
                    code_name: to_class_name(&variant.name).to_uppercase(),
                    code,
                    payload_field: Some(java_payload_field_name(&variant.fields[0].name)),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Compute the Java factory method name for a `VisitResult` variant.
///
/// The variant's snake_case Java name (`continue`) may collide with a Java reserved
/// keyword; escape such names with a trailing underscore so the generated `static`
/// factory compiles (`continue_` instead of `continue`). Non-keyword names pass
/// through `to_java_name` unchanged.
fn java_factory_name(variant_name: &str) -> String {
    let name = to_java_name(variant_name);
    if crate::core::keywords::JAVA_KEYWORDS.contains(&name.as_str()) {
        format!("{name}_")
    } else {
        name
    }
}

/// Compute the Java field identifier for a tuple variant payload.
///
/// Unnamed tuple fields in Rust IR carry positional names (`0`, `_0`, `1`, ...).
/// These are not legal Java identifiers, so substitute the synthetic name `value`.
/// Named struct-variant fields pass through `to_java_name` unchanged.
fn java_payload_field_name(field_name: &str) -> String {
    let trimmed = field_name.trim_start_matches('_');
    if !trimmed.is_empty() && trimmed.parse::<usize>().is_ok() {
        return "value".to_string();
    }
    to_java_name(field_name)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{EnumVariant, FieldDef, ParamDef, ReceiverKind};

    #[test]
    fn gen_visit_result_produces_sealed_interface() {
        let api = visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = visitor_config("DemoVisitor", "VisitContext", "FlowDecision");
        let visitor = resolve_visitor_generation(&api, &config, "Demo").expect("metadata is complete");
        let out = gen_visit_result("dev.sample_crate", &visitor);
        assert!(
            out.contains("public sealed interface FlowDecision"),
            "must define configured sealed result"
        );
        assert!(out.contains("record Proceed()"), "must have configured default variant");
        assert!(out.contains("record DropNode()"), "must have configured unit variant");
        assert!(
            out.contains("record ReplaceWith(String value)"),
            "must have payload variant"
        );
        assert!(!out.contains("VisitResult"), "must not hardcode legacy result name");
        assert!(!out.contains("Continue"), "must not hardcode legacy default variant");
    }

    #[test]
    fn gen_visitor_interface_has_all_callbacks() {
        let api = visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = visitor_config("DemoVisitor", "VisitContext", "FlowDecision");
        let visitor = resolve_visitor_generation(&api, &config, "Demo").expect("metadata is complete");
        let out = gen_visitor_interface("dev.sample_crate", &visitor);
        assert!(
            out.contains("public interface DemoVisitor"),
            "must define configured visitor interface"
        );
        assert!(out.contains("FlowDecision inspect(final VisitContext context, final String label)"));
        assert!(out.contains("return new FlowDecision.Proceed();"));
        assert!(!out.contains("NodeContext"), "must not hardcode legacy context");
        assert!(!out.contains("VisitResult"), "must not hardcode legacy result");
    }

    #[test]
    fn gen_visitor_bridge_produces_class_with_stubs() {
        let api = visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = visitor_config("DemoVisitor", "VisitContext", "FlowDecision");
        let visitor = resolve_visitor_generation(&api, &config, "Demo").expect("metadata is complete");
        let out = gen_visitor_bridge("dev.sample_crate", &visitor);
        assert!(out.contains("final class VisitorBridge"), "must define VisitorBridge");
        assert!(
            out.contains("MemorySegment callbacksStruct()"),
            "must have callbacksStruct method"
        );
        assert!(out.contains("Arena.ofConfined()"), "must use confined Arena");
        assert!(out.contains("LINKER.upcallStub("), "must register upcall stubs");
    }

    #[test]
    fn gen_visitor_bridge_has_encode_visit_result() {
        let api = visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = visitor_config("DemoVisitor", "VisitContext", "FlowDecision");
        let visitor = resolve_visitor_generation(&api, &config, "Demo").expect("metadata is complete");
        let out = gen_visitor_bridge("dev.sample_crate", &visitor);
        assert!(out.contains("encodeVisitResult"), "must have encodeVisitResult helper");
        assert!(
            out.contains("VISIT_RESULT_PROCEED"),
            "must have configured default result constant"
        );
        assert!(out.contains("case FlowDecision.Proceed ignored -> VISIT_RESULT_PROCEED;"));
        assert!(out.contains("case FlowDecision.ReplaceWith c ->"));
        assert!(out.contains("c.value()"));
        assert!(!out.contains("PreserveHtml"), "must not hardcode HTML result variant");
        assert!(!out.contains("markdown()"), "must not hardcode markdown payload");
    }

    #[test]
    fn gen_visitor_bridge_chunk_counts_consistent() {
        let api = visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = visitor_config("DemoVisitor", "VisitContext", "FlowDecision");
        let visitor = resolve_visitor_generation(&api, &config, "Demo").expect("metadata is complete");
        let src = gen_visitor_bridge("dev.test", &visitor);
        let expected = visitor.callbacks.len().div_ceil(CHUNK_SIZE);
        let stub_call_count = src.matches("offset = registerStubs").count();
        let stub_method_count = src.matches("private long registerStubs").count();
        assert_eq!(
            stub_call_count, expected,
            "constructor must invoke every registerStubsN; got {} calls, expected {}",
            stub_call_count, expected
        );
        assert_eq!(
            stub_method_count, expected,
            "must emit one registerStubsN method per chunk; got {} methods, expected {}",
            stub_method_count, expected
        );
    }

    #[test]
    fn callbacks_skip_methods_with_extra_named_dto_params() {
        let mut api = visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let trait_def = api
            .types
            .iter_mut()
            .find(|typ| typ.name == "DemoVisitor")
            .expect("visitor trait exists");
        trait_def.methods.push(MethodDef {
            name: "inspect_with_payload".to_string(),
            params: vec![
                param("context", TypeRef::Named("VisitContext".to_string())),
                param("payload", TypeRef::Named("Payload".to_string())),
            ],
            return_type: TypeRef::Named("FlowDecision".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Inspect with an unsupported named payload.".to_string(),
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
        });
        let config = visitor_config("DemoVisitor", "VisitContext", "FlowDecision");

        let visitor = resolve_visitor_generation(&api, &config, "Demo").expect("metadata is complete");

        assert_eq!(
            visitor.callbacks.len(),
            1,
            "Java visitor callbacks must stay aligned with FFI-supported callback params"
        );
        assert_eq!(visitor.callbacks[0].c_field, "inspect");
    }

    pub(crate) fn visitor_config(trait_name: &str, context_type: &str, result_type: &str) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: trait_name.to_string(),
                type_alias: Some(format!("{trait_name}Handle")),
                bind_via: BridgeBinding::OptionsField,
                options_type: Some("RunOptions".to_string()),
                options_field: Some("observer".to_string()),
                context_type: Some(context_type.to_string()),
                result_type: Some(result_type.to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    pub(crate) fn visitor_config_without_associated_types(trait_name: &str) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: trait_name.to_string(),
                bind_via: BridgeBinding::OptionsField,
                options_type: Some("RunOptions".to_string()),
                options_field: Some("observer".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    pub(crate) fn visitor_api(trait_name: &str, context_type: &str, result_type: &str) -> ApiSurface {
        ApiSurface {
            crate_name: "demo".to_string(),
            version: "0.1.0".to_string(),
            types: vec![
                type_def(
                    context_type,
                    false,
                    vec![],
                    vec![
                        field("node_type", TypeRef::Primitive(PrimitiveType::I32)),
                        field("tag_name", TypeRef::String),
                        field("depth", TypeRef::Primitive(PrimitiveType::U64)),
                        field("index_in_parent", TypeRef::Primitive(PrimitiveType::U64)),
                        optional_field("parent_tag", TypeRef::String),
                        field("is_inline", TypeRef::Primitive(PrimitiveType::Bool)),
                    ],
                ),
                type_def(
                    trait_name,
                    true,
                    vec![MethodDef {
                        name: "inspect".to_string(),
                        params: vec![
                            param("context", TypeRef::Named(context_type.to_string())),
                            param("label", TypeRef::String),
                        ],
                        return_type: TypeRef::Named(result_type.to_string()),
                        is_async: false,
                        is_static: false,
                        error_type: None,
                        doc: "Inspect a neutral node.".to_string(),
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
                    }],
                    vec![],
                ),
            ],
            functions: vec![],
            enums: vec![EnumDef {
                name: result_type.to_string(),
                rust_path: format!("demo::{result_type}"),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Proceed".to_string(),
                        is_default: true,
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "DropNode".to_string(),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "ReplaceWith".to_string(),
                        fields: vec![field("value", TypeRef::String)],
                        is_tuple: true,
                        ..EnumVariant::default()
                    },
                ],
                doc: String::new(),
                cfg: None,
                is_copy: false,
                has_serde: true,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
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

    fn type_def(name: &str, is_trait: bool, methods: Vec<MethodDef>, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            fields,
            methods,
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: !is_trait,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    fn optional_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            optional: true,
            ..field(name, ty)
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
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    #[test]
    fn java_factory_name_escapes_reserved_keywords() {
        assert_eq!(java_factory_name("Continue"), "continue_");
        assert_eq!(java_factory_name("Default"), "default_");
        assert_eq!(java_factory_name("Final"), "final_");
        // Non-keyword names pass through unchanged.
        assert_eq!(java_factory_name("Skip"), "skip");
        assert_eq!(java_factory_name("PreserveHtml"), "preserveHtml");
        assert_eq!(java_factory_name("Custom"), "custom");
    }

    #[test]
    fn java_payload_field_name_replaces_tuple_indices() {
        // Unnamed tuple fields ("0", "_0", "1") become the synthetic name "value".
        assert_eq!(java_payload_field_name("0"), "value");
        assert_eq!(java_payload_field_name("_0"), "value");
        assert_eq!(java_payload_field_name("1"), "value");
        assert_eq!(java_payload_field_name("_42"), "value");
        // Named struct-variant fields pass through `to_java_name`.
        assert_eq!(java_payload_field_name("value"), "value");
        assert_eq!(java_payload_field_name("payload_text"), "payloadText");
    }
}
