use super::types::{IrCollectionMap, IrEnumMap, IrResultFieldMap, SwiftFirstClassMap};
use std::ops::Deref;
use std::sync::Arc;

pub(super) trait MetadataView<T> {
    fn data(&self) -> &T;
    fn root(&self) -> Option<&str>;
}

#[derive(Clone)]
pub(super) struct AnchoredMap<T> {
    facts: Arc<T>,
    pub(super) root_type: Option<String>,
}

impl<T: Default> Default for AnchoredMap<T> {
    fn default() -> Self {
        Self {
            facts: Arc::default(),
            root_type: None,
        }
    }
}

impl<T> AnchoredMap<T> {
    pub(super) fn new(facts: Arc<T>, root_type: Option<String>) -> Self {
        Self { facts, root_type }
    }
}

impl<T> Deref for AnchoredMap<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.facts
    }
}

impl<T> MetadataView<T> for AnchoredMap<T> {
    fn data(&self) -> &T {
        &self.facts
    }

    fn root(&self) -> Option<&str> {
        self.root_type.as_deref()
    }
}

macro_rules! owned_metadata_view {
    ($($map:ty),+ $(,)?) => {
        $(impl MetadataView<$map> for $map {
            fn data(&self) -> &$map { self }
            fn root(&self) -> Option<&str> { self.root_type.as_deref() }
        })+
    };
}

owned_metadata_view!(IrEnumMap, IrCollectionMap, IrResultFieldMap, SwiftFirstClassMap);
