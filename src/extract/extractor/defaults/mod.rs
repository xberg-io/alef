use super::helpers::{extract_cfg_condition, is_test_gated};
use crate::core::ir::{DefaultValue, FieldDef, TypeRef};
use ahash::AHashMap;
use quote::ToTokens;
use syn;

mod function_default;
mod mutation;
mod struct_literal;

pub(crate) use function_default::{FreeFunctionIndex, collect_free_functions, fold_constant_default_functions};
use struct_literal::struct_expr_defaults;

/// Every associated function of every inherent `impl` block in one module, keyed by
/// `(type name, function name)`.
///
/// Exists so [`extract_default_values`] can follow a `fn default()` that delegates to one of
/// its own constructors instead of spelling a struct literal. Scoped to a single module for
/// the same reason [`collect_literal_consts`] is: `impl Default` and the `fn new` it calls sit
/// next to each other in the overwhelmingly common case, and resolving a constructor from
/// another module would need a crate-wide index. ~keep
pub(crate) type ConstructorIndex<'a> = AHashMap<(String, String), &'a syn::ImplItemFn>;

/// How many `Self::a() -> Self::b() -> ...` hops [`extract_default_values`] will follow.
///
/// A bound rather than a visited-set because the bound is also the honest limit of the
/// technique: past a few hops the "constructor" is a pipeline, not a constant. It doubles as
/// the cycle guard, so `fn new() { Self::fresh() }` / `fn fresh() { Self::new() }` terminates
/// instead of recursing forever. ~keep
const MAX_DELEGATION_DEPTH: usize = 4;

/// Extract concrete default values from an `impl Default for T` block.
///
/// Finds the `fn default() -> Self` method and reads its body one of three ways:
///
/// 1. a struct literal (`Self { field: expr, ... }`), each initializer lowered to a
///    [`DefaultValue`] — including the `let mut value = Self { .. }; value.field = ..; value`
///    spelling, whose mutations are applied over the literal by [`mutation::read_struct_body`];
///    or
/// 2. a delegation to one of `T`'s own constructors (`Self::new("en")`), whose parameters are
///    bound to the literal arguments the delegation passed and whose struct literal is then
///    read against that binding; or
/// 3. for a single-field type, a bare associated-const path (`Self::ONE`, `Weight::ONE`) whose
///    initializer is itself a foldable literal or single-argument tuple-struct literal of the
///    same type (`const ONE: Weight = Weight(1);`) — see [`single_field_const_tail_default`].
///
/// A body that is none of these writes [`DefaultValue::Unresolved`] to every field. That is
/// **not** the same as [`DefaultValue::Empty`]: `Empty` claims the default *is* the type's
/// zero, `Unresolved` records that alef could not read it. Collapsing the two is what let six
/// backends emit their type-zero underneath a doc comment quoting the real Rust value; see
/// [`DefaultValue::Unresolved`] and `cli::pipeline::generate::validation`, which refuses to
/// generate rather than guess. ~keep
///
/// `literal_consts` resolves a field initializer that references a sibling
/// `const NAME: T = <literal>;` declared in the same module (e.g. `NAME`, `NAME.to_string()`,
/// or `Type::NAME`) to that constant's actual value. See [`collect_literal_consts`].
pub(crate) fn extract_default_values(
    item: &syn::ItemImpl,
    self_type: &str,
    fields: &mut [FieldDef],
    literal_consts: &AHashMap<String, DefaultValue>,
    constructors: &ConstructorIndex<'_>,
    binding_excluded: bool,
) {
    let default_fn = item.items.iter().find_map(|impl_item| {
        if let syn::ImplItem::Fn(method) = impl_item
            && method.sig.ident == "default"
        {
            return Some(method);
        }
        None
    });

    let Some(default_fn) = default_fn else {
        mark_unresolved(fields, "impl Default block without a `fn default()` item");
        return;
    };

    // The declared type of each field, so a two-segment path initializer can be checked against
    // it before being lowered to `DefaultValue::EnumVariant`. See [`admits_enum_variant`]. ~keep
    let field_types: AHashMap<String, TypeRef> = fields
        .iter()
        .map(|field| (field.name.clone(), field.ty.clone()))
        .collect();
    let scope = EvalScope::new(self_type, literal_consts, &field_types);

    let defaults = if let Some(body) = mutation::read_struct_body(&default_fn.block) {
        mutation::struct_body_defaults(&body, &scope)
    } else if let Some(delegated) = follow_delegation(&default_fn.block, self_type, constructors, &scope, 0) {
        delegated
    } else if tail_is_bare_self(&default_fn.block, self_type) {
        AHashMap::new()
    } else if let Some(single_field) = single_field_const_tail_default(&default_fn.block, fields, &scope) {
        single_field
    } else {
        let body = default_fn.block.to_token_stream().to_string();
        // An item alef is not going to emit into any binding surface (`#[alef(skip)]` in any of
        // its recognized spellings, or `#[doc(hidden)]`) is stripped from every backend's output
        // independently of this flag — see `src/core/ir/items.rs`'s `binding_excluded` field.
        // Warning about a default this pass could not read is only ever actionable for a type
        // that *reaches* a binding; for an excluded type it is pure noise repeated on every
        // regen, so it is skipped here rather than upstream, matching every other
        // `binding_excluded` consumer's independent, per-site check (there is no central
        // enforcement point — see the `binding-audit-pattern` convention). `mark_unresolved`
        // still runs unconditionally: an excluded type's fields keep the honest `Unresolved`
        // value rather than a fabricated one, in case anything else ever reads them. ~keep
        if !binding_excluded {
            tracing::warn!(
                target: "alef::extract::defaults",
                rust_type = self_type,
                body = %body,
                "`impl Default` body is neither a struct literal nor a constant-foldable delegation; \
                 field defaults are unresolved"
            );
        }
        mark_unresolved(fields, &body);
        return;
    };

    for field in fields.iter_mut() {
        if let Some(default_val) = defaults.get(&field.name) {
            field.typed_default = Some(default_val.clone());
        } else {
            field.typed_default = Some(DefaultValue::Empty);
        }
    }
}

fn mark_unresolved(fields: &mut [FieldDef], body: &str) {
    for field in fields.iter_mut() {
        field.typed_default = Some(DefaultValue::Unresolved(body.to_string()));
    }
}

/// Collects the associated (receiver-less) functions of every inherent `impl` block in
/// `items`, so a delegating `fn default()` can be followed to the constructor it calls.
///
/// Trait impls are skipped: `impl Default for T` is the caller, and no other trait method is
/// a plausible delegation target. Methods with a `self` receiver are skipped because
/// `Self::name(..)` in a `fn default()` cannot reach one. ~keep
pub(crate) fn collect_constructors(items: &[syn::Item]) -> ConstructorIndex<'_> {
    let mut index = ConstructorIndex::new();
    for item in items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        // A `#[cfg(test)]` impl block is not part of the binding surface — `extractor::mod` skips
        // it when extracting — so a test-only `fn new` must not shadow the real constructor. ~keep
        if item_impl.trait_.is_some() || is_test_gated(&item_impl.attrs) {
            continue;
        }
        let Some(type_name) = path_type_name(&item_impl.self_ty) else {
            continue;
        };
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if matches!(method.sig.inputs.first(), Some(syn::FnArg::Receiver(_))) {
                continue;
            }
            index.insert((type_name.clone(), method.sig.ident.to_string()), method);
        }
    }
    index
}

fn path_type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// Collects every literal-valued `const` visible to an `impl Default` in the same module, so
/// [`extract_default_values`] can resolve a field initializer that references one instead of
/// collapsing it to `DefaultValue::Empty`.
///
/// Both module-level `const NAME: T = <literal>;` (keyed by the bare identifier) and associated
/// consts of inherent `impl` blocks (keyed `Type::NAME`) are collected. The associated-const key
/// carries the owning type because two types in one module may each declare `const DEFAULT`, and
/// a bare `Self` key would let one silently answer for the other. ~keep
///
/// Every literal kind is collected, not only `&str`. `max_pages_ceiling: DEFAULT_MAX_PAGES`
/// against `const DEFAULT_MAX_PAGES: usize = 500;` is the dominant shape of unreadable field
/// default in the consumer crates, and a numeric const is exactly as readable as a string one —
/// alef was rendering `0` for several of these, underneath a doc comment quoting the real value.
/// A const whose initializer is not a literal (`Duration::from_secs(5)`, a `concat!`) stays out:
/// evaluating it would be interpretation, not reading. ~keep
///
/// Deliberately scoped to the items of a single module/file: `impl Default` and
/// the const it references are the overwhelmingly common shape (`refresh.rs`'s
/// `DEFAULT_CATALOG_URL` alongside `CatalogRefreshConfig`'s `impl Default`), and
/// resolving a `use`-imported const from another module would need a full
/// crate-wide const index. ~keep
pub(crate) fn collect_literal_consts(items: &[syn::Item]) -> AHashMap<String, DefaultValue> {
    let mut consts = AHashMap::new();
    for item in items {
        match item {
            syn::Item::Const(item_const) => {
                if let Some(value) = const_literal_value(&item_const.expr) {
                    consts.insert(item_const.ident.to_string(), value);
                }
            }
            // Trait impls are skipped for the same reason [`collect_constructors`] skips them,
            // and a `#[cfg(test)]` impl is not part of the binding surface. ~keep
            syn::Item::Impl(item_impl) if item_impl.trait_.is_none() && !is_test_gated(&item_impl.attrs) => {
                let Some(type_name) = path_type_name(&item_impl.self_ty) else {
                    continue;
                };
                for impl_item in &item_impl.items {
                    if let syn::ImplItem::Const(assoc_const) = impl_item
                        && let Some(value) = const_literal_value(&assoc_const.expr)
                    {
                        consts.insert(format!("{type_name}::{}", assoc_const.ident), value);
                    }
                }
            }
            _ => {}
        }
    }
    consts
}

/// The value of a `const NAME: T = <literal>;`, or `None` for any other const initializer.
fn const_literal_value(expr: &syn::Expr) -> Option<DefaultValue> {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Str(s) => Some(DefaultValue::StringLiteral(s.value())),
            syn::Lit::Char(c) => Some(DefaultValue::StringLiteral(c.value().to_string())),
            syn::Lit::Bool(b) => Some(DefaultValue::BoolLiteral(b.value)),
            syn::Lit::Int(i) => i.base10_parse::<i64>().ok().map(DefaultValue::IntLiteral),
            syn::Lit::Float(f) => f.base10_parse::<f64>().ok().map(DefaultValue::FloatLiteral),
            _ => None,
        },
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => match const_literal_value(&unary.expr)? {
            DefaultValue::IntLiteral(v) => Some(DefaultValue::IntLiteral(-v)),
            DefaultValue::FloatLiteral(v) => Some(DefaultValue::FloatLiteral(-v)),
            _ => None,
        },
        // A single-argument tuple-struct/newtype literal (`Weight(1)`) folds to the literal it
        // wraps: `DefaultValue` has no struct-shaped variant, and for a single-field newtype
        // that literal *is* the field's own default once flattened (see
        // `single_field_const_tail_default`). Gated on an upper-case callee so an ordinary
        // function call (`compute(1)`, conventionally snake_case) is never mistaken for a
        // tuple-struct constructor and folded into a guess; a lower-case callee stays `None`
        // rather than risk it. ~keep
        syn::Expr::Call(call) if call.args.len() == 1 => {
            let syn::Expr::Path(path) = &*call.func else {
                return None;
            };
            let name = path.path.segments.last()?.ident.to_string();
            if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
                return None;
            }
            const_literal_value(call.args.first()?)
        }
        _ => None,
    }
}

/// Everything a field initializer can be resolved against.
///
/// `literal_consts` is module-wide and constant. `params` is populated only while reading a
/// constructor's body on behalf of a delegating `fn default()`: it binds that constructor's
/// parameters to the literal arguments the delegation passed, which is what turns
/// `fn default() { Self::new("en") }` plus `fn new(lang: &str) { Self { lang: lang.into(), .. } }`
/// into `lang = "en"` rather than a guess. ~keep
struct EvalScope<'a> {
    /// The type whose `impl Default` is being read, so `Self::NAME` can be resolved against the
    /// `Type::NAME`-keyed associated consts in `literal_consts`. ~keep
    self_type: &'a str,
    literal_consts: &'a AHashMap<String, DefaultValue>,
    /// Declared type per field name, used only to decide whether a two-segment path initializer
    /// can be an enum variant. Empty while reading a constructor body on behalf of a delegating
    /// `fn default()` would be wrong, so it is carried across `with_params` unchanged. ~keep
    field_types: &'a AHashMap<String, TypeRef>,
    params: AHashMap<String, DefaultValue>,
}

impl<'a> EvalScope<'a> {
    fn new(
        self_type: &'a str,
        literal_consts: &'a AHashMap<String, DefaultValue>,
        field_types: &'a AHashMap<String, TypeRef>,
    ) -> Self {
        Self {
            self_type,
            literal_consts,
            field_types,
            params: AHashMap::new(),
        }
    }

    fn with_params(&self, params: AHashMap<String, DefaultValue>) -> EvalScope<'a> {
        EvalScope {
            self_type: self.self_type,
            literal_consts: self.literal_consts,
            field_types: self.field_types,
            params,
        }
    }

    /// Resolves `Owner::NAME` (and `Self::NAME`, against the type being read) to the value of an
    /// associated literal const declared in the same module.
    fn associated_const(&self, owner: &str, name: &str) -> Option<DefaultValue> {
        let owner = if owner == "Self" { self.self_type } else { owner };
        self.literal_consts.get(&format!("{owner}::{name}")).cloned()
    }
}

/// True for the `DefaultValue`s that carry an actual value, as opposed to recording the
/// absence of one. Only these may be bound to a constructor parameter: binding `Empty` or
/// `Unresolved` would substitute a guess into the callee's body and lose the very
/// distinction this module exists to keep. ~keep
fn carries_value(value: &DefaultValue) -> bool {
    matches!(
        value,
        DefaultValue::BoolLiteral(_)
            | DefaultValue::StringLiteral(_)
            | DefaultValue::IntLiteral(_)
            | DefaultValue::FloatLiteral(_)
            | DefaultValue::EnumVariant(_)
            | DefaultValue::TupleVariant(_, _)
            | DefaultValue::StructVariant(_, _)
            | DefaultValue::ListLiteral(_)
    )
}

/// Follow a `fn default()` whose body is a call to one of the type's own associated
/// functions — `Self::new("en")`, `PaddleOcrConfig::for_language("en")` — and read the
/// callee's struct literal with its parameters bound to the arguments passed.
///
/// This is a constant fold, not an interpreter, and the boundary is deliberate. A callee that
/// computes a field (`side_len: base * scale_for(lang)`), branches on its argument, or builds
/// through a builder is not followed; those fields stay unresolved and get reported rather
/// than guessed. The technique covers the shape it was written for — a constructor taking
/// literal arguments and returning a struct literal — and nothing beyond it. ~keep
fn follow_delegation(
    block: &syn::Block,
    self_type: &str,
    constructors: &ConstructorIndex<'_>,
    scope: &EvalScope<'_>,
    depth: usize,
) -> Option<AHashMap<String, DefaultValue>> {
    if depth >= MAX_DELEGATION_DEPTH {
        return None;
    }

    let call = tail_call_expr(block)?;
    let syn::Expr::Path(path) = &*call.func else {
        return None;
    };
    let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
    // `Self::new(..)` and `PaddleOcrConfig::new(..)` name the same function. A longer path
    // leaves the module `constructors` indexes, so it cannot be resolved here. ~keep
    let [owner, fn_name] = segments.as_slice() else {
        return None;
    };
    if owner.as_str() != "Self" && owner.as_str() != self_type {
        return None;
    }
    // `Self::default()` inside `fn default()` is unbounded recursion in the source itself.
    if fn_name.as_str() == "default" {
        return None;
    }

    let target = constructors.get(&(self_type.to_string(), fn_name.clone()))?;

    let mut params = AHashMap::new();
    let mut arguments = call.args.iter();
    for input in &target.sig.inputs {
        let syn::FnArg::Typed(pat_type) = input else {
            return None;
        };
        let argument = arguments.next()?;
        let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            continue;
        };
        let value = expr_to_default_value(argument, scope, None);
        if carries_value(&value) {
            params.insert(pat_ident.ident.to_string(), value);
        }
    }
    // An arity mismatch means the index resolved the wrong function (or the source does not
    // compile); either way, reading its body would invent values. ~keep
    if arguments.next().is_some() {
        return None;
    }

    let inner = scope.with_params(params);
    if let Some(body) = mutation::read_struct_body(&target.block) {
        return Some(mutation::struct_body_defaults(&body, &inner));
    }
    if tail_is_bare_self(&target.block, self_type) {
        return Some(AHashMap::new());
    }
    follow_delegation(&target.block, self_type, constructors, &inner, depth + 1)
}

/// The tail expression of a block, unwrapped to a call. Only the tail is considered: an
/// earlier statement is not what the function returns.
fn tail_call_expr(block: &syn::Block) -> Option<&syn::ExprCall> {
    match block.stmts.last()? {
        syn::Stmt::Expr(expr, _) => unwrap_to_call_expr(expr),
        _ => None,
    }
}

fn unwrap_to_call_expr(expr: &syn::Expr) -> Option<&syn::ExprCall> {
    match expr {
        syn::Expr::Call(call) => Some(call),
        syn::Expr::Block(b) => tail_call_expr(&b.block),
        syn::Expr::Return(ret) => ret.expr.as_deref().and_then(unwrap_to_call_expr),
        _ => None,
    }
}

/// `fn default() -> Self { Self::ONE }` for a single-field newtype (`pub struct Weight(u32);`):
/// the tail expression is a bare associated-const path rather than a struct literal or a
/// delegation call, but the const names a value of `Self` itself, and `Self` has exactly one
/// field — so the const's value, once folded to a scalar by [`const_literal_value`], *is* that
/// field's default.
///
/// Scoped to single-field types deliberately: a multi-field struct offers no way to know which
/// field a lone `Self::NAME` scalar belongs to (`Self::NAME` could itself be a full struct
/// literal, which [`const_literal_value`] does not attempt to read — see its doc comment).
/// Guessing a field mapping would be worse than reporting `Unresolved`. ~keep
fn single_field_const_tail_default(
    block: &syn::Block,
    fields: &[FieldDef],
    scope: &EvalScope<'_>,
) -> Option<AHashMap<String, DefaultValue>> {
    let [field] = fields else {
        return None;
    };
    let path = tail_path_expr(block)?;
    let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
    let [owner, name] = segments.as_slice() else {
        return None;
    };
    let value = scope.associated_const(owner, name)?;
    let mut defaults = AHashMap::new();
    defaults.insert(field.name.clone(), value);
    Some(defaults)
}

/// The tail expression of a block, unwrapped to a bare path. Mirrors [`tail_call_expr`] for the
/// `Self::NAME` shape [`single_field_const_tail_default`] reads.
fn tail_path_expr(block: &syn::Block) -> Option<&syn::ExprPath> {
    match block.stmts.last()? {
        syn::Stmt::Expr(expr, _) => unwrap_to_path_expr(expr),
        _ => None,
    }
}

fn unwrap_to_path_expr(expr: &syn::Expr) -> Option<&syn::ExprPath> {
    match expr {
        syn::Expr::Path(path) => Some(path),
        syn::Expr::Block(b) => tail_path_expr(&b.block),
        syn::Expr::Return(ret) => ret.expr.as_deref().and_then(unwrap_to_path_expr),
        _ => None,
    }
}

/// `fn new() -> Self { Self }` (or `fn default() -> Self { Self }` directly): the tail
/// expression is the bare, one-segment path `Self`/`SelfType`, with no braces and no call.
///
/// That spelling only ever compiles for a type with zero fields — a struct with any field
/// needs `Self { field: expr, .. }` (a real `syn::ExprStruct`, already read by
/// [`mutation::read_struct_body`]/[`struct_expr_defaults`]) or a tuple struct needs `Self(expr)`
/// (a call).
/// So finding a bare `Self` tail here means there is nothing for a field default to disagree
/// about, and the "neither a struct literal nor a foldable delegation" fallback in
/// [`extract_default_values`] — the one that logs and writes [`DefaultValue::Unresolved`] to
/// every field — would otherwise fire for every zero-field type on the mere shape of the return
/// expression, not because any real default went unread. ~keep
fn tail_is_bare_self(block: &syn::Block, self_type: &str) -> bool {
    let Some(path) = tail_path_expr(block) else {
        return false;
    };
    if path.path.segments.len() != 1 {
        return false;
    }
    let ident = &path.path.segments[0].ident;
    ident == "Self" || ident == self_type
}

/// Helper trait to extract the named member from a `FieldValue`.
trait FieldMemberExt {
    fn member_named(&self) -> Option<&syn::Ident>;
}

impl FieldMemberExt for syn::FieldValue {
    fn member_named(&self) -> Option<&syn::Ident> {
        match &self.member {
            syn::Member::Named(ident) => Some(ident),
            syn::Member::Unnamed(_) => None,
        }
    }
}

/// Records that this initializer could not be read, carrying its source text so the diagnostic
/// in `cli::pipeline::generate::validation` can name the expression it refused.
fn unreadable(expr: &syn::Expr) -> DefaultValue {
    DefaultValue::Unresolved(expr.to_token_stream().to_string())
}

/// Whether a field of this declared type could hold an enum variant.
///
/// `Expr::Path` with two segments is ambiguous in Rust source: `Mode::Fast` names an enum
/// variant, `Self::DEFAULT_MODEL` names an associated const, `Duration::ZERO` names an
/// associated const of a struct. Lowering all three to [`DefaultValue::EnumVariant`] let
/// `codegen::config_gen::shared` render the *snake-cased variant name* as a string literal
/// whenever the field's type was `String`, so `model: Self::DEFAULT_MODEL` shipped as
/// `"default_model"` — a value that appears nowhere in the source crate and looks entirely
/// plausible in generated output.
///
/// `Named` is admitted rather than checked against an enum index because the enum is frequently
/// declared in a different module from the `impl Default` that names one of its variants, and
/// this pass is module-scoped by construction (see [`collect_literal_consts`]). Every non-`Named`
/// type is refused, which is where the fabrication lived.
///
/// `None` means the expression is not in a field position — a constructor argument being bound
/// by [`follow_delegation`], where no declared type is in reach — and keeps the prior reading. ~keep
fn admits_enum_variant(field_ty: Option<&TypeRef>) -> bool {
    match field_ty {
        None | Some(TypeRef::Named(_)) => true,
        Some(TypeRef::Optional(inner) | TypeRef::Vec(inner)) => admits_enum_variant(Some(&**inner)),
        Some(_) => false,
    }
}

/// Convert an expression to a `DefaultValue`.
///
/// `field_ty` is the declared type of the field being initialized, or `None` where the
/// expression is not a field initializer. It is consulted only by [`admits_enum_variant`].
///
/// Recognizes:
/// - `true` / `false` → `BoolLiteral`
/// - Integer literals → `IntLiteral`
/// - Float literals → `FloatLiteral`
/// - `"str".to_string()`, `String::from("str")`, `"str".into()` → `StringLiteral`
/// - `String::new()` → `StringLiteral("")`
/// - `'c'` (char literal) → `StringLiteral("c")`
/// - `Vec::new()`, `vec![]` → `Empty`
/// - `SomeType::default()`, `Default::default()` → `Empty`
/// - `SomeEnum::Variant`, where the field's declared type can hold one → `EnumVariant("Variant")`
/// - `SomeEnum::Variant(a, b)`, an upper-case-callee call with foldable arguments →
///   `TupleVariant("Variant", [a, b])`
/// - `SomeEnum::Variant { a: .., b: .. }`, a struct expression with foldable fields →
///   `StructVariant("Variant", [(a, ..), (b, ..)])`
/// - `CONST_NAME.to_string()` / `.to_owned()` / `.into()`, or a bare `CONST_NAME`,
///   where `CONST_NAME` resolves via `scope.literal_consts` → the constant's value
/// - `Self::CONST_NAME` / `Type::CONST_NAME`, where the associated literal const is declared in
///   the same module → the constant's value
/// - a bare constructor parameter, or `param.to_string()` / `.to_owned()` / `.into()`, where
///   `param` is bound in `scope.params` → the value the delegation passed for it
/// - Anything else → [`DefaultValue::Unresolved`]
///
/// Note the last line. `Empty` is reserved for the initializers that are *known* to be the
/// type's zero — `Vec::new()`, `Default::default()`, `vec![]` — and asserts as much. Every
/// other shape records that alef could not read the value, which is the same distinction
/// [`extract_default_values`] draws for a whole unreadable `fn default()` body, applied one
/// level down. Before this, an unreadable initializer inside a readable struct literal wrote
/// `Empty` and every backend rendered its own type-zero for it. ~keep
fn expr_to_default_value(expr: &syn::Expr, scope: &EvalScope<'_>, field_ty: Option<&TypeRef>) -> DefaultValue {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Bool(b) => DefaultValue::BoolLiteral(b.value),
            syn::Lit::Int(i) => {
                if let Ok(val) = i.base10_parse::<i64>() {
                    DefaultValue::IntLiteral(val)
                } else {
                    unreadable(expr)
                }
            }
            syn::Lit::Float(f) => {
                if let Ok(val) = f.base10_parse::<f64>() {
                    DefaultValue::FloatLiteral(val)
                } else {
                    unreadable(expr)
                }
            }
            syn::Lit::Char(c) => DefaultValue::StringLiteral(c.value().to_string()),
            syn::Lit::Str(s) => DefaultValue::StringLiteral(s.value()),
            _ => unreadable(expr),
        },

        // `&"en"` and `&CONST` reach a constructor parameter unchanged; the reference is not
        // part of the value. Parentheses and macro-expansion groups are likewise not part of
        // the value, and refusing to see through them would refuse a readable `(0.5)`. ~keep
        syn::Expr::Reference(syn::ExprReference { expr: inner, .. })
        | syn::Expr::Paren(syn::ExprParen { expr: inner, .. })
        | syn::Expr::Group(syn::ExprGroup { expr: inner, .. }) => expr_to_default_value(inner, scope, field_ty),

        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            match expr_to_default_value(&unary.expr, scope, field_ty) {
                DefaultValue::IntLiteral(v) => DefaultValue::IntLiteral(-v),
                DefaultValue::FloatLiteral(v) => DefaultValue::FloatLiteral(-v),
                _ => unreadable(expr),
            }
        }

        syn::Expr::Binary(bin) => {
            let lhs = expr_to_default_value(&bin.left, scope, field_ty);
            let rhs = expr_to_default_value(&bin.right, scope, field_ty);
            match (lhs, rhs) {
                (DefaultValue::IntLiteral(a), DefaultValue::IntLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => a
                        .checked_add(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Sub(_) => a
                        .checked_sub(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Mul(_) => a
                        .checked_mul(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Div(_) if b != 0 => DefaultValue::IntLiteral(a / b),
                    syn::BinOp::Rem(_) if b != 0 => DefaultValue::IntLiteral(a % b),
                    syn::BinOp::Shl(_) if (0..63).contains(&b) => a
                        .checked_shl(b as u32)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Shr(_) if (0..63).contains(&b) => DefaultValue::IntLiteral(a >> (b as u32)),
                    syn::BinOp::BitOr(_) => DefaultValue::IntLiteral(a | b),
                    syn::BinOp::BitAnd(_) => DefaultValue::IntLiteral(a & b),
                    syn::BinOp::BitXor(_) => DefaultValue::IntLiteral(a ^ b),
                    _ => unreadable(expr),
                },
                (DefaultValue::FloatLiteral(a), DefaultValue::FloatLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => DefaultValue::FloatLiteral(a + b),
                    syn::BinOp::Sub(_) => DefaultValue::FloatLiteral(a - b),
                    syn::BinOp::Mul(_) => DefaultValue::FloatLiteral(a * b),
                    syn::BinOp::Div(_) if b != 0.0 => DefaultValue::FloatLiteral(a / b),
                    _ => unreadable(expr),
                },
                _ => unreadable(expr),
            }
        }

        syn::Expr::MethodCall(mc) => {
            let method_name = mc.method.to_string();
            match method_name.as_str() {
                "to_string" | "to_owned" | "into" => {
                    if let syn::Expr::Lit(lit) = &*mc.receiver
                        && let syn::Lit::Str(s) = &lit.lit
                    {
                        return DefaultValue::StringLiteral(s.value());
                    }
                    match resolve_ident(&mc.receiver, scope) {
                        // `.to_string()` / `.to_owned()` on a non-string is a *conversion*, so
                        // only a string receiver survives them unchanged. `.into()` is
                        // identity-preserving for every value kind alef can represent. ~keep
                        Some(value @ DefaultValue::StringLiteral(_)) => value,
                        Some(value) if method_name == "into" => value,
                        _ => unreadable(expr),
                    }
                }
                _ => unreadable(expr),
            }
        }

        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = &*call.func {
                let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();

                if (segments == ["Some"] || segments == ["Option", "Some"])
                    && call.args.len() == 1
                    && let Some(inner) = call.args.first()
                {
                    return expr_to_default_value(inner, scope, field_ty);
                }

                if segments == ["String", "from"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Str(s) = &lit.lit
                    {
                        return DefaultValue::StringLiteral(s.value());
                    }
                    if let Some(argument) = call.args.first()
                        && let Some(value @ DefaultValue::StringLiteral(_)) = resolve_ident(argument, scope)
                    {
                        return value;
                    }
                    return unreadable(expr);
                }

                if segments == ["String", "new"] && call.args.is_empty() {
                    return DefaultValue::StringLiteral(String::new());
                }

                // `Cow::Borrowed("")` carries the value in its argument; the `Cow` itself is a
                // representation the binding layer already erases (`FieldDef::core_wrapper`), so
                // reading through it is not a guess. ~keep
                if let [.., owner, variant] = segments.as_slice()
                    && owner == "Cow"
                    && matches!(variant.as_str(), "Borrowed" | "Owned")
                    && call.args.len() == 1
                    && let Some(inner) = call.args.first()
                {
                    // The boundary the erasure argument does not cross: it holds for a value alef
                    // actually read. `Cow::Owned(detect_language())` names a core-private function,
                    // and a binding that cannot call it would render the name as the default. ~keep
                    return match expr_to_default_value(inner, scope, field_ty) {
                        DefaultValue::Unresolved(_) | DefaultValue::FunctionCall(_) => unreadable(expr),
                        resolved => resolved,
                    };
                }

                // The one family of calls whose result really is the type's zero, so `Empty` is an
                // assertion here rather than a fallback. ~keep
                if segments.len() == 2 && segments[1] == "new" && call.args.is_empty() {
                    let type_name = &segments[0];
                    if matches!(
                        type_name.as_str(),
                        "Vec" | "HashMap" | "HashSet" | "BTreeMap" | "BTreeSet" | "AHashMap" | "AHashSet"
                    ) {
                        return DefaultValue::Empty;
                    }
                }

                if segments == ["Duration", "from_secs"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Int(i) = &lit.lit
                        && let Ok(val) = i.base10_parse::<i64>()
                    {
                        return DefaultValue::IntLiteral(val * 1000);
                    }
                    return unreadable(expr);
                }

                if segments == ["Duration", "from_millis"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Int(i) = &lit.lit
                        && let Ok(val) = i.base10_parse::<i64>()
                    {
                        return DefaultValue::IntLiteral(val);
                    }
                    return unreadable(expr);
                }

                // `T::default()` / `Default::default()` is the type's zero by definition. ~keep
                if segments.last().is_some_and(|s| s == "default") {
                    return DefaultValue::Empty;
                }

                // A tuple-variant or tuple-struct constructor call whose arguments each fold
                // independently (`Mode::Custom(5)`, `Weight(1)`). Gated on an upper-case callee
                // for the same reason `const_literal_value` gates its tuple-struct fold: a
                // lower-case callee is a function by Rust convention, and reading through it
                // would be interpretation, not reading. All-or-nothing like `ListLiteral`: one
                // unfoldable argument leaves the whole call unreadable rather than a
                // partially-known payload. ~keep
                if !call.args.is_empty()
                    && let Some(variant) = segments.last()
                    && variant.starts_with(|c: char| c.is_ascii_uppercase())
                {
                    let mut values = Vec::with_capacity(call.args.len());
                    for argument in &call.args {
                        let value = expr_to_default_value(argument, scope, None);
                        if !carries_value(&value) {
                            return unreadable(expr);
                        }
                        values.push(value);
                    }
                    return DefaultValue::TupleVariant(variant.clone(), values);
                }

                if call.args.is_empty() {
                    return DefaultValue::FunctionCall(segments.join("::"));
                }
            }
            unreadable(expr)
        }

        // A struct-variant enum default (`Kind::Curated { label: "x".to_string() }`) or a plain
        // nested struct-literal default, each named field folded independently. Same
        // all-or-nothing rule as `TupleVariant`/`ListLiteral`: one unfoldable field, or a `..`
        // base that could carry fields this pass never saw, leaves the whole expression
        // unreadable rather than a partially-known payload. ~keep
        syn::Expr::Struct(struct_expr) => {
            if struct_expr.rest.is_some() {
                return unreadable(expr);
            }
            let Some(variant) = struct_expr.path.segments.last().map(|s| s.ident.to_string()) else {
                return unreadable(expr);
            };
            let mut fields = Vec::with_capacity(struct_expr.fields.len());
            for field_value in &struct_expr.fields {
                let Some(name) = field_value.member_named() else {
                    return unreadable(expr);
                };
                let value = expr_to_default_value(&field_value.expr, scope, None);
                if !carries_value(&value) {
                    return unreadable(expr);
                }
                fields.push((name.to_string(), value));
            }
            DefaultValue::StructVariant(variant, fields)
        }

        syn::Expr::Path(path) => {
            // `resolve_ident` covers the *readable* paths, including the associated const that
            // makes `model: Self::DEFAULT_MODEL` a string rather than a variant named
            // `DEFAULT_MODEL`. A value alef can resolve always beats a classification it can
            // only infer, so this runs before the enum reading below. ~keep
            if let Some(value) = resolve_ident(expr, scope) {
                return value;
            }
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() == 1 && segments[0] == "None" {
                return DefaultValue::None;
            }
            // A variant may be named through any number of module segments
            // (`crate::types::ResultFormat::Unified`), and only the last one is the variant. A
            // single segment is a bare identifier `resolve_ident` already failed to bind. ~keep
            if segments.len() >= 2
                && admits_enum_variant(field_ty)
                && let Some(name) = segments.last()
            {
                return DefaultValue::EnumVariant(name.clone());
            }
            unreadable(expr)
        }

        syn::Expr::Macro(mac) => {
            let macro_name = mac
                .mac
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if !matches!(macro_name.as_str(), "vec" | "hashmap" | "hashset") {
                return unreadable(expr);
            }
            // An empty collection macro really is the type's zero. ~keep
            if mac.mac.tokens.is_empty() {
                return DefaultValue::Empty;
            }
            // Only `vec!` is destructured. `hashmap!`/`hashset!` carry key-value and set
            // semantics `DefaultValue` cannot represent, so a populated one is unreadable rather
            // than flattened into a list that would render wrongly. ~keep
            if macro_name != "vec" {
                return unreadable(expr);
            }
            let Ok(elements) = mac
                .mac
                .parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
            else {
                // `vec![expr; N]` is not a comma list and fails to parse as one. ~keep
                return unreadable(expr);
            };
            if elements.is_empty() {
                return DefaultValue::Empty;
            }
            let mut lowered = Vec::with_capacity(elements.len());
            for element in &elements {
                let value = expr_to_default_value(element, scope, field_ty);
                // Only self-contained values may sit in an element position. A function-call
                // default cannot be evaluated at generation time, and `Empty`/`None`/
                // `Unresolved` carry no element value at all; any of them makes the whole
                // literal non-representable. Lowering a partial list would hand a backend a
                // default that silently differs from the Rust one. ~keep
                if !carries_value(&value) {
                    return unreadable(expr);
                }
                lowered.push(value);
            }
            DefaultValue::ListLiteral(lowered)
        }

        _ => unreadable(expr),
    }
}

/// Resolves a path expression against the scope: a bound constructor parameter first, then a
/// module-level string constant, then — for a two-segment path — an associated `&str` const of
/// the named type.
///
/// Parameters win because they are the narrower binding — inside a constructor body an
/// identifier that shadows a module const refers to the parameter. ~keep
///
/// The two-segment case has to live here rather than only in the `Expr::Path` arm of
/// [`expr_to_default_value`], because `Self::DEFAULT_MODEL.to_string()` reaches the extractor as
/// a *method call* whose receiver is the path, and the method-call arm resolves its receiver
/// through this function. ~keep
fn resolve_ident(expr: &syn::Expr, scope: &EvalScope<'_>) -> Option<DefaultValue> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };
    let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
    match segments.as_slice() {
        [ident] => {
            if let Some(value) = scope.params.get(ident) {
                return Some(value.clone());
            }
            scope.literal_consts.get(ident).cloned()
        }
        [.., owner, name] => scope.associated_const(owner, name),
        [] => None,
    }
}

#[cfg(test)]
mod tests;
