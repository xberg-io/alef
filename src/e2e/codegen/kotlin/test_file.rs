use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::Fixture;
use heck::ToUpperCamelCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

pub(super) fn resolve_handle_config_type(
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
pub(crate) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    render_test_file_inner(
        category,
        fixtures,
        class_name,
        function_name,
        kotlin_pkg_id,
        result_var,
        args,
        options_type,
        result_is_simple,
        e2e_config,
        type_enum_fields,
        false,
        config,
        type_defs,
    )
}

/// Variant of [`render_test_file`] used by the kotlin_android backend.
///
/// `kotlin_android_style = true` shifts two emission decisions:
///
/// 1. Every emitted `@Test` body is wrapped in `runBlocking { ... }` so the
///    suspend-only public API (the kotlin_android AAR exposes most
///    extraction entry points as `suspend fun`) can be invoked from
///    JUnit's non-suspend `@Test` methods. JVM Kotlin tests keep the
///    previous behaviour and only wrap when a `client_factory` is in play.
/// 2. Option-returning APIs are treated as Kotlin nullable `T?` (the
///    kotlin-android wrapper unwraps Java `Optional<T>` to `T?` at the
///    boundary), so `is_empty` / `not_empty` assertions on a bare option
///    result emit `== null` / `!= null` instead of `.isEmpty` /
///    `.isPresent`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_test_file_android(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    render_test_file_inner(
        category,
        fixtures,
        class_name,
        function_name,
        kotlin_pkg_id,
        result_var,
        args,
        options_type,
        result_is_simple,
        e2e_config,
        type_enum_fields,
        true,
        config,
        type_defs,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file_inner(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    kotlin_android_style: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let test_class_name = format!("{}Test", sanitize_filename(category).to_upper_camel_case());

    // If the class_name is fully qualified (contains '.'), import it and use
    // only the simple name for method calls. Otherwise use it as-is.
    let (import_path, simple_class) = if class_name.contains('.') {
        let simple = class_name.rsplit('.').next().unwrap_or(class_name);
        (class_name, simple)
    } else {
        ("", class_name)
    };

    let _ = writeln!(out, "package {kotlin_pkg_id}.e2e");
    let _ = writeln!(out);

    // Detect if any fixture in this group is an HTTP server test.
    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());

    // Detect if any non-HTTP fixture uses a client_factory (coroutine-based client).
    // When true, test functions must use `= runBlocking { ... }` to call suspend fns.
    let has_client_factory_fixtures = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let per_call_factory = cc.overrides.get("kotlin").and_then(|o| o.client_factory.as_deref());
        let global_factory = e2e_config
            .call
            .overrides
            .get("kotlin")
            .and_then(|o| o.client_factory.as_deref());
        per_call_factory.or(global_factory).is_some()
    });

    // Collect every (per-call) options_type referenced by fixtures in this file.
    // Per-call kotlin overrides win over the file-level options_type passed in.
    // Each entry is a json_object arg's options_type — we need to import each one.
    let mut per_fixture_options_types: HashSet<String> = HashSet::new();
    for f in fixtures.iter() {
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let call_overrides = cc.overrides.get("kotlin");
        let effective_opts: Option<String> = call_overrides
            .and_then(|o| o.options_type.clone())
            .or_else(|| options_type.map(|s| s.to_string()))
            .or_else(|| {
                for cand in ["csharp", "c", "go", "php", "python"] {
                    if let Some(o) = cc.overrides.get(cand) {
                        if let Some(t) = &o.options_type {
                            return Some(t.clone());
                        }
                    }
                }
                None
            });
        if let Some(opts) = effective_opts {
            // Prefer the per-call args (which carry the correct arg_type + field for the
            // resolved call); fall back to the file-level args only when the call has none.
            let fixture_args = if cc.args.is_empty() { args } else { cc.args.as_slice() };
            // Import the options type if the fixture either supplies a json_object value
            // (deserialised via ObjectMapper) OR has an *optional* json_object arg with
            // no value — the generator emits `OptionsType.builder().build()` in that
            // case to keep the call arity correct.
            let needs_opts_type = fixture_args.iter().any(|arg| {
                if arg.arg_type != "json_object" {
                    return false;
                }
                let v = crate::e2e::codegen::resolve_field(&f.input, &arg.field);
                !v.is_null() || arg.optional
            });
            if needs_opts_type {
                per_fixture_options_types.insert(opts.to_string());
            }
        }
    }
    let needs_object_mapper_for_options = !per_fixture_options_types.is_empty();

    // Collect trait bridge class names used by fixtures in this file (kotlin_android only).
    // These are specified via call overrides, e.g., class = "ValidatorBridge".
    let mut trait_bridge_classes: HashSet<String> = HashSet::new();
    if kotlin_android_style {
        for f in fixtures.iter() {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            if let Some(overrides) = cc.overrides.get("kotlin_android") {
                if let Some(bridge_class) = &overrides.class {
                    trait_bridge_classes.insert(bridge_class.clone());
                }
            }
        }
    }

    // Also need ObjectMapper when a handle arg has a non-null config.
    let needs_object_mapper_for_handle = fixtures.iter().any(|f| {
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let lang_for_recipe = if kotlin_android_style {
            "kotlin_android"
        } else {
            "kotlin"
        };
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang_for_recipe, f, cc, type_defs);
        recipe.args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = crate::e2e::codegen::resolve_field(&f.input, &a.field);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });
    // HTTP fixtures always need ObjectMapper for JSON body comparison.
    let needs_object_mapper = needs_object_mapper_for_options || needs_object_mapper_for_handle || has_http_fixtures;

    // Detect if any non-error fixture in this group is a streaming call.  The
    // kotlin_android target collects a Flow<T> into a List via `.toList()`, which
    // requires `import kotlinx.coroutines.flow.toList`.
    let has_streaming_fixtures = kotlin_android_style
        && fixtures.iter().any(|f| {
            if f.is_http_test() {
                return false;
            }
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            crate::e2e::codegen::streaming_assertions::resolve_is_streaming(f, cc.streaming_enabled())
        });

    let _ = writeln!(out, "import org.junit.jupiter.api.Test");
    let _ = writeln!(out, "import kotlin.test.assertEquals");
    let _ = writeln!(out, "import kotlin.test.assertTrue");
    let _ = writeln!(out, "import kotlin.test.assertFalse");
    let _ = writeln!(out, "import kotlin.test.assertFailsWith");
    if has_client_factory_fixtures || kotlin_android_style {
        let _ = writeln!(out, "import kotlinx.coroutines.runBlocking");
    }
    // `Flow<T>.toList()` is only available via this import — it is not part of the
    // standard Flow API in Kotlin 1.x/2.x without the explicit import.
    if has_streaming_fixtures {
        let _ = writeln!(out, "import kotlinx.coroutines.flow.toList");
    }
    // Effective binding package for FQN imports. When the binding `class_name` is
    // not fully-qualified, fall back to `kotlin_pkg_id` — the kotlin binding emits
    // top-level typealiases at that package (e.g. `package com.github.sample_core_dev`)
    // while the test files live at `<kotlin_pkg_id>.e2e`. Child packages do NOT
    // import their parent's symbols implicitly, so explicit imports are required.
    let binding_pkg_for_imports: String = if !import_path.is_empty() {
        import_path
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_else(|| kotlin_pkg_id.to_string())
    } else {
        kotlin_pkg_id.to_string()
    };
    // Only import the binding class when there are non-HTTP fixtures that call it.
    let has_call_fixtures = fixtures.iter().any(|f| !f.is_http_test());
    if has_call_fixtures {
        if !import_path.is_empty() {
            let _ = writeln!(out, "import {import_path}");
        } else if !class_name.is_empty() {
            let _ = writeln!(out, "import {binding_pkg_for_imports}.{class_name}");
        }
    }
    if needs_object_mapper {
        let _ = writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper");
        let _ = writeln!(out, "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module");
        // `registerKotlinModule()` is required on the kotlin_android target so that
        // Jackson can deserialise Kotlin data classes (which have no default
        // constructor). The extension function lives in jackson-module-kotlin.
        if kotlin_android_style {
            let _ = writeln!(out, "import com.fasterxml.jackson.module.kotlin.registerKotlinModule");
        }
    }
    // Import every options type referenced by per-call kotlin overrides in this file.
    // Options-type imports are needed for both ObjectMapper deserialisation and for
    // optional-arg defaults emitted as `OptionsType.builder().build()`.
    if has_call_fixtures {
        let mut sorted_opts: Vec<&String> = per_fixture_options_types.iter().collect();
        sorted_opts.sort();
        for opts_type in sorted_opts {
            let _ = writeln!(out, "import {binding_pkg_for_imports}.{opts_type}");
        }
    }
    // Import trait bridge classes used by fixtures (kotlin_android only).
    if !trait_bridge_classes.is_empty() {
        let mut sorted_bridges: Vec<&String> = trait_bridge_classes.iter().collect();
        sorted_bridges.sort();
        for bridge_class in sorted_bridges {
            let _ = writeln!(out, "import {binding_pkg_for_imports}.{bridge_class}");
        }
    }
    let mut handle_config_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in fixtures.iter() {
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let lang_for_recipe = if kotlin_android_style {
            "kotlin_android"
        } else {
            "kotlin"
        };
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang_for_recipe, f, cc, type_defs);
        for arg in recipe.args.iter().filter(|arg| arg.arg_type == "handle") {
            let value = crate::e2e::codegen::resolve_field(&f.input, &arg.field);
            if value.is_null() || value.is_object() && value.as_object().is_some_and(|o| o.is_empty()) {
                continue;
            }
            if let Some(config_type) = resolve_handle_config_type(arg, recipe.options_type, type_defs) {
                handle_config_types.insert(config_type);
            }
        }
    }
    for config_type in handle_config_types {
        let _ = writeln!(out, "import {binding_pkg_for_imports}.{config_type}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "/** E2e tests for category: {category}. */");
    let _ = writeln!(out, "class {test_class_name} {{");

    if needs_object_mapper {
        let _ = writeln!(out);
        let _ = writeln!(out, "    companion object {{");
        // `kotlin_android_style` tests include Kotlin data classes (e.g. ChatCompletionRequest)
        // that have no default constructor. Jackson needs `registerKotlinModule()` to use the
        // primary constructor for deserialization. Non-android (JVM) targets use Java records
        // and builders, which Jackson handles without the extra module.
        let kotlin_module_call = if kotlin_android_style {
            ".registerKotlinModule()"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "        private val MAPPER = ObjectMapper().registerModule(Jdk8Module()){kotlin_module_call}.setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)"
        );
        let _ = writeln!(out, "    }}");
    }

    for fixture in fixtures {
        super::test_method::render_test_method(
            &mut out,
            fixture,
            simple_class,
            function_name,
            result_var,
            args,
            options_type,
            result_is_simple,
            e2e_config,
            type_enum_fields,
            kotlin_android_style,
            config,
            type_defs,
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
}

/// Returns true when `ty` is a `Named(T)` reference (or `Optional<Named(T)>`)
/// where `T` is **not** a known struct name. Such fields are enum-typed and
/// must route through `.getValue()` in generated assertions.
pub(super) fn is_enum_typed(ty: &crate::core::ir::TypeRef, struct_names: &HashSet<&str>) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => !struct_names.contains(name.as_str()),
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(name) if !struct_names.contains(name.as_str()))
        }
        _ => false,
    }
}
