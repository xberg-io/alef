use super::field_defaults::non_optional_fallback;
use super::{swift_field_type, swift_typed_default_literal};
use crate::backends::swift::naming::swift_source_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::core::ir::{FieldDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

/// Decodes absent keys through proven field defaults while preserving required siblings.
/// Absent defaults and explicit null values are distinct, including for existing owner decoders.
pub(crate) fn emit_decoder_init(
    mapper: &SwiftMapper,
    visible_fields: &[&FieldDef],
    zero_arg_constructible_names: &HashSet<String>,
    every_field_may_be_absent: bool,
    deny_unknown_fields: bool,
    out: &mut String,
) {
    emit_decoder_init_with_aliases(
        mapper,
        visible_fields,
        zero_arg_constructible_names,
        every_field_may_be_absent,
        out,
        DecoderKeys {
            strict: deny_unknown_fields,
            aliases: None,
            wire_key_type: "AlefWireKey",
        },
    );
}

pub(super) struct DecoderKeys<'a> {
    pub(super) strict: bool,
    pub(super) aliases: Option<&'a super::aliases::AliasDecoding>,
    pub(super) wire_key_type: &'a str,
}

impl DecoderKeys<'_> {
    fn field(&self, field: &FieldDef, canonical: &str) -> String {
        self.aliases
            .and_then(|aliases| aliases.keys.get(&field.name))
            .cloned()
            .unwrap_or_else(|| format!(".{canonical}"))
    }
}

fn emit_preamble(keys: &DecoderKeys<'_>, out: &mut String) {
    out.push_str("    public init(from decoder: any Decoder) throws {\n");
    if let Some(aliases) = keys.aliases {
        out.push_str(&aliases.preamble);
    } else {
        if keys.strict {
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_strict_unknown_keys.swift.jinja",
                crate::alef_context! { wire_key_type => keys.wire_key_type },
            ));
        }
        out.push_str("        let container = try decoder.container(keyedBy: CodingKeys.self)\n");
    }
}

pub(super) fn emit_decoder_init_with_aliases(
    mapper: &SwiftMapper,
    visible_fields: &[&FieldDef],
    zero_arg_constructible_names: &HashSet<String>,
    every_field_may_be_absent: bool,
    out: &mut String,
    keys: DecoderKeys<'_>,
) {
    emit_preamble(&keys, out);
    for field in visible_fields {
        let camel = swift_case_ident(&field.name.to_lower_camel_case());
        let key = keys.field(field, &camel);
        let already_optional = matches!(&field.ty, TypeRef::Optional(_));
        let is_optional = field.optional || already_optional;
        let swift_ty = swift_field_type(mapper, &field.ty);

        let literal = field.typed_default.as_ref().and_then(swift_typed_default_literal);

        if is_optional {
            let inner_ty = swift_ty.strip_suffix('?').unwrap_or(&swift_ty);
            // `Option<T>` defaulting to `Some(x)` reaches the IR as the bare literal for `x`, so
            // decoding an absent key as `nil` would substitute a value the source crate never
            // produces. Only a genuinely absent literal keeps the `?? nil` rendering. ~keep
            match &literal {
                Some(fb) => out.push_str(&crate::backends::swift::template_env::render(
                    "swift_decode_missing_optional_default_assignment.swift.jinja",
                    crate::alef_context! {
                        field => camel,
                        key => key,
                        ty => inner_ty,
                        fallback => fb,
                    },
                )),
                None => out.push_str(&crate::backends::swift::template_env::render(
                    "swift_decode_optional_assignment.swift.jinja",
                    crate::alef_context! {
                        field => camel,
                        key => key,
                        ty => inner_ty,
                    },
                )),
            }
            continue;
        }

        let fallback = non_optional_fallback(
            field,
            &swift_ty,
            zero_arg_constructible_names,
            every_field_may_be_absent,
        );

        match fallback {
            Some(fb) => {
                out.push_str(&crate::backends::swift::template_env::render(
                    "swift_decode_missing_default_assignment.swift.jinja",
                    crate::alef_context! {
                        field => camel,
                        key => key,
                        ty => swift_ty,
                        fallback => fb,
                    },
                ));
            }
            None => {
                out.push_str(&crate::backends::swift::template_env::render(
                    "swift_decode_required_assignment.swift.jinja",
                    crate::alef_context! {
                        field => camel,
                        key => key,
                        ty => swift_ty,
                    },
                ));
            }
        }
    }
    out.push_str("    }\n");
}
