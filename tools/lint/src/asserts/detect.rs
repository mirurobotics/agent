// standard crates
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// internal crates
use super::extract::root_receiver;

// external crates
use proc_macro2::{TokenStream, TokenTree};

/// A single lint violation: a group of 4+ `assert_eq!` calls on fields of the
/// same variable within a test function.
#[allow(dead_code)]
pub struct Violation {
    pub file: PathBuf,
    pub line: usize,
    pub test_fn: String,
    pub receiver: String,
    pub count: usize,
}

/// Scan a source file for test functions that have too many field-by-field
/// `assert_eq!` calls on the same receiver variable.
pub fn check_file(path: &Path, source: &str, threshold: usize) -> Vec<Violation> {
    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut violations = Vec::new();
    collect_from_items(&file.items, path, source, threshold, &mut violations);
    violations
}

fn collect_from_items(
    items: &[syn::Item],
    path: &Path,
    source: &str,
    threshold: usize,
    violations: &mut Vec<Violation>,
) {
    for item in items {
        match item {
            syn::Item::Fn(item_fn) if is_test_fn(item_fn) => {
                check_test_fn(item_fn, path, source, threshold, violations);
            }
            syn::Item::Mod(item_mod) => {
                if let Some((_, ref items)) = item_mod.content {
                    collect_from_items(items, path, source, threshold, violations);
                }
            }
            _ => {}
        }
    }
}

fn is_test_fn(f: &syn::ItemFn) -> bool {
    f.attrs.iter().any(|attr| {
        if let syn::Meta::Path(ref path) = attr.meta {
            // #[test]
            if path.segments.last().is_some_and(|s| s.ident == "test") {
                return true;
            }
        }
        if let syn::Meta::List(ref list) = attr.meta {
            // #[tokio::test] or #[tokio::test(...)]
            if list.path.segments.last().is_some_and(|s| s.ident == "test") {
                return true;
            }
        }
        false
    })
}

fn check_test_fn(
    item_fn: &syn::ItemFn,
    path: &Path,
    source: &str,
    threshold: usize,
    violations: &mut Vec<Violation>,
) {
    // Check for escape hatch in the raw source lines within the function span.
    let fn_name = item_fn.sig.ident.to_string();

    // Use span to find the line range of the function in the source.
    let open_span = item_fn.block.brace_token.span.open();
    let close_span = item_fn.block.brace_token.span.close();
    let start_line = open_span.start().line;
    let end_line = close_span.end().line;

    // Extract the source lines for this function and check for escape hatch.
    let lines: Vec<&str> = source.lines().collect();
    let fn_start = start_line.saturating_sub(1); // 0-indexed
    let fn_end = end_line.min(lines.len());
    for line in &lines[fn_start..fn_end] {
        if line.contains("lint:allow(field-by-field-assert)") {
            return;
        }
    }

    // Collect assert_eq! calls with field access receivers.
    let mut records: Vec<(String, usize)> = Vec::new(); // (receiver, line)

    collect_assert_eqs_from_stmts(&item_fn.block.stmts, &mut records);

    // Group by receiver.
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (receiver, line) in records {
        groups.entry(receiver).or_default().push(line);
    }

    for (receiver, mut line_numbers) in groups {
        if line_numbers.len() >= threshold {
            line_numbers.sort();
            violations.push(Violation {
                file: path.to_path_buf(),
                line: line_numbers[0],
                test_fn: fn_name.clone(),
                receiver,
                count: line_numbers.len(),
            });
        }
    }
}

fn collect_assert_eqs_from_stmts(stmts: &[syn::Stmt], records: &mut Vec<(String, usize)>) {
    for stmt in stmts {
        collect_assert_eqs_from_stmt(stmt, records);
    }
}

fn collect_assert_eqs_from_stmt(stmt: &syn::Stmt, records: &mut Vec<(String, usize)>) {
    match stmt {
        syn::Stmt::Expr(expr, _) => collect_assert_eqs_from_expr(expr, records),
        syn::Stmt::Local(local) => {
            if let Some(init) = &local.init {
                collect_assert_eqs_from_expr(&init.expr, records);
            }
        }
        syn::Stmt::Item(_) => {}
        syn::Stmt::Macro(stmt_macro) => {
            check_macro(&stmt_macro.mac, records);
        }
    }
}

fn collect_assert_eqs_from_expr(expr: &syn::Expr, records: &mut Vec<(String, usize)>) {
    match expr {
        syn::Expr::Block(block) => {
            collect_assert_eqs_from_stmts(&block.block.stmts, records);
        }
        syn::Expr::If(expr_if) => {
            collect_assert_eqs_from_stmts(&expr_if.then_branch.stmts, records);
            if let Some((_, else_branch)) = &expr_if.else_branch {
                collect_assert_eqs_from_expr(else_branch, records);
            }
        }
        syn::Expr::Match(expr_match) => {
            for arm in &expr_match.arms {
                collect_assert_eqs_from_expr(&arm.body, records);
            }
        }
        syn::Expr::ForLoop(for_loop) => {
            collect_assert_eqs_from_stmts(&for_loop.body.stmts, records);
        }
        syn::Expr::While(while_loop) => {
            collect_assert_eqs_from_stmts(&while_loop.body.stmts, records);
        }
        syn::Expr::Loop(loop_expr) => {
            collect_assert_eqs_from_stmts(&loop_expr.body.stmts, records);
        }
        syn::Expr::Unsafe(unsafe_expr) => {
            collect_assert_eqs_from_stmts(&unsafe_expr.block.stmts, records);
        }
        syn::Expr::Macro(expr_macro) => {
            check_macro(&expr_macro.mac, records);
        }
        _ => {}
    }
}

fn check_macro(mac: &syn::Macro, records: &mut Vec<(String, usize)>) {
    let is_assert_eq = mac
        .path
        .segments
        .last()
        .is_some_and(|s| s.ident == "assert_eq");

    if !is_assert_eq {
        return;
    }

    if let Some(first_arg) = parse_first_arg(&mac.tokens) {
        if let Some(receiver) = root_receiver(&first_arg) {
            let line = mac.path.segments[0].ident.span().start().line;
            records.push((receiver, line));
        }
    }
}

/// Parse the first argument from an `assert_eq!` macro's token stream.
/// Splits on the first top-level comma (not inside any bracket/paren/brace group).
fn parse_first_arg(tokens: &TokenStream) -> Option<syn::Expr> {
    let mut first_arg_tokens = TokenStream::new();
    for tt in tokens.clone() {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' => {
                break;
            }
            _ => {
                first_arg_tokens.extend(std::iter::once(tt));
            }
        }
    }

    // Try to find the top-level comma more carefully: the simple approach above
    // works because proc_macro2 already groups delimited tokens into Group nodes.
    // A comma inside parens/brackets/braces is inside a Group and won't appear
    // as a top-level Punct. So the simple iteration is correct.

    syn::parse2::<syn::Expr>(first_arg_tokens).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // standard crates
    use std::io::Write;

    // external crates
    use tempfile::NamedTempFile;

    fn write_temp_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_flags_four_field_asserts() {
        let src = r#"
#[test]
fn test_example() {
    let req = get_request();
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
    assert_eq!(req.path, "/deployments");
    assert_eq!(req.url, "http://mock/deployments");
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].receiver, "req");
        assert_eq!(violations[0].count, 4);
        assert_eq!(violations[0].test_fn, "test_example");
    }

    #[test]
    fn test_below_threshold_no_violation() {
        let src = r#"
#[test]
fn test_example() {
    let req = get_request();
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
    assert_eq!(req.path, "/deployments");
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_different_receivers_not_grouped() {
        let src = r#"
#[test]
fn test_example() {
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, "ok");
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_escape_hatch_suppresses() {
        let src = r#"
#[test]
fn test_example() {
    // lint:allow(field-by-field-assert)
    let req = get_request();
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
    assert_eq!(req.path, "/deployments");
    assert_eq!(req.url, "http://mock/deployments");
    assert_eq!(req.body, "{}");
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_non_test_function_ignored() {
        let src = r#"
fn helper() {
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
    assert_eq!(req.path, "/deployments");
    assert_eq!(req.url, "http://mock/deployments");
    assert_eq!(req.body, "{}");
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_tokio_test_detected() {
        let src = r#"
#[tokio::test]
async fn test_async_example() {
    let req = get_request().await;
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
    assert_eq!(req.path, "/deployments");
    assert_eq!(req.url, "http://mock/deployments");
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].receiver, "req");
    }

    #[test]
    fn test_nested_mod_tests() {
        let src = r#"
mod tests {
    #[test]
    fn test_nested() {
        assert_eq!(req.call, Call::GetDeployment);
        assert_eq!(req.method, Method::GET);
        assert_eq!(req.path, "/deployments");
        assert_eq!(req.url, "http://mock/deployments");
    }
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].test_fn, "test_nested");
    }

    #[test]
    fn test_unparseable_file_skipped() {
        let src = "this is not valid rust at all {{{";
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 4);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_custom_threshold() {
        let src = r#"
#[test]
fn test_example() {
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, Method::GET);
}
"#;
        let f = write_temp_file(src);
        let violations = check_file(f.path(), src, 2);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].count, 2);
    }

    #[test]
    fn test_is_test_fn_plain() {
        let item: syn::ItemFn = syn::parse_str(
            r#"
            #[test]
            fn my_test() {}
            "#,
        )
        .unwrap();
        assert!(is_test_fn(&item));
    }

    #[test]
    fn test_is_test_fn_tokio() {
        let item: syn::ItemFn = syn::parse_str(
            r#"
            #[tokio::test]
            async fn my_test() {}
            "#,
        )
        .unwrap();
        assert!(is_test_fn(&item));
    }

    #[test]
    fn test_is_not_test_fn() {
        let item: syn::ItemFn = syn::parse_str(
            r#"
            fn helper() {}
            "#,
        )
        .unwrap();
        assert!(!is_test_fn(&item));
    }

    #[test]
    fn test_parse_first_arg_simple() {
        let tokens: TokenStream = "req.call, Call::GetDeployment".parse().unwrap();
        let expr = parse_first_arg(&tokens).unwrap();
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }

    #[test]
    fn test_parse_first_arg_with_nested_comma() {
        // The comma inside the function call should not split.
        let tokens: TokenStream = "req.items, vec![1, 2, 3]".parse().unwrap();
        let expr = parse_first_arg(&tokens).unwrap();
        assert_eq!(root_receiver(&expr), Some("req".to_string()));
    }
}
