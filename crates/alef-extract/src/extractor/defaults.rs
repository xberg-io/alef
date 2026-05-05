use ahash::AHashMap;
use alef_core::ir::{DefaultValue, FieldDef};
use syn;

/// Extract concrete default values from an `impl Default for T` block.
///
/// Finds the `fn default() -> Self` method, parses its struct literal body,
/// and maps each field initializer expression to a `DefaultValue` variant.
/// Falls back to `DefaultValue::Empty` for expressions that cannot be parsed
/// into a concrete literal (e.g., method calls, complex expressions).
pub(crate) fn extract_default_values(item: &syn::ItemImpl, fields: &mut [FieldDef]) {
    // Find the `fn default()` method
    let default_fn = item.items.iter().find_map(|impl_item| {
        if let syn::ImplItem::Fn(method) = impl_item {
            if method.sig.ident == "default" {
                return Some(method);
            }
        }
        None
    });

    let Some(default_fn) = default_fn else {
        // No fn default() found — mark all fields as Empty
        for field in fields.iter_mut() {
            field.typed_default = Some(DefaultValue::Empty);
        }
        return;
    };

    // Build a map of field name → DefaultValue from the struct literal
    let defaults = parse_default_body(&default_fn.block);

    for field in fields.iter_mut() {
        if let Some(default_val) = defaults.get(&field.name) {
            field.typed_default = Some(default_val.clone());
        } else {
            // Field exists but wasn't in the struct literal — use Empty
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

    // The body should contain a struct literal, possibly as the last expression.
    // It could be `Self { ... }` or `TypeName { ... }`.
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
    // Check the last statement (tail expression or expression statement)
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
        // Boolean and numeric literals
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

        // Unary negation: `-1`, `-3.14`
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => match expr_to_default_value(&unary.expr) {
            DefaultValue::IntLiteral(v) => DefaultValue::IntLiteral(-v),
            DefaultValue::FloatLiteral(v) => DefaultValue::FloatLiteral(-v),
            _ => DefaultValue::Empty,
        },

        // Binary arithmetic: const-fold `a OP b` where both sides are integer
        // (or both float) literals — supports common patterns like
        // `500 * 1024 * 1024` (left-associative chains fold recursively).
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

        // Method calls: "str".to_string(), "str".into(), etc.
        syn::Expr::MethodCall(mc) => {
            let method_name = mc.method.to_string();
            match method_name.as_str() {
                "to_string" | "to_owned" | "into" => {
                    // Check if receiver is a string literal
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

        // Function/associated function calls: String::from("..."), String::new(), Vec::new(),
        // SomeType::default(), Default::default()
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = &*call.func {
                let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();

                // String::from("...") or String::from(lit)
                if segments == ["String", "from"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first() {
                        if let syn::Lit::Str(s) = &lit.lit {
                            return DefaultValue::StringLiteral(s.value());
                        }
                    }
                    return DefaultValue::Empty;
                }

                // String::new() → empty string
                if segments == ["String", "new"] && call.args.is_empty() {
                    return DefaultValue::StringLiteral(String::new());
                }

                // Vec::new(), HashMap::new(), HashSet::new(), etc.
                if segments.len() == 2 && segments[1] == "new" && call.args.is_empty() {
                    let type_name = &segments[0];
                    if matches!(
                        type_name.as_str(),
                        "Vec" | "HashMap" | "HashSet" | "BTreeMap" | "BTreeSet" | "AHashMap" | "AHashSet"
                    ) {
                        return DefaultValue::Empty;
                    }
                }

                // Duration::from_secs(N) → IntLiteral(N * 1000) (milliseconds)
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

                // Duration::from_millis(N) → IntLiteral(N) (already milliseconds)
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

                // SomeType::default() or Default::default()
                if segments.last().is_some_and(|s| s == "default") {
                    return DefaultValue::Empty;
                }
            }
            DefaultValue::Empty
        }

        // Path expressions: SomeEnum::Variant (no function call), or bare `None`
        syn::Expr::Path(path) => {
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() == 2 {
                // SomeEnum::Variant → EnumVariant("Variant")
                return DefaultValue::EnumVariant(segments[1].clone());
            }
            // Bare `None` → DefaultValue::None
            if segments.len() == 1 && segments[0] == "None" {
                return DefaultValue::None;
            }
            // Single ident like `true`/`false` are handled as Lit, but just in case
            DefaultValue::Empty
        }

        // Macro calls: vec![], hashmap!{}, etc.
        syn::Expr::Macro(mac) => {
            // vec![] with empty tokens → Empty
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
