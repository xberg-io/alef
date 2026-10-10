use super::*;

/// `not_empty` is the one assertion that silently degrades into a no-op: a check that
/// stringifies before measuring, or that leans on the host language's truthiness, reads
/// as coverage while passing on empty data. Every emitter below must reject an empty
/// collection and an empty string, and must still accept a legitimate `0` / `0.0` / `false`.
#[cfg(test)]
mod not_empty_tests {
    use super::render;

    /// Compare emitted code by token sequence so a template's cosmetic indentation
    /// (which minijinja's whitespace control also influences) cannot mask a behaviour change.
    fn code(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn not_empty_for_python_rejects_empty_sized_values_but_accepts_zero() {
        let rendered = render(
            "python/assertion.jinja",
            crate::alef_context! { assertion_type => "not_empty", field_access => "result.content" },
        );
        assert_eq!(
            rendered.trim(),
            "assert result.content is not None and (not hasattr(result.content, \"__len__\") \
             or len(result.content) > 0)"
        );
    }

    #[test]
    fn not_empty_for_php_arrays_measures_the_element_count() {
        let rendered = render(
            "php/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => "$result->chunks",
                field_is_array => true,
            },
        );
        assert_eq!(
            rendered.trim(),
            "$this->assertGreaterThan(0, count($result->chunks ?? []), 'expected non-empty value');"
        );
    }

    #[test]
    fn not_empty_for_php_scalars_rejects_empty_string_but_accepts_zero() {
        let rendered = render(
            "php/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => "$result->content",
                field_is_array => false,
            },
        );
        // assertNotEmpty() routes through empty(), which would reject 0, 0.0, "0" and false.
        assert!(!rendered.contains("assertNotEmpty"), "got: {rendered}");
        assert_eq!(
            rendered.trim(),
            "$this->assertNotSame('', $result->content ?? '', 'expected non-empty value');"
        );
    }

    #[test]
    fn not_empty_for_ruby_asks_the_value_not_its_string_form() {
        let rendered = render(
            "ruby/assertion.jinja",
            crate::alef_context! { assertion_type => "not_empty", field_expr => "result.content" },
        );
        // `[].to_s` is "[]" — a non-empty string — so the old form could never fail.
        assert!(!rendered.contains(".to_s"), "got: {rendered}");
        assert_eq!(
            rendered.trim(),
            "expect(result.content.respond_to?(:empty?) ? !result.content.empty? : !result.content.nil?).to be(true)"
        );
    }

    #[test]
    fn not_empty_for_java_measures_collections_instead_of_their_string_form() {
        let rendered = render(
            "java/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => "java.util.Optional.ofNullable(result.content())",
            },
        );
        assert!(!rendered.contains("toString"), "got: {rendered}");
        assert_eq!(
            code(&rendered),
            code(
                "assertTrue(java.util.Optional.ofNullable(result.content()).filter(value -> switch ((Object) value) {
                case CharSequence text -> !text.isEmpty();
                case java.util.Collection<?> items -> !items.isEmpty();
                case java.util.Map<?, ?> entries -> !entries.isEmpty();
                default -> true;
            }).isPresent(), \"expected non-empty value\");"
            )
        );
    }

    #[test]
    fn not_empty_for_java_concrete_fields_still_use_is_empty() {
        let rendered = render(
            "java/assertion.jinja",
            crate::alef_context! { assertion_type => "not_empty", field_expr => "result.content()" },
        );
        assert_eq!(
            code(&rendered),
            code("assertFalse(result.content().isEmpty(), \"expected non-empty value\");")
        );
    }

    #[test]
    fn not_empty_for_csharp_pattern_matches_instead_of_stringifying() {
        let rendered = render(
            "csharp/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => "result.Content",
                field_needs_json_serialize => false,
                skipped_reason => "",
            },
        );
        // `.ToString()` on a struct yields the type name, and the old `?.` form did not
        // compile for a non-nullable value type.
        assert!(!rendered.contains("ToString"), "got: {rendered}");
        assert_eq!(
            code(&rendered),
            code(
                "Assert.True(((object?)result.Content) switch
            {
                null => false,
                string text => text.Length > 0,
                System.Collections.ICollection items => items.Count > 0,
                _ => true,
            }, \"expected non-empty value\");"
            )
        );
    }

    #[test]
    fn not_empty_for_csharp_collections_still_use_assert_not_empty() {
        let rendered = render(
            "csharp/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => "result.Chunks",
                field_needs_json_serialize => true,
                skipped_reason => "",
            },
        );
        assert_eq!(code(&rendered), code("Assert.NotEmpty(result.Chunks);"));
    }

    #[test]
    fn not_empty_for_zig_json_rejects_empty_array_and_empty_string() {
        let rendered = render(
            "zig/json_assertion.jinja",
            crate::alef_context! { assertion_type => "not_empty", field_expr => "_content" },
        );
        // `!= .null` accepted an empty array and an empty string.
        assert!(!rendered.contains("!= .null"), "got: {rendered}");
        assert_eq!(
            code(&rendered),
            code(
                "{
                const _ne = _content;
                try testing.expect(switch (_ne) {
                    .null => false,
                    .string => |_s| _s.len > 0,
                    .array => |_a| _a.items.len > 0,
                    .object => |_o| _o.count() > 0,
                    else => true,
                });
            }"
            )
        );
    }

    #[test]
    fn not_empty_for_typescript_sizes_strings_and_arrays_and_accepts_zero() {
        let rendered = render(
            "typescript/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => "result.content",
                field_is_optional => false,
            },
        );
        assert_eq!(
            code(&rendered),
            code(
                "{
                const _v = result.content;
                if (typeof _v === \"string\" || Array.isArray(_v)) {
                    expect(_v.length).toBeGreaterThan(0);
                } else {
                    expect(_v).toBeDefined();
                    expect(_v).not.toBeNull();
                }
            }"
            )
        );
    }
}

#[cfg(test)]
mod template_registration_tests {
    use super::TEMPLATES;
    use std::collections::HashSet;
    use std::path::Path;

    /// `go/harness_main.go.jinja` is read by its own private `minijinja::Environment` in
    /// `render_harness_main` (`src/e2e/codegen/go.rs`), via a local `include_str!` rather than
    /// through this shared `TEMPLATES` registry. That function is currently dead code — its own
    /// doc comment says the server-pattern harness is now emitted by a consumer `Extension` and
    /// alef no longer calls it, kept only pending a dead-code sweep — so the file is genuinely
    /// unreachable through `render()`, but deleting it would break `render_harness_main`'s
    /// `include_str!`, and that's an emitter-side change out of scope here. ~keep
    const ALLOWLISTED_UNREGISTERED: &[&str] = &["go/harness_main.go.jinja"];

    /// `render()` resolves names against `TEMPLATES`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into this array compiles fine
    /// (`include_str!` only runs for entries that are listed) and panics only once an
    /// emitter reaches it at generation time. Compare by content rather than by
    /// registered key: some backends register a file under a shortened or aliased name,
    /// which is fine, but every file's bytes must appear in `TEMPLATES` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/e2e/templates"));
        let registered_contents: HashSet<&str> = TEMPLATES.iter().map(|(_, content)| *content).collect();

        let mut unregistered = Vec::new();
        collect_unregistered(templates_dir, templates_dir, &registered_contents, &mut unregistered);
        unregistered.retain(|path| !ALLOWLISTED_UNREGISTERED.contains(&path.as_str()));
        unregistered.sort();
        assert!(
            unregistered.is_empty(),
            "found .jinja file(s) in templates/ whose content is not registered in TEMPLATES: {unregistered:?}"
        );
    }

    fn collect_unregistered(
        root: &Path,
        dir: &Path,
        registered_contents: &HashSet<&str>,
        unregistered: &mut Vec<String>,
    ) {
        for entry in std::fs::read_dir(dir).expect("read templates directory") {
            let entry = entry.expect("read templates directory entry");
            let path = entry.path();
            if path.is_dir() {
                collect_unregistered(root, &path, registered_contents, unregistered);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jinja") {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read template file");
            if !registered_contents.contains(content.as_str()) {
                let relative = path
                    .strip_prefix(root)
                    .expect("template path under templates root")
                    .to_str()
                    .expect("template path is valid UTF-8")
                    .replace('\\', "/");
                unregistered.push(relative);
            }
        }
    }
}

/// A synthetic assertion is appended to the output with a bare `push_str`, so the template
/// — not the caller — owns its line terminator. `trim_blocks` already eats the newline that
/// follows a block tag, so a branch that also closes with `{%- endif %}` renders a bare
/// fragment with no newline at either end, and whatever the next emitter writes lands on the
/// same physical line. That is harmless between two statements and destructive after a
/// comment, which swallows the statement that follows it. Observed in a downstream consumer's
/// generated PHP smoke suite, where two skip comments shared one physical line.
#[cfg(test)]
mod synthetic_assertion_line_discipline {
    use super::render;

    const KIND_KEYED_TEMPLATES: [&str; 3] = [
        "php/synthetic_assertion.jinja",
        "java/synthetic_assertion.jinja",
        "r/synthetic_assertion.jinja",
    ];

    fn skip_comment(template: &str) -> String {
        if template == "typescript/synthetic_assertion.jinja" {
            return render(
                template,
                crate::alef_context! {
                    assertion_type => "unsupported_by_this_backend",
                    field_name => "metadata.format.excel.sheet_count",
                },
            );
        }
        render(
            template,
            crate::alef_context! {
                assertion_kind => "skipped",
                assertion_type => "unsupported_by_this_backend",
                field_name => "metadata.format.excel.sheet_count",
            },
        )
    }

    fn every_template() -> Vec<&'static str> {
        let mut templates = KIND_KEYED_TEMPLATES.to_vec();
        templates.push("typescript/synthetic_assertion.jinja");
        templates
    }

    #[test]
    fn a_skip_comment_terminates_its_own_line() {
        for template in every_template() {
            let rendered = skip_comment(template);
            assert!(
                rendered.contains("skipped:"),
                "{template} rendered no skip comment: {rendered:?}"
            );
            assert!(
                rendered.ends_with('\n'),
                "{template} left its comment unterminated: {rendered:?}"
            );
            assert_eq!(
                rendered.matches('\n').count(),
                1,
                "{template} did not render exactly one line: {rendered:?}"
            );
        }
    }

    /// The defect is *concatenation*, so the assertion has to be made on two appended
    /// renders. Checking a single render in isolation is the vacuous version of this test:
    /// it passes whether or not the fragment can collide with its successor.
    #[test]
    fn two_appended_skip_comments_do_not_share_a_physical_line() {
        for template in every_template() {
            let mut out = String::new();
            out.push_str(&skip_comment(template));
            out.push_str(&skip_comment(template));
            assert_eq!(
                out.lines().count(),
                2,
                "{template} merged two appended assertions onto one line: {out:?}"
            );
        }
    }

    /// Positive control: the branches that emit real code must still emit it, unchanged
    /// apart from now owning their terminator.
    #[test]
    fn a_rendered_assertion_still_emits_its_statement_on_one_line() {
        let php = render(
            "php/synthetic_assertion.jinja",
            crate::alef_context! {
                assertion_kind => "chunks_content",
                assertion_type => "is_true",
                pred => "$carry",
                field_name => "chunks_have_content",
            },
        );
        assert_eq!(php, "        $this->assertTrue($carry);\n");

        let java = render(
            "java/synthetic_assertion.jinja",
            crate::alef_context! {
                assertion_kind => "chunks_content",
                assertion_type => "is_true",
                pred => "carry",
                field_name => "chunks_have_content",
            },
        );
        assert_eq!(java, "        assertTrue(carry, \"expected true\");\n");
    }

    /// A statement followed by a comment is the destructive ordering: without a terminator
    /// the comment starts on the statement's line, and every later emitter is commented out.
    #[test]
    fn a_comment_appended_after_a_statement_starts_its_own_line() {
        let mut out = String::new();
        out.push_str(&render(
            "php/synthetic_assertion.jinja",
            crate::alef_context! {
                assertion_kind => "chunks_content",
                assertion_type => "is_true",
                pred => "$carry",
                field_name => "chunks_have_content",
            },
        ));
        out.push_str(&skip_comment("php/synthetic_assertion.jinja"));

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "statement and comment shared a line: {out:?}");
        assert!(!lines[0].contains("//"), "the statement was commented out: {out:?}");
        assert!(
            lines[1].trim_start().starts_with("//"),
            "unexpected second line: {out:?}"
        );
    }
}

/// minijinja renders a Rust `bool` with PYTHON's spelling, not Rust's or JavaScript's.
///
/// ~keep This is not a quirk of any one template — it is `impl Display for minijinja::Value`
/// (`ValueRepr::Bool(val) => f.write_str(if val { "True" } else { "False" })`, minijinja 2.24.0
/// `src/value/mod.rs:794`), reached by every `{{ }}` interpolation: `Instruction::Emit` ->
/// `write_escaped` -> `AutoEscape::None` -> `write!(out, "{value}")`. minijinja is a Jinja2
/// implementation and Jinja2's booleans are Python's.
///
/// The consequence for a code generator is a hard defect, not a cosmetic one. A template that
/// emits `.toBe({{ expected }})` from a Rust `bool` produces `.toBe(True)`, which is `TS2304:
/// Cannot find name 'True'` under tsc and a `ReferenceError: True is not defined` under node —
/// both verified by execution. The same trap is waiting in every target language whose boolean
/// literal is lowercase (JS/TS, Rust, Java, Go, C, Swift, Kotlin, PHP, Dart, Zig): only Python
/// itself is spelled the way minijinja writes it.
///
/// So a Rust `bool` must never reach generated code through `{{ }}`. Use it in `{% if %}` (where
/// only its truthiness is read and the spelling never appears), or convert it to the target
/// language's own literal in Rust first — the way
/// `backends::extendr::trait_bridge` passes `has_error_check => if has_error { "true" } else
/// { "false" }` as a `&str`. This test exists so the engine's behaviour is a pinned fact rather
/// than something each generator author has to rediscover from a failing suite.
#[cfg(test)]
mod minijinja_bool_spelling_tests {
    /// The engine's own rendering, asserted directly so this cannot pass because some template
    /// happens to avoid the case. Fails if a future minijinja adopts Rust's spelling — at which
    /// point the doc above, and any workaround written against it, need revisiting.
    #[test]
    fn a_rust_bool_interpolated_by_minijinja_renders_python_spelled() {
        let mut env = minijinja::Environment::new();
        env.add_template("probe", "{{ flag }}")
            .expect("probe template is valid");
        let template = env.get_template("probe").expect("probe template is registered");

        let rendered_true = template
            .render(crate::alef_context! { flag => true })
            .expect("probe renders");
        let rendered_false = template
            .render(crate::alef_context! { flag => false })
            .expect("probe renders");

        assert_eq!(rendered_true, "True", "minijinja's bool spelling changed");
        assert_eq!(rendered_false, "False", "minijinja's bool spelling changed");
        assert_ne!(rendered_true, "true", "a bool is still not safe to interpolate");
    }

    /// The supported alternative, pinned alongside the trap so the fix is not guesswork: a Rust
    /// `&str` carrying the target language's own literal passes through unchanged.
    #[test]
    fn a_target_language_boolean_literal_passed_as_a_str_survives_intact() {
        let mut env = minijinja::Environment::new();
        env.add_template("probe", "{{ flag }}")
            .expect("probe template is valid");
        let template = env.get_template("probe").expect("probe template is registered");

        let rendered = template
            .render(crate::alef_context! { flag => if true { "true" } else { "false" } })
            .expect("probe renders");

        assert_eq!(rendered, "true", "got: {rendered}");
    }

    /// `{% if %}` reads truthiness only, so a bool is safe there — this is why the trap is
    /// narrow and why most templates in this crate are unaffected. ~keep
    #[test]
    fn a_bool_used_only_as_a_condition_never_leaks_its_spelling() {
        let mut env = minijinja::Environment::new();
        env.add_template("probe", "{% if flag %}yes{% else %}no{% endif %}")
            .expect("probe template is valid");
        let template = env.get_template("probe").expect("probe template is registered");

        let rendered = template
            .render(crate::alef_context! { flag => true })
            .expect("probe renders");

        assert_eq!(rendered, "yes", "got: {rendered}");
        assert!(!rendered.contains("True"), "got: {rendered}");
    }
}
