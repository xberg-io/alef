use crate::codegen::error_gen::gen_magnus_error_class_registrations;
use crate::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeRef};
use crate::test_support::toolchain;

fn error(name: &str, variants: &[(&str, bool)]) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        variants: variants
            .iter()
            .map(|(variant, payload)| ErrorVariant {
                name: (*variant).into(),
                is_unit: !payload,
                fields: if *payload {
                    vec![FieldDef {
                        name: "limit".into(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            })
            .collect(),
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// Ruby equivalent of the magnus registrations: `define_error` becomes a class definition, and
/// each `module_eval` call keeps its Rust string literal, whose escapes Ruby reads the same way.
fn registrations_as_ruby(lines: &[String]) -> String {
    let mut classes: Vec<(String, String)> = Vec::new();
    let mut script = String::from("module Demo; end\n");
    for line in lines {
        if let Some(rest) = line.trim().strip_prefix("let ") {
            let (var, call) = rest.split_once(" = module.define_error(\"").expect("define_error line");
            let (name, parent) = call.split_once("\", ").expect("define_error arguments");
            let parent = parent.trim_end_matches(")?;");
            let parent = classes
                .iter()
                .find(|(class_var, _)| class_var == parent)
                .map_or_else(|| "StandardError".to_string(), |(_, class)| format!("Demo::{class}"));
            script.push_str(&format!("class Demo::{name} < {parent}; end\n"));
            classes.push((var.to_string(), name.to_string()));
        } else if let Some(rest) = line
            .trim()
            .strip_prefix("module.funcall::<_, _, magnus::Value>(\"module_eval\", (")
        {
            script.push_str(&format!("Demo.module_eval({})\n", rest.trim_end_matches(",))?;")));
        }
    }
    script
}

/// Routed through the crate-wide [`toolchain::RUBY`] gate: skipped without Ruby on `PATH`,
/// a failure when `ALEF_REQUIRE_RUBY` is set. ~keep
#[test]
fn rescue_of_either_family_matches_their_shared_variant_class_in_ruby() {
    let Some(ruby) = toolchain::RUBY.open() else {
        return;
    };
    let api = ApiSurface {
        errors: vec![
            error("GraphQLError", &[("ComplexityLimitExceeded", false), ("Parse", false)]),
            error("SchemaError", &[("ComplexityLimitExceeded", true), ("Invalid", false)]),
        ],
        ..Default::default()
    };
    let lines = gen_magnus_error_class_registrations(&api).unwrap();
    let script = format!(
        "{}{}",
        registrations_as_ruby(&lines),
        r#"
def caught?(error, family)
  raise error
rescue family
  true
rescue StandardError
  false
end
shared = Demo::ComplexityLimitExceededError.new("limit")
raise "base family lost" unless caught?(shared, Demo::GraphQLError)
raise "second family missing" unless caught?(shared, Demo::SchemaError)
raise "family module missing" unless caught?(shared, Demo::SchemaError::Family)
raise "subclass widened" if caught?(shared, Demo::InvalidError)
raise "unrelated member" if caught?(Demo::ParseError.new("parse"), Demo::SchemaError)
raise "family member lost" unless caught?(Demo::InvalidError.new("invalid"), Demo::SchemaError)
puts "6 runtime checks passed"
"#
    );
    let result = std::process::Command::new(ruby)
        .args(["-e", &script])
        .output()
        .expect("spawn ruby for the shared exception class regression");
    assert!(
        result.status.success(),
        "generated Ruby failed:\n{}\n{script}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "6 runtime checks passed"
    );
}

/// `rescue` of an `exception_base` ancestor of the second family matches the shared class too.
#[test]
fn rescue_of_the_second_family_ancestors_matches_their_shared_variant_class_in_ruby() {
    let Some(ruby) = toolchain::RUBY.open() else {
        return;
    };
    let mut schema = error("SchemaError", &[("ComplexityLimitExceeded", true), ("Invalid", false)]);
    schema.exception_base = Some("RootError".into());
    let api = ApiSurface {
        errors: vec![
            error("GraphQLError", &[("ComplexityLimitExceeded", false), ("Parse", false)]),
            error("RootError", &[("Other", false)]),
            schema,
        ],
        ..Default::default()
    };
    let lines = gen_magnus_error_class_registrations(&api).unwrap();
    let script = format!(
        "{}{}",
        registrations_as_ruby(&lines),
        r#"
def caught?(error, family)
  raise error
rescue family
  true
rescue StandardError
  false
end
shared = Demo::ComplexityLimitExceededError.new("limit")
raise "base family lost" unless caught?(shared, Demo::GraphQLError)
raise "second family missing" unless caught?(shared, Demo::SchemaError)
raise "ancestor family missing" unless caught?(shared, Demo::RootError)
raise "ancestor module missing" unless shared.is_a?(Demo::RootError::Family)
raise "ancestor subclass widened" if caught?(shared, Demo::OtherError)
raise "ancestor member lost" unless caught?(Demo::InvalidError.new("invalid"), Demo::RootError)
raise "unrelated member" if caught?(Demo::ParseError.new("parse"), Demo::RootError)
puts "7 runtime checks passed"
"#
    );
    let result = std::process::Command::new(ruby)
        .args(["-e", &script])
        .output()
        .expect("spawn ruby for the shared exception ancestor regression");
    assert!(
        result.status.success(),
        "generated Ruby failed:\n{}\n{script}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "7 runtime checks passed"
    );
}
