//! `FieldResolver` methods, split by concern: construction/IR wiring,
//! field classification predicates, accessor code generation, and the chained tagged-union
//! crossing walk.

mod accessor;
mod classify;
mod construct;
mod display_safety;
mod go_shape;
mod tagged_union_crossing;
