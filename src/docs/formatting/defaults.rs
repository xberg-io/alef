use crate::core::config::Language;
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, TypeRef};
use crate::docs::enum_variant_ref::format_enum_variant_ref;
use crate::docs::type_mapping::doc_type;

struct DefaultRendering<'a> {
    field_ty: &'a TypeRef,
    lang: Language,
    api: &'a ApiSurface,
    ffi_prefix: &'a str,
    optional: bool,
}

pub(crate) fn format_field_default(field: &FieldDef, lang: Language, api: &ApiSurface, ffi_prefix: &str) -> String {
    if let Some(typed) = &field.typed_default {
        return format_typed_default(typed, &field.ty, lang, api, ffi_prefix, field.optional);
    }
    if let Some(raw) = &field.default
        && !raw.is_empty()
    {
        let collapsed = crate::docs::doc_cleaning::collapse_whitespace(raw);
        return format!("`{collapsed}`");
    }
    if field.optional {
        return match lang {
            Language::Python => "`None`".to_string(),
            Language::Node | Language::Wasm => "`null`".to_string(),
            Language::Go => "`nil`".to_string(),
            Language::Java => "`null`".to_string(),
            Language::Csharp => "`null`".to_string(),
            Language::Ruby => "`nil`".to_string(),
            Language::Php => "`null`".to_string(),
            Language::Elixir => "`nil`".to_string(),
            Language::R => "`NULL`".to_string(),
            Language::Rust => "`None`".to_string(),
            Language::Ffi | Language::C | Language::Jni => "`NULL`".to_string(),
            Language::Kotlin
            | Language::KotlinAndroid
            | Language::Swift
            | Language::Dart
            | Language::Gleam
            | Language::Zig => "`null`".to_string(),
        };
    }
    "—".to_string()
}

/// Qualifies a bare variant name (`Curated`) with its owning enum type when `field_ty` names
/// one, mirroring the fallback branch of the `DefaultValue::EnumVariant` case below — shared so
/// `TupleVariant` and `StructVariant` document a variant reference the same way a unit variant
/// does. Unlike `EnumVariant`, `TupleVariant`/`StructVariant` never carry a `Type::Variant`
/// qualified string, so there is no `splitn` case to mirror. ~keep
fn enum_variant_doc_label(variant: &str, field_ty: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    let enum_type_name_str = match field_ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    };
    match enum_type_name_str {
        Some(type_str) => format_enum_variant_ref(type_str, variant, lang, ffi_prefix),
        None => variant.to_string(),
    }
}

pub(crate) fn format_typed_default(
    val: &DefaultValue,
    field_ty: &TypeRef,
    lang: Language,
    api: &ApiSurface,
    ffi_prefix: &str,
    optional: bool,
) -> String {
    let context = DefaultRendering {
        field_ty,
        lang,
        api,
        ffi_prefix,
        optional,
    };
    context.render(val)
}

fn format_enum_default(value: &str, field_ty: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    let label = if let Some((owner, variant)) = value.split_once("::") {
        format_enum_variant_ref(owner, variant, lang, ffi_prefix)
    } else {
        enum_variant_doc_label(value, field_ty, lang, ffi_prefix)
    };
    format!("`{label}`")
}

// Documented in a language-neutral bracket form. Elements route back through this same
// function so a Python `True` stays `True` inside the list; the per-element backticks
// are stripped because the whole list carries one pair. ~keep
impl DefaultRendering<'_> {
    fn format_list_default(&self, items: &[DefaultValue]) -> String {
        let rendered: Vec<String> = items
            .iter()
            .map(|item| {
                format_typed_default(item, self.field_ty, self.lang, self.api, self.ffi_prefix, self.optional)
                    .trim_matches('`')
                    .to_string()
            })
            .collect();
        format!("`[{}]`", rendered.join(", "))
    }
}

// A tuple- or struct-variant default carries its own values, so — unlike the bare
// `EnumVariant` case — there is always something concrete to document; each argument or
// field routes back through this same function, the way `ListLiteral`'s elements do. ~keep
impl DefaultRendering<'_> {
    fn format_tuple_default(&self, variant: &str, values: &[DefaultValue]) -> String {
        let rendered: Vec<String> = values
            .iter()
            .map(|item| {
                format_typed_default(item, self.field_ty, self.lang, self.api, self.ffi_prefix, self.optional)
                    .trim_matches('`')
                    .to_string()
            })
            .collect();
        format!(
            "`{}({})`",
            enum_variant_doc_label(variant, self.field_ty, self.lang, self.ffi_prefix),
            rendered.join(", ")
        )
    }
}

impl DefaultRendering<'_> {
    fn format_struct_default(&self, variant: &str, fields: &[(String, DefaultValue)]) -> String {
        let rendered: Vec<String> = fields
            .iter()
            .map(|(name, item)| {
                let value =
                    format_typed_default(item, self.field_ty, self.lang, self.api, self.ffi_prefix, self.optional)
                        .trim_matches('`')
                        .to_string();
                format!("{name}: {value}")
            })
            .collect();
        format!(
            "`{} {{ {} }}`",
            enum_variant_doc_label(variant, self.field_ty, self.lang, self.ffi_prefix),
            rendered.join(", ")
        )
    }
}

// `Unresolved` documents the type-zero, same as `Empty`. That is the prose half of the
// very drift this variant exists to expose — generated docs claiming `0ms` for a default
// alef could not read — and it is left here only because the validation pass refuses the
// surface before docs are written unless the crate suppressed the diagnostic. ~keep
fn format_empty_default(
    field_ty: &TypeRef,
    lang: Language,
    api: &ApiSurface,
    ffi_prefix: &str,
    optional: bool,
) -> String {
    let inner_for_dur = match field_ty {
        TypeRef::Optional(inner) => Some(inner.as_ref()),
        other => Some(other),
    };
    if matches!(inner_for_dur, Some(TypeRef::Duration)) {
        return match lang {
            Language::Rust => "`Duration::default()`".to_string(),
            _ => "`0ms`".to_string(),
        };
    }

    if let Some(default) = format_empty_enum(field_ty, lang, api, ffi_prefix, optional) {
        return default;
    }
    let inner_ty = match field_ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    if matches!(inner_ty, TypeRef::Vec(_)) {
        return format_empty_vec(inner_ty, lang, ffi_prefix);
    }
    if matches!(inner_ty, TypeRef::Map(_, _)) {
        return format_empty_map(inner_ty, lang, ffi_prefix);
    }
    format_empty_optional(lang, optional)
}
fn format_none_default(lang: Language, optional: bool) -> String {
    if !optional {
        return "—".to_string();
    }
    match lang {
        Language::Python => "`None`".to_string(),
        Language::Node | Language::Wasm => "`null`".to_string(),
        Language::Go => "`nil`".to_string(),
        Language::Java => "`null`".to_string(),
        Language::Csharp => "`null`".to_string(),
        Language::Ruby => "`nil`".to_string(),
        Language::Php => "`null`".to_string(),
        Language::Elixir => "`nil`".to_string(),
        Language::R => "`NULL`".to_string(),
        Language::Rust => "`None`".to_string(),
        Language::Ffi | Language::C | Language::Jni => "`NULL`".to_string(),
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => "`null`".to_string(),
    }
}

fn format_empty_vec(inner_ty: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python => "`[]`".to_string(),
        Language::Node | Language::Wasm => "`[]`".to_string(),
        Language::Go => "`nil`".to_string(),
        Language::Java => "`Collections.emptyList()`".to_string(),
        Language::Csharp => {
            let elem_ty = if let TypeRef::Vec(elem) = inner_ty {
                doc_type(elem, lang, ffi_prefix)
            } else {
                String::new()
            };
            format!("`new List<{elem_ty}>()`")
        }
        Language::Ruby | Language::Elixir => "`[]`".to_string(),
        Language::Php => "`[]`".to_string(),
        Language::Rust => "`vec![]`".to_string(),
        Language::Ffi | Language::C | Language::Jni => "`NULL`".to_string(),
        Language::R => "`list()`".to_string(),
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => "`[]`".to_string(),
    }
}

fn format_empty_map(inner_ty: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python | Language::Ruby | Language::Php => "`{}`".to_string(),
        Language::Node | Language::Wasm => "`{}`".to_string(),
        Language::Go => "`nil`".to_string(),
        Language::Elixir => "`%{}`".to_string(),
        Language::Java => "`Collections.emptyMap()`".to_string(),
        Language::Csharp => {
            if let TypeRef::Map(k, v) = inner_ty {
                let kty = doc_type(k, lang, ffi_prefix);
                let vty = doc_type(v, lang, ffi_prefix);
                format!("`new Dictionary<{kty}, {vty}>()`")
            } else {
                "`new Dictionary<>()`".to_string()
            }
        }
        Language::Rust => "`HashMap::new()`".to_string(),
        Language::Ffi | Language::C | Language::Jni => "`NULL`".to_string(),
        Language::R => "`list()`".to_string(),
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => "`{}`".to_string(),
    }
}

fn format_empty_optional(lang: Language, optional: bool) -> String {
    if !optional {
        return "—".to_string();
    }
    match lang {
        Language::Python => "`None`".to_string(),
        Language::Node | Language::Wasm => "`null`".to_string(),
        Language::Go => "`nil`".to_string(),
        Language::Java => "`null`".to_string(),
        Language::Csharp => "`null`".to_string(),
        Language::Ruby => "`nil`".to_string(),
        Language::Php => "`null`".to_string(),
        Language::Elixir => "`nil`".to_string(),
        Language::R => "`NULL`".to_string(),
        Language::Rust => "`Default::default()`".to_string(),
        Language::Ffi | Language::C | Language::Jni => "`NULL`".to_string(),
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => "`null`".to_string(),
    }
}

fn format_empty_enum(
    field_ty: &TypeRef,
    lang: Language,
    api: &ApiSurface,
    ffi_prefix: &str,
    optional: bool,
) -> Option<String> {
    if !optional
        && let TypeRef::Named(type_name_str) = field_ty
        && let Some(enum_def) = api.enums.iter().find(|e| &e.name == type_name_str)
    {
        let variant = enum_def
            .variants
            .iter()
            .find(|v| v.is_default)
            .or_else(|| enum_def.variants.first());
        if let Some(v) = variant {
            return Some(format!(
                "`{}`",
                format_enum_variant_ref(type_name_str, &v.name, lang, ffi_prefix)
            ));
        }
    }
    None
}

impl DefaultRendering<'_> {
    fn render(&self, val: &DefaultValue) -> String {
        match val {
            DefaultValue::BoolLiteral(b) => match self.lang {
                Language::Python => format!("`{}`", if *b { "True" } else { "False" }),
                _ => format!("`{b}`"),
            },
            DefaultValue::StringLiteral(s) => {
                let normalized = crate::docs::doc_cleaning::collapse_whitespace(s);
                format!("`\"{normalized}\"`")
            }
            DefaultValue::ListLiteral(items) => self.format_list_default(items),
            DefaultValue::IntLiteral(n) => {
                if matches!(self.field_ty, TypeRef::Duration)
                    || matches!(self.field_ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Duration))
                {
                    return format!("`{n}ms`");
                }
                format!("`{n}`")
            }
            DefaultValue::FloatLiteral(f) => format!("`{f}`"),
            DefaultValue::EnumVariant(v) => format_enum_default(v, self.field_ty, self.lang, self.ffi_prefix),
            DefaultValue::TupleVariant(variant, values) => self.format_tuple_default(variant, values),
            DefaultValue::StructVariant(variant, fields) => self.format_struct_default(variant, fields),
            DefaultValue::Empty | DefaultValue::Unresolved(_) => {
                format_empty_default(self.field_ty, self.lang, self.api, self.ffi_prefix, self.optional)
            }
            DefaultValue::None => format_none_default(self.lang, self.optional),
            DefaultValue::FunctionCall(path) | DefaultValue::PublicFunctionCall(path) => format!("`{path}()`"),
        }
    }
}
