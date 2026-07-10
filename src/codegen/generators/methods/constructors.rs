use crate::codegen::generators::RustBindingConfig;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::TypeDef;

/// Generate a constructor method.
pub fn gen_constructor(typ: &TypeDef, mapper: &dyn TypeMapper, cfg: &RustBindingConfig) -> String {
    gen_constructor_with_renames(typ, mapper, cfg, None)
}

/// Like `gen_constructor` but with field renames for keyword escaping.
pub fn gen_constructor_with_renames(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    field_renames: Option<&std::collections::HashMap<String, String>>,
) -> String {
    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);

    let (param_list, sig_defaults, assignments) = if typ.has_default {
        crate::codegen::shared::config_constructor_parts_with_renames_and_cfg_restore(
            &typ.fields,
            &map_fn,
            cfg.option_duration_on_defaults,
            field_renames,
            cfg.never_skip_cfg_field_names,
        )
    } else {
        crate::codegen::shared::constructor_parts_with_renames_and_cfg_restore(
            &typ.fields,
            &map_fn,
            field_renames,
            cfg.never_skip_cfg_field_names,
        )
    };

    crate::codegen::template_env::render(
        "generators/methods/constructor.jinja",
        minijinja::context! {
            has_too_many_args => typ.fields.len() > 7,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_defaults => sig_defaults,
            signature_suffix => cfg.signature_suffix,
            constructor_attr => cfg.constructor_attr,
            param_list => param_list,
            assignments => assignments,
        },
    )
}
