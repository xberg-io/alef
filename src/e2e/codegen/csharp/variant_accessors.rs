//! Which tagged-union variants the C# binding exposes a payload accessor for, in the form the
//! field resolver needs to render a doc snippet that steps into one.
//!
//! ~keep The names are read from `backends::csharp::gen_bindings::variant_accessor_properties`,
//! the same function the generator's own emit loop consumes, rather than re-derived here. A
//! second derivation is the failure this module exists to prevent: naming an accessor the
//! binding did not generate produces a snippet that fails to compile in whichever consumer
//! happens to own that shape, and the snippet gate compiles only a subset of snippets, so it
//! would not reliably be the thing that catches it.

use crate::core::ir::EnumDef;
use crate::e2e::field_access::VariantAccessorMap;

/// Build the C# narrowing map: `(enum name, variant identifier) -> "As<Variant>"`.
pub(super) fn build_variant_accessor_map(enums: &[EnumDef]) -> VariantAccessorMap {
    let mut map = VariantAccessorMap::default();
    for enum_def in enums {
        for (accessor_pascal, _payload_type) in
            crate::backends::csharp::gen_bindings::variant_accessor_properties(enum_def)
        {
            // `variant_accessor_properties` yields the property's PascalCase stem; the emitted
            // member is `As` + that stem, per templates/variant_accessor_property.jinja. The
            // key is the variant's own identifier, which is what the resolver derives from a
            // fixture path segment. ~keep
            let variant = enum_def
                .variants
                .iter()
                .find(|variant| crate::codegen::naming::to_csharp_name(&variant.name) == accessor_pascal)
                .map(|variant| variant.name.clone());
            let Some(variant) = variant else {
                continue;
            };
            map.narrowing
                .insert((enum_def.name.clone(), variant), format!("As{accessor_pascal}"));
        }
    }
    map
}

#[cfg(test)]
#[path = "variant_accessors/tests.rs"]
mod tests;
