use super::*;
use crate::core::config::NewAlefConfig;

fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
    cfg.resolve().expect("resolve").remove(0)
}

fn minimal_config() -> ResolvedCrateConfig {
    resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []
"#,
    )
}

fn zero_arg_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn dto(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        ..Default::default()
    }
}

/// The strongest tier: a visible zero-arg, primitive-returning function is actually
/// invoked across the Magnus boundary, not merely named.
#[test]
fn calls_a_visible_zero_argument_function() {
    let api = ApiSurface {
        functions: vec![zero_arg_function("ping", TypeRef::Primitive(PrimitiveType::Bool))],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(out.starts_with("# frozen_string_literal: true\n"), "got:\n{out}");
    assert!(out.contains("require_relative \"../lib/my_lib\"\n"), "got:\n{out}");
    assert!(out.contains("RSpec.describe MyLib do\n"), "got:\n{out}");
    assert!(
        out.contains("    expect(described_class.ping).to(be(true).or(be(false)))\n"),
        "got:\n{out}"
    );
}

/// The matcher must follow the Magnus type map, not a generic truthiness check.
#[test]
fn matches_the_returned_ruby_type_for_each_return_kind() {
    let cases = [
        (TypeRef::String, "expect(described_class.probe).to(be_a(String))"),
        (
            TypeRef::Primitive(PrimitiveType::U64),
            "expect(described_class.probe).to(be_a(Integer))",
        ),
        (
            TypeRef::Primitive(PrimitiveType::F64),
            "expect(described_class.probe).to(be_a(Float))",
        ),
    ];
    for (return_type, expected) in cases {
        let api = ApiSurface {
            functions: vec![zero_arg_function("probe", return_type)],
            ..Default::default()
        };
        let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");
        assert!(out.contains(expected), "expected `{expected}`, got:\n{out}");
    }
}

/// A function taking parameters cannot be called generically (unknown ownership and
/// conversion needs per parameter), so the ladder must degrade instead of guessing.
#[test]
fn skips_functions_that_take_parameters() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            params: vec![crate::core::ir::ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..zero_arg_function("greet", TypeRef::String)
        }],
        types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("greet"), "got:\n{out}");
    assert!(
        out.contains("described_class::Widget.new(label: \"alef-scaffold\")"),
        "got:\n{out}"
    );
}

/// Async functions run through a Tokio runtime under a differently-named Rust body; the
/// seed must not be the first thing to exercise that path.
#[test]
fn skips_async_functions() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            is_async: true,
            ..zero_arg_function("fetch", TypeRef::String)
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("fetch"), "got:\n{out}");
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// A `cfg`-gated function is registered only when the extension was compiled with that
/// feature, which a scaffold-time seed cannot know.
#[test]
fn skips_cfg_gated_functions() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            cfg: Some("feature = \"extra\"".to_string()),
            ..zero_arg_function("extra_ping", TypeRef::String)
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("extra_ping"), "got:\n{out}");
}

/// A function the Magnus wrapper generator cannot delegate gets an `unimplemented` body
/// that raises `RuntimeError` when called. It is still registered and callable, so only
/// this predicate keeps the seed off it — otherwise the example would be permanently red
/// on a healthy build.
#[test]
fn skips_functions_whose_generated_body_only_raises() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            sanitized: true,
            error_type: Some("Error".to_string()),
            ..zero_arg_function("not_delegatable", TypeRef::String)
        }],
        types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("not_delegatable"), "got:\n{out}");
    assert!(
        out.contains("described_class::Widget.new(label: \"alef-scaffold\")"),
        "got:\n{out}"
    );
}

/// A fallible function can raise for reasons that have nothing to do with the binding, so
/// an infallible candidate wins even when it appears later in the surface.
#[test]
fn prefers_an_infallible_function_over_a_fallible_one() {
    let api = ApiSurface {
        functions: vec![
            FunctionDef {
                error_type: Some("Error".to_string()),
                ..zero_arg_function("might_fail", TypeRef::String)
            },
            zero_arg_function("always_works", TypeRef::String),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(described_class.always_works).to(be_a(String))\n"),
        "got:\n{out}"
    );
    assert!(!out.contains("might_fail"), "got:\n{out}");
}

/// When every candidate is fallible the strongest tier still fires: an example that can
/// fail for a real reason is worth more than degrading to a weaker one.
#[test]
fn still_calls_a_fallible_function_when_it_is_the_only_candidate() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            error_type: Some("Error".to_string()),
            ..zero_arg_function("might_fail", TypeRef::String)
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(described_class.might_fail).to(be_a(String))\n"),
        "got:\n{out}"
    );
}

/// `binding_excluded` functions never reach the generated extension, so the seed must not
/// call one.
#[test]
fn skips_binding_excluded_functions() {
    let api = ApiSurface {
        functions: vec![
            FunctionDef {
                binding_excluded: true,
                ..zero_arg_function("hidden", TypeRef::String)
            },
            zero_arg_function("visible", TypeRef::String),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(out.contains("described_class.visible"), "got:\n{out}");
    assert!(!out.contains("hidden"), "got:\n{out}");
}

/// `[crates.ruby] exclude_functions` mirrors `MagnusBackend`'s own filter, so a function
/// excluded there must be skipped here too.
#[test]
fn skips_functions_excluded_via_ruby_config() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []

[crates.ruby]
exclude_functions = ["ping"]
"#,
    );
    let api = ApiSurface {
        functions: vec![zero_arg_function("ping", TypeRef::String)],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &config, "my_lib");

    assert!(
        !out.contains("ping"),
        "excluded function must not be referenced, got:\n{out}"
    );
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// With no callable function, a literal-constructible DTO is built through the generated
/// keyword constructor and every field read back through its accessor.
#[test]
fn constructs_a_simple_dto_and_asserts_every_field() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![
                simple_field("label", TypeRef::String),
                simple_field("count", TypeRef::Primitive(PrimitiveType::U32)),
            ],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    instance = described_class::Widget.new(label: \"alef-scaffold\", count: 1)\n"),
        "got:\n{out}"
    );
    assert!(
        out.contains("    expect([instance.label, instance.count]).to(eq([\"alef-scaffold\", 1]))\n"),
        "got:\n{out}"
    );
}

/// An all-String multi-field DTO must not emit a bracketed `["alef-scaffold",
/// "alef-scaffold"]` array literal: `RUBY_SEED_STRING_LITERAL` is a hyphenated word, which
/// the `.rubocop.yml` scaffolded alongside this file still matches via `Style/WordArray`'s
/// `WordRegex` (it explicitly allows one hyphen) at its default `MinSize` of 2 -- flagging
/// every new Ruby consumer's freshly-scaffolded spec red before a single line is hand-edited.
#[test]
fn constructs_an_all_string_dto_without_a_bracketed_word_array() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![
                simple_field("label", TypeRef::String),
                simple_field("note", TypeRef::String),
            ],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect([instance.label, instance.note]).to(eq(%w[alef-scaffold alef-scaffold]))\n"),
        "got:\n{out}"
    );
    assert!(
        !out.contains("[\"alef-scaffold\", \"alef-scaffold\"]"),
        "must not emit a bracketed all-String literal array: got:\n{out}"
    );
}

/// A single-field DTO reads better as a scalar comparison than a one-element array.
#[test]
fn asserts_a_single_field_dto_without_an_array() {
    let api = ApiSurface {
        types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(instance.label).to(eq(\"alef-scaffold\"))\n"),
        "got:\n{out}"
    );
}

/// A `Named` field has no default in the generated constructor, so a partial construction
/// would raise `ArgumentError`. The whole type is rejected rather than partly built.
#[test]
fn falls_back_to_a_constant_reference_for_a_dto_with_a_named_field() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![
                simple_field("label", TypeRef::String),
                simple_field("nested", TypeRef::Named("Other".to_string())),
            ],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains(".new("), "got:\n{out}");
    assert!(
        out.contains("    expect(described_class.const_get(:Widget)).to(be_a(Module))\n"),
        "got:\n{out}"
    );
}

/// Optional fields carry `nil` semantics this seed does not model, so they disqualify the
/// construction tier rather than being guessed at.
#[test]
fn falls_back_to_a_constant_reference_for_a_dto_with_an_optional_field() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![FieldDef {
                optional: true,
                ..simple_field("label", TypeRef::String)
            }],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains(".new("), "got:\n{out}");
    assert!(out.contains("const_get(:Widget)"), "got:\n{out}");
}

/// `[crates.ruby] exclude_types` and `binding_excluded` both remove a class from the
/// generated extension, so neither may be named by the seed.
#[test]
fn skips_types_excluded_by_config_or_binding_exclusion() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []

[crates.ruby]
exclude_types = ["Excluded"]
"#,
    );
    let api = ApiSurface {
        types: vec![
            dto("Excluded", vec![simple_field("label", TypeRef::String)]),
            TypeDef {
                binding_excluded: true,
                ..dto("Hidden", vec![simple_field("label", TypeRef::String)])
            },
            dto("Visible", vec![simple_field("label", TypeRef::String)]),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &config, "my_lib");

    assert!(!out.contains("Excluded"), "got:\n{out}");
    assert!(!out.contains("Hidden"), "got:\n{out}");
    assert!(out.contains("described_class::Visible.new("), "got:\n{out}");
}

/// `MagnusBackend::generate_public_api` drops `*Update` and `*Builder` types from the
/// module's curated re-export list, so a seed naming one may reference nothing.
#[test]
fn skips_update_and_builder_types() {
    let api = ApiSurface {
        types: vec![
            dto("WidgetUpdate", vec![simple_field("label", TypeRef::String)]),
            dto("WidgetBuilder", vec![simple_field("label", TypeRef::String)]),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("WidgetUpdate"), "got:\n{out}");
    assert!(!out.contains("WidgetBuilder"), "got:\n{out}");
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// Enums are not registered as Ruby constants by the Magnus backend, so an enum-only
/// surface must degrade to the version tier rather than name a constant that is absent.
#[test]
fn never_names_an_enum_because_magnus_registers_none_as_constants() {
    let api = ApiSurface {
        enums: vec![crate::core::ir::EnumDef {
            name: "Colour".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("Colour"), "got:\n{out}");
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// An empty API surface still gets a falsifiable example: `VERSION` only resolves once the
/// gem — and therefore the native extension `native.rb` dlopens — has loaded.
#[test]
fn falls_back_to_the_version_assertion_when_the_api_surface_is_empty() {
    let out = scaffold_ruby_spec(&ApiSurface::default(), &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(described_class::VERSION).to match(/\\A\\d+\\.\\d+\\.\\d+/)\n"),
        "got:\n{out}"
    );
}

/// No tier may emit a tautology, and every tier must go through the `require_relative`
/// that dlopens the native extension — that is the property making even the weakest tier
/// falsifiable rather than decorative.
#[test]
fn no_tier_emits_a_vacuous_or_unlinked_example() {
    let surfaces = [
        ApiSurface {
            functions: vec![zero_arg_function("ping", TypeRef::String)],
            ..Default::default()
        },
        ApiSurface {
            types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
            ..Default::default()
        },
        ApiSurface {
            types: vec![dto(
                "Widget",
                vec![simple_field("nested", TypeRef::Named("Other".to_string()))],
            )],
            ..Default::default()
        },
        ApiSurface::default(),
    ];
    for api in surfaces {
        let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");
        assert!(
            out.contains("require_relative \"../lib/my_lib\""),
            "every tier must load the gem, got:\n{out}"
        );
        assert_eq!(out.matches("  it \"").count(), 1, "exactly one example, got:\n{out}");
        for tautology in ["expect(1)", "eq(1 + 1)", "to be_truthy", "to be_falsey"] {
            assert!(!out.contains(tautology), "vacuous assertion `{tautology}` in:\n{out}");
        }
        assert!(
            out.contains("described_class"),
            "the example must assert against the generated module, got:\n{out}"
        );
    }
}

/// The seed carries no alef header marker, and must not: the marker is what
/// `write_scaffold_files_report`'s ownership guard reads as "alef owns this file", which
/// would let an `overwrite: true` run (e.g. `alef version`) replace a hand-written suite.
#[test]
fn seed_content_carries_no_alef_marker() {
    let out = scaffold_ruby_spec(&ApiSurface::default(), &minimal_config(), "my_lib");

    assert!(
        !crate::core::hash::content_has_alef_marker(&out),
        "seed must stay unmarked so it is never reclaimed by an overwrite run, got:\n{out}"
    );
}

/// The seed lands at the path the generated `Rakefile`'s `RSpec::Core::RakeTask` already
/// scans, and is emitted create-only so a real suite is never overwritten.
#[test]
fn seed_is_emitted_create_only_at_the_rspec_default_path() {
    let config = minimal_config();
    let api = ApiSurface {
        version: "1.2.3".to_string(),
        ..Default::default()
    };
    let files = scaffold_ruby(&api, &config).expect("scaffold");
    let spec = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("/spec/"))
        .expect("a spec seed must be emitted");

    assert_eq!(spec.path.to_string_lossy(), "packages/ruby/spec/my_lib_spec.rb");
    assert!(!spec.generated_header, "the seed must stay create-only");
}

/// The `~keep` in this seed's rationale is load-bearing, unlike the markers the
/// render-time strip (`core::keep_marker`) removes from `.jinja` output. The seed is
/// create-only, so alef never rewrites it and the consumer's own `poly` uncomment pass is
/// what reads it — without the marker the rationale is deleted by the next `poly fmt`.
/// Pinned across every tier so a broadening of the strip cannot silently take it. ~keep
#[test]
fn every_seed_tier_keeps_its_uncomment_pass_marker() {
    let config = minimal_config();
    let tiers = [
        (
            "call",
            ApiSurface {
                functions: vec![zero_arg_function("ping", TypeRef::Primitive(PrimitiveType::Bool))],
                ..Default::default()
            },
        ),
        (
            "construct",
            ApiSurface {
                types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
                ..Default::default()
            },
        ),
        (
            "constant",
            ApiSurface {
                types: vec![dto(
                    "Widget",
                    vec![
                        simple_field("label", TypeRef::String),
                        simple_field("nested", TypeRef::Named("Other".to_string())),
                    ],
                )],
                ..Default::default()
            },
        ),
        ("version", ApiSurface::default()),
    ];

    for (tier, api) in tiers {
        let out = scaffold_ruby_spec(&api, &config, "my_lib");
        assert!(
            out.contains("replace it with a real suite. ~keep") || out.contains("break on the next release. ~keep"),
            "the {tier} tier lost its uncomment-pass marker, got:\n{out}"
        );
    }
}

/// Regression for the gemspec/RuboCop deadlock: alef's generated `.gemspec` once filtered
/// `spec.files` with `.reject { |f| f.match?(%r{...}) }`, which RuboCop's
/// `Style/SelectByRegexp` flags (it wants `grep_v`). Both the gemspec and the `.rubocop.yml`
/// that lints it are `generated_header: true`, so the consumer has no file they can hand-edit
/// to escape the violation -- `alef build` reintroduces it every run. This test does not shell
/// out to `rubocop` (not guaranteed present in CI); instead it structurally forbids the exact
/// shape the cop flags -- a `.select`/`.reject` block whose predicate is `<expr>.match?(%r{...})`
/// -- anywhere in the generated gemspec, and confirms `grep_v` is present as the replacement.
#[test]
fn gemspec_files_filter_never_reintroduces_the_select_by_regexp_anti_pattern() {
    let files = scaffold_ruby(&ApiSurface::default(), &minimal_config()).expect("scaffold");
    let gemspec = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".gemspec"))
        .expect("a gemspec must be emitted");

    let select_by_regexp_shape = regex_lite_contains_match_predicate(&gemspec.content);
    assert!(
        !select_by_regexp_shape,
        "gemspec re-introduces the Style/SelectByRegexp anti-pattern \
         (`.select`/`.reject` with a `.match?(%r{{...}})` predicate), got:\n{}",
        gemspec.content
    );
    assert!(
        gemspec.content.contains(".grep_v(%r{"),
        "gemspec must filter spec.files via grep_v, RuboCop's own autocorrect target, got:\n{}",
        gemspec.content
    );
}

#[test]
fn gemspec_is_scoped_to_this_package_and_supports_dual_spdx_licenses() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "my-lib"
sources = []

[crates.scaffold]
license = "AGPL-3.0-only OR PolyForm-Small-Business-1.0.0"
"#,
    );
    let files = scaffold_ruby(&ApiSurface::default(), &config).expect("scaffold");
    let gemspec = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".gemspec"))
        .expect("a gemspec must be emitted");

    assert!(
        gemspec
            .content
            .contains("spec.licenses      = [\"AGPL-3.0-only\", \"PolyForm-Small-Business-1.0.0\"]")
    );
    assert!(gemspec.content.contains("lib/my_lib.rb"));
    assert!(gemspec.content.contains("lib/my_lib/**/*"));
    assert!(gemspec.content.contains("ext/my_lib_rb/**/*"));
    assert!(!gemspec.content.contains("lib/**/* ext/**/*"));
}

/// Scans for `.select { |x| <expr>.match?(%r{...}) }` or `.reject { |x| <expr>.match?(%r{...}) }`
/// without pulling in a full regex engine dependency for one test: a hand-rolled scanner over
/// the small, fixed set of generated files is enough to catch the specific shape RuboCop's
/// `Style/SelectByRegexp` flags.
fn regex_lite_contains_match_predicate(content: &str) -> bool {
    for method in [".select {", ".reject {", ".select{", ".reject{"] {
        let mut search_from = 0;
        while let Some(offset) = content[search_from..].find(method) {
            let start = search_from + offset;
            let block_end = content[start..].find('}').map_or(content.len(), |end| start + end + 1);
            if content[start..block_end].contains(".match?(%r{") {
                return true;
            }
            search_from = start + method.len();
        }
    }
    false
}

/// Defense in depth: even after the `Style/SelectByRegexp` fix, a *future* RuboCop cop could
/// flag something else in the alef-owned gemspec or Rakefile, and the consumer would again have
/// no file to hand-edit around it (both carry `generated_header: true`, so `alef build`
/// overwrites any workaround). Excluding both from `AllCops.Exclude` in the generated
/// `.rubocop.yml` -- mirroring the Go backend's `exclusions: generated: lax` -- means a new cop
/// can no longer reopen this exact deadlock shape on a file the consumer cannot edit.
#[test]
fn rubocop_config_excludes_the_alef_owned_gemspec_and_rakefile() {
    let files = scaffold_ruby(&ApiSurface::default(), &minimal_config()).expect("scaffold");
    let rubocop_yml = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".rubocop.yml"))
        .expect("a .rubocop.yml must be emitted");

    assert!(
        rubocop_yml.content.contains("\"*.gemspec\""),
        "AllCops.Exclude must cover the alef-owned gemspec, got:\n{}",
        rubocop_yml.content
    );
    assert!(
        rubocop_yml.content.contains("\"Rakefile\""),
        "AllCops.Exclude must cover the alef-owned Rakefile, got:\n{}",
        rubocop_yml.content
    );
}

/// `ruby_core_dep_features` must drop an excluded name from the core dependency's own explicit
/// `features = [...]` line -- the surface `configured_swift_features`'s widening bug (see
/// `feature_gate::configured_swift_features`) showed the analogous `excluded_default_features`
/// check must also reach, not just the wrapper's `default = [...]` array. Asserts both
/// directions: an excluded name never appears, and a name nobody excluded still does.
#[test]
fn ruby_core_dep_features_drops_excluded_names_but_keeps_others() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []
[crates.ruby]
features = ["native-http", "wasm-http"]
excluded_default_features = ["native-http"]
"#,
    );
    let excluded: std::collections::HashSet<&str> = ["native-http"].into_iter().collect();

    let features_str = ruby_core_dep_features(&config, &excluded);

    assert!(
        !features_str.contains("native-http"),
        "an excluded name must never reach the dependency's features = [...] line: {features_str}"
    );
    assert!(
        features_str.contains("wasm-http"),
        "a feature nobody excluded must still be forwarded: {features_str}"
    );
}

/// End-to-end regression for the reported defect: a `[crates.ruby].target_dep_overrides` entry
/// excludes a feature for one platform, but `scaffold_ruby_cargo`'s wrapper-level
/// `[features] default = [...]` array previously forwarded every `collect_cfg_features` name
/// unconditionally, re-enabling the excluded dependency one layer down regardless of platform.
/// `excluded_default_features` must keep the name out of that `default` array (while still
/// declaring it, so `cargo build --features <name>` keeps working) without dropping an
/// unrelated, non-excluded feature from the same array.
#[test]
fn scaffold_ruby_cargo_excludes_named_feature_from_wrapper_default_but_keeps_others() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []
[crates.ruby]
gem_name = "test_lib"
excluded_default_features = ["native-http"]
[[crates.ruby.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["wasm-http"]
default_features = false
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "NativeOnly".to_string(),
                rust_path: "my_lib::NativeOnly".to_string(),
                cfg: Some(r#"feature = "native-http""#.to_string()),
                ..Default::default()
            },
            TypeDef {
                name: "WasmOnly".to_string(),
                rust_path: "my_lib::WasmOnly".to_string(),
                cfg: Some(r#"feature = "wasm-http""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let files = scaffold_ruby_cargo(&api, &config).expect("scaffold_ruby_cargo ok");
    let cargo_toml = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("Cargo.toml emitted")
        .content;

    let default_line = cargo_toml
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert!(
        !default_line.contains("native-http"),
        "excluded_default_features must drop the name from the wrapper's own default array:\n{default_line}"
    );
    assert!(
        default_line.contains("wasm-http"),
        "a feature nobody excluded must still be forwarded into default:\n{default_line}"
    );
    assert!(
        cargo_toml.contains(r#"native-http = ["my-lib/native-http"]"#),
        "the excluded feature stays declared (so `cargo build --features native-http` still \
         works), just not defaulted:\n{cargo_toml}"
    );
}

/// Regression for alef-task #374: an `excluded_default_features` name that gates no item in the
/// extracted API surface (e.g. a Cargo-only feature that only affects a dependency's `build.rs`
/// linking, such as `libheif-sys` via `heic`) is never discovered by
/// `crate::codegen::cfg::collect_cfg_features`, which walks `#[cfg(feature = "X")]` attributes on
/// IR nodes. The `[features]` table was built exclusively from that discovery set, so a
/// config-only name never got its promised opt-in forwarding entry at all -- breaking
/// `cargo build -p <crate>-rb --features <name>` on desktop, exactly the escape hatch
/// `excluded_default_features` documents as always available.
/// `scaffold_ruby_cargo_excludes_named_feature_from_wrapper_default_but_keeps_others` above does
/// not catch this: it excludes `native-http` from a cfg-gated `TypeDef`, so `native-http` IS
/// discoverable there and only exercises the already-working half.
#[test]
fn scaffold_ruby_cargo_forwards_excluded_feature_not_referenced_by_any_cfg_attribute() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []
[crates.ruby]
gem_name = "test_lib"
excluded_default_features = ["heic"]
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        ..Default::default()
    };

    let files = scaffold_ruby_cargo(&api, &config).expect("scaffold_ruby_cargo ok");
    let cargo_toml = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("Cargo.toml emitted")
        .content;

    assert!(
        cargo_toml.contains("[features]"),
        "a config-only excluded_default_features name must still produce a [features] table:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"heic = ["my-lib/heic"]"#),
        "a config-only excluded_default_features name (not referenced by any \
         #[cfg(feature = ...)] in the API surface) must still get a forwarding entry so \
         `cargo build --features heic` keeps working:\n{cargo_toml}"
    );
    let default_line = cargo_toml
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert!(
        !default_line.contains("heic"),
        "default = [...] must NOT contain excluded `heic`; got: {default_line}"
    );
}
