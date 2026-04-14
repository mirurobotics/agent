mod detect;
mod extract;

// standard crates
use std::path::PathBuf;
use std::process;

// internal crates
use crate::detect::{check_file, Violation};

// external crates
use clap::Parser;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "lint-asserts")]
#[command(about = "Detect field-by-field assert_eq! patterns in test functions")]
struct Cli {
    /// One or more directories to scan for .rs files
    #[arg(long = "path", required = true, num_args = 1..)]
    paths: Vec<PathBuf>,

    /// Minimum number of field-asserts on the same receiver to flag
    #[arg(long, default_value_t = 4)]
    threshold: usize,
}

fn main() {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let mut all_violations: Vec<Violation> = Vec::new();

    for dir in &cli.paths {
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }

            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let violations = check_file(path, &source, cli.threshold);
            all_violations.extend(violations);
        }
    }

    if all_violations.is_empty() {
        return;
    }

    // Sort by file path then line number for deterministic output.
    all_violations.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    for v in &all_violations {
        let display_path = v
            .file
            .strip_prefix(&cwd)
            .unwrap_or(&v.file)
            .display();
        println!(
            "{}:{}: {} assert_eq! calls on fields of `{}` \u{2014} consider constructing an expected struct [field-by-field-assert]",
            display_path, v.line, v.count, v.receiver
        );
    }

    println!();
    println!("{} violation(s) found.", all_violations.len());

    process::exit(1);
}
