use super::generator::Pyo3BridgeGenerator;
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::ir::MethodDef;
use std::collections::BTreeMap;

pub(super) struct HostAliasGenerator<'a> {
    pub base: &'a Pyo3BridgeGenerator,
    pub aliases: &'a BTreeMap<String, String>,
}

impl TraitBridgeGenerator for HostAliasGenerator<'_> {
    fn foreign_object_type(&self) -> &str {
        self.base.foreign_object_type()
    }

    fn bridge_imports(&self) -> Vec<String> {
        self.base.bridge_imports()
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        self.base
            .sync_body(method, spec, self.aliases.get(&method.name).map(String::as_str))
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        self.base.gen_async_method_body(method, spec)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        self.base.gen_constructor(spec)
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        self.base.gen_registration_fn(spec)
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        self.base.gen_unregistration_fn(spec)
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        self.base.gen_clear_fn(spec)
    }

    fn gen_method_presence_check(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> Option<String> {
        if self.aliases.contains_key(&method.name) {
            Some("true".into())
        } else {
            self.base.gen_method_presence_check(method, spec)
        }
    }

    fn gen_method_absence_check(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> Option<String> {
        if self.aliases.contains_key(&method.name) {
            None
        } else {
            self.base.gen_method_absence_check(method, spec)
        }
    }

    fn gen_lifecycle_presence_check(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> Option<String> {
        self.base.gen_lifecycle_presence_check(method, spec)
    }
}
