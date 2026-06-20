mod analysis;
mod async_body;
mod call_args;
mod let_bindings;
mod lossy_fields;
mod returns;
mod unimplemented;

pub use analysis::{has_named_params, is_simple_non_opaque_param};
pub use async_body::gen_async_body;
pub use call_args::{
    gen_call_args, gen_call_args_cfg, gen_call_args_with_let_bindings, gen_call_args_with_let_bindings_json_str,
    gen_call_args_with_let_bindings_mutex, gen_call_args_with_let_bindings_mutex_json_str,
};
pub(in crate::codegen::generators) use let_bindings::{gen_named_let_bindings, gen_named_let_bindings_by_ref};
pub use let_bindings::{
    gen_named_let_bindings_no_promote, gen_named_let_bindings_pub, gen_named_let_bindings_with_augmented,
    gen_serde_let_bindings,
};
pub use lossy_fields::{gen_lossy_binding_to_core_fields, gen_lossy_binding_to_core_fields_mut};
pub use returns::{
    apply_return_newtype_unwrap, primitive_return_cast_suffix, wrap_return, wrap_return_with_mutex,
    wrap_return_with_mutex_mapped,
};
pub use unimplemented::gen_unimplemented_body;
