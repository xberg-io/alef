//! Service-API codegen for the Rustler (Elixir) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.ex`** — Elixir module with a server-like class containing:
//!    - A constructor and configurator methods.
//!    - Registration decorator-style helpers that store callbacks.
//!    - A GenServer to dispatch trait_call messages to registered handlers.
//!    - A `run` entrypoint that marshals registrations to Rust.
//!
//! 2. **`service.rs`** — Rust rustler glue that:
//!    - Emits a message-passing handler bridge for each referenced `HandlerContractDef`.
//!    - Provides a `#[rustler::nif]` run function (with `schedule = "DirtyCpu"`) that
//!      receives registrations, builds the service, and drives entrypoints.
//!    - The bridge sends `{:trait_call, method, args_json, reply_id}` to the Elixir pid
//!      and awaits the response via a `complete_trait_call` NIF.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, RegistrationVariantStyle, ServiceDef, TypeRef,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a simple Elixir type annotation string.
#[allow(dead_code)]
fn elixir_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String.t()".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean()".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float()".to_owned(),
                _ => "integer()".to_owned(),
            }
        }
        TypeRef::Bytes => "binary()".to_owned(),
        TypeRef::Optional(inner) => format!("{} | nil", elixir_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("list({})", elixir_type_annotation(inner)),
        TypeRef::Map(k, v) => format!(
            "map() :: %{{optional({}) => {}}}",
            elixir_type_annotation(k),
            elixir_type_annotation(v)
        ),
        TypeRef::Unit => "nil".to_owned(),
        TypeRef::Named(n) => n.to_string(),
        TypeRef::Json => "any()".to_owned(),
        TypeRef::Path => "String.t()".to_owned(),
        TypeRef::Duration => "non_neg_integer()".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ──────────────────────────────────────────────────────────────── Elixir output ──

/// Generate the idiomatic Elixir service module (`service.ex`).
///
/// Produces an Elixir module containing:
/// - A struct holding configuration state and registrations.
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Decorator-style registration helpers from [`ServiceDef::registrations`].
/// - A GenServer that handles `{:trait_call, ...}` messages from Rust.
/// - A `run` entrypoint that delegates to the native NIF.
pub(super) fn gen_service_ex(api: &ApiSurface, module_prefix: &str) -> String {
    let mut out = String::new();

    out.push_str("# This file is generated. Do not edit.\n\n");

    for service in &api.services {
        gen_service_module(&mut out, service, api, module_prefix);
    }

    out
}

/// Format a Rust doc as an Elixir heredoc body at the given column indent.
/// Returns just the lines between `"""` markers (does not emit the markers
/// themselves). Each non-blank source line is indented to `indent` spaces so
/// the closing `"""` at the same column strips that prefix from the heredoc
/// at compile time; blank lines stay bare.
fn elixir_heredoc_body(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn gen_service_module(out: &mut String, service: &ServiceDef, api: &ApiSurface, module_prefix: &str) {
    let module_name = &service.name;
    let module_snake = service.name.to_snake_case();

    // Module declaration + @moduledoc.
    //
    // Module names are already implicitly under the `Elixir.` namespace; the
    // `defmodule Elixir.<Name>` form prepends a redundant `Elixir.` so the
    // compiled module ends up as `Elixir.Elixir.<Name>`. Emit the bare name.
    out.push_str(&format!("defmodule {module_name} do\n"));
    if !service.doc.is_empty() {
        out.push_str("  @moduledoc \"\"\"\n");
        out.push_str(&elixir_heredoc_body(&service.doc, 2));
        out.push_str("  \"\"\"\n\n");
    }

    // Alias the consumer's `Native` module so unqualified `Native.<fn>(...)`
    // calls in this module's body resolve to `<Prefix>.Native.<fn>(...)`. The
    // root service module is emitted as bare `defmodule App` and does not
    // inherit the namespace of sibling wrapper modules (`<Prefix>.RouteBuilder`,
    // etc.), so this alias is mandatory for the body's NIF calls to compile.
    if !module_prefix.is_empty() {
        out.push_str(&format!("  alias {module_prefix}.Native\n\n"));
    }

    // Struct definition
    out.push_str("  defstruct [\n");
    out.push_str("    :registrations,\n");
    for p in &service.constructor.params {
        out.push_str(&format!("    :{},\n", p.name));
    }
    for method in &service.configurators {
        for p in &method.params {
            out.push_str(&format!("    :{},\n", p.name));
        }
    }
    out.push_str("  ]\n\n");

    // Constructor
    {
        let ctor = &service.constructor;
        let params = ["options \\\\ []".to_owned()];
        let mut field_inits = vec!["registrations: []".to_owned()];

        for p in &ctor.params {
            if p.optional {
                field_inits.push(format!("{}: Keyword.get(options, :{}, nil)", p.name, p.name));
            } else {
                field_inits.push(format!("{}: Keyword.fetch!(options, :{})", p.name, p.name));
            }
        }

        if !ctor.doc.is_empty() {
            out.push_str("  @doc \"\"\"\n");
            out.push_str(&elixir_heredoc_body(&ctor.doc, 2));
            out.push_str("  \"\"\"\n");
        }
        out.push_str(&format!("  def new({}) do\n", params.join(", ")));
        out.push_str("    %__MODULE__{\n");
        for init in field_inits {
            out.push_str(&format!("      {},\n", init));
        }
        out.push_str("    }\n");
        out.push_str("  end\n\n");
    }

    // Configurator methods
    for method in &service.configurators {
        let method_name = &method.name;
        let mut params = vec!["self".to_owned()];
        for p in &method.params {
            if p.optional {
                params.push(format!("{} \\\\ nil", p.name));
            } else {
                params.push(p.name.clone());
            }
        }

        if !method.doc.is_empty() {
            out.push_str("  @doc \"\"\"\n");
            out.push_str(&elixir_heredoc_body(&method.doc, 2));
            out.push_str("  \"\"\"\n");
        }
        out.push_str(&format!("  def {}({}) do\n", method_name, params.join(", ")));
        for p in &method.params {
            out.push_str(&format!("    self = %__MODULE__{{self | {}: {}}}\n", p.name, p.name));
        }
        out.push_str("    self\n");
        out.push_str("  end\n\n");
    }

    // Registration methods as decorator-style helpers
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, module_prefix);
    }

    // GenServer module for dispatching trait_call messages
    gen_genserver_module(out, service, api);

    // Entrypoint methods
    for ep in &service.entrypoints {
        let ep_name = &ep.method;
        let mut params = vec!["self".to_owned()];
        for p in &ep.params {
            if p.optional {
                params.push(format!("{} \\\\ nil", p.name));
            } else {
                params.push(p.name.clone());
            }
        }

        match ep.kind {
            EntrypointKind::Run => {
                if !ep.doc.is_empty() {
                    out.push_str("  @doc \"\"\"\n");
                    out.push_str(&elixir_heredoc_body(&ep.doc, 2));
                    out.push_str("  \"\"\"\n");
                }
                out.push_str(&format!("  def {}({}) do\n", ep_name, params.join(", ")));
                let native_fn = format!("{}_{}", module_snake, ep_name);
                out.push_str(&format!("    Native.{}(self.registrations", native_fn));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n");
                out.push_str("  end\n\n");
            }
            EntrypointKind::Finalize => {
                if !ep.doc.is_empty() {
                    out.push_str("  @doc \"\"\"\n");
                    out.push_str(&elixir_heredoc_body(&ep.doc, 2));
                    out.push_str("  \"\"\"\n");
                }
                out.push_str(&format!("  def {}({}) do\n", ep_name, params.join(", ")));
                let native_fn = format!("{}_{}", module_snake, ep_name);
                out.push_str(&format!("    Native.{}(self.registrations", native_fn));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n");
                out.push_str("  end\n\n");
            }
        }
    }

    out.push_str("end\n\n");
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    module_prefix: &str,
) {
    let method_name = &reg.method;

    if !reg.doc.is_empty() {
        out.push_str("  @doc \"\"\"\n");
        out.push_str(&elixir_heredoc_body(&reg.doc, 2));
        out.push_str("  \"\"\"\n");
    }
    out.push_str(&format!("  def {}(self", method_name));
    for p in &reg.metadata_params {
        if p.optional {
            out.push_str(&format!(", {} \\\\ nil", p.name));
        } else {
            out.push_str(&format!(", {}", p.name));
        }
    }
    out.push_str(", handler) do\n");

    // Build metadata tuple
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{{}}}", meta_names.join(", "))
    };

    out.push_str("    # Wrap handler closure in a process if it's not already one\n");
    out.push_str("    handler_pid = case handler do\n");
    out.push_str("      pid when is_pid(pid) -> pid\n");
    out.push_str("      fun when is_function(fun) ->\n");
    out.push_str("        {:ok, pid} = GenServer.start_link(__MODULE__.HandlerWrapper, fun)\n");
    out.push_str("        pid\n");
    out.push_str("    end\n\n");
    out.push_str(&format!(
        "    entry = {{\"{}\", {}, handler_pid}}\n",
        method_name, meta_tuple
    ));
    out.push_str("    %__MODULE__{self | registrations: [entry | self.registrations]}\n");
    out.push_str("  end\n\n");

    // Emit a simple HandlerWrapper GenServer if this is the route registration
    if method_name == "route" {
        out.push_str("  # HandlerWrapper GenServer: wraps a closure for use as a handler\n");
        out.push_str("  defmodule HandlerWrapper do\n");
        out.push_str("    use GenServer\n\n");
        out.push_str("    def start_link(handler_fn) do\n");
        out.push_str("      GenServer.start_link(__MODULE__, handler_fn)\n");
        out.push_str("    end\n\n");
        out.push_str("    def init(handler_fn) do\n");
        out.push_str("      {:ok, handler_fn}\n");
        out.push_str("    end\n\n");
        out.push_str("    def handle_cast({:trait_call, _method, args_json, reply_id}, handler_fn) do\n");
        out.push_str("      case Jason.decode(args_json) do\n");
        out.push_str("        {:ok, _args} ->\n");
        out.push_str("          # Call the wrapped closure\n");
        out.push_str("          try do\n");
        out.push_str("            response = handler_fn.(nil)\n");
        out.push_str("            response_json = Jason.encode!(response)\n");
        out.push_str("            Native.complete_trait_call(reply_id, response_json)\n");
        out.push_str("          rescue\n");
        out.push_str(
            "            _e -> Native.complete_trait_call(reply_id, \"{\\\"error\\\": \\\"handler error\\\"}\")\n",
        );
        out.push_str("          end\n");
        out.push_str("        {:error, _} ->\n");
        out.push_str(
            "          Native.complete_trait_call(reply_id, \"{\\\"error\\\": \\\"json decode error\\\"}\")\n",
        );
        out.push_str("      end\n");
        out.push_str("      {:noreply, handler_fn}\n");
        out.push_str("    end\n");
        out.push_str("  end\n\n");
    }

    // Emit registration variants (decorator-style shortcuts)
    for variant in &reg.variants {
        gen_registration_variant_method(out, variant, reg, module_prefix);
    }
}

fn gen_registration_variant_method(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    module_prefix: &str,
) {
    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            emit_verb_decorator_variant(out, variant, base_reg, module_prefix);
        }
        RegistrationVariantStyle::Builder => {
            emit_builder_variant(out, variant, base_reg, module_prefix);
        }
        RegistrationVariantStyle::Hybrid => {
            emit_verb_decorator_variant(out, variant, base_reg, module_prefix);
            emit_builder_variant(out, variant, base_reg, module_prefix);
        }
    }
}

/// Convert a Rust enum value expression (e.g. `"my_crate::Method::Get"`) to an
/// Elixir function call (e.g. `"Bindings.Method.get()"`).
///
/// The last two `::` segments are taken as `TypeName::VariantName`; the variant
/// name is lowercased to form the Elixir function call. When `module_prefix` is
/// non-empty it is prepended, otherwise the bare type name is used.
fn rust_enum_expr_to_elixir(value_expr: &str, module_prefix: &str) -> String {
    let parts: Vec<&str> = value_expr.split("::").collect();
    if parts.len() >= 2 {
        let type_name = parts[parts.len() - 2];
        let variant = parts[parts.len() - 1].to_lowercase();
        if module_prefix.is_empty() {
            format!("{type_name}.{variant}()")
        } else {
            format!("{module_prefix}.{type_name}.{variant}()")
        }
    } else {
        // Fallback: use verbatim (already a literal)
        value_expr.to_owned()
    }
}

/// Build the Elixir expression that constructs the wrapper value for a variant,
/// e.g. `builder = Bindings.RouteBuilder.new(Bindings.Method.get(), path)`.
///
/// Returns `None` when the variant has no `wrapper_call`.
fn build_elixir_wrapper_constructor_expr(
    variant: &crate::core::ir::RegistrationVariant,
    module_prefix: &str,
) -> Option<String> {
    let wc = variant.wrapper_call.as_ref()?;
    let mut call_args: Vec<String> = Vec::new();
    for arg in &wc.args {
        match arg {
            crate::core::ir::WrapperConstructorArg::Fixed { value_expr, .. } => {
                call_args.push(rust_enum_expr_to_elixir(value_expr, module_prefix));
            }
            crate::core::ir::WrapperConstructorArg::Free { param } => {
                call_args.push(param.name.clone());
            }
        }
    }
    let type_name = &wc.wrapper_type_name;
    let ctor = &wc.constructor_method;
    let qualified_type = if module_prefix.is_empty() {
        type_name.clone()
    } else {
        format!("{module_prefix}.{type_name}")
    };
    let call_expr = if ctor.is_empty() || ctor == "__init__" {
        format!("{qualified_type}({})", call_args.join(", "))
    } else {
        format!("{qualified_type}.{ctor}({})", call_args.join(", "))
    };
    Some(format!("{} = {}", wc.metadata_param, call_expr))
}

/// Emit the verb-decorator form: `def variant(app, path, ..., handler) do ... end`.
///
/// When the variant has a `wrapper_call`, the body constructs the wrapper object
/// (e.g. `builder = RouteBuilder.new(Method.get(), path)`) and delegates to the
/// base registration method. Without a wrapper, it delegates directly passing the
/// free params to the base method.
fn emit_verb_decorator_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    module_prefix: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;

    if let Some(doc) = &variant.doc {
        out.push_str("  @doc \"\"\"\n");
        out.push_str(&elixir_heredoc_body(doc, 2));
        out.push_str("  \"\"\"\n");
    }

    // Emit signature: app, then signature_params, then handler
    out.push_str(&format!("  def {}(app", variant_name));
    for param in &variant.signature_params {
        if param.optional {
            out.push_str(&format!(", {} \\\\ nil", param.name));
        } else {
            out.push_str(&format!(", {}", param.name));
        }
    }
    out.push_str(", handler) do\n");

    if let Some(wrapper_expr) = build_elixir_wrapper_constructor_expr(variant, module_prefix) {
        // Wrapper pattern: build the wrapper object, then delegate to the base method.
        out.push_str(&format!("    {wrapper_expr}\n"));
        out.push_str(&format!(
            "    {}(app, {}, handler)\n",
            base_method,
            base_reg
                .metadata_params
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        // Direct pattern: build call args by substituting overrides into the base params.
        let mut call_args: Vec<String> = Vec::new();
        for base_param in &base_reg.metadata_params {
            if let Some(override_) = variant.overrides.iter().find(|o| o.param_name == base_param.name) {
                // Fixed override: convert Rust expression to Elixir
                call_args.push(rust_enum_expr_to_elixir(&override_.value_expr, module_prefix));
            } else if let Some(sig_param) = variant.signature_params.iter().find(|s| s.name == base_param.name) {
                call_args.push(sig_param.name.clone());
            }
        }
        let args_str = if call_args.is_empty() {
            String::new()
        } else {
            format!(", {}", call_args.join(", "))
        };
        out.push_str(&format!("    {}(app{}, handler)\n", base_method, args_str));
    }

    out.push_str("  end\n\n");
}

/// Emit the builder form: `def variant_decorator(app, path, ...) do ... end` returning a closure.
///
/// The returned closure accepts a handler and delegates to the verb-decorator form of the same
/// variant. When a `wrapper_call` is present, the wrapper is built inside the closure so each
/// call produces a fresh wrapper instance.
fn emit_builder_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    _base_reg: &RegistrationDef,
    module_prefix: &str,
) {
    let variant_name = &variant.name;
    let builder_name = format!("{}_decorator", variant_name);

    if let Some(doc) = &variant.doc {
        out.push_str("  @doc \"\"\"\n");
        out.push_str(&elixir_heredoc_body(doc, 2));
        out.push_str("  \"\"\"\n");
    }

    // Emit signature: app, then signature_params (no handler)
    out.push_str(&format!("  def {}(app", builder_name));
    for param in &variant.signature_params {
        if param.optional {
            out.push_str(&format!(", {} \\\\ nil", param.name));
        } else {
            out.push_str(&format!(", {}", param.name));
        }
    }
    out.push_str(") do\n");

    // Return a function that accepts the handler and delegates to the verb form.
    // The verb form already handles wrapper construction, so simply call it.
    out.push_str("    fn(handler) ->\n");
    out.push_str(&format!("      {}(app", variant_name));
    for param in &variant.signature_params {
        out.push_str(&format!(", {}", param.name));
    }
    out.push_str(", handler)\n");
    out.push_str("    end\n");

    out.push_str("  end\n\n");
    let _ = module_prefix; // consumed via emit_verb_decorator_variant delegation
}

fn gen_genserver_module(out: &mut String, service: &ServiceDef, _api: &ApiSurface) {
    let module_name = &service.name;
    let server_module = format!("{}.Handler", module_name);

    out.push_str("  # GenServer for dispatching trait_call messages from Rust.\n");
    out.push_str(&format!("  defmodule {} do\n", server_module));
    out.push_str("    use GenServer\n\n");
    out.push_str("    def start_link(state) do\n");
    out.push_str("      GenServer.start_link(__MODULE__, state)\n");
    out.push_str("    end\n\n");
    out.push_str("    def init(state) do\n");
    out.push_str("      {:ok, state}\n");
    out.push_str("    end\n\n");

    // Handle trait_call messages from Rust
    out.push_str("    def handle_cast({:trait_call, method, args_json, reply_id}, registrations) do\n");
    out.push_str("      # Decode JSON args and dispatch to registered handler\n");
    out.push_str("      case decode_args_and_dispatch(method, args_json, registrations) do\n");
    out.push_str("        {:ok, response} ->\n");
    out.push_str("          Native.complete_trait_call(reply_id, response)\n");
    out.push_str("        {:error, reason} ->\n");
    out.push_str("          error_response = %{\"error\" => reason}\n");
    out.push_str("          Native.complete_trait_call(reply_id, error_response)\n");
    out.push_str("      end\n");
    out.push_str("      {:noreply, registrations}\n");
    out.push_str("    end\n\n");

    // Helper to decode JSON args and dispatch to registered handler
    out.push_str("    defp decode_args_and_dispatch(method, args_json, registrations) do\n");
    out.push_str("      # Find handler entry for the method\n");
    out.push_str("      case find_handler(method, registrations) do\n");
    out.push_str("        nil ->\n");
    out.push_str("          {:error, \"Handler not registered for method: #{method}\"}\n");
    out.push_str("        {^method, _metadata, handler} ->\n");
    out.push_str("          # Decode JSON args (assumes handler accepts a single arg)\n");
    out.push_str("          case Jason.decode(args_json) do\n");
    out.push_str("            {:ok, args} ->\n");
    out.push_str("              # Call the registered handler with decoded args\n");
    out.push_str("              try do\n");
    out.push_str("                response = handler.(args)\n");
    out.push_str("                # Encode response to JSON\n");
    out.push_str("                case Jason.encode(response) do\n");
    out.push_str("                  {:ok, response_json} -> {:ok, response_json}\n");
    out.push_str("                  {:error, reason} -> {:error, \"Failed to encode response: #{reason}\"}\n");
    out.push_str("                end\n");
    out.push_str("              rescue\n");
    out.push_str("                e ->\n");
    out.push_str("                  {:error, \"Handler raised exception: #{inspect(e)}\"}\n");
    out.push_str("              end\n");
    out.push_str("            {:error, reason} ->\n");
    out.push_str("              {:error, \"Failed to decode args: #{reason}\"}\n");
    out.push_str("          end\n");
    out.push_str("      end\n");
    out.push_str("    end\n\n");

    // Helper to find handler entry by method name in registrations list
    out.push_str("    defp find_handler(_method, []), do: nil\n");
    out.push_str(
        "    defp find_handler(target, [{name, _metadata, _handler} = entry | _rest]) when name == target do\n",
    );
    out.push_str("      entry\n");
    out.push_str("    end\n");
    out.push_str("    defp find_handler(target, [_head | rest]) do\n");
    out.push_str("      find_handler(target, rest)\n");
    out.push_str("    end\n\n");

    out.push_str("  end\n\n");
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust rustler glue module (`service.rs`).
///
/// For each service this emits:
/// - A message-passing handler bridge struct that wraps a `LocalPid` and sends
///   `{:trait_call, ...}` messages to the Elixir GenServer, awaiting responses
///   via a `complete_trait_call` NIF.
/// - A `#[rustler::nif(schedule = "DirtyCpu")]` NIF function that accepts
///   registrations (as Elixir terms), builds the service, and drives entrypoints.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async)]\n\n");
    out.push_str("use rustler::{Encoder, LocalPid, NifResult, OwnedEnv, ResourceArc, types::atom::Atom};\n");
    out.push_str("use rustler::Error as NifError;\n");
    out.push_str("use std::collections::HashMap;\n");
    out.push_str("use std::sync::Arc;\n");
    out.push_str("use std::sync::atomic::{AtomicU64, Ordering};\n");
    out.push_str("use std::sync::{Mutex, OnceLock};\n");
    // GAP 1: Import the core crate for trait types and RouteBuilder
    out.push_str(&format!("use {}::*;\n\n", core_import));

    out.push_str("/// Atom constants used by the service NIFs.\n");
    out.push_str("mod atoms {\n");
    out.push_str("    rustler::atoms! {\n");
    out.push_str("        ok,\n");
    out.push_str("        error,\n");
    out.push_str("        trait_call,\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("static REPLY_ID_COUNTER: AtomicU64 = AtomicU64::new(0);\n\n");

    // Global registry of pending oneshot senders, keyed by reply_id. The handler
    // bridge inserts a sender when it sends a `{:trait_call, ...}` message to the
    // Elixir GenServer; the GenServer (once it has processed the call) invokes
    // the `complete_trait_call` NIF with the reply_id and the JSON response,
    // which removes the sender from the map and forwards the response through
    // the oneshot channel.
    out.push_str(
        "type TraitReplySender = tokio::sync::oneshot::Sender<String>;\n\
         type TraitReplyMap = Mutex<HashMap<u64, TraitReplySender>>;\n\n\
         static TRAIT_REPLY_MAP: OnceLock<TraitReplyMap> = OnceLock::new();\n\n\
         fn trait_reply_map() -> &'static TraitReplyMap {\n    \
             TRAIT_REPLY_MAP.get_or_init(|| Mutex::new(HashMap::new()))\n\
         }\n\n",
    );

    // Top-level `complete_trait_call` NIF — invoked by the Elixir GenServer
    // after handling a `{:trait_call, ...}` message. Removes the pending
    // sender for `reply_id` from the global map and forwards `response_json`
    // through the oneshot channel; the awaiting handler-bridge dispatch then
    // resolves with the JSON body.
    out.push_str(
        "/// Complete a pending trait call with the JSON response from Elixir.\n\
         #[rustler::nif]\n\
         pub fn complete_trait_call(reply_id: u64, response_json: String) -> Atom {\n    \
             if let Some(tx) = trait_reply_map().lock().unwrap().remove(&reply_id) {\n        \
                 let _ = tx.send(response_json);\n    \
             }\n    \
             atoms::ok()\n\
         }\n\n",
    );

    // Emit one handler bridge per unique handler contract referenced
    let referenced_contracts: Vec<&HandlerContractDef> = {
        let mut names: Vec<&str> = api
            .services
            .iter()
            .flat_map(|s| s.registrations.iter())
            .map(|r| r.callback_contract.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names.iter().filter_map(|n| find_contract(api, n)).collect()
    };

    for contract in &referenced_contracts {
        gen_handler_bridge(&mut out, contract, &core_import);
    }

    // Emit one NIF per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_nif(&mut out, service, ep, api, &core_import);
        }

        // Emit registration variant NIFs
        for reg in &service.registrations {
            for variant in &reg.variants {
                gen_registration_variant_nif(&mut out, service, reg, variant, api, &core_import);
            }
        }
    }

    out
}

/// Emit the message-passing handler bridge struct + trait impl.
///
/// The Elixir GenServer pattern:
/// 1. Rust bridge holds a `LocalPid` (safe to send across threads via Rustler's guarantees).
/// 2. When dispatch is called, bridge serializes request to JSON and sends:
///    `{:trait_call, method_name, args_json, reply_id}` to the pid.
/// 3. Bridge awaits response via a oneshot channel, keyed by reply_id.
/// 4. Elixir GenServer receives, calls the registered handler, and sends back a response.
/// 5. Bridge receives and deserializes response to the wire response type.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Elixir{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;
    let _unused = bridge_name.clone(); // silence warnings, used in format!() strings

    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Leading dispatch parameters the bridge ignores (e.g. a foreign framework type the
    // contract's dispatch method receives but the wire bridge does not consume). Their concrete
    // types cannot be reconstructed from the sanitized surface, so the library supplies them
    // verbatim via `dispatch_extra_params`. Each is emitted as a `, {decl}` prefix argument.
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    out.push_str(&format!(
        "/// Generated rustler bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps an Elixir GenServer pid so it can be used\n\
         /// as `Arc<dyn {trait_name}>` from Rust async code.\n\
         /// Uses message-passing to avoid blocking the BEAM scheduler.\n\
         /// Pending replies are stored in the module-level `TRAIT_REPLY_MAP`\n\
         /// keyed by `reply_id`; the GenServer completes them via the\n\
         /// `complete_trait_call` NIF.\n\
         pub struct {bridge_name} {{\n    \
             pid: LocalPid,\n\
         }}\n\n"
    ));

    out.push_str(&format!(
        "impl {bridge_name} {{\n    \
             /// Create a bridge from an Elixir GenServer pid.\n    \
             pub fn new(pid: LocalPid) -> Self {{\n        \
                 Self {{ pid }}\n    \
             }}\n\
         }}\n\n"
    ));

    // SAFETY: LocalPid is thread-safe in Rustler (it's an atom reference).
    out.push_str(&format!(
        "// SAFETY: LocalPid is Send+Sync as guaranteed by Rustler.\n\
         unsafe impl Send for {bridge_name} {{}}\n\
         unsafe impl Sync for {bridge_name} {{}}\n\n"
    ));

    // Build request/response type paths
    let req_path = if req_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{resp_type}")
    };

    // The future's `Output` is the contract dispatch's real return type when the library
    // supplies one (`dispatch_return_type`); otherwise the bridge yields the wire response
    // wrapped in a boxed-error `Result`. When a `response_adapter` is configured, the inner
    // fallible computation produces the wire `Result` and the adapter converts it into the
    // dispatch return type — keeping the generator ignorant of the library's response model.
    let box_err = "Box<dyn std::error::Error + Send + Sync>";
    let wire_output = format!("Result<{resp_path}, {box_err}>");
    let output_type = contract
        .dispatch_return_type
        .clone()
        .unwrap_or_else(|| wire_output.clone());
    let tail = match &contract.response_adapter {
        Some(adapter) => format!("{adapter}(outcome)"),
        None => "outcome".to_string(),
    };

    // Trait impl. Returns a boxed future directly (the canonical object-safe
    // async-trait shape) instead of via the async_trait macro, so it satisfies
    // a contract whose dispatch method is hand-written as
    // `-> Pin<Box<dyn Future<..> + Send + '_>>`.
    out.push_str(&format!(
        "impl {core_import}::{trait_name} for {bridge_name} {{\n    \
             fn {dispatch_name}(\n        \
                 &self{extra_param},\n        \
                 {wire_name}: {req_path},\n    \
             ) -> std::pin::Pin<Box<dyn std::future::Future<Output = {output_type}> + Send + '_>> {{\n        \
                 Box::pin(async move {{\n            \
                     let outcome: {wire_output} = async move {{\n                \
                         let request_json = serde_json::to_string(&{wire_name})\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                         let reply_id = REPLY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);\n                \
                         let (tx, rx) = tokio::sync::oneshot::channel();\n                \
                         trait_reply_map().lock().unwrap().insert(reply_id, tx);\n\n                \
                         // Send trait_call message to Elixir GenServer\n                \
                         {{\n                    \
                             let pid = self.pid;\n                    \
                             let method_name = \"{dispatch_name}\";\n                    \
                             let request_json_clone = request_json.clone();\n                    \
                             tokio::task::spawn_blocking(move || {{\n                        \
                                 let mut env = OwnedEnv::new();\n                        \
                                 let _ = env.send_and_clear(&pid, |env| {{\n                            \
                                     (Atom::from_str(env, \"trait_call\").unwrap(),\n                            \
                                      method_name, request_json_clone.as_str(), reply_id).encode(env)\n                        \
                                 }});\n                    \
                             }}).await.ok();\n                \
                         }}\n\n                \
                         // Await response\n                \
                         let response_json = rx.await\n                    \
                             .map_err(|e| Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)) as {box_err})?;\n\n                \
                         let response: {resp_path} = serde_json::from_str(&response_json)\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n                \
                         Ok(response)\n            \
                     }}\n            \
                     .await;\n\n            \
                     {tail}\n        \
                 }})\n    \
             }}\n\
         }}\n\n"
    ));
}

/// Emit the `#[rustler::nif]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list and any entrypoint params.
/// 2. Constructs the native service owner via its constructor.
/// 3. Iterates registrations and wraps each in the appropriate bridge.
/// 4. Calls the owner's registration methods.
/// 5. Calls the owner's entrypoint (blocking if `Run`, async if async).
fn gen_run_nif(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature with lifetime-annotated Term
    let mut params = vec!["registrations: rustler::Term<'_>".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = params.join(", ");

    out.push_str(&format!(
        "/// Drive `{owner_path}::{ep_method}` from Elixir.\n\
         ///\n\
         /// This NIF is scheduled on the dirty CPU scheduler to avoid blocking\n\
         /// the BEAM scheduler during the (potentially long) run operation.\n\
         ///\n\
         /// # Arguments\n\
         ///\n\
         /// - `registrations` — Elixir list of `{{method_name, metadata, handler}}` tuples\n\
         ///   where `handler` is an Elixir function/closure that accepts request JSON and returns response JSON.\n"
    ));
    for p in &ep.params {
        out.push_str(&format!("/// - `{}` — entrypoint parameter\n", p.name));
    }
    out.push_str("///\n");
    out.push_str("/// # Returns\n");
    out.push_str("/// `:ok` or `{{:error, reason}}` after the entrypoint completes.\n");

    out.push_str(&format!(
        "#[rustler::nif(schedule = \"DirtyCpu\")]\n\
         pub fn {fn_name}({param_sig}) -> NifResult<Atom> {{\n"
    ));

    out.push_str("    // Parse registrations from Elixir term\n");
    out.push_str("    let registration_list: Vec<rustler::Term<'_>> = registrations\n");
    out.push_str("        .decode::<Vec<rustler::Term<'_>>>()\n");
    out.push_str("        .unwrap_or_else(|_| vec![]);\n\n");

    out.push_str("    // Build the service owner from its constructor\n");
    out.push_str(&format!("    let mut owner = {owner_path}::new();\n\n"));

    out.push_str("    // Register handlers from Elixir registrations\n");
    out.push_str("    // Each registration entry is a tuple: {method_name, metadata, handler_pid}\n");
    out.push_str("    for reg_entry in registration_list {\n");
    out.push_str("        if let Ok((method_name, metadata, handler_pid)) = reg_entry.decode::<(String, rustler::Term<'_>, rustler::LocalPid)>()\n");
    out.push_str("        {\n");

    // Generate dispatch for each registration
    for (i, reg) in service.registrations.iter().enumerate() {
        let contract_name = &reg.callback_contract;
        let reg_method = &reg.method;
        let metadata_param_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
        let bridge_wrapper = format!("Elixir{contract_name}Bridge");

        if i == 0 {
            out.push_str("            ");
        } else {
            out.push_str("            } else ");
        }

        out.push_str(&format!("if method_name == \"{}\" {{\n", reg_method));

        // Decode metadata if present
        if !metadata_param_names.is_empty() {
            // The Elixir registration method always wraps metadata in a tuple `{...}`
            // (see gen_registration_method), so a single param `path` arrives as the
            // 1-element Elixir tuple `{path}`. A 1-element Elixir tuple decodes to a Rust
            // 1-tuple `(T,)`, so emit a trailing comma when there is exactly one param.
            let trailing = if metadata_param_names.len() == 1 { "," } else { "" };
            let tuple_types = reg
                .metadata_params
                .iter()
                .map(|p| {
                    // Opaque types are passed as ResourceArc<super::T> where super::T is the
                    // local lib-module wrapper (implements rustler::Resource). The wildcard
                    // import in service.rs would shadow a bare `T` name, so qualify with `super::`.
                    if let TypeRef::Named(n) = &p.ty {
                        if api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque) {
                            return format!("rustler::ResourceArc<super::{}>", n);
                        }
                    }
                    typeref_to_rust_type(&p.ty, core_import)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let tuple_types_with_trailing = format!("{}{}", tuple_types, trailing);
            out.push_str(&format!(
                "                if let Ok(({names}{trailing})) = metadata.decode::<({types})>()\n",
                names = metadata_param_names.join(", "),
                trailing = trailing,
                types = tuple_types_with_trailing
            ));
            out.push_str("                {\n");
            // Decode and bind opaque metadata params to locals for later use.
            // ResourceArc<super::T> derefs to super::T (the local wrapper); wrapper.inner is
            // Arc<CoreType>. Call as_ref() on the Arc to get &CoreType, then clone to own it.
            for meta_param in reg.metadata_params.iter() {
                let is_opaque = if let TypeRef::Named(n) = &meta_param.ty {
                    api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque)
                } else {
                    false
                };
                if is_opaque {
                    if let TypeRef::Named(n) = &meta_param.ty {
                        out.push_str(&format!(
                            "                    let {pname}: {core_import}::{name} = (*{pname}).inner.as_ref().clone();\n",
                            pname = meta_param.name,
                            core_import = core_import,
                            name = n,
                        ));
                    }
                }
            }
            out.push_str(&format!(
                "                    let bridge = {bridge_wrapper}::new(handler_pid);\n"
            ));
            // GAP 3: Cast bridge to Arc<dyn HandlerContract> via upcast
            out.push_str(&format!(
                "                    let handler: Arc<dyn {core_import}::{trait_name}> = Arc::new(bridge);\n",
                trait_name = &reg.callback_contract
            ));
            let args_list = metadata_param_names
                .iter()
                .map(|name| format!("{}, ", name))
                .collect::<String>();
            // GAP 4: Call with 2 args (metadata + handler), not 3 (metadata + wrapper + handler)
            out.push_str(&format!(
                "                    let _ = owner.{reg_method}({}handler);\n",
                args_list
            ));
            out.push_str("                }\n");
        } else {
            out.push_str(&format!(
                "                let bridge = {bridge_wrapper}::new(handler_pid);\n"
            ));
            // GAP 3: Cast bridge to Arc<dyn HandlerContract>
            out.push_str(&format!(
                "                let handler: Arc<dyn {core_import}::{trait_name}> = Arc::new(bridge);\n",
                trait_name = &reg.callback_contract
            ));
            // GAP 4: Call with handler directly
            out.push_str(&format!("                let _ = owner.{reg_method}(handler);\n"));
        }
    }

    if !service.registrations.is_empty() {
        out.push_str("            }\n");
    }
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    // Generate entrypoint call
    out.push_str("    // Call the entrypoint method\n");
    match ep.kind {
        EntrypointKind::Run => {
            // For async run, we need to block on the future
            let ep_params = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
            out.push_str("    let rt = tokio::runtime::Runtime::new().map_err(|_e| {\n");
            out.push_str("        NifError::Atom(\"runtime_error\")\n");
            out.push_str("    })?;\n\n");
            if ep.params.is_empty() {
                out.push_str("    let result = rt.block_on(owner.run());\n");
            } else {
                out.push_str(&format!("    let result = rt.block_on(owner.run({}));\n", ep_params));
            }
            out.push_str("    match result {\n");
            out.push_str("        Ok(_) => Ok(atoms::ok()),\n");
            out.push_str("        Err(_e) => Err(NifError::Atom(\"error\")),\n");
            out.push_str("    }\n");
        }
        EntrypointKind::Finalize => {
            // For finalize, call the method by its actual name (e.g. `into_router`).
            let ep_params = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
            if ep.params.is_empty() {
                out.push_str(&format!("    match owner.{}() {{\n", ep_method));
            } else {
                out.push_str(&format!("    match owner.{}({}) {{\n", ep_method, ep_params));
            }
            out.push_str("        Ok(_) => Ok(atoms::ok()),\n");
            out.push_str("        Err(_e) => Err(NifError::Atom(\"error\")),\n");
            out.push_str("    }\n");
        }
    }

    out.push_str("}\n\n");
}

/// Emit a NIF for one registration variant.
///
/// The variant builds a wrapper (if `wrapper_call` is set) and calls the base
/// registration method with the constructed wrapper + fixed args + free args.
fn gen_registration_variant_nif(
    out: &mut String,
    service: &ServiceDef,
    base_reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let variant_name = &variant.name;
    let nif_name = format!("{}_{}", service_snake, variant_name);
    let base_method = &base_reg.method;
    let contract_name = &base_reg.callback_contract;
    let bridge_wrapper = format!("Elixir{contract_name}Bridge");
    let owner_path = &service.rust_path;

    // Build NIF signature
    let mut params = vec!["registrations: rustler::Term<'_>".to_owned()];
    for param in &variant.signature_params {
        let rust_ty = typeref_to_rust_type(&param.ty, core_import);
        params.push(format!("{}: {}", param.name, rust_ty));
    }
    params.push("handler: rustler::LocalPid".to_owned());
    let param_sig = params.join(", ");

    out.push_str(&format!(
        "/// Registration variant `{}` for the `{}` base method.\n\
         ///\n\
         /// This NIF pre-builds the wrapper and delegates to the base registration.\n\
         #[rustler::nif(schedule = \"DirtyCpu\")]\n\
         pub fn {}({}) -> NifResult<Atom> {{\n",
        variant_name, base_method, nif_name, param_sig
    ));

    // Parse registrations
    out.push_str("    let registration_list: Vec<rustler::Term<'_>> = registrations\n");
    out.push_str("        .decode::<Vec<rustler::Term<'_>>>()\n");
    out.push_str("        .unwrap_or_else(|_| vec![]);\n\n");

    // Build service owner
    out.push_str(&format!("    let mut owner = {owner_path}::new();\n\n"));

    // Build wrapper if needed
    if let Some(wrapper_call) = &variant.wrapper_call {
        let wrapper_type_path = &wrapper_call.wrapper_type_path;
        let wrapper_type_name = &wrapper_call.wrapper_type_name;
        let constructor_method = &wrapper_call.constructor_method;

        out.push_str(&format!(
            "    // Build {} via {}\n",
            wrapper_type_name, wrapper_type_path
        ));
        out.push_str(&format!(
            "    let wrapper = {wrapper_type_path}::{constructor_method}(\n"
        ));

        for arg in &wrapper_call.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed {
                    param_name: _,
                    value_expr,
                } => {
                    out.push_str(&format!("        {},\n", value_expr));
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    out.push_str(&format!("        {},\n", param.name));
                }
            }
        }

        out.push_str("    );\n\n");
    }

    // Register handlers
    out.push_str("    // Register the handler with wrapper or direct metadata\n");
    out.push_str("    for reg_entry in registration_list {\n");
    out.push_str(
        "        if let Ok((_method, _metadata, handler_pid)) = reg_entry.decode::<(String, rustler::Term<'_>, rustler::LocalPid)>()\n",
    );
    out.push_str("        {\n");

    let metadata_param_names: Vec<&str> = base_reg.metadata_params.iter().map(|p| p.name.as_str()).collect();

    if !metadata_param_names.is_empty() {
        let trailing = if metadata_param_names.len() == 1 { "," } else { "" };
        let tuple_types = base_reg
            .metadata_params
            .iter()
            .map(|p| {
                // Opaque types use super:: to name the local lib-module wrapper that implements
                // rustler::Resource. The wildcard import in service.rs would shadow a bare name.
                if let TypeRef::Named(n) = &p.ty {
                    if api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque) {
                        return format!("rustler::ResourceArc<super::{}>", n);
                    }
                }
                typeref_to_rust_type(&p.ty, core_import)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let tuple_types_with_trailing = format!("{}{}", tuple_types, trailing);

        out.push_str(&format!(
            "            if let Ok(({names}{trailing})) = _metadata.decode::<({types})>()\n",
            names = metadata_param_names.join(", "),
            trailing = trailing,
            types = tuple_types_with_trailing
        ));
        out.push_str("            {\n");

        for meta_param in base_reg.metadata_params.iter() {
            let is_opaque = if let TypeRef::Named(n) = &meta_param.ty {
                api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque)
            } else {
                false
            };
            if is_opaque {
                if let TypeRef::Named(n) = &meta_param.ty {
                    // ResourceArc<super::T> derefs to the local wrapper super::T; wrapper.inner
                    // is Arc<CoreType>. Use as_ref() then clone() to obtain an owned CoreType.
                    out.push_str(&format!(
                        "                let {pname}: {core_import}::{name} = (*{pname}).inner.as_ref().clone();\n",
                        pname = meta_param.name,
                        core_import = core_import,
                        name = n,
                    ));
                }
            }
        }

        out.push_str(&format!(
            "                let bridge = {bridge_wrapper}::new(handler_pid);\n"
        ));
        // GAP 3 (variant): Cast bridge to Arc<dyn HandlerContract>
        out.push_str(&format!(
            "                let handler: Arc<dyn {core_import}::{contract_name}> = Arc::new(bridge);\n",
            contract_name = base_reg.callback_contract
        ));

        // Call base registration with metadata + handler (not wrapper)
        // GAP 4 (variant): 2-arg call, not 3-arg
        if let Some(wrapper_call) = &variant.wrapper_call {
            let _metadata_param = &wrapper_call.metadata_param;
            out.push_str(&format!(
                "                let _ = owner.{}({}, handler);\n",
                base_method,
                metadata_param_names.join(", ")
            ));
        } else {
            out.push_str(&format!(
                "                let _ = owner.{}({}, handler);\n",
                base_method,
                metadata_param_names.join(", ")
            ));
        }

        out.push_str("            }\n");
    } else {
        out.push_str(&format!(
            "            let bridge = {bridge_wrapper}::new(handler_pid);\n"
        ));
        out.push_str(&format!(
            "            let _ = owner.{}(std::sync::Arc::new(bridge));\n",
            base_method
        ));
    }

    out.push_str("        }\n");
    out.push_str("    }\n\n");

    out.push_str("    Ok(atoms::ok())\n");
    out.push_str("}\n\n");
}

/// Map a `TypeRef` to a Rust type string for use in generated NIF signatures.
fn typeref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "u8".to_owned(),
                PrimitiveType::U16 => "u16".to_owned(),
                PrimitiveType::U32 => "u32".to_owned(),
                PrimitiveType::U64 => "u64".to_owned(),
                PrimitiveType::I8 => "i8".to_owned(),
                PrimitiveType::I16 => "i16".to_owned(),
                PrimitiveType::I32 => "i32".to_owned(),
                PrimitiveType::I64 => "i64".to_owned(),
                PrimitiveType::F32 => "f32".to_owned(),
                PrimitiveType::F64 => "f64".to_owned(),
                PrimitiveType::Usize => "usize".to_owned(),
                PrimitiveType::Isize => "isize".to_owned(),
            }
        }
        TypeRef::Bytes => "Vec<u8>".to_owned(),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_rust_type(k, core_import),
            typeref_to_rust_type(v, core_import)
        ),
        TypeRef::Unit => "()".to_owned(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
        TypeRef::Json => "serde_json::Value".to_owned(),
        TypeRef::Path => "std::path::PathBuf".to_owned(),
        TypeRef::Duration => "std::time::Duration".to_owned(),
    }
}

// ──────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the rustler backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust rustler glue
/// - `{elixir_pkg}/service.ex`   — idiomatic Elixir module
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(
        config.output_paths.get("elixir"),
        &config.name,
        "packages/elixir/native/{name}_nif/src/",
    );

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // Elixir module — pass the consumer's module prefix so the
    // service module can `alias <Prefix>.Native`.
    let (_, module_prefix) = super::helpers::get_module_info(api, config);
    let service_ex = gen_service_ex(api, &module_prefix);

    // Determine Elixir package output directory
    let elixir_pkg = config.output_paths.get("elixir").map(PathBuf::from).unwrap_or_else(|| {
        let app_name = config.elixir_app_name();
        PathBuf::from(format!("packages/elixir/lib/{}", app_name))
    });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: elixir_pkg.join("service.ex"),
            content: service_ex,
            generated_header: true,
        },
    ])
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
        ServiceDef, TypeRef,
    };

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one configurator, one registration
    ///   (bound to an async handler contract), and Run + Finalize entrypoints.
    /// - One [`HandlerContractDef`] with wire request/response DTO names.
    fn make_fixture_surface() -> ApiSurface {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create a new service owner.".to_owned(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let configurator = MethodDef {
            name: "with_timeout".to_owned(),
            params: vec![ParamDef {
                name: "timeout_ms".to_owned(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("TestService".to_owned()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Set request timeout.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Register a request handler for a path and method.".to_owned(),
            variants: vec![],
        };

        let run_ep = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![ParamDef {
                name: "addr".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("ServiceError".to_owned()),
            doc: "Run the service.".to_owned(),
        };

        let finalize_ep = EntrypointDef {
            method: "into_router".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Named("Router".to_owned()),
            error_type: None,
            doc: "Consume and convert into a router.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![configurator],
            registrations: vec![registration],
            entrypoints: vec![run_ep, finalize_ep],
            doc: "A test service owner.".to_owned(),
            cfg: None,
        };

        let dispatch_method = MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "request".to_owned(),
                ty: TypeRef::Named("RequestData".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("ResponseData".to_owned()),
            is_async: true,
            is_static: false,
            error_type: Some("HandlerError".to_owned()),
            doc: "Dispatch a request.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: dispatch_method,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("ResponseData".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Async trait for handling requests.".to_owned(),
        };

        ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![contract],
            ..ApiSurface::default()
        }
}
    }

    /// `gen_service_ex` emits a module named after the service owner.
    #[test]
    fn elixir_output_contains_service_module() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        // The compiled namespace is implicitly `Elixir.<Name>`, so the emitted
        // source must NOT re-prefix it (`defmodule Elixir.<Name>` compiles to
        // `Elixir.Elixir.<Name>`).
        assert!(
            output.contains("defmodule TestService do"),
            "expected `defmodule TestService do` in output:\n{output}"
        );
    }

    /// `gen_service_ex` emits a struct definition.
    #[test]
    fn elixir_output_contains_struct_definition() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("defstruct"),
            "expected `defstruct` in output:\n{output}"
        );
        assert!(
            output.contains(":registrations"),
            "expected `:registrations` field in output:\n{output}"
        );
    }

    /// `gen_service_ex` emits a constructor.
    #[test]
    fn elixir_output_contains_constructor() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(output.contains("def new("), "expected `def new(` in output:\n{output}");
    }

    /// `gen_service_ex` emits configurator methods.
    #[test]
    fn elixir_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("def with_timeout("),
            "expected `with_timeout` configurator:\n{output}"
        );
    }

    /// `gen_service_ex` emits a registration method.
    #[test]
    fn elixir_output_contains_registration() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("def add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
    }

    /// `gen_service_ex` emits a GenServer module.
    #[test]
    fn elixir_output_contains_genserver_module() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("defmodule TestService.Handler do"),
            "expected `TestService.Handler` GenServer:\n{output}"
        );
        assert!(
            output.contains("use GenServer"),
            "expected `use GenServer` in output:\n{output}"
        );
    }

    /// `gen_service_ex` emits the `run` entrypoint.
    #[test]
    fn elixir_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(output.contains("def run("), "expected `def run(` in output:\n{output}");
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct ElixirRequestHandlerBridge"),
            "expected `ElixirRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for ElixirRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[rustler::nif]` run entry point.
    #[test]
    fn rust_output_contains_nif_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[rustler::nif(schedule = \"DirtyCpu\")]"),
            "expected `#[rustler::nif(schedule = \"DirtyCpu\")]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
        );
    }

    /// Full `generate()` call returns two files when services are non-empty.
    #[test]
    fn generate_returns_two_files_for_non_empty_services() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert_eq!(files.len(), 2, "expected 2 generated files, got {}", files.len());
        let paths: Vec<&str> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"service.rs"), "expected service.rs in output");
        assert!(paths.contains(&"service.ex"), "expected service.ex in output");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    /// Elixir GenServer `handle_cast` actually decodes args and calls handler.
    #[test]
    fn elixir_genserver_handle_cast_decodes_args_and_dispatches() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");

        // Assert that handle_cast decodes args_json
        assert!(
            output.contains("decode_args_and_dispatch(method, args_json, registrations)"),
            "expected decode_args_and_dispatch call in handle_cast:\n{output}"
        );

        // Assert that it calls complete_trait_call with reply_id
        assert!(
            output.contains("Native.complete_trait_call(reply_id, response)"),
            "expected Native.complete_trait_call(reply_id, response) call:\n{output}"
        );

        // Assert that there are NO stub comments or empty placeholders
        assert!(
            !output.contains("simplified stub"),
            "found 'simplified stub' comment — dispatch should not be stubbed:\n{output}"
        );
        assert!(
            !output.contains("TODO"),
            "found TODO comment in dispatch logic:\n{output}"
        );
        assert!(
            !output.contains("# This is a simplified stub"),
            "found stub marker in dispatch:\n{output}"
        );
    }

    /// Elixir GenServer dispatch helper decodes JSON and calls registered handler.
    #[test]
    fn elixir_genserver_dispatch_helper_invokes_handler() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");

        // Assert that decode_args_and_dispatch helper exists
        assert!(
            output.contains("defp decode_args_and_dispatch(method, args_json, registrations) do"),
            "expected decode_args_and_dispatch helper function:\n{output}"
        );

        // Assert that it decodes JSON
        assert!(
            output.contains("Jason.decode(args_json)"),
            "expected Jason.decode(args_json) in dispatch:\n{output}"
        );

        // Assert that it calls the registered handler
        assert!(
            output.contains("response = handler.(args)"),
            "expected handler.(args) invocation:\n{output}"
        );

        // Assert that response is encoded back to JSON
        assert!(
            output.contains("Jason.encode(response)"),
            "expected Jason.encode(response) in dispatch:\n{output}"
        );

        // Assert that find_handler helper looks up by method name
        assert!(
            output.contains("defp find_handler"),
            "expected find_handler helper function:\n{output}"
        );
    }

    /// Rust NIF parses registrations and constructs service owner.
    #[test]
    fn rust_nif_parses_registrations_and_constructs_owner() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);

        // Assert that registrations are parsed from Elixir term
        assert!(
            output.contains("let registration_list: Vec<rustler::Term<'_>> = registrations"),
            "expected registration list parsing in NIF:\n{output}"
        );

        // Assert that service owner is constructed
        assert!(
            output.contains("let mut owner = my_crate::TestService::new()"),
            "expected owner construction in NIF:\n{output}"
        );

        // Assert that registrations are iterated and dispatched
        assert!(
            output.contains("for reg_entry in registration_list"),
            "expected registration iteration in NIF:\n{output}"
        );

        // Assert that no stub markers remain
        assert!(
            !output.contains("TODO: Parse registrations"),
            "found TODO in registration parsing — should be implemented:\n{output}"
        );
        assert!(
            !output.contains("For now, return a stub"),
            "found stub return in NIF — should be fully implemented:\n{output}"
        );
    }

    /// No empty-JSON or stub responses in generated code.
    ///
    /// Verifies that the Rust NIF actually invokes `owner.run(...)` or `owner.finalize(...)`
    /// and does not emit stub placeholder responses.
    #[test]
    fn no_stub_responses_in_generated_code() {
        let surface = make_fixture_surface();
        let config = make_test_config();

        let elixir_output = gen_service_ex(&surface, "");
        let rust_output = gen_service_rs(&surface, &config);

        // Elixir should not return empty JSON map
        assert!(
            !elixir_output.contains("response = {:ok, %{}}"),
            "found stub response {{:ok, %{{}}}} in Elixir generated code:\n{elixir_output}"
        );

        // Elixir should not have commented-out complete_trait_call
        assert!(
            !elixir_output.contains("# Native.complete_trait_call"),
            "found commented-out complete_trait_call in Elixir:\n{elixir_output}"
        );

        // Rust should not contain stub comment markers
        assert!(
            !rust_output.contains("would be called here"),
            "found 'would be called here' stub comment in Rust NIF:\n{rust_output}"
        );
        assert!(
            !rust_output.contains("would happen here"),
            "found 'would happen here' stub comment in Rust NIF:\n{rust_output}"
        );

        // Rust should actually call owner.run(...) or owner.finalize(...)
        assert!(
            rust_output.contains("owner.run(") || rust_output.contains("owner.finalize("),
            "Rust NIF should call owner.run(...) or owner.finalize(...), found neither:\n{rust_output}"
        );

        // Rust should register handlers before calling entrypoint
        assert!(
            rust_output.contains("ElixirRequestHandlerBridge"),
            "Rust NIF should create handler bridge instances:\n{rust_output}"
        );

        // Regression: Rust should NOT contain illegal if-let type ascription pattern
        // (`: Result<...> =` on if-let patterns is a syntax error in Rust)
        assert!(
            !rust_output.contains("): Result<"),
            "found illegal if-let type ascription pattern '): Result<' in generated Rust:\n{rust_output}"
        );

        // Rust Term args must be lifetime-annotated (Term<'_> or Term<'a>)
        assert!(
            rust_output.contains("Term<'_>"),
            "expected lifetime-annotated Term<'_> in generated Rust NIF signature:\n{rust_output}"
        );
    }

    /// Verify that registration variant style is respected in generated Elixir code.
    ///
    /// Regression test for issue #26: the rustler backend must pattern-match on
    /// `RegistrationVariantStyle` and emit the appropriate Elixir registration forms.
    #[test]
    fn registration_variant_style_hybrid_emits_both_forms() {
        let mut surface = make_fixture_surface();
        let _config = make_test_config();

        // Attach a Hybrid-styled variant `get` so the variant emission loop runs.
        // The base `add_handler` is emitted unconditionally by gen_registration_method;
        // RegistrationVariantStyle gates only the per-variant verb/builder emission.
        surface.services[0].registrations[0]
            .variants
            .push(crate::core::ir::RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![crate::core::ir::RegistrationVariantOverride {
                    param_name: "method".to_owned(),
                    value_expr: "\"GET\"".to_owned(),
                }],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: None,
                style: RegistrationVariantStyle::Hybrid,
            });

        let elixir_output = gen_service_ex(&surface, "");

        // Hybrid → verb-decorator form
        assert!(
            elixir_output.contains("def get(app, path, handler) do"),
            "expected verb-decorator form 'def get(app, path, handler) do' in Elixir output:\n{elixir_output}"
        );

        // Hybrid → builder form
        assert!(
            elixir_output.contains("def get_decorator(app, path) do"),
            "expected builder form 'def get_decorator(app, path) do' in Elixir output:\n{elixir_output}"
        );
    }

    /// Verify that send_trait_call message is emitted in generated handler bridge.
    ///
    /// Regression test for issue #119: the handler bridge must send the trait_call message
    /// to the Elixir GenServer via OwnedEnv::send_and_clear, not just await silently.
    #[test]
    fn handler_bridge_sends_trait_call_message() {
        let surface = make_fixture_surface();
        let config = make_test_config();

        let rust_output = gen_service_rs(&surface, &config);

        // Verify that OwnedEnv is imported
        assert!(
            rust_output.contains("OwnedEnv"),
            "expected OwnedEnv import in generated code"
        );

        // Verify that send_and_clear is called
        assert!(
            rust_output.contains("env.send_and_clear(&pid"),
            "expected env.send_and_clear(&pid, ...) call in generated handler bridge:\n{rust_output}"
        );

        // Verify that trait_call atom is sent
        assert!(
            rust_output.contains("Atom::from_str(env, \"trait_call\")"),
            "expected atom::from_str for 'trait_call' in generated message:\n{rust_output}"
        );

        // Verify that the method name is included in the message
        assert!(
            rust_output.contains("method_name"),
            "expected method_name variable in trait_call message"
        );

        // Verify that request_json is included
        assert!(
            rust_output.contains("request_json_clone"),
            "expected request JSON to be sent in trait_call message"
        );

        // Verify that reply_id is included
        assert!(
            rust_output.contains("reply_id)"),
            "expected reply_id in trait_call tuple"
        );

        // Regression: ensure the old commented-out line is not present
        assert!(
            !rust_output.contains("// crate::nif_support::send_trait_call"),
            "found old commented-out send_trait_call in output — should be replaced with real call"
        );

        // Verify spawn_blocking wraps the send
        assert!(
            rust_output.contains("tokio::task::spawn_blocking(move || {"),
            "expected spawn_blocking to wrap the message send"
        );
    }

    /// Verify that Rust codegen emits core crate import + trait implementation.
    /// This tests GAP 1 (core import) and GAP 3 (trait cast).
    #[test]
    fn rust_codegen_emits_core_import_and_trait_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let rust_output = gen_service_rs(&surface, &config);

        // GAP 1: Verify core crate import
        assert!(
            rust_output.contains("use my_crate::*;"),
            "expected core crate wildcard import in gen_service_rs output:\n{rust_output}"
        );

        // GAP 3: Verify bridge trait implementation
        assert!(
            rust_output.contains("impl my_crate::RequestHandler for ElixirRequestHandlerBridge"),
            "expected trait impl for bridge in generated output:\n{rust_output}"
        );

        // Verify handler variable bindings for trait casting
        assert!(
            rust_output.contains("let handler: Arc<dyn my_crate::RequestHandler> = Arc::new(bridge);"),
            "expected handler trait cast in registration code:\n{rust_output}"
        );

        // Verify bridge struct definition
        assert!(
            rust_output.contains("pub struct ElixirRequestHandlerBridge"),
            "expected ElixirRequestHandlerBridge struct definition:\n{rust_output}"
        );
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_test_config() -> ResolvedCrateConfig {
        use crate::core::config::resolved::ResolvedCrateConfig;
        ResolvedCrateConfig {
            name: "my-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }
}
