mod fields;
mod render;
mod wrappers;

pub use fields::{field_conversion_to_core, field_conversion_to_core_cfg};
pub use render::{gen_from_binding_to_core, gen_from_binding_to_core_cfg, gen_from_lifetime_type_constructor};
pub use wrappers::apply_core_wrapper_to_core;

#[cfg(test)]
mod field_default_tests;
#[cfg(test)]
mod legacy_newtype_tests;
#[cfg(test)]
mod tests;
