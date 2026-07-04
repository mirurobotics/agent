// standard crates
use std::path::{Path, PathBuf};

// external crates
use syn::visit::Visit;

/// A single lint violation: a function or closure whose body exceeds the
/// maximum number of non-blank, non-comment lines.
pub struct Violation {
    pub file: PathBuf,
    /// 1-based line of the `fn` keyword (or closure start).
    pub line: usize,
    /// `None` for closures.
    pub name: Option<String>,
    /// Non-blank, non-comment body lines.
    pub count: usize,
    pub limit: usize,
}

/// Scan a source file for functions and closures whose bodies exceed
/// `max_lines` non-blank, non-comment lines. A `max_lines` of 0 disables the
/// check. Test code (files under a `tests` directory, `#[cfg(test)]` items,
/// `#[test]` functions, `#[cfg(feature = "test")]` items) is exempt.
pub fn check_file(path: &Path, source: &str, max_lines: usize) -> Vec<Violation> {
    if max_lines == 0 {
        return Vec::new();
    }
    if path.components().any(|c| c.as_os_str() == "tests") {
        return Vec::new();
    }
    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut visitor = FnVisitor {
        path,
        lines: source.lines().collect(),
        max_lines,
        violations: Vec::new(),
    };
    visitor.visit_file(&file);
    visitor.violations
}

struct FnVisitor<'a> {
    path: &'a Path,
    lines: Vec<&'a str>,
    max_lines: usize,
    violations: Vec<Violation>,
}

impl FnVisitor<'_> {
    fn check_block(&mut self, block: &syn::Block, anchor: usize, name: Option<String>) {
        if self.is_suppressed(anchor) {
            return;
        }
        let count = count_body_lines(&self.lines, block);
        if count > self.max_lines {
            self.violations.push(Violation {
                file: self.path.to_path_buf(),
                line: anchor,
                name,
                count,
                limit: self.max_lines,
            });
        }
    }

    /// True when the 1-based `line` or the line immediately above it contains
    /// the `lint:allow(funclen)` escape hatch.
    fn is_suppressed(&self, line: usize) -> bool {
        let has_marker = |n: usize| {
            n >= 1
                && self
                    .lines
                    .get(n - 1)
                    .is_some_and(|l| l.contains("lint:allow(funclen)"))
        };
        has_marker(line) || has_marker(line.saturating_sub(1))
    }
}

impl<'ast> Visit<'ast> for FnVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if is_test_cfg(&node.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if is_test_cfg(&node.attrs) {
            return;
        }
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        if is_test_cfg(&node.attrs) {
            return;
        }
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if is_test_cfg(&node.attrs) || is_test_fn(&node.attrs) {
            return;
        }
        let line = node.sig.fn_token.span.start().line;
        self.check_block(&node.block, line, Some(node.sig.ident.to_string()));
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if is_test_cfg(&node.attrs) || is_test_fn(&node.attrs) {
            return;
        }
        let line = node.sig.fn_token.span.start().line;
        self.check_block(&node.block, line, Some(node.sig.ident.to_string()));
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if is_test_cfg(&node.attrs) || is_test_fn(&node.attrs) {
            return;
        }
        if let Some(block) = &node.default {
            let line = node.sig.fn_token.span.start().line;
            self.check_block(block, line, Some(node.sig.ident.to_string()));
        }
        syn::visit::visit_trait_item_fn(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        if let syn::Expr::Block(body) = &*node.body {
            let line = node.or1_token.span.start().line;
            self.check_block(&body.block, line, None);
        }
        syn::visit::visit_expr_closure(self, node);
    }
}

/// Count the non-blank, non-comment source lines strictly between the block's
/// braces (the brace lines themselves are excluded).
fn count_body_lines(lines: &[&str], block: &syn::Block) -> usize {
    let open = block.brace_token.span.open().start().line;
    let close = block.brace_token.span.close().start().line;
    if close <= open + 1 {
        return 0;
    }
    lines
        .get(open..close - 1)
        .unwrap_or(&[])
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("//")
        })
        .count()
}

/// True for `#[cfg(test)]`, `#[cfg(feature = "test")]`, `#[cfg(any(test, ...))]`
/// and similar test-gating cfg attributes.
fn is_test_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if let syn::Meta::List(ref list) = attr.meta {
            return list.path.is_ident("cfg") && list.tokens.to_string().contains("test");
        }
        false
    })
}

/// True for `#[test]`, `#[tokio::test]`, and `#[tokio::test(...)]` functions.
fn is_test_fn(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::Path(path) => path.segments.last().is_some_and(|s| s.ident == "test"),
        syn::Meta::List(list) => list.path.segments.last().is_some_and(|s| s.ident == "test"),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(n: usize) -> String {
        "    let x = 1;\n".repeat(n)
    }

    fn src_fn(n: usize) -> String {
        format!("fn sample() {{\n{}}}\n", body(n))
    }

    fn check(source: &str, max_lines: usize) -> Vec<Violation> {
        check_file(Path::new("src/lib.rs"), source, max_lines)
    }

    #[test]
    fn under_limit_no_violation() {
        assert!(check(&src_fn(2), 3).is_empty());
    }

    #[test]
    fn exactly_at_limit_no_violation() {
        assert!(check(&src_fn(3), 3).is_empty());
    }

    #[test]
    fn over_limit_reports_violation() {
        let violations = check(&src_fn(4), 3);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].name.as_deref(), Some("sample"));
        assert_eq!(violations[0].count, 4);
        assert_eq!(violations[0].line, 1);
        assert_eq!(violations[0].limit, 3);
    }

    #[test]
    fn blank_lines_not_counted() {
        let src = format!("fn sample() {{\n{}\n\n\n{}}}\n", body(2), body(1));
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn comment_lines_not_counted() {
        let src = format!(
            "fn sample() {{\n    // one\n    // two\n    // three\n{}}}\n",
            body(3)
        );
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn closure_over_limit_reported_without_name() {
        let src = format!("fn outer() {{\n    let f = || {{\n{}    }};\n}}\n", body(4));
        let violations = check(&src, 3);
        let closure = violations
            .iter()
            .find(|v| v.name.is_none())
            .expect("closure violation");
        assert_eq!(closure.count, 4);
        assert_eq!(closure.line, 2);
    }

    #[test]
    fn impl_method_over_limit_reported() {
        let src = format!(
            "struct S;\nimpl S {{\n    fn method(&self) {{\n{}    }}\n}}\n",
            body(4)
        );
        let violations = check(&src, 3);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].name.as_deref(), Some("method"));
    }

    #[test]
    fn trait_default_method_reported_and_bodyless_ignored() {
        let src = format!(
            "trait T {{\n    fn no_body(&self);\n    fn with_body(&self) {{\n{}    }}\n}}\n",
            body(4)
        );
        let violations = check(&src, 3);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].name.as_deref(), Some("with_body"));
    }

    #[test]
    fn test_fn_exempt() {
        let src = format!("#[test]\nfn sample() {{\n{}}}\n", body(10));
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn tokio_test_fn_exempt() {
        let src = format!("#[tokio::test]\nasync fn sample() {{\n{}}}\n", body(10));
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn cfg_test_mod_exempt() {
        let src = format!(
            "#[cfg(test)]\nmod tests {{\n    fn helper() {{\n{}    }}\n}}\n",
            body(10)
        );
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn cfg_feature_test_fn_exempt() {
        let src = format!("#[cfg(feature = \"test\")]\nfn sample() {{\n{}}}\n", body(10));
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn tests_path_exempt() {
        let violations = check_file(Path::new("agent/tests/foo.rs"), &src_fn(10), 3);
        assert!(violations.is_empty());
    }

    #[test]
    fn suppression_on_line_above_fn() {
        let src = format!("// lint:allow(funclen)\nfn sample() {{\n{}}}\n", body(10));
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn suppression_trailing_on_fn_line() {
        let src = format!("fn sample() {{ // lint:allow(funclen)\n{}}}\n", body(10));
        assert!(check(&src, 3).is_empty());
    }

    #[test]
    fn other_allow_comment_does_not_suppress() {
        let src = format!(
            "// lint:allow(field-by-field-assert)\nfn sample() {{\n{}}}\n",
            body(10)
        );
        assert_eq!(check(&src, 3).len(), 1);
    }

    #[test]
    fn threshold_zero_disables() {
        assert!(check(&src_fn(100), 0).is_empty());
    }

    #[test]
    fn unparseable_source_returns_empty() {
        assert!(check("this is not valid rust at all {{{", 3).is_empty());
    }
}
