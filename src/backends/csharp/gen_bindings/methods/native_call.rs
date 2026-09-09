//! The "call the P/Invoke stub and judge its result" block shared by the four wrapper bodies in
//! `wrappers.rs` (free function and method, each sync and async).
//!
//! Those four sites had four copies of the same sequence, and the presence channel needs the
//! argument list *before* the primary call is emitted — the companion clears the FFI crate's
//! last-error slot on entry, so it has to run first or it wipes an error the primary just
//! recorded. Rather than hoist the argument construction four times, the whole sequence lives
//! here once: build args, gate on presence, call, judge. ~keep

use super::super::marshalling::{returns_ptr, zero_sentinel};
use super::super::result_presence::presence_gate;
use crate::backends::csharp::template_env::render;
use crate::core::ir::{ReceiverKind, TypeRef};

/// How a wrapper family reports a failed primary call whose return value is pointer-shaped.
///
/// The two families genuinely differ and always have: free-function wrappers consult the crate's
/// last-error slot, method wrappers test the returned sentinel itself. Modelled rather than
/// unified so this refactor stays behaviour-preserving for every shape but the one it fixes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FailureCheck {
    LastError,
    NullSentinel,
}

/// One wrapper's primary P/Invoke call, with everything needed to emit it and judge its result.
pub(super) struct NativeCall<'a> {
    pub cs_native_name: &'a str,
    pub return_type: &'a TypeRef,
    /// The IR receiver, passed through to the presence-companion eligibility authority. `None`
    /// for a free function or a static method — the same distinction the FFI backend makes.
    pub receiver: Option<&'a ReceiverKind>,
    pub has_error_type: bool,
    /// The positional arguments, receiver included, exactly as the primary call passes them.
    pub args: Vec<String>,
    /// The indent the `var nativeResult = ...` line and the `);` sit on.
    pub call_indent: &'a str,
    /// The indent each argument line sits on.
    pub argument_indent: &'a str,
    pub failure_check: FailureCheck,
}

impl NativeCall<'_> {
    pub(super) fn emit(&self, out: &mut String) {
        if let Some(gate) = presence_gate(
            self.return_type,
            self.receiver,
            self.cs_native_name,
            &self.args.join(", "),
            self.call_indent,
            &self.gate_failure_block(),
        ) {
            out.push_str(&gate);
        }

        out.push_str(self.call_indent);
        if *self.return_type != TypeRef::Unit || self.has_error_type {
            out.push_str("var nativeResult = ");
        }
        out.push_str(
            render(
                "native_call_start.jinja",
                minijinja::context! { method_name => self.cs_native_name },
            )
            .trim_end_matches('\n'),
        );

        if self.args.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (index, arg) in self.args.iter().enumerate() {
                out.push_str(
                    render(
                        "indented_arg.jinja",
                        minijinja::context! { arg, indent => self.argument_indent },
                    )
                    .trim_end_matches('\n'),
                );
                if index < self.args.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(self.call_indent);
            out.push_str(");\n");
        }

        self.emit_result_check(out);
    }

    /// The throw the presence gate runs when the companion answers `-1` rather than `0`.
    ///
    /// Always the last-error idiom, never the sentinel one: at gate time `nativeResult` does not
    /// exist yet. Emitted only for a fallible wrapper, mirroring Go's `wrapper_returns_error`. ~keep
    fn gate_failure_block(&self) -> String {
        if !self.has_error_type {
            return String::new();
        }
        render(
            "last_error_throw.jinja",
            minijinja::context! { indent => format!("{}    ", self.call_indent) },
        )
    }

    fn emit_result_check(&self, out: &mut String) {
        if *self.return_type == TypeRef::Unit {
            if self.has_error_type {
                out.push_str(&render(
                    "status_error_throw.jinja",
                    minijinja::context! { indent => self.call_indent },
                ));
            }
            return;
        }
        if returns_ptr(self.return_type) {
            let zero = zero_sentinel(self.return_type);
            let template = if matches!(self.return_type, TypeRef::Optional(_)) {
                "null_result_return.jinja"
            } else if self.failure_check == FailureCheck::NullSentinel {
                "null_sentinel_throw.jinja"
            } else {
                "last_error_throw.jinja"
            };
            out.push_str(&render(
                template,
                minijinja::context! { indent => self.call_indent, zero },
            ));
            return;
        }
        // A scalar-shaped return carries no sentinel of its own, so absence (when it is even
        // expressible) was already reported by the presence gate above and only a genuine error
        // is left to check. Before the gate existed this arm was unreachable for every
        // `Optional` return, because `returns_ptr` claimed them all. ~keep
        if self.has_error_type {
            out.push_str(&render(
                "last_error_throw.jinja",
                minijinja::context! { indent => self.call_indent },
            ));
        }
    }
}
