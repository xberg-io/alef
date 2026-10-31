use super::*;

pub(super) struct ResolverFactory<'a> {
    pub(super) types: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    pub(super) functions: &'a [crate::core::ir::FunctionDef],
    #[cfg(test)]
    pub(super) root_constructions: std::cell::Cell<usize>,
}

impl<'a> ResolverFactory<'a> {
    pub(super) fn new(
        types: &'a [crate::core::ir::TypeDef],
        enums: &'a [crate::core::ir::EnumDef],
        functions: &'a [crate::core::ir::FunctionDef],
    ) -> Self {
        Self {
            types,
            enums,
            functions,
            #[cfg(test)]
            root_constructions: std::cell::Cell::new(0),
        }
    }

    pub(super) fn build(
        &self,
        config: &E2eConfig,
        call: &crate::core::config::e2e::CallConfig,
        language: &str,
    ) -> FieldResolver {
        #[cfg(test)]
        self.root_constructions.set(self.root_constructions.get() + 1);
        build_resolver(config, call, language, self.types, self.enums, self.functions)
    }

    pub(super) fn anchor(
        &self,
        resolver: FieldResolver,
        call: &crate::core::config::e2e::CallConfig,
        language: &str,
    ) -> FieldResolver {
        #[cfg(test)]
        self.root_constructions.set(self.root_constructions.get() + 1);
        anchor_to_declared_result_type(resolver, call, language, self.types, self.enums, self.functions)
    }
}

pub(super) fn resolve_from_factory(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    factory: &ResolverFactory<'_>,
) -> Vec<PresentationOperation> {
    if fixture.docs.is_none() {
        return Vec::new();
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if returns_native_bytes(call, language, factory.types, factory.functions) {
        return Vec::new();
    }
    let resolver = factory.build(e2e_config, call, language);
    resolve_anchored(fixture, e2e_config, language, &resolver, factory)
}

pub(super) fn resolve_with_factory(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    resolver: &FieldResolver,
    factory: &ResolverFactory<'_>,
) -> Vec<PresentationOperation> {
    let Some(_) = fixture.docs.as_ref() else {
        return Vec::new();
    };
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // The two callers that supply their own resolver (php's getter map, swift's first-class map)
    // build it from config alone, so the anchoring has to be applied here rather than at each
    // construction site -- one place decides what a snippet knows about its result type. ~keep
    let resolver = &factory.anchor(resolver.clone(), call, language);
    resolve_anchored(fixture, e2e_config, language, resolver, factory)
}
