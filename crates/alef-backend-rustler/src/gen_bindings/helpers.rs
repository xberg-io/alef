use ahash::AHashSet;
use alef_codegen::doc_emission::doc_first_paragraph_joined;
use alef_codegen::shared::binding_fields;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{FieldDef, TypeDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::HashMap;

use crate::template_env;

/// Get module name and prefix from config or derive from crate name.
pub(super) fn get_module_info(_api: &alef_core::ir::ApiSurface, config: &ResolvedCrateConfig) -> (String, String) {
    let app_name = config.elixir_app_name();
    let module_prefix = app_name.to_pascal_case();
    (app_name, module_prefix)
}

/// Generate a type-appropriate unimplemented body for Rustler (no todo!()).
pub(super) fn gen_rustler_unimplemented_body(return_type: &TypeRef, fn_name: &str, has_error: bool) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(String::from(\"{err_msg}\"))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!("panic!(\"alef: {fn_name} not auto-delegatable\")"),
        }
    }
}

/// Map a return type, wrapping opaque Named types in ResourceArc.
/// Handles both bare opaque returns (T) and optional opaque returns (Option<T>).
pub(super) fn map_return_type(
    ty: &TypeRef,
    mapper: &crate::type_map::RustlerMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(n) if opaque_types.contains(n) => format!("ResourceArc<{n}>"),
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                if opaque_types.contains(n) {
                    return format!("Option<ResourceArc<{n}>>");
                }
            }
            mapper.map_type(ty)
        }
        _ => mapper.map_type(ty),
    }
}

/// Generate the `{AppModule}.Native` Elixir module with NIF stubs for all functions and methods.
pub(super) fn gen_native_ex(
    api: &alef_core::ir::ApiSurface,
    app_name: &str,
    app_module: &str,
    _crate_name: &str,
    config: &ResolvedCrateConfig,
    exclude_functions: &AHashSet<&str>,
    exclude_types: &AHashSet<&str>,
) -> String {
    let mut out = String::with_capacity(1024);

    let repo_url = config.github_repo();
    // The env var that forces a local source build: {APP_NAME_UPPER}_BUILD
    let build_env_var = format!("{}_BUILD", app_name.to_uppercase());

    out.push_str(&hash::header(CommentStyle::Hash));
    let ctx = minijinja::context! {
        app_module => app_module,
        app_name => app_name,
        repo_url => repo_url,
        build_env_var => build_env_var,
    };
    out.push_str(&template_env::render("native_module_header.jinja", ctx));

    // Stubs for top-level API functions
    let mut last_was_multiline = true;
    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let fn_name = if func.is_async {
            let n = func.name.as_str();
            if n.ends_with("_async") {
                n.to_string()
            } else {
                format!("{n}_async")
            }
        } else {
            func.name.clone()
        };
        let underscored_params: Vec<String> = func
            .params
            .iter()
            .map(|p| format!("_{}", p.name.to_snake_case()))
            .collect();
        last_was_multiline = write_nif_stub(&mut out, &fn_name, &underscored_params, last_was_multiline);

        // For functions that have a visitor bridge (FunctionParam pattern), also emit the
        // async visitor variant stub plus the visitor_reply NIF stub (once).
        let has_visitor_bridge = config.trait_bridges.iter().any(|b| {
            b.bind_via != alef_core::config::BridgeBinding::OptionsField
                && func.params.iter().any(|p| {
                    b.param_name.as_deref() == Some(p.name.as_str()) || {
                        let named = match &p.ty {
                            TypeRef::Named(n) => Some(n.as_str()),
                            TypeRef::Optional(inner) => {
                                if let TypeRef::Named(n) = inner.as_ref() {
                                    Some(n.as_str())
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        named.map(|n| b.type_alias.as_deref() == Some(n)).unwrap_or(false)
                    }
                })
        });
        if has_visitor_bridge {
            // Params for convert_with_visitor: same as convert but visitor is required (not optional).
            let with_visitor_params: Vec<String> = func
                .params
                .iter()
                .map(|p| format!("_{}", p.name.to_snake_case()))
                .collect();
            last_was_multiline = write_nif_stub(
                &mut out,
                &format!("{fn_name}_with_visitor"),
                &with_visitor_params,
                last_was_multiline,
            );
        }

        // For functions that have an options_field visitor bridge, emit
        // `{fn_name}_with_visitor` stub with the original params + `_visitor` appended.
        let has_options_field_bridge = config.trait_bridges.iter().any(|b| {
            b.bind_via == alef_core::config::BridgeBinding::OptionsField
                && func.params.iter().any(|p| {
                    let type_name = match &p.ty {
                        TypeRef::Named(n) => Some(n.as_str()),
                        TypeRef::Optional(inner) => {
                            if let TypeRef::Named(n) = inner.as_ref() {
                                Some(n.as_str())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    type_name.is_some_and(|n| b.options_type.as_deref() == Some(n))
                })
        });
        if has_options_field_bridge {
            // Params: all original params (options is Option<String>) + _visitor at the end.
            let mut with_visitor_params: Vec<String> = func
                .params
                .iter()
                .map(|p| format!("_{}", p.name.to_snake_case()))
                .collect();
            with_visitor_params.push("_visitor".to_string());
            last_was_multiline = write_nif_stub(
                &mut out,
                &format!("{fn_name}_with_visitor"),
                &with_visitor_params,
                last_was_multiline,
            );
        }
    }

    // visitor_reply stub: emitted once when there are visitor bridges.
    if !config.trait_bridges.is_empty() {
        last_was_multiline = write_nif_stub(
            &mut out,
            "visitor_reply",
            &["_ref_id".to_string(), "_result".to_string()],
            last_was_multiline,
        );
        // Visitor trait call completion stubs (for async trait methods)
        last_was_multiline = write_nif_stub(
            &mut out,
            "complete_trait_call",
            &["_reply_id".to_string(), "_result_json".to_string()],
            last_was_multiline,
        );
        last_was_multiline = write_nif_stub(
            &mut out,
            "fail_trait_call",
            &["_reply_id".to_string(), "_error_message".to_string()],
            last_was_multiline,
        );
    }

    // Trait bridge registration stubs (register_fn, unregister_fn, clear_fn).
    // These are emitted for each trait bridge that doesn't exclude Elixir.
    for bridge in &config.trait_bridges {
        if bridge.exclude_languages.contains(&"elixir".to_string()) {
            continue;
        }

        // register_fn stub: takes (env, pid, name) -> Atom
        if let Some(register_fn) = &bridge.register_fn {
            let params = vec!["_pid".to_string(), "_name".to_string()];
            last_was_multiline = write_nif_stub(&mut out, register_fn, &params, last_was_multiline);
        }

        // unregister_fn stub: takes (env, name) -> Atom
        if let Some(unregister_fn) = &bridge.unregister_fn {
            let params = vec!["_name".to_string()];
            last_was_multiline = write_nif_stub(&mut out, unregister_fn, &params, last_was_multiline);
        }

        // clear_fn stub: takes (env) -> Atom (no args besides env)
        if let Some(clear_fn) = &bridge.clear_fn {
            let params = vec![];
            last_was_multiline = write_nif_stub(&mut out, clear_fn, &params, last_was_multiline);
        }
    }

    // Streaming-adapter method keys are emitted as start/next pairs below — skip
    // them in the regular method-stub loop.
    let streaming_method_keys: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
        .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
        .collect();

    // Stubs for type methods
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
    {
        for method in typ
            .methods
            .iter()
            .filter(|m| !exclude_functions.contains(m.name.as_str()))
            .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
        {
            let nif_fn_name = if method.is_async {
                format!("{}_{}_async", typ.name.to_lowercase(), method.name)
            } else {
                format!("{}_{}", typ.name.to_lowercase(), method.name)
            };

            let mut underscored_params: Vec<String> = Vec::new();
            if method.receiver.is_some() {
                underscored_params.push("_obj".to_string());
            }
            for p in &method.params {
                underscored_params.push(format!("_{}", elixir_safe_param_name(&p.name)));
            }

            last_was_multiline = write_nif_stub(&mut out, &nif_fn_name, &underscored_params, last_was_multiline);
        }
    }

    // Stubs for streaming-adapter NIF pairs: `{owner_lc}_{name}_start(_obj, _req)`
    // and `{owner_lc}_{name}_next(_handle)`. Both NIFs are scheduled on DirtyCpu.
    for adapter in config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
    {
        let Some(owner) = adapter.owner_type.as_deref() else {
            continue;
        };
        let owner_lc = owner.to_lowercase();
        let start_fn = format!("{owner_lc}_{}_start", adapter.name);
        let next_fn = format!("{owner_lc}_{}_next", adapter.name);
        let mut start_params = vec!["_obj".to_string()];
        for p in &adapter.params {
            start_params.push(format!("_{}", elixir_safe_param_name(&p.name)));
        }
        last_was_multiline = write_nif_stub(&mut out, &start_fn, &start_params, last_was_multiline);
        last_was_multiline = write_nif_stub(&mut out, &next_fn, &["_handle".to_string()], last_was_multiline);
    }

    // Stubs for *_from_json helper NIFs (generated for all serde-capable types)
    for typ in api.types.iter().filter(|t| {
        !t.is_trait && !t.is_opaque && !t.fields.is_empty() && t.has_serde && !exclude_types.contains(t.name.as_str())
    }) {
        let from_json_fn_name = format!("{}_from_json", typ.name.to_snake_case());
        // *_from_json takes a JSON string and returns Result<Type, String>
        let params = vec!["_json".to_string()];
        last_was_multiline = write_nif_stub(&mut out, &from_json_fn_name, &params, last_was_multiline);
    }

    out.push_str(&template_env::render(
        "native_module_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Write a NIF stub line, splitting onto two lines when the single-line form exceeds 120 chars.
///
/// `prev_was_multiline` should be `true` when the previous stub was multi-line. This is used
/// to insert a single blank separator line around multi-line defs (mix format requirement):
/// - single → multi: blank before multi
/// - multi → single: blank before single
/// - multi → multi: single blank between them (not double)
/// - single → single: no blank
///
/// Returns `true` when this stub was written in multi-line form.
///
/// Single-line form:  `  def fn_name(args), do: :erlang.nif_error(:nif_not_loaded)`
/// Two-line form:
/// ```elixir
///   def fn_name(args),
///     do: :erlang.nif_error(:nif_not_loaded)
/// ```
fn write_nif_stub(out: &mut String, fn_name: &str, params: &[String], prev_was_multiline: bool) -> bool {
    let args = params.join(", ");
    // Elixir convention: omit parens on zero-arg defs
    let sig = if args.is_empty() {
        fn_name.to_string()
    } else {
        format!("{fn_name}({args})")
    };
    // "  def <sig>, do: :erlang.nif_error(:nif_not_loaded)"
    let single_line_len = 6 + sig.len() + 40;
    if single_line_len > 120 {
        let ctx = minijinja::context! { sig => sig, prev_was_multiline => prev_was_multiline };
        out.push_str(&template_env::render("nif_stub_multi_line.jinja", ctx));
        true
    } else {
        let ctx = minijinja::context! { sig => sig };
        out.push_str(&template_env::render("nif_stub_single_line.jinja", ctx));
        false
    }
}

/// Generate a `defmodule {AppModule}.{TypeName}` file with a `defstruct` for a non-opaque type.
pub(super) fn gen_elixir_struct_module(
    typ: &TypeDef,
    app_module: &str,
    enum_defaults: &HashMap<String, String>,
    opaque_types: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&hash::header(CommentStyle::Hash));

    let doc_first = if !typ.doc.is_empty() {
        doc_first_paragraph_joined(&typ.doc).replace('"', "\\\"")
    } else {
        String::new()
    };
    let ctx = minijinja::context! {
        app_module => app_module,
        type_name => &typ.name,
        has_doc => !typ.doc.is_empty(),
        doc => doc_first,
    };
    out.push_str(&template_env::render("struct_module_header.jinja", ctx));

    // Emit @type t typespec before defstruct
    let default_types: AHashSet<String> = enum_defaults.keys().cloned().collect();
    out.push_str("  @type t :: %__MODULE__{\n");

    let fields: Vec<_> = binding_fields(&typ.fields).collect();
    if !fields.is_empty() {
        for (i, field) in fields.iter().enumerate() {
            let field_name = field.name.to_snake_case();
            let field_type = elixir_typespec(&field.ty, opaque_types, &default_types);
            let field_defaults_to_nil = matches!(
                field.ty,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
            );
            let field_type_with_optional =
                if (field.optional || field_defaults_to_nil) && !matches!(field.ty, TypeRef::Optional(_)) {
                    format!("{field_type} | nil")
                } else {
                    field_type
                };

            if i == fields.len() - 1 {
                // Last field: no comma
                out.push_str(&format!("    {field_name}: {field_type_with_optional}\n"));
            } else {
                out.push_str(&format!("    {field_name}: {field_type_with_optional},\n"));
            }
        }
    }
    out.push_str("  }\n\n");

    // defstruct with defaults - use bare keyword list style (mix format compliant)
    if fields.is_empty() {
        out.push_str(&template_env::render("struct_empty.jinja", minijinja::context! {}));
    } else {
        out.push_str("  defstruct ");
        for (i, field) in fields.iter().enumerate() {
            let default = elixir_field_default(field, &field.ty, enum_defaults, opaque_types);
            let name = field.name.to_snake_case();
            if i == 0 {
                out.push_str(&template_env::render(
                    "elixir_enum_field_first.jinja",
                    minijinja::context! {
                        name => &name,
                        default => &default,
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "elixir_enum_field_rest.jinja",
                    minijinja::context! {
                        name => &name,
                        default => &default,
                    },
                ));
            }
        }
        out.push('\n');
    }
    out.push_str(&template_env::render(
        "struct_module_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Generate an idiomatic Elixir wrapper module for an opaque type.
///
/// The native NIF returns the opaque type as a Rustler resource (passed as
/// `reference()` to Elixir). This wrapper wraps the reference in a struct
/// (`%TreeSitterLanguagePack.Parser{ref: ...}`) and exposes the type's
/// methods as functions that delegate to the corresponding NIF
/// (`{type_lower}_{method_name}`) provided by `{AppModule}.Native`.
///
/// Async methods delegate to the `_async` NIF variant (see
/// `gen_bindings/functions.rs`). Methods that map to a `Streaming` adapter
/// emit a `Stream.unfold/2`-based wrapper that drives the underlying
/// `_start`/`_next` NIF pair instead of attempting a sync call.
pub(super) fn gen_elixir_opaque_module(typ: &TypeDef, app_module: &str, config: &ResolvedCrateConfig) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&hash::header(CommentStyle::Hash));

    let doc_first = if !typ.doc.is_empty() {
        doc_first_paragraph_joined(&typ.doc).replace('"', "\\\"")
    } else {
        String::new()
    };
    let ctx = minijinja::context! {
        app_module => app_module,
        type_name => &typ.name,
        has_doc => !typ.doc.is_empty(),
        doc => doc_first,
    };
    out.push_str(&template_env::render("struct_module_header.jinja", ctx));

    // Native alias and reference-only struct.
    out.push_str(&format!("  alias {app_module}.Native\n\n"));
    out.push_str("  defstruct [:ref]\n\n");
    out.push_str("  @type t :: %__MODULE__{ref: reference()}\n\n");

    let type_lower = typ.name.to_lowercase();

    // Streaming-adapter method names owned by this type. Sync calls would fail
    // (the NIFs are `{name}_start`/`{name}_next`); emit a Stream wrapper instead.
    let streaming_method_names: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
        .filter(|a| a.owner_type.as_deref() == Some(typ.name.as_str()))
        .map(|a| a.name.clone())
        .collect();

    // Constructor for types with a default — wraps the native default reference.
    if typ.has_default {
        out.push_str("  @doc \"Build a default instance.\"\n");
        out.push_str("  @spec new() :: t()\n");
        out.push_str("  def new do\n");
        out.push_str(&format!("    %__MODULE__{{ref: Native.{type_lower}_default()}}\n"));
        out.push_str("  end\n\n");
    }

    // Wrapper for each method. Methods with a receiver take the struct as the
    // first argument and pass `obj.ref` to the NIF. Static methods (no receiver)
    // are emitted as module-level functions.
    for method in &typ.methods {
        let method_name = method.name.to_snake_case();

        // Streaming methods: emit a Stream.unfold wrapper driving _start/_next NIFs.
        if streaming_method_names.contains(&method.name) {
            let start_fn = format!("{type_lower}_{}_start", method.name);
            let next_fn = format!("{type_lower}_{}_next", method.name);

            let mut def_args: Vec<String> = Vec::new();
            let mut start_call_args: Vec<String> = Vec::new();
            if method.receiver.is_some() {
                def_args.push("obj".to_string());
                start_call_args.push("obj.ref".to_string());
            }
            for p in &method.params {
                let safe = elixir_safe_param_name(&p.name);
                def_args.push(safe.clone());
                start_call_args.push(safe);
            }

            let doc_first = method.doc.lines().next().unwrap_or("").replace('"', "\\\"");
            if !doc_first.is_empty() {
                out.push_str(&format!("  @doc \"{doc_first}\"\n"));
            }
            out.push_str(&format!("  def {method_name}({}) do\n", def_args.join(", ")));
            out.push_str(&format!(
                "    case Native.{start_fn}({}) do\n",
                start_call_args.join(", ")
            ));
            out.push_str("      {:ok, handle} ->\n");
            out.push_str("        stream =\n");
            out.push_str("          Stream.unfold(handle, fn h ->\n");
            out.push_str(&format!("            case Native.{next_fn}(h) do\n"));
            out.push_str("              {:ok, nil} -> nil\n");
            out.push_str("              {:ok, chunk_json} when is_binary(chunk_json) ->\n");
            out.push_str("                {Jason.decode!(chunk_json, keys: :atoms), h}\n");
            out.push_str("              {:ok, chunk} -> {chunk, h}\n");
            out.push_str("              {:error, _} -> nil\n");
            out.push_str("            end\n");
            out.push_str("          end)\n");
            out.push_str("        {:ok, stream}\n");
            out.push_str("      {:error, reason} -> {:error, reason}\n");
            out.push_str("    end\n");
            out.push_str("  end\n\n");
            continue;
        }

        // Async methods delegate to the `_async` NIF unless the Rust name already
        // ends in `_async` (preserved per functions.rs convention).
        let nif_fn = if method.is_async {
            if method.name.ends_with("_async") {
                format!("{type_lower}_{}", method.name)
            } else {
                format!("{type_lower}_{}_async", method.name)
            }
        } else {
            format!("{type_lower}_{}", method.name)
        };

        let mut call_args: Vec<String> = Vec::new();
        let mut def_args: Vec<String> = Vec::new();
        if method.receiver.is_some() {
            def_args.push("obj".to_string());
            call_args.push("obj.ref".to_string());
        }
        for p in &method.params {
            let safe = elixir_safe_param_name(&p.name);
            def_args.push(safe.clone());
            call_args.push(safe);
        }

        let doc_first = method.doc.lines().next().unwrap_or("").replace('"', "\\\"");
        if !doc_first.is_empty() {
            out.push_str(&format!("  @doc \"{doc_first}\"\n"));
        }
        out.push_str(&format!("  def {method_name}({}) do\n", def_args.join(", ")));
        out.push_str(&format!("    Native.{nif_fn}({})\n", call_args.join(", ")));
        out.push_str("  end\n\n");
    }

    // Methods leave a trailing blank line after `end`; `mix format` rejects a
    // blank between the last def's `end` and the module's closing `end`.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str(&template_env::render(
        "struct_module_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Elixir built-in type names that must not be redefined with `@type`.
///
/// Emitting `@type list :: ...` shadows the built-in `list/0` and produces a
/// Dialyzer/Elixir compiler warning. Append `_variant` to any name that
/// collides with one of these identifiers.
const ELIXIR_BUILTIN_TYPES: &[&str] = &[
    "any",
    "as_boolean",
    "atom",
    "binary",
    "boolean",
    "byte",
    "char",
    "charlist",
    "float",
    "fun",
    "function",
    "identifier",
    "integer",
    "iodata",
    "iolist",
    "keyword",
    "list",
    "map",
    "mfa",
    "module",
    "no_return",
    "node",
    "none",
    "number",
    "pid",
    "port",
    "reference",
    "string",
    "struct",
    "term",
    "timeout",
    "tuple",
];

/// Return a `@type` name that does not collide with an Elixir built-in type.
///
/// If `name` matches one of the Elixir built-in type identifiers it is suffixed
/// with `_variant` so the generated `@type` declaration does not shadow the
/// built-in and trigger compiler or Dialyzer warnings.
pub(super) fn elixir_safe_type_name(name: &str) -> String {
    if ELIXIR_BUILTIN_TYPES.contains(&name) {
        format!("{name}_variant")
    } else {
        name.to_owned()
    }
}
/// Elixir built-in module attributes that cannot be used as custom `@attribute` names.
///
/// Emitting `@doc :doc` (for an enum variant named `Doc`) raises a compiler error because
/// `@doc` is a built-in module attribute. Append `_attr` when the snake_case variant name
/// collides with one of these identifiers.
const ELIXIR_RESERVED_MODULE_ATTRIBUTES: &[&str] = &[
    "after_compile",
    "before_compile",
    "behaviour",
    "callback",
    "compile",
    "deprecated",
    "derive",
    "dialyzer",
    "doc",
    "enforce_keys",
    "external_resource",
    "file",
    "impl",
    "moduledoc",
    "on_definition",
    "on_load",
    "opaque",
    "optional_callbacks",
    "spec",
    "type",
    "typedoc",
    "typep",
    "vsn",
];

/// Return a module attribute name that does not collide with an Elixir built-in attribute.
///
/// If `name` matches a reserved Elixir module attribute (e.g. `doc`, `type`, `spec`)
/// it is suffixed with `_attr` so the generated `@attribute` declaration does not
/// shadow the built-in and trigger a compiler error.
pub(super) fn elixir_safe_attr_name(name: &str) -> String {
    if ELIXIR_RESERVED_MODULE_ATTRIBUTES.contains(&name) {
        format!("{name}_attr")
    } else {
        name.to_owned()
    }
}

/// Elixir reserved words that cannot be used as parameter names.
const ELIXIR_RESERVED_WORDS: &[&str] = &[
    "after", "and", "catch", "cond", "do", "else", "end", "false", "fn", "for", "if", "in", "nil", "not", "or",
    "raise", "receive", "rescue", "true", "try", "unless", "when", "with",
];

/// Ensure a parameter name does not collide with an Elixir reserved word.
pub(super) fn elixir_safe_param_name(name: &str) -> String {
    let snake = name.to_snake_case();
    if ELIXIR_RESERVED_WORDS.contains(&snake.as_str()) {
        format!("{snake}_val")
    } else {
        snake
    }
}

/// Return an Elixir atom value (without leading `:`, as the template adds it).
/// If the atom contains non-identifier characters, it is quoted as `"atom:value"`.
///
/// Valid Elixir identifiers are: `[a-zA-Z_][a-zA-Z_0-9]*[?!]?`.
/// Atoms containing colons, dashes, or other special chars are wrapped as `"atom:value"`.
/// This is used for enum variant atom values that may contain `#[serde(rename)]` strings.
pub(super) fn elixir_safe_atom(atom_value: &str) -> String {
    // Check if atom is a valid Elixir identifier: [a-zA-Z_][a-zA-Z0-9_]*[?!]?
    fn is_valid_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let mut chars = s.chars();
        let first = chars.next().unwrap();
        if !first.is_ascii_alphabetic() && first != '_' {
            return false;
        }
        loop {
            match chars.next() {
                None => return true,
                Some(c) => {
                    if !c.is_ascii_alphanumeric() && c != '_' && c != '?' && c != '!' {
                        return false;
                    }
                    // ? and ! must be at the end
                    if (c == '?' || c == '!') && chars.as_str() != "" {
                        return false;
                    }
                }
            }
        }
    }

    if is_valid_identifier(atom_value) {
        atom_value.to_string()
    } else {
        format!(r#""{atom_value}""#)
    }
}

/// Generate a `defmodule {AppModule}.{EnumName}` file for an enum.
///
/// Simple enums (all variants have no fields) get a `@type t :: :variant1 | :variant2 | ...`
/// union type using snake_case atoms, mirroring the Rustler `NifUnitEnum` atom encoding.
///
/// Data enums (one or more variants have fields) get a module with per-variant type aliases
/// since Elixir has no single structural type for tagged union variants.
#[allow(dead_code)]
pub(super) fn gen_elixir_enum_module(enum_def: &alef_core::ir::EnumDef, app_module: &str) -> String {
    gen_elixir_enum_module_with_known_types(enum_def, app_module, &AHashSet::new())
}

pub(super) fn gen_elixir_enum_module_with_known_types(
    enum_def: &alef_core::ir::EnumDef,
    app_module: &str,
    known_types: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(256);

    out.push_str(&hash::header(CommentStyle::Hash));

    let doc_first = enum_def.doc.lines().next().unwrap_or("").replace('"', "\\\"");
    let ctx = minijinja::context! {
        app_module => app_module,
        enum_name => &enum_def.name,
        has_doc => !enum_def.doc.is_empty(),
        doc => doc_first,
    };
    out.push_str(&template_env::render("enum_module_header.jinja", ctx));

    let is_simple = enum_def.variants.iter().all(|v| v.fields.is_empty());

    if is_simple {
        // @type t :: :variant_one | :variant_two | ...
        // Rustler NifUnitEnum encodes variants as atoms using the variant name as-is,
        // but Elixir convention for atoms uses snake_case.
        let atom_arms: Vec<String> = enum_def
            .variants
            .iter()
            .map(|v| {
                let atom = v
                    .serde_rename
                    .clone()
                    .unwrap_or_else(|| alef_codegen::naming::pascal_to_snake(&v.name));
                format!(":{}", elixir_safe_atom(&atom))
            })
            .collect();
        // Emit multi-line @type when the single-line form exceeds 120 chars
        let single_line = format!("  @type t :: {}", atom_arms.join(" | "));
        if single_line.len() <= 120 {
            out.push_str(&template_env::render(
                "elixir_enum_type_single_line.jinja",
                minijinja::context! {
                    arms => &atom_arms.join(" | "),
                },
            ));
        } else {
            out.push_str("  @type t ::\n");
            for (i, arm) in atom_arms.iter().enumerate() {
                if i == 0 {
                    out.push_str(&template_env::render(
                        "elixir_enum_type_arm_first.jinja",
                        minijinja::context! {
                            arm => arm,
                        },
                    ));
                } else {
                    out.push_str(&template_env::render(
                        "elixir_enum_type_arm_rest.jinja",
                        minijinja::context! {
                            arm => arm,
                        },
                    ));
                }
            }
        }
        out.push('\n');

        // Module attributes for each variant value — convenient aliases
        for variant in &enum_def.variants {
            // Use original variant name (snake_cased) as the identifier, not serde_rename.
            let attr_name = elixir_safe_attr_name(&alef_codegen::naming::pascal_to_snake(&variant.name));
            // But the atom value should use serde_rename if available, properly quoted if needed.
            let atom_value = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| alef_codegen::naming::pascal_to_snake(&variant.name));
            let atom_literal = elixir_safe_atom(&atom_value);
            out.push_str(&template_env::render(
                "elixir_enum_attr.jinja",
                minijinja::context! {
                    attr_name => &attr_name,
                    atom_name => &atom_literal,
                },
            ));
        }
        out.push('\n');
        // Export the values so callers can reference MyEnum.variant_name/0
        for variant in &enum_def.variants {
            // Use original variant name (snake_cased) as the function identifier.
            let fn_name = alef_codegen::naming::pascal_to_snake(&variant.name);
            let attr_name = elixir_safe_attr_name(&fn_name);
            out.push_str(&template_env::render(
                "elixir_enum_accessor.jinja",
                minijinja::context! {
                    atom_name => &fn_name,
                    attr_name => &attr_name,
                },
            ));
        }
    } else {
        // Data enum: provide a @type t :: term() and per-variant type aliases
        out.push_str("  @type t :: term()\n");
        out.push('\n');
        for variant in &enum_def.variants {
            let variant_atom = format!(":{}", alef_codegen::naming::pascal_to_snake(&variant.name));
            let type_name = elixir_safe_type_name(&alef_codegen::naming::pascal_to_snake(&variant.name));
            if variant.fields.is_empty() {
                // Unit variant: just an atom
                out.push_str(&template_env::render(
                    "elixir_data_enum_unit_type.jinja",
                    minijinja::context! {
                        type_name => &type_name,
                        variant_atom => &variant_atom,
                    },
                ));
            } else {
                // Struct variant: a map with a type tag and payload-derived field names
                let field_types: Vec<String> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(idx, f)| {
                        // Determine the type name for type inference
                        let type_name = match &f.ty {
                            TypeRef::Named(n) => Some(n.as_str()),
                            TypeRef::String => Some("String"),
                            TypeRef::Bytes => Some("bytes"),
                            TypeRef::Char => Some("char"),
                            TypeRef::Path => Some("path"),
                            TypeRef::Json => Some("json"),
                            TypeRef::Primitive(p) => match p {
                                alef_core::ir::PrimitiveType::Bool => Some("bool"),
                                alef_core::ir::PrimitiveType::U8 => Some("u8"),
                                alef_core::ir::PrimitiveType::U16 => Some("u16"),
                                alef_core::ir::PrimitiveType::U32 => Some("u32"),
                                alef_core::ir::PrimitiveType::U64 => Some("u64"),
                                alef_core::ir::PrimitiveType::Usize => Some("usize"),
                                alef_core::ir::PrimitiveType::I8 => Some("i8"),
                                alef_core::ir::PrimitiveType::I16 => Some("i16"),
                                alef_core::ir::PrimitiveType::I32 => Some("i32"),
                                alef_core::ir::PrimitiveType::I64 => Some("i64"),
                                alef_core::ir::PrimitiveType::Isize => Some("isize"),
                                alef_core::ir::PrimitiveType::F32 => Some("f32"),
                                alef_core::ir::PrimitiveType::F64 => Some("f64"),
                            },
                            _ => None,
                        };

                        // Derive field name using payload-informed naming
                        let field_name =
                            elixir_field_name_with_type(&f.name, idx, type_name, &variant.name, variant.fields.len());

                        // Emit concrete type using elixir_typespec
                        // If the field type is a known API type, resolve to Module.t()
                        let field_type = if let TypeRef::Named(n) = &f.ty {
                            if known_types.contains(n) {
                                format!("{app_module}.{}.t()", n)
                            } else {
                                let opaque_types = AHashSet::new();
                                let default_types = AHashSet::new();
                                elixir_typespec(&f.ty, &opaque_types, &default_types)
                            }
                        } else {
                            let opaque_types = AHashSet::new();
                            let default_types = AHashSet::new();
                            elixir_typespec(&f.ty, &opaque_types, &default_types)
                        };

                        format!("{field_name}: {field_type}")
                    })
                    .collect();
                out.push_str(&template_env::render(
                    "elixir_data_enum_struct_type.jinja",
                    minijinja::context! {
                        type_name => &type_name,
                        variant_atom => &variant_atom,
                        field_types => field_types.join(", "),
                    },
                ));
            }
        }
    }

    out.push_str(&template_env::render(
        "enum_module_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Derive a payload-informed field name for union/enum tuple variants.
///
/// For tuple variants with a single payload, this function derives smarter names:
/// - If the field name is positional (like `_0`), infer from the type:
///   - Named type `PdfMetadata` with variant name `Pdf` → strip prefix "Pdf" → `metadata`
///   - Primitive type (String, bool, etc.) → use generic `value`
/// - If the field name is a struct field name (like `reason`), use it directly.
/// - For multiple tuple fields, use generic names: `value0`, `value1`, etc.
fn elixir_field_name_with_type(
    field_name: &str,
    field_idx: usize,
    field_type_name: Option<&str>,
    variant_name: &str,
    total_fields: usize,
) -> String {
    let stripped = field_name.trim_start_matches('_');

    // If field name is non-positional (not `_N`), use it directly (struct variant).
    if !stripped.is_empty() && !stripped.chars().all(|c| c.is_ascii_digit()) {
        return stripped.to_snake_case();
    }

    // For positional fields, derive from type if available and single field.
    if total_fields == 1 {
        if let Some(type_name) = field_type_name {
            // Try to strip variant name as prefix. E.g., `Pdf` variant with `PdfMetadata` type.
            if let Some(remainder) = type_name.strip_prefix(variant_name) {
                // Convert `Metadata` to `metadata`
                let derived = remainder.to_snake_case();
                if !derived.is_empty() {
                    return derived;
                }
            }

            // For primitive types (String, bool, etc.), use generic `value`.
            if is_primitive_type(type_name) {
                return "value".to_string();
            }
        }
    }

    // For multiple fields or when inference fails, use generic names.
    if total_fields > 1 {
        return format!("value{}", field_idx);
    }

    // Fallback: use `value` for single non-inferred field.
    "value".to_string()
}

/// Check if a type name is a primitive type (String, bool, integers, floats, etc.).
fn is_primitive_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "bool"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "isize"
            | "f32"
            | "f64"
            | "char"
            | "byte"
            | "unit"
    )
}

/// Format an integer literal with underscore separators for Elixir conventions.
/// E.g. 5242880 → "5_242_880". Numbers < 1000 are returned unchanged.
fn elixir_format_integer(n: i64) -> String {
    let (neg, s) = if n < 0 {
        (true, (-n).to_string())
    } else {
        (false, n.to_string())
    };
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push('_');
        }
        result.push(c);
    }
    let formatted: String = result.chars().rev().collect();
    if neg { format!("-{formatted}") } else { formatted }
}

/// Derive an Elixir default expression for a struct field.
fn elixir_field_default(
    field: &FieldDef,
    ty: &TypeRef,
    enum_defaults: &HashMap<String, String>,
    _opaque_types: &AHashSet<String>,
) -> String {
    use alef_core::ir::DefaultValue;

    // G7: Check if the field is nilable — if so, always default to nil.
    // A field is nilable if: field.optional=true OR ty=TypeRef::Optional(...)
    let is_nilable = field.optional || matches!(ty, TypeRef::Optional(_));
    if is_nilable {
        // Always default to nil for nilable fields, regardless of any typed_default.
        // This ensures the defstruct default aligns with the @type spec (T | nil).
        return "nil".to_string();
    }

    if let Some(td) = &field.typed_default {
        return match td {
            DefaultValue::BoolLiteral(b) => (if *b { "true" } else { "false" }).to_string(),
            DefaultValue::StringLiteral(s) => format!("\"{}\"", s.replace('"', "\\\"")),
            DefaultValue::IntLiteral(i) => elixir_format_integer(*i),
            DefaultValue::FloatLiteral(f) => format!("{f}"),
            DefaultValue::EnumVariant(v) => format!(":{}", v.to_snake_case()),
            DefaultValue::Empty => elixir_zero_value(ty, enum_defaults),
            DefaultValue::None => "nil".to_string(),
        };
    }

    // No typed_default: use type-appropriate zero
    elixir_zero_value(ty, enum_defaults)
}

/// Generate a type-appropriate zero/default value for Elixir.
///
/// G7: Defaults align with @type specs:
/// - String-like values → `nil` unless an explicit default is present
/// - Non-nilable numbers → `0` or `0.0`
/// - Non-nilable booleans → `false`
/// - Non-nilable lists → `[]`
/// - Non-nilable maps → `%{}`
/// - Struct/Named types → first variant default (enum) or `nil`
fn elixir_zero_value(ty: &TypeRef, enum_defaults: &HashMap<String, String>) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "false".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "nil".to_string(),
        TypeRef::Bytes => "<<>>".to_string(),
        TypeRef::Duration => "0".to_string(),
        TypeRef::Vec(_) => "[]".to_string(),
        TypeRef::Map(_, _) => "%{}".to_string(),
        TypeRef::Optional(_) => "nil".to_string(),
        TypeRef::Unit => "nil".to_string(),
        TypeRef::Named(name) => {
            if let Some(variant) = enum_defaults.get(name) {
                format!(":{variant}")
            } else {
                "nil".to_string()
            }
        }
    }
}

/// Map a TypeRef to an Elixir typespec string for `@spec` annotations.
///
/// `default_types` lists types that are passed as JSON strings at the NIF boundary
/// (types with `has_default = true`).  Their typespec is `String.t() | nil` rather
/// than `map()` because callers encode them with `Jason.encode!/1`.
pub(super) fn elixir_typespec(
    ty: &TypeRef,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String.t()".to_string(),
        TypeRef::Bytes => "binary()".to_string(),
        TypeRef::Unit => "nil".to_string(),
        TypeRef::Duration => "non_neg_integer()".to_string(),
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "boolean()".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "float()".to_string(),
            alef_core::ir::PrimitiveType::U8
            | alef_core::ir::PrimitiveType::U16
            | alef_core::ir::PrimitiveType::U32
            | alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::Usize => "non_neg_integer()".to_string(),
            alef_core::ir::PrimitiveType::I8
            | alef_core::ir::PrimitiveType::I16
            | alef_core::ir::PrimitiveType::I32
            | alef_core::ir::PrimitiveType::I64
            | alef_core::ir::PrimitiveType::Isize => "integer()".to_string(),
        },
        TypeRef::Named(name) => {
            if opaque_types.contains(name) {
                "reference()".to_string()
            } else if default_types.contains(name) {
                // Passed as an optional JSON string; nil means use defaults.
                "String.t() | nil".to_string()
            } else {
                "map()".to_string()
            }
        }
        TypeRef::Optional(inner) => {
            format!("{} | nil", elixir_typespec(inner, opaque_types, default_types))
        }
        TypeRef::Vec(inner) => {
            format!("[{}]", elixir_typespec(inner, opaque_types, default_types))
        }
        TypeRef::Map(_, _) => "map()".to_string(),
    }
}

/// Map a return TypeRef to an Elixir typespec for `@spec` return annotations.
pub(super) fn elixir_return_typespec(
    ty: &TypeRef,
    has_error: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    let base = elixir_typespec(ty, opaque_types, default_types);
    if has_error {
        format!("{{:ok, {}}} | {{:error, String.t()}}", base)
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{EnumDef, EnumVariant, FieldDef};

    #[test]
    fn test_elixir_field_name_with_type_payload_derived() {
        // Named type Pdf with variant Pdf → strip Pdf → metadata
        let name = elixir_field_name_with_type("_0", 0, Some("PdfMetadata"), "Pdf", 1);
        assert_eq!(name, "metadata");

        // Named type Excel with variant Excel → strip Excel → metadata
        let name = elixir_field_name_with_type("_0", 0, Some("ExcelMetadata"), "Excel", 1);
        assert_eq!(name, "metadata");

        // Docx variant with DocxMetadata type → strip Docx → metadata
        let name = elixir_field_name_with_type("_0", 0, Some("DocxMetadata"), "Docx", 1);
        assert_eq!(name, "metadata");
    }

    #[test]
    fn test_elixir_field_name_with_type_primitive() {
        // Primitive String type → value
        let name = elixir_field_name_with_type("_0", 0, Some("String"), "Error", 1);
        assert_eq!(name, "value");

        // Primitive bool type → value
        let name = elixir_field_name_with_type("_0", 0, Some("bool"), "Flag", 1);
        assert_eq!(name, "value");
    }

    #[test]
    fn test_elixir_field_name_with_type_multiple_fields() {
        // Multiple fields → generic value0, value1
        let name = elixir_field_name_with_type("_0", 0, None, "Pair", 2);
        assert_eq!(name, "value0");

        let name = elixir_field_name_with_type("_1", 1, None, "Pair", 2);
        assert_eq!(name, "value1");
    }

    #[test]
    fn test_elixir_field_name_with_type_named_field() {
        // Non-positional field name → use directly
        let name = elixir_field_name_with_type("reason", 0, Some("String"), "Error", 1);
        assert_eq!(name, "reason");
    }

    #[test]
    fn test_gen_elixir_enum_module_data_enum_with_payload_derived_names() {
        // Create FormatMetadata enum with Pdf(PdfMetadata) and Docx(DocxMetadata) variants
        let format_enum = EnumDef {
            name: "FormatMetadata".to_string(),
            rust_path: "my_crate::FormatMetadata".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pdf".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Named("PdfMetadata".into()),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Docx".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Named("DocxMetadata".into()),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let result = gen_elixir_enum_module(&format_enum, "Kreuzberg");

        // Should emit @type pdf with metadata field (not value_0) and concrete type (not term())
        assert!(
            result.contains("@type pdf :: %{type: :pdf, metadata: map()}"),
            "should use payload-derived 'metadata' field name with concrete type map(); got:\n{result}"
        );

        // Should emit @type docx with metadata field (not value_0) and concrete type (not term())
        assert!(
            result.contains("@type docx :: %{type: :docx, metadata: map()}"),
            "should use payload-derived 'metadata' field name with concrete type map(); got:\n{result}"
        );

        // Must not use the old generic name for variant fields
        assert!(
            !result.contains("value_0: term()"),
            "should not use generic value_0 field name with term() type; got:\n{result}"
        );
    }

    #[test]
    fn test_elixir_safe_atom_valid_identifier() {
        // Returns value without leading :, since template adds it
        assert_eq!(elixir_safe_atom("img"), "img");
        assert_eq!(elixir_safe_atom("picture_source"), "picture_source");
        assert_eq!(elixir_safe_atom("valid?"), "valid?");
        assert_eq!(elixir_safe_atom("valid!"), "valid!");
    }

    #[test]
    fn test_elixir_safe_atom_with_special_chars() {
        // Atoms with colons must be quoted (without leading :, template adds it)
        assert_eq!(elixir_safe_atom("og:image"), r#""og:image""#);
        assert_eq!(elixir_safe_atom("twitter:image"), r#""twitter:image""#);
        // Atoms with dashes must be quoted
        assert_eq!(elixir_safe_atom("some-value"), r#""some-value""#);
    }

    #[test]
    fn test_gen_elixir_enum_module_with_serde_rename_special_chars() {
        // Create ImageSource enum with serde_rename containing colons
        let image_source_enum = EnumDef {
            name: "ImageSource".to_string(),
            rust_path: "my_crate::ImageSource".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Img".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "OgImage".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("og:image".to_string()),
                },
                EnumVariant {
                    name: "TwitterImage".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("twitter:image".to_string()),
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let result = gen_elixir_enum_module(&image_source_enum, "Kreuzcrawl");

        // @type should contain quoted atoms for special chars
        assert!(
            result.contains(":img | :\"og:image\" | :\"twitter:image\""),
            "should emit quoted atoms in @type for serde_rename with colons; got:\n{result}"
        );

        // Attributes should use snake_case identifiers, not the serde_rename value
        assert!(
            result.contains("@og_image "),
            "should use @og_image attribute name (from variant OgImage), not @og:image; got:\n{result}"
        );
        assert!(
            result.contains("@twitter_image "),
            "should use @twitter_image attribute name (from variant TwitterImage), not @twitter:image; got:\n{result}"
        );

        // Accessors (functions) should also use snake_case names
        assert!(
            result.contains("def og_image, do: @og_image"),
            "should emit def og_image() function name, not def og:image(); got:\n{result}"
        );
        assert!(
            result.contains("def twitter_image, do: @twitter_image"),
            "should emit def twitter_image() function name, not def twitter:image(); got:\n{result}"
        );

        // Ensure the attribute values are properly quoted atoms
        assert!(
            result.contains(r#"@og_image :"og:image""#),
            "should emit @og_image with quoted atom value; got:\n{result}"
        );
        assert!(
            result.contains(r#"@twitter_image :"twitter:image""#),
            "should emit @twitter_image with quoted atom value; got:\n{result}"
        );
    }

    #[test]
    fn test_gen_elixir_enum_module_resolves_known_payload_types() {
        // Create FormatMetadata enum with both known and unknown payload types
        let format_enum = EnumDef {
            name: "FormatMetadata".to_string(),
            rust_path: "my_crate::FormatMetadata".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pdf".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Named("PdfMetadata".into()),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Other".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Named("UnknownType".into()),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        // Simulate calling with known types available
        let mut known_types = AHashSet::new();
        known_types.insert("PdfMetadata".to_string());

        let result = gen_elixir_enum_module_with_known_types(&format_enum, "Kreuzberg", &known_types);

        // Known type should resolve to module.t()
        assert!(
            result.contains("Kreuzberg.PdfMetadata.t()"),
            "should resolve PdfMetadata to Kreuzberg.PdfMetadata.t(); got:\n{result}"
        );

        // Unknown type should fall back to map()
        assert!(
            result.contains("value: map()"),
            "should fall back to map() for unknown type; got:\n{result}"
        );
    }
}
