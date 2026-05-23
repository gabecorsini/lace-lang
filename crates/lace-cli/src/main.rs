use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lace_effects::{check_program as check_effects, IssueLevel};
use lace_interp::{run as run_program, RuntimeError};
use lace_parser::parse_program;
use lace_types::check_program as check_types;

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
            match run_program(&program) {
                Ok(value) => {
                    println!("run ok: {value:?}");
                }
                Err(err) => {
                    report_runtime_error(&err);
                    anyhow::bail!("runtime execution failed");
                }
            }
        }
        Commands::Check { file } => {
            let program = load_and_validate(&file)?;
            println!(
                "check ok: parsed and validated {} top-level item(s).",
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

    let type_errors = check_types(&program);
    if !type_errors.is_empty() {
        for err in &type_errors {
            eprintln!("type error: {err}");
        }
        anyhow::bail!("failed with {} type error(s)", type_errors.len());
    }

    let effect_issues = check_effects(&program);
    let effect_errors = effect_issues
        .iter()
        .filter(|i| matches!(i.level, IssueLevel::Error))
        .count();
    for issue in &effect_issues {
        match issue.level {
            IssueLevel::Error => eprintln!("effect error in {}: {}", issue.function, issue.message),
            IssueLevel::Warning => eprintln!(
                "effect warning in {}: {}",
                issue.function, issue.message
            ),
        }
    }
    if effect_errors > 0 {
        anyhow::bail!("failed with {} effect error(s)", effect_errors);
    }

    Ok(program)
}

fn report_runtime_error(err: &RuntimeError) {
    match err.span {
        Some(span) => eprintln!("runtime error at {}..{}: {}", span.start, span.end, err.message),
        None => eprintln!("runtime error: {}", err.message),
    }
}
