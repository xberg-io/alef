//! C# e2e visitor fixture generation.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{MethodDef, TypeRef};
use crate::e2e::escape::escape_csharp;
use crate::e2e::fixture::CallbackAction;
use heck::ToLowerCamelCase;
use std::fmt::Write as FmtWrite;

use super::stubs::collect_named_types;
use crate::backends::csharp::trait_bridge::csharp_type_visible_pub;

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Build a C# visitor: add an instantiation line to `setup_lines` and push
/// a private nested class declaration to `class_decls` (emitted at class scope,
/// outside any method body — C# does not allow local class declarations inside
/// methods).  Each fixture gets a unique class name derived from its ID to avoid
/// duplicate-name compile errors when multiple visitor fixtures exist per file.
/// Returns the visitor variable name for use as a call argument.
pub(super) fn build_csharp_visitor(
    setup_lines: &mut Vec<String>,
    class_decls: &mut Vec<String>,
    fixture_id: &str,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    visitor_config: &CsharpVisitorConfig,
) -> String {
    use heck::ToUpperCamelCase;
    let class_name = format!("{}Visitor", fixture_id.to_upper_camel_case());
    let var_name = format!("_visitor_{}", fixture_id.replace('-', "_"));

    setup_lines.push(format!("var {var_name} = new {class_name}();"));

    // Build the class declaration string (indented for nesting inside the test class).
    let mut decl = String::new();
    if visitor_config.has_missing_metadata {
        decl.push_str(
            "    #error C# visitor fixtures require trait_bridge context_type, result_type, and IR method metadata; add it to alef.toml or skip visitor fixtures for C#\n",
        );
    }
    // No explicit accessibility modifier: nested types default to `private`
    // (matching the previous behavior inside the e2e test class), while types
    // declared at file/namespace scope after top-level statements default to
    // `internal` — an explicit `private` there is illegal (CS1527).
    decl.push_str(&format!(
        "    sealed class {class_name} : I{}\n",
        visitor_config.trait_name
    ));
    decl.push_str("    {\n");

    // Emit all methods: use fixture action if specified, otherwise default to Continue.
    for method in &visitor_config.methods {
        let method_name = method.name.as_str();
        if let Some(action) = visitor_spec.callbacks.get(method_name) {
            emit_csharp_visitor_method(&mut decl, method_name, action, visitor_config);
        } else {
            // Default: Continue for methods not in the fixture
            emit_csharp_visitor_method(&mut decl, method_name, &CallbackAction::Continue, visitor_config);
        }
    }

    decl.push_str("    }\n");
    class_decls.push(decl);

    var_name
}

pub(super) struct CsharpVisitorConfig {
    trait_name: String,
    context_type: String,
    result_type: String,
    methods: Vec<CsharpVisitorMethod>,
    has_missing_metadata: bool,
}

struct CsharpVisitorMethod {
    name: String,
    params: Option<String>,
}

pub(super) fn resolve_csharp_visitor_config(
    config: &ResolvedCrateConfig,
    call_override: Option<&crate::e2e::config::CallOverride>,
    type_defs: &[crate::core::ir::TypeDef],
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
) -> CsharpVisitorConfig {
    let trait_name = call_override
        .and_then(|override_config| override_config.visitor_trait.clone())
        .or_else(|| {
            type_defs
                .iter()
                .find(|type_def| {
                    type_def.is_trait
                        && visitor_spec
                            .callbacks
                            .keys()
                            .any(|name| type_def.methods.iter().any(|method| method.name == *name))
                })
                .map(|type_def| type_def.name.clone())
        })
        .unwrap_or_else(|| "Visitor".to_string());

    let trait_def = type_defs.iter().find(|type_def| type_def.name == trait_name);
    let bridge = config
        .trait_bridges_for(crate::core::config::Language::Csharp)
        .find(|bridge| bridge.trait_name == trait_name);
    let methods: Vec<CsharpVisitorMethod> = trait_def
        .map(|type_def| {
            type_def
                .methods
                .iter()
                .map(|method| CsharpVisitorMethod {
                    name: method.name.clone(),
                    params: Some(csharp_visitor_params(method)),
                })
                .collect()
        })
        .unwrap_or_else(|| {
            visitor_spec
                .callbacks
                .keys()
                .cloned()
                .map(|name| CsharpVisitorMethod { name, params: None })
                .collect()
        });

    let callback_methods: Vec<&MethodDef> = trait_def
        .map(|type_def| {
            visitor_spec
                .callbacks
                .keys()
                .filter_map(|name| type_def.methods.iter().find(|method| method.name == *name))
                .collect()
        })
        .unwrap_or_default();
    let context_type = bridge.and_then(|bridge| bridge.context_type.clone()).or_else(|| {
        callback_methods
            .iter()
            .find_map(|method| first_named_param_type(method))
    });
    let result_type = bridge.and_then(|bridge| bridge.result_type.clone()).or_else(|| {
        callback_methods
            .iter()
            .find_map(|method| named_type(&method.return_type))
    });
    let has_missing_metadata =
        context_type.is_none() || result_type.is_none() || methods.iter().any(|method| method.params.is_none());

    CsharpVisitorConfig {
        trait_name,
        context_type: context_type.unwrap_or_else(|| "SyntaxContext".to_string()),
        result_type: result_type.unwrap_or_else(|| "WalkDecision".to_string()),
        methods,
        has_missing_metadata,
    }
}

fn csharp_visitor_params(method: &MethodDef) -> String {
    method
        .params
        .iter()
        .map(|param| {
            // No exclusion set here — every named type this visitor signature references is
            // treated as visible, matching the prior behavior of `csharp_type_for_stub`.
            let mut visible_type_names = std::collections::HashSet::new();
            collect_named_types(&param.ty, &mut visible_type_names);
            format!(
                "{} {}",
                csharp_type_visible_pub(&param.ty, &visible_type_names),
                param.name.to_lower_camel_case()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn first_named_param_type(method: &MethodDef) -> Option<String> {
    method.params.iter().find_map(|param| named_type(&param.ty))
}

fn named_type(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(name) => Some(name.clone()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => named_type(inner),
        TypeRef::Map(key, value) => named_type(key).or_else(|| named_type(value)),
        _ => None,
    }
}

/// Emit a C# visitor method into a class declaration string.
fn emit_csharp_visitor_method(
    decl: &mut String,
    method_name: &str,
    action: &CallbackAction,
    visitor_config: &CsharpVisitorConfig,
) {
    let camel_method = method_to_camel(method_name);
    let params = visitor_config
        .methods
        .iter()
        .find(|method| method.name == method_name)
        .and_then(|method| method.params.clone())
        .unwrap_or_else(|| format!("{} context", visitor_config.context_type));

    let (action_type, action_value) = match action {
        CallbackAction::Skip => ("skip", String::new()),
        CallbackAction::Continue => ("continue", String::new()),
        CallbackAction::PreserveHtml => ("preserve_html", String::new()),
        CallbackAction::Custom { output } => ("custom", escape_csharp(output)),
        CallbackAction::CustomTemplate { template, .. } => {
            let camel = snake_case_template_to_camel(template);
            ("custom_template", escape_csharp(&camel))
        }
    };

    let rendered = crate::e2e::template_env::render(
        "csharp/visitor_method.jinja",
        minijinja::context! {
            camel_method => camel_method,
            params => params,
            result_type => &visitor_config.result_type,
            action_type => action_type,
            action_value => action_value,
            // Emit the unsupported-diagnostic stub only when the trait_bridge or
            // IR metadata is incomplete. The Jinja template renders generic
            // `new {result_type}.Continue()`/`Skip()`/`PreserveHtml()`/`Custom(...)`
            // expressions, so any discriminated-union with that record shape
            // compiles directly — not only the default fallback type. The host
            // project opts in by setting `[[trait_bridges]].result_type` in
            // `alef.toml`.
            unsupported_diagnostic => visitor_config.has_missing_metadata.then(|| {
                format!(
                    "visitor fixture callback '{method_name}' requires explicit e2e metadata for result type '{}'",
                    visitor_config.result_type
                )
            }),
        },
    );
    let _ = write!(decl, "{}", rendered);
}

/// Convert snake_case method names to C# PascalCase.
fn method_to_camel(snake: &str) -> String {
    use heck::ToUpperCamelCase;
    snake.to_upper_camel_case()
}

/// Rewrite `{snake_case}` placeholders in a custom template to `{camelCase}` so
/// they match C# parameter names (which alef emits in camelCase).
fn snake_case_template_to_camel(template: &str) -> String {
    use heck::ToLowerCamelCase;
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut name = String::new();
            while let Some(&nc) = chars.peek() {
                if nc == '}' {
                    chars.next();
                    break;
                }
                name.push(nc);
                chars.next();
            }
            out.push('{');
            out.push_str(&name.to_lower_camel_case());
            out.push('}');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::fixture::VisitorSpec;
    use std::collections::BTreeMap;

    /// Regression test for CS1527: a visitor class declared with an explicit
    /// `private` modifier is illegal when emitted at file/namespace scope
    /// after top-level statements (the docs-snippet context). Omitting the
    /// modifier is legal in both the snippet context (defaults to `internal`)
    /// and the e2e test-class context (nested types default to `private`).
    #[test]
    fn visitor_class_declaration_has_no_explicit_accessibility_modifier() {
        let mut setup_lines = Vec::new();
        let mut class_decls = Vec::new();
        let visitor_spec = VisitorSpec {
            callbacks: BTreeMap::new(),
        };
        let visitor_config = CsharpVisitorConfig {
            trait_name: "HtmlVisitor".to_string(),
            context_type: "NodeContext".to_string(),
            result_type: "VisitResult".to_string(),
            methods: Vec::new(),
            has_missing_metadata: false,
        };

        build_csharp_visitor(
            &mut setup_lines,
            &mut class_decls,
            "visitor_video_skip",
            &visitor_spec,
            &visitor_config,
        );

        assert_eq!(class_decls.len(), 1);
        let decl = &class_decls[0];
        assert!(
            !decl.contains("private"),
            "visitor class declaration must not carry an explicit accessibility \
             modifier (CS1527 at file scope): {decl}"
        );
        assert!(
            decl.contains("sealed class VisitorVideoSkipVisitor : IHtmlVisitor"),
            "expected an unqualified sealed class declaration: {decl}"
        );
    }
}
