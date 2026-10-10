use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef};
use crate::docs::descriptions::generate_enum_variant_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at};
use crate::docs::enum_payload_fields::variant_field_descriptions;
use crate::docs::formatting::escape_table_cell;
use crate::docs::naming::{enum_variant_name, field_name, lang_code_fence, type_name};
use crate::docs::{clean_doc, doc_type, template_env, version_labels};

use super::function_render::push_version_annotation;

/// For a WASM untagged data enum, the note-plus-code-block that replaces the `Value |
/// Description` table below: that table's framing (a fixed set of named, referenceable
/// "values") does not hold for these types at all in the WASM binding -- `enums::gen_enum` is
/// never called for them (see its own `~keep` note), so there is no `Wasm{Enum}` class or
/// member to reference by name. What a JS/TS caller actually sees is the structural TypeScript
/// union `backends::wasm::docs_ts_type_for_untagged_enum` computes -- the SAME function the
/// WASM backend itself calls to emit the `.d.ts`, so this can never independently drift from
/// the real generated type the way the old per-variant table did. Returns `None` for every
/// other language, and for a WASM enum that does not lower this way (see that function's own
/// doc). ~keep
fn wasm_untagged_union_note(
    en: &EnumDef,
    lang: Language,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
) -> Option<String> {
    if lang != Language::Wasm {
        return None;
    }
    let ts_type = crate::backends::wasm::docs_ts_type_for_untagged_enum(en, api, config)?;
    let mut out = String::new();
    out.push_str(
        "This type has no dedicated class in the WASM binding. It is expressed as the following \
         structural TypeScript type:\n\n",
    );
    out.push_str(&template_env::render(
        "code_block.jinja",
        crate::alef_context! { lang_code => lang_code_fence(lang), body => ts_type },
    ));
    Some(out)
}

pub(super) fn render_enum(
    en: &EnumDef,
    lang: Language,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let ename = type_name(&en.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        crate::alef_context! { marker => "####", title => ename },
    ));

    push_version_annotation(&mut out, &en.version);

    let doc = clean_doc(&en.doc, lang);
    // Nest under the `####` heading emitted just above, rather than shifting by a fixed
    // number of levels. A fixed `+2` assumes the doc comment starts at `#`, and a section that
    // starts anywhere else lands ABOVE its own parent: a rustdoc `# Observability` surfaced as
    // `###` under a `####` item, so it read as a sibling of the page's `### Functions` section and
    // took a bogus entry in the table of contents with it. ~keep
    let doc = demote_headings_to_start_at(&doc, 5);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    if let Some(note) = wasm_untagged_union_note(en, lang, config, api) {
        out.push_str(&note);
        out.push('\n');
        return out;
    }

    out.push_str("| Value | Description |\n");
    out.push_str("|-------|-------------|\n");
    for variant in &en.variants {
        let vname = enum_variant_name(&variant.name, lang, ffi_prefix);
        let mut vdoc = if !variant.doc.is_empty() {
            clean_doc_inline(&variant.doc, lang)
        } else {
            generate_enum_variant_description(&variant.name)
        };
        let variant_fields: Vec<_> = if lang == Language::Rust {
            variant.fields.iter().collect()
        } else {
            binding_fields(&variant.fields).collect()
        };
        if !variant_fields.is_empty() {
            let (fields_desc, fields_label) = variant_field_descriptions(
                en,
                variant,
                &variant_fields,
                api,
                |name| field_name(name, lang),
                |f| doc_type(&f.ty, lang, ffi_prefix),
            );
            vdoc = format!("{vdoc} — {fields_label}: {}", fields_desc.join(", "));
        }
        if let Some(ref since) = variant.version.since {
            let since = version_labels::major_minor(since);
            vdoc = format!("{vdoc} — **Since:** `v{since}`");
        }
        if let Some(ref dep) = variant.version.deprecated {
            let dep_note = match (&dep.since, &dep.note) {
                (Some(s), Some(n)) => format!("Deprecated since `v{}`: {n}", version_labels::major_minor(s)),
                (Some(s), None) => format!("Deprecated since `v{}`", version_labels::major_minor(s)),
                (None, Some(n)) => format!("Deprecated: {n}"),
                (None, None) => "Deprecated".to_string(),
            };
            vdoc = format!("{vdoc} — {dep_note}");
        }
        out.push_str(&template_env::render(
            "variant_row.jinja",
            crate::alef_context! { name => escape_table_cell(&vname), doc => escape_table_cell(&vdoc) },
        ));
    }
    out.push('\n');

    out
}
