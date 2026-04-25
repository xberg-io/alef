use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{FieldDef, TypeDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::HashMap;

/// Get module name and prefix from config or derive from crate name.
pub(super) fn get_module_info(_api: &alef_core::ir::ApiSurface, config: &AlefConfig) -> (String, String) {
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
pub(super) fn map_return_type(
    ty: &TypeRef,
    mapper: &crate::type_map::RustlerMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(n) if opaque_types.contains(n) => format!("ResourceArc<{n}>"),
        _ => mapper.map_type(ty),
    }
}

/// Generate the `{AppModule}.Native` Elixir module with NIF stubs for all functions and methods.
pub(super) fn gen_native_ex(
    api: &alef_core::ir::ApiSurface,
    app_name: &str,
    app_module: &str,
    _crate_name: &str,
    config: &AlefConfig,
    exclude_functions: &AHashSet<&str>,
    exclude_types: &AHashSet<&str>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(1024);

    let repo_url = config.github_repo();
    // The env var that forces a local source build: {APP_NAME_UPPER}_BUILD
    let build_env_var = format!("{}_BUILD", app_name.to_uppercase());

    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "defmodule {app_module}.Native do");
    let _ = writeln!(out, "  @moduledoc false");
    let _ = writeln!(out);
    let _ = writeln!(out, "  use RustlerPrecompiled,");
    let _ = writeln!(out, "    otp_app: :{app_name},");
    let _ = writeln!(out, "    crate: \"{app_name}_nif\",");
    let _ = writeln!(out, "    base_url:");
    let _ = writeln!(
        out,
        "      \"{repo_url}/releases/download/v#{{Mix.Project.config()[:version]}}\","
    );
    let _ = writeln!(out, "    version: Mix.Project.config()[:version],");
    let _ = writeln!(
        out,
        "    force_build: System.get_env(\"{build_env_var}\") in [\"1\", \"true\"] or Mix.env() in [:test, :dev],"
    );
    let _ = writeln!(out, "    targets:");
    let _ = writeln!(
        out,
        "      ~w(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu),"
    );
    let _ = writeln!(out, "    nif_versions: [\"2.16\", \"2.17\"]");
    let _ = writeln!(out);

    // Stubs for top-level API functions
    let mut last_was_multiline = true;
    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let fn_name = if func.is_async {
            format!("{}_async", func.name)
        } else {
            func.name.clone()
        };
        let underscored_params: Vec<String> = func
            .params
            .iter()
            .map(|p| format!("_{}", p.name.to_snake_case()))
            .collect();
        last_was_multiline = write_nif_stub(&mut out, &fn_name, &underscored_params, last_was_multiline);

        // For functions that have a visitor bridge, also emit the async visitor variant stub
        // plus the visitor_reply NIF stub (once, for the first such function).
        let has_visitor_bridge = config.trait_bridges.iter().any(|b| {
            func.params.iter().any(|p| {
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
    }

    // visitor_reply stub: emitted once when there are visitor bridges.
    if !config.trait_bridges.is_empty() {
        last_was_multiline = write_nif_stub(
            &mut out,
            "visitor_reply",
            &["_ref_id".to_string(), "_result".to_string()],
            last_was_multiline,
        );
    }

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

    let _ = writeln!(out, "end");
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
    use std::fmt::Write;
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
        if !prev_was_multiline {
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "  def {sig},");
        let _ = writeln!(out, "    do: :erlang.nif_error(:nif_not_loaded)");
        let _ = writeln!(out);
        true
    } else {
        let _ = writeln!(out, "  def {sig}, do: :erlang.nif_error(:nif_not_loaded)");
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
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "defmodule {app_module}.{} do", typ.name);

    if !typ.doc.is_empty() {
        let doc_first = typ.doc.lines().next().unwrap_or("").replace('"', "\\\"");
        let _ = writeln!(out, "  @moduledoc \"{doc_first}\"");
    } else {
        let _ = writeln!(out, "  @moduledoc false");
    }
    let _ = writeln!(out);

    // defstruct with defaults - use bare keyword list style (mix format compliant)
    let fields: Vec<_> = typ.fields.iter().collect();
    if fields.is_empty() {
        let _ = writeln!(out, "  defstruct []");
    } else {
        let _ = write!(out, "  defstruct ");
        for (i, field) in fields.iter().enumerate() {
            let default = elixir_field_default(field, &field.ty, enum_defaults, opaque_types);
            let name = field.name.to_snake_case();
            if i == 0 {
                let _ = write!(out, "{name}: {default}");
            } else {
                let _ = write!(out, ",\n            {name}: {default}");
            }
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "end");
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

/// Generate a `defmodule {AppModule}.{EnumName}` file for an enum.
///
/// Simple enums (all variants have no fields) get a `@type t :: :variant1 | :variant2 | ...`
/// union type using snake_case atoms, mirroring the Rustler `NifUnitEnum` atom encoding.
///
/// Data enums (one or more variants have fields) get a module with per-variant type aliases
/// since Elixir has no single structural type for tagged union variants.
pub(super) fn gen_elixir_enum_module(enum_def: &alef_core::ir::EnumDef, app_module: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(256);

    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "defmodule {app_module}.{} do", enum_def.name);

    if !enum_def.doc.is_empty() {
        let doc_first = enum_def.doc.lines().next().unwrap_or("").replace('"', "\\\"");
        let _ = writeln!(out, "  @moduledoc \"{doc_first}\"");
    } else {
        let _ = writeln!(out, "  @moduledoc false");
    }
    let _ = writeln!(out);

    let is_simple = enum_def.variants.iter().all(|v| v.fields.is_empty());

    if is_simple {
        // @type t :: :variant_one | :variant_two | ...
        // Rustler NifUnitEnum encodes variants as atoms using the variant name as-is,
        // but Elixir convention for atoms uses snake_case.
        let atom_arms: Vec<String> = enum_def
            .variants
            .iter()
            .map(|v| format!(":{}", v.name.to_snake_case()))
            .collect();
        // Emit multi-line @type when the single-line form exceeds 120 chars
        let single_line = format!("  @type t :: {}", atom_arms.join(" | "));
        if single_line.len() <= 120 {
            let _ = writeln!(out, "{single_line}");
        } else {
            let _ = writeln!(out, "  @type t ::");
            for (i, arm) in atom_arms.iter().enumerate() {
                if i == 0 {
                    let _ = writeln!(out, "          {arm}");
                } else {
                    let _ = writeln!(out, "          | {arm}");
                }
            }
        }
        let _ = writeln!(out);

        // Module attributes for each variant value — convenient aliases
        for variant in &enum_def.variants {
            let attr_name = variant.name.to_snake_case();
            let _ = writeln!(out, "  @{attr_name} :{attr_name}");
        }
        let _ = writeln!(out);
        // Export the values so callers can reference MyEnum.variant_name/0
        for variant in &enum_def.variants {
            let attr_name = variant.name.to_snake_case();
            let _ = writeln!(out, "  @spec {attr_name}() :: t()");
            let _ = writeln!(out, "  def {attr_name}, do: @{attr_name}");
        }
    } else {
        // Data enum: provide a @type t :: term() and per-variant type aliases
        let _ = writeln!(out, "  @type t :: term()");
        let _ = writeln!(out);
        for variant in &enum_def.variants {
            let variant_atom = format!(":{}", variant.name.to_snake_case());
            let type_name = elixir_safe_type_name(&variant.name.to_snake_case());
            if variant.fields.is_empty() {
                // Unit variant: just an atom
                let _ = writeln!(out, "  @type {type_name} :: {variant_atom}");
            } else {
                // Struct variant: a map with a type tag
                let field_types: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        let name = f.name.to_snake_case();
                        let safe_name = if name.starts_with(|c: char| c.is_ascii_digit()) {
                            format!("value_{name}")
                        } else {
                            name
                        };
                        format!("{safe_name}: term()")
                    })
                    .collect();
                let _ = writeln!(
                    out,
                    "  @type {type_name} :: %{{type: {variant_atom}, {}}}",
                    field_types.join(", ")
                );
            }
        }
    }

    let _ = writeln!(out, "end");
    out
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

    // No typed_default: use optional flag or type-appropriate zero
    if field.optional {
        return "nil".to_string();
    }
    elixir_zero_value(ty, enum_defaults)
}

/// Generate a type-appropriate zero/default value for Elixir.
fn elixir_zero_value(ty: &TypeRef, enum_defaults: &HashMap<String, String>) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "false".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"".to_string(),
        TypeRef::Bytes => "\"\"".to_string(),
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
