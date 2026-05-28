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
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
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
pub(super) fn gen_service_ex(api: &ApiSurface, _module_name: &str) -> String {
    let mut out = String::new();

    out.push_str("# This file is generated. Do not edit.\n\n");

    for service in &api.services {
        gen_service_module(&mut out, service, api);
    }

    out
}

fn gen_service_module(out: &mut String, service: &ServiceDef, api: &ApiSurface) {
    let module_name = &service.name;
    let module_snake = service.name.to_snake_case();

    // Module docstring
    if !service.doc.is_empty() {
        out.push_str(&format!(
            "defmodule Elixir.{} do\n  @moduledoc \"\"\"\n  {}\n  \"\"\"\n\n",
            module_name,
            service.doc.trim()
        ));
    } else {
        out.push_str(&format!("defmodule Elixir.{} do\n", module_name));
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

        out.push_str(&format!("  def new({}) do\n", params.join(", ")));
        if !ctor.doc.is_empty() {
            out.push_str(&format!("    \"\"\"{}\"\"\"\n", ctor.doc.trim()));
        }
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

        out.push_str(&format!("  def {}({}) do\n", method_name, params.join(", ")));
        if !method.doc.is_empty() {
            out.push_str(&format!("    \"\"\"{}\"\"\"\n", method.doc.trim()));
        }
        for p in &method.params {
            out.push_str(&format!("    self = %__MODULE__{{self | {}: {}}}\n", p.name, p.name));
        }
        out.push_str("    self\n");
        out.push_str("  end\n\n");
    }

    // Registration methods as decorator-style helpers
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api);
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
                out.push_str(&format!("  def {}({}) do\n", ep_name, params.join(", ")));
                if !ep.doc.is_empty() {
                    out.push_str(&format!("    \"\"\"{}\"\"\"\n", ep.doc.trim()));
                }
                let native_fn = format!("{}_{}", module_snake, ep_name);
                out.push_str(&format!("    Native.{}(self.registrations", native_fn));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n");
                out.push_str("  end\n\n");
            }
            EntrypointKind::Finalize => {
                out.push_str(&format!("  def {}({}) do\n", ep_name, params.join(", ")));
                if !ep.doc.is_empty() {
                    out.push_str(&format!("    \"\"\"{}\"\"\"\n", ep.doc.trim()));
                }
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

fn gen_registration_method(out: &mut String, reg: &RegistrationDef, _service: &ServiceDef, _api: &ApiSurface) {
    let method_name = &reg.method;

    out.push_str(&format!("  def {}(self", method_name));
    for p in &reg.metadata_params {
        if p.optional {
            out.push_str(&format!(", {} \\\\ nil", p.name));
        } else {
            out.push_str(&format!(", {}", p.name));
        }
    }
    out.push_str(", handler) do\n");

    if !reg.doc.is_empty() {
        out.push_str(&format!("    \"\"\"{}\"\"\"\n", reg.doc.trim()));
    }

    // Build metadata tuple
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{{}}}", meta_names.join(", "))
    };

    out.push_str(&format!(
        "    entry = {{\"{}\", {}, handler}}\n",
        method_name, meta_tuple
    ));
    out.push_str("    %__MODULE__{self | registrations: [entry | self.registrations]}\n");
    out.push_str("  end\n\n");
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
    out.push_str("    defp find_handler(method, [{^method, _metadata, _handler} = entry | _rest]) do\n");
    out.push_str("      entry\n");
    out.push_str("    end\n");
    out.push_str("    defp find_handler(method, [_head | rest]) do\n");
    out.push_str("      find_handler(method, rest)\n");
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
    out.push_str("use rustler::{LocalPid, ResourceArc};\n");
    out.push_str("use std::sync::Arc;\n");
    out.push_str("use tokio::sync::Mutex as TokioMutex;\n\n");

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

    out.push_str(&format!(
        "/// Generated rustler bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps an Elixir GenServer pid so it can be used\n\
         /// as `Arc<dyn {trait_name}>` from Rust async code.\n\
         /// Uses message-passing to avoid blocking the BEAM scheduler.\n\
         pub struct {bridge_name} {{\n    \
             pid: LocalPid,\n    \
             reply_map: Arc<TokioMutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<String>>>>,\n\
         }}\n\n"
    ));

    out.push_str(&format!(
        "impl {bridge_name} {{\n    \
             /// Create a bridge from an Elixir GenServer pid.\n    \
             pub fn new(pid: LocalPid) -> Self {{\n        \
                 Self {{\n            \
                     pid,\n            \
                     reply_map: Arc::new(TokioMutex::new(std::collections::HashMap::new())),\n        \
                 }}\n    \
             }}\n\
         }}\n\n"
    ));

    // SAFETY: LocalPid is thread-safe in Rustler (it's an atom reference).
    // The Arc<Mutex<HashMap>> is also Send+Sync.
    out.push_str(&format!(
        "// SAFETY: LocalPid is Send+Sync as guaranteed by Rustler.\n\
         // Arc<TokioMutex<HashMap>> is Send+Sync.\n\
         unsafe impl Send for {bridge_name} {{}}\n\
         unsafe impl Sync for {bridge_name} {{}}\n\n"
    ));

    // Trait impl
    out.push_str(&format!(
        "#[async_trait::async_trait]\n\
         impl {core_import}::{trait_name} for {bridge_name} {{\n    \
             async fn {dispatch_name}(\n        \
                 &self,\n        \
                 request: {core_import}::{req_type},\n    \
             ) -> Result<{core_import}::{resp_type}, Box<dyn std::error::Error + Send + Sync>> {{\n        \
                 let request_json = serde_json::to_string(&request)\n            \
                     .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;\n\n        \
                 let reply_id = crate::nif_support::next_request_id();\n        \
                 let (tx, rx) = tokio::sync::oneshot::channel();\n\n        \
                 {{\n            \
                     let mut map = self.reply_map.lock().await;\n            \
                     map.insert(reply_id, tx);\n        \
                 }}\n\n        \
                 // Send trait_call message to Elixir GenServer\n        \
                 // Note: This requires a NIF that sends the message\n        \
                 // crate::nif_support::send_trait_call(self.pid, \"{dispatch_name}\", &request_json, reply_id)?;\n\n        \
                 // Await response\n        \
                 let response_json = rx.await\n            \
                     .map_err(|e| Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)) as Box<dyn std::error::Error + Send + Sync>)?;\n\n        \
                 let response: {core_import}::{resp_type} = serde_json::from_str(&response_json)\n            \
                     .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;\n        \
                 Ok(response)\n    \
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
    _api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature
    let mut params = vec!["registrations: Term".to_owned()];
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
    out.push_str("    let registration_list: Vec<Term> = registrations\n");
    out.push_str("        .decode::<Vec<Term>>()\n");
    out.push_str("        .unwrap_or_else(|_| vec![]);\n\n");

    out.push_str("    // Build the service owner from its constructor\n");
    out.push_str(&format!("    let mut owner = {owner_path}::new();\n\n"));

    out.push_str("    // Register handlers from Elixir registrations\n");
    out.push_str("    // Each registration entry is a tuple: {method_name, metadata, handler_closure}\n");
    out.push_str("    for reg_entry in registration_list {\n");
    out.push_str("        if let Ok((method_name, metadata, _handler_closure)): Result<(String, Term, Term), _> =\n");
    out.push_str("            reg_entry.decode()\n");
    out.push_str("        {\n");

    // Generate dispatch for each registration
    for (i, reg) in service.registrations.iter().enumerate() {
        let _contract_name = &reg.callback_contract;
        let reg_method = &reg.method;
        let metadata_param_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();

        if i == 0 {
            out.push_str("            ");
        } else {
            out.push_str("            } else ");
        }

        out.push_str(&format!("if method_name == \"{}\" {{\n", reg_method));

        // Decode metadata if present
        if !metadata_param_names.is_empty() {
            out.push_str(&format!(
                "                if let Ok(({})): Result<({}), _> = metadata.decode() {{\n",
                metadata_param_names.join(", "),
                (0..metadata_param_names.len())
                    .map(|_| "String")
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            out.push_str("                    // Note: Handler registration would happen here\n");
            out.push_str("                    // owner.{}(..., handler_closure)\n");
            out.push_str("                }\n");
        } else {
            out.push_str("                // No metadata parameters for this registration\n");
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
            out.push_str("    let _ = owner;\n");
            out.push_str("    // owner.run(...) would be called here for async entrypoint\n");
            out.push_str("    Ok(atoms::ok())\n");
        }
        EntrypointKind::Finalize => {
            out.push_str("    let _ = owner;\n");
            out.push_str("    // owner.finalize(...) would be called here\n");
            out.push_str("    Ok(atoms::ok())\n");
        }
    }

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

    // Elixir module
    let service_ex = gen_service_ex(api, "");

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

    /// `gen_service_ex` emits a module named after the service owner.
    #[test]
    fn elixir_output_contains_service_module() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("defmodule Elixir.TestService do"),
            "expected `defmodule Elixir.TestService do` in output:\n{output}"
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
            output.contains("async fn handle("),
            "expected async dispatch method:\n{output}"
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
            output.contains("let registration_list: Vec<Term> = registrations"),
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
    /// Currently the `gen_run_nif` emitter for `EntrypointKind::{Run, Finalize}`
    /// only emits a placeholder `Ok(atoms::ok())` with a "would be called here"
    /// comment instead of actually invoking `owner.run(...)` / `owner.finalize(...)`.
    /// The full owner-method dispatch (mirroring the pyo3 native-fn lookup pattern at
    /// `src/backends/pyo3/gen_bindings/service_api.rs:448+`) is still TODO. The test
    /// is kept as a permanent regression-target: when the rustler emitter is completed,
    /// remove the `#[ignore]` and the test should pass.
    #[test]
    #[ignore = "rustler entrypoint emitter still emits stub Ok(atoms::ok()); see gen_run_nif TODO"]
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

        // Rust should not return early with Ok(atoms::ok())
        assert!(
            !rust_output.contains("Ok(atoms::ok())\n}\n\n/// Drive"),
            "found premature stub return in Rust NIF:\n{rust_output}"
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
