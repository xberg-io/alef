use std::collections::{HashMap, HashSet};

use super::collect_pyo3_variant_constructors;
use crate::codegen::naming::python_enum_payload_getter_name;
use crate::core::ir::{EnumDef, EnumVariant};

/// Reserve every factory and bare getter before allocating payload getter suffixes. ~keep
pub(crate) fn dict_getter_py_name(enum_def: &EnumDef, variant: &EnumVariant) -> String {
    let mut allocator = PayloadNameAllocator::new(enum_def);
    for item in &enum_def.variants {
        let name = allocator.allocate(item);
        if item.name == variant.name {
            return name;
        }
    }
    python_enum_payload_getter_name(&variant.name, false, &allocator.reserved)
}

pub(crate) fn dict_getter_py_names(enum_def: &EnumDef) -> Vec<String> {
    let mut allocator = PayloadNameAllocator::new(enum_def);
    // A single-name lookup returns its first match even when IR contains duplicate variant names. ~keep
    let mut first = HashMap::new();
    enum_def
        .variants
        .iter()
        .map(|variant| {
            let name = allocator.allocate(variant);
            first.entry(variant.name.as_str()).or_insert(name).clone()
        })
        .collect()
}

struct PayloadNameAllocator<'a> {
    fixed: HashSet<String>,
    bare_constructors: HashSet<&'a str>,
    reserved: HashSet<String>,
    allocated: HashSet<String>,
}

impl<'a> PayloadNameAllocator<'a> {
    fn new(enum_def: &'a EnumDef) -> Self {
        let constructors = collect_pyo3_variant_constructors(enum_def);
        let mut fixed: HashSet<String> = constructors.iter().map(|ctor| ctor.snake_name.clone()).collect();
        if let Some(tag) = &enum_def.serde_tag {
            fixed.insert(tag.clone());
        }
        let bare_constructors = constructors
            .iter()
            .filter(|ctor| !ctor.is_tuple)
            .map(|ctor| ctor.variant_name)
            .collect();
        let empty = HashSet::new();
        let mut reserved = fixed.clone();
        reserved.extend(
            enum_def
                .variants
                .iter()
                .map(|item| python_enum_payload_getter_name(&item.name, false, &empty)),
        );
        Self {
            fixed,
            bare_constructors,
            reserved,
            allocated: HashSet::new(),
        }
    }

    fn allocate(&mut self, variant: &EnumVariant) -> String {
        let bare = python_enum_payload_getter_name(&variant.name, false, &HashSet::new());
        if !self.fixed.contains(&bare) && !self.allocated.contains(&bare) {
            self.reserved.remove(&bare);
        }
        let name = python_enum_payload_getter_name(
            &variant.name,
            self.bare_constructors.contains(variant.name.as_str()),
            &self.reserved,
        );
        self.reserved.insert(name.clone());
        self.allocated.insert(name.clone());
        name
    }
}
