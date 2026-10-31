macro_rules! forward_snippet_rendering {
    ($renderer:expr) => {
        fn render_snippet_body_with_functions(
            &self,
            fixture: &$crate::e2e::fixture::Fixture,
            e2e_config: &$crate::e2e::config::E2eConfig,
            config: &$crate::core::config::ResolvedCrateConfig,
            type_defs: &[$crate::core::ir::TypeDef],
            enums: &[$crate::core::ir::EnumDef],
            functions: &[$crate::core::ir::FunctionDef],
            errors: &[$crate::core::ir::ErrorDef],
        ) -> anyhow::Result<String> {
            let session = $crate::e2e::codegen::SnippetRenderSession::new(type_defs, enums, functions, errors);
            ($renderer)(fixture, e2e_config, config, &session)
        }

        fn render_snippet_body_in_session(
            &self,
            fixture: &$crate::e2e::fixture::Fixture,
            e2e_config: &$crate::e2e::config::E2eConfig,
            config: &$crate::core::config::ResolvedCrateConfig,
            session: &$crate::e2e::codegen::SnippetRenderSession<'_>,
        ) -> anyhow::Result<String> {
            ($renderer)(fixture, e2e_config, config, session)
        }
    };
}

macro_rules! one_shot_snippet_renderer {
    ($visibility:vis $name:ident -> $result:ty, $renderer:path) => {
        $visibility fn $name(
            fixture: &$crate::e2e::fixture::Fixture,
            e2e_config: &$crate::e2e::config::E2eConfig,
            config: &$crate::core::config::ResolvedCrateConfig,
            type_defs: &[$crate::core::ir::TypeDef],
            enums: &[$crate::core::ir::EnumDef],
            functions: &[$crate::core::ir::FunctionDef],
        ) -> $result {
            let session = $crate::e2e::codegen::SnippetRenderSession::new(type_defs, enums, functions, &[]);
            $renderer(fixture, e2e_config, config, &session)
        }
    };
}
