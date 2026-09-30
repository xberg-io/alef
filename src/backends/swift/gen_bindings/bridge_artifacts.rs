use crate::backends::swift::gen_bindings::boxes::{swift_adapter_conversions, swift_box_ffi_type, swift_box_params};
use crate::backends::swift::naming::{swift_rust_shim_ident as swift_ident, swift_source_ident as swift_case_ident};
use crate::codegen::serde_enum_repr::{SerdeEnumRepr, serde_enum_repr};
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) mod umbrella_header;

/// The directory cargo actually writes build output into for the workspace the process cwd sits
/// in, or `None` when no ancestor carries a `Cargo.lock` to anchor the search.
///
/// Resolved through [`crate::core::cargo_target_dir`] rather than joined as
/// `workspace_root/"target"`: a host that redirects output via `CARGO_TARGET_DIR` or a
/// `.cargo/config.toml` `build.target-dir` otherwise leaves every scan below matching nothing,
/// which is indistinguishable from an unbuilt crate (alef #477). ~keep
fn swift_bridge_target_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let workspace_root = std::iter::once(cwd.clone())
        .chain(cwd.ancestors().skip(1).map(|p| p.to_path_buf()))
        .take(8)
        .find(|p| p.join("Cargo.lock").exists())?;
    Some(crate::core::cargo_target_dir::for_workspace_root(&workspace_root))
}

pub(crate) fn find_swift_bridge_out_dir(binding_crate_name: &str) -> Option<PathBuf> {
    let target = swift_bridge_target_root()?;

    let crate_prefix = format!("{binding_crate_name}-");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;

    for profile in ["release", "debug"] {
        let build_dir = target.join(profile).join("build");
        let entries = match std::fs::read_dir(&build_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with(&crate_prefix) {
                continue;
            }
            let out = entry.path().join("out");
            let marker = out.join("SwiftBridgeCore.swift");
            if !marker.exists() {
                continue;
            }
            let mtime = std::fs::metadata(&marker)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                best = Some((mtime, out));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Emit the swift-bridge-produced Swift/header trio, or `None` when there is nothing to
/// write.
///
/// `consult_build_output` gates whether `target/`'s swift-bridge build output is read at
/// all. It must be `false` from [`super::generate`] (the `alef generate` path) and `true`
/// only from the [`PostBuildStep::MaterializeSwiftBridge`] post-build step
/// (`cli::pipeline::commands::build`), which runs unconditionally after `alef generate`'s
/// `complete_generated_artifacts` triggers a cheap `cargo check` of this crate
/// (`SwiftBackend::generate_post_build_config`, task #541) and after `alef build` triggers a
/// real `cargo build --release` (`SwiftBackend::build_config_with_config`) -- both populate the
/// same `OUT_DIR` via this crate's own `build.rs`, so either satisfies `find_swift_bridge_out_dir`.
///
/// The two calls used to be one, unconditionally reading `target/`, and that was the alef
/// #A/#B bug: `target/`'s build directory is a side effect of *this same command's own*
/// post-build step, so whether it exists yet is a function of run ordering, not of source
/// input. A `generate()` call early in the run saw no directory and emitted nothing (or a
/// placeholder); an otherwise-identical call after a build had populated `target/` emitted
/// the full trio and fed it through the ownership-guarded writer, which then refused two
/// files it had never seen before — the refusal set and the "Generated N files" count both
/// moved between two runs of an unmodified tree. `MaterializeSwiftBridge` already writes
/// these same three files unguarded, straight to disk, every time this crate's `cargo
/// build` succeeds — it is the one place a build tool's own output belongs. Routing
/// `generate()` through the identical `target/`-reading branch duplicated that writer
/// through the ownership-guarded path where the files can never carry a marker (swift
/// bridge's own header/import conventions rule out `generated_header: true`), so the
/// duplication was strictly worse than a no-op: it could only refuse or race, never help.
///
/// Ownership bookkeeping for the trio is handled separately, by
/// [`PostBuildStep::owned_paths`] (`core::backend`), not by this function returning `None`
/// or `Some`: the orphan sweep in `bin_cli::core_commands` needs these three paths claimed
/// on every run `MaterializeSwiftBridge` is configured to touch them, independent of
/// whether this call found anything new to write this time. ~keep
pub(crate) fn emit_swift_bridge_files(
    crate_name: &str,
    binding_crate_name: &str,
    package_root: &std::path::Path,
    consult_build_output: bool,
) -> anyhow::Result<Option<Vec<GeneratedFile>>> {
    let out_dir = consult_build_output
        .then(|| find_swift_bridge_out_dir(binding_crate_name))
        .flatten();
    let out_dir = match out_dir {
        Some(d) => d,
        None => {
            let sources_rust_bridge_c = package_root.join("Sources").join("RustBridgeC");
            let header_path = sources_rust_bridge_c.join("RustBridgeC.h");

            // A populated header means a prior build already materialized the real trio,
            // and `MaterializeSwiftBridge` -- not this call -- is what keeps it current from
            // here on (see that step's `owned_paths`, which is what stops the orphan sweep
            // from reading this `None` as "alef no longer generates this" and deleting a
            // file nothing here regenerated -- the alef #B incident). Re-deriving content
            // here instead would have to round-trip through `normalize_content`, which the
            // unguarded `MaterializeSwiftBridge` write never applies to what lands on disk;
            // the two would disagree on whitespace and the ownership guard would refuse the
            // "fix" as foreign, which is worse than doing nothing. ~keep
            if let Ok(existing) = std::fs::read_to_string(&header_path)
                && umbrella_header::is_populated(&existing)
            {
                return Ok(None);
            }
            let minimal_header = format!(
                "#ifndef RUST_BRIDGE_C_H\n\
                 #define RUST_BRIDGE_C_H\n\
                 \n\
                 // Placeholder header for the RustBridgeC SwiftPM target.\n\
                 // `alef build` (or `alef generate`, which checks this crate as a cheap\n\
                 // post-build step) populates this header automatically once\n\
                 // `{binding_crate_name}` has been checked or built at least once.\n\
                 // The typedefs below are the minimum required for SwiftBridgeCore.swift\n\
                 // to compile before that has happened.\n\
                 \n\
                 #include <stdbool.h>\n\
                 #include <stdint.h>\n\
                 \n\
                 typedef struct RustStr {{\n  \
                 uint8_t *const start;\n  \
                 uintptr_t len;\n\
                 }} RustStr;\n\
                 typedef struct __private__FfiSlice {{\n  \
                 void *const start;\n  \
                 uintptr_t len;\n\
                 }} __private__FfiSlice;\n\
                 typedef struct __private__OptionU8 {{\n  \
                 uint8_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionU8;\n\
                 typedef struct __private__OptionI8 {{\n  \
                 int8_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionI8;\n\
                 typedef struct __private__OptionU16 {{\n  \
                 uint16_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionU16;\n\
                 typedef struct __private__OptionI16 {{\n  \
                 int16_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionI16;\n\
                 typedef struct __private__OptionU32 {{\n  \
                 uint32_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionU32;\n\
                 typedef struct __private__OptionI32 {{\n  \
                 int32_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionI32;\n\
                 typedef struct __private__OptionU64 {{\n  \
                 uint64_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionU64;\n\
                 typedef struct __private__OptionI64 {{\n  \
                 int64_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionI64;\n\
                 typedef struct __private__OptionUsize {{\n  \
                 uintptr_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionUsize;\n\
                 typedef struct __private__OptionIsize {{\n  \
                 intptr_t val;\n  \
                 bool is_some;\n\
                 }} __private__OptionIsize;\n\
                 typedef struct __private__OptionF32 {{\n  \
                 float val;\n  \
                 bool is_some;\n\
                 }} __private__OptionF32;\n\
                 typedef struct __private__OptionF64 {{\n  \
                 double val;\n  \
                 bool is_some;\n\
                 }} __private__OptionF64;\n\
                 typedef struct __private__OptionBool {{\n  \
                 bool val;\n  \
                 bool is_some;\n\
                 }} __private__OptionBool;\n\
                 \n\
                 #endif /* RUST_BRIDGE_C_H */\n"
            );
            return Ok(Some(vec![GeneratedFile {
                path: header_path,
                content: minimal_header,
                generated_header: false,
            }]));
        }
    };

    let core_swift_src = out_dir.join("SwiftBridgeCore.swift");
    let crate_swift_src = out_dir
        .join(binding_crate_name)
        .join(format!("{binding_crate_name}.swift"));
    let core_h_src = out_dir.join("SwiftBridgeCore.h");
    let crate_h_src = out_dir.join(binding_crate_name).join(format!("{binding_crate_name}.h"));

    for p in [&core_swift_src, &crate_swift_src, &core_h_src, &crate_h_src] {
        if !p.exists() {
            return Ok(None);
        }
    }

    let core_swift = std::fs::read_to_string(&core_swift_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", core_swift_src.display()))?;
    let crate_swift = std::fs::read_to_string(&crate_swift_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", crate_swift_src.display()))?;
    let core_h = std::fs::read_to_string(&core_h_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", core_h_src.display()))?;
    let crate_h = std::fs::read_to_string(&crate_h_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", crate_h_src.display()))?;

    let core_swift_content = make_swift_bridge_ref_ptr_public(&append_rust_string_ref_to_string_extension(
        &add_retroactive_to_imported_protocol_conformances(&prepend_rust_bridge_c_import(&core_swift)),
    ));
    let crate_swift_content = make_swift_bridge_ref_ptr_public(&prepend_rust_bridge_c_import(&crate_swift));

    let sources_rust_bridge = package_root.join("Sources").join("RustBridge");
    let sources_rust_bridge_c = package_root.join("Sources").join("RustBridgeC");

    // Two files existing under `out/` is not on its own evidence that concatenating them
    // yields a usable header, and this writer is unguarded -- it goes straight to disk. The
    // committed header is therefore passed in so a partial assembly can be refused in favor of
    // it, and so an assembly that only differs from it by formatting keeps the committed bytes
    // instead of restarting the reformat/re-stamp churn. ~keep
    let committed_header = std::fs::read_to_string(sources_rust_bridge_c.join("RustBridgeC.h")).ok();
    let rust_bridge_c_h =
        umbrella_header::resolve_fresh(binding_crate_name, &core_h, &crate_h, committed_header.as_deref())
            .map_err(|e| anyhow::anyhow!("{e:#} (swift-bridge build output read from {})", out_dir.display()))?;
    let _ = crate_name;
    let files = vec![
        GeneratedFile {
            path: sources_rust_bridge.join("SwiftBridgeCore.swift"),
            content: core_swift_content,
            generated_header: false,
        },
        GeneratedFile {
            path: sources_rust_bridge.join(format!("{binding_crate_name}.swift")),
            content: crate_swift_content,
            generated_header: false,
        },
        GeneratedFile {
            path: sources_rust_bridge_c.join("RustBridgeC.h"),
            content: rust_bridge_c_h,
            generated_header: false,
        },
    ];
    Ok(Some(files))
}

/// Write `files` (the trio [`emit_swift_bridge_files`] produces) to disk, normalizing each
/// file's content the same way the standard `write_files`/`write_files_report` pipeline does
/// before hashing.
///
/// `emit_swift_bridge_files`' real (non-placeholder) output is read straight off swift-bridge's
/// own build output (`out_dir.join("SwiftBridgeCore.h")` and friends -- an external tool's
/// generated files, not alef's own template output) and is written here by
/// `PostBuildStep::MaterializeSwiftBridge`, the one write path in the whole `alef build` command
/// that goes straight to `std::fs::write` instead of through that pipeline. Any trailing
/// whitespace or missing final newline swift-bridge's own codegen happens to emit (e.g. a
/// `#include <stdbool.h>` line with a trailing space) therefore reached the committed
/// `RustBridgeC.h`/`*.swift` files uncorrected until this normalized every line the same way the
/// in-process placeholder path already does. ~keep
pub(crate) fn write_materialized_files(files: Vec<GeneratedFile>, project_root: &Path) -> anyhow::Result<()> {
    for f in files {
        if let Some(parent) = f.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("failed to create directory {}: {e}", parent.display()))?;
        }
        let normalized = crate::cli::pipeline::normalize_content(project_root, &f.path, &f.content);
        std::fs::write(&f.path, &normalized)
            .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", f.path.display()))?;
    }
    Ok(())
}

/// How many files a real materialization writes: `SwiftBridgeCore.swift`,
/// `{binding_crate}.swift` and `RustBridgeC.h`. The placeholder branch of
/// [`emit_swift_bridge_files`] also returns `Some`, of the header alone, so the count -- not
/// `Option::is_some` -- is what separates "copied fresh build output" from "wrote a stand-in".
const MATERIALIZED_TRIO_LEN: usize = 3;

/// Run `PostBuildStep::MaterializeSwiftBridge`: read swift-bridge's own build output and write the
/// trio it produces into `package_root`.
///
/// Reports what was actually written, and fails when build output that was expected is missing.
/// Before this existed the caller logged one unconditional success line outside the `if let` that
/// guarded the write, so all three outcomes -- trio copied, placeholder written, nothing written
/// at all -- rendered as the same line and exit 0, which is how alef #477 (`CARGO_TARGET_DIR`
/// pointing outside the project) shipped stale committed bridge files while claiming to have
/// refreshed them.
///
/// The three outcomes are told apart by [`emit_swift_bridge_files`]'s return value alone, and the
/// distinction that matters is *not* "is Swift configured" -- this step only exists in a build
/// plan the Swift backend built, so a Swift package is configured by construction. It is "has
/// this crate ever been built here":
///
/// - `Some(trio)`: fresh output found and copied. The success line, with its count.
/// - `Some(placeholder)`: no build output *and* no populated committed header, i.e. a project's
///   first generation before `{binding_crate_name}` has ever been compiled. Legitimate, and
///   deliberately not an error -- `core::backend`'s
///   `materialize_swift_bridge_does_not_claim_trio_members_it_never_wrote` pins that this is the
///   normal first-run state.
/// - `None`: either a populated header is already committed but this run found no build output,
///   or the output directory exists with part of the four-file set missing. Both mean a build
///   that was supposed to have run did not leave usable output where this step looks, which is
///   exactly #477's shape -- so both are errors now. ~keep
pub(crate) fn materialize_from_build_output(
    binding_crate_name: &str,
    package_root: &Path,
    project_root: &Path,
) -> anyhow::Result<()> {
    let materialized = emit_swift_bridge_files("", binding_crate_name, package_root, true).map_err(|e| {
        anyhow::anyhow!("failed to re-materialize swift-bridge files for '{binding_crate_name}': {e:#}")
    })?;
    let Some(files) = materialized else {
        let searched = swift_bridge_target_root()
            .map(|root| root.display().to_string())
            .unwrap_or_else(|| "<no Cargo.lock found above the current directory>".to_owned());
        anyhow::bail!(
            "no usable swift-bridge build output for '{binding_crate_name}': searched \
             {searched}/{{release,debug}}/build/{binding_crate_name}-*/out for SwiftBridgeCore.swift, \
             SwiftBridgeCore.h, {binding_crate_name}/{binding_crate_name}.swift and \
             {binding_crate_name}/{binding_crate_name}.h. The committed files under {} were left unchanged. \
             Build '{binding_crate_name}' (its build script populates OUT_DIR) and run alef under the same \
             cargo target directory as that build",
            package_root.display()
        );
    };
    let written = files.len();
    write_materialized_files(files, project_root)
        .map_err(|e| anyhow::anyhow!("failed to write swift-bridge files for '{binding_crate_name}': {e:#}"))?;
    if written == MATERIALIZED_TRIO_LEN {
        tracing::info!(
            "Re-materialized {written} swift-bridge files for '{binding_crate_name}' from fresh build output"
        );
    } else {
        tracing::info!(
            "Wrote {written} placeholder swift-bridge file(s) for '{binding_crate_name}': the crate has not been \
             built here yet, so no swift-bridge build output exists to copy"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "bridge_artifacts/materialize_write_tests.rs"]
mod materialize_write_tests;

pub(super) fn emit_inbound_protocols(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_types: &HashSet<String>,
    out: &mut String,
) {
    for bridge_cfg in config.trait_bridges_for(Language::Swift) {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        if bridge_cfg.exclude_languages.iter().any(|l| l == "swift") {
            continue;
        }
        let trait_name = &bridge_cfg.trait_name;
        let type_alias = match bridge_cfg.type_alias.as_deref() {
            Some(a) => a,
            None => continue,
        };
        let Some(options_type) = bridge_cfg.options_type.as_deref() else {
            continue;
        };
        let Some(field) = bridge_cfg.resolved_options_field() else {
            continue;
        };
        let result_type_name = bridge_cfg.result_type.as_deref();
        let protocol_return_type = result_type_name.unwrap_or("Void");

        let Some(trait_def) = api.types.iter().find(|t| t.is_trait && t.name == *trait_name) else {
            continue;
        };

        let result_enum = result_type_name.and_then(|name| api.enums.iter().find(|e| e.name == name));
        let box_name = format!("Swift{trait_name}Box");
        let adapter_name = format!("_{trait_name}ProtocolAdapter");
        let protocol_name = format!("{trait_name}Protocol");
        let delegate_protocol_name = format!("_Swift{trait_name}BoxDelegate");
        let factory_fn = format!("make{}Handle", trait_name.to_upper_camel_case());

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_protocol_open.swift.jinja",
            minijinja::context! { protocol_name => &protocol_name, },
        ));
        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let method_camel = method_snake.to_lower_camel_case();
            let params = swift_protocol_params(method, exclude_types);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_protocol_method.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    params => &params,
                    return_type => protocol_return_type,
                },
            ));
        }
        out.push_str("}\n\n");

        let default_case = result_enum
            .and_then(|en| en.variants.iter().find(|v| v.fields.is_empty()))
            .map(|v| swift_case_ident(&v.name.to_lower_camel_case()));
        let default_case_doc = default_case.as_ref().map(|case| format!(".{case}"));
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_protocol_default_open.swift.jinja",
            minijinja::context! {
                protocol_name => &protocol_name,
                default_case => default_case_doc.as_deref(),
            },
        ));
        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let method_camel = method_snake.to_lower_camel_case();
            let underscore_params = swift_protocol_underscore_params(method, exclude_types);
            let (return_type, body) = if let Some(default_case) = &default_case {
                (Some(protocol_return_type), format!("return .{default_case}"))
            } else {
                (None, String::new())
            };
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_protocol_default_method.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    params => &underscore_params,
                    return_type => return_type,
                    body => &body,
                },
            ));
        }
        out.push_str("}\n\n");

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_adapter_open.swift.jinja",
            minijinja::context! {
                adapter_name => &adapter_name,
                delegate_protocol_name => &delegate_protocol_name,
                protocol_name => &protocol_name,
            },
        ));
        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let method_camel = method_snake.to_lower_camel_case();
            let delegate_method = swift_ident(&method_camel);
            let delegate_params = swift_box_params(method);
            let (conversion_lines, call_args) = swift_adapter_conversions(method, exclude_types);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_adapter_method_open.swift.jinja",
                minijinja::context! {
                    method_name => &delegate_method,
                    params => &delegate_params,
                },
            ));
            for line in &conversion_lines {
                out.push_str(&crate::backends::swift::template_env::render(
                    "swift_forwarder_conversion_line.swift.jinja",
                    minijinja::context! { line => line, },
                ));
            }
            let result_json = if let Some(result_type_name) = result_type_name.filter(|_| result_enum.is_some()) {
                format!(
                    "        return {}_toJson(inner.{method_camel}({call_args}))\n",
                    result_type_name.to_snake_case()
                )
            } else {
                let call = if call_args.is_empty() {
                    format!("inner.{method_camel}()")
                } else {
                    format!("inner.{method_camel}({call_args})")
                };
                crate::backends::swift::template_env::render(
                    "swift_bridge_adapter_void_return.swift.jinja",
                    minijinja::context! { call => &call, },
                )
            };
            out.push_str(&result_json);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_adapter_method_close.swift.jinja",
                minijinja::context! {},
            ));
        }
        out.push_str("}\n\n");

        if let Some(en) = result_enum {
            let result_type_name = en.name.as_str();
            let fn_name = format!("{}_toJson", result_type_name.to_snake_case());
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_result_helper_open.swift.jinja",
                minijinja::context! {
                    result_type_name => result_type_name,
                    function_name => &fn_name,
                },
            ));
            let repr = serde_enum_repr(en);
            for variant in &en.variants {
                let variant_name = &variant.name;
                let swift_case = swift_case_ident(&variant_name.to_lower_camel_case());
                let variant_wire = crate::codegen::naming::wire_variant_value(
                    variant_name,
                    variant.serde_rename.as_deref(),
                    en.serde_rename_all.as_deref(),
                );
                if variant.fields.is_empty() {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "swift_bridge_result_unit_case.swift.jinja",
                        minijinja::context! {
                            swift_case => &swift_case,
                            variant_wire => &variant_wire,
                            repr => repr.kind(),
                            tag_key => repr.tag(),
                        },
                    ));
                } else if variant.is_tuple && variant.fields.len() == 1 {
                    assert!(
                        !matches!(repr, SerdeEnumRepr::Internal { .. }),
                        "`{}::{variant_name}` is a newtype variant of an internally tagged enum \
                         (`#[serde(tag = \"...\")]` with no `content`). serde cannot serialize \
                         that at all — it fails with `cannot serialize tagged newtype variant` — \
                         so there is no correct JSON for the Swift bridge to emit. Add \
                         `#[serde(content = \"...\")]` to make the enum adjacently tagged, or give \
                         the variant named fields.",
                        en.name
                    );
                    out.push_str(&crate::backends::swift::template_env::render(
                        "swift_bridge_result_newtype_case.swift.jinja",
                        minijinja::context! {
                            swift_case => &swift_case,
                            variant_wire => &variant_wire,
                            repr => repr.kind(),
                            tag_key => repr.tag(),
                            content_key => repr.content(),
                        },
                    ));
                }
            }
            out.push_str("    }\n}\n\n");
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_json_escape_helper.swift.jinja",
                minijinja::context! {},
            ));
            out.push('\n');
        }

        let opts_snake = options_type.to_snake_case();
        let options_fn = format!("{opts_snake}FromJsonWith{}", field.to_upper_camel_case()).to_lower_camel_case();
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_factory.swift.jinja",
            minijinja::context! {
                protocol_name => &protocol_name,
                type_alias => type_alias,
                options_fn => &options_fn,
                factory_fn => &factory_fn,
                box_name => &box_name,
                adapter_name => &adapter_name,
            },
        ));
        out.push('\n');

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_options_forwarder.swift.jinja",
            minijinja::context! {
                options_type => options_type,
                type_alias => type_alias,
                options_fn => &options_fn,
                field => &field,
            },
        ));
        out.push('\n');
    }
}

pub(super) fn already_emitted_top_level_names(api: &ApiSurface) -> HashSet<String> {
    let mut names: HashSet<String> = HashSet::new();
    for func in &api.functions {
        if func.is_async {
            continue;
        }
        let first = func.params.first().map(|p| &p.ty);
        let is_bytes_or_path = matches!(first, Some(TypeRef::Bytes) | Some(TypeRef::Path));
        if !is_bytes_or_path {
            continue;
        }
        if convenience_name_shadows_bridge(func) {
            continue;
        }
        let swift_inner = swift_ident(&func.name.to_lower_camel_case());
        let wrapper_name = if swift_inner.ends_with("Sync") {
            swift_inner[..swift_inner.len() - 4].to_string()
        } else {
            swift_inner
        };
        names.insert(wrapper_name);
    }
    names
}

pub(super) fn emit_ref_property_extensions(api: &ApiSurface) -> Option<(String, String)> {
    let eligible_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && !t.methods.is_empty())
        .collect();

    if eligible_types.is_empty() {
        return None;
    }

    let mut content = String::new();
    content.push_str("import RustBridge\n\n");
    content.push_str("// MARK: - Property-access ergonomics for e2e tests\n");
    content.push_str("//\n");
    content.push_str("// This file provides computed-property aliases for methods on swift-bridge-generated types,\n");
    content.push_str("// allowing callers to write `result.mimeType` rather than `result.mimeType()`.\n");
    content.push_str("// These extensions are especially useful in e2e test assertions where the alef\n");
    content.push_str("// fixture generator emits property-access syntax.\n");
    content.push_str("//\n");
    content.push_str("// Although these are primarily for test convenience, they are part of the public API\n");
    content.push_str("// and can be used in production code for more ergonomic access to generated ref types.\n");

    let mut has_any_extensions = false;

    for ty in eligible_types {
        let mut type_has_extensions = false;
        let mut type_content = String::new();
        for method in &ty.methods {
            if method.is_async || method.is_static || method.binding_excluded {
                continue;
            }
            if !matches!(&method.return_type, TypeRef::String) || method.params.is_empty() {
                continue;
            }
            if !method.params.iter().all(|p| is_extension_param_bridgeable(&p.ty, api)) {
                continue;
            }

            if !type_has_extensions {
                type_content.push('\n');
                type_content.push_str(&crate::backends::swift::template_env::render(
                    "swift_ref_extension_open.swift.jinja",
                    minijinja::context! { type_name => &ty.name, },
                ));
                type_has_extensions = true;
            } else {
                type_content.push('\n');
            }

            let camel = method.name.to_lower_camel_case();
            type_content.push_str(&crate::backends::swift::template_env::render(
                "swift_ref_string_alias_property.swift.jinja",
                minijinja::context! {
                    method_name => &camel,
                    property_name => &camel,
                },
            ));
        }
        if type_has_extensions {
            type_content.push_str("}\n");
            type_content.push_str(&crate::backends::swift::template_env::render(
                "swift_ref_extension_inheritance_comment.swift.jinja",
                minijinja::context! {
                    type_name => &ty.name,
                },
            ));
            content.push_str(&type_content);
            has_any_extensions = true;
        }
    }

    if has_any_extensions {
        Some(("RustBridgeRefExtensions.swift".to_string(), content))
    } else {
        None
    }
}

fn convenience_name_shadows_bridge(func: &FunctionDef) -> bool {
    let swift_name = swift_ident(&func.name.to_lower_camel_case());
    let wrapper_name = if swift_name.ends_with("Sync") {
        swift_name[..swift_name.len() - 4].to_string()
    } else {
        swift_name.clone()
    };
    wrapper_name == swift_name
}

fn swift_protocol_params(method: &MethodDef, exclude_types: &HashSet<String>) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let name = p.name.to_lower_camel_case();
            let ty = swift_inbound_type(&p.ty, p.optional, exclude_types);
            format!("_ {name}: {ty}")
        })
        .collect();
    params.join(", ")
}

fn swift_protocol_underscore_params(method: &MethodDef, exclude_types: &HashSet<String>) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let name = p.name.to_lower_camel_case();
            let ty = swift_inbound_type(&p.ty, p.optional, exclude_types);
            format!("_ _{name}: {ty}")
        })
        .collect();
    params.join(", ")
}

fn swift_inbound_type(ty: &TypeRef, optional: bool, exclude_types: &HashSet<String>) -> String {
    use crate::core::ir::PrimitiveType;
    let inner = match ty {
        TypeRef::Named(name) if exclude_types.contains(name) => "String".to_string(),
        TypeRef::Named(name) => name.clone(),
        TypeRef::String => "String".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "Int".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Vec(inner) => format!("RustVec<{}>", swift_box_ffi_type(inner, false)),
        TypeRef::Optional(inner) => return format!("{}?", swift_inbound_type(inner, false, exclude_types)),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Bytes => "RustVec<UInt8>".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "Double".to_string(),
        TypeRef::Map(_, _) => "String".to_string(),
    };
    if optional { format!("{inner}?") } else { inner }
}

fn append_rust_string_ref_to_string_extension(content: &str) -> String {
    const MARKER: &str = "// alef: RustStringRef.toString() shim";
    if let Some(idx) = content.find(MARKER) {
        let mut head = content[..idx].to_string();
        while head.ends_with('\n') {
            head.pop();
        }
        head.push('\n');
        head
    } else {
        content.to_string()
    }
}

fn make_swift_bridge_ref_ptr_public(content: &str) -> String {
    content
        .replace(
            "    var ptr: UnsafeMutableRawPointer",
            "    public var ptr: UnsafeMutableRawPointer",
        )
        .replace("    var isOwned: Bool = true", "    public var isOwned: Bool = true")
}

fn add_retroactive_to_imported_protocol_conformances(content: &str) -> String {
    const TARGETS: &[(&str, &str)] = &[
        (
            "extension RustStr: Identifiable",
            "extension RustStr: @retroactive Identifiable",
        ),
        (
            "extension RustStr: Equatable",
            "extension RustStr: @retroactive Equatable",
        ),
    ];
    let mut out = content.to_string();
    for (from, to) in TARGETS {
        out = out.replace(from, to);
    }
    out
}

fn prepend_rust_bridge_c_import(content: &str) -> String {
    const IMPORT: &str = "import RustBridgeC";
    const IGNORE: &str = "// swift-format-ignore-file";
    let head: Vec<&str> = content.lines().take(5).collect();
    let has_import = head.iter().any(|l| l.trim() == IMPORT);
    let has_ignore = head.iter().any(|l| l.trim() == IGNORE);
    match (has_import, has_ignore) {
        (true, true) => content.to_string(),
        (true, false) => format!("{IGNORE}\n{content}"),
        (false, true) => format!("{IMPORT}\n\n{content}"),
        (false, false) => format!("{IGNORE}\n{IMPORT}\n\n{content}"),
    }
}

fn is_extension_param_bridgeable(ty: &TypeRef, api: &ApiSurface) -> bool {
    match ty {
        TypeRef::Named(n) if n.starts_with("Result") || n == "Result" => false,
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Path
        | TypeRef::Bytes
        | TypeRef::Duration
        | TypeRef::Unit => true,
        TypeRef::Named(n) => {
            if let Some(enum_def) = api.enums.iter().find(|e| &e.name == n) {
                enum_def.has_serde
            } else {
                true
            }
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_extension_param_bridgeable(inner, api),
        TypeRef::Map(..) | TypeRef::Char | TypeRef::Json => false,
    }
}

#[cfg(test)]
mod tests {
    use super::emit_swift_bridge_files;

    /// The alef #A regression, isolated to this function: `alef generate` must call this
    /// with `consult_build_output: false`, which skips `find_swift_bridge_out_dir` (and
    /// therefore `target/`) entirely -- so two calls against the same starting disk state
    /// return the same thing, independent of whatever this same command's own post-build
    /// step may have populated under `target/` between them. Two calls back-to-back stand
    /// in for two consecutive `alef generate` runs over an unchanged tree. ~keep
    #[test]
    fn placeholder_header_is_emitted_identically_across_two_consecutive_calls() {
        let package_root = tempfile::tempdir().expect("temp package root");

        let first = emit_swift_bridge_files("sample_lib", "sample-lib-swift", package_root.path(), false)
            .expect("first call")
            .expect("placeholder header expected when nothing exists yet");
        let second = emit_swift_bridge_files("sample_lib", "sample-lib-swift", package_root.path(), false)
            .expect("second call")
            .expect("placeholder header expected when nothing exists yet");

        assert_eq!(first.len(), 1, "only the placeholder header, not the real trio");
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].path, second[0].path);
        assert_eq!(first[0].content, second[0].content);
        assert!(
            !first[0].content.contains("__swift_bridge__$"),
            "a placeholder must never claim swift-bridge's real declarations"
        );
    }

    /// Companion to the placeholder case: once a prior build has populated the header,
    /// `consult_build_output: false` must keep answering "nothing new to write" on every
    /// later call, not just the first -- see `emit_swift_bridge_files`'s doc for why
    /// re-deriving content here (rather than leaving it to `MaterializeSwiftBridge`) would
    /// disagree with `normalize_content` and get refused as foreign. ~keep
    #[test]
    fn populated_header_yields_nothing_to_write_across_two_consecutive_calls() {
        let package_root = tempfile::tempdir().expect("temp package root");
        let header_dir = package_root.path().join("Sources").join("RustBridgeC");
        std::fs::create_dir_all(&header_dir).expect("create RustBridgeC dir");
        std::fs::write(
            header_dir.join("RustBridgeC.h"),
            "#ifndef RUST_BRIDGE_C_H\n#define RUST_BRIDGE_C_H\nvoid __swift_bridge__$example(void);\n#endif\n",
        )
        .expect("seed a populated header");

        let first =
            emit_swift_bridge_files("sample_lib", "sample-lib-swift", package_root.path(), false).expect("first call");
        let second =
            emit_swift_bridge_files("sample_lib", "sample-lib-swift", package_root.path(), false).expect("second call");

        assert!(
            first.is_none(),
            "a populated header has nothing new for this path to write"
        );
        assert!(second.is_none(), "must stay stable, not just true on the first call");
    }
}
