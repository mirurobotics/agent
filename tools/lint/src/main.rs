mod app;
mod asserts;
mod checker;
mod classifier;
mod config;
mod fixer;
mod normalizer;
mod parser;

// standard crates
use std::process;

// internal crates
use crate::app::{run, Cli};

// external crates
use clap::Parser as ClapParser;

fn main() {
    let cli = Cli::parse();
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let exit_code = run(&cli, &mut stdout, &mut stderr);
    if exit_code != 0 {
        process::exit(exit_code);
    }
}
