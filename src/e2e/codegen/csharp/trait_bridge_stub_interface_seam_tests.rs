//! Cross-generator guard: the C# e2e test-backend stub must implement the exact interface the
//! production C# trait-bridge backend emits.
//!
//! Both the production `I{TraitName}` interface (`backends::csharp::trait_bridge`) and the e2e
//! stub class (`e2e::codegen::csharp::stubs`) build their method signatures independently, once
//! per generator. Before this fix the stub routed through a hand-rolled, duplicate type mapper
//! (`csharp_type_for_stub_visible`) and `heck::ToUpperCamelCase` for names, instead of the
//! production seam (`csharp_type_visible_pub`, `csharp_type_name`, `to_csharp_name`). That
//! duplicate mapper silently drifted from the real one on exactly the shapes exercised here:
//! `Json`, `Duration`, `Option<Duration>`, a `Vec<Named>` type whose name folds under a C#
//! initialism, and a method name that folds under the same initialism. A stub emitted from the
//! diverged mapper does not compile against the real interface (CS0535/CS0246).
//!
//! This test does not assume which side is "correct" — it renders both generators independently
//! and asserts every stub method's parameter *type* list and return type match the interface's,
//! in order. Parameter *names* are intentionally excluded from the comparison: C# does not
//! require parameter names to match between an interface and an implementing class, and the two
//! generators legitimately choose different naming conventions (PascalCase in the interface,
//! camelCase in the stub).

use super::stubs::emit_test_backend;
use crate::backends::csharp::trait_bridge::gen_trait_bridges_file;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;
use std::collections::HashSet;

fn empty_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Extract the ordered list of C# parameter *types* (names stripped) for `method_name`'s
/// signature within `text`, plus its return type. Handles both regular-method form
/// (`Ret Name(T1 a, T2 b)`) and C#'s zero-param-property form (`Ret Name { get; }`), since the
/// stub and interface generators both use whichever form the method's arity calls for.
fn canonical_signature(text: &str, method_name: &str) -> (String, Vec<String>) {
    let call_marker = format!(" {method_name}(");
    if let Some(call_idx) = text.find(&call_marker) {
        let ret = text[..call_idx]
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or_default()
            .to_string();
        let params_start = call_idx + call_marker.len();
        let params_end = text[params_start..]
            .find(')')
            .map(|i| params_start + i)
            .unwrap_or_else(|| panic!("unterminated parameter list for {method_name} in:\n{text}"));
        let inner = text[params_start..params_end].trim();
        let types = if inner.is_empty() {
            Vec::new()
        } else {
            inner
                .split(", ")
                .map(|tok| {
                    tok.rsplit_once(' ')
                        .map(|(ty, _)| ty.to_string())
                        .unwrap_or_else(|| tok.to_string())
                })
                .collect()
        };
        return (ret, types);
    }

    let prop_marker = format!(" {method_name} {{ get; }}");
    let prop_idx = text
        .find(&prop_marker)
        .unwrap_or_else(|| panic!("no signature found for `{method_name}` in:\n{text}"));
    let ret = text[..prop_idx]
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or_default()
        .to_string();
    (ret, Vec::new())
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..ParamDef::default()
    }
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        ..MethodDef::default()
    }
}

/// Builds the fixture trait exercised by every assertion below: `Json`, `Duration`,
/// `Option<Duration>`, and `Vec<Named>` params on one method, plus a zero-param `get_uuid`
/// method returning a `Named` type — the exact shapes the divergent stub mapper got wrong.
fn fixture_trait_and_bridge() -> (TypeDef, TraitBridgeConfig) {
    let configure = method(
        "configure",
        vec![
            param("payload", TypeRef::Json),
            param("max_wait", TypeRef::Duration),
            param("timeout", TypeRef::Optional(Box::new(TypeRef::Duration))),
            param("pairs", TypeRef::Vec(Box::new(TypeRef::Named("UuidPair".to_string())))),
        ],
        TypeRef::Unit,
    );
    let get_uuid = method("get_uuid", Vec::new(), TypeRef::Named("UuidPair".to_string()));

    let trait_def = TypeDef {
        name: "XMLBackend".to_string(),
        is_trait: true,
        methods: vec![configure, get_uuid],
        ..TypeDef::default()
    };

    let bridge = TraitBridgeConfig {
        trait_name: "XMLBackend".to_string(),
        register_fn: Some("register_xml_backend".to_string()),
        ..TraitBridgeConfig::default()
    };

    (trait_def, bridge)
}

#[test]
fn stub_signatures_are_a_subset_of_the_production_interface_signatures() {
    let (trait_def, bridge) = fixture_trait_and_bridge();
    let visible_type_names: HashSet<&str> = ["UuidPair"].into_iter().collect();

    let interface_file = gen_trait_bridges_file(
        "TestNamespace",
        "test_prefix",
        &[("XMLBackend".to_string(), &bridge, &trait_def)],
        &visible_type_names,
    );
    let interface_text = interface_file.content;

    let methods: Vec<&MethodDef> = trait_def.methods.iter().collect();
    let fixture = empty_fixture("xml_backend_stub");
    let emission = emit_test_backend(&bridge, &methods, &fixture);
    let stub_text = emission.setup_block;

    // The interface name itself must use the same initialism-aware casing as the interface
    // generator (`csharp_type_name`), not a bare `heck::ToUpperCamelCase` of the trait name.
    assert!(
        interface_text.contains("interface IXMLBackend"),
        "production interface should be named IXMLBackend, got:\n{interface_text}"
    );
    assert!(
        stub_text.contains(": IXMLBackend"),
        "stub should implement IXMLBackend, got:\n{stub_text}"
    );

    for method_name in ["Configure", "GetUUID"] {
        let (interface_ret, interface_params) = canonical_signature(&interface_text, method_name);
        let (stub_ret, stub_params) = canonical_signature(&stub_text, method_name);

        assert_eq!(
            stub_ret, interface_ret,
            "return type for {method_name}: stub emitted `{stub_ret}`, interface declares `{interface_ret}`"
        );
        assert_eq!(
            stub_params, interface_params,
            "parameter types for {method_name}: stub emitted {stub_params:?}, interface declares {interface_params:?}"
        );
    }
}
