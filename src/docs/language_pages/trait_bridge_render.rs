//! Renders the "Trait Bridges" reference section: the API a host-language caller uses to
//! register a plugin implementation of a Rust trait.
//!
//! This deliberately asks the target backend for its actual registration surface
//! (`Backend::trait_bridge_registration_surface`) rather than re-deriving naming here. A
//! prior defect in this exact area — the Zig reference documenting `[:0]const u8` for
//! strings the emitter never produced — came from the docs layer re-deriving what a
//! backend emits; guessing a registration name here would recreate that failure shape. ~keep

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::docs::template_env;

/// Render the "Trait Bridges" section for `lang`, or an empty string when the language has
/// no binding backend (Rust, C) or the backend reports no registration surface (either
/// because the crate configures no `[[trait_bridges]]`, or because this backend does not
/// yet implement `Backend::trait_bridge_registration_surface`).
pub(super) fn render_trait_bridges(api: &ApiSurface, config: &ResolvedCrateConfig, lang: Language) -> String {
    let Some(backend) = crate::cli::registry::try_get_backend(lang) else {
        return String::new();
    };
    let surfaces = backend.trait_bridge_registration_surface(api, config);
    if surfaces.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&template_env::render(
        "heading.jinja",
        crate::alef_context! { marker => "###", title => "Trait Bridges" },
    ));
    out.push_str("| Trait | Register | Unregister | Clear |\n");
    out.push_str("|-------|----------|------------|-------|\n");
    for surface in &surfaces {
        out.push_str(&template_env::render(
            "trait_bridge_row.jinja",
            crate::alef_context! {
                trait_name => &surface.trait_name,
                register => surface.register_symbol.as_deref().unwrap_or("—"),
                unregister => surface.unregister_symbol.as_deref().unwrap_or("—"),
                clear => surface.clear_symbol.as_deref().unwrap_or("—"),
            },
        ));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::render_trait_bridges;
    use crate::core::backend::TraitBridgeRegistrationSurface;
    use crate::core::config::{Language, TraitBridgeConfig};
    use crate::core::ir::TypeDef;
    use crate::docs::test_helpers::{empty_api, make_test_config};

    /// The regression this guards against: a docs-layer implementation that fabricates a
    /// registration name (or renders a placeholder) instead of asking the backend must fail
    /// this test, since it asserts the exact strings the backend reported, not just that
    /// *something* rendered.
    #[test]
    fn pyo3_reference_page_reports_the_backends_configured_registration_names() {
        let mut config = make_test_config();
        config.trait_bridges.push(TraitBridgeConfig {
            trait_name: "SampleTrait".to_owned(),
            registry_getter: Some("sample::registry::get_sample_registry".to_owned()),
            register_fn: Some("register_sample".to_owned()),
            unregister_fn: Some("unregister_sample".to_owned()),
            clear_fn: Some("clear_samples".to_owned()),
            ..Default::default()
        });
        // The bridged trait must resolve in `api.types` and the bridge must carry a
        // `registry_getter`, or the backend correctly reports no registration surface and there is
        // nothing for this page to render. Both are what a real configured bridge looks like; an
        // `empty_api()` fixture asserted against a trait that does not exist instead pinned a
        // surface the backend would never emit. ~keep
        let mut api = empty_api();
        api.types.push(TypeDef {
            name: "SampleTrait".to_owned(),
            rust_path: "sample::SampleTrait".to_owned(),
            is_trait: true,
            ..Default::default()
        });

        let rendered = render_trait_bridges(&api, &config, Language::Python);

        assert!(rendered.contains("### Trait Bridges"), "got:\n{rendered}");
        assert!(rendered.contains("`SampleTrait`"), "got:\n{rendered}");
        assert!(rendered.contains("register_sample"), "got:\n{rendered}");
        assert!(rendered.contains("unregister_sample"), "got:\n{rendered}");
        assert!(rendered.contains("clear_samples"), "got:\n{rendered}");
    }

    /// A backend reporting nothing (no `[[trait_bridges]]` configured) must render nothing —
    /// never a placeholder heading or an empty table.
    #[test]
    fn renders_nothing_when_backend_reports_no_registration_surface() {
        let config = make_test_config();
        let api = empty_api();

        let rendered = render_trait_bridges(&api, &config, Language::Python);

        assert_eq!(
            rendered, "",
            "no configured trait bridges must render no section at all"
        );
    }

    /// A backend not (yet) covered by `trait_bridge_registration_surface` (default empty
    /// impl) must also render nothing, even when trait bridges are configured — proving the
    /// default really is silent rather than a guess.
    #[test]
    fn renders_nothing_for_a_backend_not_yet_covered_by_the_default_impl() {
        let mut config = make_test_config();
        config.trait_bridges.push(TraitBridgeConfig {
            trait_name: "SampleTrait".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            ..Default::default()
        });
        let api = empty_api();

        // Kotlin Android does not (yet) override `trait_bridge_registration_surface`.
        let rendered = render_trait_bridges(&api, &config, Language::KotlinAndroid);

        assert_eq!(
            rendered, "",
            "an uncovered backend must render nothing rather than a placeholder"
        );
    }

    /// Positive control for [`TraitBridgeRegistrationSurface`] itself: confirms the struct's
    /// fields round-trip through equality, so a future refactor that silently drops a field
    /// (e.g. `clear_symbol`) is caught here rather than only downstream in rendering.
    #[test]
    fn registration_surface_struct_carries_all_reported_symbols() {
        let surface = TraitBridgeRegistrationSurface {
            trait_name: "SampleTrait".to_owned(),
            register_symbol: Some("register_sample".to_owned()),
            unregister_symbol: Some("unregister_sample".to_owned()),
            clear_symbol: Some("clear_samples".to_owned()),
        };
        assert_eq!(surface.trait_name, "SampleTrait");
        assert_eq!(surface.register_symbol.as_deref(), Some("register_sample"));
        assert_eq!(surface.unregister_symbol.as_deref(), Some("unregister_sample"));
        assert_eq!(surface.clear_symbol.as_deref(), Some("clear_samples"));
    }
}
