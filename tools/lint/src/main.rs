mod checker;
mod classifier;
mod config;
mod fixer;
mod parser;

use std::path::PathBuf;
use std::process;

use clap::Parser as ClapParser;
use walkdir::WalkDir;

use checker::check;
use classifier::Classifier;
use config::Config;
use fixer::fix_file;
use parser::parse;

#[derive(ClapParser)]
#[command(
    name = "lint-imports",
    about = "Lint Rust import grouping, ordering, and comment headers"
)]
struct Cli {
    /// Directory to scan for .rs files
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Auto-fix violations in place
    #[arg(long)]
    fix: bool,

    /// Path to .lint-imports.toml config file
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    let config = match &cli.config {
        Some(p) => Config::from_file(p),
        None => Config::find_from(&cli.path),
    };
    let classifier = Classifier::new(&config);

    let mut total_diagnostics = 0usize;
    let mut files_fixed = 0usize;

    let mut rs_files: Vec<PathBuf> = WalkDir::new(&cli.path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "rs"))
        .map(|e| e.into_path())
        .collect();
    rs_files.sort();

    for file_path in &rs_files {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: could not read {}: {e}", file_path.display());
                continue;
            }
        };

        let block = parse(&content);

        if cli.fix {
            let fixed = fix_file(&content, &block, &classifier, &config);
            if fixed != content {
                if let Err(e) = std::fs::write(file_path, &fixed) {
                    eprintln!("error: could not write {}: {e}", file_path.display());
                } else {
                    files_fixed += 1;
                    println!("fixed: {}", file_path.display());
                }
            }
        } else {
            let diagnostics = check(file_path, &block, &classifier, &config);
            if !diagnostics.is_empty() {
                for d in &diagnostics {
                    println!(
                        "{}:{}: {} [{}]",
                        file_path.display(),
                        d.line,
                        d.message,
                        d.kind
                    );
                }
                total_diagnostics += diagnostics.len();
            }
        }
    }

    if cli.fix {
        println!("\n{files_fixed} file(s) fixed.");
    } else if total_diagnostics > 0 {
        println!("\n{total_diagnostics} violation(s) found.");
        process::exit(1);
    }
}
