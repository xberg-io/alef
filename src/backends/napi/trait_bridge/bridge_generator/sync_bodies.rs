//! Synchronous trait-method bodies: the on-JS-thread direct call and the off-thread slow path.

use super::*;

impl NapiBridgeGenerator {
    /// Body of the `<method>_on_js_thread` private inherent method: today's direct-call shape,
    /// unchanged — `self.env()`/`self.obj(&env)`/`func.call(..)` are safe here because this path
    /// only ever runs on the JS thread (checked by the caller in
    /// [`TraitBridgeGenerator::gen_sync_method_body`]).
    pub(super) fn gen_sync_on_js_thread_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let js_method_name = to_camel_case(&method.name);
        let snake_method_name = method.name.clone();
        let has_error = method.error_type.is_some();

        let js_args_exprs = build_napi_args(
            method,
            spec.bridge_config,
            &self.struct_param_types,
            &self.type_prefix,
            &spec.trait_def.name,
            &self.unsupported_args,
            &self.json_safe_named_types(),
        );
        let (args_tuple_ty, empty_args, tuple_args) = args_tuple_pieces(&js_args_exprs);

        let error_lookup =
            spec.make_error("format!(\"Method '{}' not found on bridge object: {}\", self.cached_name, e)");
        let error_call = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{}' failed: {{}}\", self.cached_name, e)",
            method.name
        ));

        let has_default_impl = method.has_default_impl;
        if matches!(method.return_type, TypeRef::Unit) {
            crate::backends::napi::template_env::render(
                "sync_method_unit_return.jinja",
                crate::alef_context! {
                    wrapper => spec.wrapper_name(),
                    method_name => &js_method_name,
                    snake_case_method_name => &snake_method_name,
                    args_tuple_ty => args_tuple_ty,
                    has_error => has_error,
                    has_default_impl => has_default_impl,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                },
            )
        } else {
            self.gen_sync_on_js_thread_non_unit_body(
                method,
                spec,
                &SyncOnJsThreadArgs {
                    js_method_name: &js_method_name,
                    snake_method_name: &snake_method_name,
                    has_error,
                    has_default_impl,
                    args_tuple_ty: &args_tuple_ty,
                    empty_args,
                    tuple_args: &tuple_args,
                    error_lookup: &error_lookup,
                    error_call: &error_call,
                },
            )
        }
    }

    /// Non-`Unit`-return half of [`Self::gen_sync_on_js_thread_body`]. Decodes the JS method's
    /// return value through the same `AlefJsReply<T>` + [`Self::plan_return_decode`] machinery
    /// the async and off-thread slow paths use, instead of the string-coercion +
    /// `serde_json::from_str::<T>` the `sync_method_non_unit_return.jinja` template used to
    /// render: coercing a bare JS string return (e.g. `Renderer::render_result`'s `String`) to
    /// its own JSON text and re-parsing it as JSON fails on every call, because an unquoted
    /// string is not valid JSON — only numeric-ish returns happened to survive by coincidence.
    /// Declaring the JS `Function`'s return type as `AlefJsReply<{reply_ty}>` lets napi decode
    /// it natively (a `Promise` return is a protocol error for a declared-synchronous method,
    /// reported by [`AlefJsReply::settle_sync`] rather than silently polled once).
    fn gen_sync_on_js_thread_non_unit_body(
        &self,
        method: &MethodDef,
        spec: &TraitBridgeSpec,
        args: &SyncOnJsThreadArgs<'_>,
    ) -> String {
        let SyncOnJsThreadArgs {
            js_method_name,
            snake_method_name,
            has_error,
            has_default_impl,
            args_tuple_ty,
            empty_args,
            tuple_args,
            error_lookup,
            error_call,
        } = *args;
        let decode = self.plan_return_decode(&method.return_type);
        let reply_ty = decode.reply_ty().to_string();
        let is_identity = matches!(decode, ReturnDecode::Native { .. });

        let error_settle = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{}' rejected: {{}}\", self.cached_name, e)",
            method.name
        ));
        let error_parse = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' failed to parse return value for method '{}': {{}}\", self.cached_name, e)",
            method.name
        ));

        // Mirrors `sync_method_unit_return.jinja`'s missing-object/missing-property arms: a
        // default-method bridge treats either as a no-op (substitute the default), a required
        // fallible method propagates, a required infallible method logs and substitutes.
        let missing_arm = |infallible_warn_step: &str| -> String {
            if has_default_impl {
                let default_expr = if has_error {
                    "Ok(Default::default())"
                } else {
                    "Default::default()"
                };
                format!("Err(_) => return {default_expr},")
            } else if has_error {
                format!("Err(e) => return Err({error_lookup}),")
            } else {
                format!(
                    "Err(e) => {{\n    \
                     tracing::warn!(wrapper = \"{}\", method = \"{}\", error = %e, \"{infallible_warn_step}; returning default\");\n    \
                     return Default::default();\n\
                     }}",
                    spec.wrapper_name(),
                    method.name,
                )
            }
        };
        let obj_missing_arm = missing_arm("bridge object unavailable");
        let prop_missing_arm = missing_arm("method not found on bridge object");

        let call_expr = if empty_args {
            "func.call(())".to_string()
        } else {
            format!("func.call(napi::bindgen_prelude::FnArgs::from({tuple_args}))")
        };

        let convert_fail_stmt = if has_error {
            format!("return Err({error_parse});")
        } else {
            format!(
                "tracing::warn!(wrapper = \"{}\", method = \"{}\", error = %e, \"host returned an unparseable value; returning default\");\n    return Default::default();",
                spec.wrapper_name(),
                method.name,
            )
        };
        let convert = if is_identity {
            String::new()
        } else {
            decode.convert_stmt(&convert_fail_stmt)
        };
        let result_var = if is_identity { "__decoded" } else { "__result" };
        let tail = if has_error {
            format!("Ok({result_var})")
        } else {
            result_var.to_string()
        };

        let call_err_arm = if has_error {
            format!("Err(e) => return Err({error_call}),")
        } else {
            format!(
                "Err(e) => {{\n    \
                 tracing::warn!(wrapper = \"{wrapper}\", method = \"{method_name}\", error = %e, \"host callback threw; returning default\");\n    \
                 return Default::default();\n\
                 }}",
                wrapper = spec.wrapper_name(),
                method_name = method.name,
            )
        };
        let settle_err_arm = if has_error {
            format!("Err(e) => return Err({error_settle}),")
        } else {
            format!(
                "Err(e) => {{\n    \
                 tracing::warn!(wrapper = \"{wrapper}\", method = \"{method_name}\", error = %e, \"host callback rejected; returning default\");\n    \
                 return Default::default();\n\
                 }}",
                wrapper = spec.wrapper_name(),
                method_name = method.name,
            )
        };

        format!(
            "let __env = self.env();\n\
             let __obj = match self.obj(&__env) {{\n    Ok(o) => o,\n    {obj_missing_arm}\n}};\n\
             let func: napi::bindgen_prelude::Function<{args_tuple_ty}, AlefJsReply<{reply_ty}>> = \
             match __obj.get_named_property(\"{js_method_name}\").or_else(|_| __obj.get_named_property(\"{snake_method_name}\")) {{\n    \
             Ok(f) => f,\n    {prop_missing_arm}\n}};\n\
             let __reply: AlefJsReply<{reply_ty}> = match {call_expr} {{\n    Ok(v) => v,\n    {call_err_arm}\n}};\n\
             let __decoded: {reply_ty} = match __reply.settle_sync(&self.cached_name, \"{method_name}\") {{\n    \
             Ok(v) => v,\n    {settle_err_arm}\n}};\n\
             {convert}\n\
             {tail}",
            method_name = method.name,
        )
    }

    /// Body of the trait-impl method's off-JS-thread slow path: block synchronously on the
    /// method's threadsafe function via a bounded `mpsc` channel, re-checking `aborted()` on
    /// each wake so environment teardown surfaces as an error instead of hanging forever.
    pub(super) fn gen_sync_slow_path_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let tsfn_field = Self::tsfn_field_name(method);
        let has_error = method.error_type.is_some();
        let has_default_impl = method.has_default_impl && self.is_forwardable(method);
        let payload_expr = self.owned_payload_expr(method);

        let infallible_default = "Default::default()";
        // The trait-impl method is only ever entered here once the caller-visible presence
        // check has already passed: `gen_method_presence_check` (`self.<m>_tsfn.is_some()`)
        // wraps every forwardable method in a guard that returns the Rust-default delegate
        // BEFORE this body runs (see `default_method_guard.jinja` / `gen_bridge_trait_impl`), so
        // by the time control reaches here the `Option` is always `Some`. Unwrapping it a second
        // time with its own `Default::default()` fallback would not even type-check for a
        // fallible method (`Result<T, E>` has no `Default` impl) -- `.expect(..)` documents the
        // real invariant instead of re-deriving a (wrong) one.
        let missing_tsfn_arm = if has_default_impl {
            format!("let __tsfn = self.{tsfn_field}.as_ref().expect(\"presence-checked by the default-method guard\");")
        } else {
            format!("let __tsfn = &self.{tsfn_field};")
        };

        // The channel carries the RAW decode target (`ReturnDecode::reply_ty`, e.g. `JsReport` or
        // `serde_json::Value`), not the method's real return type -- `Ok(v) => v` on its own
        // (the shape this block had before) skipped `ReturnDecode::convert_stmt` entirely and
        // handed back the wrong type for every `Named`/`Optional`/`VecNamed`/`Json` return
        // (caught by a scratch-crate compile of the "enum return" fixture shape: `Ok(v) => v`
        // returned `serde_json::Value` where the method's signature said `Mode`). Every arm that
        // yields a settled value now goes through the same conversion the async path uses.
        let decode = if matches!(method.return_type, TypeRef::Unit) {
            None
        } else {
            Some(self.plan_return_decode(&method.return_type))
        };
        let reply_ty = decode
            .as_ref()
            .map(|d| d.reply_ty().to_string())
            .unwrap_or_else(|| "()".to_string());

        let error_parse = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' failed to parse return value for method '{}': {{}}\", self.cached_name, e)",
            method.name
        ));
        let convert_fail_stmt = if has_error {
            format!("return Err({error_parse});")
        } else {
            format!(
                "tracing::warn!(wrapper = \"{}\", method = \"{}\", error = %e, \"host returned an unparseable value; returning default\");\n    return {infallible_default};",
                spec.wrapper_name(),
                method.name
            )
        };
        // `Ok(v) => { <convert> <tail> }` — `convert` is `""` (identity) for a `Unit` return OR a
        // `ReturnDecode::Native` return (`__decoded` already IS the return type; see the doc on
        // the matching special case in `gen_async_method_body`, which explains why the redundant
        // `let __result = __decoded;` immediately before a bare `__result` tail must be skipped
        // rather than emitted -- `clippy::let_and_return` under the real gate's `-D warnings`).
        let convert_and_tail = |ok_prefix: &str, ok_suffix: &str| -> String {
            match &decode {
                None => format!("{ok_prefix}v{ok_suffix}"),
                Some(ReturnDecode::Native { .. }) => {
                    format!("{ok_prefix}v{ok_suffix}")
                }
                Some(d) => format!(
                    "{{ let __decoded = v; {convert} {ok_prefix}__result{ok_suffix} }}",
                    convert = d.convert_stmt(&convert_fail_stmt),
                ),
            }
        };

        let error_settle = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{}' rejected: {{}}\", self.cached_name, e)",
            method.name
        ));

        let recv_body = if has_error {
            format!(
                "let __settled: napi::Result<{reply_ty}> = match __rx.recv_timeout(std::time::Duration::from_millis(50)) {{\n\
                 \x20   Ok(v) => v,\n\
                 \x20   Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {{\n\
                 \x20       if __tsfn.aborted() {{\n\
                 \x20           return Err({error_call_timeout});\n\
                 \x20       }}\n\
                 \x20       continue;\n\
                 \x20   }}\n\
                 \x20   Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err({error_disconnected}),\n\
                 }};\n\
                 return match __settled {{\n\
                 \x20   Ok(v) => {ok_arm},\n\
                 \x20   Err(e) => Err({error_settle}),\n\
                 }};",
                error_call_timeout = spec.make_error(&format!(
                    "format!(\"Plugin '{{}}' method '{}' timed out\", self.cached_name)",
                    method.name
                )),
                error_disconnected = spec.make_error(&format!(
                    "format!(\"Plugin '{{}}' method '{}' channel disconnected before a reply arrived\", self.cached_name)",
                    method.name
                )),
                ok_arm = convert_and_tail("Ok(", ")"),
            )
        } else {
            format!(
                "let __settled: napi::Result<{reply_ty}> = match __rx.recv_timeout(std::time::Duration::from_millis(50)) {{\n\
                 \x20   Ok(v) => v,\n\
                 \x20   Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {{\n\
                 \x20       if __tsfn.aborted() {{\n\
                 \x20           tracing::warn!(wrapper = \"{wrapper}\", method = \"{method_name}\", \"threadsafe call timed out; returning default\");\n\
                 \x20           return {infallible_default};\n\
                 \x20       }}\n\
                 \x20       continue;\n\
                 \x20   }}\n\
                 \x20   Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {{\n\
                 \x20       tracing::warn!(wrapper = \"{wrapper}\", method = \"{method_name}\", \"threadsafe call channel disconnected; returning default\");\n\
                 \x20       return {infallible_default};\n\
                 \x20   }}\n\
                 }};\n\
                 return match __settled {{\n\
                 \x20   Ok(v) => {ok_arm},\n\
                 \x20   Err(e) => {{\n\
                 \x20       tracing::warn!(wrapper = \"{wrapper}\", method = \"{method_name}\", error = %e, \"host callback failed; returning default\");\n\
                 \x20       {infallible_default}\n\
                 \x20   }}\n\
                 }};",
                wrapper = spec.wrapper_name(),
                method_name = method.name,
                ok_arm = convert_and_tail("", ""),
            )
        };

        format!(
            "{missing_tsfn_arm}\n\
             let __payload = {payload_expr};\n\
             let __plugin_name = self.cached_name.clone();\n\
             let (__tx, __rx) = std::sync::mpsc::sync_channel::<napi::Result<{reply_ty}>>(1);\n\
             let __enqueue_status = __tsfn.call_with_return_value(\n\
             \x20   __payload,\n\
             \x20   napi::threadsafe_function::ThreadsafeFunctionCallMode::NonBlocking,\n\
             \x20   move |reply: napi::Result<AlefJsReply<{reply_ty}>>, _env: napi::Env| {{\n\
             \x20       // Declared synchronous, so there is no executor here to await a Promise: a host\n\
             \x20       // that returns one anyway is a protocol error (`settle_sync` reports it as such)\n\
             \x20       // rather than being polled once and misread as immediately ready.\n\
             \x20       let settled = reply.and_then(|r| r.settle_sync(&__plugin_name, \"{method_name}\"));\n\
             \x20       let _ = __tx.send(settled);\n\
             \x20       Ok(())\n\
             \x20   }},\n\
             );\n\
             if __enqueue_status != napi::Status::Ok {{\n\
             \x20   {enqueue_fail}\n\
             }}\n\
             loop {{\n{recv_body}\n}}",
            method_name = method.name,
            enqueue_fail = if has_error {
                format!("return Err({});", spec.make_error(&format!(
                    "format!(\"Plugin '{{}}' method '{}' could not be enqueued: {{:?}}\", self.cached_name, __enqueue_status)",
                    method.name
                )))
            } else {
                format!(
                    "tracing::warn!(wrapper = \"{}\", method = \"{}\", status = ?__enqueue_status, \"failed to enqueue threadsafe call; returning default\");\n    return {infallible_default};",
                    spec.wrapper_name(),
                    method.name
                )
            }
        )
    }
}
