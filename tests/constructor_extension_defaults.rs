use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::ApiSurface;

const CASES: &[(&str, &str, &str)] = &[
    ("String", "\"-*+\".to_string()", "\"-*+\""),
    ("usize", "7", "7L"),
    ("f64", "2.5", "2.5"),
    ("Vec<String>", "Vec::new()", "List.of()"),
    (
        "Vec<String>",
        "vec![\"a\".to_string(), \"b\".to_string()]",
        "java.util.List.of(\"a\", \"b\")",
    ),
    ("Mode", "Mode::Fast", "Mode.FAST"),
    ("Option<String>", "None", "null"),
    ("String", "\"~keep\".to_string()", "\"\\u007ekeep\""),
];

fn api(ty: &str, value: &str, middle: bool, annotated: bool) -> anyhow::Result<ApiSurface> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lib.rs");
    let added = format!("pub added_value: {ty},");
    let fields = if middle {
        format!("pub enabled: bool, {added} pub depth: Option<usize>,")
    } else {
        format!("pub enabled: bool, pub depth: Option<usize>, {added}")
    };
    let history = if annotated {
        "#[alef(legacy_constructor_fields = [\"enabled\", \"depth\"])]"
    } else {
        ""
    };
    std::fs::write(
        &path,
        format!(
            r#"
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Mode {{ Slow, Fast }}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
{history}
pub struct Options {{ {fields} }}
impl Default for Options {{ fn default() -> Self {{ Self {{ enabled: false, depth: None, added_value: {value} }} }} }}
"#
        ),
    )?;
    alef::extract::extractor::extract(&[&path], "fixture", "1.0.0", None)
}

fn config() -> ResolvedCrateConfig {
    let raw: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["php", "wasm", "zig", "java", "dart", "python", "swift"]
[[crates]]
name = "fixture"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    raw.resolve().unwrap().remove(0)
}

fn render(backend: &dyn Backend, surface: &ApiSurface) -> String {
    let files = backend.generate_bindings(surface, &config()).unwrap();
    assert!(!files.is_empty());
    files
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn recorded_constructor_accepts_each_resolved_default_in_the_middle_and_at_the_end() {
    for (ty, value, java) in CASES {
        for middle in [true, false] {
            let surface = api(ty, value, middle, true).unwrap();
            let owner = surface.types.iter().find(|ty| ty.name == "Options").unwrap();
            let field = owner.fields.iter().find(|field| field.name == "added_value").unwrap();
            assert!(
                owner.constructor_extension_value(field).is_some(),
                "{ty}, middle={middle}"
            );
            let java_source = render(&alef::backends::java::JavaBackend, &surface);
            assert!(
                java_source.contains("public Options(boolean enabled, Long depth)"),
                "{java_source}"
            );
            let arguments = if middle {
                format!("this(enabled, {java}, depth);")
            } else {
                format!("this(enabled, depth, {java});")
            };
            assert!(java_source.contains(&arguments), "{java_source}");
            for backend in [
                &alef::backends::php::PhpBackend as &dyn Backend,
                &alef::backends::wasm::WasmBackend,
                &alef::backends::dart::DartBackend,
                &alef::backends::zig::ZigBackend,
                &alef::backends::pyo3::Pyo3Backend,
                &alef::backends::swift::SwiftBackend,
            ] {
                assert!(!render(backend, &surface).is_empty(), "{ty}, middle={middle}");
            }
        }
    }
}

#[test]
fn unknown_default_is_rejected_with_the_field_and_type() {
    let error = api("String", "std::env::var(\"UNKNOWN\").unwrap_or_default()", true, true)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("added_value") && error.contains("String") && error.contains("resolved Rust default"),
        "{error}"
    );
    assert!(api("String", "std::env::var(\"UNKNOWN\").unwrap_or_default()", true, false).is_ok());
}

#[test]
fn dart_rewrite_uses_only_mirror_metadata_and_preserves_other_classes() {
    let source = "class Options {\n  // unmatched brace }\n  /* opening { */\n  const Options({\n    required this.enabled,\n    required this.addedValue,\n  });\n}\nclass Other { const Other({required this.addedValue,}); }\n";
    let marker = "// ~keep alef-constructor-default: [\"Options\",\"addedValue\",\"const [\\\"a\\\", \\\"b\\\"]\"]";
    let result = alef::backends::dart::apply_constructor_defaults(source, marker).unwrap();
    assert!(result.contains("this.addedValue = const [\"a\", \"b\"],"), "{result}");
    assert!(result.contains("class Other { const Other({required this.addedValue,}); }"));
    assert_eq!(
        alef::backends::dart::apply_constructor_defaults(&result, marker).unwrap(),
        result
    );
    assert_eq!(
        alef::backends::dart::apply_constructor_defaults(source, "").unwrap(),
        source
    );
    assert!(alef::backends::dart::apply_constructor_defaults("class Other {}", marker).is_err());
}

#[test]
fn emitted_dart_mirror_metadata_restores_enum_defaults_after_frb() {
    let surface = api("Mode", "Mode::Fast", true, true).unwrap();
    let mirror = render(&alef::backends::dart::DartBackend, &surface);
    assert!(
        mirror.contains("[\"Options\",\"addedValue\",\"Mode.fast\"]"),
        "{mirror}"
    );
    let dart =
        "class Options { const Options({required this.enabled, required this.addedValue, required this.depth,}); }";
    let result = alef::backends::dart::apply_constructor_defaults(dart, &mirror).unwrap();
    assert!(result.contains("this.addedValue = Mode.fast,"), "{result}");
    assert!(result.contains("required this.enabled") && result.contains("required this.depth"));
}

#[test]
fn generated_java_legacy_constructor_runs_with_string_integer_list_and_enum_defaults() {
    let fields = "pub label: String, pub count: usize, pub items: Vec<String>, pub mode: Mode,";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, format!(r#"
#[derive(Clone, serde::Serialize, serde::Deserialize)] pub enum Mode {{ Slow, Fast }}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["enabled", "depth"])]
pub struct Options {{ pub enabled: bool, {fields} pub depth: Option<usize> }}
impl Default for Options {{ fn default() -> Self {{ Self {{ enabled: false, depth: None, label: "-*+".into(), count: 7, items: vec!["a".into()], mode: Mode::Fast }} }} }}
"#)).unwrap();
    let surface = alef::extract::extractor::extract(&[&path], "fixture", "1.0.0", None).unwrap();
    let source = render(&alef::backends::java::JavaBackend, &surface);
    let constructor = regex::Regex::new(r"public Options\([^)]*\)\s*\{\s*this\([^;]+;\s*\}")
        .unwrap()
        .find(&source)
        .expect("generated delegating constructor")
        .as_str();
    let java = format!(
        r#"
import java.util.List;
enum Mode {{ SLOW, FAST }}
record Options(boolean enabled, String label, Long count, List<String> items, Mode mode, Long depth) {{ {constructor} }}
class Probe {{ public static void main(String[] args) {{
  Options old = new Options(true, 31L);
  if (!old.enabled() || old.depth() != 31L || !old.label().equals("-*+") || old.count() != 7L || !old.items().equals(List.of("a")) || old.mode() != Mode.FAST) throw new AssertionError(old);
}} }}
"#
    );
    let path = directory.path().join("Probe.java");
    std::fs::write(&path, java).unwrap();
    let compile = std::process::Command::new("javac").arg(&path).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = std::process::Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "Probe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

fn find_constructor(items: Vec<syn::Item>) -> Option<syn::ImplItemFn> {
    items.into_iter().find_map(|item| match item {
        syn::Item::Impl(implementation) => implementation.items.into_iter().find_map(|item| match item {
            syn::ImplItem::Fn(function) if function.sig.ident == "new" || function.sig.ident == "__construct" => {
                Some(function)
            }
            _ => None,
        }),
        syn::Item::Mod(module) => module.content.and_then(|(_, items)| find_constructor(items)),
        _ => None,
    })
}

#[test]
fn generated_php_and_wasm_constructors_recover_nonzero_rust_defaults_at_runtime() {
    use quote::ToTokens;
    for (ty, value, _) in CASES[..5].iter().chain(CASES.last()) {
        for backend in [
            &alef::backends::php::PhpBackend as &dyn Backend,
            &alef::backends::wasm::WasmBackend,
        ] {
            let mut surface = api(ty, value, true, true).unwrap();
            let owner = surface.types.iter_mut().find(|ty| ty.name == "Options").unwrap();
            owner.fields.retain(|field| field.name != "depth");
            owner.version.legacy_constructor_fields = vec!["enabled".into()];
            let files = backend.generate_bindings(&surface, &config()).unwrap();
            let mut function = files
                .iter()
                .filter(|file| file.path.extension().is_some_and(|extension| extension == "rs"))
                .find_map(|file| find_constructor(syn::parse_file(&file.content).unwrap().items))
                .expect("generated Rust constructor");
            function.attrs.clear();
            let name = function.sig.ident.to_string();
            let mapped_type = constructor_extension_type(&function);
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("probe.rs");
            let source = format!(
                r#"
struct Options {{ enabled: bool, added_value: {mapped_type} }}
type WasmOptions = Options;
impl Default for Options {{ fn default() -> Self {{ Self {{ enabled: false, added_value: {value} }} }} }}
impl Options {{ {} }}
fn main() {{ let old = Options::{name}(None, None); assert!(!old.enabled); let expected: {mapped_type} = {value}; assert_eq!(old.added_value, expected); }}
"#,
                function.to_token_stream()
            );
            std::fs::write(&path, source).unwrap();
            let binary = directory.path().join("probe");
            let compile = std::process::Command::new("rustc")
                .args(["--edition=2024", path.to_str().unwrap(), "-o", binary.to_str().unwrap()])
                .output()
                .unwrap();
            assert!(
                compile.status.success(),
                "{ty}: {}",
                String::from_utf8_lossy(&compile.stderr)
            );
            assert!(std::process::Command::new(binary).status().unwrap().success());
        }
    }
}

#[test]
fn dart_build_registers_constructor_defaults_on_the_generated_bridge() {
    use alef::core::backend::{PostBuildStep, PostProcessor};
    let build = alef::backends::dart::DartBackend.build_config_for(&config()).unwrap();
    let paths = build
        .post_build
        .iter()
        .filter_map(|step| match step {
            PostBuildStep::PostProcessFile {
                path,
                processor: PostProcessor::FrbDartOptionalFieldsWithDefaults,
            } => Some(path),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].ends_with("fixture_bridge_generated/lib.dart"));
}

fn constructor_extension_type(function: &syn::ImplItemFn) -> String {
    use quote::ToTokens;
    let syn::FnArg::Typed(parameter) = function.sig.inputs.last().unwrap() else {
        panic!("field parameter")
    };
    let syn::Type::Path(path) = parameter.ty.as_ref() else {
        panic!("optional field")
    };
    let syn::PathArguments::AngleBracketed(arguments) = &path.path.segments.last().unwrap().arguments else {
        panic!("optional argument")
    };
    arguments.args.first().unwrap().to_token_stream().to_string()
}

#[test]
fn emitted_zig_defaults_compile_and_run_for_every_supported_default_kind() {
    let conditions = [
        "std.mem.eql(u8, old.added_value, \"-*+\")",
        "old.added_value == 7",
        "old.added_value == 2.5",
        "old.added_value.len == 0",
        "old.added_value.len == 2 and std.mem.eql(u8, old.added_value[1], \"b\")",
        "old.added_value == .fast",
        "old.added_value == null",
        "std.mem.eql(u8, old.added_value, \"~keep\")",
    ];
    assert_eq!(CASES.len(), conditions.len());
    for ((ty, value, _), condition) in CASES.iter().zip(conditions) {
        let directory = tempfile::tempdir().unwrap();
        let source = render(&alef::backends::zig::ZigBackend, &api(ty, value, true, true).unwrap());
        std::fs::write(directory.path().join("bindings.zig"), source).unwrap();
        std::fs::write(
            directory.path().join("probe.zig"),
            format!(
                r#"
const std = @import("std");
const bindings = @import("bindings.zig");
pub fn main() void {{
    const old = bindings.Options{{ .enabled = true, .depth = null }};
    if (!old.enabled or !({condition})) unreachable;
}}
"#
            ),
        )
        .unwrap();
        let binary = directory.path().join("probe");
        let compile = std::process::Command::new("zig")
            .current_dir(directory.path())
            .args(["build-exe", "probe.zig", &format!("-femit-bin={}", binary.display())])
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{ty}: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        assert!(std::process::Command::new(binary).status().unwrap().success());
    }
}

#[test]
fn dart_constructor_defaults_run_with_scalar_list_enum_and_null_values() {
    let types = [
        "String",
        "int",
        "double",
        "List<String>",
        "List<String>",
        "Mode",
        "String?",
        "String",
    ];
    let checks = [
        "old.addedValue == '-*+'",
        "old.addedValue == 7",
        "old.addedValue == 2.5",
        "old.addedValue.isEmpty",
        "old.addedValue.length == 2 && old.addedValue[1] == 'b'",
        "old.addedValue == Mode.fast",
        "old.addedValue == null",
        "old.addedValue == '~keep'",
    ];
    assert_eq!(CASES.len(), types.len());
    assert_eq!(CASES.len(), checks.len());
    for (((ty, value, _), dart_type), check) in CASES.iter().zip(types).zip(checks) {
        let surface = api(ty, value, false, true).unwrap();
        let mirror = render(&alef::backends::dart::DartBackend, &surface);
        let input = format!(
            r#"
enum Mode {{ slow, fast }}
class Options {{
  final bool enabled;
  final int? depth;
  final {dart_type} addedValue;
  const Options({{required this.enabled, required this.depth, required this.addedValue,}});
}}
void main() {{ final old = Options(enabled: true, depth: 31); if (!old.enabled || old.depth != 31 || !({check})) throw StateError('default lost'); }}
"#
        );
        let source = alef::backends::dart::apply_constructor_defaults(&input, &mirror).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("probe.dart");
        std::fs::write(&path, source).unwrap();
        let output = std::process::Command::new("dart")
            .arg(&path)
            .output()
            .expect("Dart SDK is required for constructor default runtime checks");
        assert!(
            output.status.success(),
            "{ty}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn kotlin_retains_published_positional_fields_before_defaulted_extensions() {
    for (ty, value, _) in CASES {
        for middle in [true, false] {
            let surface = api(ty, value, middle, true).unwrap();
            let source = render(&alef::backends::kotlin_android::KotlinAndroidBackend, &surface);
            let constructor = &source[source.find("data class Options").unwrap()..];
            let enabled = constructor.find("val enabled").unwrap();
            let depth = constructor.find("val depth").unwrap();
            let added = constructor.find("val addedValue").unwrap();
            assert!(enabled < depth && depth < added, "{ty}, middle={middle}: {constructor}");
        }
    }
}

#[test]
fn legacy_boolean_lookup_keeps_its_public_return_type_and_normalization() {
    for value in ["false", "true", "Default::default()"] {
        let surface = api("bool", value, true, true).unwrap();
        let owner = surface.types.iter().find(|ty| ty.name == "Options").unwrap();
        let field = owner.fields.iter().find(|field| field.name == "added_value").unwrap();
        let result: Option<bool> = owner.constructor_extension_default(field);
        assert_eq!(result, Some(value == "true"));
    }
}
