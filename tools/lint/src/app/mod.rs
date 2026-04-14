// standard crates
use std::io::Write;
use std::path::{Path, PathBuf};

// internal crates
use crate::asserts::detect::{check_file as check_asserts, Violation};
use crate::checker::check;
use crate::classifier::Classifier;
use crate::config::Config;
use crate::fixer::fix_file;
use crate::parser::parse;

// external crates
use clap::Parser as ClapParser;
use walkdir::WalkDir;

#[derive(ClapParser)]
#[command(
    name = "lint",
    about = "Lint Rust import grouping, ordering, comment headers, and test assert patterns"
)]
pub struct Cli {
    /// Directory to scan for import checking
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Auto-fix import violations in place
    #[arg(long)]
    pub fix: bool,

    /// Path to .lint-imports.toml config file
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Directories to scan for field-by-field assert detection in test files
    #[arg(long = "assert-paths", num_args = 1..)]
    pub assert_paths: Vec<PathBuf>,

    /// Minimum field-asserts on the same receiver to flag (default: 4)
    #[arg(long = "assert-threshold", default_value_t = 4)]
    pub assert_threshold: usize,
}

pub fn run(cli: &Cli, stdout: &mut impl Write, stderr: &mut impl Write) -> i32 {
    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    run_from_dir(&base_dir, cli, stdout, stderr)
}

fn run_from_dir(
    base_dir: &Path,
    cli: &Cli,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    let mut totals = Totals::default();

    // Run import checking if --path is provided.
    if let Some(ref path) = cli.path {
        let config = load_config(base_dir, cli);
        let classifier = Classifier::new(&config);

        for file_path in rust_files(base_dir, path) {
            process_file(
                &file_path,
                cli,
                &classifier,
                &config,
                &mut totals,
                stdout,
                stderr,
            );
        }
    }

    // Run assert checker if paths are specified.
    let assert_violations = if !cli.assert_paths.is_empty() {
        run_assert_check(base_dir, cli, stdout)
    } else {
        Vec::new()
    };

    if cli.fix {
        let _ = writeln!(stdout, "\n{} file(s) fixed.", totals.files_fixed);
        if !assert_violations.is_empty() {
            return 1;
        }
        return 0;
    }

    let total_issues = totals.diagnostics + assert_violations.len();
    if total_issues > 0 {
        let _ = writeln!(stdout, "\n{} violation(s) found.", total_issues);
        return 1;
    }

    0
}

fn run_assert_check(
    base_dir: &Path,
    cli: &Cli,
    stdout: &mut impl Write,
) -> Vec<Violation> {
    let mut all_violations: Vec<Violation> = Vec::new();

    for dir in &cli.assert_paths {
        let resolved = resolve_input_path(base_dir, dir);
        for entry in WalkDir::new(&resolved).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            all_violations.extend(check_asserts(path, &source, cli.assert_threshold));
        }
    }

    if all_violations.is_empty() {
        return all_violations;
    }

    all_violations.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for v in &all_violations {
        let display_path = v.file.strip_prefix(&cwd).unwrap_or(&v.file).display();
        let _ = writeln!(
            stdout,
            "{}:{}: {} assert_eq! calls on fields of `{}` \u{2014} consider constructing an expected struct [field-by-field-assert]",
            display_path, v.line, v.count, v.receiver
        );
    }

    all_violations
}

#[derive(Default)]
struct Totals {
    diagnostics: usize,
    files_fixed: usize,
}

fn load_config(base_dir: &Path, cli: &Cli) -> Config {
    match (&cli.config, &cli.path) {
        (Some(cfg), _) => Config::from_file(&resolve_input_path(base_dir, cfg)),
        (None, Some(path)) => Config::find_from(&resolve_input_path(base_dir, path)),
        (None, None) => Config::default(),
    }
}

fn rust_files(base_dir: &Path, path: &Path) -> Vec<PathBuf> {
    let root = resolve_input_path(base_dir, path);
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .map(|entry| std::fs::canonicalize(entry.path()).unwrap_or_else(|_| entry.into_path()))
        .collect();
    files.sort();
    files
}

fn resolve_input_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn process_file(
    file_path: &Path,
    cli: &Cli,
    classifier: &Classifier,
    config: &Config,
    totals: &mut Totals,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) {
    let content = match std::fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(error) => {
            let _ = writeln!(
                stderr,
                "warning: could not read {}: {error}",
                file_path.display()
            );
            return;
        }
    };

    let block = parse(&content);
    if should_skip_reexport_shell(file_path, &content, &block) {
        return;
    }

    if cli.fix {
        let fixed = fix_file(file_path, &content, &block, classifier, config);
        if fixed == content {
            return;
        }

        if let Err(error) = std::fs::write(file_path, &fixed) {
            let _ = writeln!(
                stderr,
                "error: could not write {}: {error}",
                file_path.display()
            );
            return;
        }

        totals.files_fixed += 1;
        let _ = writeln!(stdout, "fixed: {}", file_path.display());
        return;
    }

    let diagnostics = check(file_path, &block, classifier, config);
    if diagnostics.is_empty() {
        return;
    }

    for diagnostic in &diagnostics {
        let _ = writeln!(
            stdout,
            "{}:{}: {} [{}]",
            file_path.display(),
            diagnostic.line,
            diagnostic.message,
            diagnostic.kind
        );
    }
    totals.diagnostics += diagnostics.len();
}

fn should_skip_reexport_shell(
    file_path: &Path,
    content: &str,
    block: &crate::parser::ImportBlock,
) -> bool {
    if file_path.file_name().and_then(|name| name.to_str()) != Some("mod.rs") {
        return false;
    }

    let uses = block.use_statements();
    if uses.is_empty()
        || !uses
            .iter()
            .all(|stmt| stmt.attrs.is_empty() && stmt.text.trim_start().starts_with("pub use "))
    {
        return false;
    }

    let lines: Vec<&str> = content.lines().collect();
    let prelude_ok = lines
        .iter()
        .take(block.start_line.saturating_sub(1))
        .all(|line| {
            let trimmed = line.trim();
            trimmed.is_empty() || is_module_declaration(trimmed)
        });
    let trailing_ok = lines
        .iter()
        .skip(block.end_line.saturating_sub(1))
        .all(|line| line.trim().is_empty());

    prelude_ok && trailing_ok
}

fn is_module_declaration(trimmed: &str) -> bool {
    let Some(stmt) = trimmed.strip_suffix(';') else {
        return false;
    };

    let parts: Vec<&str> = stmt.split_whitespace().collect();
    match parts.as_slice() {
        ["mod", _] => true,
        [visibility, "mod", _] => visibility.starts_with("pub"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn cli(path: PathBuf, fix: bool, config: Option<PathBuf>) -> Cli {
        Cli {
            path: Some(path),
            fix,
            config,
            assert_paths: Vec::new(),
            assert_threshold: 4,
        }
    }

    #[test]
    fn run_check_mode_returns_zero_for_clean_tree() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("clean.rs"),
            "\
// standard crates
use std::sync::Arc;

fn main() {
    let _ = Arc::new(1);
}
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), false, None),
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit_code, 0);
        assert!(
            stdout.is_empty(),
            "unexpected stdout: {:?}",
            String::from_utf8_lossy(&stdout)
        );
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_check_mode_reports_violations() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("violations.rs"),
            "use crate::app::state::AppState;\n",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), false, None),
            &mut stdout,
            &mut stderr,
        );
        let stdout = String::from_utf8(stdout).unwrap();

        assert_eq!(exit_code, 1);
        assert!(stdout.contains("missing-header"));
        assert!(stdout.contains("violation(s) found."));
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_check_mode_reports_multi_anchor_internal_imports() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join(".lint-imports.toml");
        fs::write(&config_path, "internal_crates = []\n").unwrap();
        fs::write(
            dir.path().join("violations.rs"),
            "\
// internal crates
use crate::{concurrent_cache_tests, single_thread_cache_tests};
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), false, Some(config_path)),
            &mut stdout,
            &mut stderr,
        );
        let stdout = String::from_utf8(stdout).unwrap();

        assert_eq!(exit_code, 1);
        assert!(stdout.contains("multi-anchor-internal-import"));
        assert!(stdout.contains("violation(s) found."));
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_fix_mode_rewrites_file_and_reports_count() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join(".lint-imports.toml");
        fs::write(&config_path, "internal_crates = []\n").unwrap();
        let file_path = dir.path().join("fix.rs");
        fs::write(
            &file_path,
            "\
use crate::filesys::dir::Dir;
use crate::filesys::Overwrite;
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), true, Some(config_path)),
            &mut stdout,
            &mut stderr,
        );
        let stdout = String::from_utf8(stdout).unwrap();
        let fixed = fs::read_to_string(&file_path).unwrap();

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("fixed:"));
        assert!(stdout.contains("1 file(s) fixed."));
        assert!(fixed.contains("use crate::filesys::{Overwrite, dir::Dir};"));
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_fix_mode_rewrites_nested_relative_path() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join(".lint-imports.toml");
        let nested_dir = dir.path().join("src/http");
        let file_path = nested_dir.join("deployments.rs");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(&config_path, "internal_crates = []\n").unwrap();
        fs::write(
            &file_path,
            "\
// internal crates
use super::errors::HTTPErr;

fn main() {}
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run_from_dir(
            &nested_dir,
            &cli(PathBuf::from("."), true, Some(config_path.clone())),
            &mut stdout,
            &mut stderr,
        );
        let stdout = String::from_utf8(stdout).unwrap();
        let fixed = fs::read_to_string(&file_path).unwrap();

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("fixed:"));
        assert!(fixed.contains("use crate::http::errors::HTTPErr;"));
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_fix_mode_rewrites_nested_relative_mod_file() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join(".lint-imports.toml");
        let nested_dir = dir.path().join("src/storage");
        let file_path = nested_dir.join("mod.rs");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(&config_path, "internal_crates = []\n").unwrap();
        fs::write(
            &file_path,
            "\
// internal crates
use self::errors::StorageErr;
use self::layout::Layout;

fn main() {}
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run_from_dir(
            &nested_dir,
            &cli(PathBuf::from("."), true, Some(config_path.clone())),
            &mut stdout,
            &mut stderr,
        );
        let stdout = String::from_utf8(stdout).unwrap();
        let fixed = fs::read_to_string(&file_path).unwrap();

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("fixed:"));
        assert!(fixed.contains("use crate::storage::{errors::StorageErr, layout::Layout};"));
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_check_mode_ignores_reexport_shell_mod_rs() {
        let dir = tempdir().unwrap();
        let module_dir = dir.path().join("src/services/deployment");
        let file_path = module_dir.join("mod.rs");
        fs::create_dir_all(&module_dir).unwrap();
        fs::write(
            &file_path,
            "\
mod get;

pub use get::*;
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), false, None),
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit_code, 0);
        assert!(
            stdout.is_empty(),
            "unexpected stdout: {:?}",
            String::from_utf8_lossy(&stdout)
        );
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_fix_mode_preserves_reexport_shell_mod_rs() {
        let dir = tempdir().unwrap();
        let module_dir = dir.path().join("src/services/deployment");
        let file_path = module_dir.join("mod.rs");
        fs::create_dir_all(&module_dir).unwrap();
        fs::write(
            &file_path,
            "\
mod get;

pub use get::*;
",
        )
        .unwrap();

        let original = fs::read_to_string(&file_path).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), true, None),
            &mut stdout,
            &mut stderr,
        );
        let fixed = fs::read_to_string(&file_path).unwrap();
        let stdout = String::from_utf8(stdout).unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(fixed, original);
        assert!(stdout.contains("0 file(s) fixed."));
        assert!(
            stderr.is_empty(),
            "unexpected stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn run_warns_and_continues_when_rs_path_is_unreadable() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("broken.rs")).unwrap();
        fs::write(
            dir.path().join("good.rs"),
            "\
// standard crates
use std::fmt::Debug;
",
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = run(
            &cli(dir.path().to_path_buf(), false, None),
            &mut stdout,
            &mut stderr,
        );
        let stderr = String::from_utf8(stderr).unwrap();

        assert_eq!(exit_code, 0);
        assert!(stderr.contains("warning: could not read"));
    }
}
