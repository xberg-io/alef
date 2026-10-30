#[path = "../src/test_support/ruby_native.rs"]
#[allow(
    dead_code,
    reason = "target-selection controls do not invoke the shared Ruby interpreter probe"
)]
mod ruby_native;
