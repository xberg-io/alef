use crate::codegen::naming::{go_error_type_name, go_free_function_name, go_type_name, to_go_name};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, FunctionDef, TypeDef};
use crate::e2e::{config::E2eConfig, fixture::Fixture};
use anyhow::{Result, bail};

use super::adapter_target_params::{flattened_stream_params, target_params_or};
use super::ir_signature::{go_ir_named_type, go_is_bridge_param, go_options_param_is_pointer};
use super::setup::{GoArgsContext, build_args_and_setup};

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> Result<String> {
    let lang = "go";
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    // The snippet path is the only Go emitter that spells typed Go literals, so it is the one
    // that opts into the core-IR seam: `with_functions` turns `target_params` from
    // `IrAbsent` into a real answer about what each argument's parameter is declared as. ~keep
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs)
        .with_functions(functions);
    let flattened_params = flattened_stream_params(config, type_defs, lang, call);
    let target_params = target_params_or(&flattened_params, || recipe.target_params(lang));
    let override_config = recipe.override_config;
    let import_alias = override_config
        .and_then(|value| value.alias.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|value| value.alias.as_deref())
        })
        .unwrap_or("pkg");
    let module = override_config
        .and_then(|value| value.module.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|value| value.module.as_deref())
        })
        .or_else(|| config.go.as_ref().and_then(|value| value.module.as_deref()))
        .unwrap_or(&call.module);
    let reserved_type_names: std::collections::HashSet<String> = type_defs
        .iter()
        .filter(|value| !value.is_trait)
        .map(|value| go_type_name(&value.name))
        .chain(enums.iter().map(|value| go_type_name(&value.name)))
        .collect();
    let base_function = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function);
    let function_name = go_free_function_name(base_function, &reserved_type_names);
    let data_enum_names: std::collections::HashSet<&str> = enums
        .iter()
        .filter(|value| {
            value
                .variants
                .iter()
                .any(|variant| variant.fields.iter().any(|field| !field.name.is_empty()))
        })
        .map(|value| value.name.as_str())
        .collect();
    let options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|value| value.options_type.as_deref())
    });
    // `functions` is the same IR the Go binding backend generated the actual signature
    // from (see `gen_bindings::functions::gen_function_wrapper`). When the target call
    // resolves to a known free function, derive `options_ptr` from its real parameter
    // instead of trusting the hand-authored `options_ptr` override, which can drift from
    // what the binding backend emits. The override remains the fallback for calls this
    // harness cannot resolve to a `FunctionDef` (e.g. method calls, synthetic call
    // names) — those keep today's config-driven behavior unchanged. ~keep
    let core_lookup_name = call.core_lookup_name(lang);
    let target_function = core_lookup_name
        .as_deref()
        .and_then(|name| functions.iter().find(|value| value.name == name));
    let opaque_names: std::collections::HashSet<&str> = type_defs
        .iter()
        .filter(|value| value.is_opaque)
        .map(|value| value.name.as_str())
        .collect();
    let options_param = target_function.and_then(|function| {
        function
            .params
            .iter()
            .find(|param| go_ir_named_type(&param.ty) == options_type)
    });
    let options_ptr = options_param
        .map(|param| go_options_param_is_pointer(param, &opaque_names))
        .unwrap_or_else(|| {
            override_config.is_some_and(|value| value.options_ptr)
                || call.overrides.get(lang).is_some_and(|value| value.options_ptr)
                || e2e_config
                    .call
                    .overrides
                    .get(lang)
                    .is_some_and(|value| value.options_ptr)
        });
    let (mut package_decls, mut setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        fixture,
        GoArgsContext {
            import_alias,
            options_type,
            options_ptr,
            expects_error: false,
            data_enum_names: &data_enum_names,
            config,
            type_defs,
            enums,
            native_dtos: true,
            target: target_params,
        },
    )?;
    let mut configured_arg_count = recipe.args.len();
    if let Some(visitor_spec) = &fixture.visitor {
        // Silently dropping the visitor here published a snippet that compiles but omits
        // the one behaviour the fixture exists to demonstrate, under a heading that still
        // promises it — a reader cannot tell that from a language that legitimately needs
        // no visitor. Fail closed instead, matching `php::snippet` and `csharp::snippet`;
        // a deliberate omission belongs in the fixture's `docs.coverage_exceptions`,
        // which records a reader-visible reason. ~keep
        let Some(options_type) =
            options_type.or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
        else {
            bail!(
                "Go documentation snippet `{}` needs an options type for its visitor",
                fixture.id
            );
        };
        let struct_name = super::visitors::visitor_struct_name(&fixture.id);
        let binding = super::visitors::resolve_go_visitor_binding(config, type_defs, visitor_spec, import_alias);
        let mut declaration = String::new();
        super::visitors::emit_go_visitor_struct(
            &mut declaration,
            &struct_name,
            visitor_spec,
            import_alias,
            binding.as_ref(),
        );
        package_decls.push(declaration);
        setup_lines.push(format!("visitor := &{struct_name}{{}}"));
        // Attach the visitor to the options value the call ALREADY binds, when there is one.
        // Unconditionally introducing a second `opts` object was wrong twice over: the call then
        // carried both bindings (`Convert(html, &options, opts)`, a hard "too many arguments" from
        // the Go compiler, since `replace_go_options` only recognised a literal trailing `nil` and
        // appended in every other case), and the fresh empty object silently discarded whatever
        // options the fixture had actually configured. ~keep
        match bound_options_argument(&setup_lines, &args, recipe.args, options_type) {
            Some(name) => setup_lines.push(format!("{name}.Visitor = visitor")),
            None => {
                setup_lines.push(format!("opts := &{import_alias}.{options_type}{{}}"));
                setup_lines.push("opts.Visitor = visitor".to_string());
                if !args.ends_with(", nil") {
                    configured_arg_count += 1;
                }
                args = replace_go_options(&args);
            }
        }
    }
    if !recipe.extra_args.is_empty() {
        // Bridge/visitor parameters (per `config.trait_bridges`) are real parameters on
        // the extracted Rust function, but the Go binding backend strips them from its
        // emitted signature (see `is_bridge_param` in `gen_bindings::functions`) — so
        // they must not be counted toward the Go-visible arity `extra_args` is clamped
        // against. Falls back to appending every configured `extra_args` verbatim when
        // the call has no resolvable `FunctionDef` (unchanged prior behavior). ~keep
        let bridge_param_names: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|bridge| bridge.param_name.clone())
            .collect();
        let bridge_type_aliases: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|bridge| bridge.type_alias.clone())
            .collect();
        let real_go_param_count = target_function.map(|function| {
            function
                .params
                .iter()
                .filter(|param| !go_is_bridge_param(param, &bridge_param_names, &bridge_type_aliases))
                .count()
        });
        let allowed_extra_args = real_go_param_count
            .map(|limit| limit.saturating_sub(configured_arg_count))
            .unwrap_or(recipe.extra_args.len());
        let extras = recipe.extra_args[..allowed_extra_args.min(recipe.extra_args.len())].join(", ");
        if !extras.is_empty() {
            args = if args.is_empty() {
                extras
            } else {
                format!("{args}, {extras}")
            };
        }
    }
    let client_factory = override_config
        .and_then(|value| value.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|value| value.client_factory.as_deref())
        });
    let (call_prefix, client_setup) = if let Some(factory) = client_factory {
        // The second positional parameter of a generated Go client factory (e.g.
        // `CreateClient(apiKey string, baseURL *string, ...)`) is `baseURL *string` — the
        // same slot every other language's `client_factory` shape reserves for the
        // documented base URL. A Go string literal has no address, so a documented URL
        // needs the `ptr[T any]` helper `build_args_and_setup` already emits for
        // pointer-typed literals elsewhere in this snippet. ~keep
        let base_url_arg = match crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client()) {
            Some(url) => {
                if !package_decls.iter().any(|decl| decl.starts_with("func ptr[")) {
                    package_decls.push("func ptr[T any](value T) *T { return &value }".to_string());
                }
                format!("ptr(\"{}\")", crate::e2e::escape::escape_go(url))
            }
            None => "nil".to_string(),
        };
        let call_line = format!(
            "\tclient, clientErr := {import_alias}.{}(\"your-api-key\", {base_url_arg}, nil, nil, nil)",
            to_go_name(factory),
        );
        // The Go binding backend gives every opaque handle a `Free()` (not `Close()`) and
        // registers no `runtime.SetFinalizer`/`AddCleanup` (`backends/go/templates/opaque_type.jinja`),
        // so a snippet that constructs a client and returns leaks the FFI handle. `defer` is the
        // scope construct rather than a trailing call because the body below panics on the
        // operation's error path, and deferred calls still run while a panic unwinds. It is
        // registered after the construction guard so it is never reached with a nil client. ~keep
        (
            "client".to_string(),
            format!("{call_line}\n\tif clientErr != nil {{\n\t\tpanic(clientErr)\n\t}}\n\tdefer client.Free()"),
        )
    } else {
        (import_alias.to_string(), String::new())
    };
    let call_expr = format!("{call_prefix}.{function_name}({args})");
    let returns_error = override_config
        .and_then(|value| value.returns_result)
        .unwrap_or(call.returns_result)
        || recipe
            .args
            .iter()
            .any(|arg| matches!(arg.arg_type.as_str(), "json_object" | "bytes"))
        || client_factory.is_some();
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let mut standard_imports = std::collections::BTreeSet::new();
    let setup_lines: Vec<String> = setup_lines.into_iter().map(snippet_setup_line).collect();
    let joined_setup = setup_lines.join("\n");
    let joined_declarations = package_decls.join("\n");
    if joined_setup.contains("os.") || joined_declarations.contains("os.") {
        standard_imports.insert("os");
    }
    if joined_setup.contains("json.") {
        standard_imports.insert("encoding/json");
    }
    if joined_setup.contains("strings.") {
        standard_imports.insert("strings");
    }
    if !call.returns_void || expects_error || joined_setup.contains("fmt.") {
        standard_imports.insert("fmt");
    }
    if expects_error {
        standard_imports.insert("errors");
        standard_imports.insert("os");
    }
    let mut imports = standard_imports
        .into_iter()
        .map(|path| (path.to_string(), String::new()))
        .collect::<Vec<_>>();
    imports.push((module.to_string(), import_alias.to_string()));
    imports.sort_by(|left, right| left.0.cmp(&right.0));
    let imports = imports
        .into_iter()
        .map(|(path, alias)| minijinja::context! { path => path, alias => alias })
        .collect::<Vec<_>>();

    let presentation =
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang, type_defs, enums, functions);
    Ok(crate::e2e::template_env::render(
        "go/snippet_body.jinja",
        minijinja::context! {
            imports => imports,
            package_decls => package_decls, setup_lines => setup_lines, client_setup => client_setup,
            call_expr => call_expr, result_var => call.effective_result_var(), returns_error => returns_error,
            returns_void => call.returns_void,
            expects_error => expects_error,
            error_type => go_error_type_name(&config.error_type_name(), &config.go_package_name()),
            import_alias => import_alias,
            presentation => presentation,
        },
    )
    .trim_end()
    .to_string())
}

/// The local variable the call's options argument is already bound to, when the argument builder
/// emitted one and the call passes it in the trailing slot.
///
/// Anchored at the end of `args` rather than split on commas: an earlier argument can be a raw
/// backtick string literal containing commas, so only a suffix match is safe here. ~keep
fn bound_options_argument(
    setup_lines: &[String],
    args: &str,
    configured_args: &[crate::e2e::config::ArgMapping],
    options_type: &str,
) -> Option<String> {
    let candidate = configured_args
        .iter()
        .rev()
        .find(|arg| arg.arg_type == "json_object")
        .map(|arg| arg.name.clone())?;
    let binds_it = setup_lines
        .iter()
        .any(|line| line.starts_with(&format!("{candidate} := ")));
    let passes_it = args.ends_with(&format!(", &{candidate}")) || args.ends_with(&format!(", {candidate}"));
    // The binding is only reusable when it really is an options object; a `json_object` argument of
    // some other DTO type carries no `Visitor` field to set. ~keep
    let is_options = setup_lines
        .iter()
        .any(|line| line.starts_with(&format!("{candidate} := ")) && line.contains(options_type));
    (binds_it && passes_it && is_options).then_some(candidate)
}

fn replace_go_options(args: &str) -> String {
    if let Some(prefix) = args.strip_suffix(", nil") {
        format!("{prefix}, opts")
    } else if args.is_empty() {
        "opts".to_string()
    } else {
        format!("{args}, opts")
    }
}

fn snippet_setup_line(line: String) -> String {
    line.lines()
        .map(|part| {
            if part.contains("t.Fatalf(") {
                format!("{})", part.replace("t.Fatalf(", "panic(fmt.Sprintf("))
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ParamDef, TypeRef};

    /// A visitor fixture whose call already binds an options value must attach the visitor to THAT
    /// binding. Introducing a second `opts` object made the call carry both — `Convert(html,
    /// &options, opts)`, which is a hard "too many arguments" from the Go compiler — and threw away
    /// whatever options the fixture had configured. ~keep
    #[test]
    fn a_visitor_attaches_to_the_options_value_the_call_already_binds() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "html": "<p>hi</p>" });
        fixture.visitor = serde_json::from_value(serde_json::json!({
            "callbacks": {"visit_link": {"action": "skip"}}
        }))
        .expect("visitor spec");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![
            crate::e2e::config::ArgMapping {
                name: "html".into(),
                field: "html".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "options".into(),
                arg_type: "json_object".into(),
                optional: true,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ];
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                fields: Vec::new(),
                ..TypeDef::default()
            }],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("options := pkg.SampleConfig{}"), "{body}");
        assert!(body.contains("options.Visitor = visitor"), "{body}");
        assert!(!body.contains("opts := "), "no second options object: {body}");
        assert!(!body.contains(", opts)"), "the call must not carry two options: {body}");
        assert!(body.contains(", &options)"), "{body}");
    }

    #[test]
    fn visitor_options_replace_nil_argument() {
        assert_eq!(replace_go_options("html, nil"), "html, opts");
        assert_eq!(replace_go_options("html"), "html, opts");
    }
    use crate::e2e::config::{CallConfig, CallOverride};

    fn make_param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
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

    fn fixture() -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "quick_start".to_string(),
            category: None,
            description: "Quick start".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: Vec::new(),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
        }
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let documented: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let e2e = E2eConfig {
            call: CallConfig {
                function: "process".to_string(),
                module: "github.com/example/library".to_string(),
                result_var: "result".to_string(),
                ..CallConfig::default()
            },
            result_fields: ["summary".to_string(), "items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };

        let body = render_snippet_body(&documented, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(body.contains("result := pkg.Process()"), "{body}");
        assert!(body.contains("fmt.Printf(\"%v\\n\", result.Summary)"), "{body}");
        assert!(body.contains("for _, item := range result.Items {"), "{body}");
        assert!(body.contains("fmt.Printf(\"%+v\\n\", item.Label)"), "{body}");
        assert!(
            !body.contains("fmt.Printf(\"%+v\\n\", result)"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
    }

    #[test]
    fn snippet_reuses_the_test_call_shape_without_test_harness() {
        let e2e = E2eConfig {
            call: CallConfig {
                function: "load_document".to_string(),
                module: "github.com/example/library".to_string(),
                result_var: "document".to_string(),
                returns_result: true,
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };
        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(body.contains("pkg \"github.com/example/library\""));
        let fmt_position = body.find("\"fmt\"").expect("fmt import");
        let package_position = body.find("pkg \"").expect("binding import");
        assert!(fmt_position < package_position, "{body}");
        assert!(body.contains("document, err := pkg.LoadDocument()"));
        assert!(!body.contains("testing"));
        assert!(!body.contains("assert."));
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let mut fixture = fixture();
        fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();
        e2e.call.returns_result = true;
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(body.contains("_, err := pkg."), "{body}");
        assert!(body.contains("var typedError pkg.Error"), "{body}");
        assert!(body.contains("errors.As(err, &typedError)"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
    }

    #[test]
    fn snippet_replaces_testing_failures_in_typed_setup() {
        assert_eq!(
            snippet_setup_line("if err != nil {\n\tt.Fatalf(\"decode: %v\", err)\n}".into()),
            "if err != nil {\n\tpanic(fmt.Sprintf(\"decode: %v\", err))\n}"
        );
    }

    #[test]
    fn void_snippet_does_not_import_fmt_when_it_is_unused() {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "reset".into();
        e2e.call.module = "github.com/example/library".into();
        e2e.call.returns_void = true;

        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(!body.contains("\"fmt\""), "{body}");
    }

    #[test]
    fn snippet_separates_package_and_import_declarations() {
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();

        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(body.starts_with("package main\n\nimport (\n"), "{body}");
        assert!(!body.contains("package main import"), "{body}");
    }

    #[test]
    fn snippet_matches_gofmt_when_available() {
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();
        e2e.call.function = "process".into();
        e2e.call.result_var = "result".into();
        e2e.call.returns_result = true;
        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");
        let Ok(mut child) = std::process::Command::new("gofmt")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            assert!(body.contains("\tresult, err := pkg.Process()"), "{body}");
            return;
        };
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("gofmt stdin")
            .write_all(body.as_bytes())
            .expect("write Go snippet");
        let output = child.wait_with_output().expect("wait for gofmt");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            String::from_utf8(output.stdout)
                .expect("gofmt output is UTF-8")
                .trim_end(),
            body
        );
    }

    #[test]
    fn snippet_constructs_known_dto_without_json_round_trip() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({
            "payload": {"kind": "active", "label": "sample", "retry": true, "timeout": 30}
        });
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();
        e2e.call.function = "process".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = [
            ("payload", "input.payload", Some("SampleInput")),
            ("config", "input.config", None),
        ]
        .into_iter()
        .map(|(name, field, element_type)| crate::e2e::config::ArgMapping {
            name: name.into(),
            field: field.into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: element_type.map(str::to_string),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        })
        .collect();
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                options_ptr: true,
                options_type: Some("SampleConfig".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[
                TypeDef {
                    name: "SampleInput".into(),
                    fields: vec![
                        crate::core::ir::FieldDef {
                            name: "kind".into(),
                            ty: crate::core::ir::TypeRef::Named("SampleKind".into()),
                            default: Some("active".into()),
                            typed_default: Some(crate::core::ir::DefaultValue::EnumVariant("active".into())),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "label".into(),
                            ty: crate::core::ir::TypeRef::String,
                            typed_default: Some(crate::core::ir::DefaultValue::StringLiteral(String::new())),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "retry".into(),
                            ty: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                            // `needs_omitempty_pointer` (backends::go::gen_bindings::types::helpers) requires
                            // `default.is_some()` — the field's real `#[serde(default)]` attribute, not merely
                            // the container's `impl Default` — before it will treat a non-zero `typed_default`
                            // as pointer-worthy. Without this the field is (correctly) rendered as a required,
                            // non-pointer value and this fixture stops exercising the pointer-cast path it
                            // exists to pin. ~keep
                            default: Some("/* serde(default) */".into()),
                            typed_default: Some(crate::core::ir::DefaultValue::BoolLiteral(true)),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "timeout".into(),
                            ty: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
                            default: Some("/* serde(default) */".into()),
                            typed_default: Some(crate::core::ir::DefaultValue::IntLiteral(30)),
                            ..Default::default()
                        },
                    ],
                    has_default: true,
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "SampleConfig".into(),
                    ..TypeDef::default()
                },
            ],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            body.contains(
                "payload := pkg.SampleInput{\n\t\tKind:    ptr(pkg.SampleKind(`active`)),\n\t\tLabel:   `sample`,\n\t\tRetry:   ptr(true),\n\t\tTimeout: ptr(int64(30)),"
            ),
            "{body}"
        );
        assert!(body.contains("config := pkg.SampleConfig{}"), "{body}");
        // `options_ptr` is set, so the binding's signature takes `*SampleConfig`; the bound DTO has
        // to be passed by address. This assertion read `pkg.Process(payload, config)` while the
        // branch that emits it ignored `options_ptr` entirely -- it pinned the defect. ~keep
        assert!(body.contains("pkg.Process(payload, &config)"), "{body}");
        assert!(!body.contains("pkg.Process(payload, nil)"), "{body}");
        assert!(!body.contains("json.Unmarshal"), "{body}");
        assert!(!body.contains("encoding/json"), "{body}");
    }

    /// A fixture that supplies no options at all still reaches the binding through the
    /// native-DTO branch, which binds a typed empty literal and passes it by name. That branch was
    /// the only one of the seven `json_object` paths that never consulted `options_ptr`, so on a
    /// crate whose options parameter is `Option<T>` (pointer in Go) every optionless fixture --
    /// the majority of them -- emitted `Convert(html, options)` against `func Convert(html string,
    /// options *ConversionOptions)` and failed to compile. ~keep
    #[test]
    fn an_absent_options_object_is_passed_by_address_when_the_binding_takes_a_pointer() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "html": "<p>hi</p>" });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![
            crate::e2e::config::ArgMapping {
                name: "html".into(),
                field: "html".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "options".into(),
                arg_type: "json_object".into(),
                optional: true,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ];
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                fields: Vec::new(),
                ..TypeDef::default()
            }],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("options := pkg.SampleConfig{}"), "{body}");
        assert!(body.contains(", &options)"), "{body}");
        assert!(!body.contains(", options)"), "{body}");
    }

    #[test]
    fn snippet_honors_shared_options_pointer_and_prints_fields() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "options": {} });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "options".into(),
            field: "options".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "go".into(),
            crate::e2e::config::CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                ..Default::default()
            },
        );
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(rendered.contains("&options"), "{rendered}");
        assert!(rendered.contains("fmt.Printf(\"%+v\\n\", result)"), "{rendered}");
    }

    /// Cluster 1 of the htmd defect: 118 fixtures passed a value where the Go binding
    /// took `*ConversionOptions`. The `options_ptr` config override is hand-authored and
    /// can go stale; when the real `FunctionDef` for the call is available, its
    /// `optional` flag on the options parameter — the same fact
    /// `gen_bindings::functions::gen_function_wrapper` reads to decide `*T` vs `T` — must
    /// win over a stale `options_ptr = false`. ~keep
    #[test]
    fn options_ptr_prefers_the_real_signature_over_a_stale_config_false() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "options": {} });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "options".into(),
            field: "options".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: false,
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "convert".into(),
            params: vec![
                make_param("html", TypeRef::String, false),
                make_param("options", TypeRef::Named("SampleConfig".into()), true),
            ],
            ..FunctionDef::default()
        }];
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &functions,
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("&options"),
            "the real signature marks the options param `optional`, so the binding backend \
             emits `*SampleConfig` — the snippet must pass `&options` regardless of the \
             config's stale `options_ptr = false`: {rendered}"
        );
    }

    /// The inverse of the above: a stale `options_ptr = true` must not force a pointer
    /// when the real signature takes the options struct by value. ~keep
    #[test]
    fn options_ptr_prefers_the_real_signature_over_a_stale_config_true() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "options": {} });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "options".into(),
            field: "options".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "convert".into(),
            params: vec![
                make_param("html", TypeRef::String, false),
                make_param("options", TypeRef::Named("SampleConfig".into()), false),
            ],
            ..FunctionDef::default()
        }];
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &functions,
        )
        .expect("snippet renders");

        assert!(
            !rendered.contains("&options"),
            "the real signature's options param is not `optional`, so the binding backend \
             emits a value `SampleConfig` — the snippet must not take its address just \
             because the config's stale `options_ptr = true` says so: {rendered}"
        );
        assert!(rendered.contains("pkg.Convert(options)"), "{rendered}");
    }

    /// Cluster 2 of the htmd defect: 53 fixtures called `htmd.Convert` with more
    /// arguments than the binding accepts. `extra_args` is meant for slots the real
    /// signature actually has (e.g. a visitor-accepting overload); when the resolved
    /// call's `FunctionDef` shows no room left, the configured extras must be dropped
    /// instead of emitted as an argument the binding's `Convert` does not declare. ~keep
    #[test]
    fn extra_args_are_clamped_to_the_real_signatures_remaining_arity() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "html": "<p>hi</p>", "options": {} });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![
            crate::e2e::config::ArgMapping {
                name: "html".into(),
                field: "html".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "options".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ];
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                extra_args: vec!["nil".into()],
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "convert".into(),
            params: vec![
                make_param("html", TypeRef::String, false),
                make_param("options", TypeRef::Named("SampleConfig".into()), true),
            ],
            ..FunctionDef::default()
        }];
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &functions,
        )
        .expect("snippet renders");

        assert!(rendered.contains("pkg.Convert("), "{rendered}");
        assert!(
            !rendered.contains(", nil)"),
            "the real `convert` signature has no third parameter, so a configured \
             trailing `extra_args = [\"nil\"]` (sized for a different, visitor-accepting \
             overload) must be dropped rather than emitted as a third positional \
             argument: {rendered}"
        );
    }

    fn visitor_fixture() -> Fixture {
        let mut fixture = fixture();
        fixture.id = "visitor_link_rewrite".into();
        fixture.description = "Visitor rewrites links".into();
        fixture.input = serde_json::json!({ "html": "<a href=\"a\">a</a>" });
        fixture.visitor = serde_json::from_value(serde_json::json!({
            "callbacks": {"visit_link": {"action": "skip"}}
        }))
        .expect("visitor spec");
        fixture
    }

    fn visitor_e2e() -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.module = "github.com/example/sample".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "html".into(),
            field: "html".into(),
            arg_type: "string".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e
    }

    fn bridge_config(options_type: Option<&str>) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "HtmlVisitor".into(),
                type_alias: Some("VisitorHandle".into()),
                param_name: Some("visitor".into()),
                options_type: options_type.map(str::to_string),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    /// Regression: a visitor fixture with no resolvable options type used to fall through
    /// the `if let` chain, publishing a snippet that compiles but silently omits the
    /// visitor the fixture exists to demonstrate — while the docs page around it still
    /// carries the fixture's visitor title. It must fail closed, matching PHP and C#. ~keep
    #[test]
    fn visitor_without_a_trait_bridge_options_type_fails_instead_of_dropping_the_visitor() {
        let error = render_snippet_body(&visitor_fixture(), &visitor_e2e(), &bridge_config(None), &[], &[], &[])
            .expect_err("a visitor with no options type must not render");

        assert_eq!(
            format!("{error}"),
            "Go documentation snippet `visitor_link_rewrite` needs an options type for its visitor"
        );
    }

    /// Positive control for the above: with the bridge's `options_type` configured, the
    /// ordinary visitor path is unchanged and wires the visitor into the real type. ~keep
    #[test]
    fn visitor_with_a_trait_bridge_options_type_still_wires_the_visitor() {
        let rendered = render_snippet_body(
            &visitor_fixture(),
            &visitor_e2e(),
            &bridge_config(Some("ConversionOptions")),
            &[],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("visitor := &testVisitorVisitorLinkRewrite{}"),
            "{rendered}"
        );
        assert!(rendered.contains("opts := &pkg.ConversionOptions{}"), "{rendered}");
        assert!(rendered.contains("opts.Visitor = visitor"), "{rendered}");
        // `struct {` with the space gofmt requires -- this assertion previously pinned the
        // unformatted `struct{`, so it protected the defect instead of the behaviour. ~keep
        assert!(
            rendered.contains("type testVisitorVisitorLinkRewrite struct {"),
            "{rendered}"
        );
    }

    /// Pins that a `client_factory` documentation snippet never points the reader at the
    /// mock server (`MOCK_SERVER*` env vars, the `/fixtures/<id>` route, or the literal
    /// `"test-key"` credential) and does construct a client for the reader.
    ///
    /// Go diverges from java/csharp/zig on the credential: those read it from the
    /// environment, while this file's `client_setup` inlines the reader-substitutable
    /// placeholder `"your-api-key"`. That is a convention difference, not a harness
    /// leak, so this pins the placeholder rather than asserting an environment read the
    /// generator does not perform.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let mut fixture = fixture();
        fixture.id = "rate_limit_429".into();
        fixture.description = "Rate limited".into();
        fixture.mock_response = Some(crate::e2e::fixture::MockResponse {
            status: 429,
            body: None,
            stream_chunks: None,
            headers: Default::default(),
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains(".CreateClient(\"your-api-key\", nil, nil, nil, nil)"),
            "client is not constructed with a reader-substitutable credential:\n{body}"
        );
        assert!(
            body.contains("client.Chat("),
            "the call must go through the constructed client:\n{body}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its Go
    /// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling.
    /// Paired with `client_factory_snippet_never_points_the_reader_at_the_mock_server`
    /// above (whose fixture declares no `docs.client` and must keep rendering the bare,
    /// `nil`-in-that-slot call) as the negative control: an indiscriminate "always add
    /// base_url" change would fail that test. ~keep
    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url",
            "description": "Custom base URL",
            "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture must parse");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(
            body.contains(
                "client, clientErr := pkg.CreateClient(\"your-api-key\", ptr(\"https://llm.internal.example.com/v1\"), nil, nil, nil)"
            ),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
        assert!(
            body.contains("func ptr[T any](value T) *T { return &value }"),
            "the base-url pointer needs the shared ptr[T any] helper declared:\n{body}"
        );
    }

    fn client_factory_snippet(expects_error: bool) -> String {
        let mut fixture = fixture();
        fixture.id = "rate_limit_429".into();
        fixture.description = "Rate limited".into();
        if expects_error {
            fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        }
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[]).expect("snippet renders")
    }

    /// The Go binding backend registers no finalizer for an opaque handle, so a snippet that
    /// constructs a client and never calls `Free()` publishes a leak. `defer` rather than a
    /// trailing call is the load-bearing part: the body panics on the operation's error path,
    /// and only a deferred call still runs while a panic unwinds. ~keep
    #[test]
    fn client_factory_snippet_defers_the_clients_release() {
        let body = client_factory_snippet(false);

        assert!(
            body.contains("\tdefer client.Free()"),
            "a constructed client must be released, tab-indented for gofmt:\n{body}"
        );
        let release = body.find("defer client.Free()").expect("release statement");
        let guard = body.find("panic(clientErr)").expect("construction guard");
        let call = body.find("client.Chat(").expect("operation call");
        assert!(
            guard < release && release < call,
            "the release must be deferred after the construction guard and before the call:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_defers_the_clients_release`: the fixture
    /// that documents a failure is the one Kotlin's straight-line `client.close()` leaks, so pin
    /// that Go's release is registered before the failing call rather than after it. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_factory_snippet(true);

        let release = body.find("defer client.Free()").expect("release statement");
        let call = body.find("client.Chat(").expect("operation call");
        assert!(
            release < call,
            "an expects-error snippet must defer the release before the call that fails:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so it must gain no release at all.
    /// Without this, an unconditional `defer` would emit a call on an identifier that does not
    /// exist and rewrite every client-less snippet in the published corpus. ~keep
    #[test]
    fn snippet_without_a_client_factory_emits_no_release() {
        let body = render_snippet_body(
            &fixture(),
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            !body.contains("defer "),
            "a snippet that constructs no client must emit no deferred release:\n{body}"
        );
        assert!(
            !body.contains(".Free()"),
            "a snippet that constructs no client must emit no release call:\n{body}"
        );
    }

    // ---------------------------------------------------------------------------------
    // Enum-typed DTO field lowering.
    //
    // `native_go_dto_literal_at` renders every `TypeRef::Named` field it cannot resolve to a
    // struct as the conversion `alias.Type(<value>)`. That is legal Go only when the binding
    // declares the target with an underlying type of `string` or `[]byte`; the Go binding
    // backend also emits enums as `struct` and as sealed `interface`, and against those the
    // conversion is a `cannot convert` compile error. The fixtures below cover one case per
    // emitted shape so the fix cannot pass by converting -- or by refusing -- everything.
    // ---------------------------------------------------------------------------------

    fn dto_field(name: &str, ty: TypeRef, optional: bool) -> crate::core::ir::FieldDef {
        crate::core::ir::FieldDef {
            name: name.into(),
            ty,
            optional,
            ..crate::core::ir::FieldDef::default()
        }
    }

    fn request_type(fields: Vec<crate::core::ir::FieldDef>) -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            rust_path: "samplelib::SampleRequest".into(),
            fields,
            ..TypeDef::default()
        }
    }

    fn variant(name: &str, fields: Vec<crate::core::ir::FieldDef>) -> crate::core::ir::EnumVariant {
        crate::core::ir::EnumVariant {
            name: name.into(),
            fields,
            ..crate::core::ir::EnumVariant::default()
        }
    }

    fn sample_enum(name: &str, variants: Vec<crate::core::ir::EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.into(),
            rust_path: format!("samplelib::{name}"),
            variants,
            serde_rename_all: Some("snake_case".into()),
            ..EnumDef::default()
        }
    }

    /// `type SampleMode string` plus a const block -- `GoEnumRepresentation::UnitString`.
    fn unit_enum() -> EnumDef {
        sample_enum(
            "SampleMode",
            vec![variant("Fast", Vec::new()), variant("Careful", Vec::new())],
        )
    }

    /// `type SampleChoice struct { .. }` -- every data field is a tuple field and one of them
    /// is a `Named` struct, which is `gen_tuple_tagged_union_type`'s condition.
    fn struct_shaped_enum() -> EnumDef {
        sample_enum(
            "SampleChoice",
            vec![
                variant(
                    "Mode",
                    vec![dto_field("_0", TypeRef::Named("SampleMode".into()), false)],
                ),
                variant(
                    "Explicit",
                    vec![dto_field("_0", TypeRef::Named("SampleTarget".into()), false)],
                ),
            ],
        )
    }

    /// `type SampleDocument interface { .. }` -- a struct variant with named fields is not a
    /// tuple enum, which is `gen_data_enum_type`'s condition.
    fn interface_shaped_enum() -> EnumDef {
        sample_enum(
            "SampleDocument",
            vec![variant("Url", vec![dto_field("url", TypeRef::String, false)])],
        )
    }

    /// `type SampleInput json.RawMessage` -- all tuple fields, none `Named`, and one is a
    /// `Vec`, which is `is_passthrough_raw_message_enum`'s condition.
    fn raw_message_enum() -> EnumDef {
        sample_enum(
            "SampleInput",
            vec![
                variant("Single", vec![dto_field("_0", TypeRef::String, false)]),
                variant(
                    "Multiple",
                    vec![dto_field("_0", TypeRef::Vec(Box::new(TypeRef::String)), false)],
                ),
            ],
        )
    }

    fn request_arg() -> crate::e2e::config::ArgMapping {
        crate::e2e::config::ArgMapping {
            name: "request".into(),
            field: "input.request".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: Some("SampleRequest".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn request_fixture(request: serde_json::Value) -> Fixture {
        Fixture {
            id: "send_request".into(),
            description: "Send a request".into(),
            input: serde_json::json!({ "request": request }),
            ..fixture()
        }
    }

    fn request_e2e() -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "send".into();
        e2e.call.module = "example.com/sample".into();
        e2e.call.result_var = "result".into();
        e2e.call.returns_result = true;
        e2e.call.args = vec![request_arg()];
        e2e
    }

    fn render_request_snippet(
        request: serde_json::Value,
        fields: Vec<crate::core::ir::FieldDef>,
        enums: &[EnumDef],
    ) -> anyhow::Result<String> {
        render_request_snippet_with_types(request, fields, Vec::new(), enums)
    }

    fn render_request_snippet_with_types(
        request: serde_json::Value,
        fields: Vec<crate::core::ir::FieldDef>,
        extra_types: Vec<TypeDef>,
        enums: &[EnumDef],
    ) -> anyhow::Result<String> {
        let mut type_defs = vec![request_type(fields)];
        type_defs.extend(extra_types);
        render_snippet_body(
            &request_fixture(request),
            &request_e2e(),
            &ResolvedCrateConfig::default(),
            &type_defs,
            enums,
            &[],
        )
    }

    /// A plain DTO used as an enum variant's payload.
    fn sample_target_type() -> TypeDef {
        TypeDef {
            name: "SampleTarget".into(),
            rust_path: "samplelib::SampleTarget".into(),
            fields: vec![dto_field("name", TypeRef::String, false)],
            ..TypeDef::default()
        }
    }

    /// `type SampleSelector struct { .. }` with no discriminator field: `#[serde(untagged)]`
    /// over single-field tuple variants. This is the shape of the field that produced five of
    /// the eleven gaps -- and the one whose fixture values are bare strings.
    fn untagged_struct_enum() -> EnumDef {
        EnumDef {
            serde_untagged: true,
            ..sample_enum(
                "SampleSelector",
                vec![
                    variant(
                        "Mode",
                        vec![dto_field("_0", TypeRef::Named("SampleMode".into()), false)],
                    ),
                    variant(
                        "Explicit",
                        vec![dto_field("_0", TypeRef::Named("SampleTarget".into()), false)],
                    ),
                ],
            )
        }
    }

    /// `type SampleTagged struct { Kind string; .. }` -- `#[serde(tag = "kind")]` over tuple
    /// variants, which `gen_tuple_tagged_union_type` renders with a discriminator field.
    fn tagged_struct_enum() -> EnumDef {
        EnumDef {
            serde_tag: Some("kind".into()),
            ..sample_enum(
                "SampleTagged",
                vec![variant(
                    "Explicit",
                    vec![dto_field("_0", TypeRef::Named("SampleTarget".into()), false)],
                )],
            )
        }
    }

    /// `type SampleDoc interface { .. }` with `#[serde(tag = "type")]` -- the shape of the two
    /// enums behind the other six gaps.
    fn tagged_interface_enum() -> EnumDef {
        EnumDef {
            serde_tag: Some("type".into()),
            ..sample_enum(
                "SampleDoc",
                vec![
                    variant("Remote", vec![dto_field("location", TypeRef::String, false)]),
                    variant("Inline", vec![dto_field("payload", TypeRef::String, false)]),
                ],
            )
        }
    }

    /// The release-blocking half: a field declared as an IR enum must be filled with the
    /// constant the Go binding declares for that variant. `gen_unit_enum_type` names it
    /// `go_type_name(enum) + to_go_name(variant)` and initialises it to the variant's
    /// `wire_variant_value` -- which is exactly the string fixture JSON carries -- so `"fast"`
    /// resolves to `pkg.SampleModeFast`. The bare literal is doubly wrong on an optional field:
    /// `ptr[T any](value T) *T` would infer `*string`, not `*SampleMode`. ~keep
    #[test]
    fn enum_typed_dto_field_lowers_to_the_binding_constant_not_a_conversion() {
        let rendered = render_request_snippet(
            serde_json::json!({"mode": "fast"}),
            vec![dto_field("mode", TypeRef::Named("SampleMode".into()), true)],
            &[unit_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Mode: ptr(pkg.SampleModeFast)"),
            "an enum-typed field must be filled with the binding's declared constant:\n{rendered}"
        );
        assert!(
            !rendered.contains("pkg.SampleMode(`fast`)"),
            "the blind string conversion must be gone:\n{rendered}"
        );
        assert!(
            !rendered.contains("ptr(`fast`)"),
            "a bare string literal would infer *string, not *SampleMode:\n{rendered}"
        );
    }

    /// The control that stops the fix from passing by qualifying everything: the same fixture
    /// value, `"fast"`, against a field the core really declares as a `String`. The plain Go
    /// string literal is the correct lowering there, so it must survive byte-for-byte. A fix
    /// keyed on the value rather than on the field's declared type would fail here. ~keep
    #[test]
    fn string_typed_dto_field_still_lowers_to_a_plain_go_string_literal() {
        let rendered = render_request_snippet(
            serde_json::json!({"label": "fast"}),
            vec![dto_field("label", TypeRef::String, false)],
            &[unit_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Label: `fast`"),
            "a string-typed field must keep its plain literal:\n{rendered}"
        );
        assert!(
            !rendered.contains("SampleMode"),
            "a string-typed field must not be qualified with an enum that merely shares its \
             value:\n{rendered}"
        );
    }

    /// The refusal's remaining half, and the control on the constructors below: `SampleChoice`
    /// is serde's DEFAULT external tagging, whose wire form is the single-key object
    /// `{"explicit": {..}}`. A bare string names no key, so it identifies no variant, and no
    /// expression of the emitted `type SampleChoice struct { .. }` follows from it -- neither a
    /// conversion (`cannot convert`) nor a composite literal (which variant?). The emitter must
    /// still refuse here: a recorded coverage gap beats a published snippet that does not build,
    /// and a fix that constructed *something* for every struct-shaped enum would fail this. ~keep
    #[test]
    fn struct_shaped_data_enum_dto_field_is_refused_when_no_variant_is_identified() {
        let error = match render_request_snippet(
            serde_json::json!({"choice": "auto"}),
            vec![dto_field("choice", TypeRef::Named("SampleChoice".into()), true)],
            &[struct_shaped_enum(), unit_enum()],
        ) {
            Ok(rendered) => panic!("a struct-shaped enum must never take a string conversion:\n{rendered}"),
            Err(error) => format!("{error:#}"),
        };

        assert!(
            error.contains("`choice`"),
            "must name the field it refused to fill: {error}"
        );
        assert!(
            error.contains("SampleChoice"),
            "must name the Go type the field actually has: {error}"
        );
        assert!(
            error.contains("struct"),
            "must name the Go declaration that makes the conversion illegal: {error}"
        );
        assert!(
            error.contains("auto"),
            "must quote the offending value so the operator can find the fixture entry: {error}"
        );
        assert!(
            error.contains("cannot convert"),
            "must keep naming the compile error it refused to publish rather than degrading to a bare \
             `incompatible`: {error}"
        );
        assert!(
            error.contains("docs.coverage_exceptions"),
            "must keep naming the recorded-exception escape hatch: {error}"
        );
    }

    /// The same control for the sealed-interface shape: `SampleDocument` carries neither
    /// `#[serde(tag)]` nor `#[serde(untagged)]`, so the JSON below has no discriminator to read
    /// and the emitted decoder has none to write. An interface cannot be converted to and cannot
    /// be constructed directly, so with no variant identified there is no expression at all --
    /// and the refusal must survive the variant construction added for the tagged and untagged
    /// forms. ~keep
    #[test]
    fn interface_shaped_data_enum_dto_field_is_refused_when_no_variant_is_identified() {
        let error = match render_request_snippet(
            serde_json::json!({"document": {"url": "https://example.com/doc.pdf"}}),
            vec![dto_field("document", TypeRef::Named("SampleDocument".into()), false)],
            &[interface_shaped_enum()],
        ) {
            Ok(rendered) => panic!("a sealed interface must never take a string conversion:\n{rendered}"),
            Err(error) => format!("{error:#}"),
        };

        assert!(error.contains("`document`"), "must name the field: {error}");
        assert!(error.contains("SampleDocument"), "must name the Go type: {error}");
        assert!(
            error.contains("interface"),
            "must name the Go declaration that makes the conversion illegal: {error}"
        );
    }

    /// The control that bounds the refusal: `type SampleInput json.RawMessage` has an
    /// underlying type of `[]byte`, which a Go string constant converts to, so the snippets
    /// built on that shape compile today and must keep compiling. A refusal keyed on "the
    /// value is not an object" or on "the name is an enum" would delete them. ~keep
    #[test]
    fn raw_message_enum_dto_field_keeps_its_conversion() {
        let rendered = render_request_snippet(
            serde_json::json!({"prompt": ["one", "two"]}),
            vec![dto_field("prompt", TypeRef::Named("SampleInput".into()), false)],
            &[raw_message_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Prompt: pkg.SampleInput(`[\"one\",\"two\"]`)"),
            "a json.RawMessage enum must keep the conversion that already compiles:\n{rendered}"
        );
    }

    /// The second control: a value that names no variant of a `type X string` enum still has a
    /// legal conversion, and validation fixtures assert on exactly such rejected values. Only
    /// the absence of a legal expression justifies a refusal -- not the absence of a match. ~keep
    #[test]
    fn unit_enum_value_matching_no_variant_keeps_its_conversion() {
        let rendered = render_request_snippet(
            serde_json::json!({"mode": "not-a-mode"}),
            vec![dto_field("mode", TypeRef::Named("SampleMode".into()), false)],
            &[unit_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Mode: pkg.SampleMode(`not-a-mode`)"),
            "an unmatched value must keep the conversion the binding accepts:\n{rendered}"
        );
    }

    // ---------------------------------------------------------------------------------
    // Constructing a value of a struct- or interface-shaped enum.
    //
    // The refusal above is correct but terminal: neither a `struct` nor an `interface` target
    // has *any* conversion, so "give the fixture a value matching one of the variants" cannot
    // be acted on until the emitter can build the variant. These pin the expression built for
    // each emitted shape, spelled the way `backends::go::gen_bindings::types::enums` declares
    // it -- a name invented here would not compile any better than the conversion it replaces.
    // ---------------------------------------------------------------------------------

    /// The five-fixture half of the blocker: an `#[serde(untagged)]` enum over single-field
    /// tuple variants is `type SampleSelector struct { Mode *SampleMode; Explicit *SampleTarget }`,
    /// and the fixture value is the bare string `"fast"`. The untagged decoder picks the first
    /// variant whose payload can hold the value, so a JSON string selects the `type X string`
    /// payload -- and the constant, not a `*string`, is what that pointer field takes. ~keep
    #[test]
    fn untagged_struct_enum_builds_the_variant_a_bare_string_selects() {
        let rendered = render_request_snippet_with_types(
            serde_json::json!({"selector": "fast"}),
            vec![dto_field("selector", TypeRef::Named("SampleSelector".into()), true)],
            vec![sample_target_type()],
            &[untagged_struct_enum(), unit_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Selector: ptr(pkg.SampleSelector{Mode: ptr(pkg.SampleModeFast)})"),
            "an untagged struct enum must be built from the variant its payload type admits:\n{rendered}"
        );
        assert!(
            !rendered.contains("pkg.SampleSelector(`fast`)"),
            "the `cannot convert` conversion must not come back:\n{rendered}"
        );
        assert!(
            !rendered.contains("Explicit:"),
            "a string must not select the struct-payload variant:\n{rendered}"
        );
    }

    /// The same enum, the other variant: a JSON object cannot be a `type SampleMode string`, so
    /// the untagged selection falls through to the struct payload and builds it as a nested DTO
    /// literal. The pointer is `&` here rather than `ptr(...)` because a composite literal is
    /// addressable where a constant is not. ~keep
    #[test]
    fn untagged_struct_enum_builds_the_variant_a_json_object_selects() {
        let rendered = render_request_snippet_with_types(
            serde_json::json!({"selector": {"name": "search"}}),
            vec![dto_field("selector", TypeRef::Named("SampleSelector".into()), true)],
            vec![sample_target_type()],
            &[untagged_struct_enum(), unit_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Selector: ptr(pkg.SampleSelector{Explicit: &pkg.SampleTarget{"),
            "a JSON object must select the struct-payload variant:\n{rendered}"
        );
        assert!(
            rendered.contains("Name: `search`"),
            "the payload's own fields must be lowered as a DTO literal:\n{rendered}"
        );
        assert!(
            !rendered.contains("Mode:"),
            "an object must not select the string-payload variant:\n{rendered}"
        );
    }

    /// An internally tagged struct union additionally declares the discriminator field its
    /// generated `MarshalJSON` switches on. A literal that set only the variant pointer would
    /// compile and then serialise through the marshaler's tag-only fallback, dropping the
    /// payload -- so the tag is part of the constructed expression, not decoration. ~keep
    #[test]
    fn internally_tagged_struct_enum_sets_the_discriminator_field() {
        let rendered = render_request_snippet_with_types(
            serde_json::json!({"selector": {"kind": "explicit", "name": "search"}}),
            vec![dto_field("selector", TypeRef::Named("SampleTagged".into()), true)],
            vec![sample_target_type()],
            &[tagged_struct_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Selector: ptr(pkg.SampleTagged{Kind: `explicit`, Explicit: &pkg.SampleTarget{"),
            "an internally tagged union must set both the tag and the variant pointer:\n{rendered}"
        );
    }

    /// serde's default external tagging is a single-key object, and the emitted struct keys its
    /// pointer fields by the variant's own wire name (`explicit`), not by the snake-cased field
    /// name the internally tagged generator uses. Keying on the wrong one would look up the
    /// wrong JSON member and silently build an empty enum. ~keep
    #[test]
    fn externally_tagged_struct_enum_builds_the_variant_its_single_key_names() {
        let rendered = render_request_snippet_with_types(
            serde_json::json!({"choice": {"explicit": {"name": "search"}}}),
            vec![dto_field("choice", TypeRef::Named("SampleChoice".into()), true)],
            vec![sample_target_type()],
            &[struct_shaped_enum(), unit_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Choice: ptr(pkg.SampleChoice{Explicit: &pkg.SampleTarget{"),
            "an externally tagged union must be built from the variant its key names:\n{rendered}"
        );
        assert!(
            rendered.contains("Name: `search`"),
            "the payload under that key is the variant's payload:\n{rendered}"
        );
    }

    /// The six-fixture half: a sealed interface has no constructor and no conversion, so the
    /// only thing a snippet can write is the CONCRETE variant struct the binding declares
    /// (`{Enum}{Variant}`), which satisfies the interface through its marker method. The
    /// discriminator selects it and is not itself a field of that struct -- the emitted
    /// `MarshalJSON` writes it back from the variant's own `Type()` method. ~keep
    #[test]
    fn tagged_interface_enum_builds_the_concrete_variant_struct() {
        let rendered = render_request_snippet(
            serde_json::json!({"document": {"type": "remote", "location": "https://example.com/doc.pdf"}}),
            vec![dto_field("document", TypeRef::Named("SampleDoc".into()), false)],
            &[tagged_interface_enum()],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("Document: pkg.SampleDocRemote{Location: `https://example.com/doc.pdf`}"),
            "a sealed interface must be filled with the concrete variant struct:\n{rendered}"
        );
        assert!(
            !rendered.contains("pkg.SampleDoc{"),
            "the interface type itself has no composite literal:\n{rendered}"
        );
        assert!(
            !rendered.contains("Type:"),
            "the discriminator selects the variant struct; it is not one of its fields:\n{rendered}"
        );
    }

    /// The discriminator must actually be matched: an unknown tag names no variant, so the
    /// emitter refuses instead of picking the first one. Without this a typo'd fixture would
    /// publish a snippet demonstrating the wrong variant. ~keep
    #[test]
    fn tagged_interface_enum_with_an_unknown_discriminator_is_refused() {
        let error = match render_request_snippet(
            serde_json::json!({"document": {"type": "carrier-pigeon", "location": "somewhere"}}),
            vec![dto_field("document", TypeRef::Named("SampleDoc".into()), false)],
            &[tagged_interface_enum()],
        ) {
            Ok(rendered) => panic!("an unknown discriminator must not select a variant:\n{rendered}"),
            Err(error) => format!("{error:#}"),
        };

        assert!(error.contains("`document`"), "must name the field: {error}");
        assert!(error.contains("SampleDoc"), "must name the enum: {error}");
        assert!(
            error.contains("interface"),
            "must name the emitted Go declaration: {error}"
        );
    }

    // ---------------------------------------------------------------------------------
    // Json-typed DTO fields, and the top-level argument arms the core-IR seam converts.
    // ---------------------------------------------------------------------------------

    /// alef #234, end to end. `go_struct_field_expression` used to drop a `TypeRef::Json`
    /// field on its catch-all, so a published snippet compiled while omitting the schema it
    /// exists to document. Both halves are asserted: the value reaches the literal, and the
    /// `encoding/json` import that the `json.RawMessage` conversion needs is pulled in --
    /// the import machinery keys on `json.` appearing in a rendered setup line, so an arm
    /// that spelled the conversion any other way would emit uncompilable Go. ~keep
    #[test]
    fn snippet_emits_a_json_typed_field_and_imports_encoding_json() {
        let body = render_request_snippet(
            serde_json::json!({"schema": {"type": "object", "required": ["name"]}}),
            vec![dto_field("schema", TypeRef::Json, false)],
            &[],
        )
        .expect("snippet renders");

        assert!(
            body.contains("request := pkg.SampleRequest{"),
            "the snippet must be emitted at all before anything is asserted about it:\n{body}"
        );
        assert!(
            // Key-sorted by serde_json's BTreeMap-backed Map, not fixture order. ~keep
            body.contains("Schema: json.RawMessage(`{\"required\":[\"name\"],\"type\":\"object\"}`)"),
            "the documented schema must appear in the literal:\n{body}"
        );
        assert!(
            !body.contains("request := pkg.SampleRequest{}"),
            "the only field must not be dropped, leaving an empty literal:\n{body}"
        );
        assert!(
            body.contains("\"encoding/json\""),
            "a snippet spelling `json.RawMessage` must import encoding/json:\n{body}"
        );
    }

    /// An optional `TypeRef::Json` field is `*json.RawMessage`, so the conversion is
    /// address-taken through the `ptr` helper -- which the snippet must then declare. ~keep
    #[test]
    fn snippet_emits_the_pointer_helper_for_an_optional_json_typed_field() {
        let body = render_request_snippet(
            serde_json::json!({"schema": {"type": "object"}}),
            vec![dto_field("schema", TypeRef::Json, true)],
            &[],
        )
        .expect("snippet renders");

        assert!(
            body.contains("Schema: ptr(json.RawMessage(`{\"type\":\"object\"}`))"),
            "{body}"
        );
        assert!(
            body.contains("func ptr[T any](value T) *T { return &value }"),
            "the pointer helper must be declared alongside its use:\n{body}"
        );
    }

    fn mode_arg() -> crate::e2e::config::ArgMapping {
        crate::e2e::config::ArgMapping {
            name: "mode".into(),
            field: "input.mode".into(),
            arg_type: "string".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn send_function(param_type: TypeRef) -> FunctionDef {
        FunctionDef {
            name: "send".into(),
            params: vec![ParamDef {
                name: "mode".into(),
                ty: param_type,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("SampleResponse".into()),
            ..FunctionDef::default()
        }
    }

    fn render_mode_snippet(enums: &[EnumDef], functions: &[FunctionDef]) -> String {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "send".into();
        e2e.call.module = "example.com/sample".into();
        e2e.call.result_var = "result".into();
        e2e.call.returns_result = true;
        e2e.call.args = vec![mode_arg()];
        let fixture = Fixture {
            id: "send_mode".into(),
            description: "Send with a mode".into(),
            input: serde_json::json!({"mode": "careful"}),
            ..fixture()
        };
        render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], enums, functions)
            .expect("snippet renders")
    }

    /// The `Known` half. `build_args_and_setup`'s catch-all had no `TargetParams`, so an
    /// argument filling a declared enum parameter was rendered as a bare fixture literal --
    /// documentation that never names the constant a reader is supposed to use. With the IR
    /// in scope it resolves to the binding's own constant. ~keep
    #[test]
    fn a_declared_enum_argument_renders_as_the_bindings_constant() {
        let body = render_mode_snippet(&[unit_enum()], &[send_function(TypeRef::Named("SampleMode".into()))]);

        assert!(
            body.contains("pkg.Send(pkg.SampleModeCareful)"),
            "a declared enum parameter must take the binding's constant:\n{body}"
        );
        assert!(
            !body.contains("pkg.Send(`careful`)"),
            "the bare fixture literal must not survive when the IR names the type:\n{body}"
        );
    }

    /// The `IrAbsent` half, and the one that matters most: every generator that threads no
    /// core IR must emit exactly what it emitted before the seam existed. A test covering
    /// only the `Known` case would let this regress silently for every IR-less consumer. ~keep
    #[test]
    fn an_ir_less_argument_keeps_its_pre_seam_literal() {
        let body = render_mode_snippet(&[unit_enum()], &[]);

        assert!(
            body.contains("pkg.Send(`careful`)"),
            "with no functions registry there is no declared type, so the literal stands:\n{body}"
        );
        assert!(
            !body.contains("SampleModeCareful"),
            "an absent IR licenses no type claim:\n{body}"
        );
    }

    /// A declared parameter whose type names nothing the IR knows is the third state: the
    /// seam answers `Known`, but nothing here can prove what the name is, so the rendering is
    /// left alone rather than guessed at with a blind `pkg.T(...)` conversion. ~keep
    #[test]
    fn a_declared_but_unknown_type_keeps_its_literal() {
        let body = render_mode_snippet(&[], &[send_function(TypeRef::Named("MysteryMode".into()))]);

        assert!(
            body.contains("pkg.Send(`careful`)"),
            "an unresolvable declared type must not be converted blindly:\n{body}"
        );
        assert!(!body.contains("pkg.MysteryMode("), "{body}");
    }

    /// A declared `String` parameter names no type at all, so the catch-all must keep its
    /// literal -- the negative control that keeps the conversion scoped to named types. ~keep
    #[test]
    fn a_declared_string_parameter_keeps_its_literal() {
        let body = render_mode_snippet(&[unit_enum()], &[send_function(TypeRef::String)]);

        assert!(body.contains("pkg.Send(`careful`)"), "{body}");
    }
}
