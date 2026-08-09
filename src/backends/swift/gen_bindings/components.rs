//! Public Swift component-manager wrappers.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> Option<String> {
    if config.components.is_empty() {
        return None;
    }
    Some(
        r#"public enum AlefComponentError: Error, LocalizedError {
    case unsupported(String)
    case runtime(String)

    public var errorDescription: String? {
        switch self {
        case .unsupported(let message): return message
        case .runtime(let message): return message
        }
    }
}

private struct AlefComponentResponse<Value: Decodable>: Decodable {
    let ok: Value?
    let err: String?
}

private func alefDecodeComponentResponse<Value: Decodable>(
    _ response: RustString,
    as type: Value.Type
) throws -> Value {
    let decoded = try JSONDecoder().decode(
        AlefComponentResponse<Value>.self,
        from: Data(response.toString().utf8)
    )
    if let error = decoded.err {
        throw AlefComponentError.runtime(error)
    }
    guard let value = decoded.ok else {
        throw AlefComponentError.runtime("Native component operation returned no value")
    }
    return value
}

private func alefEnsureComponentPlatform() throws {
#if os(iOS) || os(tvOS) || os(watchOS) || os(visionOS)
    throw AlefComponentError.unsupported(
        "Downloadable native components are unsupported on Apple mobile targets; bundle the feature profile into the application instead."
    )
#endif
}

/// Ensure a signed native component is downloaded, verified, and loaded.
public func componentLoad(_ component: String) throws {
    try alefEnsureComponentPlatform()
    _ = try alefDecodeComponentResponse(
        RustBridge.component_load(RustString(component)),
        as: Bool.self
    )
}

/// Prefetch one component, or every configured component when `component` is nil.
public func componentPrefetch(_ component: String? = nil) throws -> [String] {
    try alefEnsureComponentPlatform()
    return try alefDecodeComponentResponse(
        RustBridge.component_prefetch(component.map(RustString.init)),
        as: [String].self
    )
}

/// Return `missing`, `cached:<path>`, or `loaded:<path>` for a component.
public func componentStatus(_ component: String) throws -> String {
    try alefEnsureComponentPlatform()
    return try alefDecodeComponentResponse(
        RustBridge.component_status(RustString(component)),
        as: String.self
    )
}

/// Return the verified content-addressed cache path for a component.
public func componentCachePath(_ component: String) throws -> String {
    try alefEnsureComponentPlatform()
    return try alefDecodeComponentResponse(
        RustBridge.component_cache_path(RustString(component)),
        as: String.self
    )
}

"#
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ComponentProfileConfig;

    #[test]
    fn emits_public_swift_api_with_explicit_mobile_exclusion() {
        let config = ResolvedCrateConfig {
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo_core::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["aarch64-apple-darwin".into()],
            }],
            ..ResolvedCrateConfig::default()
        };
        let generated = generate(&config).expect("component API");
        assert!(generated.contains("public func componentLoad"));
        assert!(generated.contains("public func componentPrefetch"));
        assert!(generated.contains("public func componentStatus"));
        assert!(generated.contains("public func componentCachePath"));
        assert!(generated.contains("#if os(iOS) || os(tvOS)"));
        assert!(generated.contains("RustBridge.component_load"));
        assert!(generated.contains("AlefComponentResponse<Value>"));
    }
}
