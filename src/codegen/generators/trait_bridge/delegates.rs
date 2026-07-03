use heck::ToPascalCase;

use super::{TraitBridgeGenerator, TraitBridgeSpec, trait_method_signature};
use crate::core::ir::{MethodDef, ReceiverKind};

/// Name of the per-method default delegate type, e.g.
/// `PyOcrBackendBridgeDefaultProcessDocument`.
pub fn default_delegate_name(spec: &TraitBridgeSpec, method: &MethodDef) -> String {
    format!("{}Default{}", spec.wrapper_name(), method.name.to_pascal_case())
}

/// The Rust-defaulted own methods this bridge forwards to the host: the generator
/// opted the method in via [`TraitBridgeGenerator::gen_method_presence_check`] and
/// the method's shape supports guard-and-delegate emission (`&self` receiver, owned
/// return). Methods excluded here keep the previous behavior — omitted from the
/// bridge impl, so the trait's default body always runs.
pub fn forwarded_defaulted_methods<'a>(
    spec: &TraitBridgeSpec<'a>,
    generator: &dyn TraitBridgeGenerator,
) -> Vec<&'a MethodDef> {
    spec.trait_def
        .methods
        .iter()
        .filter(|m| {
            m.trait_source.is_none()
                && m.has_default_impl
                && !m.returns_ref
                && matches!(m.receiver, Some(ReceiverKind::Ref))
                && generator.gen_method_presence_check(m, spec).is_some()
        })
        .collect()
}

/// Generate the per-method default delegates for every forwarded defaulted method.
///
/// For a defaulted method `x`, `WrapperDefaultX` implements the trait by forwarding
/// every own method EXCEPT `x` back through the bridge — so when the bridge falls
/// back to `WrapperDefaultX(self).x(...)`, the trait's genuine Rust default body for
/// `x` runs, and any `self.*` call inside that body still reaches host overrides.
/// The `Plugin` super-trait (when configured) is forwarded for the same reason.
///
/// Returns an empty string when the generator forwards no defaulted methods.
pub fn gen_bridge_default_delegates(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> String {
    let forwarded = forwarded_defaulted_methods(spec, generator);
    if forwarded.is_empty() {
        return String::new();
    }

    let wrapper = spec.wrapper_name();
    let trait_path = spec.trait_path();
    let super_trait_path = spec.bridge_config.super_trait.as_deref().map(|name| {
        if name.contains("::") {
            name.to_string()
        } else {
            format!("{}::{}", spec.core_import, name)
        }
    });
    let error_path = spec.error_path();

    let own_methods: Vec<&MethodDef> = spec
        .trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none())
        .collect();

    let mut out = String::with_capacity(2048);
    for (i, defaulted) in forwarded.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }

        // Forward every own method except the delegated one. Methods the bridge does
        // not emit at all (non-forwarded defaulted methods) are also skipped — the
        // delegate inherits their default bodies the same way the bridge does.
        let bridge_emitted: Vec<&&MethodDef> = own_methods
            .iter()
            .filter(|m| m.name != defaulted.name && (!m.has_default_impl || forwarded.iter().any(|f| f.name == m.name)))
            .collect();

        let methods: Vec<minijinja::Value> = bridge_emitted
            .iter()
            .map(|m| {
                let sig = trait_method_signature(m, spec);
                let args = if sig.arg_names.is_empty() {
                    "self.0".to_string()
                } else {
                    format!("self.0, {}", sig.arg_names)
                };
                let await_suffix = if m.is_async { ".await" } else { "" };
                minijinja::context! {
                    async_kw => sig.async_kw,
                    name => &m.name,
                    all_params => sig.all_params,
                    ret => sig.ret,
                    forward_call => format!("{trait_path}::{}({args}){await_suffix}", m.name),
                }
            })
            .collect();

        let has_async_methods = bridge_emitted.iter().any(|m| m.is_async);

        out.push_str(&crate::codegen::template_env::render(
            "generators/trait_bridge/default_delegate.jinja",
            minijinja::context! {
                delegate_name => default_delegate_name(spec, defaulted),
                method_name => &defaulted.name,
                wrapper_name => &wrapper,
                trait_path => &trait_path,
                super_trait_path => super_trait_path.as_deref(),
                error_path => &error_path,
                has_async_methods => has_async_methods,
                async_trait_is_send => generator.async_trait_is_send(),
                methods => methods,
            },
        ));
    }

    out
}
