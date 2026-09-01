//! How the Dart binding spells a tagged-union narrowing, in the form the field resolver needs
//! to render a doc snippet that steps into a variant payload.
//!
//! ~keep Unlike C#, the authority here is not alef. The Dart binding is flutter_rust_bridge
//! output (see `backends::dart`), so `<Union>_<Variant>` is frb/freezed's own subclass naming
//! and alef renders nothing that could be read as the source of truth. The one place that
//! naming is already encoded and proven against a compiled consumer is the assertion emitter's
//! `narrow_tagged_union_expression`; this builds the same two facts so the snippet path and the
//! assertion path cannot spell the cast differently. The payload accessor goes through
//! `codegen::naming::dart_tuple_field_identifier`, which is what the binding generator itself
//! uses to name a tuple field, so `_0` becomes `field0` here for the same reason it does there.

use crate::codegen::naming::dart_tuple_field_identifier;
use crate::core::ir::EnumDef;
use crate::e2e::field_access::VariantAccessorMap;

/// Build the Dart narrowing map: the freezed subclass to cast to, plus the accessor for the
/// payload inside it.
///
/// Only variants carrying exactly one field are included. A unit variant has no payload to
/// step into, and a multi-field variant has no single accessor that could stand for one, so
/// offering either would render a member that does not exist.
pub(super) fn build_variant_accessor_map(enums: &[EnumDef]) -> VariantAccessorMap {
    let mut map = VariantAccessorMap::default();
    for enum_def in enums {
        for variant in enum_def.variants.iter().filter(|variant| !variant.binding_excluded) {
            let [field] = variant.fields.as_slice() else {
                continue;
            };
            let key = (enum_def.name.clone(), variant.name.clone());
            map.narrowing
                .insert(key.clone(), format!("{}_{}", enum_def.name, variant.name));
            map.payload
                .insert(key, dart_tuple_field_identifier(field.name.trim_start_matches('_')));
        }
    }
    map
}

#[cfg(test)]
#[path = "variant_accessors/tests.rs"]
mod tests;
