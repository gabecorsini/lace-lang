use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use lace_ast::{EffectExpr, EffectTag, TopLevelItem};
use lace_effects::{check_program as check_effects, EffectIssue, IssueLevel};
use lace_interp::{run_function_with_options, run_with_options, RunOptions, RuntimeError, Value};
use lace_parser::{parse_program, ParseError};
use lace_types::{check_program as check_types, TypeError};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

#[derive(Debug, Parser)]
#[command(name = "lace", about = "Lace language CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a .lace program (parse + type + effect checks, then execute)
    Run {
        file: PathBuf,
        #[arg(long)]
        checkpoint: Option<PathBuf>,
    },
    /// Run @test functions from a .lace file or directory
    Test {
        path: PathBuf,
        #[arg(long)]
        checkpoint: Option<PathBuf>,
    },
    /// Replay a program from a previous checkpoint
    Replay {
        checkpoint: PathBuf,
        file: Option<PathBuf>,
    },
    /// Parse + typecheck + effect-check without executing
    Check { file: PathBuf },
    /// Interactive REPL
    Repl {
        #[arg(long)]
        checkpoint: Option<PathBuf>,
        #[arg(long)]
        replay: Option<PathBuf>,
    },
    /// Version and build information
    Version,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{} {}", "error:".red().bold(), format!("{err:#}").red());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { file, checkpoint } => {
            let source = load_source(&file)?;
            let (program, effect_issues) = validate_source(&source)?;

            print_effect_summary(&program, &effect_issues);

            let options = RunOptions {
                checkpoint_path: checkpoint.map(|p| p.display().to_string()),
                replay_mode: false,
                source_path: Some(file.display().to_string()),
            };

            match run_with_options(&program, options) {
                Ok(value) => {
                    println!(
                        "{} {}",
                        "run ok:".green().bold(),
                        render_value(&value).bright_white()
                    );
                }
                Err(err) => {
                    report_runtime_error(&source, &err);
                    anyhow::bail!("runtime execution failed");
                }
            }
        }
        Commands::Test { path, checkpoint } => {
            let started = Instant::now();
            let files = collect_test_files(&path)?;
            if files.is_empty() {
                anyhow::bail!("no .lace files found at {}", path.display());
            }

            let mut all_tests: Vec<(PathBuf, String)> = Vec::new();

            for file in &files {
                let source = load_source(file)?;
                let (program, _effect_issues) = validate_source(&source)?;

                let tests = collect_tests(&program);
                for test in tests {
                    all_tests.push((file.clone(), test.name.clone()));
                }
            }

            println!("running {} tests", all_tests.len());

            let mut passed = 0usize;
            let mut failed = 0usize;
            let mut failures: Vec<(String, String)> = Vec::new();

            for (file, test_name) in all_tests {
                let source = load_source(&file)?;
                let (program, _issues) = validate_source(&source)?;
                let options = RunOptions {
                    checkpoint_path: checkpoint.clone().map(|p| p.display().to_string()),
                    replay_mode: false,
                    source_path: Some(file.display().to_string()),
                };

                match run_function_with_options(&program, &test_name, options) {
                    Ok(_) => {
                        passed += 1;
                        println!("test {} ... {}", test_name, "ok".green().bold());
                    }
                    Err(err) => {
                        failed += 1;
                        println!("test {} ... {}", test_name, "FAILED".red().bold());
                        failures.push((test_name, format_test_failure_message(&source, &err)));
                    }
                }
            }

            if !failures.is_empty() {
                println!();
                println!("{}", "failures:".red().bold());
                for (name, message) in &failures {
                    println!("  {}: {}", name, message);
                }
            }

            println!();
            if failed == 0 {
                println!(
                    "test result: {}. {} passed; {} failed; finished in {:.2}s",
                    "ok".green().bold(),
                    passed,
                    failed,
                    started.elapsed().as_secs_f64()
                );
            } else {
                println!(
                    "test result: {}. {} passed; {} failed; finished in {:.2}s",
                    "FAILED".red().bold(),
                    passed,
                    failed,
                    started.elapsed().as_secs_f64()
                );
                std::process::exit(1);
            }
        }
        Commands::Replay { checkpoint, file } => {
            let file = file.unwrap_or_else(|| checkpoint.with_extension("lace"));
            let source = load_source(&file)?;
            let (program, effect_issues) = validate_source(&source)?;

            print_effect_summary(&program, &effect_issues);

            let options = RunOptions {
                checkpoint_path: Some(checkpoint.display().to_string()),
                replay_mode: true,
                source_path: Some(file.display().to_string()),
            };
            match run_with_options(&program, options) {
                Ok(value) => {
                    println!(
                        "{} {}",
                        "replay ok:".green().bold(),
                        render_value(&value).bright_white()
                    );
                }
                Err(err) => {
                    report_runtime_error(&source, &err);
                    anyhow::bail!("replay execution failed");
                }
            }
        }
        Commands::Check { file } => {
            let source = load_source(&file)?;
            let (program, effect_issues) = validate_source(&source)?;

            print_effect_summary(&program, &effect_issues);
            println!(
                "{} parsed and validated {} top-level item(s).",
                "check ok:".green().bold(),
                program.items.len()
            );
        }
        Commands::Repl { checkpoint, replay } => {
            run_repl(checkpoint, replay)?;
        }
        Commands::Version => {
            print_version();
        }
    }

    Ok(())
}

fn print_version() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    println!("{} {}", "lace".bold(), env!("CARGO_PKG_VERSION"));
    println!("build profile: {profile}");
    println!(
        "platform: {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some(sha) = option_env!("GIT_COMMIT") {
        println!("git commit: {sha}");
    }
}

fn load_source(path: &PathBuf) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("failed to read source file {}", path.display()))
}

fn collect_test_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        if path.extension().and_then(|s| s.to_str()) == Some("lace") {
            return Ok(vec![path.to_path_buf()]);
        }
        anyhow::bail!("test path is not a .lace file: {}", path.display());
    }

    if !path.is_dir() {
        anyhow::bail!("test path does not exist: {}", path.display());
    }

    let mut out = Vec::new();
    collect_test_files_recursive(path, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_test_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_test_files_recursive(&path, out)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("lace") {
            out.push(path);
        }
    }

    Ok(())
}

fn collect_tests(program: &lace_ast::Program) -> Vec<lace_ast::FnDecl> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            TopLevelItem::Function(f)
                if f.annotations.iter().any(|a| a.name == "test") =>
            {
                Some(f.clone())
            }
            _ => None,
        })
        .collect()
}

fn format_test_failure_message(source: &str, err: &RuntimeError) -> String {
    if let Some(span) = err.span {
        format!("{} [{}]", err.message, render_span_excerpt(source, span.start, span.end))
    } else {
        err.message.clone()
    }
}

fn validate_source(source: &str) -> Result<(lace_ast::Program, Vec<EffectIssue>)> {
    let (program, parse_errors) = parse_program(source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            report_parse_error(source, err);
        }
        anyhow::bail!("failed with {} parse error(s)", parse_errors.len());
    }

    let program = program.context("parser returned no program")?;

    let type_errors = check_types(&program);
    if !type_errors.is_empty() {
        for err in &type_errors {
            report_type_error(source, err);
        }
        anyhow::bail!("failed with {} type error(s)", type_errors.len());
    }

    let effect_issues = check_effects(&program);
    report_effect_issues(&effect_issues);

    let effect_errors = effect_issues
        .iter()
        .filter(|i| matches!(i.level, IssueLevel::Error))
        .count();
    if effect_errors > 0 {
        anyhow::bail!("failed with {} effect error(s)", effect_errors);
    }

    Ok((program, effect_issues))
}

fn print_effect_summary(program: &lace_ast::Program, effect_issues: &[EffectIssue]) {
    let main_effects = program
        .items
        .iter()
        .find_map(|item| match item {
            lace_ast::TopLevelItem::Function(f) if f.name == "main" => Some(
                f.effects
                    .iter()
                    .map(effect_expr_name)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            _ => None,
        })
        .unwrap_or_else(|| "(none declared)".to_string());

    let warnings = effect_issues
        .iter()
        .filter(|i| matches!(i.level, IssueLevel::Warning))
        .count();

    println!(
        "{} [{}]{}",
        "effects:".cyan().bold(),
        main_effects,
        if warnings > 0 {
            format!(
                "  ({} warning{})",
                warnings,
                if warnings == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        }
    );
}

fn effect_expr_name(expr: &EffectExpr) -> String {
    match expr {
        EffectExpr::Builtin(tag) => match tag {
            EffectTag::Pure => "Pure".to_string(),
            EffectTag::Io => "IO".to_string(),
            EffectTag::Mut => "Mut".to_string(),
            EffectTag::ToolCall => "ToolCall".to_string(),
            EffectTag::Time => "Time".to_string(),
            EffectTag::Rand => "Rand".to_string(),
        },
        EffectExpr::Variable(name) => name.clone(),
    }
}

fn report_effect_issues(effect_issues: &[EffectIssue]) {
    for issue in effect_issues {
        match issue.level {
            IssueLevel::Error => eprintln!(
                "{} in {}: {}",
                "effect error".red().bold(),
                issue.function.bold(),
                issue.message
            ),
            IssueLevel::Warning => eprintln!(
                "{} in {}: {}",
                "effect warning".yellow().bold(),
                issue.function.bold(),
                issue.message
            ),
        }
    }
}

fn run_repl(checkpoint: Option<PathBuf>, replay: Option<PathBuf>) -> Result<()> {
    let mut rl = DefaultEditor::new().context("failed to initialize rustyline")?;
    let history_path = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".lace_repl_history");
    let _ = rl.load_history(&history_path);

    println!(
        "{} {}",
        "Lace REPL".bold().bright_cyan(),
        format!("v{}", env!("CARGO_PKG_VERSION")).bright_white()
    );
    println!(
        "{}",
        "Effect system: enabled (parse + type + effect checks on every input)".cyan()
    );
    println!(
        "{}",
        "Commands: :quit, :q, :reset, :checkpoint <path>, :replay <path>; end line with \\ for multiline"
            .dimmed()
    );

    let mut session_lines: Vec<String> = Vec::new();
    let mut default_options = RunOptions {
        checkpoint_path: checkpoint.map(|p| p.display().to_string()),
        replay_mode: false,
        source_path: None,
    };
    if let Some(replay_path) = replay {
        default_options.checkpoint_path = Some(replay_path.display().to_string());
        default_options.replay_mode = true;
    }

    loop {
        let mut line = match rl.readline("lace> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("repl input failed: {e}")),
        };

        while line.trim_end().ends_with('\\') {
            line.pop();
            let cont = match rl.readline(".... ") {
                Ok(s) => s,
                Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
                Err(e) => return Err(anyhow::anyhow!("repl multiline input failed: {e}")),
            };
            line.push('\n');
            line.push_str(&cont);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(trimmed);

        if trimmed == ":quit" || trimmed == ":q" {
            break;
        }
        if trimmed == ":reset" {
            session_lines.clear();
            println!("{}", "session reset".green());
            continue;
        }

        if let Some(path) = trimmed.strip_prefix(":checkpoint ") {
            let path = path.trim();
            if path.is_empty() {
                println!("usage: :checkpoint <path>");
                continue;
            }
            default_options.checkpoint_path = Some(path.to_string());
            default_options.replay_mode = false;
            println!("{} {}", "checkpoint path set:".green(), path);
            continue;
        }

        if let Some(path) = trimmed.strip_prefix(":replay ") {
            let path = path.trim();
            if path.is_empty() {
                println!("usage: :replay <checkpoint-or-journal-path>");
                continue;
            }
            default_options.checkpoint_path = Some(path.to_string());
            default_options.replay_mode = true;
            println!("{} {}", "replay mode enabled:".green(), path);
            continue;
        }

        let mut trial_lines = session_lines.clone();
        trial_lines.push(line.clone());
        let source = make_repl_program(&trial_lines);

        match validate_source(&source) {
            Ok((program, _issues)) => match run_with_options(&program, default_options.clone()) {
                Ok(value) => {
                    session_lines = trial_lines;
                    println!(
                        "{} {} {}",
                        "=>".green().bold(),
                        render_value(&value).bright_white(),
                        format!(": {}", value_type_name(&value)).dimmed()
                    );
                }
                Err(err) => report_runtime_error(&source, &err),
            },
            Err(err) => {
                eprintln!("{} {err:#}", "repl error:".red().bold());
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

fn make_repl_program(lines: &[String]) -> String {
    let mut src = String::from("fn main() -> Dynamic [IO, ToolCall, Time, Rand] {\n");
    for l in lines {
        src.push_str(l);
        src.push('\n');
    }
    src.push_str("}\n");
    src
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Unit => "Unit",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Bool(_) => "Bool",
        Value::String(_) => "String",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record { .. } => "Record",
        Value::Variant { .. } => "Variant",
    }
}

fn render_value(v: &Value) -> String {
    format!("{v:?}")
}

fn report_parse_error(source: &str, err: &ParseError) {
    let (message, start, end) = match err {
        ParseError::Message {
            message,
            span_start,
            span_end,
        } => (message.as_str(), *span_start, *span_end),
    };

    eprintln!("{}: {}", "parse error".red().bold(), message);
    eprintln!("{}", render_span_excerpt(source, start, end).dimmed());
}

fn report_type_error(source: &str, err: &TypeError) {
    eprintln!("{}: {err}", "type error".red().bold());

    if let Some((start, end)) = type_error_span(err) {
        eprintln!("{}", render_span_excerpt(source, start, end).dimmed());
    }
}

fn type_error_span(err: &TypeError) -> Option<(usize, usize)> {
    match err {
        TypeError::UnknownIdentifier {
            span_start,
            span_end,
            ..
        }
        | TypeError::Mismatch {
            span_start,
            span_end,
            ..
        }
        | TypeError::UnknownFunction {
            span_start,
            span_end,
            ..
        }
        | TypeError::InvalidPattern {
            span_start,
            span_end,
            ..
        } => Some((*span_start, *span_end)),
        TypeError::UnknownRecordType { .. } | TypeError::InvalidToolDecl { .. } => None,
    }
}

fn report_runtime_error(source: &str, err: &RuntimeError) {
    eprintln!("{}: {}", "runtime error".red().bold(), err.message);
    if let Some(span) = err.span {
        eprintln!(
            "{}",
            render_span_excerpt(source, span.start, span.end).dimmed()
        );
    }
}

fn render_span_excerpt(source: &str, span_start: usize, span_end: usize) -> String {
    let starts = line_starts(source);
    let (line, col) = offset_to_line_col(span_start, source, &starts);
    let line_text = source_line(source, line).unwrap_or("");

    let safe_start = span_start.min(source.len());
    let safe_end = span_end.min(source.len()).max(safe_start + 1);
    let caret_width = safe_end
        .saturating_sub(safe_start)
        .min(line_text.len().saturating_sub(col.saturating_sub(1)));
    let caret_width = caret_width.max(1);

    let gutter = format!("{:>4} | ", line);
    let mut caret = String::new();
    caret.push_str(&" ".repeat(gutter.len()));
    caret.push_str(&" ".repeat(col.saturating_sub(1)));
    caret.push_str(&"^".repeat(caret_width));

    format!("--> line {line}, col {col}\n{gutter}{line_text}\n{caret}")
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    for (idx, ch) in source.char_indices() {
        if ch == '\n' {
            out.push(idx + 1);
        }
    }
    out
}

fn offset_to_line_col(offset: usize, source: &str, starts: &[usize]) -> (usize, usize) {
    let offset = offset.min(source.len());
    let idx = starts.partition_point(|&s| s <= offset).saturating_sub(1);
    let line_start = starts[idx];
    let line = idx + 1;
    let col = source[line_start..offset].chars().count() + 1;
    (line, col)
}

fn source_line(source: &str, one_based_line: usize) -> Option<&str> {
    source.lines().nth(one_based_line.saturating_sub(1))
}
