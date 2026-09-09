use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_spec_file(
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    class_name: Option<&str>,
    gem_name: &str,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    needs_spec_helper: bool,
    uses_harness: bool,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    errors: &[crate::core::ir::ErrorDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> String {
    // Resolve client_factory from ruby override.
    let client_factory = e2e_config
        .call
        .overrides
        .get("ruby")
        .and_then(|o| o.client_factory.as_deref());

    // Build requires list. spec_helper MUST be required before the gem/module
    // to ensure ENV vars (like E2E_ALLOW_PRIVATE_NETWORK) are set in libc
    // before the native binding loads and caches the SSRF policy.
    let has_http = fixtures.iter().any(|f| f.is_http_test());
    let mut requires = Vec::new();

    if needs_spec_helper || has_http {
        requires.push("spec_helper".to_string());
    }

    let require_name = if module_path.is_empty() { gem_name } else { module_path };
    requires.push(require_name.replace('-', "_"));
    requires.push("json".to_string());

    // Build the Ruby module/class qualifier for calls.
    let ruby_module = super::values::ruby_module_name(module_path);
    let call_receiver = class_name.map(|s| s.to_string()).unwrap_or_else(|| ruby_module.clone());

    // Check for array contains assertions
    let has_array_contains = fixtures.iter().any(|fixture| {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let fr = FieldResolver::new(
            e2e_config.effective_fields(cc),
            e2e_config.effective_fields_optional(cc),
            e2e_config.effective_result_fields(cc),
            e2e_config.effective_fields_array(cc),
            &std::collections::HashSet::new(),
        );
        fixture.assertions.iter().any(|a| {
            matches!(a.assertion_type.as_str(), "contains" | "contains_all" | "not_contains")
                && a.field
                    .as_deref()
                    .is_some_and(|f| !f.is_empty() && fr.is_array(fr.resolve(f)))
        })
    });

    // Build examples
    let mut examples = Vec::new();
    for fixture in fixtures {
        if fixture.http.is_some() {
            // HTTP example is handled separately (uses shared driver or server-pattern)
            let mut out = String::new();
            if uses_harness {
                super::http::render_http_example_sut(&mut out, fixture);
            } else {
                super::http::render_http_example(&mut out, fixture);
            }
            examples.push(out);
        } else {
            // Resolve per-fixture call config so we can detect streaming up front.
            let fixture_call = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            // Build per-call field resolver using the effective field sets for this call.
            let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) =
                FieldResolver::ir_field_sets(type_defs);
            // Anchor the IR-derived enum classification (`with_ir_enum_map`) at the call's
            // declared Rust return type so a leaf field name that means different things on
            // different types resolves per owner, mirroring the rust/csharp/gleam/swift/dart
            // e2e generators. This is purely additive: `is_enum` still consults
            // `with_enum_fields` (the hand-maintained `fields_enum` config) FIRST, so an
            // explicit config entry always wins. ~keep
            let call_root_type = resolve_declared_result_type(fixture_call, "ruby", CallIr { functions, type_defs });
            let fixture_call_resolver = FieldResolver::new(
                e2e_config.effective_fields(fixture_call),
                e2e_config.effective_fields_optional(fixture_call),
                e2e_config.effective_result_fields(fixture_call),
                e2e_config.effective_fields_array(fixture_call),
                &std::collections::HashSet::new(),
            )
            .with_enum_fields(e2e_config.effective_fields_enum(fixture_call).clone())
            .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
            .with_ruby_hash_serialized_enum_names(super::enum_variant_access::hash_serialized_enum_names(enums))
            .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, "ruby"), call_root_type)
            .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields);
            let field_resolver = &fixture_call_resolver;
            let fixture_call_overrides = fixture_call.overrides.get("ruby");
            let raw_function_name = fixture_call_overrides
                .and_then(|o| o.function.as_ref())
                .cloned()
                .unwrap_or_else(|| fixture_call.function.clone());

            let is_streaming = crate::e2e::codegen::streaming_assertions::resolve_is_streaming(
                fixture,
                fixture_call.streaming_enabled(),
            );

            if !renders_a_real_example(
                fixture,
                field_resolver,
                result_is_simple,
                fixture_call.streaming_enabled(),
            ) {
                let test_name = crate::e2e::escape::sanitize_ident(&fixture.id);
                let description_literal =
                    crate::e2e::escape::ruby_string_literal(&format!("{test_name}: {}", fixture.description));
                let mut out = String::new();
                out.push_str(&format!("  it {description_literal} do\n"));
                out.push_str("    skip 'Fixture has no assertions to validate'\n");
                out.push_str("  end\n");
                examples.push(out);
            } else {
                let fixture_result_var = fixture_call.effective_result_var();
                // Use fixture.resolved_args() so per-fixture args (e.g. trait-bridge
                // test_backend stubs) take precedence over the call-config default.
                let fixture_args = fixture.resolved_args(fixture_call);
                let fixture_client_factory = fixture_call_overrides
                    .and_then(|o| o.client_factory.as_deref())
                    .or(client_factory);
                let fixture_function_name = if fixture_call.r#async
                    && !is_streaming
                    && fixture_client_factory.is_some()
                    && !raw_function_name.ends_with("_async")
                {
                    format!("{raw_function_name}_async")
                } else {
                    raw_function_name
                };
                let fixture_options_type = fixture_call_overrides
                    .and_then(|o| o.options_type.as_deref())
                    .or(options_type);

                let fixture_extra_args: Vec<String> =
                    fixture_call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
                // Use per-fixture-call result_is_simple so per-call overrides like
                // `speech` (returns bytes) take precedence over the top-level call default.
                let fixture_result_is_simple =
                    fixture_call.result_is_simple || fixture_call_overrides.is_some_and(|o| o.result_is_simple);
                // Per-call enum_fields take precedence — e.g. `[crates.e2e.calls.create_batch.overrides.ruby] enum_fields`
                // labels `status = "BatchStatus"` for the batch lifecycle, but the global
                // `[crates.e2e.call.overrides.ruby]` map only carries chat-shape entries.
                let fixture_enum_fields: &HashMap<String, String> =
                    fixture_call_overrides.map(|o| &o.enum_fields).unwrap_or(enum_fields);
                let adapter_lookup_name = fixture_call.core_lookup_name("ruby");
                let adapter_lookup_names: Vec<&str> = adapter_lookup_name.as_deref().into_iter().collect();
                let adapter_req_type_owned: Option<String> = adapter_lookup_names
                    .iter()
                    .find_map(|name| adapters.iter().find(|a| a.name == *name))
                    .and_then(|a| a.request_type.as_deref())
                    .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
                let streaming_item_type_owned =
                    crate::e2e::codegen::recipe::streaming_item_type(fixture_call, adapters, &adapter_lookup_names)
                        .map(str::to_string);
                let example = if is_streaming {
                    super::examples::render_chat_stream_example(
                        fixture,
                        &fixture_function_name,
                        &call_receiver,
                        &ruby_module,
                        fixture_args,
                        fixture_options_type,
                        fixture_enum_fields,
                        e2e_config,
                        fixture_client_factory,
                        &fixture_extra_args,
                        adapter_req_type_owned.as_deref(),
                        streaming_item_type_owned.as_deref(),
                        config,
                        type_defs,
                        errors,
                    )
                } else {
                    super::examples::render_example(
                        fixture,
                        &fixture_function_name,
                        &call_receiver,
                        &ruby_module,
                        fixture_result_var,
                        fixture_args,
                        field_resolver,
                        fixture_options_type,
                        fixture_enum_fields,
                        e2e_config.effective_fields_enum(fixture_call),
                        fixture_result_is_simple,
                        fixture_call.returns_void,
                        e2e_config,
                        fixture_client_factory,
                        &fixture_extra_args,
                        adapter_req_type_owned.as_deref(),
                        config,
                        type_defs,
                        errors,
                    )
                };
                // ~keep Ruby's error path renders one `expect { .. }.to raise_error(..)` and
                // returns, so every other assertion on an error fixture — most often an `equals`
                // against `error.status_code` — leaves no trace in the generated spec. The marker
                // is appended after the emitted `it ... end` block, at `describe` body level,
                // rather than inside it: the block is built in `examples.rs`, which another
                // change owns.
                let markers = crate::e2e::codegen::error_path_assertions::render(fixture, "  # ", "ruby");
                let separator = if markers.is_empty() || example.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                examples.push(format!("{example}{separator}{markers}"));
            }
        }
    }

    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    crate::e2e::template_env::render(
        "ruby/test_file.jinja",
        minijinja::context! {
            category => category,
            requires => requires,
            has_array_contains => has_array_contains,
            has_http => has_http,
            examples => examples,
            header => header,
        },
    )
}

/// Whether [`render_spec_file`] emits a real example for this fixture rather than a `skip` stub.
///
/// The category-level gate in `ruby.rs` decides whether to emit the spec file at all, and this
/// decides what goes in it; they must ask the same question. They did not: the category gate
/// omitted `is_streaming`, so a category whose fixtures were all streaming produced no file —
/// and nothing reports an absent language, because `alef verify` walks emitted markers, the
/// empty-category check in `e2e/validate.rs` only fires when *every* language skips a category,
/// and `fixture_inclusion` never consults an emitter's capability. Keep the two callers on this
/// one predicate so they cannot drift apart again. ~keep
pub(super) fn renders_a_real_example(
    fixture: &Fixture,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    streaming_enabled: Option<bool>,
) -> bool {
    if fixture.is_http_test() {
        return true;
    }
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    let is_streaming = crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, streaming_enabled);
    expects_error
        || has_not_error
        || is_streaming
        || !fixture.assertions.is_empty()
        || has_usable_assertion(fixture, field_resolver, result_is_simple)
}

/// Check if a fixture has at least one assertion that will produce an executable
/// expect() call (not just a skip comment).
pub(super) fn has_usable_assertion(fixture: &Fixture, field_resolver: &FieldResolver, result_is_simple: bool) -> bool {
    fixture.assertions.iter().any(|a| {
        // not_error is implicit (call succeeding), error is handled separately.
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        // Check field validity.
        if let Some(f) = &a.field {
            if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
                return false;
            }
            // When result_is_simple, skip non-content fields.
            if result_is_simple {
                let f_lower = f.to_lowercase();
                if !f.is_empty()
                    && f_lower != "content"
                    && (f_lower.starts_with("metadata")
                        || f_lower.starts_with("document")
                        || f_lower.starts_with("structure"))
                {
                    return false;
                }
            }
        }
        true
    })
}
