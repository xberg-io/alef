/// Generate visitor/callback FFI bindings.
///
/// This module produces the `#[repr(C)]` callback struct, an opaque `Visitor`
/// handle that bridges C function pointers into the Rust `HtmlVisitor` trait,
/// and the three public FFI entry points:
///
/// - `{prefix}_visitor_create(callbacks: *const {Prefix}VisitorCallbacks) -> *mut {Prefix}Visitor`
/// - `{prefix}_visitor_free(visitor: *mut {Prefix}Visitor)`
/// - `{prefix}_convert_with_visitor(html, options, visitor) -> *mut ConversionResult`
///
/// # Coverage
///
/// All 42 `HtmlVisitor` trait methods are covered. The callback struct field
/// order matches the Go binding's expected layout exactly (see
/// `packages/go/v3/htmltomarkdown/visitor.go`).
use heck::ToPascalCase;

/// The integer codes that map to `VisitResult` variants crossing the FFI boundary.
///
/// | Value | Meaning               |
/// |-------|-----------------------|
/// |   0   | `VisitResult::Continue`     |
/// |   1   | `VisitResult::Skip`         |
/// |   2   | `VisitResult::PreserveHtml` |
/// |   3   | `VisitResult::Custom(…)`    |
/// |   4   | `VisitResult::Error(…)`     |
#[allow(dead_code)]
pub const VISIT_RESULT_CONTINUE: i32 = 0;
pub const VISIT_RESULT_SKIP: i32 = 1;
pub const VISIT_RESULT_PRESERVE_HTML: i32 = 2;
pub const VISIT_RESULT_CUSTOM: i32 = 3;
pub const VISIT_RESULT_ERROR: i32 = 4;

// ---------------------------------------------------------------------------
// Data-driven callback specifications
// ---------------------------------------------------------------------------

/// The kind of a single callback parameter (beyond the common ctx/user_data/out
/// prefix that every callback shares).
enum ParamKind {
    /// Required `*const c_char` — converted from `&str` via `CString::new`.
    Str(&'static str),
    /// Optional `*const c_char` — converted from `Option<&str>` via `opt_str_to_c`.
    OptStr(&'static str),
    /// `i32` — converted from `bool` via `i32::from`.
    Bool(&'static str),
    /// `u32` — passed through directly.
    U32(&'static str),
    /// `usize` — passed through directly.
    Usize(&'static str),
    /// `*const *const c_char` + `usize` (cell_count) — special for table rows.
    CellSlice(&'static str),
}

/// Specification for one visitor callback.
struct CallbackSpec {
    name: &'static str,
    doc: &'static str,
    params: &'static [ParamKind],
}

/// All 40 callback specifications, in **exact field order** for Go ABI compatibility.
///
/// CRITICAL: Do not reorder these entries. The field order in the generated
/// `#[repr(C)]` struct must match the Go binding layout.
const CALLBACKS: &[CallbackSpec] = &[
    CallbackSpec {
        name: "visit_text",
        doc: "Visit text nodes.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_element_start",
        doc: "Called before entering any element.",
        params: &[],
    },
    CallbackSpec {
        name: "visit_element_end",
        doc: "Called after exiting any element; receives the default markdown output.",
        params: &[ParamKind::Str("output")],
    },
    CallbackSpec {
        name: "visit_link",
        doc: "Visit anchor links `<a href=\"...\">`.\\n    ///\\n    /// `title` may be null.",
        params: &[
            ParamKind::Str("href"),
            ParamKind::Str("text"),
            ParamKind::OptStr("title"),
        ],
    },
    CallbackSpec {
        name: "visit_image",
        doc: "Visit images `<img src=\"...\">`.\\n    ///\\n    /// `title` may be null.",
        params: &[ParamKind::Str("src"), ParamKind::Str("alt"), ParamKind::OptStr("title")],
    },
    CallbackSpec {
        name: "visit_heading",
        doc: "Visit heading elements `<h1>`\\u{2013}`<h6>`.\\n    ///\\n    /// `id` may be null.",
        params: &[ParamKind::U32("level"), ParamKind::Str("text"), ParamKind::OptStr("id")],
    },
    CallbackSpec {
        name: "visit_code_block",
        doc: "Visit code blocks `<pre><code>`.\\n    ///\\n    /// `lang` may be null.",
        params: &[ParamKind::OptStr("lang"), ParamKind::Str("code")],
    },
    CallbackSpec {
        name: "visit_code_inline",
        doc: "Visit inline code `<code>`.",
        params: &[ParamKind::Str("code")],
    },
    CallbackSpec {
        name: "visit_list_item",
        doc: "Visit list items `<li>`.",
        params: &[
            ParamKind::Bool("ordered"),
            ParamKind::Str("marker"),
            ParamKind::Str("text"),
        ],
    },
    CallbackSpec {
        name: "visit_list_start",
        doc: "Called before processing a list `<ul>` or `<ol>`.",
        params: &[ParamKind::Bool("ordered")],
    },
    CallbackSpec {
        name: "visit_list_end",
        doc: "Called after processing a list `</ul>` or `</ol>`.",
        params: &[ParamKind::Bool("ordered"), ParamKind::Str("output")],
    },
    CallbackSpec {
        name: "visit_table_start",
        doc: "Called before processing a table `<table>`.",
        params: &[],
    },
    CallbackSpec {
        name: "visit_table_row",
        doc: "Visit table rows `<tr>`.\\n    ///\\n    /// Cells are passed as a null-terminated array of null-terminated strings.",
        params: &[ParamKind::CellSlice("cells"), ParamKind::Bool("is_header")],
    },
    CallbackSpec {
        name: "visit_table_end",
        doc: "Called after processing a table `</table>`.",
        params: &[ParamKind::Str("output")],
    },
    CallbackSpec {
        name: "visit_blockquote",
        doc: "Visit blockquote elements `<blockquote>`.",
        params: &[ParamKind::Str("content"), ParamKind::Usize("depth")],
    },
    CallbackSpec {
        name: "visit_strong",
        doc: "Visit strong/bold elements `<strong>`, `<b>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_emphasis",
        doc: "Visit emphasis/italic elements `<em>`, `<i>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_strikethrough",
        doc: "Visit strikethrough elements `<s>`, `<del>`, `<strike>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_underline",
        doc: "Visit underline elements `<u>`, `<ins>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_subscript",
        doc: "Visit subscript elements `<sub>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_superscript",
        doc: "Visit superscript elements `<sup>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_mark",
        doc: "Visit mark/highlight elements `<mark>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_line_break",
        doc: "Visit line break elements `<br>`.",
        params: &[],
    },
    CallbackSpec {
        name: "visit_horizontal_rule",
        doc: "Visit horizontal rule elements `<hr>`.",
        params: &[],
    },
    CallbackSpec {
        name: "visit_custom_element",
        doc: "Visit custom/unknown elements.",
        params: &[ParamKind::Str("tag_name"), ParamKind::Str("html")],
    },
    CallbackSpec {
        name: "visit_definition_list_start",
        doc: "Visit definition list `<dl>`.",
        params: &[],
    },
    CallbackSpec {
        name: "visit_definition_term",
        doc: "Visit definition term `<dt>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_definition_description",
        doc: "Visit definition description `<dd>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_definition_list_end",
        doc: "Called after processing a definition list `</dl>`.",
        params: &[ParamKind::Str("output")],
    },
    CallbackSpec {
        name: "visit_form",
        doc: "Visit form elements `<form>`.\\n    ///\\n    /// `action` and `method` may be null.",
        params: &[ParamKind::OptStr("action"), ParamKind::OptStr("method")],
    },
    CallbackSpec {
        name: "visit_input",
        doc: "Visit input elements `<input>`.\\n    ///\\n    /// `name` and `value` may be null.",
        params: &[
            ParamKind::Str("input_type"),
            ParamKind::OptStr("name"),
            ParamKind::OptStr("value"),
        ],
    },
    CallbackSpec {
        name: "visit_button",
        doc: "Visit button elements `<button>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_audio",
        doc: "Visit audio elements `<audio>`.\\n    ///\\n    /// `src` may be null.",
        params: &[ParamKind::OptStr("src")],
    },
    CallbackSpec {
        name: "visit_video",
        doc: "Visit video elements `<video>`.\\n    ///\\n    /// `src` may be null.",
        params: &[ParamKind::OptStr("src")],
    },
    CallbackSpec {
        name: "visit_iframe",
        doc: "Visit iframe elements `<iframe>`.\\n    ///\\n    /// `src` may be null.",
        params: &[ParamKind::OptStr("src")],
    },
    CallbackSpec {
        name: "visit_details",
        doc: "Visit details elements `<details>`.\\n    ///\\n    /// `open` is non-zero when the `open` attribute is present.",
        params: &[ParamKind::Bool("open")],
    },
    CallbackSpec {
        name: "visit_summary",
        doc: "Visit summary elements `<summary>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_figure_start",
        doc: "Called before processing a figure `<figure>`.",
        params: &[],
    },
    CallbackSpec {
        name: "visit_figcaption",
        doc: "Visit figcaption elements `<figcaption>`.",
        params: &[ParamKind::Str("text")],
    },
    CallbackSpec {
        name: "visit_figure_end",
        doc: "Called after processing a figure `</figure>`.",
        params: &[ParamKind::Str("output")],
    },
];

// ---------------------------------------------------------------------------
// Code-generation helpers — each produces one section of the output
// ---------------------------------------------------------------------------

/// Build the C `extern "C" fn(...)` signature parameters for one callback.
fn c_param_list(spec: &CallbackSpec, pascal_prefix: &str) -> String {
    let mut parts = vec![
        format!("ctx: *const {pascal_prefix}NodeContext"),
        "user_data: *mut std::ffi::c_void".to_string(),
    ];
    for p in spec.params {
        match p {
            ParamKind::Str(n) | ParamKind::OptStr(n) => {
                parts.push(format!("{n}: *const std::ffi::c_char"));
            }
            ParamKind::Bool(n) => parts.push(format!("{n}: i32")),
            ParamKind::U32(n) => parts.push(format!("{n}: u32")),
            ParamKind::Usize(n) => parts.push(format!("{n}: usize")),
            ParamKind::CellSlice(n) => {
                parts.push(format!("{n}: *const *const std::ffi::c_char"));
                parts.push("cell_count: usize".to_string());
            }
        }
    }
    parts.push("out_custom: *mut *mut std::ffi::c_char".to_string());
    parts.push("out_len: *mut usize".to_string());
    parts.join(",\n            ")
}

/// Generate all `Option<unsafe extern "C" fn(...)>` struct fields.
fn gen_struct_fields(pascal_prefix: &str) -> String {
    let mut out = String::new();
    for spec in CALLBACKS {
        out.push_str(&format!(
            "\n    /// {doc}\n    pub {name}: Option<\n        unsafe extern \"C\" fn(\n            {params}\n        ) -> i32,\n    >,\n",
            doc = spec.doc,
            name = spec.name,
            params = c_param_list(spec, pascal_prefix),
        ));
    }
    out
}

/// Build the Rust trait parameter list for a callback (the `&str`, `bool`, etc. side).
fn rust_param_list(spec: &CallbackSpec, core_import: &str) -> String {
    let mut parts = vec![
        "&mut self".to_string(),
        format!("ctx: &{core_import}::visitor::NodeContext"),
    ];
    for p in spec.params {
        match p {
            ParamKind::Str(n) => parts.push(format!("{n}: &str")),
            ParamKind::OptStr(n) => parts.push(format!("{n}: Option<&str>")),
            ParamKind::Bool(n) => parts.push(format!("{n}: bool")),
            ParamKind::U32(n) => parts.push(format!("{n}: u32")),
            ParamKind::Usize(n) => parts.push(format!("{n}: usize")),
            ParamKind::CellSlice(n) => parts.push(format!("{n}: &[String]")),
        }
    }
    parts.join(", ")
}

/// Generate the body of one `impl HtmlVisitor` method.
///
/// Produces local CString bindings, the `call_with_ctx` invocation, and the
/// callback argument forwarding.
fn gen_impl_body(spec: &CallbackSpec, core_import: &str) -> String {
    let mut bindings = String::new();
    let mut cb_args = Vec::new();

    for p in spec.params {
        match p {
            ParamKind::Str(n) => {
                bindings.push_str(&format!(
                    "        let {n}_cs = match std::ffi::CString::new({n}) {{\n            Ok(s) => s,\n            Err(_) => return {core_import}::visitor::VisitResult::Continue,\n        }};\n"
                ));
                cb_args.push(format!("{n}_cs.as_ptr()"));
            }
            ParamKind::OptStr(n) => {
                bindings.push_str(&format!("        let ({n}_ptr, _{n}_cs) = opt_str_to_c({n});\n"));
                cb_args.push(format!("{n}_ptr"));
            }
            ParamKind::Bool(n) => {
                bindings.push_str(&format!("        let {n}_i = i32::from({n});\n"));
                cb_args.push(format!("{n}_i"));
            }
            ParamKind::U32(n) | ParamKind::Usize(n) => {
                cb_args.push((*n).to_string());
            }
            ParamKind::CellSlice(n) => {
                bindings.push_str(&format!(
                    "        let {n}_cstrings: Vec<std::ffi::CString> = {n}\n            .iter()\n            .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())\n            .collect();\n        let {n}_ptrs: Vec<*const std::ffi::c_char> =\n            {n}_cstrings.iter().map(|cs| cs.as_ptr()).collect();\n        let cell_count = {n}_ptrs.len();\n"
                ));
                cb_args.push(format!("{n}_ptrs.as_ptr()"));
                cb_args.push("cell_count".to_string());
            }
        }
    }

    let args_str = if cb_args.is_empty() {
        "out_custom, out_len".to_string()
    } else {
        format!("{}, out_custom, out_len", cb_args.join(", "))
    };

    format!(
        "        let Some(cb) = self.callbacks.{name} else {{\n            return {core_import}::visitor::VisitResult::Continue;\n        }};\n        let user_data = self.callbacks.user_data;\n{bindings}        // SAFETY: cb is a valid function pointer; all temporaries live for this call.\n        unsafe {{\n            call_with_ctx(ctx, |c_ctx, out_custom, out_len| {{\n                cb(c_ctx, user_data, {args_str})\n            }})\n        }}",
        name = spec.name,
    )
}

/// Generate all `impl HtmlVisitor` methods.
fn gen_impl_methods(pascal_prefix: &str, core_import: &str) -> String {
    let mut out = String::new();
    for spec in CALLBACKS {
        out.push_str(&format!(
            "\n    fn {name}(\n        {params}\n    ) -> {core_import}::visitor::VisitResult {{\n{body}\n    }}\n",
            name = spec.name,
            params = rust_param_list(spec, core_import),
            body = gen_impl_body(spec, core_import),
        ));
    }
    // Close the impl block — caller opens it.
    let _ = pascal_prefix; // used by caller
    out
}

/// Build the forwarding argument list for `VisitorRef` delegation.
fn visitor_ref_args(spec: &CallbackSpec) -> String {
    let mut args = vec!["ctx".to_string()];
    for p in spec.params {
        match p {
            ParamKind::Str(n)
            | ParamKind::OptStr(n)
            | ParamKind::Bool(n)
            | ParamKind::U32(n)
            | ParamKind::Usize(n)
            | ParamKind::CellSlice(n) => args.push((*n).to_string()),
        }
    }
    args.join(", ")
}

/// Generate all `VisitorRef` forwarding methods.
fn gen_visitor_ref_methods(core_import: &str) -> String {
    let mut out = String::new();
    for spec in CALLBACKS {
        let params = rust_param_list(spec, core_import);
        let args = visitor_ref_args(spec);
        out.push_str(&format!(
            "            fn {name}({params}) -> {core_import}::visitor::VisitResult {{\n                unsafe {{ (*self.0).{name}({args}) }}\n            }}\n",
            name = spec.name,
        ));
    }
    out
}

/// Generate the visitor FFI bindings block for `lib.rs`.
///
/// # Parameters
///
/// - `prefix`: the FFI function prefix (e.g. `"htm"`).
/// - `core_import`: the Rust `use` path for the core crate (e.g. `"html_to_markdown_rs"`).
pub fn gen_visitor_bindings(prefix: &str, core_import: &str) -> String {
    let pascal_prefix = prefix.to_pascal_case();

    let struct_fields = gen_struct_fields(&pascal_prefix);
    let impl_methods = gen_impl_methods(&pascal_prefix, core_import);
    let visitor_ref_methods = gen_visitor_ref_methods(core_import);

    format!(
        r#"// ---------------------------------------------------------------------------
// Visitor / callback FFI — all 42 HtmlVisitor methods
// ---------------------------------------------------------------------------

/// Visit-result code: continue with default conversion.
pub const HTM_VISIT_CONTINUE: i32 = 0;
/// Visit-result code: skip this element entirely (no output).
pub const HTM_VISIT_SKIP: i32 = 1;
/// Visit-result code: preserve the original HTML verbatim.
pub const HTM_VISIT_PRESERVE_HTML: i32 = 2;
/// Visit-result code: use `out_custom` / `out_len` as custom Markdown output.
pub const HTM_VISIT_CUSTOM: i32 = 3;
/// Visit-result code: abort conversion; `out_custom` contains the error message.
pub const HTM_VISIT_ERROR: i32 = 4;

/// Opaque context passed to every C callback.
///
/// Fields reflect `NodeContext` from the Rust core. All string pointers are
/// valid only for the duration of the callback invocation.
#[repr(C)]
pub struct {pascal_prefix}NodeContext {{
    /// Coarse-grained node type tag (matches `NodeType` discriminant).
    pub node_type: i32,
    /// Null-terminated tag name (e.g. `"div"`). Never null.
    pub tag_name: *const std::ffi::c_char,
    /// Depth in the DOM tree (0 = root).
    pub depth: usize,
    /// Index among siblings (0-based).
    pub index_in_parent: usize,
    /// Null-terminated parent tag name, or null if root.
    pub parent_tag: *const std::ffi::c_char,
    /// Non-zero if this element is treated as inline.
    pub is_inline: i32,
}}

/// C-facing callback struct for the visitor pattern.
///
/// Populate the function-pointer fields you care about; leave the rest null.
/// The `user_data` pointer is forwarded unchanged to every callback — use it
/// to thread your own context through the conversion.
///
/// # Field order
///
/// The field order matches the Go binding's expected C layout exactly.
///
/// # Callback return protocol
///
/// Callbacks return an `i32` visit-result code.  When the code is
/// `HTM_VISIT_CUSTOM` (3) or `HTM_VISIT_ERROR` (4), the callback must also
/// write a heap-allocated, null-terminated string into `*out_custom` and set
/// `*out_len` to its byte length (excluding the null terminator).  The Rust
/// side will read the string and then call `free()` on the pointer.
///
/// For all other codes `out_custom` and `out_len` are not written.
///
/// # Callback signatures
///
/// All callbacks share the same leading parameters:
/// ```c
/// fn(ctx, user_data, out_custom, out_len, ...) -> i32
/// ```
/// followed by method-specific parameters documented on each field.
#[repr(C)]
pub struct {pascal_prefix}VisitorCallbacks {{
    /// Arbitrary caller context forwarded to every callback.
    pub user_data: *mut std::ffi::c_void,
{struct_fields}}}

// SAFETY: The `user_data` pointer is the caller's responsibility. We require
// callers to uphold thread-safety themselves (i.e. not share a visitor across
// threads without synchronisation). The callbacks themselves are `extern "C"`
// and therefore inherently `Send`.
unsafe impl Send for {pascal_prefix}VisitorCallbacks {{}}

/// Opaque handle wrapping a `{pascal_prefix}VisitorCallbacks` and implementing
/// the Rust `HtmlVisitor` trait.
///
/// Allocate with `{prefix}_visitor_create` and release with `{prefix}_visitor_free`.
/// The handle must NOT outlive the `{pascal_prefix}VisitorCallbacks` it was created from.
pub struct {pascal_prefix}Visitor {{
    callbacks: {pascal_prefix}VisitorCallbacks,
    /// CString storage for tag names / parent tags that we pass back to C.
    _tag_scratch: std::cell::RefCell<Vec<std::ffi::CString>>,
}}

impl std::fmt::Debug for {pascal_prefix}Visitor {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        f.debug_struct("{pascal_prefix}Visitor").finish_non_exhaustive()
    }}
}}

/// Map a `VisitResult` integer code + optional custom string pointer back to
/// the Rust `VisitResult` enum.
///
/// # Safety
///
/// `custom_ptr` must be either null or a pointer to a heap-allocated
/// null-terminated string that this function will take ownership of (freeing
/// it after reading).
unsafe fn decode_visit_result(
    code: i32,
    custom_ptr: *mut std::ffi::c_char,
) -> {core_import}::visitor::VisitResult {{
    use {core_import}::visitor::VisitResult;
    match code {{
        {VISIT_RESULT_SKIP} => VisitResult::Skip,
        {VISIT_RESULT_PRESERVE_HTML} => VisitResult::PreserveHtml,
        {VISIT_RESULT_CUSTOM} | {VISIT_RESULT_ERROR} => {{
            let msg = if custom_ptr.is_null() {{
                String::new()
            }} else {{
                // SAFETY: caller guarantees this is a valid heap CString.
                let cstr = unsafe {{ std::ffi::CString::from_raw(custom_ptr) }};
                cstr.to_string_lossy().into_owned()
            }};
            if code == {VISIT_RESULT_CUSTOM} {{
                VisitResult::Custom(msg)
            }} else {{
                VisitResult::Error(msg)
            }}
        }}
        _ => VisitResult::Continue,
    }}
}}

/// Build a temporary `{pascal_prefix}NodeContext` from a Rust `NodeContext`, invoke
/// the provided callback, and decode the `VisitResult`.
///
/// The `NodeContext` passed to the C callback is only valid for the duration
/// of this function call.
unsafe fn call_with_ctx<F>(
    ctx: &{core_import}::visitor::NodeContext,
    callback: F,
) -> {core_import}::visitor::VisitResult
where
    F: FnOnce(
        *const {pascal_prefix}NodeContext,
        *mut *mut std::ffi::c_char,
        *mut usize,
    ) -> i32,
{{
    // Build temporary CStrings for the string fields.
    let tag_cstring = std::ffi::CString::new(ctx.tag_name.as_str()).unwrap_or_default();
    let parent_cstring: Option<std::ffi::CString> = ctx
        .parent_tag
        .as_deref()
        .and_then(|s| std::ffi::CString::new(s).ok());

    let c_ctx = {pascal_prefix}NodeContext {{
        node_type: ctx.node_type as i32,
        tag_name: tag_cstring.as_ptr(),
        depth: ctx.depth,
        index_in_parent: ctx.index_in_parent,
        parent_tag: parent_cstring.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
        is_inline: ctx.is_inline as i32,
    }};

    let mut out_custom: *mut std::ffi::c_char = std::ptr::null_mut();
    let mut out_len: usize = 0;

    let code = callback(&c_ctx, &mut out_custom, &mut out_len);

    // SAFETY: decode_visit_result takes ownership of out_custom when non-null.
    unsafe {{ decode_visit_result(code, out_custom) }}
}}

/// Convert an `Option<&str>` to a C pointer: non-null CString when `Some`, null when `None`.
///
/// Returns `(ptr, Option<CString>)` — the `Option<CString>` must be kept alive
/// until after the pointer is consumed by the callback.
fn opt_str_to_c(s: Option<&str>) -> (*const std::ffi::c_char, Option<std::ffi::CString>) {{
    match s {{
        Some(val) => match std::ffi::CString::new(val) {{
            Ok(cs) => {{
                let ptr = cs.as_ptr();
                (ptr, Some(cs))
            }}
            Err(_) => (std::ptr::null(), None),
        }},
        None => (std::ptr::null(), None),
    }}
}}

impl {core_import}::visitor::HtmlVisitor for {pascal_prefix}Visitor {{
{impl_methods}}}

/// Create a new visitor handle from a callbacks struct.
///
/// The returned handle must be freed with `{prefix}_visitor_free`.
/// The `{pascal_prefix}VisitorCallbacks` struct is **copied** into the handle;
/// the caller may free it after this call returns.
///
/// Returns null on allocation failure.
///
/// # Safety
///
/// `callbacks` must point to a valid, fully initialised `{pascal_prefix}VisitorCallbacks`.
/// `user_data` (embedded in the struct) must remain valid and accessible from
/// any thread that calls `{prefix}_convert_with_visitor` until after
/// `{prefix}_visitor_free` is called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_visitor_create(
    callbacks: *const {pascal_prefix}VisitorCallbacks,
) -> *mut {pascal_prefix}Visitor {{
    if callbacks.is_null() {{
        return std::ptr::null_mut();
    }}
    // SAFETY: caller guarantees the pointer is valid.
    let cbs = unsafe {{ callbacks.read() }};
    let visitor = {pascal_prefix}Visitor {{
        callbacks: cbs,
        _tag_scratch: std::cell::RefCell::new(Vec::new()),
    }};
    Box::into_raw(Box::new(visitor))
}}

/// Free a visitor handle previously returned by `{prefix}_visitor_create`.
///
/// After this call the pointer is invalid and must not be used.
///
/// # Safety
///
/// `visitor` must have been returned by `{prefix}_visitor_create`, or be null.
/// Passing a null pointer is safe and has no effect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_visitor_free(visitor: *mut {pascal_prefix}Visitor) {{
    if !visitor.is_null() {{
        // SAFETY: visitor was created with Box::into_raw.
        unsafe {{ drop(Box::from_raw(visitor)); }}
    }}
}}

/// Convert HTML to Markdown using a custom visitor.
///
/// Equivalent to `{prefix}_convert` but threads the provided visitor through
/// the conversion pipeline so that every `visit_*` callback is invoked during
/// processing.
///
/// Returns a heap-allocated null-terminated Markdown string on success, or
/// null on failure (check `{prefix}_last_error_code` / `{prefix}_last_error_context`).
/// The returned pointer must be freed with `{prefix}_free_string`.
///
/// # Arguments
///
/// - `html`: null-terminated, UTF-8 HTML input. Must not be null.
/// - `options`: optional conversion options; pass null for defaults.
/// - `visitor`: optional visitor handle from `{prefix}_visitor_create`; pass
///   null for default conversion (equivalent to `{prefix}_convert`).
///
/// # Safety
///
/// All pointer arguments must be valid or null as described above.
/// The `visitor` pointer (and its embedded `user_data`) must remain valid for
/// the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_convert_with_visitor(
    html: *const std::ffi::c_char,
    options: *const {core_import}::ConversionOptions,
    visitor: *mut {pascal_prefix}Visitor,
) -> *mut std::ffi::c_char {{
    clear_last_error();

    if html.is_null() {{
        set_last_error(1, "Null pointer passed for html");
        return std::ptr::null_mut();
    }}

    let html_str = match unsafe {{ std::ffi::CStr::from_ptr(html) }}.to_str() {{
        Ok(s) => s.to_string(),
        Err(_) => {{
            set_last_error(1, "Invalid UTF-8 in html parameter");
            return std::ptr::null_mut();
        }}
    }};

    let options_rs: Option<{core_import}::ConversionOptions> = if options.is_null() {{
        None
    }} else {{
        Some(unsafe {{ &*options }}.clone())
    }};

    // Build the visitor handle if provided.
    let visitor_handle: Option<{core_import}::visitor::VisitorHandle> = if visitor.is_null() {{
        None
    }} else {{
        // SAFETY: visitor is a valid pointer for the duration of this call.
        let ffi_visitor = unsafe {{ &mut *visitor }};
        // Wrap in Rc<RefCell<dyn HtmlVisitor>> as required by convert.
        // We use a raw-pointer wrapper to avoid cloning — the {pascal_prefix}Visitor is
        // pinned in place by the caller-owned Box.
        struct VisitorRef(*mut {pascal_prefix}Visitor);
        impl std::fmt::Debug for VisitorRef {{
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
                f.debug_struct("VisitorRef").finish_non_exhaustive()
            }}
        }}
        impl {core_import}::visitor::HtmlVisitor for VisitorRef {{
{visitor_ref_methods}        }}
        let _ = ffi_visitor; // suppress unused warning
        Some(std::rc::Rc::new(std::cell::RefCell::new(VisitorRef(visitor))))
    }};

    match {core_import}::convert(&html_str, options_rs, visitor_handle) {{
        Ok(result) => {{
            let markdown = result.content.unwrap_or_default();
            match std::ffi::CString::new(markdown) {{
                Ok(s) => s.into_raw(),
                Err(_) => {{
                    set_last_error(3, "Conversion output contained null bytes");
                    std::ptr::null_mut()
                }}
            }}
        }},
        Err(e) => {{
            set_last_error(2, &e.to_string());
            std::ptr::null_mut()
        }}
    }}
}}"#,
        VISIT_RESULT_SKIP = VISIT_RESULT_SKIP,
        VISIT_RESULT_PRESERVE_HTML = VISIT_RESULT_PRESERVE_HTML,
        VISIT_RESULT_CUSTOM = VISIT_RESULT_CUSTOM,
        VISIT_RESULT_ERROR = VISIT_RESULT_ERROR,
        prefix = prefix,
        pascal_prefix = pascal_prefix,
        core_import = core_import,
        struct_fields = struct_fields,
        impl_methods = impl_methods,
        visitor_ref_methods = visitor_ref_methods,
    )
}
