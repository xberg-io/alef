# Unsigned Dart native probe

The probe preserves the original 54 cases and adds 67 named cases (121 total). Five unsigned boundaries exercise nested DTOs and caller-owned probe handles; direct invalid newtypes, record fields and nested record/optional/list/map leaves must be rejected. The fixture-owned Rust counter proves valid calls reach the core once and invalid calls never reach it; this does not instrument the FRB codec itself.

FRB’s generalized `Uint64List.fromList` casts values to unsigned 64-bit elements at construction. Once an invalid value has been cast, its original value is unavailable to a later argument validator. This probe tests all five valid unsigned list boundaries and deliberately makes no invalid-list rejection promise. Scalar and ordinary map/DTO BigInt leaves remain subject to range validation.

The separate existing Dart stream fixture now tests 42 and `u64::MAX` without removing its optional-null, typed-error or cancellation controls. The unsigned callback generation test checks carrier types only. Actual five-boundary Dart-to-Rust and Rust-to-Dart callback runtime coverage remains unfinished: this fixture has no configured callable callback registration/core-invocation path. Neither fixture has been executed as part of this test-only checkpoint.
