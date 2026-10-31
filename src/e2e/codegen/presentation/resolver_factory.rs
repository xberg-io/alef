use super::*;
use crate::e2e::field_access::{
    IrCollectionMap, IrEnumMap, IrResultFieldMap, OptionalityRule, SharedIrFields, VariantAccessorMap,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

struct CrateFacts {
    fields: SharedIrFields,
    enums: Arc<IrEnumMap>,
    collections: Arc<IrCollectionMap>,
}

#[derive(Clone, Default)]
struct CallFacts {
    root: Option<String>,
    native_bytes: bool,
}

pub(crate) enum VariantAccessorBackend {
    CSharp,
    Dart,
}

/// Shared immutable IR metadata for one documentation-snippet render run.
///
/// A session borrows one model; create a new session for a different model or render run.
pub struct SnippetRenderSession<'a> {
    factory: ResolverFactory<'a>,
    errors: &'a [crate::core::ir::ErrorDef],
    csharp_variants: OnceLock<Arc<VariantAccessorMap>>,
    dart_variants: OnceLock<Arc<VariantAccessorMap>>,
}

impl<'a> SnippetRenderSession<'a> {
    /// Prepare a session without eagerly building metadata for unused snippet backends.
    pub fn new(
        types: &'a [crate::core::ir::TypeDef],
        enums: &'a [crate::core::ir::EnumDef],
        functions: &'a [crate::core::ir::FunctionDef],
        errors: &'a [crate::core::ir::ErrorDef],
    ) -> Self {
        Self {
            factory: ResolverFactory::new(types, enums, functions),
            errors,
            csharp_variants: OnceLock::new(),
            dart_variants: OnceLock::new(),
        }
    }

    /// The immutable type definitions supplied when this run began.
    pub fn type_defs(&self) -> &[crate::core::ir::TypeDef] {
        self.factory.types
    }

    /// The immutable enum definitions supplied when this run began.
    pub fn enums(&self) -> &[crate::core::ir::EnumDef] {
        self.factory.enums
    }

    /// The free-function registry used to resolve each call's own result type.
    pub fn functions(&self) -> &[crate::core::ir::FunctionDef] {
        self.factory.functions
    }

    /// The error registry retained for functions-aware custom snippet renderers.
    pub fn errors(&self) -> &[crate::core::ir::ErrorDef] {
        self.errors
    }

    pub(crate) fn apply_derived_shows(&self, fixture: &mut Fixture, config: &E2eConfig, language: &str) {
        let _span = tracing::trace_span!("snippet_presentation").entered();
        super::derived_shows::apply_derived_shows(fixture, config, language, &self.factory);
    }

    pub(crate) fn enrich_variant_fields(
        &self,
        resolver: FieldResolver,
        backend: VariantAccessorBackend,
        build: impl FnOnce(&[crate::core::ir::EnumDef]) -> VariantAccessorMap,
    ) -> FieldResolver {
        let _span = tracing::trace_span!("snippet_presentation").entered();
        let cache = match backend {
            VariantAccessorBackend::CSharp => &self.csharp_variants,
            VariantAccessorBackend::Dart => &self.dart_variants,
        };
        let variants = cache.get_or_init(|| Arc::new(build(self.enums())));
        resolver
            .with_shared_ir_fields(&self.factory.crate_facts().fields)
            .with_shared_variant_accessors(Arc::clone(variants))
    }

    pub(crate) fn resolve(&self, fixture: &Fixture, config: &E2eConfig, language: &str) -> Vec<PresentationOperation> {
        let _span = tracing::trace_span!("snippet_presentation").entered();
        resolve_from_factory(fixture, config, language, &self.factory)
    }

    pub(crate) fn resolve_with(
        &self,
        fixture: &Fixture,
        config: &E2eConfig,
        language: &str,
        resolver: &FieldResolver,
    ) -> Vec<PresentationOperation> {
        let _span = tracing::trace_span!("snippet_presentation").entered();
        resolve_with_factory(fixture, config, language, resolver, &self.factory)
    }
}

pub(super) struct ResolverFactory<'a> {
    pub(super) types: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    pub(super) functions: &'a [crate::core::ir::FunctionDef],
    facts: OnceLock<CrateFacts>,
    result_fields: Mutex<HashMap<OptionalityRule, Arc<IrResultFieldMap>>>,
    calls: Mutex<HashMap<String, CallFacts>>,
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
            facts: OnceLock::new(),
            result_fields: Mutex::new(HashMap::new()),
            calls: Mutex::new(HashMap::new()),
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
        let resolver = FieldResolver::new(
            config.effective_fields(call),
            config.effective_fields_optional(call),
            config.effective_result_fields(call),
            config.effective_fields_array(call),
            config.effective_fields_method_calls(call),
        )
        .with_shared_ir_fields(&self.crate_facts().fields);
        self.anchor_shared(resolver, call, language)
    }

    pub(super) fn anchor(
        &self,
        resolver: FieldResolver,
        call: &crate::core::config::e2e::CallConfig,
        language: &str,
    ) -> FieldResolver {
        #[cfg(test)]
        self.root_constructions.set(self.root_constructions.get() + 1);
        self.anchor_shared(resolver, call, language)
    }

    fn crate_facts(&self) -> &CrateFacts {
        self.facts.get_or_init(|| CrateFacts {
            fields: SharedIrFields::new(self.types),
            enums: Arc::new(FieldResolver::ir_enum_fields(self.types, self.enums)),
            collections: Arc::new(FieldResolver::ir_collection_fields(self.types)),
        })
    }

    fn call_facts(&self, call: &crate::core::config::e2e::CallConfig, language: &str) -> CallFacts {
        let Some(name) = call.core_lookup_name(language) else {
            return CallFacts::default();
        };
        let mut calls = self.calls.lock().expect("presentation call metadata lock poisoned");
        if let Some(facts) = calls.get(name.as_ref()) {
            return facts.clone();
        }
        let ir = crate::e2e::codegen::call_ir::CallIr {
            functions: self.functions,
            type_defs: self.types,
        };
        let signature = ir.signature(&name);
        let facts = CallFacts {
            root: signature.as_ref().and_then(|signature| {
                crate::e2e::codegen::call_ir::named_type(signature.return_type).map(str::to_owned)
            }),
            native_bytes: signature
                .is_some_and(|signature| matches!(signature.return_type, crate::core::ir::TypeRef::Bytes)),
        };
        calls.insert(name.into_owned(), facts.clone());
        facts
    }

    pub(super) fn returns_native_bytes(&self, call: &crate::core::config::e2e::CallConfig, language: &str) -> bool {
        self.call_facts(call, language).native_bytes
    }

    fn anchor_shared(
        &self,
        resolver: FieldResolver,
        call: &crate::core::config::e2e::CallConfig,
        language: &str,
    ) -> FieldResolver {
        let facts = self.crate_facts();
        let mut results = self
            .result_fields
            .lock()
            .expect("presentation result metadata lock poisoned");
        let result = results
            .entry(OptionalityRule::for_language(language))
            .or_insert_with(|| Arc::new(FieldResolver::ir_result_field_facts(self.types, language)));
        resolver.with_shared_ir_maps(
            Arc::clone(&facts.enums),
            Arc::clone(&facts.collections),
            Arc::clone(result),
            self.call_facts(call, language).root,
        )
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
    if factory.call_facts(call, language).native_bytes {
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
