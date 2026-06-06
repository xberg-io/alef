/// Emit a Gleam test backend stub.
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> crate::e2e::codegen::TestBackendEmission {
    crate::e2e::codegen::TestBackendEmission::unimplemented("gleam")
}
