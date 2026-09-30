use crate::codegen::naming::to_java_name;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::FunctionDef;
use heck::ToSnakeCase;

use super::super::helpers::safe_java_field_name;
use super::params_returns::param_type_name;

#[derive(Debug, Clone)]
pub(super) struct VisitorFunctionBridge {
    pub(super) options_param_java: String,
    pub(super) options_param_c: String,
    pub(super) options_type_handle: String,
    pub(super) options_field_java: String,
    pub(super) options_field_native: String,
    pub(super) internal_method_name: String,
}

pub(super) fn visitor_bridge_for_function(
    func: &FunctionDef,
    config: &ResolvedCrateConfig,
) -> Option<VisitorFunctionBridge> {
    config
        .trait_bridges_for(crate::core::config::Language::Java)
        .find_map(|bridge| visitor_bridge_for_trait_bridge(func, bridge))
}

fn visitor_bridge_for_trait_bridge(func: &FunctionDef, bridge: &TraitBridgeConfig) -> Option<VisitorFunctionBridge> {
    if bridge.bind_via != BridgeBinding::OptionsField {
        return None;
    }

    let options_type = bridge.options_type.as_deref()?;
    let options_field = bridge.resolved_options_field()?;
    let options_param = func
        .params
        .iter()
        .find(|param| param_type_name(param) == Some(options_type))?;
    let options_param_java = to_java_name(&options_param.name);

    Some(VisitorFunctionBridge {
        options_param_c: format!("c{options_param_java}"),
        options_param_java,
        options_type_handle: options_type.to_snake_case().to_uppercase(),
        options_field_java: safe_java_field_name(options_field),
        options_field_native: options_field.to_snake_case(),
        internal_method_name: format!("{}WithVisitorInternal", to_java_name(&func.name)),
    })
}
