use crate::core::ir::{DefaultValue, FieldDef};
use ahash::AHashMap;
use syn;

/// Extract concrete default values from an `impl Default for T` block.
///
/// Finds the `fn default() -> Self` method, parses its struct literal body,
/// and maps each field initializer expression to a `DefaultValue` variant.
/// Falls back to `DefaultValue::Empty` for expressions that cannot be parsed
/// into a concrete literal (e.g., method calls, complex expressions).
pub(crate) fn extract_default_values(item: &syn::ItemImpl, fields: &mut [FieldDef]) {
    let default_fn = item.items.iter().find_map(|impl_item| {
        if let syn::ImplItem::Fn(method) = impl_item {
            if method.sig.ident == "default" {
                return Some(method);
            }
        }
        None
    });

    let Some(default_fn) = default_fn else {
        for field in fields.iter_mut() {
            field.typed_default = Some(DefaultValue::Empty);
        }
        return;
    };

    let defaults = parse_default_body(&default_fn.block);

    for field in fields.iter_mut() {
        if let Some(default_val) = defaults.get(&field.name) {
            field.typed_default = Some(default_val.clone());
        } else {
            field.typed_default = Some(DefaultValue::Empty);
        }
    }
}

/// Parse the body of a `fn default()` to extract field → `DefaultValue` mappings.
///
/// Looks for a struct literal (`Self { field: expr, ... }`) in the function body
/// and maps each field initializer to a `DefaultValue`.
fn parse_default_body(block: &syn::Block) -> AHashMap<String, DefaultValue> {
    let mut defaults = AHashMap::new();

    let struct_expr = find_struct_expr(block);

    let Some(struct_expr) = struct_expr else {
        return defaults;
    };

    for field in &struct_expr.fields {
        let Some(ident) = &field.member_named() else {
            continue;
        };
        let field_name = ident.to_string();
        let default_val = expr_to_default_value(&field.expr);
        defaults.insert(field_name, default_val);
    }

    defaults
}

/// Recursively search a block for a struct expression (`Self { ... }` or `Name { ... }`).
fn find_struct_expr(block: &syn::Block) -> Option<&syn::ExprStruct> {
    for stmt in block.stmts.iter().rev() {
        match stmt {
            syn::Stmt::Expr(expr, _) => {
                if let Some(s) = unwrap_to_struct_expr(expr) {
                    return Some(s);
                }
            }
            syn::Stmt::Local(local) => {
                if let Some(init) = &local.init {
                    if let Some(s) = unwrap_to_struct_expr(&init.expr) {
                        return Some(s);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Try to unwrap an expression to a struct expression, looking through blocks.
fn unwrap_to_struct_expr(expr: &syn::Expr) -> Option<&syn::ExprStruct> {
    match expr {
        syn::Expr::Struct(s) => Some(s),
        syn::Expr::Block(b) => find_struct_expr(&b.block),
        _ => None,
    }
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

/// Convert an expression to a `DefaultValue`.
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
/// - `SomeEnum::Variant` → `EnumVariant("Variant")`
/// - Anything else → `Empty`
fn expr_to_default_value(expr: &syn::Expr) -> DefaultValue {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Bool(b) => DefaultValue::BoolLiteral(b.value),
            syn::Lit::Int(i) => {
                if let Ok(val) = i.base10_parse::<i64>() {
                    DefaultValue::IntLiteral(val)
                } else {
                    DefaultValue::Empty
                }
            }
            syn::Lit::Float(f) => {
                if let Ok(val) = f.base10_parse::<f64>() {
                    DefaultValue::FloatLiteral(val)
                } else {
                    DefaultValue::Empty
                }
            }
            syn::Lit::Char(c) => DefaultValue::StringLiteral(c.value().to_string()),
            syn::Lit::Str(s) => DefaultValue::StringLiteral(s.value()),
            _ => DefaultValue::Empty,
        },

        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => match expr_to_default_value(&unary.expr) {
            DefaultValue::IntLiteral(v) => DefaultValue::IntLiteral(-v),
            DefaultValue::FloatLiteral(v) => DefaultValue::FloatLiteral(-v),
            _ => DefaultValue::Empty,
        },

        syn::Expr::Binary(bin) => {
            let lhs = expr_to_default_value(&bin.left);
            let rhs = expr_to_default_value(&bin.right);
            match (lhs, rhs) {
                (DefaultValue::IntLiteral(a), DefaultValue::IntLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => a
                        .checked_add(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Sub(_) => a
                        .checked_sub(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Mul(_) => a
                        .checked_mul(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Div(_) if b != 0 => DefaultValue::IntLiteral(a / b),
                    syn::BinOp::Rem(_) if b != 0 => DefaultValue::IntLiteral(a % b),
                    syn::BinOp::Shl(_) if (0..63).contains(&b) => a
                        .checked_shl(b as u32)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Shr(_) if (0..63).contains(&b) => DefaultValue::IntLiteral(a >> (b as u32)),
                    syn::BinOp::BitOr(_) => DefaultValue::IntLiteral(a | b),
                    syn::BinOp::BitAnd(_) => DefaultValue::IntLiteral(a & b),
                    syn::BinOp::BitXor(_) => DefaultValue::IntLiteral(a ^ b),
                    _ => DefaultValue::Empty,
                },
                (DefaultValue::FloatLiteral(a), DefaultValue::FloatLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => DefaultValue::FloatLiteral(a + b),
                    syn::BinOp::Sub(_) => DefaultValue::FloatLiteral(a - b),
                    syn::BinOp::Mul(_) => DefaultValue::FloatLiteral(a * b),
                    syn::BinOp::Div(_) if b != 0.0 => DefaultValue::FloatLiteral(a / b),
                    _ => DefaultValue::Empty,
                },
                _ => DefaultValue::Empty,
            }
        }

        syn::Expr::MethodCall(mc) => {
            let method_name = mc.method.to_string();
            match method_name.as_str() {
                "to_string" | "to_owned" | "into" => {
                    if let syn::Expr::Lit(lit) = &*mc.receiver {
                        if let syn::Lit::Str(s) = &lit.lit {
                            return DefaultValue::StringLiteral(s.value());
                        }
                    }
                    DefaultValue::Empty
                }
                _ => DefaultValue::Empty,
            }
        }

        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = &*call.func {
                let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();

                if (segments == ["Some"] || segments == ["Option", "Some"]) && call.args.len() == 1 {
                    if let Some(inner) = call.args.first() {
                        return expr_to_default_value(inner);
                    }
                }

                if segments == ["String", "from"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first() {
                        if let syn::Lit::Str(s) = &lit.lit {
                            return DefaultValue::StringLiteral(s.value());
                        }
                    }
                    return DefaultValue::Empty;
                }

                if segments == ["String", "new"] && call.args.is_empty() {
                    return DefaultValue::StringLiteral(String::new());
                }

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
                    if let Some(syn::Expr::Lit(lit)) = call.args.first() {
                        if let syn::Lit::Int(i) = &lit.lit {
                            if let Ok(val) = i.base10_parse::<i64>() {
                                return DefaultValue::IntLiteral(val * 1000);
                            }
                        }
                    }
                    return DefaultValue::Empty;
                }

                if segments == ["Duration", "from_millis"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first() {
                        if let syn::Lit::Int(i) = &lit.lit {
                            if let Ok(val) = i.base10_parse::<i64>() {
                                return DefaultValue::IntLiteral(val);
                            }
                        }
                    }
                    return DefaultValue::Empty;
                }

                if segments.last().is_some_and(|s| s == "default") {
                    return DefaultValue::Empty;
                }
            }
            DefaultValue::Empty
        }

        syn::Expr::Path(path) => {
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() == 2 {
                return DefaultValue::EnumVariant(segments[1].clone());
            }
            if segments.len() == 1 && segments[0] == "None" {
                return DefaultValue::None;
            }
            DefaultValue::Empty
        }

        syn::Expr::Macro(mac) => {
            let macro_name = mac
                .mac
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if matches!(macro_name.as_str(), "vec" | "hashmap" | "hashset") && mac.mac.tokens.is_empty() {
                return DefaultValue::Empty;
            }
            DefaultValue::Empty
        }

        _ => DefaultValue::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_value_of(expr_src: &str) -> DefaultValue {
        let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
        expr_to_default_value(&expr)
    }

    #[test]
    fn some_int_literal_unwraps_to_inner_int() {
        assert_eq!(
            default_value_of("Some(50 * 1024 * 1024)"),
            DefaultValue::IntLiteral(52_428_800)
        );
    }

    #[test]
    fn some_string_literal_unwraps_to_inner_string() {
        assert_eq!(
            default_value_of(r#"Some("hi".to_string())"#),
            DefaultValue::StringLiteral("hi".to_string())
        );
    }

    #[test]
    fn qualified_option_some_unwraps() {
        assert_eq!(default_value_of("Option::Some(5)"), DefaultValue::IntLiteral(5));
    }

    #[test]
    fn bare_none_stays_none() {
        assert_eq!(default_value_of("None"), DefaultValue::None);
    }
}
