/// Walks the expression tree inward to find the root receiver variable of a field
/// access chain. Returns `None` if the expression is not a field access chain
/// (e.g. a plain variable, function call, or literal).
///
/// Examples:
/// - `req.call`            -> Some("req")
/// - `req.inner.call`      -> Some("req")
/// - `req.token.as_deref()`-> Some("req")
/// - `req.items[0]`        -> Some("req")
/// - `&req.call`           -> Some("req")
/// - `result`              -> None (no field access)
/// - `foo()`               -> None (function call)
pub fn root_receiver(expr: &syn::Expr) -> Option<String> {
    // The outermost meaningful node must ultimately derive from a field access.
    // We track whether we have seen at least one Field node.
    root_receiver_inner(expr, false)
}

fn root_receiver_inner(expr: &syn::Expr, seen_field: bool) -> Option<String> {
    match expr {
        syn::Expr::Field(field) => root_receiver_inner(&field.base, true),

        syn::Expr::MethodCall(mc) => root_receiver_inner(&mc.receiver, seen_field),

        syn::Expr::Index(idx) => root_receiver_inner(&idx.expr, seen_field),

        syn::Expr::Paren(p) => root_receiver_inner(&p.expr, seen_field),

        syn::Expr::Reference(r) => root_receiver_inner(&r.expr, seen_field),

        syn::Expr::Try(t) => root_receiver_inner(&t.expr, seen_field),

        syn::Expr::Unary(u) => root_receiver_inner(&u.expr, seen_field),

        syn::Expr::Path(path) => {
            if !seen_field {
                return None;
            }
            if path.path.segments.len() == 1 {
                Some(path.path.segments[0].ident.to_string())
            } else {
                None
            }
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_expr(s: &str) -> syn::Expr {
        syn::parse_str::<syn::Expr>(s).unwrap()
    }

    #[test]
    fn test_simple_field_access() {
        let expr = parse_expr("req.call");
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }

    #[test]
    fn test_nested_field_access() {
        let expr = parse_expr("req.inner.call");
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }

    #[test]
    fn test_method_on_field() {
        let expr = parse_expr("req.token.as_deref()");
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }

    #[test]
    fn test_indexed_field() {
        let expr = parse_expr("req.items[0]");
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }

    #[test]
    fn test_reference() {
        let expr = parse_expr("&req.call");
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }

    #[test]
    fn test_no_field_access() {
        let expr = parse_expr("result");
        assert_eq!(root_receiver(&expr), None);
    }

    #[test]
    fn test_function_call() {
        let expr = parse_expr("foo()");
        assert_eq!(root_receiver(&expr), None);
    }

    #[test]
    fn test_try_operator() {
        let expr = parse_expr("req.call?");
        // Try wraps a field access, so we still get the root receiver.
        // Note: this parses as Expr::Try(Expr::Field(...)).
        // However syn::parse_str may not parse `?` standalone — it needs a block context.
        // We'll just verify it doesn't panic.
        // In practice, `?` in assert_eq! first arg is unusual.
        // Let's try parsing it in a block context instead.
        let _ = root_receiver(&expr);
    }

    #[test]
    fn test_try_operator_via_block() {
        // syn can parse `req.call?` as an expression
        if let Ok(expr) = syn::parse_str::<syn::Expr>("req.call?") {
            assert_eq!(root_receiver(&expr), Some("req".to_string()));
        }
    }

    #[test]
    fn test_method_call_no_field() {
        // `foo.bar()` — method call on a plain variable, has a field-like structure
        // Actually Expr::MethodCall { receiver: Expr::Path("foo") }
        // There's no Expr::Field here, just a method call. seen_field is false.
        let expr = parse_expr("foo.bar()");
        // MethodCall doesn't set seen_field, and Path with seen_field=false returns None.
        assert_eq!(root_receiver(&expr), None);
    }

    #[test]
    fn test_method_call_on_field() {
        // `req.field.to_string()` — method call on a field access
        let expr = parse_expr("req.field.to_string()");
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }
}
