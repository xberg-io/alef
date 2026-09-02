//! Kotlin argument construction and setup helpers.

use heck::ToUpperCamelCase;

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::config::ArgMapping;
use crate::e2e::escape::escape_kotlin;
use crate::e2e::fixture::Fixture;

/// Build setup lines and the argument list for the function call.
///
/// Returns `Ok((setup_lines, args_string))`, or an error when a `test_backend` arg
/// cannot be rendered as a compilable expression (missing/unregistered trait, or the
/// resolved backend's stub emitter is unimplemented) — see the `test_backend` branch
/// below for why this must fail loudly rather than degrade to a placeholder. ~keep
///
/// An optional `json_object` arg with no fixture value is filled from the *declared* parameter,
/// not from the fixture's own `optional` flag. When the core IR says the parameter is
/// `Option<T>`, both Kotlin emitters render it `name: T? = null`
/// (`kotlin_android`'s `facade_param`, and `object_wrapper::methods` for the JVM wrapper), so
/// `null` is emitted — the value the generated signature already defaults to.
///
/// Only when the target does not declare it optional (or no IR is in scope to ask) does this
/// fall back to a constructor: `OptionsType()` for `kotlin_android_style = true`, whose backend
/// emits plain Kotlin data classes with no `.builder()` companion, and
/// `OptionsType.builder().build()` for the Java-facade-backed JVM target, whose DTOs are the
/// Java records reached by typealias. Both of those forms assume a constructor this module
/// cannot verify exists, which is why they are the fallback and not the rule. ~keep
pub(super) struct KotlinArgsContext<'a> {
    pub(super) fixture: &'a Fixture,
    pub(super) class_name: &'a str,
    pub(super) options_type: Option<&'a str>,
    pub(super) fixture_id: &'a str,
    pub(super) kotlin_android_style: bool,
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    /// The IR enum registry. Needed so a `test_backend` stub can look up the real shape
    /// (plain `enum class` vs. sealed class) and default variant of an enum-typed trait
    /// method's return value, instead of guessing a bare `TypeName()` call that targets an
    /// always-private (or, for a sealed class, always-protected) constructor. ~keep
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    /// True for a streaming `owner_type` adapter, where the facade exposes the
    /// call as an instance method on the handle rather than as a positional
    /// argument to a static/client call (`engine.streamItems(req)`, not
    /// `Facade.streamItems(engine, req)`). Mirrors
    /// `JavaArgsContext::owner_handle_is_receiver`: the handle's construction
    /// line is still emitted, only its presence in the positional argument
    /// list is skipped. ~keep
    pub(super) owner_handle_is_receiver: bool,
    /// What the core IR declares about the target's parameters, keyed to whichever of
    /// `"kotlin"`/`"kotlin_android"` this context renders. [`TargetParams::IrAbsent`] keeps the
    /// pre-IR lowering exactly, so a call site with no IR to supply is unaffected. Consulted
    /// only when a fixture's own `ArgMapping::optional` claims an absent value may be defaulted
    /// — see the `arg.optional` branch below for why that claim is not, by itself, a claim about
    /// this target's generated signature. ~keep
    pub(super) target_params: TargetParams<'a>,
}

/// Everything `normalize_typed_json`'s `kotlin_android_style` field-filling needs to decide
/// whether a bare constructor parameter can be honestly materialised in a fixture literal.
/// Bundled into one struct rather than three parameters threaded through every recursive call —
/// `required_field_stub` recurses through the whole type graph and each of these three is needed
/// at every level. ~keep
struct KotlinFillContext<'a> {
    type_defs: &'a [crate::core::ir::TypeDef],
    enum_defaults: std::collections::HashMap<String, String>,
    default_constructible_types: std::collections::HashSet<String>,
}

pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[ArgMapping],
    context: KotlinArgsContext<'_>,
) -> anyhow::Result<(Vec<String>, String)> {
    let KotlinArgsContext {
        fixture,
        class_name,
        options_type,
        fixture_id,
        kotlin_android_style,
        config,
        type_defs,
        enums,
        owner_handle_is_receiver,
        target_params,
    } = context;
    if args.is_empty() {
        return Ok((Vec::new(), String::new()));
    }
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };

    // `MAPPER.readValue(...)` fixture literals for `kotlin_android_style` only: Jackson's
    // `registerKotlinModule()` (`test_file.rs`'s mapper setup) enforces Kotlin's own
    // constructor-required-ness, so a required (no-Kotlin-default) `Named` field the fixture's
    // JSON omits throws `MissingKotlinParameterException` at test run time, even though the
    // field's absence is exactly what a `#[serde(default = "...")]` the extractor could not
    // constant-fold is *for* (see `kotlin_field_default`'s own doc comment). This mirrors that
    // same emitter's `default_constructible_type_names` computation exactly — same inputs, same
    // fixpoint — so a field only gets filled here when its type's Kotlin data class is provably
    // constructible with no arguments in the ACTUAL generated binding, never a guess at what the
    // Rust value would be. Left empty for the non-android JVM target, whose Java-record DTOs
    // carry no such Kotlin-constructor strictness. ~keep
    let kotlin_fill_context = if kotlin_android_style {
        let enum_defaults: std::collections::HashMap<String, String> = enums
            .iter()
            .filter(|candidate| {
                candidate.serde_tag.is_none()
                    && !candidate.serde_untagged
                    && candidate.variants.iter().all(|variant| variant.fields.is_empty())
            })
            .map(|candidate| {
                let default_variant = candidate
                    .variants
                    .iter()
                    .find(|variant| variant.is_default)
                    .map(|variant| variant.name.clone())
                    .unwrap_or_default();
                (candidate.name.clone(), default_variant)
            })
            .collect();
        let default_constructible_types =
            crate::backends::kotlin::default_constructible_type_names(type_defs, &enum_defaults);
        KotlinFillContext {
            type_defs,
            enum_defaults,
            default_constructible_types,
        }
    } else {
        KotlinFillContext {
            type_defs,
            enum_defaults: std::collections::HashMap::new(),
            default_constructible_types: std::collections::HashSet::new(),
        }
    };

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for (index, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value) {
                setup_lines.push(format!("val {} = \"{}\"", arg.name, escape_kotlin(url)));
            } else if fixture.has_host_root_route() {
                setup_lines.push(format!(
                    "val {} = System.getProperty(\"mockServer.{fixture_id}\", (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\")",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "val {} = (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            let value = crate::e2e::codegen::resolve_urls_field(input, &arg.field);
            if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, value) {
                let literals = urls
                    .into_iter()
                    .map(|url| format!("\"{}\"", escape_kotlin(url)))
                    .collect::<Vec<_>>()
                    .join(", ");
                setup_lines.push(format!("val {} = listOf({literals})", arg.name));
                parts.push(arg.name.clone());
                continue;
            }
            // Not preserved as-is: each element is a bare path (`/page1`) that must be
            // resolved against the per-fixture mock-server base at runtime, the same
            // resolution `mock_url` above uses for a single URL. Mirrors
            // `java/args.rs`'s and `typescript/test_file/args.rs`'s `{name}Base` +
            // `.map(...)` — this arm was previously missing here entirely, so a batch
            // fixture's raw relative paths fell through unresolved into
            // `listOf("/page1", "/page2", ...)`, which mock-server does not serve. ~keep
            let paths_literal = value
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| format!("\"{}\"", escape_kotlin(s))))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let name = &arg.name;
            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
            setup_lines.push(format!(
                "val {name}Base = System.getProperty(\"mockServer.{fixture_id}\", System.getenv(\"{env_key}\") ?: ((System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"))"
            ));
            setup_lines.push(format!(
                "val {name} = listOf({paths_literal}).map {{ if (it.startsWith(\"http\")) it else {name}Base + it }}"
            ));
            parts.push(name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("val {} = {class_name}.{constructor_name}(null)", arg.name,));
            } else {
                let name = &arg.name;
                if let Some(config_type) = super::test_file::resolve_handle_config_type(arg, options_type, type_defs) {
                    // `normalize_typed_json` also fills a `MissingKotlinParameterException`-prone
                    // required field (e.g. `CrawlConfig.ssrf`) the fixture's own JSON left out —
                    // see `fill_missing_required_kotlin_fields`. This is the sole path
                    // `createEngine`/other `handle`-typed args' config JSON takes, so a fixture
                    // like `warc_basic_output`'s `{"respect_robots_txt":false,...}` (no `ssrf` key
                    // at all) needs it here, not just the `json_object` DTO-arg path below. ~keep
                    let normalized_config =
                        normalize_typed_json(config_value, &config_type, &kotlin_fill_context);
                    let json_str = serde_json::to_string(&normalized_config).unwrap_or_default();
                    setup_lines.push(format!(
                        "val {name}Config = MAPPER.readValue({}, {config_type}::class.java)",
                        super::values::kotlin_string_literal(&json_str),
                    ));
                    setup_lines.push(format!(
                        "val {} = {class_name}.{constructor_name}({name}Config)",
                        arg.name,
                        name = name,
                    ));
                } else {
                    setup_lines.push(format!("val {} = {class_name}.{constructor_name}(null)", arg.name,));
                }
            }
            // For a streaming owner_type adapter the handle is the instance-method
            // receiver, not a positional argument — emit its construction but omit
            // it from the call's argument list.
            if owner_handle_is_receiver {
                continue;
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "test_backend" {
            let lang = if kotlin_android_style {
                "kotlin_android"
            } else {
                "kotlin"
            };

            // A `test_backend` arg fills a non-null `I{TraitName}` interface parameter.
            // There is no fixture-supplied value to fall back to and no safe default —
            // unlike every other arg branch above, "the trait isn't configured" and
            // "the backend can't build a stub" have no compilable positional value.
            // Fail generation loudly instead of guessing (`null` into a non-null
            // parameter is itself a compile error, not a safe default). ~keep
            let Some(trait_name) = &arg.trait_name else {
                anyhow::bail!(
                    "e2e fixture `{fixture_id}` declares a `test_backend` arg `{}` with no `trait_name` configured; cannot generate a `{lang}` stub without knowing which trait to implement",
                    arg.name
                );
            };
            let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) else {
                anyhow::bail!(
                    "e2e fixture `{fixture_id}` requires trait `{trait_name}` for its `test_backend` arg `{}`, but no `[[crates.trait_bridges]]` entry named `{trait_name}` is configured",
                    arg.name
                );
            };

            // Collect methods from both the main trait and its super-trait (if present).
            // The super-trait methods are needed so stubs implement the full interface.
            let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                .iter()
                .find(|t| t.name == *trait_name)
                .map(|t| t.methods.iter().collect())
                .unwrap_or_default();

            // If there's a super-trait, also collect its methods.
            if let Some(super_trait) = &trait_bridge.super_trait {
                // Extract the simple name from the full path (e.g., "Plugin" from "sample_core::plugins::Plugin").
                let super_trait_simple = super_trait.rsplit("::").next().unwrap_or(super_trait.as_str());
                if let Some(super_type) = type_defs.iter().find(|t| t.name == super_trait_simple) {
                    for method in &super_type.methods {
                        // Only add if not already present (avoid duplicates).
                        if !methods.iter().any(|m| m.name == method.name) {
                            methods.push(method);
                        }
                    }
                }
            }

            // For kotlin_android, filter out methods whose return type or parameters
            // reference types in the `exclude_types` list.  The binding generator
            // omits those methods from the generated interface, so the test stub
            // must not attempt to implement them.
            if kotlin_android_style {
                let excluded: std::collections::HashSet<&str> = config
                    .kotlin_android
                    .as_ref()
                    .map(|c| c.exclude_types.iter().map(String::as_str).collect())
                    .unwrap_or_default();
                if !excluded.is_empty() {
                    methods.retain(|m| {
                        !excluded.iter().any(|ex| m.return_type.references_named(ex))
                            && m.params
                                .iter()
                                .all(|p| !excluded.iter().any(|ex| p.ty.references_named(ex)))
                    });
                }
            }

            // `emit_test_backend` panics rather than return a placeholder when a
            // language has no real `test_backend` stub generator (e.g. Kotlin JVM
            // today) — see `TestBackendEmission`'s doc comment. ~keep
            let emission = crate::e2e::codegen::emit_test_backend(lang, trait_bridge, &methods, fixture, enums, "");
            setup_lines.push(emission.setup_block);
            parts.push(emission.arg_expr);
            continue;
        }

        // Use resolve_field so field = "input" resolves to the whole fixture input.
        let val_resolved = crate::e2e::codegen::resolve_field(input, &arg.field);
        let val: Option<&serde_json::Value> = if val_resolved.is_null() {
            None
        } else {
            Some(val_resolved)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                let target_declares_optional = target_params.declares_param_optional(lang, &arg.name, index, type_defs);
                if target_declares_optional == Some(true) {
                    // The declared parameter is `Option<T>`, and BOTH Kotlin emitters render that
                    // as `name: T? = null` — `facade_param`'s `is_dto_named` branch for
                    // kotlin_android, and `object_wrapper::methods`' `if p.optional { " = null" }`
                    // for the Kotlin/JVM wrapper. `null` is therefore the value the generated
                    // signature itself defaults to, and the only omitted-argument spelling that is
                    // guaranteed to compile.
                    //
                    // A constructor is NOT: `OptionsType()` compiles only for a type in
                    // `backends::kotlin::default_constructible_type_names` (every emitted
                    // constructor parameter carries a Kotlin default — see `c746610e2`, which
                    // exists precisely because a Rust `Default` impl does not imply that), and
                    // `OptionsType.builder().build()` only for a Java record
                    // `backends::java::gen_bindings::types::builders::should_emit_builder` chose
                    // to give a builder factory. Neither authority is reachable from here, and
                    // guessing "yes" produced `No value passed for parameter 'x'` /
                    // `unresolved reference: builder` in generated snippets. Asking the declared
                    // optionality instead removes the need to guess: when the target says the
                    // argument may be omitted, no constructor has to exist at all. ~keep
                    parts.push("null".to_string());
                } else if arg.arg_type == "json_object" {
                    let default_constructor = if let Some(opts_type) = options_type {
                        if kotlin_android_style {
                            format!("{}()", opts_type)
                        } else {
                            format!("{}.builder().build()", opts_type)
                        }
                    } else {
                        // Infer the type from available config types in type_defs.
                        let inferred_type = super::test_file::resolve_handle_config_type(
                            &crate::e2e::config::ArgMapping {
                                name: arg.name.clone(),
                                field: arg.field.clone(),
                                arg_type: "handle".to_string(),
                                optional: arg.optional,
                                owned: false,
                                element_type: None,
                                go_type: None,
                                vec_inner_is_ref: false,
                                trait_name: None,
                            },
                            None,
                            type_defs,
                        )
                        .unwrap_or_else(|| {
                            // Fallback: try the pattern "{field}Config"
                            let candidate = format!("{}Config", arg.name.to_upper_camel_case());
                            if type_defs.iter().any(|t| t.name == candidate) {
                                candidate
                            } else {
                                arg.name.to_upper_camel_case()
                            }
                        });
                        format!("{}()", inferred_type)
                    };
                    parts.push(default_constructor);
                } else {
                    // `arg.optional` is the *fixture author's* claim that the fixture's own
                    // `input` JSON may leave this field out — not a claim about what this
                    // Kotlin target actually declares. The facade signature generator
                    // (`facade_param`) grants a parameter `= null` only when the IR's own
                    // `ParamDef::optional` says so; when it does not, the generated Kotlin
                    // parameter is non-nullable and required, and splicing a bare `null` here
                    // is a compile error (`Null can not be a value of a non-null type`) rather
                    // than the harmless placeholder it is in Java, whose boxed types accept
                    // `null` syntactically. Ask the same authority the signature generator
                    // used before trusting the fixture's claim. ~keep
                    let target_requires_argument = target_declares_optional == Some(false);
                    if target_requires_argument {
                        parts.push(typed_zero_default(&arg.arg_type));
                    } else {
                        parts.push("null".to_string());
                    }
                }
            }
            None | Some(serde_json::Value::Null) => {
                parts.push(typed_zero_default(&arg.arg_type));
            }
            Some(v) => {
                // Typed arrays carry `element_type` and are materialised as `listOf(...)`.
                // For kotlin_android batch APIs the element type is a binding class
                // (e.g. BatchBytesItem) that wraps multiple fields from JSON objects.
                // For JVM bindings, when element_type is present, deserialize objects via Jackson
                // instead of emitting raw JSON strings.
                if arg.arg_type == "json_object" && v.is_array() && arg.element_type.is_some() {
                    let element_type = arg.element_type.as_deref().unwrap();
                    let mock_base_var = if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                        let base_var = format!("{}MockBaseUrl", arg.name);
                        setup_lines.push(format!(
                            "val {base_var} = System.getProperty(\"mockServer.{fixture_id}\", System.getenv(\"{env_key}\") ?: (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") + \"/fixtures/{fixture_id}\"))"
                        ));
                        Some(base_var)
                    } else {
                        None
                    };
                    let items: Vec<String> = v
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|item| {
                                    // For object items, deserialize via Jackson to the element type
                                    if item.is_object() {
                                        let normalized = crate::e2e::codegen::transform_json_keys_for_language(item, "snake_case");
                                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                                        let literal = super::values::kotlin_string_literal(&json_str);
                                        if let Some(base_var) = mock_base_var.as_deref()
                                            && crate::e2e::codegen::value_contains_mock_url_placeholder(item)
                                        {
                                            format!(
                                                "MAPPER.readValue({literal}.replace(\"{}\", {base_var}), {element_type}::class.java)",
                                                escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                                            )
                                        } else {
                                            format!("MAPPER.readValue({literal}, {element_type}::class.java)")
                                        }
                                    } else if element_type == "String" {
                                        if let Some(raw) = item.as_str()
                                            && let Some(base_var) = mock_base_var.as_deref()
                                            && raw.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                                        {
                                            format!(
                                                "\"{}\".replace(\"{}\", {base_var})",
                                                escape_kotlin(raw),
                                                escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                                            )
                                        } else {
                                            super::values::json_to_kotlin(item)
                                        }
                                    } else if let Some(path) = item.as_str() {
                                        // For string items (file paths), construct the element with the path
                                        if kotlin_android_style {
                                            format!(
                                                "{element_type}(java.nio.file.Files.readAllBytes(java.nio.file.Paths.get(\"{}\")), java.nio.charset.StandardCharsets.UTF_8)",
                                                escape_kotlin(path)
                                            )
                                        } else {
                                            // JVM version takes Path objects, not ByteArray
                                            format!(
                                                "{element_type}(java.nio.file.Paths.get(\"{}\"))",
                                                escape_kotlin(path)
                                            )
                                        }
                                    } else {
                                        // Fallback for other literal types
                                        super::values::json_to_kotlin(item)
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    parts.push(format!("listOf({})", items.join(", ")));
                    continue;
                }
                // For json_object args, deserialize via Jackson or use pre-deserialized variable.
                //
                // This is the sole emitter of the `val {arg.name} = MAPPER.readValue(...)`
                // binding for json_object args: it is shared by both the e2e test emitter
                // (test_method.rs) and standalone docs snippets (snippet.rs), and must be
                // fully self-contained for both callers — neither duplicates this logic. ~keep
                if arg.arg_type == "json_object" {
                    if let Some(opts_type) =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, v)
                    {
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                            // The mock server's base URL is only known at test run time, so
                            // the placeholder is swapped in via a runtime `.replace(...)`
                            // rather than baked into the literal at codegen time. Doc-file
                            // markers are not combined with this path, mirroring the
                            // config_type-inference branch below. ~keep
                            let json_value = normalize_typed_json(v, opts_type, &kotlin_fill_context);
                            let json_str = serde_json::to_string(&json_value).unwrap_or_default();
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            let base_var = format!("{}MockBaseUrl", arg.name);
                            let json_var = format!("{}Json", arg.name);
                            setup_lines.push(format!(
                                "val {base_var} = System.getProperty(\"mockServer.{fixture_id}\", System.getenv(\"{env_key}\") ?: ((System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"))"
                            ));
                            setup_lines.push(format!(
                                "val {json_var} = {}.replace(\"{}\", {base_var})",
                                super::values::kotlin_string_literal(&json_str),
                                escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                            ));
                            setup_lines.push(format!(
                                "val {} = MAPPER.readValue({json_var}, {opts_type}::class.java)",
                                arg.name
                            ));
                        } else {
                            let files = fixture.docs_files_for_arg(&arg.field);
                            let mut json_value = v.clone();
                            let file_reads = prepare_docs_file_reads(&mut json_value, &files);
                            json_value = normalize_typed_json(&json_value, opts_type, &kotlin_fill_context);
                            append_docs_file_setup(&mut setup_lines, &arg.name, &json_value, opts_type, &file_reads);
                        }
                        parts.push(arg.name.clone());
                    } else {
                        // Infer the config type and deserialize
                        let config_type = super::test_file::resolve_handle_config_type(
                            &crate::e2e::config::ArgMapping {
                                name: arg.name.clone(),
                                field: arg.field.clone(),
                                arg_type: "handle".to_string(),
                                optional: arg.optional,
                                owned: false,
                                element_type: None,
                                go_type: None,
                                vec_inner_is_ref: false,
                                trait_name: None,
                            },
                            None,
                            type_defs,
                        )
                        .unwrap_or_else(|| {
                            // Fallback to derived type
                            let candidate = format!("{}Config", arg.name.to_upper_camel_case());
                            if type_defs.iter().any(|t| t.name == candidate) {
                                candidate
                            } else {
                                arg.name.to_upper_camel_case()
                            }
                        });

                        // Setup deserialization
                        let files = fixture.docs_files_for_arg(&arg.field);
                        let mut json_value = v.clone();
                        let file_reads = files
                            .iter()
                            .enumerate()
                            .filter_map(|(index, file)| {
                                let marker = format!("__ALEF_DOC_FILE_{index}__");
                                let target = if file.field.is_empty() {
                                    Some(&mut json_value)
                                } else {
                                    json_value.pointer_mut(&file.field)
                                }?;
                                *target = serde_json::Value::String(marker.clone());
                                Some((index, marker, file.path.clone()))
                            })
                            .collect::<Vec<_>>();
                        let json_value =
                            normalize_typed_json(&json_value, &config_type, &kotlin_fill_context);
                        let json_str = serde_json::to_string(&json_value).unwrap_or_default();
                        let var_name = format!("{}_Config", arg.name);
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            let base_var = format!("{}MockBaseUrl", arg.name);
                            let json_var = format!("{}Json", var_name);
                            setup_lines.push(format!(
                                "val {base_var} = System.getProperty(\"mockServer.{fixture_id}\", System.getenv(\"{env_key}\") ?: ((System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"))"
                            ));
                            setup_lines.push(format!(
                                "val {json_var} = {}.replace(\"{}\", {base_var})",
                                super::values::kotlin_string_literal(&json_str),
                                crate::e2e::escape::escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                            ));
                            setup_lines.push(format!(
                                "val {var_name} = MAPPER.readValue({json_var}, {config_type}::class.java)"
                            ));
                        } else if file_reads.is_empty() {
                            setup_lines.push(format!(
                                "val {var_name} = MAPPER.readValue({}, {config_type}::class.java)",
                                super::values::kotlin_string_literal(&json_str)
                            ));
                        } else {
                            let replacements = file_reads
                                .iter()
                                .map(|(index, marker, _)| format!(".replace(\"{marker}\", {}File{index})", arg.name))
                                .collect::<String>();
                            for (index, _, path) in &file_reads {
                                setup_lines.push(
                                    crate::e2e::template_env::render(
                                        "kotlin/docs_file_read.jinja",
                                        minijinja::context! {
                                            variable => arg.name,
                                            index => index,
                                            path => escape_kotlin(path),
                                        },
                                    )
                                    .trim_end()
                                    .to_string(),
                                );
                            }
                            setup_lines.push(
                                crate::e2e::template_env::render(
                                    "kotlin/snippet_json_object_setup.jinja",
                                    minijinja::context! {
                                        variable => var_name,
                                        json_literal => super::values::kotlin_string_literal(&json_str),
                                        replacements => replacements,
                                        type_name => config_type,
                                    },
                                )
                                .trim_end()
                                .to_string(),
                            );
                        }
                        parts.push(var_name);
                    }
                    continue;
                }
                // bytes args in Kotlin binding carry a relative file path (e.g. "docx/fake.docx")
                // that the Kotlin API resolves and reads internally.
                // - JVM binding: pass the path string directly
                // - android binding: need to read bytes and wrap in ByteArray
                if arg.arg_type == "bytes" {
                    let val = super::values::json_to_kotlin(v);
                    if kotlin_android_style {
                        // kotlin_android needs ByteArray, not String path
                        // Emit code to read the file as bytes
                        if v.is_string() {
                            parts.push(format!(
                                "java.nio.file.Files.readAllBytes(java.nio.file.Paths.get({val}))"
                            ));
                        } else {
                            parts.push("byteArrayOf()".to_string());
                        }
                    } else {
                        parts.push(val);
                    }
                    continue;
                }
                // file_path args: Kotlin module wraps the Java facade (which takes Path),
                // but kotlin_android has a different signature that takes a plain String.
                if arg.arg_type == "file_path" {
                    let val = super::values::json_to_kotlin(v);
                    if kotlin_android_style {
                        // kotlin_android binding takes a plain String path
                        parts.push(val);
                    } else {
                        // Kotlin (JVM) binding re-exports Java facade which takes java.nio.file.Path
                        parts.push(format!("java.nio.file.Path.of({val})"));
                    }
                    continue;
                }
                parts.push(super::values::json_to_kotlin(v));
            }
        }
    }

    Ok((setup_lines, parts.join(", ")))
}

/// A type-appropriate non-null literal for a declared-required Kotlin parameter with no fixture
/// value, keyed on `arg.arg_type`. Shared by the "genuinely required, no value" arm and the
/// "fixture claims optional, but the target requires it" arm so the two can never emit different
/// placeholders for the same situation — a bare `null` is only ever safe for a nullable Kotlin
/// type. ~keep
fn typed_zero_default(arg_type: &str) -> String {
    match arg_type {
        "string" => "\"\"".to_string(),
        "int" | "integer" => "0".to_string(),
        "float" | "number" => "0.0".to_string(),
        "bool" | "boolean" => "false".to_string(),
        _ => "null".to_string(),
    }
}

fn normalize_typed_json(value: &serde_json::Value, type_name: &str, ctx: &KotlinFillContext<'_>) -> serde_json::Value {
    let Some(type_def) = ctx.type_defs.iter().find(|candidate| candidate.name == type_name) else {
        return crate::e2e::codegen::transform_json_keys_for_language(value, "snake_case");
    };
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut normalized = serde_json::Map::new();
    for (key, field_value) in object {
        let field = type_def.fields.iter().find(|field| {
            field.name == *key
                || crate::codegen::naming::wire_field_name(
                    &field.name,
                    field.serde_rename.as_deref(),
                    type_def.serde_rename_all.as_deref(),
                ) == *key
        });
        let Some(field) = field else {
            normalized.insert(key.clone(), field_value.clone());
            continue;
        };
        let wire_name = crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            type_def.serde_rename_all.as_deref(),
        );
        normalized.insert(wire_name, normalize_typed_value(field_value, &field.ty, ctx));
    }
    let mut memo = std::collections::HashMap::new();
    let mut visiting = std::collections::HashSet::new();
    fill_missing_required_kotlin_fields(&mut normalized, type_def, ctx, &mut memo, &mut visiting);
    serde_json::Value::Object(normalized)
}

/// Materialise a JSON stub for every constructor-required field a `kotlin_android_style`
/// fixture literal left out, when doing so is provably safe — recursing through nested
/// required types rather than requiring the whole immediate field type to have a compilable
/// zero-arg Kotlin constructor.
///
/// `CrawlConfig.ssrf: SsrfPolicy` (no Kotlin default; `SsrfPolicy::from_env` is env-dependent
/// and genuinely unresolvable, see `kotlin_field_default`'s doc comment) makes `SsrfPolicy`
/// itself default-constructible once every one of *its* fields has a real Kotlin default, so
/// `"ssrf": {}` is enough there. But a field whose type is bare *because one of its own nested
/// fields* is one of these (`UrlExtractionConfig.crawl: crawlberg::CrawlConfig`, itself bare
/// only because of `crawl.ssrf`) is not in `default_constructible_types` as a whole — Jackson
/// cannot synthesise `UrlExtractionConfig()` from `{}` alone, since `crawl` has no Kotlin
/// default either. `required_field_stub` recurses one field at a time instead: reuse
/// `kotlin_field_default` to ask, per field, "does the real binding already give this a
/// default" (skip it — Jackson gets there on its own) or "is it bare" (recurse into a `Named`
/// type's own stub, or refuse the whole containing type when it is a bare scalar/collection —
/// there is no honest JSON literal alef can spell for those, the same "no default" the Kotlin
/// binding itself renders). The net effect for `url: UrlExtractionConfig` is
/// `{"crawl": {"ssrf": {}}}`, not a blind `{}`. ~keep
fn fill_missing_required_kotlin_fields(
    object: &mut serde_json::Map<String, serde_json::Value>,
    type_def: &crate::core::ir::TypeDef,
    ctx: &KotlinFillContext<'_>,
    memo: &mut std::collections::HashMap<String, Option<serde_json::Map<String, serde_json::Value>>>,
    visiting: &mut std::collections::HashSet<String>,
) {
    for field in &type_def.fields {
        if field.binding_excluded || field.serde_skip || field.serde_flatten || field.optional {
            continue;
        }
        let crate::core::ir::TypeRef::Named(nested_type_name) = &field.ty else {
            continue;
        };
        let wire_name = crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            type_def.serde_rename_all.as_deref(),
        );
        if object.contains_key(&wire_name) {
            continue;
        }
        let has_kotlin_default = !crate::backends::kotlin::kotlin_field_default(
            &field.ty,
            field.optional,
            field.typed_default.as_ref(),
            &ctx.enum_defaults,
            &ctx.default_constructible_types,
        )
        .is_empty();
        if has_kotlin_default {
            continue;
        }
        if let Some(stub) = required_field_stub(nested_type_name, ctx, memo, visiting) {
            object.insert(wire_name, serde_json::Value::Object(stub));
        }
    }
}

/// Build the JSON stub `fill_missing_required_kotlin_fields` inserts for one bare `Named`
/// field, recursing into `type_name`'s own bare-but-`Named` fields. `None` when some field
/// along the way is bare and not `Named` (a scalar/`Vec`/`Map` with no honest literal alef can
/// spell) or `type_name` is unknown (opaque/external type this pass cannot see into) — the
/// caller then leaves the parent field exactly as the fixture wrote it. `visiting` guards a
/// recursive type (`type_name` reachable from itself) the same way, since there is no
/// terminating stub for a cycle. `memo` avoids re-walking a type reached from multiple fields.
fn required_field_stub(
    type_name: &str,
    ctx: &KotlinFillContext<'_>,
    memo: &mut std::collections::HashMap<String, Option<serde_json::Map<String, serde_json::Value>>>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if let Some(cached) = memo.get(type_name) {
        return cached.clone();
    }
    if !visiting.insert(type_name.to_string()) {
        return None;
    }
    let result = (|| {
        let type_def = ctx.type_defs.iter().find(|candidate| candidate.name == type_name)?;
        let mut stub = serde_json::Map::new();
        for field in &type_def.fields {
            if field.binding_excluded || field.serde_skip || field.serde_flatten || field.optional {
                continue;
            }
            let has_kotlin_default = !crate::backends::kotlin::kotlin_field_default(
                &field.ty,
                field.optional,
                field.typed_default.as_ref(),
                &ctx.enum_defaults,
                &ctx.default_constructible_types,
            )
            .is_empty();
            if has_kotlin_default {
                continue;
            }
            let crate::core::ir::TypeRef::Named(nested_type_name) = &field.ty else {
                return None;
            };
            let nested_stub = required_field_stub(nested_type_name, ctx, memo, visiting)?;
            let wire_name = crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                type_def.serde_rename_all.as_deref(),
            );
            stub.insert(wire_name, serde_json::Value::Object(nested_stub));
        }
        Some(stub)
    })();
    visiting.remove(type_name);
    memo.insert(type_name.to_string(), result.clone());
    result
}

fn normalize_typed_value(
    value: &serde_json::Value,
    field_type: &crate::core::ir::TypeRef,
    ctx: &KotlinFillContext<'_>,
) -> serde_json::Value {
    match field_type {
        crate::core::ir::TypeRef::Named(name) => normalize_typed_json(value, name, ctx),
        crate::core::ir::TypeRef::Optional(inner) => normalize_typed_value(value, inner, ctx),
        crate::core::ir::TypeRef::Vec(inner) => serde_json::Value::Array(
            value
                .as_array()
                .map(|items| items.iter().map(|item| normalize_typed_value(item, inner, ctx)).collect())
                .unwrap_or_default(),
        ),
        _ => value.clone(),
    }
}

fn prepare_docs_file_reads(
    value: &mut serde_json::Value,
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> Vec<(usize, String, String)> {
    files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            let marker = format!("__ALEF_DOC_FILE_{index}__");
            let target = if file.field.is_empty() {
                Some(&mut *value)
            } else {
                value.pointer_mut(&file.field)
            }?;
            *target = serde_json::Value::String(marker.clone());
            Some((index, marker, file.path.clone()))
        })
        .collect()
}

fn append_docs_file_setup(
    setup_lines: &mut Vec<String>,
    variable: &str,
    value: &serde_json::Value,
    type_name: &str,
    file_reads: &[(usize, String, String)],
) {
    let replacements = file_reads
        .iter()
        .map(|(index, marker, _)| format!(".replace(\"{marker}\", {variable}File{index})"))
        .collect::<String>();
    for (index, _, path) in file_reads {
        setup_lines.push(
            crate::e2e::template_env::render(
                "kotlin/docs_file_read.jinja",
                minijinja::context! { variable => variable, index => index, path => escape_kotlin(path) },
            )
            .trim_end()
            .to_string(),
        );
    }
    let json = serde_json::to_string(value).unwrap_or_default();
    setup_lines.push(
        crate::e2e::template_env::render(
            "kotlin/snippet_json_object_setup.jinja",
            minijinja::context! {
                variable => variable,
                json_literal => super::values::kotlin_string_literal(&json),
                replacements => replacements,
                type_name => type_name,
            },
        )
        .trim_end()
        .to_string(),
    );
}
