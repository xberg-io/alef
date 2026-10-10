use crate::backends::java::type_map::{java_boxed_type, java_type};
use crate::codegen::mut_writeback;
use crate::codegen::naming::to_java_name;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

use super::super::helpers::is_bridge_param_java;

pub(super) fn gen_async_wrapper_method(
    out: &mut String,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    opaque_types: &AHashSet<String>,
) {
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = java_type(&p.ty);
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    // A `&mut T` DTO parameter on a unit-returning async function cannot resolve a
    // `CompletableFuture<Void>`: the sync overload this wraps (`sync_functions.rs`) already
    // returns the write-back's updated `T` instead of `void` (issue #380), so this overload must
    // declare `CompletableFuture<T>` and hand that value back instead of discarding it behind
    // `return null;`. `reject_unsupported_writeback` (called by `generate_bindings` before any
    // file is emitted) has already ruled out every `&mut` DTO shape this can't express. ~keep
    let writeback = mut_writeback::writeback_param(&func.params, &func.return_type, opaque_types);
    let return_type = match writeback {
        Some(wb) => java_boxed_type(&wb.ty).to_string(),
        None => match &func.return_type {
            TypeRef::Unit => "Void".to_string(),
            other => java_boxed_type(other).to_string(),
        },
    };

    let sync_method_name = to_java_name(&func.name);
    let async_method_name = format!("{}Async", sync_method_name);
    let param_names: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| to_java_name(&p.name))
        .collect();

    out.push_str(&crate::backends::java::template_env::render(
        "ffi_async_method_signature.jinja",
        crate::alef_context! {
            return_type => &return_type,
            async_method_name => &async_method_name,
            params => params.join(", "),
        },
    ));
    out.push_str("        return CompletableFuture.supplyAsync(() -> {\n");
    out.push_str("            try {\n");
    if writeback.is_none() && matches!(func.return_type, TypeRef::Unit) {
        out.push_str("                ");
        out.push_str(&sync_method_name);
        out.push('(');
        out.push_str(&param_names.join(", "));
        out.push_str(");\n");
        out.push_str("                return null;\n");
    } else {
        out.push_str("                return ");
        out.push_str(&sync_method_name);
        out.push('(');
        out.push_str(&param_names.join(", "));
        out.push_str(");\n");
    }
    out.push_str("            } catch (Throwable e) {\n");
    out.push_str("                throw new CompletionException(e);\n");
    out.push_str("            }\n");
    out.push_str("        });\n");
    out.push_str("    }\n");
}

#[cfg(test)]
mod tests {
    use super::gen_async_wrapper_method;
    use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
    use ahash::AHashSet;
    use std::collections::HashSet;

    fn record_param(is_ref: bool, is_mut: bool) -> ParamDef {
        ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref,
            is_mut,
            ..ParamDef::default()
        }
    }

    fn render(func: &FunctionDef, opaque_types: &AHashSet<String>) -> String {
        let mut out = String::new();
        let bridge_param_names = HashSet::new();
        let bridge_type_aliases = HashSet::new();
        gen_async_wrapper_method(&mut out, func, &bridge_param_names, &bridge_type_aliases, opaque_types);
        out
    }

    /// Regression test for issue #380 (Java async overload): a `&mut T` DTO parameter on a
    /// unit-returning `async fn` previously rendered the `*Async` overload as
    /// `CompletableFuture<Void> tagRecordAsyncAsync(final Record record)` with a body that called
    /// the (already write-back-corrected) sync method and threw its return value away with
    /// `return null;`. The future must instead resolve with the mutated value.
    #[test]
    fn java_async_overload_mut_dto_param_returns_the_updated_value() {
        let func = FunctionDef {
            name: "tag_record_async".to_owned(),
            rust_path: "sample_core::tag_record_async".to_owned(),
            params: vec![record_param(true, true)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = render(&func, &AHashSet::new());

        assert!(
            output.contains("CompletableFuture<Record>"),
            "expected the async overload to resolve with the mutated DTO type instead of Void:\n{output}"
        );
        assert!(
            !output.contains("CompletableFuture<Void>"),
            "must not still advertise a Void future:\n{output}"
        );
        // Load-bearing round-trip: the body must call the write-back-corrected sync method AND
        // hand its return value back through the future instead of discarding it with `return
        // null;`.
        assert!(
            output.contains("return tagRecordAsync(record);"),
            "expected the body to return the sync method's mutated result:\n{output}"
        );
        assert!(
            !output.contains("return null;"),
            "must not discard the mutated value behind `return null;`:\n{output}"
        );
    }

    /// Negative control for issue #380: an async immutable `&T` DTO param must not gain
    /// write-back semantics -- the future must stay `CompletableFuture<Void>`.
    #[test]
    fn java_async_overload_immutable_dto_param_keeps_void_future() {
        let func = FunctionDef {
            name: "read_record_async".to_owned(),
            rust_path: "sample_core::read_record_async".to_owned(),
            params: vec![record_param(true, false)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = render(&func, &AHashSet::new());

        assert!(
            output.contains("CompletableFuture<Void>"),
            "immutable borrow must keep a Void future:\n{output}"
        );
        assert!(
            output.contains("return null;"),
            "immutable borrow must not gain a write-back return:\n{output}"
        );
    }

    /// Negative control for issue #380: an async owned `T` DTO param must render unaffected by
    /// write-back.
    #[test]
    fn java_async_overload_owned_dto_param_unaffected_by_writeback() {
        let func = FunctionDef {
            name: "consume_record_async".to_owned(),
            rust_path: "sample_core::consume_record_async".to_owned(),
            params: vec![record_param(false, false)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = render(&func, &AHashSet::new());

        assert!(
            output.contains("CompletableFuture<Void>"),
            "owned param must keep a Void future:\n{output}"
        );
        assert!(
            output.contains("return null;"),
            "owned param must not gain a write-back return:\n{output}"
        );
    }

    /// Negative control: a normal (non-`&mut`) async function that already returns a value
    /// must render unaffected -- the future resolves with the already-correct return type.
    #[test]
    fn java_async_overload_non_unit_return_unaffected_by_writeback() {
        let func = FunctionDef {
            name: "make_record_async".to_owned(),
            rust_path: "sample_core::make_record_async".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("Record".to_owned()),
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = render(&func, &AHashSet::new());

        assert!(
            output.contains("CompletableFuture<Record>"),
            "expected the pre-existing non-unit return type to keep flowing through:\n{output}"
        );
        assert!(
            output.contains("return makeRecordAsync();"),
            "expected the body to return the sync method's result unmodified:\n{output}"
        );
    }
}
