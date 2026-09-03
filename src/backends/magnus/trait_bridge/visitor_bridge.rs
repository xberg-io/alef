use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, visitor_param_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

/// Generate a visitor-style bridge wrapping a Magnus `magnus::Value`.
///
/// Every trait method checks if the Ruby object responds to a snake_case method,
/// then calls it via `funcall` and maps the return value to the configured result enum.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    api: &ApiSurface,
) -> anyhow::Result<()> {
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Magnus,
    )?;
    let methods: Vec<String> =
        crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg)
            .into_iter()
            .map(|m| gen_visitor_method_magnus(m, bridge_cfg, type_paths, &result_metadata))
            .collect();

    let rendered = crate::backends::magnus::template_env::render(
        "visitor_bridge.rs.jinja",
        minijinja::context! {
            core_crate => core_crate,
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
            struct_name => struct_name,
            trait_path => trait_path,
            methods => methods,
            ctor_params => super::bridge_functions::BRIDGE_CTOR_PARAMS,
            ctor_return_type => super::bridge_functions::BRIDGE_CTOR_RETURN_TYPE,
        },
    );
    let debug_count = rendered.matches("impl std::fmt::Debug").count();
    if debug_count != 1 {
        tracing::warn!(
            "[ALEF BUG] visitor_bridge.rs.jinja rendered {} Debug impls (expected 1) for struct {}",
            debug_count,
            struct_name
        );
        tracing::warn!(
            "[ALEF BUG] Rendered output (first 2000 chars):\n{}",
            &rendered[..rendered.len().min(2000)]
        );
    }
    out.push_str(&rendered);
    out.push('\n');
    Ok(())
}

/// Generate a single visitor method that checks Ruby respond_to and calls via funcall.
fn gen_visitor_method_magnus(
    method: &MethodDef,
    bridge_cfg: &TraitBridgeConfig,
    type_paths: &std::collections::HashMap<String, String>,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) -> String {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let signature = sig_parts.join(", ");

    let return_type = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let has_args = !method.params.is_empty();
    let args_tuple = if has_args {
        let args_exprs: Vec<String> = method.params.iter().map(|p| build_magnus_arg(p, bridge_cfg)).collect();
        if args_exprs.len() == 1 {
            format!("({},)", args_exprs[0])
        } else {
            format!("({})", args_exprs.join(", "))
        }
    } else {
        String::new()
    };

    crate::backends::magnus::template_env::render(
        "visitor_method.rs.jinja",
        minijinja::context! {
            name => name,
            signature => signature,
            return_type => return_type,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&return_type, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &return_type,
                result_metadata,
                "s",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            has_args => has_args,
            args_tuple => args_tuple,
        },
    )
}

/// Build a single Magnus funcall arg expression for a visitor method parameter.
fn build_magnus_arg(p: &crate::core::ir::ParamDef, bridge_cfg: &TraitBridgeConfig) -> String {
    if let TypeRef::Named(n) = &p.ty
        && Some(n.as_str()) == bridge_cfg.context_type.as_deref()
    {
        return format!("nodecontext_to_rb_hash({}{})", if p.is_ref { "" } else { "&" }, p.name);
    }
    if p.optional && matches!(&p.ty, TypeRef::String) {
        return format!(
            "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; match {} {{ Some(s) => ruby.str_new(s).as_value(), None => ruby.qnil().as_value() }} }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new({}) }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!(
            "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new({}.as_str()) }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::Vec(_)) {
        let ruby = "unsafe { magnus::Ruby::get_unchecked() }";
        return format!(
            "{{ let arr = {ruby}.ary_new_capa({name}.len()); for item in {name} {{ let _ = arr.push(item.to_string()); }} arr }}",
            name = p.name,
        );
    }
    p.name.to_string()
}

#[cfg(test)]
mod tests {
    use super::gen_visitor_method_magnus;
    use crate::codegen::visitor_result::VisitorResultVariant;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::MethodDef;

    fn variant(name: &str, wire_name: &str, cfg: Option<&str>) -> VisitorResultVariant {
        VisitorResultVariant {
            name: name.to_string(),
            wire_name: wire_name.to_string(),
            code: 0,
            cfg: cfg.map(str::to_string),
        }
    }

    /// The regression this task fixes: `visitor_method.rs.jinja` referenced
    /// `{{ return_type }}::{{ variant.name }}` for every `unit_result_variants`/
    /// `payload_result_variants` entry with no `#[cfg(...)]` guard at all -- E0599 in a build
    /// excluding a gated variant's feature, once `codegen::visitor_result` started carrying
    /// `VisitorResultVariant::cfg`. A gated variant (already filtered to host-owned-only by
    /// `visitor_result_metadata_from_enum_checked`) must render with its guard, in both the
    /// payload (`if let`) and unit (`match` arm) positions.
    #[test]
    fn gated_variant_renders_cfg_guard_in_both_positions() {
        use crate::codegen::visitor_result::VisitorResultMetadata;

        let metadata = VisitorResultMetadata {
            default_variant: variant("Continue", "continue", None),
            unit_variants: vec![
                variant("Continue", "continue", None),
                variant("Thumbnail", "thumbnail", Some(r#"feature = "thumbnails""#)),
            ],
            string_payload_variants: vec![variant("Custom", "custom", Some(r#"feature = "thumbnails""#))],
        };
        let method = MethodDef {
            name: "visit".to_string(),
            ..Default::default()
        };
        let bridge_cfg = TraitBridgeConfig::default();
        let type_paths = std::collections::HashMap::new();

        let generated = gen_visitor_method_magnus(&method, &bridge_cfg, &type_paths, &metadata);

        assert_eq!(
            generated.matches("#[cfg(feature = \"thumbnails\")]").count(),
            2,
            "the gated unit variant's match arm AND the gated payload variant's if-let must \
             each carry the guard exactly once, got:\n{generated}"
        );
        assert!(
            generated.contains("Thumbnail"),
            "the gated unit variant must still be referenced, got:\n{generated}"
        );
        assert!(
            generated.contains("Custom"),
            "the gated payload variant must still be referenced, got:\n{generated}"
        );
    }

    /// Negative control: metadata with no gated variants renders no `#[cfg(...)]` at all.
    #[test]
    fn ungated_metadata_renders_no_cfg() {
        use crate::codegen::visitor_result::VisitorResultMetadata;

        let metadata = VisitorResultMetadata {
            default_variant: variant("Continue", "continue", None),
            unit_variants: vec![variant("Continue", "continue", None)],
            string_payload_variants: vec![],
        };
        let method = MethodDef {
            name: "visit".to_string(),
            ..Default::default()
        };
        let bridge_cfg = TraitBridgeConfig::default();
        let type_paths = std::collections::HashMap::new();

        let generated = gen_visitor_method_magnus(&method, &bridge_cfg, &type_paths, &metadata);

        assert!(
            !generated.contains("#[cfg("),
            "ungated metadata must not emit #[cfg(...)], got:\n{generated}"
        );
    }
}
