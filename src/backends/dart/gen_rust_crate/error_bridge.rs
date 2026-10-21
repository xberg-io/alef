use crate::codegen::naming::{PublicIdentifierKind, internal_rust_identifier, public_host_identifier};
use crate::core::config::Language;
use crate::core::ir::{ErrorDef, ErrorVariant, FieldDef, TypeRef};

pub(super) fn uses_typed_error(error_type: Option<&str>, errors: &[ErrorDef]) -> bool {
    error_type.is_some_and(|name| {
        errors.iter().any(|error| {
            !error.binding_excluded
                && (error.rust_path == name || error.original_rust_path == name || error.name == name)
        })
    })
}

pub(super) fn result_error_type(error_type: Option<&str>, errors: &[ErrorDef]) -> &'static str {
    if uses_typed_error(error_type, errors) {
        "AlefBridgeError"
    } else {
        "String"
    }
}

pub(super) fn error_mapper(error_type: Option<&str>, errors: &[ErrorDef]) -> &'static str {
    if uses_typed_error(error_type, errors) {
        "AlefBridgeError::from"
    } else {
        "|e| e.to_string()"
    }
}

pub(super) fn emit(out: &mut String, errors: &[ErrorDef], source_crate: &str) {
    let errors = errors
        .iter()
        .filter(|error| !error.binding_excluded)
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return;
    }
    let invalid_input = carrier_name("InvalidInput", &errors);
    let native_failure = carrier_name("NativeFailure", &errors);
    let mut decode_arms = Vec::new();
    let mut encode_arms = Vec::new();
    let entries = errors.iter().map(|error| {
        let dart_name = public_host_identifier(Language::Dart, PublicIdentifierKind::Type, &error.name);
        let dart_factory = public_host_identifier(Language::Dart, PublicIdentifierKind::Method, &error.name);
        decode_arms.push(format!("AlefBridgeErrorWire_{dart_name}(:final error) => error"));
        encode_arms.push(format!("{dart_name} error => AlefBridgeErrorWire.{dart_factory}(error: error)"));
        crate::alef_context! {
            name => &error.name,
            core_path => if error.rust_path.is_empty() { format!("{source_crate}::{}", error.name) } else { error.rust_path.replace('-', "_") },
            needs_message => error.variants.iter().any(|variant| variant.is_tuple && variant.fields.iter().all(|field| field.binding_excluded)),
            arms => error.variants.iter().map(|variant| variant_arm(error, variant, source_crate)).collect::<Vec<_>>(),
        }
    }).collect::<Vec<_>>();
    for (carrier, exception, message) in [
        (&invalid_input, "ArgumentError", "error.message.toString()"),
        (&native_failure, "StateError", "error.message"),
    ] {
        let dart_name = public_host_identifier(Language::Dart, PublicIdentifierKind::Type, carrier);
        let dart_factory = public_host_identifier(Language::Dart, PublicIdentifierKind::Method, carrier);
        decode_arms.push(format!(
            "AlefBridgeErrorWire_{dart_name}(:final message) => {exception}(message)"
        ));
        encode_arms.push(format!(
            "{exception} error => AlefBridgeErrorWire.{dart_factory}(message: {message})"
        ));
    }
    encode_arms.push("_ => throw ArgumentError.value({}, 'error', 'Unsupported bridge error')".into());
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_typed_error_bridge.jinja",
        crate::alef_context! {
            errors => entries,
            invalid_input => invalid_input,
            native_failure => native_failure,
            decode_code => format!("{:?}", format!("switch ({{}}) {{ {} }}", decode_arms.join(", "))),
            encode_code => format!("{:?}", format!("switch ({{}}) {{ {} }}", encode_arms.join(", "))),
        },
    ));
}

fn carrier_name(preferred: &str, errors: &[&ErrorDef]) -> String {
    let mut name = preferred.to_string();
    let mut suffix = 0;
    while errors.iter().any(|error| {
        public_host_identifier(Language::Dart, PublicIdentifierKind::Type, &error.name)
            == public_host_identifier(Language::Dart, PublicIdentifierKind::Type, &name)
    }) {
        suffix += 1;
        name = format!("{preferred}{suffix}");
    }
    name
}

fn variant_arm(error: &ErrorDef, variant: &ErrorVariant, source_crate: &str) -> minijinja::Value {
    let core_path = if error.rust_path.is_empty() {
        format!("{source_crate}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };
    crate::alef_context! {
        pattern => variant_pattern(&core_path, variant),
        variant => &variant.name,
        fields => variant_fields(variant, source_crate),
    }
}

fn variant_pattern(core_path: &str, variant: &ErrorVariant) -> String {
    if variant.is_unit {
        return format!("{core_path}::{}", variant.name);
    }
    if variant.is_tuple {
        let bindings = variant
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                if field.binding_excluded {
                    "_".to_string()
                } else {
                    format!("field{index}")
                }
            })
            .collect::<Vec<_>>();
        return format!("{core_path}::{}({})", variant.name, bindings.join(", "));
    }
    let bindings = variant
        .fields
        .iter()
        .filter(|field| !field.binding_excluded)
        .map(|field| internal_rust_identifier(&field.name))
        .collect::<Vec<_>>();
    let bindings = if bindings.is_empty() {
        String::new()
    } else {
        format!("{},", bindings.join(", "))
    };
    format!("{core_path}::{} {{ {bindings} .. }}", variant.name)
}

fn variant_fields(variant: &ErrorVariant, source_crate: &str) -> Vec<minijinja::Value> {
    let mut fields = variant.fields.iter().enumerate().filter(|(_, field)| !field.binding_excluded)
        .enumerate().map(|(visible_index, (index, field))| {
            let binding = if variant.is_tuple { format!("field{index}") } else { internal_rust_identifier(&field.name) };
            crate::alef_context! {
                name => if variant.is_tuple { format!("field{visible_index}") } else { internal_rust_identifier(&field.name) },
                value => field_expression(&binding, field, source_crate),
            }
        }).collect::<Vec<_>>();
    if variant.is_tuple && fields.is_empty() {
        fields.push(crate::alef_context! { name => "field0", value => "message" });
    }
    fields
}

fn field_expression(binding: &str, field: &FieldDef, source_crate: &str) -> String {
    if let TypeRef::Optional(inner_ty) = &field.ty {
        let inner = FieldDef {
            ty: *inner_ty.clone(),
            optional: false,
            ..field.clone()
        };
        let flatten = if field.optional { ".flatten()" } else { "" };
        return format!(
            "{binding}{flatten}.map(|value| {})",
            field_expression("value", &inner, source_crate)
        );
    }
    if field.optional {
        let inner = FieldDef {
            optional: false,
            ..field.clone()
        };
        return format!(
            "{binding}.map(|value| {})",
            field_expression("value", &inner, source_crate)
        );
    }
    if let TypeRef::Duration = field.ty {
        return format!("{binding}.as_millis() as i64");
    }
    if field.sanitized {
        return format!("{binding}.to_string()");
    }
    if field.ty == TypeRef::String {
        return format!("{binding}.into()");
    }
    super::enum_fields::enum_variant_field_conv(binding, field, source_crate)
}
