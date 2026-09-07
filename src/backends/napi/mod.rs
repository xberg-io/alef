//! Node.js (NAPI-RS) binding generator backend for alef.

mod gen_bindings;
pub(crate) mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::NapiBackend;
pub(crate) use gen_bindings::enums::is_json_passthrough_data_enum;
pub(crate) use gen_bindings::enums::is_tagged_data_enum;
pub(crate) use gen_bindings::enums::string_enum_variant_js_value;
pub(crate) use gen_bindings::enums::tagged_enum_binding_field_js_name;
pub(crate) use gen_bindings::enums::tagged_enum_discriminant_js_name;
/// Re-exported so the TypeScript e2e snippet generator's tests can typecheck a generated
/// snippet against the exact `.d.ts` union type this function produces, rather than a
/// hand-guessed copy of it. See `internal_tagged_union_dts_lines`'s doc comment for why this
/// matters. Test-only: nothing in the non-test binary calls it. ~keep
#[cfg(test)]
pub(crate) use gen_bindings::errors::internal_tagged_union_dts_lines;
/// Re-exported for the same reason as [`napi_field_is_optional`], for the other half of a call
/// shape: e2e snippet codegen decides "may this argument be omitted?" with the predicate this
/// backend's `.d.ts` writer emits from. See
/// [`gen_bindings::errors::napi_param_is_optional`]. ~keep
pub(crate) use gen_bindings::errors::napi_param_is_optional;
/// Re-exported so e2e snippet codegen decides "is this field optional in the Node binding?"
/// with the predicate this backend emits from, instead of a second copy that can drift. See
/// [`gen_bindings::types::napi_field_is_optional`]. ~keep
pub(crate) use gen_bindings::types::napi_field_is_optional;
