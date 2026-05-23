use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lace_parser::parse_program;
use lace_types::check_program;

#[derive(Debug, Parser)]
#[command(name = "lace", about = "Lace language CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run { file: PathBuf },
    Check { file: PathBuf },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { file } => {
            let program = load_and_validate(&file)?;
            println!(
                "run ok: parsed and type-checked {} top-level item(s). runtime execution is not implemented yet.",
                program.items.len()
            );
        }
        Commands::Check { file } => {
            let program = load_and_validate(&file)?;
            println!(
                "check ok: parsed and type-checked {} top-level item(s).",
                program.items.len()
            );
        }
    }

    Ok(())
}

fn load_and_validate(path: &PathBuf) -> Result<lace_ast::Program> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read source file {}", path.display()))?;

    let (program, parse_errors) = parse_program(&source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            eprintln!("parse error: {err}");
        }
        anyhow::bail!("failed with {} parse error(s)", parse_errors.len());
    }

    let program = program.context("parser returned no program")?;
    let type_errors = check_program(&program);
    if !type_errors.is_empty() {
        for err in &type_errors {
            eprintln!("type error: {err}");
        }
        anyhow::bail!("failed with {} type error(s)", type_errors.len());
    }

    Ok(program)
}
