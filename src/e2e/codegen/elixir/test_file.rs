use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use super::stubs::emit_test_backend;
use super::values::elixir_module_name;
use super::{http, test_case};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module_path: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    handle_struct_type: Option<&str>,
    handle_atom_list_fields: &std::collections::HashSet<String>,
    adapters: &[crate::core::config::extras::AdapterConfig],
    enums: &[crate::core::ir::EnumDef],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");

    // First pass: collect all trait-bridge module definitions from fixtures.
    // These must be emitted at the file level (before the test module defmodule),
    // not nested inside it, so modules can be defined at the correct scope.
    let mut trait_bridge_module_defs = Vec::new();
    for fixture in fixtures {
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let resolved_args = fixture.resolved_args(call_config);
        for arg in resolved_args.iter() {
            if arg.arg_type == "test_backend" {
                if let Some(trait_name) = &arg.trait_name {
                    if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                        let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                            .iter()
                            .find(|t| t.name == *trait_name)
                            .map(|t| t.methods.iter().collect())
                            .unwrap_or_default();
                        if let Some(super_trait) = &trait_bridge.super_trait {
                            if let Some(super_type) = type_defs.iter().find(|t| &t.name == super_trait) {
                                for method in &super_type.methods {
                                    if !methods.iter().any(|m| m.name == method.name) {
                                        methods.push(method);
                                    }
                                }
                            }
                        }
                        let elixir_nif_module = format!("{module_path}.Native");
                        let emission = emit_test_backend(trait_bridge, &methods, fixture, &elixir_nif_module);

                        // Extract module defs from the combined setup_block
                        if let Some(pos) = emission.setup_block.find("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
                            let marker_start = emission.setup_block[..pos].rfind('\n').unwrap_or(0);
                            let module_defs_str = emission.setup_block[..marker_start].trim_end().to_string();
                            for line in module_defs_str.lines() {
                                if !line.is_empty() {
                                    trait_bridge_module_defs.push(line.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Emit trait-bridge module definitions at file level (before test module defmodule).
    // Strip leading whitespace since emit_test_backend includes indentation.
    for module_def_line in &trait_bridge_module_defs {
        let _ = writeln!(out, "{}", module_def_line.trim_start());
    }

    let _ = writeln!(out, "defmodule E2e.{}Test do", elixir_module_name(category));

    // Add client helper when there are HTTP fixtures in this group.
    let has_http = fixtures.iter().any(|f| f.is_http_test());

    // Use async: false for NIF tests — concurrent Tokio runtimes created by DirtyCpu NIFs
    // on ARM64 macOS cause SIGBUS when tests run in parallel. HTTP-only tests can stay async.
    let async_flag = if has_http { "true" } else { "false" };
    let _ = writeln!(out, "  use ExUnit.Case, async: {async_flag}");

    if has_http {
        let _ = writeln!(out);
        let _ = writeln!(out, "  defp mock_server_url do");
        let _ = writeln!(
            out,
            "    System.get_env(\"MOCK_SERVER_URL\") || \"http://localhost:8080\""
        );
        let _ = writeln!(out, "  end");
    }

    // Emit a shared helper for array field contains assertions — extracts string
    // representations from each item's attributes so String.contains? works on struct lists.
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
    if has_array_contains {
        let _ = writeln!(out);
        let _ = writeln!(out, "  defp alef_e2e_item_texts(item) when is_binary(item), do: [item]");
        let _ = writeln!(out, "  defp alef_e2e_item_texts(item) do");
        let _ = writeln!(out, "    [:kind, :name, :signature, :path, :alias, :text, :source]");
        let _ = writeln!(out, "    |> Enum.filter(&Map.has_key?(item, &1))");
        let _ = writeln!(out, "    |> Enum.flat_map(fn attr ->");
        let _ = writeln!(out, "      case Map.get(item, attr) do");
        let _ = writeln!(out, "        nil -> []");
        let _ = writeln!(
            out,
            "        atom when is_atom(atom) -> [atom |> to_string() |> String.capitalize()]"
        );
        let _ = writeln!(out, "        str -> [inspect(str)]");
        let _ = writeln!(out, "      end");
        let _ = writeln!(out, "    end)");
        let _ = writeln!(out, "  end");
    }

    // Emit a helper to convert FormatMetadata struct to a string representation
    // (pattern-match on the image field and extract the format string).
    let has_format_metadata = fixtures.iter().any(|fixture| {
        fixture.assertions.iter().any(|a| {
            a.field
                .as_deref()
                .is_some_and(|f| f.contains("format") && f.contains("metadata"))
        })
    });
    if has_format_metadata {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  defp alef_e2e_format_to_string(value) when is_binary(value), do: value"
        );
        let _ = writeln!(out, "  defp alef_e2e_format_to_string(metadata) do");
        let _ = writeln!(out, "    case metadata.image do");
        let _ = writeln!(out, "      %{{format: fmt}} when is_binary(fmt) -> fmt");
        let _ = writeln!(out, "      _ ->");
        let _ = writeln!(out, "        case metadata.pdf do");
        let _ = writeln!(out, "          %{{}} -> \"PDF\"");
        let _ = writeln!(out, "          _ ->");
        let _ = writeln!(out, "            case metadata.html do");
        let _ = writeln!(out, "              %{{}} -> \"HTML\"");
        let _ = writeln!(out, "              _ -> inspect(metadata)");
        let _ = writeln!(out, "            end");
        let _ = writeln!(out, "        end");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
    }

    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        if let Some(http) = &fixture.http {
            http::render_http_test_case(&mut out, fixture, http);
        } else {
            test_case::render_test_case(
                &mut out,
                fixture,
                e2e_config,
                module_path,
                function_name,
                result_var,
                args,
                options_type,
                options_default_fn,
                enum_fields,
                handle_struct_type,
                handle_atom_list_fields,
                adapters,
                enums,
                config,
                type_defs,
            );
        }
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "end");
    out
}
