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
use lace_types::{check_program_full, TypeError, TypeWarning};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde::Deserialize;

// ─── Manifest ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LaceManifest {
    package: PackageManifest,
}

#[derive(Debug, Deserialize)]
struct PackageManifest {
    name: String,
    version: String,
    entry: Option<String>,
}

/// Walk upward from `start` looking for a lace.toml.
/// Returns (project_root, manifest) if found.
fn find_manifest(start: &Path) -> Option<(PathBuf, LaceManifest)> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("lace.toml");
        if candidate.exists() {
            let contents = fs::read_to_string(&candidate).ok()?;
            let manifest: LaceManifest = toml::from_str(&contents).ok()?;
            return Some((dir, manifest));
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "lace", about = "Lace language CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a .lace program (parse + type + effect checks, then execute)
    ///
    /// If FILE is omitted, looks for lace.toml in the current directory and
    /// uses src/main.lace as the entry-point.
    Run {
        file: Option<PathBuf>,
        #[arg(long)]
        checkpoint: Option<PathBuf>,
        /// Suppress warnings
        #[arg(long)]
        no_warnings: bool,
        /// Suppress tool effect logging (no journal file written)
        #[arg(long)]
        no_tool_log: bool,
        /// Write structured tool-call log to this file
        #[arg(long, value_name = "PATH")]
        log_file: Option<String>,
        /// Use the bytecode VM instead of the tree-walking interpreter
        #[arg(long)]
        vm: bool,
        /// Suppress decorators; print raw output only
        #[arg(short = 'q', long = "quiet")]
        quiet: bool,
    },
    /// Discover and run test functions from *_test.lace / *.test.lace files
    ///
    /// Test functions are regular fns prefixed with `test_` (or annotated with @test).
    /// Without a PATH argument, discovers test files in the current directory.
    Test {
        /// A specific .lace test file or directory to search (default: current dir)
        path: Option<PathBuf>,
        #[arg(long)]
        checkpoint: Option<PathBuf>,
        /// Only run tests whose name contains this substring
        #[arg(long)]
        filter: Option<String>,
    },
    /// Replay a program from a previous checkpoint
    Replay {
        checkpoint: PathBuf,
        file: Option<PathBuf>,
    },
    /// Parse + typecheck + effect-check without executing
    ///
    /// If FILE is omitted, looks for lace.toml in the current directory and
    /// uses src/main.lace as the entry-point.
    Check {
        file: Option<PathBuf>,
        /// Suppress warnings
        #[arg(long)]
        no_warnings: bool,
    },
    /// Compile/check a Lace project (type check + effect check, project-aware)
    Build {
        /// Suppress warnings
        #[arg(long)]
        no_warnings: bool,
    },
    /// Create a new Lace project scaffold
    New {
        /// Name of the new project
        name: String,
    },
    /// Interactive REPL
    Repl {
        #[arg(long)]
        checkpoint: Option<PathBuf>,
        #[arg(long)]
        replay: Option<PathBuf>,
    },
    /// Explain a Lace error code (e.g. E001, E002)
    Explain {
        /// Error code to explain (e.g. E001)
        code: String,
    },
    /// Format a .lace source file in-place (or to stdout)
    Fmt {
        file: PathBuf,
        /// Write formatted output to stdout instead of in-place
        #[arg(long)]
        stdout: bool,
        /// Check if file would change (exit 1 if so); useful for CI
        #[arg(long)]
        check: bool,
    },
    /// Generate HTML documentation from .lace source files
    Doc {
        /// Path to a directory containing .lace files (defaults to current directory)
        path: Option<PathBuf>,
    },
    /// Version and build information
    Version,
    /// Start the Lace Language Server (LSP) over stdio
    ///
    /// Configure your editor to invoke `lace lsp` as the language server command.
    Lsp,
    /// Compile a .lace file to bytecode (.lacec)
    Compile {
        /// Source file to compile
        file: String,
    },
    /// Run a .lace or .lacec file using the bytecode VM
    Vm {
        /// Source (.lace) or bytecode (.lacec) file to run
        file: String,
    },
    /// Bundle a .lace program into a self-contained executable
    Bundle {
        /// Source file to bundle
        file: String,
        /// Output path (defaults to <stem>.bin)
        #[arg(long)]
        output: Option<String>,
    },
    /// Watch a .lace file and re-run it on every save (Ctrl+C to stop)
    Watch {
        /// The .lace file to watch and run
        file: PathBuf,
        /// Suppress warnings
        #[arg(long)]
        no_warnings: bool,
    },
}

fn main() {
    // Check if we are a bundled binary
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(bytes) = std::fs::read(&exe) {
            if let Some(bytecode) = extract_bundle(&bytes) {
                let tool_log = !std::env::args().any(|a| a == "--no-tool-log");
                match lace_vm::run_bytes(&bytecode, tool_log) {
                    Ok(_) => std::process::exit(0),
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
    }
    if let Err(err) = run() {
        eprintln!("{} {}", "error:".red().bold(), format!("{err:#}").red());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { file, checkpoint, no_warnings, no_tool_log, log_file, vm, quiet } => {
            let (file, _manifest) = resolve_entrypoint(file, "run")?;
            let source = load_source(&file)?;

            if vm {
                if let Err(e) = lace_vm::run_source(&source, true) {
                    eprintln!("{} {}", "vm error:".red().bold(), format!("{e}").red());
                    std::process::exit(1);
                }
            } else {
                let (program, effect_issues) = validate_source_with_warnings(&source, no_warnings)?;

                if !quiet {
                    print_effect_summary(&program, &effect_issues);
                }

                let options = RunOptions {
                    checkpoint_path: checkpoint.map(|p| p.display().to_string()),
                    replay_mode: false,
                    source_path: Some(file.display().to_string()),
                    suppress_tool_log: no_tool_log,
                    log_file,
                };

                match run_with_options(&program, options) {
                    Ok(value) => {
                        if quiet {
                            match &value {
                                Value::Unit => {} // suppress Unit output in quiet mode
                                Value::String(s) => println!("{}", s),
                                other => println!("{:?}", other),
                            }
                        } else {
                            println!(
                                "{} {}",
                                "run ok:".green().bold(),
                                render_value(&value).bright_white()
                            );
                        }
                    }
                    Err(err) => {
                        report_runtime_error_with_file(&source, &err, Some(&file.display().to_string()));
                        anyhow::bail!("runtime execution failed");
                    }
                }
            }
        }
        Commands::Test { path, checkpoint, filter } => {
            let started = Instant::now();
            let search_path = path.unwrap_or_else(|| {
                std::env::current_dir().expect("failed to get current directory")
            });
            let files = collect_test_files(&search_path)?;
            if files.is_empty() {
                anyhow::bail!("no test files found at {}", search_path.display());
            }

            let mut all_tests: Vec<(PathBuf, String)> = Vec::new();

            for file in &files {
                let source = load_source(file)?;
                let (program, _effect_issues) = validate_source(&source)?;

                let tests = collect_tests(&program);
                for test in tests {
                    if let Some(ref f) = filter {
                        if !test.name.contains(f.as_str()) {
                            continue;
                        }
                    }
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
                    suppress_tool_log: true,
                    log_file: None,
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
                suppress_tool_log: false,
                log_file: None,
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
                    report_runtime_error_with_file(&source, &err, Some(&file.display().to_string()));
                    anyhow::bail!("replay execution failed");
                }
            }
        }
        Commands::Check { file, no_warnings } => {
            let (file, manifest) = resolve_entrypoint(file, "check")?;
            let source = load_source(&file)?;
            let (program, effect_issues) = validate_source_with_warnings(&source, no_warnings)?;

            print_effect_summary(&program, &effect_issues);

            if let Some(m) = manifest {
                println!(
                    "{} {} {} — {} top-level item(s) in {}",
                    "check ok:".green().bold(),
                    m.package.name.bold(),
                    format!("v{}", m.package.version).dimmed(),
                    program.items.len(),
                    file.display()
                );
            } else {
                println!(
                    "{} parsed and validated {} top-level item(s).",
                    "check ok:".green().bold(),
                    program.items.len()
                );
            }
        }
        Commands::Build { no_warnings } => {
            run_build(no_warnings)?;
        }
        Commands::New { name } => {
            run_new(&name)?;
        }
        Commands::Repl { checkpoint, replay } => {
            run_repl(checkpoint, replay)?;
        }
        Commands::Explain { code } => {
            print_error_explanation(&code);
        }
        Commands::Fmt { file, stdout, check } => {
            run_fmt(&file, stdout, check)?;
        }
        Commands::Doc { path } => {
            run_doc(path)?;
        }
        Commands::Version => {
            print_version();
        }
        Commands::Lsp => {
            lace_lsp::run_server();
        }
        Commands::Compile { file } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {file}"))?;
            match lace_vm::compile_to_bytes(&source) {
                Ok(bytes) => {
                    let out_path = PathBuf::from(&file).with_extension("lacec");
                    fs::write(&out_path, &bytes)
                        .with_context(|| format!("failed to write {}", out_path.display()))?;
                    println!("compiled: {}", out_path.display());
                }
                Err(e) => {
                    eprintln!("{} {}", "compile error:".red().bold(), format!("{e}").red());
                    std::process::exit(1);
                }
            }
        }
        Commands::Vm { file } => {
            let result = if file.ends_with(".lacec") {
                let bytes = fs::read(&file)
                    .with_context(|| format!("failed to read {file}"))?;
                lace_vm::run_bytes(&bytes, true)
            } else {
                let source = fs::read_to_string(&file)
                    .with_context(|| format!("failed to read {file}"))?;
                lace_vm::run_source(&source, true)
            };
            if let Err(e) = result {
                eprintln!("{} {}", "vm error:".red().bold(), format!("{e}").red());
                std::process::exit(1);
            }
        }
        Commands::Bundle { file, output } => {
            run_bundle(&file, output.as_deref())?;
        }
        Commands::Watch { file, no_warnings } => {
            run_watch(&file, no_warnings)?;
        }
    }

    Ok(())
}

// ─── resolve_entrypoint ───────────────────────────────────────────────────────

/// Resolve the source file to use.
/// - If `file` is Some, use it directly (no manifest required).
/// - If `file` is None, walk up from CWD looking for lace.toml and use src/main.lace.
fn resolve_entrypoint(
    file: Option<PathBuf>,
    cmd: &str,
) -> Result<(PathBuf, Option<LaceManifest>)> {
    if let Some(f) = file {
        return Ok((f, None));
    }

    let cwd = std::env::current_dir().context("failed to get current directory")?;
    if let Some((root, manifest)) = find_manifest(&cwd) {
        let entry = if let Some(e) = manifest.package.entry.as_deref() {
            root.join(e)
        } else {
            root.join("src").join("main.lace")
        };
        if !entry.exists() {
            anyhow::bail!(
                "lace.toml found at {} but entry '{}' does not exist.\n\
                 Run `lace new {}` to scaffold a new project, or create the entry file manually.",
                root.display(),
                entry.display(),
                manifest.package.name
            );
        }
        Ok((entry, Some(manifest)))
    } else {
        anyhow::bail!(
            "no file argument given and no lace.toml found in {} or any parent directory.\n\
             Usage: lace {} <file.lace>",
            cwd.display(),
            cmd
        )
    }
}

// ─── lace build ──────────────────────────────────────────────────────────────

fn run_build(no_warnings: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let (root, manifest) = find_manifest(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no lace.toml found in {} or any parent directory.\n\
             Run `lace new <name>` to create a new project.",
            cwd.display()
        )
    })?;

    let pkg_name = manifest.package.name.clone();
    let pkg_version = manifest.package.version.clone();

    println!(
        "{} {} {}",
        "building".cyan().bold(),
        pkg_name.bold(),
        format!("v{pkg_version}").dimmed()
    );

    // Collect all .lace files under src/
    let src_dir = root.join("src");
    if !src_dir.exists() {
        anyhow::bail!(
            "src/ directory not found in project at {}",
            root.display()
        );
    }

    let mut lace_files: Vec<PathBuf> = Vec::new();
    collect_lace_files_recursive(&src_dir, &mut lace_files)?;
    lace_files.sort();

    let started = Instant::now();
    let mut total_items = 0usize;
    let mut had_errors = false;

    for file in &lace_files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file.as_path())
            .display()
            .to_string();

        let source = load_source(file)?;
        match validate_source_with_warnings(&source, no_warnings) {
            Ok((program, effect_issues)) => {
                let warnings = effect_issues
                    .iter()
                    .filter(|i| matches!(i.level, IssueLevel::Warning))
                    .count();
                total_items += program.items.len();
                if warnings > 0 {
                    println!(
                        "  {} {} ({} warning{})",
                        "ok".green().bold(),
                        rel,
                        warnings,
                        if warnings == 1 { "" } else { "s" }
                    );
                } else {
                    println!("  {} {}", "ok".green().bold(), rel);
                }
            }
            Err(err) => {
                println!("  {} {}: {err:#}", "error".red().bold(), rel);
                had_errors = true;
            }
        }
    }

    println!();
    let elapsed = started.elapsed().as_secs_f64();

    if had_errors {
        println!(
            "{} {} {} — build failed in {:.2}s",
            "build:".red().bold(),
            pkg_name.bold(),
            format!("v{pkg_version}").dimmed(),
            elapsed
        );
        std::process::exit(1);
    } else {
        println!(
            "{} {} {} — {} file(s), {} item(s) in {:.2}s",
            "build ok:".green().bold(),
            pkg_name.bold(),
            format!("v{pkg_version}").dimmed(),
            lace_files.len(),
            total_items,
            elapsed
        );
    }

    Ok(())
}

fn collect_lace_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_lace_files_recursive(&path, out)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("lace") {
            out.push(path);
        }
    }
    Ok(())
}

// ─── lace new ────────────────────────────────────────────────────────────────

fn run_new(name: &str) -> Result<()> {
    let project_dir = PathBuf::from(name);
    if project_dir.exists() {
        anyhow::bail!("directory '{}' already exists", name);
    }

    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("failed to create {}", src_dir.display()))?;

    // lace.toml
    let manifest_content = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
    );
    fs::write(project_dir.join("lace.toml"), &manifest_content)
        .context("failed to write lace.toml")?;

    // src/main.lace
    let main_content = "fn main() [IO] {\n    println(\"Hello from {name}!\")\n}\n"
        .replace("{name}", name);
    fs::write(src_dir.join("main.lace"), &main_content)
        .context("failed to write src/main.lace")?;

    // src/ is already created above — no extra work needed

    println!("{} {}", "created".green().bold(), name.bold());
    println!("  {}", format!("{name}/lace.toml").dimmed());
    println!("  {}", format!("{name}/src/main.lace").dimmed());
    println!();
    println!("To get started:");
    println!("  cd {name}");
    println!("  lace run");

    Ok(())
}

// ─── shared helpers (unchanged from Phase 12) ────────────────────────────────

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
            // Accept *_test.lace, *.test.lace, or any file passed directly
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if stem.ends_with("_test") || stem.ends_with(".test") {
                out.push(path);
            }
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
                if f.name.starts_with("test_")
                    || f.annotations.iter().any(|a| a.name == "test") =>
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
    validate_source_with_warnings(source, false)
}

fn validate_source_with_warnings(source: &str, no_warnings: bool) -> Result<(lace_ast::Program, Vec<EffectIssue>)> {
    let (program, parse_errors) = parse_program(source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            report_parse_error(source, err);
        }
        anyhow::bail!("failed with {} parse error(s)", parse_errors.len());
    }

    let program = program.context("parser returned no program")?;

    let (type_errors, type_warnings) = check_program_full(&program);
    let shown = type_errors.len().min(20);
    for err in &type_errors[..shown] {
        report_type_error(source, err);
    }
    if type_errors.len() > 20 {
        eprintln!(
            "{} ... and {} more error(s)",
            "note:".yellow().bold(),
            type_errors.len() - 20
        );
    }
    if !type_errors.is_empty() {
        anyhow::bail!("failed with {} type error(s)", type_errors.len());
    }

    if !no_warnings {
        for w in &type_warnings {
            report_type_warning(source, w);
        }
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
        "Type :help for available commands. Use up-arrow for history. Brace-balanced multiline supported."
            .dimmed()
    );

    let mut session_lines: Vec<String> = Vec::new();
    let mut default_options = RunOptions {
        checkpoint_path: checkpoint.map(|p| p.display().to_string()),
        replay_mode: false,
        source_path: None,
        suppress_tool_log: false,
        log_file: None,
    };
    if let Some(replay_path) = replay {
        default_options.checkpoint_path = Some(replay_path.display().to_string());
        default_options.replay_mode = true;
    }

    loop {
        let line = match rl.readline("lace> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("repl input failed: {e}")),
        };

        // Brace-balanced multiline: collect until braces balance
        let mut full_input = line.clone();
        loop {
            let opens: usize = full_input.chars().filter(|&c| c == '{').count();
            let closes: usize = full_input.chars().filter(|&c| c == '}').count();
            let trimmed_check = full_input.trim_end();
            let ends_brace = trimmed_check.ends_with('{');
            if opens <= closes && !ends_brace {
                break;
            }
            let cont = match rl.readline("... ") {
                Ok(s) => s,
                Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
                Err(e) => return Err(anyhow::anyhow!("repl multiline input failed: {e}")),
            };
            full_input.push('\n');
            full_input.push_str(&cont);
        }

        // Legacy backslash continuation (still supported)
        let mut line = full_input;
        while line.trim_end().ends_with('\\') {
            line.pop();
            let cont = match rl.readline("... ") {
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
        if trimmed == ":help" {
            println!("{}", "Available REPL commands:".bold());
            println!("  :help                     — show this help");
            println!("  :quit, :q                 — exit the REPL");
            println!("  :clear, :reset            — reset interpreter state (clear session)");
            println!("  :checkpoint <path>        — set checkpoint path for tool calls");
            println!("  :replay <path>            — enable replay mode from checkpoint");
            println!();
            println!("{}", "Multiline input:".bold());
            println!("  Brace-balanced: continue typing after '{{'; braces close automatically");
            println!("  Backslash: end line with \\ to continue on next line");
            continue;
        }
        if trimmed == ":reset" || trimmed == ":clear" {
            session_lines.clear();
            println!("{}", "session cleared".green());
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
        Value::Map(_) => "Map",
        Value::Closure { .. } => "Fn",
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
    eprintln!("{} {}: {err}", "error".red().bold(), format!("[{}]", err.code()).red());

    if let Some((start, end)) = type_error_span(err) {
        eprintln!("{}", render_span_excerpt(source, start, end).dimmed());
    }
}

fn report_type_warning(source: &str, warn: &TypeWarning) {
    eprintln!("{} {}: {warn}", "warning".yellow().bold(), format!("[{}]", warn.code()).yellow());
    match warn {
        TypeWarning::UnusedVariable { span_start, span_end, .. } => {
            eprintln!("{}", render_span_excerpt(source, *span_start, *span_end).dimmed());
        }
        TypeWarning::PureFnCallsEffectful { .. } => {
            // No span available for W004
        }
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
        TypeError::NonExhaustiveMatch {
            span_start,
            span_end,
            ..
        } => Some((*span_start, *span_end)),
    }
}

fn report_runtime_error(source: &str, err: &RuntimeError) {
    report_runtime_error_with_file(source, err, None);
}

fn report_runtime_error_with_file(source: &str, err: &RuntimeError, file: Option<&str>) {
    eprintln!("{} {}", "error:".red().bold(), err.message);
    if let Some(span) = err.span {
        eprintln!(
            "{}",
            render_span_excerpt_with_file(source, span.start, span.end, file).dimmed()
        );
    }
}

fn render_span_excerpt(source: &str, span_start: usize, span_end: usize) -> String {
    render_span_excerpt_with_file(source, span_start, span_end, None)
}

fn render_span_excerpt_with_file(source: &str, span_start: usize, span_end: usize, file: Option<&str>) -> String {
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

    let location = match file {
        Some(f) => format!("  --> {}:{}:{}", f, line, col),
        None => format!("  --> {}:{}", line, col),
    };
    format!("{location}\n{gutter}{line_text}\n{caret}")
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

fn print_error_explanation(code: &str) {
    let explanation = match code.to_uppercase().as_str() {
        "E001" => Some((
            "E001 — Unknown identifier or function",
            "The referenced name does not exist in the current scope. \
             Check for typos. The compiler will suggest similar names if available.\n\n\
             Example:\n  let x = unknwon_var   // E001: did you mean 'unknown_var'?",
        )),
        "E002" => Some((
            "E002 — Type mismatch",
            "A value of one type was used where a different type was expected.\n\n\
             Example:\n  let x: Int = \"hello\"  // E002: expected Int, found String",
        )),
        "E003" => Some((
            "E003 — Wrong argument count",
            "A function was called with the wrong number of arguments.\n\n\
             Example:\n  print(\"a\", \"b\")  // E003: print takes 1 arg, got 2",
        )),
        "E004" => Some((
            "E004 — Missing or unknown record type / field",
            "A record type or field was referenced that does not exist.\n\n\
             Example:\n  let r = UnknownRecord { x: 1 }  // E004",
        )),
        "E005" => Some((
            "E005 — Invalid tool declaration",
            "A tool declaration has an invalid option (e.g. negative retries, \
             duplicate timeout, unknown mock function).\n\n\
             Example:\n  tool my_tool() -> String { retries: -1 }  // E005",
        )),
        "E006" => Some((
            "E006 — IO effect in pure function",
            "A pure `fn` cannot call IO-tagged functions (Http, File, Process, etc.) \
             or `tool` declarations. To perform IO, declare the function as `tool`.\n\n\
             Example fix:\n\
             # Before:\n\
             fn fetch(url: String) -> String {\n\
                 Http.get(url)  # ERROR: IO in pure fn\n\
             }\n\n\
             # After:\n\
             tool fetch(url: String) -> Result<String, String> {\n\
                 Http.get(url)\n\
             }",
        )),
        "W001" => Some((
            "W001 — Unused variable",
            "A variable was declared with 'let' but never referenced. \
             Prefix the name with '_' to suppress this warning.\n\n\
             Example:\n  let unused = 42   // W001: unused variable 'unused'",
        )),
        "W004" => Some((
            "W004 — Pure fn calls effectful function",
            "A plain fn calls an effectful function (Http.*, Env.set, or a tool). \
             Consider declaring the fn as a tool to make the effect explicit.\n\n\
             Example:\n  fn fetch_data() -> String {\n    Http.get(\"https://example.com\")  // W004\n  }",
        )),
        _ => None,
    };

    match explanation {
        Some((title, body)) => {
            println!("{}", title.bold());
            println!();
            println!("{body}");
        }
        None => {
            eprintln!(
                "{}: unknown error code '{}'. Known codes: E001, E002, E003, E004, E005, W001, W004",
                "error".red().bold(),
                code
            );
            std::process::exit(1);
        }
    }
}

fn run_fmt(file: &PathBuf, to_stdout: bool, check: bool) -> Result<()> {
    let source = load_source(file)?;
    let (program, parse_errors) = parse_program(&source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            report_parse_error(&source, err);
        }
        anyhow::bail!("fmt: failed with {} parse error(s)", parse_errors.len());
    }
    let program = program.context("parser returned no program")?;

    let formatted = fmt_program(&program);

    if check {
        if source != formatted {
            eprintln!("{} {} would be reformatted", "fmt check:".yellow().bold(), file.display());
            std::process::exit(1);
        } else {
            println!("{} {} is already formatted", "fmt check:".green().bold(), file.display());
        }
    } else if to_stdout {
        print!("{formatted}");
    } else {
        fs::write(file, &formatted)
            .with_context(|| format!("failed to write formatted output to {}", file.display()))?;
        println!("{} {}", "fmt ok:".green().bold(), file.display());
    }
    Ok(())
}

fn run_doc(path: Option<PathBuf>) -> Result<()> {
    // Resolve directory
    let dir = if let Some(p) = path {
        p
    } else {
        // Try to find project root via lace.toml, fall back to CWD
        let cwd = std::env::current_dir().context("failed to get cwd")?;
        if let Some((root, _)) = find_manifest(&cwd) {
            root
        } else {
            cwd
        }
    };

    // Collect .lace files
    let mut lace_files: Vec<PathBuf> = Vec::new();
    if dir.is_file() {
        lace_files.push(dir.clone());
    } else {
        collect_lace_files_recursive(&dir, &mut lace_files)?;
    }
    lace_files.sort();

    if lace_files.is_empty() {
        anyhow::bail!("no .lace files found in {}", dir.display());
    }

    // Gather documented items
    #[derive(Debug)]
    struct DocItem {
        kind: &'static str,
        name: String,
        signature: String,
        doc: String,
        source_file: String,
    }

    let mut all_items: Vec<DocItem> = Vec::new();

    for file in &lace_files {
        let source = load_source(file)?;
        let (program, parse_errors) = lace_parser::parse_program(&source);
        if !parse_errors.is_empty() {
            // Skip files with parse errors for doc generation
            continue;
        }
        let program = match program {
            Some(p) => p,
            None => continue,
        };

        let rel = file
            .strip_prefix(&dir)
            .unwrap_or(file.as_path())
            .display()
            .to_string();

        for item in &program.items {
            match item {
                TopLevelItem::Function(f) => {
                    if let Some(doc) = &f.doc_comment {
                        let sig = build_fn_signature(f);
                        all_items.push(DocItem {
                            kind: "fn",
                            name: f.name.clone(),
                            signature: sig,
                            doc: doc.clone(),
                            source_file: rel.clone(),
                        });
                    }
                }
                TopLevelItem::Record(r) => {
                    if let Some(doc) = &r.doc_comment {
                        let sig = format!(
                            "record {}{}",
                            r.name,
                            if r.generics.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    "<{}>",
                                    r.generics
                                        .iter()
                                        .map(|g| g.name.clone())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            }
                        );
                        all_items.push(DocItem {
                            kind: "record",
                            name: r.name.clone(),
                            signature: sig,
                            doc: doc.clone(),
                            source_file: rel.clone(),
                        });
                    }
                }
                TopLevelItem::Tool(t) => {
                    if let Some(doc) = &t.doc_comment {
                        let params = t
                            .params
                            .iter()
                            .map(|p| format!("{}: {}", p.name, fmt_type_expr(&p.ty)))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let sig = format!(
                            "tool {}({}) -> {}",
                            t.name,
                            params,
                            fmt_type_expr(&t.ret_ty)
                        );
                        all_items.push(DocItem {
                            kind: "tool",
                            name: t.name.clone(),
                            signature: sig,
                            doc: doc.clone(),
                            source_file: rel.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // Create docs/ output directory
    let out_dir = if dir.is_file() {
        dir.parent().unwrap_or(&dir).join("docs")
    } else {
        dir.join("docs")
    };
    fs::create_dir_all(&out_dir).context("failed to create docs/ directory")?;

    // CSS (inlined — aligned with docs/site/style.css palette)
    let css = r#"
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html { scroll-behavior: smooth; font-size: 16px; }
body { background: #0d1117; color: #e6edf3; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif; line-height: 1.7; min-height: 100vh; display: flex; flex-direction: column; }
a { color: #58a6ff; text-decoration: none; transition: color 0.15s ease; }
a:hover { color: #79b8ff; text-decoration: underline; }
/* Nav */
.topnav { position: sticky; top: 0; z-index: 100; background: #161b22; border-bottom: 1px solid #21262d; height: 56px; display: flex; align-items: center; padding: 0 1.5rem; gap: 2rem; }
.topnav h1 { font-size: 1.2rem; font-weight: 700; color: #f0f6fc; }
.topnav .subtitle { color: #8b949e; font-size: 0.88rem; }
/* Layout */
.page-wrap { display: flex; flex: 1; max-width: 1100px; margin: 0 auto; width: 100%; padding: 0 1rem; gap: 2rem; }
/* Sidebar */
aside { width: 220px; flex-shrink: 0; padding: 1.5rem 0; position: sticky; top: 56px; max-height: calc(100vh - 56px); overflow-y: auto; }
aside .search-box { width: 100%; padding: 0.4rem 0.7rem; background: #161b22; border: 1px solid #30363d; border-radius: 6px; color: #e6edf3; font-size: 0.88rem; margin-bottom: 1rem; outline: none; }
aside .search-box:focus { border-color: #58a6ff; }
aside .toc-section { margin-bottom: 1rem; }
aside .toc-label { font-size: 0.72rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.06em; color: #8b949e; margin-bottom: 0.4rem; }
aside .toc-link { display: block; padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.88rem; color: #8b949e; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
aside .toc-link:hover { background: #161b22; color: #58a6ff; text-decoration: none; }
/* Content */
.content { flex: 1; padding: 2rem 0; min-width: 0; }
/* Search */
.top-search { display: flex; gap: 0.75rem; align-items: center; margin-bottom: 1.5rem; }
.top-search input { flex: 1; padding: 0.5rem 0.9rem; background: #161b22; border: 1px solid #30363d; border-radius: 8px; color: #e6edf3; font-size: 0.95rem; outline: none; }
.top-search input:focus { border-color: #58a6ff; }
.no-results { color: #8b949e; font-size: 0.95rem; padding: 1rem 0; display: none; }
/* Item cards */
.item-list { display: flex; flex-direction: column; gap: 0.6rem; }
.item-card { background: #161b22; border: 1px solid #21262d; border-radius: 8px; padding: 1rem 1.25rem; text-decoration: none; color: inherit; transition: border-color 0.15s; display: block; }
.item-card:hover { border-color: #58a6ff; text-decoration: none; }
.item-card .card-header { display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.3rem; }
.kind { font-size: 0.72rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; padding: 2px 7px; border-radius: 4px; }
.kind-fn { background: #1c2d3e; color: #58a6ff; }
.kind-record { background: #1a3025; color: #3fb950; }
.kind-tool { background: #2d2208; color: #d29922; }
.item-card .name { font-weight: 600; font-size: 1rem; color: #f0f6fc; }
.item-card pre { background: #0d1117; border: 1px solid #21262d; border-radius: 6px; padding: 0.7rem 1rem; overflow-x: auto; font-size: 0.85rem; margin: 0.5rem 0; }
code { font-family: "Fira Code", "Cascadia Code", "Consolas", monospace; }
.doc-preview { color: #8b949e; font-size: 0.9rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.doc-full { color: #c9d1d9; font-size: 0.95rem; line-height: 1.8; margin: 0.75rem 0; }
.meta { color: #6e7681; font-size: 0.82rem; margin-top: 0.5rem; }
/* Item detail page */
.detail-header { margin-bottom: 1.5rem; }
.detail-header h2 { font-size: 1.5rem; color: #f0f6fc; display: flex; align-items: center; gap: 0.6rem; margin-top: 0; border-bottom: 1px solid #21262d; padding-bottom: 0.5rem; }
.back-link { display: inline-flex; align-items: center; gap: 0.3rem; color: #58a6ff; font-size: 0.9rem; margin-bottom: 1.2rem; }
.back-link:hover { text-decoration: underline; }
"#;

    // JS for client-side search (index page)
    let search_js = r#"
(function() {
  var input = document.getElementById('search-input');
  var cards = Array.from(document.querySelectorAll('.item-card[data-name]'));
  var noResults = document.querySelector('.no-results');
  var tocLinks = Array.from(document.querySelectorAll('.toc-link[data-name]'));
  function filter() {
    var q = input.value.trim().toLowerCase();
    var visible = 0;
    cards.forEach(function(card) {
      var name = card.dataset.name.toLowerCase();
      var kind = card.dataset.kind.toLowerCase();
      var doc = (card.dataset.doc || '').toLowerCase();
      var match = !q || name.includes(q) || kind.includes(q) || doc.includes(q);
      card.style.display = match ? '' : 'none';
      if (match) visible++;
    });
    tocLinks.forEach(function(link) {
      var name = link.dataset.name.toLowerCase();
      link.style.display = (!q || name.includes(q)) ? '' : 'none';
    });
    noResults.style.display = (visible === 0 && q) ? '' : 'none';
  }
  input.addEventListener('input', filter);
  // Also wire sidebar search
  var sideInput = document.getElementById('sidebar-search');
  if (sideInput) {
    sideInput.addEventListener('input', function() {
      input.value = sideInput.value;
      filter();
    });
  }
})();
"#;

    // Build sidebar ToC entries grouped by kind
    let toc_html: String = {
        let kinds = ["fn", "tool", "record", "enum"];
        let mut parts = Vec::new();
        for kind in &kinds {
            let items_of_kind: Vec<_> = all_items.iter().filter(|i| i.kind == *kind).collect();
            if items_of_kind.is_empty() { continue; }
            let links: String = items_of_kind.iter().map(|item| {
                let slug = format!("{}-{}.html", item.kind, slug_name(&item.name));
                format!(r#"<a class="toc-link" href="{slug}" data-name="{name}">{name}</a>"#,
                    slug = slug,
                    name = html_escape(&item.name))
            }).collect::<Vec<_>>().join("\n");
            parts.push(format!(
                r#"<div class="toc-section"><div class="toc-label">{kind}</div>{links}</div>"#,
                kind = kind,
                links = links
            ));
        }
        parts.join("\n")
    };

    // Generate individual item pages
    for item in &all_items {
        let slug = format!("{}-{}.html", item.kind, slug_name(&item.name));
        let html = format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{name} — Lace Docs</title>
<style>{css}</style>
</head>
<body>
<nav class="topnav"><h1>Lace Docs</h1><span class="subtitle">{kind} {name}</span></nav>
<div class="page-wrap">
<aside>
<input class="search-box" id="sidebar-search" type="search" placeholder="Search…" />
{toc}
</aside>
<div class="content">
<a class="back-link" href="index.html">← All items</a>
<div class="detail-header">
<h2><span class="kind kind-{kind}">{kind}</span> {name}</h2>
</div>
<pre><code>{sig}</code></pre>
<div class="doc-full">{doc}</div>
<p class="meta">Defined in <code>{src}</code></p>
</div>
</div>
</body>
</html>"#,
            name = html_escape(&item.name),
            kind = item.kind,
            css = css,
            toc = toc_html,
            sig = html_escape(&item.signature),
            doc = html_escape(&item.doc).replace('\n', "<br>"),
            src = html_escape(&item.source_file),
        );
        fs::write(out_dir.join(&slug), &html)
            .with_context(|| format!("failed to write {slug}"))?;
    }

    // Generate index.html with sidebar, search, and cards
    let item_rows: String = all_items
        .iter()
        .map(|item| {
            let slug = format!("{}-{}.html", item.kind, slug_name(&item.name));
            let preview = item.doc.lines().next().unwrap_or("").to_string();
            format!(
                r#"<a class="item-card" href="{slug}" data-name="{name}" data-kind="{kind}" data-doc="{doc_data}">
  <div class="card-header"><span class="kind kind-{kind}">{kind}</span><span class="name">{name}</span></div>
  <div class="doc-preview">{preview}</div>
</a>"#,
                slug = slug,
                kind = item.kind,
                name = html_escape(&item.name),
                doc_data = html_escape(&preview),
                preview = html_escape(&preview),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let index_html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Lace Docs</title>
<style>{css}</style>
</head>
<body>
<nav class="topnav"><h1>Lace Docs</h1><span class="subtitle">{count} documented item(s)</span></nav>
<div class="page-wrap">
<aside>
<input class="search-box" id="sidebar-search" type="search" placeholder="Filter…" />
{toc}
</aside>
<div class="content">
<div class="top-search">
  <input id="search-input" type="search" placeholder="Search functions, types, tools…" autofocus />
</div>
<p class="no-results">No items match your search.</p>
<div class="item-list">
{rows}
</div>
</div>
</div>
<script>{js}</script>
</body>
</html>"#,
        css = css,
        count = all_items.len(),
        toc = toc_html,
        rows = item_rows,
        js = search_js,
    );
    fs::write(out_dir.join("index.html"), &index_html).context("failed to write docs/index.html")?;

    println!(
        "{} docs/ — {} item(s) documented",
        "doc ok:".green().bold(),
        all_items.len()
    );

    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn slug_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

fn build_fn_signature(f: &lace_ast::FnDecl) -> String {
    let params = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, fmt_type_expr(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = f
        .ret_ty
        .as_ref()
        .map(|t| format!(" -> {}", fmt_type_expr(t)))
        .unwrap_or_default();
    let effects = if f.effects.is_empty() {
        String::new()
    } else {
        let tags = f.effects.iter().map(fmt_effect_expr).collect::<Vec<_>>().join(", ");
        format!(" [{tags}]")
    };
    format!("fn {}({}){}{}", f.name, params, ret, effects)
}

fn fmt_program(program: &lace_ast::Program) -> String {
    let mut out = String::new();
    let items = &program.items;
    for (i, item) in items.iter().enumerate() {
        out.push_str(&fmt_top_level_item(item));
        if i + 1 < items.len() {
            out.push('\n');
        }
    }
    // Ensure single trailing newline
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn fmt_top_level_item(item: &lace_ast::TopLevelItem) -> String {
    use lace_ast::TopLevelItem;
    match item {
        TopLevelItem::Function(f) => fmt_fn_decl(f),
        TopLevelItem::Const(c) => {
            format!("const {}: {} = {}\n", c.name, fmt_type_expr(&c.ty), fmt_expr(&c.expr))
        }
        _ => {
            // For record, enum, import, tool — emit a placeholder comment for now
            // (a full pretty-printer is out of scope; emit source unchanged)
            String::new()
        }
    }
}

fn fmt_fn_decl(f: &lace_ast::FnDecl) -> String {
    let params = f.params.iter()
        .map(|p| format!("{}: {}", p.name, fmt_type_expr(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = f.ret_ty.as_ref().map(|t| format!(" -> {}", fmt_type_expr(t))).unwrap_or_default();
    let effects = if f.effects.is_empty() {
        String::new()
    } else {
        let tags = f.effects.iter().map(fmt_effect_expr).collect::<Vec<_>>().join(", ");
        format!(" [{tags}]")
    };
    let body = fmt_block(&f.body, 0);
    format!("fn {}({}){}{} {}\n", f.name, params, ret, effects, body)
}

fn fmt_block(block: &lace_ast::Block, indent: usize) -> String {
    let pad = "    ".repeat(indent + 1);
    let close_pad = "    ".repeat(indent);
    let mut lines = vec!["{".to_string()];
    for stmt in &block.stmts {
        lines.push(format!("{}{}", pad, fmt_stmt(stmt, indent + 1)));
    }
    if let Some(tail) = &block.tail_expr {
        lines.push(format!("{}{}", pad, fmt_expr(tail)));
    }
    lines.push(format!("{}}}", close_pad));
    lines.join("\n")
}

fn fmt_stmt(stmt: &lace_ast::Stmt, indent: usize) -> String {
    use lace_ast::Stmt;
    match stmt {
        Stmt::Let(s) => {
            if let Some(ty) = &s.ty {
                format!("let {}: {} = {}", s.name, fmt_type_expr(ty), fmt_expr(&s.expr))
            } else {
                format!("let {} = {}", s.name, fmt_expr(&s.expr))
            }
        }
        Stmt::MutLet(s) => {
            if let Some(ty) = &s.ty {
                format!("mut {}: {} = {}", s.name, fmt_type_expr(ty), fmt_expr(&s.expr))
            } else {
                format!("mut {} = {}", s.name, fmt_expr(&s.expr))
            }
        }
        Stmt::Assign(a) => format!("{} = {}", a.name, fmt_expr(&a.expr)),
        Stmt::Expr(e) => fmt_expr(e),
        Stmt::For(f) => {
            let body = fmt_block(&f.body, indent);
            format!("for {} in {} {}", f.name, fmt_expr(&f.iter), body)
        }
        Stmt::While(w) => {
            let body = fmt_block(&w.body, indent);
            format!("while {} {}", fmt_expr(&w.cond), body)
        }
        Stmt::PureBlock(b) => fmt_block(b, indent),
    }
}

fn fmt_expr(expr: &lace_ast::Expr) -> String {
    use lace_ast::{BinaryOp, Expr, Literal, UnaryOp};
    match expr {
        Expr::Literal(l, _) => match l {
            Literal::Int(n) => n.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::String(s) => format!("\"{s}\""),
            Literal::Bool(b) => b.to_string(),
        },
        Expr::Ident(name, _) => name.clone(),
        Expr::Binary { left, op, right, .. } => {
            let op_str = match op {
                BinaryOp::Add => "+", BinaryOp::Sub => "-",
                BinaryOp::Mul => "*", BinaryOp::Div => "/",
                BinaryOp::IntDiv => "//", BinaryOp::Rem => "%",
                BinaryOp::Eq => "==", BinaryOp::Ne => "!=",
                BinaryOp::Lt => "<", BinaryOp::Gt => ">",
                BinaryOp::Le => "<=", BinaryOp::Ge => ">=",
                BinaryOp::And => "and", BinaryOp::Or => "or",
                BinaryOp::Concat => "++",
            };
            format!("{} {} {}", fmt_expr(left), op_str, fmt_expr(right))
        }
        Expr::Unary { op, expr, .. } => {
            let op_str = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "not ",
            };
            format!("{}{}", op_str, fmt_expr(expr))
        }
        Expr::FnCall(call) => {
            let args = call.args.iter().map(|a| fmt_expr(a)).collect::<Vec<_>>().join(", ");
            format!("{}({})", call.name, args)
        }
        Expr::Block(b) => fmt_block(b, 0),
        Expr::If(i) => {
            let mut parts = Vec::new();
            for (j, (cond, blk)) in i.branches.iter().enumerate() {
                let kw = if j == 0 { "if" } else { "else if" };
                parts.push(format!("{} {} {}", kw, fmt_expr(cond), fmt_block(blk, 0)));
            }
            if let Some(else_blk) = &i.else_block {
                parts.push(format!("else {}", fmt_block(else_blk, 0)));
            }
            parts.join(" ")
        }
        Expr::Return { value, .. } => match value {
            Some(v) => format!("return {}", fmt_expr(v)),
            None => "return".to_string(),
        },
        Expr::ListLiteral { elems, .. } => {
            let items = elems.iter().map(|e| fmt_expr(e)).collect::<Vec<_>>().join(", ");
            format!("[{items}]")
        }
        Expr::TupleLiteral { elems, .. } => {
            let items = elems.iter().map(|e| fmt_expr(e)).collect::<Vec<_>>().join(", ");
            format!("({items})")
        }
        // Fallback: emit a comment placeholder
        _ => "/* expr */".to_string(),
    }
}

fn fmt_type_expr(ty: &lace_ast::TypeExpr) -> String {
    use lace_ast::{PrimitiveType, TypeExpr};
    match ty {
        TypeExpr::Primitive(p, _) => match p {
            PrimitiveType::Int => "Int".to_string(),
            PrimitiveType::Float => "Float".to_string(),
            PrimitiveType::Bool => "Bool".to_string(),
            PrimitiveType::String => "String".to_string(),
            PrimitiveType::Bytes => "Bytes".to_string(),
            PrimitiveType::Unit => "Unit".to_string(),
        },
        TypeExpr::Dynamic(_) => "Dynamic".to_string(),
        TypeExpr::Named { name, .. } => name.clone(),
        TypeExpr::Generic { name, args, .. } => {
            let a = args.iter().map(|a| fmt_type_expr(a)).collect::<Vec<_>>().join(", ");
            format!("{name}<{a}>")
        }
        TypeExpr::Tuple { elems, .. } => {
            let e = elems.iter().map(|e| fmt_type_expr(e)).collect::<Vec<_>>().join(", ");
            format!("({e})")
        }
        TypeExpr::Function { params, ret, .. } => {
            let p = params.iter().map(|p| fmt_type_expr(p)).collect::<Vec<_>>().join(", ");
            format!("fn({p}) -> {}", fmt_type_expr(ret))
        }
    }
}

fn fmt_effect_expr(expr: &lace_ast::EffectExpr) -> String {
    use lace_ast::{EffectExpr, EffectTag};
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

// ─── Bundle ──────────────────────────────────────────────────────────────────

const BUNDLE_MAGIC: &[u8] = &[0x4C, 0x41, 0x43, 0x45, 0x42, 0x4E, 0x44, 0x4C]; // "LACEBNDL"

fn extract_bundle(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 16 {
        return None;
    }
    let tail = &bytes[bytes.len() - 8..];
    if tail != BUNDLE_MAGIC {
        return None;
    }
    let len_bytes = &bytes[bytes.len() - 16..bytes.len() - 8];
    let len = u64::from_le_bytes(len_bytes.try_into().ok()?) as usize;
    let start = bytes.len().checked_sub(16 + len)?;
    Some(bytes[start..bytes.len() - 16].to_vec())
}

fn run_bundle(file: &str, output: Option<&str>) -> Result<()> {
    let source = fs::read_to_string(file)
        .with_context(|| format!("failed to read {file}"))?;

    let bytecode = lace_vm::compile_to_bytes(&source)
        .map_err(|e| anyhow::anyhow!("compile error: {}", e))?;

    let exe = std::env::current_exe().context("failed to get current executable")?;
    let mut exe_bytes = fs::read(&exe)
        .with_context(|| format!("failed to read executable {}", exe.display()))?;

    // If the current exe is itself a bundle, strip the trailer so we don't double-embed
    if extract_bundle(&exe_bytes).is_some() {
        let trailer_len = 16 + {
            let len_bytes = &exe_bytes[exe_bytes.len() - 16..exe_bytes.len() - 8];
            u64::from_le_bytes(len_bytes.try_into().unwrap()) as usize
        };
        let new_len = exe_bytes.len() - trailer_len;
        exe_bytes.truncate(new_len);
    }

    // Append bytecode + length (u64 LE) + magic
    exe_bytes.extend_from_slice(&bytecode);
    let len_bytes = (bytecode.len() as u64).to_le_bytes();
    exe_bytes.extend_from_slice(&len_bytes);
    exe_bytes.extend_from_slice(BUNDLE_MAGIC);

    // Determine output path
    let out_path = if let Some(o) = output {
        PathBuf::from(o)
    } else {
        PathBuf::from(file).with_extension("bin")
    };

    fs::write(&out_path, &exe_bytes)
        .with_context(|| format!("failed to write {}", out_path.display()))?;

    // Set executable bit on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&out_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&out_path, perms)?;
    }

    println!(
        "{} {}  ({} bytes)",
        "bundled:".green().bold(),
        out_path.display(),
        exe_bytes.len()
    );

    Ok(())
}

// ─── lace watch ──────────────────────────────────────────────────────────────

fn run_watch(file: &Path, no_warnings: bool) -> Result<()> {
    use std::time::{Duration, SystemTime};

    let file = file.to_path_buf();

    // Helper: get mtime, returns SystemTime::UNIX_EPOCH on error
    let get_mtime = |p: &Path| -> SystemTime {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    };

    // Helper: run the file once
    let run_once = |p: &Path, no_warn: bool| {
        // Clear terminal
        print!("\x1B[2J\x1B[H");
        let _ = std::io::Write::flush(&mut std::io::stdout());

        eprintln!(
            "{} {}",
            "watch:".cyan().bold(),
            p.display()
        );

        let source = match fs::read_to_string(p) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{} {}", "error reading file:".red().bold(), e);
                return;
            }
        };

        match validate_source_with_warnings(&source, no_warn) {
            Ok((program, effect_issues)) => {
                print_effect_summary(&program, &effect_issues);

                let options = RunOptions {
                    checkpoint_path: None,
                    replay_mode: false,
                    source_path: Some(p.display().to_string()),
                    suppress_tool_log: true,
                    log_file: None,
                };

                match run_with_options(&program, options) {
                    Ok(value) => {
                        if !matches!(value, Value::Unit) {
                            println!("{}", render_value(&value).bright_white());
                        }
                    }
                    Err(err) => {
                        report_runtime_error_with_file(&source, &err, Some(&p.display().to_string()));
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}", "error:".red().bold(), e);
            }
        }
    };

    eprintln!(
        "{} watching {} — press Ctrl+C to stop",
        "watch:".cyan().bold(),
        file.display()
    );

    // Run immediately
    run_once(&file, no_warnings);

    let mut last_mtime = get_mtime(&file);

    loop {
        std::thread::sleep(Duration::from_millis(500));

        let mtime = get_mtime(&file);
        if mtime != last_mtime {
            last_mtime = mtime;
            run_once(&file, no_warnings);
        }
    }
}
