use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::fixture::{CallbackAction, VisitorSpec};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

pub(super) fn attach_visitor(
    setup: &mut Vec<String>,
    args: &str,
    visitor: &VisitorSpec,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Option<String> {
    let bridge = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.options_type.is_some() && bridge.resolved_options_field().is_some())?;
    let trait_def = type_defs.iter().find(|type_def| type_def.name == bridge.trait_name)?;
    let result_type = bridge.result_type.as_deref().unwrap_or("VisitResult");
    let result_enum = enums.iter().find(|enum_def| enum_def.name == result_type);
    setup.push(format!("val visitor = object : {} {{", bridge.trait_name));
    for (name, action) in &visitor.callbacks {
        let method = trait_def.methods.iter().find(|method| method.name == *name)?;
        let parameters = method
            .params
            .iter()
            .map(|parameter| {
                let mut imports = BTreeSet::new();
                let ty = crate::backends::kotlin::kotlin_type_str_pub(&parameter.ty, parameter.optional, &mut imports);
                format!("{}: {ty}", parameter.name.to_lower_camel_case())
            })
            .collect::<Vec<_>>()
            .join(", ");
        setup.push(format!(
            "    override fun {}({parameters}): {result_type} = {}",
            name.to_lower_camel_case(),
            action_expression(action, method, result_type, result_enum)
        ));
    }
    setup.push("}".to_string());
    let options_type = bridge.options_type.as_deref()?;
    let field = bridge.resolved_options_field()?.to_lower_camel_case();
    setup.push(format!("val options = {options_type}().copy({field} = visitor)"));
    Some(replace_options(args, options_type))
}

fn replace_options(args: &str, options_type: &str) -> String {
    for suffix in ["null".to_string(), format!("{options_type}()")] {
        if args == suffix {
            return "options".to_string();
        }
        if let Some(prefix) = args.strip_suffix(&format!(", {suffix}")) {
            return format!("{prefix}, options");
        }
    }
    if args.is_empty() {
        "options".to_string()
    } else {
        format!("{args}, options")
    }
}

/// ~keep Kotlin's binding backend (`backends/kotlin/gen_bindings/object_wrapper/enums`)
/// has no static-factory generator, unlike Java/C#/Go/Dart: every sum type is a
/// `sealed class` whose unit variants are `object`s (referenced bare, e.g.
/// `VisitResult.Skip`, no parens — it is a singleton, not a call) and whose
/// payload-carrying variants are `data class`es (constructed with parens, e.g.
/// `VisitResult.Custom("text")`). A single `Outer.variant()`-style rule cannot express
/// both, so `construct_variant` picks the form per variant from the real `EnumDef`
/// when the result-type enum is available in the extracted API surface, rather than
/// re-hardcoding the split as a second, driftable assumption.
fn action_expression(
    action: &CallbackAction,
    method: &crate::core::ir::MethodDef,
    result_type: &str,
    result_enum: Option<&EnumDef>,
) -> String {
    match action {
        CallbackAction::Skip => construct_variant(result_type, "Skip", result_enum, None),
        CallbackAction::Continue => construct_variant(result_type, "Continue", result_enum, None),
        CallbackAction::PreserveHtml => construct_variant(result_type, "PreserveHtml", result_enum, None),
        CallbackAction::Custom { output } => {
            let escaped = crate::e2e::escape::escape_kotlin(output);
            construct_variant(result_type, "Custom", result_enum, Some(format!("\"{escaped}\"")))
        }
        CallbackAction::CustomTemplate { template, .. } => {
            let mut expression = crate::e2e::escape::escape_kotlin(template);
            for parameter in &method.params {
                let name = parameter.name.to_lower_camel_case();
                expression = expression.replace(&format!("{{{}}}", parameter.name), &format!("${name}"));
            }
            construct_variant(result_type, "Custom", result_enum, Some(format!("\"{expression}\"")))
        }
    }
}

/// ~keep Build `Outer.Variant` (unit — a bare reference to the `object` singleton) or
/// `Outer.Variant(args)` (payload — a `data class` constructor call). When
/// `result_enum` resolves and declares the named variant, its field count is
/// authoritative; otherwise falls back to treating any action that carries `args` as
/// payload-shaped, matching the fixture protocol's own Skip/Continue/PreserveHtml
/// (unit) vs. Custom (payload) convention.
fn construct_variant(
    result_type: &str,
    variant_name: &str,
    result_enum: Option<&EnumDef>,
    args: Option<String>,
) -> String {
    let has_payload = result_enum
        .and_then(|enum_def| enum_def.variants.iter().find(|variant| variant.name == variant_name))
        .map(|variant| !variant.fields.is_empty())
        .unwrap_or_else(|| args.is_some());
    if has_payload {
        format!("{result_type}.{variant_name}({})", args.unwrap_or_default())
    } else {
        format!("{result_type}.{variant_name}")
    }
}

#[cfg(test)]
mod tests {
    use super::{attach_visitor, construct_variant, replace_options};
    use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};
    use std::collections::BTreeMap;

    /// Whether `kotlinc` runs, not merely resolves: a version-manager shim spawns fine then exits
    /// non-zero, so a spawn-only check below would leave the compile gate unreachable and fire
    /// the assert everywhere Kotlin is absent. ~keep
    fn kotlinc_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("kotlinc")
                .arg("-version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    /// ~keep The real shape alef's Kotlin backend emits for a sum type with unit and
    /// payload variants: `object Skip : VisitResult()` / `data class Custom(...)`.
    /// See `backends/kotlin/gen_bindings/object_wrapper/enums/mod.rs`.
    fn visit_result_enum() -> EnumDef {
        EnumDef {
            name: "VisitResult".into(),
            variants: vec![
                EnumVariant {
                    name: "Skip".into(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Continue".into(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "PreserveHtml".into(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Custom".into(),
                    fields: vec![FieldDef {
                        name: "0".into(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    }],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        }
    }

    fn text_visitor_trait() -> TypeDef {
        TypeDef {
            name: "TextVisitor".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "visit_text".into(),
                params: vec![ParamDef {
                    name: "text".into(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("VisitResult".into()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }
    }

    fn text_visitor_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "TextVisitor".into(),
                options_type: Some("RenderOptions".into()),
                options_field: Some("visitor".into()),
                result_type: Some("VisitResult".into()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    #[test]
    fn replaces_default_options_argument() {
        assert_eq!(replace_options("html, null", "Options"), "html, options");
        assert_eq!(replace_options("html, Options()", "Options"), "html, options");
    }

    #[test]
    fn construct_variant_reads_unit_vs_payload_from_the_real_enum() {
        let enum_def = visit_result_enum();
        assert_eq!(
            construct_variant("VisitResult", "Skip", Some(&enum_def), None),
            "VisitResult.Skip"
        );
        assert_eq!(
            construct_variant("VisitResult", "Custom", Some(&enum_def), Some("\"text\"".to_string())),
            "VisitResult.Custom(\"text\")"
        );
    }

    #[test]
    fn construct_variant_falls_back_to_args_presence_when_enum_is_unknown() {
        assert_eq!(construct_variant("VisitResult", "Skip", None, None), "VisitResult.Skip");
        assert_eq!(
            construct_variant("VisitResult", "Custom", None, Some("\"text\"".to_string())),
            "VisitResult.Custom(\"text\")"
        );
    }

    #[test]
    fn emits_trait_override_and_attaches_options() {
        let visitor = VisitorSpec {
            callbacks: BTreeMap::from([("visit_text".into(), CallbackAction::Skip)]),
        };
        let config = text_visitor_config();
        let types = [text_visitor_trait()];
        let mut setup = Vec::new();

        let args = attach_visitor(&mut setup, "html, null", &visitor, &config, &types, &[]).expect("visitor metadata");
        let rendered = setup.join("\n");
        assert_eq!(args, "html, options");
        assert!(rendered.contains("object : TextVisitor"), "{rendered}");
        assert!(
            rendered.contains("override fun visitText(text: String): VisitResult = VisitResult.Skip"),
            "{rendered}"
        );
        assert!(
            rendered.contains("RenderOptions().copy(visitor = visitor)"),
            "{rendered}"
        );
        if kotlinc_is_runnable() {
            let directory = tempfile::tempdir().expect("temporary Kotlin project");
            let source = format!(
                "sealed class VisitResult {{\n    object Skip : VisitResult()\n}}\n\
                 interface TextVisitor {{ fun visitText(text: String): VisitResult = VisitResult.Skip }}\n\
                 data class RenderOptions(val visitor: TextVisitor? = null)\n\
                 fun render(html: String, options: RenderOptions) = html + options.toString()\n\
                 fun main() {{\n{}\nrender(\"sample\", options)\n}}",
                setup
                    .iter()
                    .map(|line| format!("    {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let path = directory.path().join("Snippet.kt");
            std::fs::write(&path, source).expect("write Kotlin snippet");
            let output = std::process::Command::new("kotlinc")
                .arg(&path)
                .arg("-d")
                .arg(directory.path().join("snippet.jar"))
                .output()
                .expect("run kotlinc");
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
    }

    #[test]
    fn emits_data_class_construction_for_a_payload_variant() {
        let visitor = VisitorSpec {
            callbacks: BTreeMap::from([(
                "visit_text".into(),
                CallbackAction::Custom {
                    output: "replacement".into(),
                },
            )]),
        };
        let config = text_visitor_config();
        let types = [text_visitor_trait()];
        let enums = [visit_result_enum()];
        let mut setup = Vec::new();

        attach_visitor(&mut setup, "html, null", &visitor, &config, &types, &enums).expect("visitor metadata");
        let rendered = setup.join("\n");
        assert!(rendered.contains("VisitResult.Custom(\"replacement\")"), "{rendered}");
        assert!(
            !rendered.contains("VisitResult.custom("),
            "must not emit the Dart-style lowercase factory call, got:\n{rendered}"
        );

        if kotlinc_is_runnable() {
            let directory = tempfile::tempdir().expect("temporary Kotlin project");
            let source = format!(
                "sealed class VisitResult {{\n    data class Custom(val field0: String) : VisitResult()\n}}\n\
                 interface TextVisitor {{ fun visitText(text: String): VisitResult = VisitResult.Custom(\"replacement\") }}\n\
                 data class RenderOptions(val visitor: TextVisitor? = null)\n\
                 fun render(html: String, options: RenderOptions) = html + options.toString()\n\
                 fun main() {{\n{}\nrender(\"sample\", options)\n}}",
                setup
                    .iter()
                    .map(|line| format!("    {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let path = directory.path().join("Snippet.kt");
            std::fs::write(&path, source).expect("write Kotlin snippet");
            let output = std::process::Command::new("kotlinc")
                .arg(&path)
                .arg("-d")
                .arg(directory.path().join("snippet.jar"))
                .output()
                .expect("run kotlinc");
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
    }
}
