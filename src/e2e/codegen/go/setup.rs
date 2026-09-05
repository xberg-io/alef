//! Go argument and setup rendering.

use crate::e2e::escape::go_string_literal;

use super::json_values::{json_to_go, json_to_go_yields_string_literal};

mod args;

fn json_object_go_type<'a>(arg: &'a crate::e2e::config::ArgMapping, options_type: Option<&'a str>) -> Option<&'a str> {
    arg.go_type.as_deref().or(arg.element_type.as_deref()).or(options_type)
}

/// Qualify `type_name` with `import_alias`, applying the same Go acronym uppercasing the real
/// Go backend applies when it emits the type's declaration
/// (`backends::go::gen_bindings::types::structs::gen_struct_type`,
/// `backends::go::gen_bindings::types::enums`).
///
/// `type_name` here is an IR name straight off the Rust source (e.g. `JsonSchemaFormat`), not
/// a Go identifier — every real emitter call site runs it through
/// [`crate::codegen::naming::go_type_name`] before writing it out, and this snippet-literal path
/// must resolve to the identical identifier or the generated Go does not compile.
/// `go_type_name` is idempotent on a name that is already correctly cased (e.g. an explicit
/// `arg.go_type` config override), so applying it unconditionally is safe. ~keep
fn qualified_go_type(import_alias: &str, type_name: &str) -> String {
    if type_name.contains('.') {
        type_name.to_string()
    } else {
        format!("{import_alias}.{}", crate::codegen::naming::go_type_name(type_name))
    }
}

/// The Go shape the binding backend emits for `type_name`, when the name is an IR enum.
///
/// `None` covers both "not an enum" and "not in the IR at all"; neither licenses any claim
/// about the emitted declaration, so every caller keeps its pre-existing rendering. ~keep
fn go_enum_shape(
    enums: &[crate::core::ir::EnumDef],
    type_name: &str,
) -> Option<crate::backends::go::GoEnumRepresentation> {
    enums
        .iter()
        .find(|candidate| candidate.name == type_name)
        .map(crate::backends::go::go_enum_representation)
}

/// The zero-valued Go expression for an argument of `type_name` whose fixture supplies no value.
///
/// The composite literal `pkg.T{}` is only legal when the binding emits `T` as a struct (or a
/// slice, which `type T json.RawMessage` is). Against a sealed `interface` it is `invalid
/// composite literal type`, and against `type T string` it is the same error — both of which the
/// unconditional `pkg.T{}` used to emit. Each arm below is the zero value of the declaration
/// `backends::go::gen_bindings::types::enums::gen_enum_type` actually writes. ~keep
fn go_empty_value_expression(import_alias: &str, type_name: &str, enums: &[crate::core::ir::EnumDef]) -> String {
    match go_enum_shape(enums, type_name) {
        Some(crate::backends::go::GoEnumRepresentation::DataInterface) => "nil".to_string(),
        Some(
            crate::backends::go::GoEnumRepresentation::UnitString
            | crate::backends::go::GoEnumRepresentation::NewtypeTupleString,
        ) => "\"\"".to_string(),
        _ => format!("{}{{}}", qualified_go_type(import_alias, type_name)),
    }
}

/// The `ptr[T any]` / `mustReadFile` package-level helpers a rendered literal depends on.
///
/// A literal can reach `package_decls` from more than one argument arm, and a helper emitted
/// twice is a duplicate declaration in the generated file — hence the membership check rather
/// than an unconditional push. ~keep
fn ensure_value_helpers(package_decls: &mut Vec<String>, literal: &str) {
    if literal.contains("ptr(")
        && !package_decls
            .iter()
            .any(|declaration| declaration.starts_with("func ptr["))
    {
        package_decls.push("func ptr[T any](value T) *T { return &value }".to_string());
    }
    if literal.contains("mustReadFile(")
        && !package_decls
            .iter()
            .any(|declaration| declaration.starts_with("func mustReadFile("))
    {
        package_decls.push(
            crate::e2e::template_env::render("go/read_file_helper.jinja", minijinja::context! {})
                .trim_end()
                .to_string(),
        );
    }
}

/// How much of an offending fixture value [`named_field_type_mismatch`] quotes back.
///
/// The value is named so an operator can find the fixture entry that produced it, but a
/// fixture `input` can be arbitrarily large and a diagnostic is not a place to reprint it. ~keep
const MAX_DIAGNOSTIC_VALUE_CHARS: usize = 80;

/// Refuse a fixture value that has no expression of the target field's declared Go type.
///
/// Names the field, the Go type the binding declares for it, the Go declaration kind that
/// makes the value unusable, and the value itself — everything an operator needs to decide
/// between fixing the fixture and recording a `docs.coverage_exceptions` entry. ~keep
///
/// The compile error this prevents: Go converts an untyped string constant only to a defined
/// type whose underlying type is `string` or `[]byte`, so `{go_type}(<value>)` against any other
/// representation is `cannot convert`. Alef refuses rather than publishing a snippet that does
/// not compile. ~keep
fn named_field_type_mismatch(
    owner_type: &str,
    field_name: &str,
    go_type: &str,
    representation: crate::backends::go::GoEnumRepresentation,
    rendered: &str,
) -> anyhow::Error {
    let quoted: String = rendered.chars().take(MAX_DIAGNOSTIC_VALUE_CHARS).collect();
    let elided = if quoted.len() < rendered.len() { "..." } else { "" };
    let declaration = representation.go_declaration();
    anyhow::anyhow!(
        "e2e go codegen: field `{field_name}` of `{owner_type}` is the IR enum `{go_type}`, which Go emits as \
         `type {go_type} {declaration}`; the fixture value lowers to {quoted}{elided}, and \
         `{go_type}({quoted}{elided})` is a `cannot convert` compile error. Give the fixture a value matching one \
         of `{go_type}`'s variants, or record a `docs.coverage_exceptions` entry for go."
    )
}

/// The lowering environment a native Go DTO literal is built in.
///
/// The four registries travel together through every level of the recursion — a struct
/// field, an enum payload, a sealed-interface variant's own named fields — and threading
/// them individually pushes each recursive entry point past the argument limit. ~keep
#[derive(Clone, Copy)]
pub(super) struct GoValueContext<'a> {
    pub import_alias: &'a str,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
    pub files: &'a [crate::e2e::fixture::FixtureDocsFileInput],
}

/// Where in the fixture a value is being lowered, and into what.
///
/// `owner_type` and `field_name` are what a refusal names, so they must describe the field
/// that actually failed rather than the outermost DTO an operator would then have to search. ~keep
#[derive(Clone, Copy)]
pub(super) struct GoFieldSite<'a> {
    pub owner_type: &'a str,
    pub field_name: &'a str,
    /// This field's `docs.files` pointer, e.g. `/document/content`.
    pub pointer: &'a str,
}

/// Lower a scalar fixture value into an expression of the Go type declared for `type_name`.
///
/// This is the one point where the value's JSON shape and the *declared type it lands in* are
/// both in hand. Without the enum lookup below, every unresolved `TypeRef::Named` was rendered
/// as the blind conversion `alias.Type(<value>)`, which happens to compile for the enums Go
/// emits as `type X string` / `type X json.RawMessage` and cannot compile for the ones it emits
/// as a struct or a sealed interface. A name that resolves to no IR enum is left exactly as it
/// was: nothing here can prove what such a type is, and a false refusal deletes published
/// documentation. ~keep
///
/// The order of the three answers is deliberate. A named constant and the string conversion
/// come first so the representations that already compile keep their exact rendering; only
/// then does [`super::enum_literals::go_enum_value_expression`] try to *construct* a value of a
/// struct- or interface-shaped enum, and only a value that identifies no variant reaches the
/// refusal. ~keep
fn go_named_scalar_expression(
    value: &serde_json::Value,
    type_name: &str,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<String> {
    let go_type = crate::codegen::naming::go_type_name(type_name);
    let conversion = format!("{}.{go_type}({})", context.import_alias, json_to_go(value));
    let Some(enum_def) = context.enums.iter().find(|candidate| candidate.name == type_name) else {
        return Ok(conversion);
    };
    if let Some(wire_value) = value.as_str()
        && let Some(constant) = crate::backends::go::go_enum_constant_for_wire_value(enum_def, wire_value)
    {
        return Ok(format!("{}.{constant}", context.import_alias));
    }
    let representation = crate::backends::go::go_enum_representation(enum_def);
    // A value that names no variant is still emitted as the conversion when the binding
    // declares a convertible underlying type — `type X string` accepts any string, which is
    // what validation fixtures asserting on a rejected value depend on. ~keep
    if representation.accepts_string_conversion() && json_to_go_yields_string_literal(value) {
        return Ok(conversion);
    }
    if let Some(constructed) = super::enum_literals::go_enum_value_expression(value, enum_def, context, site)? {
        return Ok(constructed);
    }
    Err(named_field_type_mismatch(
        site.owner_type,
        site.field_name,
        &go_type,
        representation,
        &json_to_go(value),
    ))
}

/// Lower a value into an expression of the Go type `type_name`, as a field of that type takes
/// it: a composite literal when the value is a DTO object, otherwise the scalar lowering, each
/// address-taken when the emitted field is a pointer.
///
/// A composite literal is addressable with `&`; a constant, a conversion and a bare literal are
/// not, which is why the second arm goes through the generic `ptr[T any]` helper instead. ~keep
pub(super) fn go_named_field_expression(
    value: &serde_json::Value,
    type_name: &str,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
    uses_pointer: bool,
) -> anyhow::Result<String> {
    if let Some(nested) = native_go_dto_literal_at(value, type_name, context, site.pointer)? {
        return Ok(if uses_pointer { format!("&{nested}") } else { nested });
    }
    let literal = go_named_scalar_expression(value, type_name, context, site)?;
    Ok(if uses_pointer {
        format!("ptr({literal})")
    } else {
        literal
    })
}

/// `ptr[T any](value T) *T` infers `T` from the argument expression. A bare numeric literal
/// (e.g. `30`) defaults to Go's untyped-constant rule (`int` for integers, `float64` for
/// floats), which only matches the field's actual pointer type by coincidence —
/// `*uint`/`*uint64`/`*int32`/etc. fields need an explicit conversion so `ptr(...)` infers the
/// field's real width instead. `bool` has exactly one Go type, so it never needs this. ~keep
fn go_primitive_field_expression(
    primitive: &crate::core::ir::PrimitiveType,
    inner: &crate::core::ir::TypeRef,
    literal: String,
    uses_pointer: bool,
) -> String {
    if !uses_pointer {
        return literal;
    }
    if matches!(primitive, crate::core::ir::PrimitiveType::Bool) {
        format!("ptr({literal})")
    } else {
        let go_primitive_type = crate::backends::go::type_map::go_struct_field_type(inner);
        format!("ptr({go_primitive_type}({literal}))")
    }
}

/// Lower one IR field's fixture value into the expression its emitted Go field takes.
///
/// `Ok(None)` means the value's JSON shape has no expression at this field's type and the
/// field is omitted from the literal (Go's zero value stands in); `Err` means the value has no
/// valid expression at all and no snippet may be published. Shared by the struct-literal
/// builder and by the sealed-interface variant builder, which fills fields declared exactly
/// the same way. ~keep
pub(super) fn go_struct_field_expression(
    field: &crate::core::ir::FieldDef,
    value: &serde_json::Value,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
    uses_pointer: bool,
) -> anyhow::Result<Option<String>> {
    let inner = match &field.ty {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    if context.files.iter().any(|file| file.field == site.pointer) && matches!(inner, crate::core::ir::TypeRef::Bytes) {
        let Some(path) = value.as_str() else {
            return Ok(None);
        };
        return Ok(Some(format!("mustReadFile({})", go_string_literal(path))));
    }
    let expression = match inner {
        crate::core::ir::TypeRef::String | crate::core::ir::TypeRef::Path => {
            let Some(literal) = value.as_str().map(go_string_literal) else {
                return Ok(None);
            };
            if uses_pointer {
                format!("ptr({literal})")
            } else {
                literal
            }
        }
        crate::core::ir::TypeRef::Bytes => {
            let Some(array) = value.as_array() else {
                return Ok(None);
            };
            let items = array.iter().filter_map(serde_json::Value::as_u64).collect::<Vec<_>>();
            format!(
                "[]byte{{{}}}",
                items.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            )
        }
        crate::core::ir::TypeRef::Named(name) => go_named_field_expression(value, name, context, site, uses_pointer)?,
        crate::core::ir::TypeRef::Primitive(primitive) => {
            go_primitive_field_expression(primitive, inner, json_to_go(value), uses_pointer)
        }
        crate::core::ir::TypeRef::Json => {
            // The Go binding backend declares this field `json.RawMessage` (`*json.RawMessage`
            // when the field is pointer-shaped) — see `backends::go::type_map::go_type`. Falling
            // through to the catch-all below dropped the field from the emitted literal, so a
            // published snippet compiled while silently omitting the very value it documents
            // (alef #234). `json.RawMessage` has underlying type `[]byte`, which a Go untyped
            // string constant converts to, so the raw JSON text is a legal conversion operand.
            //
            // The spelling is load-bearing: `snippet.rs` decides whether to import
            // `encoding/json` by scanning its rendered setup lines for `json.`, so this
            // expression is what pulls the import in. ~keep
            if value.is_null() {
                return Ok(None);
            }
            let literal = format!("json.RawMessage({})", go_string_literal(&value.to_string()));
            if uses_pointer {
                format!("ptr({literal})")
            } else {
                literal
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(expression))
}

/// Lower a *top-level* argument value into an expression of the type the core IR declares for
/// the parameter it fills, or `Ok(None)` to leave the caller's pre-IR rendering alone.
///
/// The top-level argument arms have no `FieldDef` to read a type off, so before the
/// `crate::e2e::codegen::call_ir::TargetParams` seam existed every one of them rendered a bare
/// `json_to_go` literal against whatever the parameter was declared as. This converts only the
/// two cases the IR can actually settle — a declared IR enum, and a JSON object landing in a
/// declared IR struct. A name the IR does not know, an opaque handle, and a scalar against a
/// struct are all left exactly as they were: nothing here can prove what expression they want,
/// and a wrong guess is a published snippet that does not compile. ~keep
fn typed_named_argument_expression(
    value: &serde_json::Value,
    type_name: &str,
    context: GoValueContext<'_>,
    arg_name: &str,
    uses_pointer: bool,
) -> anyhow::Result<Option<String>> {
    let is_struct = context
        .type_defs
        .iter()
        .any(|definition| definition.name == type_name && !definition.is_opaque);
    let is_enum = context.enums.iter().any(|candidate| candidate.name == type_name);
    if !is_enum && !(is_struct && value.is_object()) {
        return Ok(None);
    }
    let site = GoFieldSite {
        owner_type: type_name,
        field_name: arg_name,
        pointer: "",
    };
    go_named_field_expression(value, type_name, context, site, uses_pointer).map(Some)
}

pub(super) fn native_go_dto_literal(
    value: &serde_json::Value,
    type_name: &str,
    context: GoValueContext<'_>,
) -> anyhow::Result<Option<String>> {
    native_go_dto_literal_at(value, type_name, context, "")
}

/// One [`native_go_dto_literal_at`] struct field lowered to `(Go field name, expression)`, or
/// `Ok(None)` when the fixture supplies no value for it -- the field is then omitted from the
/// literal and Go's zero value stands in. ~keep
fn lower_dto_field(
    field: &crate::core::ir::FieldDef,
    object: &serde_json::Map<String, serde_json::Value>,
    definition: &crate::core::ir::TypeDef,
    pointer: &str,
    context: GoValueContext<'_>,
    struct_names: &std::collections::HashSet<&str>,
) -> anyhow::Result<Option<(String, String)>> {
    // Fixture JSON is authored in wire format (the same JSON the binding's
    // `json.Unmarshal` accepts), so it must be looked up by the field's
    // resolved wire name — not the Rust field identifier — exactly like the
    // Go binding backend resolves the `json:"..."` tag in
    // `backends::go::gen_bindings::types::structs::gen_struct_type`.
    let wire_name = crate::codegen::naming::wire_field_name(
        &field.name,
        field.serde_rename.as_deref(),
        definition.serde_rename_all.as_deref(),
    );
    let Some(value) = object.get(&wire_name).or_else(|| object.get(&field.name)) else {
        return Ok(None);
    };
    let field_pointer = format!("{pointer}/{}", field.name);
    // Mirrors `gen_struct_type`'s `use_default_pointer` exactly. It must stay exact:
    // the fixture literal is assigned to the emitted struct field, so any disagreement
    // about pointer-ness is a Go compile error in the generated e2e suite. The former
    // extra `definition.has_default &&` conjunct was such a disagreement — the emitter
    // never consulted it. ~keep
    let uses_pointer = field.optional
        || (!field.optional && crate::backends::go::needs_omitempty_pointer(definition, field, struct_names));
    let site = GoFieldSite {
        owner_type: &definition.name,
        field_name: &field.name,
        pointer: &field_pointer,
    };
    let Some(expression) = go_struct_field_expression(field, value, context, site, uses_pointer)? else {
        return Ok(None);
    };
    Ok(Some((crate::codegen::naming::to_go_name(&field.name), expression)))
}

/// `Ok(None)` means "this is not a struct literal, render it some other way"; `Err` means the
/// value has no valid expression at this field's declared type and no snippet may be published
/// for it. Collapsing the two would turn a refusal back into the blind conversion. ~keep
fn native_go_dto_literal_at(
    value: &serde_json::Value,
    type_name: &str,
    context: GoValueContext<'_>,
    pointer: &str,
) -> anyhow::Result<Option<String>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(definition) = context.type_defs.iter().find(|definition| definition.name == type_name) else {
        return Ok(None);
    };
    // Mirrors the `struct_names` set `binding_file.rs` builds for the real Go emitter (every
    // non-opaque `TypeDef`) — see the `uses_pointer` comment below for why this must agree. ~keep
    let struct_names: std::collections::HashSet<&str> = context
        .type_defs
        .iter()
        .filter(|t| !t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();
    let field_values = definition
        .fields
        .iter()
        .map(|field| lower_dto_field(field, object, definition, pointer, context, &struct_names))
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let max_name_len = field_values
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or_default();
    let fields = field_values
        .into_iter()
        .map(|(name, expression)| {
            let padding = " ".repeat(max_name_len.saturating_sub(name.len()));
            minijinja::context! { name => name, padding => padding, expression => expression }
        })
        .collect::<Vec<_>>();
    if fields.is_empty() {
        return Ok(Some(
            crate::e2e::template_env::render(
                "go/empty_dto_literal.jinja",
                minijinja::context! { type_name => qualified_go_type(context.import_alias, type_name) },
            )
            .trim_end()
            .to_string(),
        ));
    }
    Ok(Some(
        crate::e2e::template_env::render(
            "go/dto_literal.jinja",
            minijinja::context! {
                type_name => qualified_go_type(context.import_alias, type_name), fields => fields,
            },
        )
        .trim_end()
        .to_string(),
    ))
}

/// Read-only inputs to [`build_args_and_setup`], bundled because every field is invariant
/// state the whole per-argument loop reads -- the loop's own accumulators (`package_decls`,
/// `setup_lines`, `parts`) are mutated every iteration, so they stay ordinary locals rather
/// than fields here. ~keep
#[derive(Clone, Copy)]
pub(super) struct GoArgsContext<'a> {
    pub import_alias: &'a str,
    pub options_type: Option<&'a str>,
    pub options_ptr: bool,
    pub expects_error: bool,
    pub data_enum_names: &'a std::collections::HashSet<&'a str>,
    pub config: &'a crate::core::config::ResolvedCrateConfig,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
    pub native_dtos: bool,
    pub target: crate::e2e::codegen::call_ir::TargetParams<'a>,
}

/// Returns `Err` when a configured value has no expression of its target's declared Go type.
///
/// Only reachable with `native_dtos` set (the documentation-snippet path), where the emitter
/// renders fixture values straight into typed Go struct literals. `render_snippet_body`'s
/// caller turns the error into a recorded missing-snippet entry, so a refusal is a visible
/// coverage gap rather than a published snippet that fails `go vet`. ~keep
///
/// `target` is what the core IR declares about the parameters these arguments fill. Only the
/// documentation-snippet caller supplies a real one; the e2e test-file callers pass
/// [`crate::e2e::codegen::call_ir::TargetParams::IrAbsent`], which licenses no type claim, so
/// every rendering below falls back to exactly what it emitted before the seam existed. ~keep
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture: &crate::e2e::fixture::Fixture,
    context: GoArgsContext<'_>,
) -> anyhow::Result<(Vec<String>, Vec<String>, String)> {
    let fixture_id = &fixture.id;

    if args.is_empty() {
        return Ok((Vec::new(), Vec::new(), String::new()));
    }

    let mut package_decls: Vec<String> = Vec::new();
    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for (arg_index, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            args::render_mock_url_arg(arg, input, fixture, fixture_id, &mut setup_lines, &mut parts);
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            args::render_mock_url_list_arg(arg, input, fixture, fixture_id, &mut setup_lines, &mut parts);
            continue;
        }

        if arg.arg_type == "test_backend" {
            args::render_test_backend_arg(arg, fixture, context, &mut package_decls, &mut parts);
            continue;
        }

        if arg.arg_type == "handle" {
            args::render_handle_arg(arg, input, context, &mut setup_lines, &mut parts);
            continue;
        }

        let mut sink = GoArgSink {
            package_decls: &mut package_decls,
            setup_lines: &mut setup_lines,
            parts: &mut parts,
        };
        render_data_argument(arg, arg_index, input, fixture, context, &mut sink)?;
    }

    Ok((package_decls, setup_lines, parts.join(", ")))
}

/// Mutable accumulators the per-argument renderers push into. Bundled because every renderer
/// past a plain expression builder takes at least two of them. ~keep
struct GoArgSink<'a> {
    package_decls: &'a mut Vec<String>,
    setup_lines: &'a mut Vec<String>,
    parts: &'a mut Vec<String>,
}

/// Renders one `args` entry that isn't a `mock_url`, `mock_url_list`, `test_backend` or
/// `handle` special case -- the four early `continue`s in [`build_args_and_setup`]'s loop. ~keep
fn render_data_argument(
    arg: &crate::e2e::config::ArgMapping,
    arg_index: usize,
    input: &serde_json::Value,
    fixture: &crate::e2e::fixture::Fixture,
    context: GoArgsContext<'_>,
    sink: &mut GoArgSink<'_>,
) -> anyhow::Result<()> {
    let GoArgsContext {
        import_alias,
        options_type,
        type_defs,
        enums,
        native_dtos,
        ..
    } = context;
    let fixture_id = &fixture.id;
    let val: Option<&serde_json::Value> = if arg.field == "input" {
        Some(input.get("extract_input").unwrap_or(input))
    } else {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        input.get(field)
    };
    let docs_files = fixture.docs_files_for_arg(&arg.field);
    let value_context = GoValueContext {
        import_alias,
        type_defs,
        enums,
        files: &docs_files,
    };
    // The type the core IR declares for the parameter this `args` entry fills. `None` on
    // every IR-less path (`TargetParams::IrAbsent`), which is what keeps the test-file
    // emitters rendering exactly what they rendered before this seam was threaded in. ~keep
    let declared_type_name = context.target.declared_type_name(&arg.name, arg_index);
    // `json_object_go_type` reads only hand-authored config (`go_type`, `element_type`,
    // `options_type`). When none of the three is set the old code fell through to a bare
    // JSON string literal spliced against whatever the parameter really is; the IR knows
    // that name, so it is the last resort rather than nothing. ~keep
    let json_object_type = json_object_go_type(arg, options_type).or(declared_type_name);
    // Spelling a typed Go literal is the native-DTO mode's business. The test-file
    // emitters unmarshal their argument values from JSON instead, so a declared type
    // gives them nothing to render and must not change what they emit. ~keep
    let native_declared_type = if native_dtos { declared_type_name } else { None };
    let render_context = args::GoArgRenderContext {
        arg_index,
        fixture_id,
        json_object_type,
        native_declared_type,
        value_context,
    };

    dispatch_data_argument(arg, val, render_context, context, sink)
}

/// The rendering arms every `args` entry reaching [`render_data_argument`] falls into, once its
/// `render_context` is known -- split out purely to keep `render_data_argument` under this
/// crate's function-length limit; the arms and their order are unchanged. ~keep
fn dispatch_data_argument(
    arg: &crate::e2e::config::ArgMapping,
    val: Option<&serde_json::Value>,
    render_context: args::GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
    sink: &mut GoArgSink<'_>,
) -> anyhow::Result<()> {
    if args::try_render_empty_native_json_object(arg, val, render_context, context, sink.setup_lines, sink.parts)? {
        return Ok(());
    }

    if arg.arg_type == "bytes" {
        args::render_bytes_arg(arg, val, sink.setup_lines, sink.parts);
        return Ok(());
    }

    match val {
        None | Some(serde_json::Value::Null) if arg.optional => {
            sink.parts.push(args::optional_null_argument_expression(arg, render_context, context));
        }
        None | Some(serde_json::Value::Null) => {
            sink.parts.push(args::required_null_argument_expression(arg, render_context, context));
        }
        Some(v) => match arg.arg_type.as_str() {
            "json_object" => {
                let expression = args::render_json_object_argument(
                    v,
                    arg,
                    render_context,
                    context,
                    sink.package_decls,
                    sink.setup_lines,
                )?;
                sink.parts.push(expression);
            }
            "string" if arg.optional => {
                let expression = args::render_optional_string_argument(
                    v,
                    arg,
                    render_context,
                    sink.package_decls,
                    sink.setup_lines,
                )?;
                sink.parts.push(expression);
            }
            _ => {
                let expression = args::render_typed_default_argument(
                    v,
                    arg,
                    render_context,
                    context,
                    sink.package_decls,
                    sink.setup_lines,
                )?;
                sink.parts.push(expression);
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod sealed_interface_argument_tests {
    use super::{GoArgsContext, build_args_and_setup};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
    use crate::e2e::codegen::call_ir::TargetParams;
    use crate::e2e::config::ArgMapping;
    use crate::e2e::fixture::Fixture;

    /// A variant carrying a NAMED field is not a tuple variant, which is exactly
    /// `go_enum_representation`'s condition for emitting `type SampleDoc interface { .. }`.
    fn sealed_document_enum() -> EnumDef {
        EnumDef {
            name: "SampleDoc".into(),
            rust_path: "samplelib::SampleDoc".into(),
            variants: vec![EnumVariant {
                name: "Url".into(),
                fields: vec![FieldDef {
                    name: "url".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            }],
            serde_rename_all: Some("snake_case".into()),
            ..EnumDef::default()
        }
    }

    fn document_arg() -> ArgMapping {
        ArgMapping {
            name: "document".into(),
            field: "input.document".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: Some("SampleDoc".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    /// Renders through the e2e test-file mode (`native_dtos = false`, `TargetParams::IrAbsent`)
    /// so nothing below depends on the core-IR seam -- these are shape decisions the emitter
    /// can make from the enum registry it has always been handed. ~keep
    fn render(input: serde_json::Value, enums: &[EnumDef]) -> (String, String) {
        let fixture = Fixture {
            id: "load_document".into(),
            input,
            ..Fixture::default()
        };
        let (_declarations, setup, args) = build_args_and_setup(
            &fixture.input,
            &[document_arg()],
            &fixture,
            GoArgsContext {
                import_alias: "pkg",
                options_type: None,
                options_ptr: false,
                expects_error: false,
                data_enum_names: &std::collections::HashSet::new(),
                config: &crate::core::config::ResolvedCrateConfig::default(),
                type_defs: &[],
                enums,
                native_dtos: false,
                target: TargetParams::IrAbsent,
            },
        )
        .expect("args render");
        (setup.join("\n"), args)
    }

    /// `encoding/json` cannot decode into an interface value: `var d pkg.SampleDoc;
    /// json.Unmarshal(data, &d)` compiles and then fails at RUN time. The binding backend
    /// emits `UnmarshalSampleDoc` for exactly this, and the sibling array arm already used
    /// it -- the scalar arm did not. ~keep
    #[test]
    fn a_sealed_interface_argument_is_built_by_its_unmarshal_dispatcher() {
        let (setup, args) = render(
            serde_json::json!({"document": {"url": "https://example.com/doc.pdf"}}),
            &[sealed_document_enum()],
        );

        assert!(
            setup.contains("document, documentErr := pkg.UnmarshalSampleDoc([]byte("),
            "{setup}"
        );
        assert!(
            !setup.contains("var document pkg.SampleDoc"),
            "json.Unmarshal into an interface fails at run time:\n{setup}"
        );
        assert_eq!(args, "document");
    }

    /// The negative control that keeps the dispatcher scoped to interface-shaped enums: a
    /// name the enum registry does not know keeps the `var x T; json.Unmarshal` rendering. ~keep
    #[test]
    fn a_struct_typed_argument_keeps_the_json_unmarshal_rendering() {
        let (setup, args) = render(
            serde_json::json!({"document": {"url": "https://example.com/doc.pdf"}}),
            &[],
        );

        assert!(setup.contains("var document pkg.SampleDoc"), "{setup}");
        assert!(!setup.contains("UnmarshalSampleDoc"), "{setup}");
        assert_eq!(args, "document");
    }

    /// `pkg.SampleDoc{}` is `invalid composite literal type` when `SampleDoc` is an
    /// interface; `nil` is the interface's zero value. ~keep
    #[test]
    fn an_absent_sealed_interface_argument_defaults_to_nil() {
        let (_setup, args) = render(serde_json::json!({"document": null}), &[sealed_document_enum()]);

        assert_eq!(args, "nil");
    }

    /// The same for an explicitly empty object, which takes a different arm. ~keep
    #[test]
    fn an_empty_sealed_interface_argument_defaults_to_nil() {
        let (_setup, args) = render(serde_json::json!({"document": {}}), &[sealed_document_enum()]);

        assert_eq!(args, "nil");
    }

    /// The negative control for the zero-value change: a name that is not an IR enum still
    /// gets the composite literal, so every struct-typed argument in the corpus is untouched. ~keep
    #[test]
    fn an_absent_struct_typed_argument_keeps_its_composite_literal() {
        let (_setup, args) = render(serde_json::json!({"document": null}), &[]);

        assert_eq!(args, "pkg.SampleDoc{}");
    }

    /// A unit enum is `type SampleDoc string`, which also has no composite literal -- its
    /// zero value is the empty string. ~keep
    #[test]
    fn an_absent_unit_enum_argument_defaults_to_the_empty_string() {
        let unit = EnumDef {
            name: "SampleDoc".into(),
            rust_path: "samplelib::SampleDoc".into(),
            variants: vec![EnumVariant {
                name: "Url".into(),
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        };
        let (_setup, args) = render(serde_json::json!({"document": null}), &[unit]);

        assert_eq!(args, "\"\"");
    }
}

#[cfg(test)]
mod test_backend_fallback_tests {
    use super::{GoArgsContext, build_args_and_setup};
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::ArgMapping;
    use crate::e2e::fixture::Fixture;

    fn test_backend_arg(trait_name: Option<&str>) -> ArgMapping {
        ArgMapping {
            name: "backend".into(),
            field: "backend".into(),
            arg_type: "test_backend".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: trait_name.map(str::to_string),
        }
    }

    /// A `test_backend` arg whose trait has no matching `[[crates.trait_bridges]]`
    /// entry (or no `trait_name` at all) has no compilable value to fall back to.
    /// This used to silently splice a `nil` argument plus a `// test_backend
    /// unimplemented for go` comment into the generated call; it must now fail
    /// generation loudly instead. Regression guard for the `TestBackendEmission`
    /// unimplemented-sentinel removal. ~keep
    #[test]
    fn unregistered_trait_panics_instead_of_falling_back_to_nil() {
        let config = ResolvedCrateConfig::default();
        let fixture = Fixture {
            id: "register_sample_backend".into(),
            ..Fixture::default()
        };
        let args = vec![test_backend_arg(Some("SampleBackend"))];

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build_args_and_setup(
                &fixture.input,
                &args,
                &fixture,
                GoArgsContext {
                    import_alias: "sample",
                    options_type: None,
                    options_ptr: false,
                    expects_error: false,
                    data_enum_names: &std::collections::HashSet::new(),
                    config: &config,
                    type_defs: &[],
                    enums: &[],
                    native_dtos: false,
                    target: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
                },
            )
        }));

        let error = result.expect_err("an unregistered trait must panic, not return generated Go code");
        let message = error
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| error.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        assert!(
            message.contains("cannot generate a Go stub"),
            "panic message should explain the unresolved trait bridge, got: {message}"
        );
    }

    /// A `test_backend` arg with no `trait_name` at all must fail the same way —
    /// this is the other half of the fallback condition (missing `trait_name` vs.
    /// an unresolved one), both of which used to reach the same silent `nil` path.
    #[test]
    fn missing_trait_name_panics_instead_of_falling_back_to_nil() {
        let config = ResolvedCrateConfig::default();
        let fixture = Fixture {
            id: "register_sample_backend".into(),
            ..Fixture::default()
        };
        let args = vec![test_backend_arg(None)];

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build_args_and_setup(
                &fixture.input,
                &args,
                &fixture,
                GoArgsContext {
                    import_alias: "sample",
                    options_type: None,
                    options_ptr: false,
                    expects_error: false,
                    data_enum_names: &std::collections::HashSet::new(),
                    config: &config,
                    type_defs: &[],
                    enums: &[],
                    native_dtos: false,
                    target: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
                },
            )
        }));

        assert!(
            result.is_err(),
            "a `test_backend` arg with no `trait_name` must panic, not return generated Go code"
        );
    }
}
