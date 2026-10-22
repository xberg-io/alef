//! Wraps a Swift call into the Rust bridge so a failure throws the typed Swift error.
//!
//! The bridge throws an `AlefBridgeError`. `alefBridgedCall` and `alefBridgedCallAsync` (emitted
//! with the error enum, see `errors::emit_bridge_conversions`) convert it, so every call that can
//! fail goes through them.

/// The Swift expression for `call`, a `RustBridge.name(args)` call.
///
/// `throws` is true when the bridge function returns a `Result`. An `async` bridge function
/// always returns a `Result`, so `throws` is true for it as well.
pub(super) fn bridged_call(call: &str, throws: bool, is_async: bool) -> String {
    match (throws, is_async) {
        (true, true) => format!("try await alefBridgedCallAsync {{ try await {call} }}"),
        (true, false) => format!("try alefBridgedCall {{ try {call} }}"),
        (false, true) => format!("await {call}"),
        (false, false) => call.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::bridged_call;

    #[test]
    fn a_throwing_call_goes_through_the_conversion() {
        assert_eq!(
            bridged_call("RustBridge.f(x)", true, false),
            "try alefBridgedCall { try RustBridge.f(x) }"
        );
        assert_eq!(
            bridged_call("RustBridge.f(x)", true, true),
            "try await alefBridgedCallAsync { try await RustBridge.f(x) }"
        );
    }

    #[test]
    fn a_call_that_cannot_fail_stays_plain() {
        assert_eq!(bridged_call("RustBridge.f(x)", false, false), "RustBridge.f(x)");
        assert_eq!(bridged_call("RustBridge.f(x)", false, true), "await RustBridge.f(x)");
    }
}
