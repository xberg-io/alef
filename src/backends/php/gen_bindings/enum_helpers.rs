use crate::codegen::naming::wire_variant_value;

pub(super) fn sanitize_php_enum_case(name: &str) -> String {
    if name.eq_ignore_ascii_case("class") {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

pub(super) fn php_enum_case_value(
    enum_def: &crate::core::ir::EnumDef,
    variant: &crate::core::ir::EnumVariant,
) -> String {
    wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    )
}
