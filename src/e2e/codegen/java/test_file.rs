use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::Fixture;
use heck::ToUpperCamelCase;

use super::test_method::render_test_method;
use super::values::{collect_nested_type_names, is_java_builtin_type, is_numeric_type_hint};
use super::visitor::{java_visitor_binding, java_visitor_imports};

fn resolve_handle_config_type(
    arg: &crate::e2e::config::ArgMapping,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    if arg.arg_type != "handle" {
        return None;
    }
    options_type.map(str::to_string).or_else(|| {
        let candidate = format!("{}Config", arg.name.to_upper_camel_case());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    java_group_id: &str,
    binding_pkg: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    uses_harness: bool,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let test_class_name = format!("{}Test", sanitize_filename(category).to_upper_camel_case());

    // If the class_name is fully qualified (contains '.'), import it and use
    // only the simple name for method calls.  Otherwise use it as-is.
    let (import_path, simple_class) = if class_name.contains('.') {
        let simple = class_name.rsplit('.').next().unwrap_or(class_name);
        (class_name, simple)
    } else {
        ("", class_name)
    };

    // Check if any fixture (with its resolved call) will emit MAPPER usage.
    let lang_for_om = "java";
    let needs_object_mapper_for_handle = fixtures.iter().any(|f| {
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang_for_om, f, call_cfg, type_defs);
        recipe.args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = super::super::resolve_field(&f.input, &a.field);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });
    // HTTP fixtures always need ObjectMapper for JSON body comparison.
    let has_http_fixtures = fixtures.iter().any(|f| f.http.is_some());
    let needs_object_mapper = needs_object_mapper_for_handle || has_http_fixtures;

    // Collect all options_type values used (class-level + per-fixture call overrides).
    let mut all_options_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(t) = options_type {
        all_options_types.insert(t.to_string());
    }
    for f in fixtures.iter() {
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        if let Some(ov) = call_cfg.overrides.get(lang_for_om) {
            if let Some(t) = &ov.options_type {
                all_options_types.insert(t.clone());
            }
        }
        // Auto-fallback: when the Java override does not declare an options_type
        // but another non-prefixed binding (csharp/c/go/php/python) does, mirror
        // that name into the import set so the auto-emitted `Type.fromJson(json)`
        // expression compiles. The Java POJO class name matches the Rust source
        // type name for these backends.
        let java_has_type = call_cfg
            .overrides
            .get(lang_for_om)
            .and_then(|o| o.options_type.as_deref())
            .is_some();
        if !java_has_type {
            for cand in ["csharp", "c", "go", "php", "python"] {
                if let Some(o) = call_cfg.overrides.get(cand) {
                    if let Some(t) = &o.options_type {
                        all_options_types.insert(t.clone());
                        break;
                    }
                }
            }
        }
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang_for_om, f, call_cfg, type_defs);
        if f.visitor.is_some() {
            if let Some(binding) = java_visitor_binding(config, type_defs, f.visitor.as_ref(), recipe.options_type) {
                all_options_types.insert(binding.options_type);
            }
        }
        for arg in recipe.args.iter().filter(|arg| arg.arg_type == "handle") {
            let value = super::super::resolve_field(&f.input, &arg.field);
            if value.is_null() || value.is_object() && value.as_object().is_some_and(|o| o.is_empty()) {
                continue;
            }
            if let Some(handle_type) = resolve_handle_config_type(arg, recipe.options_type, type_defs) {
                all_options_types.insert(handle_type);
            }
        }
        // Detect complex json_object array element types used in this fixture.
        for arg in &call_cfg.args {
            if let Some(elem_type) = &arg.element_type {
                if arg.arg_type == "json_object" && !is_numeric_type_hint(elem_type) && !is_java_builtin_type(elem_type)
                {
                    // Complex types in json_object arrays need JsonUtil.
                    // Skip Java built-in types (String, Boolean, Integer, etc.).
                    all_options_types.insert(elem_type.clone());
                }
            }
        }
    }

    // Collect nested config types actually referenced in fixture builder expressions.
    // Note: enum types don't need explicit imports since they're in the same package.
    let mut nested_types_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in fixtures.iter() {
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        for arg in &call_cfg.args {
            if arg.arg_type == "json_object" {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                if let Some(val) = f.input.get(field) {
                    if !val.is_null() && !val.is_array() {
                        if let Some(obj) = val.as_object() {
                            collect_nested_type_names(obj, nested_types, &mut nested_types_used);
                        }
                    }
                }
            }
        }
    }

    // Effective binding package for FQN imports of binding types
    // (ChatCompletionRequest, etc.). Prefer the explicit `[crates.java] package`
    // wired in via `binding_pkg`; fall back to the package derived from a
    // fully-qualified `class_name` when present.
    let binding_pkg_for_imports: String = if !binding_pkg.is_empty() {
        binding_pkg.to_string()
    } else if !import_path.is_empty() {
        import_path
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Build imports list
    let mut imports: Vec<String> = Vec::new();
    imports.push("import org.junit.jupiter.api.Test;".to_string());
    imports.push("import static org.junit.jupiter.api.Assertions.*;".to_string());

    // Import the test entry-point class itself when it is fully-qualified or
    // when we know the binding package — emit the FQN so javac resolves it.
    if !import_path.is_empty() {
        imports.push(format!("import {import_path};"));
    } else if !binding_pkg_for_imports.is_empty() && !class_name.is_empty() {
        imports.push(format!("import {binding_pkg_for_imports}.{class_name};"));
    }

    if needs_object_mapper {
        imports.push("import com.fasterxml.jackson.databind.ObjectMapper;".to_string());
        imports.push("import com.fasterxml.jackson.datatype.jdk8.Jdk8Module;".to_string());
    }

    // Import all options types used across fixtures (for builder expressions and MAPPER).
    if !all_options_types.is_empty() {
        for opts_type in &all_options_types {
            let qualified = if binding_pkg_for_imports.is_empty() {
                opts_type.clone()
            } else {
                format!("{binding_pkg_for_imports}.{opts_type}")
            };
            imports.push(format!("import {qualified};"));
        }
    }

    // Import nested options types
    if !nested_types_used.is_empty() && !binding_pkg_for_imports.is_empty() {
        for type_name in &nested_types_used {
            imports.push(format!("import {binding_pkg_for_imports}.{type_name};"));
        }
    }

    // Import visitor types when any fixture uses visitor callbacks.
    let has_visitor_fixtures = fixtures.iter().any(|f| f.visitor.is_some());
    if has_visitor_fixtures && !binding_pkg_for_imports.is_empty() {
        for type_name in java_visitor_imports(config, type_defs, fixtures) {
            imports.push(format!("import {binding_pkg_for_imports}.{type_name};"));
        }
    }

    // Import Optional when using builder expressions with optional fields.
    // Also import JsonUtil for `JsonUtil.fromJson(json, Type.class)` calls emitted when
    // options_via resolves to "from_json" (the default whenever an options_type is present).
    if !all_options_types.is_empty() {
        imports.push("import java.util.Optional;".to_string());
        if !binding_pkg_for_imports.is_empty() {
            imports.push(format!("import {binding_pkg_for_imports}.JsonUtil;"));
        }
    }

    // Import streaming DTOs when any fixture is streaming (uses chat_stream
    // or references streaming-virtual fields like `chunks`/`stream_content`).
    // The collect_snippet emits `new ArrayList<ItemType>()` so the item type
    // class must be importable for type inference and method resolution.
    //
    // Use `resolve_is_streaming` so per-call `streaming = false` opt-outs are
    // honoured: consumers like parser-language-pack ship a real `chunks`
    // result field on their non-streaming process result, and would otherwise
    // get a spurious import plus virtual-aggregator accessor expansion on
    // `chunks`-shaped assertions.
    let has_streaming_fixture = fixtures.iter().any(|f| {
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(f, call_cfg.streaming_enabled())
    });
    if has_streaming_fixture && !binding_pkg_for_imports.is_empty() {
        // Derive streaming DTO imports from declared adapters so each project pulls
        // in only the request and item types it actually exposes.
        let mut streaming_imports: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for adapter in adapters {
            if !matches!(adapter.pattern, crate::core::config::extras::AdapterPattern::Streaming) {
                continue;
            }
            if let Some(item) = adapter.item_type.as_deref() {
                let simple = item.rsplit("::").next().unwrap_or(item);
                if !simple.is_empty() {
                    streaming_imports.insert(simple.to_string());
                }
            }
            if let Some(req) = adapter.request_type.as_deref() {
                let simple = req.rsplit("::").next().unwrap_or(req);
                if !simple.is_empty() {
                    streaming_imports.insert(simple.to_string());
                }
            }
        }
        for ty in streaming_imports {
            imports.push(format!("import {binding_pkg_for_imports}.{ty};"));
        }
    }

    // Render all test methods
    let mut fixtures_body = String::new();
    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_method(
            &mut fixtures_body,
            fixture,
            simple_class,
            function_name,
            result_var,
            args,
            options_type,
            result_is_simple,
            e2e_config,
            nested_types,
            nested_types_optional,
            adapters,
            config,
            type_defs,
        );
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    // Render template
    crate::e2e::template_env::render(
        "java/test_file.jinja",
        minijinja::context! {
            header => header,
            java_group_id => java_group_id,
            test_class_name => test_class_name,
            category => category,
            imports => imports,
            needs_object_mapper => needs_object_mapper,
            fixtures_body => fixtures_body,
            uses_harness => uses_harness,
        },
    )
}
