use super::resolver_reuse_tests::{config, enums, functions, rich_types};
use super::*;
use crate::e2e::field_access::metadata_storage_tests::assert_shared_maps;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing_subscriber::prelude::*;

#[derive(Clone, Default)]
pub(super) struct FactCounts {
    pub(super) counts: Arc<Mutex<HashMap<String, usize>>>,
    pub(super) outside_presentation: Arc<Mutex<HashMap<String, usize>>>,
    pub(super) call_body: Arc<Mutex<HashMap<String, usize>>>,
    pub(super) entries: Arc<Mutex<HashMap<String, u64>>>,
    pub(super) presentation_only: bool,
}

impl FactCounts {
    pub(super) fn observe<T>(&self, operation: impl FnOnce() -> T) -> T {
        // tracing-core's JustOne fast path reads the registering thread's default. Keep a second
        // registered dispatcher alive so another thread's NoSubscriber cannot disable our callsites. ~keep
        let _interest_guard = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let subscriber = tracing_subscriber::registry().with(self.clone());
        tracing::subscriber::with_default(subscriber, operation)
    }

    pub(super) fn related_counts(&self) -> HashMap<String, usize> {
        let mut counts = self.counts.lock().unwrap().clone();
        for (name, count) in self.outside_presentation.lock().unwrap().iter() {
            *counts.entry(name.clone()).or_default() += count;
        }
        counts
    }
}

#[derive(Default)]
struct FactName {
    name: Option<String>,
    entries: Option<u64>,
}

impl tracing::field::Visit for FactName {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "ir_metadata_fact" {
            self.name = Some(value.to_owned());
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == "metadata_entries" {
            self.entries = Some(value);
        }
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl<S> tracing_subscriber::Layer<S> for FactCounts
where
    S: tracing::Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, context: tracing_subscriber::layer::Context<'_, S>) {
        let mut fact = FactName::default();
        event.record(&mut fact);
        let Some(name) = fact.name else { return };
        if let Some(entries) = fact.entries {
            self.entries.lock().unwrap().insert(name.clone(), entries);
        }
        let mut presentation = !self.presentation_only;
        let mut call_body = false;
        if let Some(scope) = context.event_scope(event) {
            for span in scope.from_root() {
                presentation |= span.metadata().name() == "snippet_presentation";
                call_body |= span.metadata().name() == "snippet_call_body";
            }
        }
        let bucket = if presentation {
            &self.counts
        } else if call_body {
            &self.call_body
        } else {
            &self.outside_presentation
        };
        *bucket.lock().unwrap().entry(name).or_default() += 1;
    }
}

#[test]
fn repeated_resolver_requests_reuse_the_same_ir_map_storage() {
    let types = rich_types();
    let enums = enums();
    let functions = functions();
    let config = config();
    let factory = ResolverFactory::new(&types, &enums, &functions);
    let first = factory.build(&config, &config.call, "rust");
    let second = factory.build(&config, &config.call, "rust");
    assert!(first.is_optional("title"));
    assert!(first.is_array("items"));
    assert_shared_maps(&first, &second);
}

#[test]
fn element_anchoring_shares_facts_without_changing_the_original_root() {
    let types = rich_types();
    let enums = enums();
    let functions = functions();
    let config = config();
    let factory = ResolverFactory::new(&types, &enums, &functions);
    let result = factory.build(&config, &config.call, "rust");
    let item = resolver_anchored_at_element(&result, "items");
    assert!(result.is_optional("title"));
    assert!(!item.is_optional("title"));
    assert_shared_maps(&result, &item);
}

#[test]
fn actual_fact_constructors_run_once_per_model_and_optionality_rule() {
    let counts = FactCounts::default();
    counts.observe(|| {
        let types = rich_types();
        let enums = enums();
        let functions = functions();
        let config = config();
        let factory = ResolverFactory::new(&types, &enums, &functions);
        for language in ["rust", "python", "node", "typescript"] {
            let resolver = factory.build(&config, &config.call, language);
            assert!(resolver.is_array("items"));
        }
    });
    let actual = counts.counts.lock().unwrap();
    assert_eq!(actual.len(), 4, "all four actual construction paths must emit evidence");
    assert_eq!(
        *actual,
        HashMap::from([
            ("fields".into(), 1),
            ("enum".into(), 1),
            ("collection".into(), 1),
            ("result".into(), 2)
        ])
    );
}
