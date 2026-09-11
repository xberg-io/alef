pub(super) static TEMPLATES: &[(&str, &str)] = &[
    ("enum_header.jinja", include_str!("../templates/enum_header.jinja")),
    (
        "error_enum_header.jinja",
        include_str!("../templates/error_enum_header.jinja"),
    ),
    ("error_case.jinja", include_str!("../templates/error_case.jinja")),
    (
        "error_case_with_data.jinja",
        include_str!("../templates/error_case_with_data.jinja"),
    ),
    (
        "enum_unit_header.jinja",
        include_str!("../templates/enum_unit_header.jinja"),
    ),
    (
        "enum_unit_variant.jinja",
        include_str!("../templates/enum_unit_variant.jinja"),
    ),
    (
        "enum_case_unit.jinja",
        include_str!("../templates/enum_case_unit.jinja"),
    ),
    (
        "enum_case_with_data.jinja",
        include_str!("../templates/enum_case_with_data.jinja"),
    ),
    (
        "enum_from_impl_header.jinja",
        include_str!("../templates/enum_from_impl_header.jinja"),
    ),
    (
        "enum_from_variant.jinja",
        include_str!("../templates/enum_from_variant.jinja"),
    ),
];
