use crate::backends::rustler::template_env;
use crate::codegen::doc_emission::doc_first_paragraph_joined;
use crate::codegen::shared::binding_fields;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::HashMap;

use super::context::emit_elixir_doc_attr;
use super::json_values::{
    elixir_field_default, elixir_field_name_with_type, elixir_safe_atom, elixir_safe_attr_name, elixir_safe_param_name,
    elixir_safe_type_name, elixir_struct_field_typespec, elixir_typespec,
};

/// Generate a `defmodule {AppModule}.{TypeName}` file with a `defstruct` for a non-opaque type.
pub(in crate::backends::rustler::gen_bindings) fn gen_elixir_struct_module(
    typ: &TypeDef,
    app_module: &str,
    enum_defaults: &HashMap<String, String>,
    opaque_types: &AHashSet<String>,
    known_struct_types: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&hash::header(CommentStyle::Hash));

    let ctx = minijinja::context! {
        app_module => app_module,
        type_name => &typ.name,
    };
    out.push_str(&template_env::render("struct_module_header.jinja", ctx));
    if !typ.doc.is_empty() {
        emit_elixir_doc_attr(&mut out, "moduledoc", &typ.doc, "  ");
    } else {
        out.push_str("  @moduledoc false\n");
    }
    out.push('\n');

    // Emit @typedoc and @type t typespec before defstruct.
    let default_types: AHashSet<String> = enum_defaults.keys().cloned().collect();
    if !typ.doc.is_empty() {
        let first_para = doc_first_paragraph_joined(&typ.doc);
        emit_elixir_doc_attr(&mut out, "typedoc", &first_para, "  ");
    }
    out.push_str("  @type t :: %__MODULE__{\n");

    let fields: Vec<_> = binding_fields(&typ.fields).collect();
    if !fields.is_empty() {
        for (i, field) in fields.iter().enumerate() {
            let field_name = field.name.to_snake_case();
            let field_type =
                elixir_struct_field_typespec(&field.ty, app_module, opaque_types, &default_types, known_struct_types);
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

            // mix format aligns struct fields to the column of the opening `{` in
            // `@type t :: %__MODULE__{`, which is at column 24 (10-space indent).
            out.push_str(&template_env::render(
                "elixir_struct_type_field.ex.jinja",
                minijinja::context! {
                    field_name => &field_name,
                    field_type => &field_type_with_optional,
                    is_last => i == fields.len() - 1,
                },
            ));
        }
    }
    // Closing brace aligned to the column of the field indent (8 spaces) —
    // mix format hoists the `}` to this column when it wraps the typespec.
    out.push_str("        }\n\n");

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

    // Add Jason.Encoder implementation for option structs (has_default = true).
    // This allows Elixir code to pass structs to NIF functions that expect JSON-encoded options.
    // Filter out nil values to avoid serde deserialization issues with nested structs.
    if typ.has_default {
        out.push('\n');
        out.push_str("  defimpl Jason.Encoder do\n");
        out.push_str("    @doc false\n");
        out.push_str("    def encode(value, opts) do\n");
        out.push_str("      value\n");
        out.push_str("      |> Map.from_struct()\n");
        out.push_str("      |> Enum.reject(fn {_k, v} -> v == nil end)\n");
        out.push_str("      |> Enum.into(%{})\n");
        out.push_str("      |> Jason.Encoder.encode(opts)\n");
        out.push_str("    end\n");
        out.push_str("  end\n");
    }

    // Add valid?/1 instance method for HeaderMetadata-like types with is_valid in Rust.
    if typ.name == "HeaderMetadata" {
        out.push('\n');
        out.push_str("  @doc \"Validate that the header level is within valid range (1-6).\"\n");
        out.push_str("  @spec valid?(t()) :: boolean()\n");
        out.push_str("  def valid?(%__MODULE__{level: level}) do\n");
        out.push_str("    level >= 1 and level <= 6\n");
        out.push_str("  end\n");
    }

    // `mix format` rejects a blank between the last block's `end` and the
    // module's closing `end`; trim trailing blanks before emitting the footer.
    while out.ends_with("\n\n") {
        out.pop();
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
/// (`%SampleLanguagePack.Parser{ref: ...}`) and exposes the type's
/// methods as functions that delegate to the corresponding NIF
/// (`{type_lower}_{method_name}`) provided by `{AppModule}.Native`.
///
/// Async methods delegate to the `_async` NIF variant (see
/// `gen_bindings/functions.rs`). Methods that map to a `Streaming` adapter
/// emit a `Stream.unfold/2`-based wrapper that drives the underlying
/// `_start`/`_next` NIF pair instead of attempting a sync call.
pub(in crate::backends::rustler::gen_bindings) fn gen_elixir_opaque_module(
    typ: &TypeDef,
    app_module: &str,
    config: &ResolvedCrateConfig,
) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&hash::header(CommentStyle::Hash));

    let ctx = minijinja::context! {
        app_module => app_module,
        type_name => &typ.name,
    };
    out.push_str(&template_env::render("struct_module_header.jinja", ctx));
    if !typ.doc.is_empty() {
        emit_elixir_doc_attr(&mut out, "moduledoc", &typ.doc, "  ");
    } else {
        out.push_str("  @moduledoc false\n");
    }
    out.push('\n');

    // Native alias and reference-only struct. Only emit the alias when the
    // body actually references Native.foo — otherwise mix compile
    // --warnings-as-errors flags an unused-alias warning. The body uses
    // Native for the default-constructor body, for any method wrapper, or for
    // the variant-wrapper static `new` constructor (emitted by the general
    // method loop below — no special-case is needed because the IR already
    // places the static `new` in `typ.methods`).
    let needs_native_alias = typ.has_default || !typ.methods.is_empty() || typ.is_variant_wrapper;
    if needs_native_alias {
        out.push_str(&template_env::render(
            "elixir_native_alias.ex.jinja",
            minijinja::context! {
                app_module => app_module,
            },
        ));
    }
    out.push_str("  defstruct [:ref]\n\n");
    if !typ.doc.is_empty() {
        let first_para = doc_first_paragraph_joined(&typ.doc);
        emit_elixir_doc_attr(&mut out, "typedoc", &first_para, "  ");
    }
    out.push_str("  @type t :: %__MODULE__{ref: reference()}\n\n");

    let type_lower = typ.name.to_lowercase();

    // Streaming-adapter method names owned by this type. Sync calls would fail
    // (the NIFs are `{name}_start`/`{name}_next`); emit a Stream wrapper instead.
    let streaming_method_names: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter(|a| a.owner_type.as_deref() == Some(typ.name.as_str()))
        .map(|a| a.name.clone())
        .collect();

    // Constructor for types with a default — wraps the native default reference.
    if typ.has_default {
        out.push_str(&template_env::render(
            "elixir_opaque_new.ex.jinja",
            minijinja::context! {
                type_lower => &type_lower,
            },
        ));
    }

    // Wrapper for each method. Methods with a receiver take the struct as the
    // first argument and pass `obj.ref` to the NIF. Static methods (no receiver)
    // are emitted as module-level functions.
    for method in &typ.methods {
        let method_name = method.name.to_snake_case();

        // Skip emitting `new/0` wrapper if type has a default, since we already
        // emitted `def new/0` above (lines 765–771). If the Rust type has both
        // `impl Default` and `pub fn new()`, we only want one Elixir `def new/0`.
        if typ.has_default && method.name == "new" && method.receiver.is_none() {
            // Instead of skipping silently, emit the `default/0` function if the
            // method is the `default()` constructor (uncommon, but defensively handle it).
            continue;
        }

        // Similarly, skip emitting `default/0` wrapper from Rust methods if we already
        // have a `has_default` block above. We'll emit it as a separate function below.
        if typ.has_default && method.name == "default" && method.receiver.is_none() {
            continue;
        }

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
            out.push_str(&template_env::render(
                "elixir_opaque_stream_method.ex.jinja",
                minijinja::context! {
                    doc_first => &doc_first,
                    method_name => &method_name,
                    def_args => &def_args.join(", "),
                    start_fn => &start_fn,
                    start_call_args => &start_call_args.join(", "),
                    next_fn => &next_fn,
                },
            ));
            // Template ends with newline; add blank line for mix format compatibility
            out.push('\n');
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

        // For static methods (no receiver) on opaque types, wrap the return value
        // in the struct if the return type matches the module's type.
        let is_static = method.receiver.is_none();
        let returns_self = matches!(&method.return_type, TypeRef::Named(n) if n == &typ.name);

        // Ensure blank line before @doc (mix format requirement between defs)
        if !doc_first.is_empty() && !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }

        out.push_str(&template_env::render(
            "elixir_opaque_method_wrapper.ex.jinja",
            minijinja::context! {
                doc_first => &doc_first,
                method_name => &method_name,
                def_args => &def_args.join(", "),
                returns_self => is_static && returns_self,
                nif_fn => &nif_fn,
                call_args => &call_args.join(", "),
            },
        ));
        // Template ends with newline; add blank line for mix format compatibility
        out.push('\n');
    }

    // Emit a separate `default/0` function if the type has a default.
    // This wraps the `{type_lower}_default()` NIF and is distinct from `new/0`.
    if typ.has_default {
        out.push_str(&template_env::render(
            "elixir_opaque_default.ex.jinja",
            minijinja::context! {
                type_lower => &type_lower,
            },
        ));
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

/// Generate a `defmodule {AppModule}.{EnumName}` file for an enum.
///
/// Simple enums (all variants have no fields) get a `@type t :: :variant1 | :variant2 | ...`
/// union type using snake_case atoms, mirroring the Rustler `NifUnitEnum` atom encoding.
///
/// Data enums (one or more variants have fields) get a module with per-variant type aliases
/// since Elixir has no single structural type for tagged union variants.
#[allow(dead_code)]
pub(in crate::backends::rustler::gen_bindings) fn gen_elixir_enum_module(
    enum_def: &crate::core::ir::EnumDef,
    app_module: &str,
) -> String {
    gen_elixir_enum_module_with_known_types(enum_def, app_module, &AHashSet::new())
}

pub(in crate::backends::rustler::gen_bindings) fn gen_elixir_enum_module_with_known_types(
    enum_def: &crate::core::ir::EnumDef,
    app_module: &str,
    known_types: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(256);

    out.push_str(&hash::header(CommentStyle::Hash));

    let ctx = minijinja::context! {
        app_module => app_module,
        enum_name => &enum_def.name,
    };
    out.push_str(&template_env::render("enum_module_header.jinja", ctx));
    if !enum_def.doc.is_empty() {
        emit_elixir_doc_attr(&mut out, "moduledoc", &enum_def.doc, "  ");
    } else {
        out.push_str("  @moduledoc false\n");
    }
    out.push('\n');

    let is_simple = enum_def.variants.iter().all(|v| v.fields.is_empty());

    if is_simple {
        // @type t :: :variant_one | :variant_two | ...
        // Rustler NifUnitEnum encodes variants as atoms using the variant name as-is,
        // but Elixir convention for atoms uses snake_case.
        // Guard against Elixir reserved words (end, fn, do, etc.).
        let atom_arms: Vec<String> = enum_def
            .variants
            .iter()
            .map(|v| {
                let atom_value = match v.serde_rename.as_deref() {
                    Some(rename) => rename.to_owned(),
                    None => {
                        let snake_name = crate::codegen::naming::pascal_to_snake(&v.name);
                        elixir_safe_param_name(&snake_name)
                    }
                };
                format!(":{}", elixir_safe_atom(&atom_value))
            })
            .collect();
        if !enum_def.doc.is_empty() {
            let first_para = doc_first_paragraph_joined(&enum_def.doc);
            emit_elixir_doc_attr(&mut out, "typedoc", &first_para, "  ");
        }
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
            // Guard against Elixir reserved words (end, fn, do, etc.) and module attributes.
            let snake_name = crate::codegen::naming::pascal_to_snake(&variant.name);
            let safe_name = elixir_safe_param_name(&snake_name);
            let attr_name = elixir_safe_attr_name(&safe_name);
            // But the atom value should use serde_rename if available, properly quoted if needed.
            let atom_value = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| crate::codegen::naming::pascal_to_snake(&variant.name));
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
            // Guard against Elixir reserved words (end, fn, do, etc.) and module attributes.
            let snake_name = crate::codegen::naming::pascal_to_snake(&variant.name);
            let safe_name = elixir_safe_param_name(&snake_name);
            let attr_name = elixir_safe_attr_name(&safe_name);
            if !variant.doc.is_empty() {
                let first_para = doc_first_paragraph_joined(&variant.doc);
                emit_elixir_doc_attr(&mut out, "doc", &first_para, "  ");
            }
            out.push_str(&template_env::render(
                "elixir_enum_accessor.jinja",
                minijinja::context! {
                    atom_name => &safe_name,
                    attr_name => &attr_name,
                },
            ));
        }
    } else {
        // Data enum: provide a @type t :: term() and per-variant type aliases
        if !enum_def.doc.is_empty() {
            let first_para = doc_first_paragraph_joined(&enum_def.doc);
            emit_elixir_doc_attr(&mut out, "typedoc", &first_para, "  ");
        }
        out.push_str("  @type t :: term()\n");
        out.push('\n');
        for variant in &enum_def.variants {
            let snake_name = crate::codegen::naming::pascal_to_snake(&variant.name);
            // The atom VALUE in `%{type: :atom}` must match the runtime atom the NifTaggedEnum
            // decoder and the generated constructor use — both derive it from `snake_name` via
            // `elixir_safe_atom` (e.g. `End` → `:end`, a valid atom). The `@type` LHS name, by
            // contrast, is an identifier in type position where reserved words are illegal
            // (`@type end ::` won't compile), so it keeps the reserved-word-guarded form.
            let variant_atom = format!(":{}", elixir_safe_atom(&snake_name));
            let type_name = elixir_safe_type_name(&elixir_safe_param_name(&snake_name));
            if !variant.doc.is_empty() {
                let first_para = doc_first_paragraph_joined(&variant.doc);
                emit_elixir_doc_attr(&mut out, "typedoc", &first_para, "  ");
            }
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
                                crate::core::ir::PrimitiveType::Bool => Some("bool"),
                                crate::core::ir::PrimitiveType::U8 => Some("u8"),
                                crate::core::ir::PrimitiveType::U16 => Some("u16"),
                                crate::core::ir::PrimitiveType::U32 => Some("u32"),
                                crate::core::ir::PrimitiveType::U64 => Some("u64"),
                                crate::core::ir::PrimitiveType::Usize => Some("usize"),
                                crate::core::ir::PrimitiveType::I8 => Some("i8"),
                                crate::core::ir::PrimitiveType::I16 => Some("i16"),
                                crate::core::ir::PrimitiveType::I32 => Some("i32"),
                                crate::core::ir::PrimitiveType::I64 => Some("i64"),
                                crate::core::ir::PrimitiveType::Isize => Some("isize"),
                                crate::core::ir::PrimitiveType::F32 => Some("f32"),
                                crate::core::ir::PrimitiveType::F64 => Some("f64"),
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

        // Per-variant constructors: `def <snake>(<params>), do: {:<atom>, %{<field>: <param>, ...}}`.
        // Each builds the `{:variant, %{field: value}}` tagged-tuple form that the NifTaggedEnum
        // decoder consumes — the plain-direct model (no NIF, no core conversion; the binding enum is
        // already binding-shaped). Variant selection (skip unit/tuple/`binding_excluded`, yield to a
        // hand-written `impl` method of the same name) is shared with the pyo3/magnus/php/extendr
        // paths via `collect_variant_constructors`.
        let constructors = crate::codegen::generators::collect_variant_constructors(enum_def);
        if !constructors.is_empty() {
            out.push('\n');
            for ctor in &constructors {
                let atom = elixir_safe_atom(&ctor.snake_name);
                let fn_name = elixir_safe_param_name(&ctor.snake_name);
                // Pair each Elixir param name with the map entry `field: param`. The map key is the
                // Rust field name (an atom); the encoder reduces over it (renaming where needed) so
                // the wire shape matches what serde expects.
                let params: Vec<String> = ctor.params.iter().map(|p| elixir_safe_param_name(&p.name)).collect();
                let map_entries: Vec<String> = ctor
                    .params
                    .iter()
                    .zip(&params)
                    .map(|(p, param_name)| format!("{}: {param_name}", p.name))
                    .collect();
                out.push_str(&template_env::render(
                    "elixir_enum_variant_constructor.jinja",
                    minijinja::context! {
                        fn_name => &fn_name,
                        params => params.join(", "),
                        atom => &atom,
                        map_entries => map_entries.join(", "),
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
