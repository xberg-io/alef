use std::collections::{BTreeMap, BTreeSet};

use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FieldDef, ReceiverKind, TypeRef};

pub(crate) const RUNTIME: &str = include_str!("../../../codegen/visitor_context_scope.rs");

/// The visitor context accessors Python reads through its callback-scoped lazy reader, keyed by
/// context type. The private-accessor promotion leaves these to the reader instead of carrying a
/// copy (alef ADR 0020).
pub(crate) fn lazy_readers(config: &ResolvedCrateConfig) -> &BTreeMap<String, String> {
    static NONE: BTreeMap<String, String> = BTreeMap::new();
    config
        .python
        .as_ref()
        .map_or(&NONE, |python| &python.visitor_context_accessors)
}

pub(crate) struct ScopedContextAccessor {
    method: String,
    context: String,
    fields: Vec<String>,
}

impl ScopedContextAccessor {
    pub(crate) fn resolve(
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
        bridges: &[TraitBridgeConfig],
        configured: &BTreeMap<String, String>,
    ) -> anyhow::Result<BTreeMap<String, Self>> {
        configured
            .iter()
            .map(|(context, method)| {
                let accessor = Self::new(api, config, bridges, context, method)?;
                Ok((context.clone(), accessor))
            })
            .collect()
    }

    fn new(
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
        bridges: &[TraitBridgeConfig],
        context: &str,
        method: &str,
    ) -> anyhow::Result<Self> {
        let matching: BTreeSet<_> = bridges
            .iter()
            .filter(|bridge| bridge.context_type.as_deref() == Some(context))
            .filter_map(|bridge| {
                super::active_bridge_trait(bridge, api).filter(|typ| super::is_visitor_bridge(typ, bridge))
            })
            .map(|typ| typ.name.as_str())
            .collect();
        anyhow::ensure!(
            matching.len() == 1,
            "Python context {context} must belong to exactly one visitor bridge"
        );
        let typ = api
            .types
            .iter()
            .find(|typ| typ.name == context)
            .ok_or_else(|| anyhow::anyhow!("Python context {context} is not in the API"))?;
        let valid = typ.methods.iter().any(|m| {
            m.name == method
                && !m.binding_excluded
                && !m.is_async
                && m.cfg.is_none()
                && m.receiver == Some(ReceiverKind::Ref)
                && m.params.is_empty()
                && m.error_type.is_none()
                && matches!(&m.return_type, TypeRef::Map(k, v) if **k == TypeRef::String && **v == TypeRef::String)
        });
        anyhow::ensure!(
            valid,
            "Python context accessor {context}.{method} must be a visible synchronous string-map getter"
        );
        anyhow::ensure!(
            !crate::codegen::shared::binding_fields(&typ.fields).any(|field| {
                crate::backends::pyo3::gen_bindings::python_visible_field_name(config, context, field) == method
            }),
            "Python context accessor {context}.{method} collides with a Python field getter"
        );
        let fields = crate::codegen::shared::binding_fields(&typ.fields)
            .map(|field| borrowed_field(api, config, context, field))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            context: context.to_string(),
            method: method.to_string(),
            fields,
        })
    }

    pub(crate) fn method(&self) -> &str {
        &self.method
    }

    pub(super) fn initializer(&self) -> String {
        format!("{} {{ {} }}", self.context, self.fields.join(", "))
    }

    fn scope(&self) -> String {
        format!("ALEF_VISITOR_SCOPE_{}", self.context.to_ascii_uppercase())
    }

    fn dispatch_name(&self) -> String {
        format!("alef_call_with_scoped_{}", self.context.to_ascii_lowercase())
    }

    pub(crate) fn declaration(&self, core_type: &str) -> String {
        let scope = self.scope();
        let dispatch = self.dispatch_name();
        let method = &self.method;
        format!(
            r#"
std::thread_local! {{
    static {scope}: alef_visitor_scope::CallbackAccessorScope<std::collections::HashMap<String, String>> =
        const {{ alef_visitor_scope::CallbackAccessorScope::new() }};
}}

fn {dispatch}<R>(object: &Bound<'_, PyAny>, ctx: &{core_type}, call: impl FnOnce() -> R) -> R {{
    alef_visitor_scope::CallbackAccessorScope::with(
        &{scope},
        object.as_ptr() as usize,
        || ctx.{method}().iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
        call,
    )
}}
"#
        )
    }

    pub(crate) fn method_source(&self, core_type: &str) -> String {
        let scope = self.scope();
        let method = &self.method;
        format!(
            r#"
    #[pyo3(signature = ())]
    pub fn {method}(slf: PyRef<'_, Self>) -> std::collections::HashMap<String, String> {{
        {scope}.with(|scope| scope.read(slf.as_ptr() as usize)).unwrap_or_else(|| {{
            let core_self = {core_type}::from((*slf).clone());
            core_self.{method}().iter().map(|(key, value)| (key.clone(), value.clone())).collect()
        }})
    }}
"#
        )
    }

    pub(super) fn call_in_scope(&self, arg: &str, borrow: &str, call: &str) -> String {
        let dispatch = self.dispatch_name();
        format!("{dispatch}(&{arg}, {borrow}, || {call})")
    }
}

fn borrowed_field(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    context: &str,
    field: &FieldDef,
) -> anyhow::Result<String> {
    let name = &field.name;
    let core_field = crate::core::keywords::rust_raw_ident(name);
    let source = format!("ctx.{core_field}");
    let binding_field = config
        .resolve_field_name(Language::Python, context, name)
        .unwrap_or_else(|| name.clone());
    let unsupported = || anyhow::anyhow!("Python context {context} has unsupported callback field {name}");
    anyhow::ensure!(
        field.cfg.is_none() && field.newtype_wrapper.is_none(),
        "{}",
        unsupported()
    );
    let expression = match &field.ty {
        TypeRef::String if field.optional => format!("{source}.as_deref().map(str::to_owned)"),
        TypeRef::String => format!("{source}.to_string()"),
        TypeRef::Optional(inner) if **inner == TypeRef::String => format!("{source}.as_deref().map(str::to_owned)"),
        TypeRef::Primitive(_) => source,
        TypeRef::Named(name) => {
            let enum_def = api
                .enums
                .iter()
                .find(|typ| typ.name == *name && typ.variants.iter().all(|v| v.fields.is_empty()))
                .ok_or_else(unsupported)?;
            if enum_def.is_copy {
                format!("{source}.into()")
            } else {
                format!("{source}.clone().into()")
            }
        }
        _ => return Err(unsupported()),
    };
    Ok(format!("{binding_field}: {expression}"))
}
