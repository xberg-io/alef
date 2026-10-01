mod enums;
mod structs;

#[allow(unused_imports)]
pub(crate) use enums::{
    enum_constant_entries, flat_field_name, gen_enum_constants, gen_external_enum_serde_impls, gen_flat_data_enum,
    gen_flat_data_enum_from_impls, gen_flat_data_enum_methods, is_labeled_string_enum, is_tagged_data_enum,
    is_untagged_data_enum, ty_references_untagged_data_enum,
};
#[allow(unused_imports)]
pub(crate) use structs::{
    gen_opaque_struct_methods_with_exclude, gen_php_struct, gen_struct_methods, php_constructor_optional_field_names,
    php_field_base_constructor_optional, ty_is_or_wraps_json,
};
pub use structs::{gen_struct_methods_with_exclude, is_php_prop_scalar, php_field_can_be_constructor_param};
