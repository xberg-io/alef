use super::anchored_maps::AnchoredMap;
use super::types::{FieldResolver, IrCollectionMap, IrEnumMap, IrResultFieldMap};
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) struct SharedIrFields {
    reachable: Arc<HashSet<String>>,
    excluded: Arc<HashSet<String>>,
    optional: Arc<HashSet<String>>,
}

impl SharedIrFields {
    pub(crate) fn new(types: &[crate::core::ir::TypeDef]) -> Self {
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(types);
        Self {
            reachable: Arc::new(reachable),
            excluded: Arc::new(excluded),
            optional: Arc::new(optional),
        }
    }
}

impl FieldResolver {
    pub(crate) fn with_shared_ir_fields(mut self, fields: &SharedIrFields) -> Self {
        self.ir_reachable_fields = Arc::clone(&fields.reachable);
        self.ir_known_excluded_fields = Arc::clone(&fields.excluded);
        self.optional_fields.include_shared(Arc::clone(&fields.optional));
        self.warn_on_result_fields_contradicting_ir();
        self
    }

    pub(crate) fn with_shared_ir_maps(
        mut self,
        enums: Arc<IrEnumMap>,
        collections: Arc<IrCollectionMap>,
        fields: Arc<IrResultFieldMap>,
        root: Option<String>,
    ) -> Self {
        self.ir_enum_map = AnchoredMap::new(enums, root.clone());
        self.ir_collection_map = AnchoredMap::new(collections, root.clone());
        self.ir_result_field_map = AnchoredMap::new(fields, root);
        self
    }

    pub(crate) fn with_ir_root(mut self, root: Option<String>) -> Self {
        self.ir_enum_map.root_type = root.clone();
        self.ir_collection_map.root_type = root.clone();
        self.ir_result_field_map.root_type = root;
        self
    }
}
