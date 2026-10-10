use crate::core::ir::{DefaultValue, EnumDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::template_env;
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::Language;

use super::render_type::{format_param, render_type};

/// Single source of truth for the Dart call shape of a `config` parameter.
///
/// `true` means the wrapper declares it as a named-optional parameter (`{Type? config}`)
/// and every caller must pass it with a `config:` label; `false` means it stays a required
/// positional parameter and every caller must pass it positionally.
///
/// Two emitters need this one fact and must never derive it independently: [`emit_function`]
/// below writes the declaration, and `e2e::codegen::dart::test_case` writes the call sites
/// that appear in generated doc snippets and e2e tests. Each half is well-formed on its own,
/// so nothing but the composed output can show a disagreement — hence this shared function
/// rather than two matching rules. ~keep
///
/// Only a parameter literally named `config` is eligible, and only when alef can emit a Dart
/// expression for its default: FRB-generated DTOs use `required` named parameters for every
/// field, so a bare `Type()` constructor compiles only when alef can produce a value for
/// every field — see [`config_default_expression`] for the two ways that value is obtained.
pub(crate) fn config_param_is_named_optional(
    param_name: &str,
    type_name: &str,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> bool {
    param_name == "config" && config_default_expression(type_name, type_defs, enums).is_some()
}

fn is_optional_config_param(p: &crate::core::ir::ParamDef, type_defs: &[TypeDef], enums: &[EnumDef]) -> bool {
    let TypeRef::Named(name) = &p.ty else {
        return false;
    };
    config_param_is_named_optional(&p.name, name, type_defs, enums)
}

/// Dart expression producing the default value for an optional `config` parameter.
///
/// Two strategies, in order:
///
/// 1. A synthesized constructor call (`Type(field: value, …)`), used when alef can emit a
///    literal for every field.
/// 2. The generated `create<Type>FromJson(json: '{}')` bridge helper, which round-trips an
///    empty object through serde on the Rust side.
///
/// The fallback exists because strategy 1 cannot render `#[serde(default = "path")]`
/// fields — those reach the IR as [`DefaultValue::FunctionCall`], whose body alef never
/// sees. A single such field used to disqualify the whole type, which silently downgraded
/// the wrapper to a required positional `config` (`ExtractionConfig` has six of them).
/// Deferring to serde is also more faithful than the synthesized literal: it yields the
/// value the function-call default actually returns rather than a zero value.
///
/// Every wrapper is `async`, so `await` is legal in the emitted default expression.
fn config_default_expression(name: &str, type_defs: &[TypeDef], enums: &[EnumDef]) -> Option<String> {
    default_expression_for_named_type(name, type_defs, enums).or_else(|| from_json_default_expression(name, type_defs))
}

/// `await create<Type>FromJson(json: '{}')`, or `None` when no such helper is generated.
///
/// Mirrors the emission predicate in `gen_rust_crate` — the helper exists for exactly the
/// non-trait, non-opaque, serde-bearing types — so this never names a function that was
/// not emitted.
fn from_json_default_expression(name: &str, type_defs: &[TypeDef]) -> Option<String> {
    let ty = type_defs.iter().find(|ty| {
        ty.name == name && ty.has_default && ty.has_serde && !ty.is_trait && !ty.is_opaque && !ty.binding_excluded
    })?;
    let snake = public_host_identifier(Language::Rust, PublicIdentifierKind::Function, &ty.name);
    let dart_fn = format!("create_{snake}_from_json").to_lower_camel_case();
    Some(format!("await {dart_fn}(json: '{{}}')"))
}

pub(super) fn emit_function(
    f: &FunctionDef,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    if !f.doc.is_empty() {
        let doc_lines: Vec<String> = f.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            crate::alef_context! {
                indent => "  ",
                lines => doc_lines,
            },
        ));
    }
    if let Some(ref error_ty) = f.error_type {
        out.push_str(&template_env::render(
            "function_throws_annotation.jinja",
            crate::alef_context! {
                error_ty => error_ty.as_str(),
            },
        ));
    }

    let fn_name = dart_safe_ident(&f.name.to_lower_camel_case());

    let config_param = f.params.iter().find(|p| is_optional_config_param(p, type_defs, enums));
    let config_default = config_param.and_then(|p| match &p.ty {
        TypeRef::Named(n) => config_default_expression(n, type_defs, enums).map(|default| (n.as_str(), default)),
        _ => None,
    });

    let params_str = if let Some((cfg_type, _)) = &config_default {
        let required_params: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_optional_config_param(p, type_defs, enums))
            .map(|p| format_param(p, imports))
            .collect();
        let config_sig = format!("{{{cfg_type}? config}}");
        if required_params.is_empty() {
            config_sig
        } else {
            format!("{}, {config_sig}", required_params.join(", "))
        }
    } else {
        let required: Vec<String> = f
            .params
            .iter()
            .filter(|p| !p.optional)
            .map(|p| format_param(p, imports))
            .collect();
        let optional: Vec<String> = f
            .params
            .iter()
            .filter(|p| p.optional)
            .map(|p| format_param(p, imports))
            .collect();
        match (required.is_empty(), optional.is_empty()) {
            (true, true) => String::new(),
            (false, true) => required.join(", "),
            (true, false) => format!("{{{}}}", optional.join(", ")),
            (false, false) => format!("{}, {{{}}}", required.join(", "), optional.join(", ")),
        }
    };

    let call_args_str = if let Some((_, default_expr)) = &config_default {
        let non_config: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_optional_config_param(p, type_defs, enums))
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect();
        let config_arg = format!("config: config ?? {default_expr}");
        if non_config.is_empty() {
            config_arg
        } else {
            format!("{}, {config_arg}", non_config.join(", "))
        }
    } else {
        f.params
            .iter()
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    {
        // A `&mut T` DTO writeback function's IR `return_type` still records the original
        // `Unit` -- extraction is unchanged -- but the FRB bridge fn this facade method
        // delegates to now returns `T` (see `gen_rust_crate::bridge_fn`). The facade must
        // declare the same `Future<T>`, not `Future<void>`, or it would discard the value
        // the bridge call actually produces. ~keep
        let opaque_types: ahash::AHashSet<String> = type_defs
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        let effective_return_type =
            crate::codegen::mut_writeback::effective_return_type(&f.params, &f.return_type, &opaque_types)
                .unwrap_or_else(|| f.return_type.clone());
        let return_ty = if matches!(effective_return_type, TypeRef::Unit) {
            "Future<void>".to_string()
        } else {
            format!("Future<{}>", render_type(&effective_return_type, imports))
        };
        out.push_str(&template_env::render(
            "function_signature_async.jinja",
            crate::alef_context! {
                return_ty => return_ty,
                fn_name => fn_name.as_str(),
                params => params_str.as_str(),
            },
        ));
        out.push_str(&template_env::render(
            "function_await_return.jinja",
            crate::alef_context! {
                fn_name => fn_name.as_str(),
                call_args_str => call_args_str.as_str(),
            },
        ));
        out.push_str("  }\n");
    }
}

fn default_expression_for_named_type(name: &str, type_defs: &[TypeDef], enums: &[EnumDef]) -> Option<String> {
    let ty = type_defs.iter().find(|ty| ty.name == name && ty.has_default)?;
    let fields: Vec<String> = ty
        .fields
        .iter()
        .filter(|field| !field.binding_excluded)
        .map(|field| {
            let field_name = dart_safe_ident(&field.name.to_lower_camel_case());
            let value = default_expression_for_field(field, type_defs, enums)?;
            Some(format!("{field_name}: {value}"))
        })
        .collect::<Option<Vec<_>>>()?;

    if fields.is_empty() {
        Some(format!("{name}()"))
    } else {
        Some(format!("{name}({})", fields.join(", ")))
    }
}

fn default_expression_for_field(
    field: &crate::core::ir::FieldDef,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Option<String> {
    if let Some(default) = &field.typed_default {
        // `field.ty` is already unwrapped from `Option<T>` (see
        // `extract::extractor::helpers::fields::unwrap_optional`), so `zero_value_for_type`
        // below cannot tell "the field's own type, `Option<Enum>`, is at its zero" (`None`) from
        // "the wrapped `Enum` is at its zero" (its own default variant). `Empty` on an optional
        // field means the former; rendering the latter would synthesize a concrete enum variant
        // for a value the Rust source left absent, and that materialized value would then be
        // passed to Rust as this `config` parameter's default, indistinguishable from a caller's
        // explicit choice. `DefaultValue::None` on an optional field already answers `null`
        // further down in `render_default_value`; only `Empty` needs the same answer written
        // explicitly here because its type-driven fallback (`zero_value_for_type`) does not know
        // about `field.optional`. ~keep
        if field.optional && matches!(default, DefaultValue::Empty) {
            return Some("null".to_string());
        }
        return render_default_value(&field.ty, default, type_defs, enums);
    }
    if field.optional {
        return Some("null".to_string());
    }
    zero_value_for_type(&field.ty, type_defs, enums)
}

fn render_default_value(
    ty: &TypeRef,
    default: &DefaultValue,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Option<String> {
    match default {
        DefaultValue::BoolLiteral(value) => Some(value.to_string()),
        DefaultValue::StringLiteral(value) => Some(format!("'{}'", escape_dart_string(value))),
        DefaultValue::IntLiteral(value) => Some(value.to_string()),
        // `{f}` on a non-finite `f64` prints `NaN`/`inf`, neither of which names anything in Dart
        // (`double.nan`, `double.infinity`), so the emitted default would not parse. Returning
        // `None` propagates all the way up and routes the whole type through the serde round-trip
        // below, which is the faithful answer rather than a substituted zero. The whole-valued
        // rule comes along with the shared helper. ~keep
        DefaultValue::FloatLiteral(value) => crate::codegen::shared::float_literal_digits(*value),
        DefaultValue::EnumVariant(variant) => render_enum_variant_default(ty, variant, enums),
        DefaultValue::ListLiteral(items) => {
            let element_ty = match ty {
                TypeRef::Vec(inner) => inner.as_ref(),
                other => other,
            };
            let rendered: Option<Vec<String>> = items
                .iter()
                .map(|item| render_default_value(element_ty, item, type_defs, enums))
                .collect();
            // An element this renderer cannot express falls back to the empty collection rather
            // than a partial list, matching the extractor's all-or-nothing rule. ~keep
            match rendered {
                Some(values) => Some(format!("const [{}]", values.join(", "))),
                None => zero_value_for_type(ty, type_defs, enums),
            }
        }
        DefaultValue::Empty => zero_value_for_type(ty, type_defs, enums),
        // `Unresolved`: alef could not read the real default out of `impl Default`.
        // `TupleVariant`/`StructVariant`: alef read the value, but this renderer has no Dart
        // expression for "construct enum variant X with these field values" the way it does for
        // a bare `EnumVariant`. Both used to fall through to `zero_value_for_type` (as `Empty`
        // still does above), which ships the *type's* zero underneath a doc comment quoting the
        // real (unrendered) Rust default — a value the source never actually specified.
        // Returning `None` here, like `FunctionCall` below, is what lets the `?` in
        // `default_expression_for_named_type` bail the whole synthesized literal out to
        // `config_default_expression`'s JSON round-trip fallback, which is faithful rather than
        // a guess. ~keep
        DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..) => None,
        DefaultValue::None => Some("null".to_string()),
        DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => None,
    }
}

fn zero_value_for_type(ty: &TypeRef, type_defs: &[TypeDef], enums: &[EnumDef]) -> Option<String> {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => Some("false".to_string()),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => Some("0.0".to_string()),
        TypeRef::Primitive(_) => Some("0".to_string()),
        TypeRef::String | TypeRef::Char | TypeRef::Path => Some("''".to_string()),
        TypeRef::Bytes => Some("Uint8List(0)".to_string()),
        TypeRef::Vec(inner) => Some(empty_vec_literal(inner)),
        TypeRef::Map(_, _) | TypeRef::Json => Some("{}".to_string()),
        TypeRef::Optional(_) | TypeRef::Unit => Some("null".to_string()),
        TypeRef::Duration => Some("Duration.zero".to_string()),
        TypeRef::Named(name) => {
            if let Some(default) = default_enum_variant(name, enums) {
                render_enum_variant_default(ty, default, enums)
            } else {
                default_expression_for_named_type(name, type_defs, enums)
            }
        }
    }
}

/// Empty-`Vec` default that matches the FRB-mapped Dart type.
///
/// Alef's `gen_rust_crate` widens every Rust integer to `i64` and every float
/// to `f64` in the FRB-facing mirror struct (see `gen_rust_crate::mirror`),
/// matching FRB's own widening behavior. FRB then maps `Vec<i64>` →
/// `Int64List` and `Vec<f64>` → `Float64List` in the Dart class. `Vec<u8>` is
/// a special case (kept as `Vec<u8>` for byte buffers, mapped to `Uint8List`).
///
/// A plain `[]` literal is `List<dynamic>` and fails to satisfy the FRB ctor's
/// typed-list parameter, so we emit the typed-list constructor matching the
/// widened FRB type. Non-primitive element types (Strings, named structs,
/// nested Vecs, etc.) stay as `List<T>` in FRB and accept `[]`.
fn empty_vec_literal(inner: &TypeRef) -> String {
    match inner {
        TypeRef::Primitive(PrimitiveType::U8) => "Uint8List(0)".to_string(),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "Float64List(0)".to_string(),
        TypeRef::Primitive(
            PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64,
        ) => "Int64List(0)".to_string(),
        _ => "[]".to_string(),
    }
}

fn render_enum_variant_default(ty: &TypeRef, variant: &str, enums: &[EnumDef]) -> Option<String> {
    let TypeRef::Named(name) = ty else {
        return None;
    };
    let variant_name = dart_safe_ident(&variant.to_lower_camel_case());
    let enum_def = enums.iter().find(|e| e.name == *name)?;
    let enum_variant = enum_def.variants.iter().find(|v| v.name == variant)?;
    let is_flat_enum = enum_def.variants.iter().all(|v| v.fields.is_empty());
    if is_flat_enum && enum_variant.fields.is_empty() {
        Some(format!("{name}.{variant_name}"))
    } else {
        Some(format!("{name}.{variant_name}()"))
    }
}

fn default_enum_variant<'a>(name: &str, enums: &'a [EnumDef]) -> Option<&'a str> {
    enums
        .iter()
        .find(|e| e.name == name)
        .and_then(|e| e.variants.iter().find(|v| v.is_default))
        .map(|v| v.name.as_str())
}

fn escape_dart_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::PrimitiveType;

    #[test]
    fn empty_vec_of_integer_primitive_uses_int64list_ctor() {
        let widened_to_int64 = [
            PrimitiveType::U16,
            PrimitiveType::U32,
            PrimitiveType::U64,
            PrimitiveType::I8,
            PrimitiveType::I16,
            PrimitiveType::I32,
            PrimitiveType::I64,
        ];
        for prim in widened_to_int64 {
            let prim_dbg = format!("{prim:?}");
            let got = empty_vec_literal(&TypeRef::Primitive(prim));
            assert_eq!(got, "Int64List(0)", "Vec<{prim_dbg}> empty default");
        }
        assert_eq!(
            empty_vec_literal(&TypeRef::Primitive(PrimitiveType::U8)),
            "Uint8List(0)"
        );
    }

    #[test]
    fn empty_vec_of_float_primitive_uses_float64list_ctor() {
        assert_eq!(
            empty_vec_literal(&TypeRef::Primitive(PrimitiveType::F32)),
            "Float64List(0)"
        );
        assert_eq!(
            empty_vec_literal(&TypeRef::Primitive(PrimitiveType::F64)),
            "Float64List(0)"
        );
    }

    #[test]
    fn empty_vec_of_string_or_named_stays_list_literal() {
        assert_eq!(empty_vec_literal(&TypeRef::String), "[]");
        assert_eq!(empty_vec_literal(&TypeRef::Named("Foo".to_string())), "[]");
        assert_eq!(empty_vec_literal(&TypeRef::Vec(Box::new(TypeRef::String))), "[]");
    }

    #[test]
    fn bytes_default_is_typed_uint8list() {
        assert_eq!(
            zero_value_for_type(&TypeRef::Bytes, &[], &[]),
            Some("Uint8List(0)".to_string())
        );
    }

    fn detection_policy_enum() -> EnumDef {
        EnumDef {
            name: "DetectionPolicy".to_string(),
            has_default: true,
            variants: vec![
                crate::core::ir::EnumVariant {
                    name: "PreferContent".to_string(),
                    is_default: true,
                    ..Default::default()
                },
                crate::core::ir::EnumVariant {
                    name: "ContentOnly".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// Security control (task #558): `default_expression_for_field` is what synthesizes the
    /// literal `config`-parameter default (`Type(field: value, …)`) a caller gets when they omit
    /// a `config` argument entirely. An `Option<Enum>` field with no explicit default resolves to
    /// `typed_default = Some(DefaultValue::Empty)` after extraction, and the emitted Dart
    /// expression for that field must stay `null` — never the enum's own `#[default]` variant.
    /// `zero_value_for_type` cannot make this distinction on its own: `field.ty` is already
    /// unwrapped from `Option<T>` by the extractor, so it only ever sees the bare enum and has no
    /// way to know the field itself was optional. Forwarding a materialized variant here is
    /// indistinguishable from a caller's explicit choice once it reaches the Rust core, and can
    /// silently override a stricter policy the caller set elsewhere (e.g. a global
    /// content-only mode).
    #[test]
    fn optional_enum_field_with_no_explicit_default_renders_null_not_the_variant() {
        use crate::core::ir::FieldDef;

        let enums = [detection_policy_enum()];
        let field = FieldDef {
            name: "detection_policy".to_string(),
            ty: TypeRef::Named("DetectionPolicy".to_string()),
            optional: true,
            typed_default: Some(DefaultValue::Empty),
            ..Default::default()
        };

        let rendered = default_expression_for_field(&field, &[], &enums);

        assert_eq!(
            rendered,
            Some("null".to_string()),
            "an Option<Enum> field with no explicit default must render `null`, not a \
             materialized variant like `DetectionPolicy.preferContent`"
        );
    }

    /// Negative control: a *required* (non-optional) `Enum` field with the same `Empty` typed
    /// default legitimately does resolve to the enum's own default variant. A fix that
    /// suppressed every `Empty`-on-`Named` default (rather than only the optional-field case)
    /// would pass the positive test above while silently breaking this legitimate one.
    #[test]
    fn required_enum_field_with_empty_default_still_renders_the_default_variant() {
        use crate::core::ir::FieldDef;

        let enums = [detection_policy_enum()];
        let field = FieldDef {
            name: "detection_policy".to_string(),
            ty: TypeRef::Named("DetectionPolicy".to_string()),
            optional: false,
            typed_default: Some(DefaultValue::Empty),
            ..Default::default()
        };

        let rendered = default_expression_for_field(&field, &[], &enums);

        assert_eq!(
            rendered,
            Some("DetectionPolicy.preferContent".to_string()),
            "a required enum field must still render its own `#[default]` variant"
        );
    }

    /// Negative control: an `Option<Enum>` field that genuinely does have an explicit default
    /// naming a concrete variant (`typed_default = Some(EnumVariant(..))`, not narrowed from
    /// `Empty`) must still render that real value.
    #[test]
    fn optional_enum_field_with_explicit_variant_default_still_renders_it() {
        use crate::core::ir::FieldDef;

        let enums = [detection_policy_enum()];
        let field = FieldDef {
            name: "detection_policy".to_string(),
            ty: TypeRef::Named("DetectionPolicy".to_string()),
            optional: true,
            typed_default: Some(DefaultValue::EnumVariant("ContentOnly".to_string())),
            ..Default::default()
        };

        let rendered = default_expression_for_field(&field, &[], &enums);

        assert_eq!(
            rendered,
            Some("DetectionPolicy.contentOnly".to_string()),
            "an explicit variant default on an optional field must still be rendered"
        );
    }
}

/// The `config` parameter's call shape is one fact written by two emitters: the Dart wrapper
/// declaration here, and the call site in `e2e::codegen::dart::test_case` that lands in every
/// generated doc snippet and e2e test. Each half is well-formed in isolation, so only the
/// composed output can reveal a disagreement — which is why these tests run both emitters over
/// one input and compare, instead of pinning each side to its own expected string. ~keep
#[cfg(test)]
mod call_shape_agreement_tests {
    use super::*;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ParamDef;
    use crate::e2e::codegen::E2eCodegen;
    use crate::e2e::codegen::dart::DartE2eCodegen;
    use crate::e2e::config::{ArgMapping, E2eConfig};
    use crate::e2e::fixture::Fixture;

    fn config_type(name: &str, has_default: bool) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            has_default,
            has_serde: true,
            ..Default::default()
        }
    }

    fn embed_function(config_type_name: &str) -> FunctionDef {
        FunctionDef {
            name: "embed".to_string(),
            params: vec![ParamDef {
                name: "config".to_string(),
                ty: TypeRef::Named(config_type_name.to_string()),
                ..Default::default()
            }],
            return_type: TypeRef::String,
            ..Default::default()
        }
    }

    /// `true` when the emitted wrapper declares `{Type? config}`, `false` when it declares a
    /// required positional `Type config`.
    fn binding_declares_named_config(config_type_name: &str, type_defs: &[TypeDef]) -> bool {
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_function(
            &embed_function(config_type_name),
            type_defs,
            &[],
            &mut out,
            &mut imports,
        );
        let named = out.contains(&format!("{{{config_type_name}? config}}"));
        let positional = out.contains(&format!("({config_type_name} config)"));
        assert!(
            named != positional,
            "binding must declare exactly one of the two shapes:\n{out}"
        );
        named
    }

    /// `true` when the generated snippet passes the config with a `config:` label, `false` when
    /// it passes it positionally.
    fn snippet_passes_named_config(config_type_name: &str, type_defs: &[TypeDef]) -> bool {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "embed_text",
            "description": "Embed text",
            "input": {"config": {"model": "small"}}
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "embed".into();
        e2e_config.call.result_var = "result".into();
        e2e_config.call.options_type = Some(config_type_name.to_string());
        e2e_config.call.args.push(ArgMapping {
            name: "config".into(),
            field: "input.config".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        });

        let body = DartE2eCodegen
            .render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), type_defs, &[])
            .expect("snippet");

        let named = body.contains("embed(config: config)");
        let positional = body.contains("embed(config)");
        assert!(
            named != positional,
            "snippet must pass the config in exactly one of the two shapes:\n{body}"
        );
        named
    }

    /// A `config` type alef can default (`has_default` + `has_serde`) gets the named-optional
    /// declaration, so the snippet must use the `config:` label. The type name deliberately
    /// contains `Embedding`, the substring `test_case.rs` used to route to a positional call.
    #[test]
    fn snippet_call_site_agrees_with_binding_for_a_defaultable_config() {
        let type_defs = [config_type("EmbeddingConfig", true)];

        let binding_named = binding_declares_named_config("EmbeddingConfig", &type_defs);
        let snippet_named = snippet_passes_named_config("EmbeddingConfig", &type_defs);

        assert!(
            binding_named,
            "a defaultable `config` is declared `{{EmbeddingConfig? config}}`, which Dart can only \
             be passed by label"
        );
        assert_eq!(
            binding_named, snippet_named,
            "the snippet call site and the binding signature disagree about the `config` \
             parameter shape; both must derive it from `config_param_is_named_optional`"
        );
    }

    /// The mirror case: without a `Default` impl alef cannot synthesize a default expression, so
    /// the wrapper keeps `config` required and positional and a labelled call site would not
    /// compile.
    #[test]
    fn snippet_call_site_agrees_with_binding_for_a_config_without_a_default() {
        let type_defs = [config_type("ExtractionConfig", false)];

        let binding_named = binding_declares_named_config("ExtractionConfig", &type_defs);
        let snippet_named = snippet_passes_named_config("ExtractionConfig", &type_defs);

        assert!(
            !binding_named,
            "with no `Default` impl there is no default expression to emit, so `config` stays \
             required and positional"
        );
        assert_eq!(
            binding_named, snippet_named,
            "the snippet call site and the binding signature disagree about the `config` \
             parameter shape; both must derive it from `config_param_is_named_optional`"
        );
    }

    #[test]
    fn shared_predicate_answers_only_for_a_parameter_named_config() {
        let type_defs = [config_type("EmbeddingConfig", true)];

        assert!(config_param_is_named_optional(
            "config",
            "EmbeddingConfig",
            &type_defs,
            &[]
        ));
        assert!(
            !config_param_is_named_optional("settings", "EmbeddingConfig", &type_defs, &[]),
            "only a parameter literally named `config` is eligible"
        );
        assert!(
            !config_param_is_named_optional("config", "UnknownConfig", &type_defs, &[]),
            "a type alef cannot default has no default expression to make the parameter optional"
        );
    }
}
