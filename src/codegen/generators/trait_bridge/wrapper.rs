use crate::core::ir::MethodDef;

use super::{TraitBridgeGenerator, TraitBridgeSpec};

pub fn gen_bridge_wrapper_struct(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> String {
    let wrapper = spec.wrapper_name();
    let foreign_type = generator.foreign_object_type();

    let extra_fields: Vec<minijinja::Value> = generator
        .extra_bridge_fields(spec)
        .into_iter()
        .map(|(name, ty)| minijinja::context! { name => name, ty => ty })
        .collect();

    crate::codegen::template_env::render(
        "generators/trait_bridge/wrapper_struct.jinja",
        minijinja::context! {
            wrapper_prefix => spec.wrapper_prefix,
            trait_name => &spec.trait_def.name,
            wrapper_name => wrapper,
            foreign_type => foreign_type,
            extra_fields => extra_fields,
        },
    )
}

/// Generate `impl std::fmt::Debug for Wrapper`.
///
/// Required by trait bounds on `Plugin` super-trait (and many others) that
/// extend `Debug`. Without this, generic plugin-pattern bridges fail to
/// compile when the user's trait has a `Debug` super-trait bound.
pub fn gen_bridge_debug_impl(spec: &TraitBridgeSpec) -> String {
    let wrapper = spec.wrapper_name();
    crate::codegen::template_env::render(
        "generators/trait_bridge/debug_impl.jinja",
        minijinja::context! {
            wrapper_name => wrapper,
        },
    )
}

/// Generate `impl SuperTrait for Wrapper` when the bridge config specifies a super-trait.
///
/// Forwards `name()`, `version()`, `initialize()`, and `shutdown()` to the
/// foreign object, using `cached_name` for `name()`.
///
/// The super-trait path is derived from the config's `super_trait` field. If it
/// contains `::`, it's used as-is; otherwise it's qualified as `{core_import}::{super_trait}`.
pub fn gen_bridge_plugin_impl(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    let super_trait_name = spec.bridge_config.super_trait.as_deref()?;

    let wrapper = spec.wrapper_name();
    let core_import = spec.core_import;

    // Derive the fully-qualified super-trait path
    let super_trait_path = if super_trait_name.contains("::") {
        super_trait_name.to_string()
    } else {
        format!("{core_import}::{super_trait_name}")
    };

    // Build synthetic MethodDefs for the Plugin methods and delegate to the generator
    // for the actual call bodies. The Plugin trait interface is well-known: name(),
    // version(), initialize(), shutdown().
    let error_path = spec.error_path();

    // version() -> String — delegate to foreign object
    let version_method = MethodDef {
        name: "version".to_string(),
        params: vec![],
        return_type: crate::core::ir::TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let version_body = generator.gen_sync_method_body(&version_method, spec);

    // initialize() -> Result<(), ErrorType>
    let init_method = MethodDef {
        name: "initialize".to_string(),
        params: vec![],
        return_type: crate::core::ir::TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some(error_path.clone()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut init_body = generator.gen_sync_method_body(&init_method, spec);
    // A host that doesn't define initialize() opted out of the lifecycle hook —
    // treat it as a no-op instead of failing registration.
    if let Some(presence) = generator.gen_lifecycle_presence_check(&init_method, spec) {
        init_body = format!("if !({presence}) {{\n    return Ok(());\n}}\n{init_body}");
    }

    // shutdown() -> Result<(), ErrorType>
    let shutdown_method = MethodDef {
        name: "shutdown".to_string(),
        params: vec![],
        return_type: crate::core::ir::TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some(error_path.clone()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut shutdown_body = generator.gen_sync_method_body(&shutdown_method, spec);
    // Same no-op tolerance for shutdown() at unregistration.
    if let Some(presence) = generator.gen_lifecycle_presence_check(&shutdown_method, spec) {
        shutdown_body = format!("if !({presence}) {{\n    return Ok(());\n}}\n{shutdown_body}");
    }

    // Split method bodies into lines for template iteration
    let version_lines: Vec<&str> = version_body.lines().collect();
    let init_lines: Vec<&str> = init_body.lines().collect();
    let shutdown_lines: Vec<&str> = shutdown_body.lines().collect();

    Some(crate::codegen::template_env::render(
        "generators/trait_bridge/plugin_impl.jinja",
        minijinja::context! {
            super_trait_path => super_trait_path,
            wrapper_name => wrapper,
            error_path => error_path,
            version_lines => version_lines,
            init_lines => init_lines,
            shutdown_lines => shutdown_lines,
        },
    ))
}
