use anyhow::{bail, Context, Result};
use clap::Parser;
use ink_cli_protocol::{write_event_json_line, ChoiceItem, CliEvent};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "inklecate-rs")]
#[command(about = "Rust inklecate-compatible CLI (phase-3 scaffold)")]
struct Cli {
    /// 输入文件（.ink 或 .json）
    input: String,

    /// Output file name (compile mode)
    #[arg(short = 'o', long = "output")]
    output_file: Option<String>,

    /// Play mode
    #[arg(short = 'p', long = "play")]
    play_mode: bool,

    /// JSON output mode
    #[arg(short = 'j', long = "json")]
    json_output: bool,

    /// Stats mode
    #[arg(short = 's', long = "stats")]
    stats: bool,

    /// Verbose mode
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Count all visits (compiler option)
    #[arg(short = 'c', long = "count-all-visits")]
    count_all_visits: bool,

    /// Keep open after story finish (play mode)
    #[arg(short = 'k', long = "keep-open")]
    keep_open_after_story_finish: bool,

    /// Plugin directory (can be used multiple times)
    #[arg(short = 'x', long = "plugin-dir")]
    plugin_dirs: Vec<String>,

    /// 严格模式：若出现未实现 warning 则返回非 0
    #[arg(long = "strict")]
    strict: bool,

    /// 自动选择第 N 个选项（从 0 开始），可重复传入用于多轮 choice
    #[arg(long = "choose")]
    choose_indices: Vec<usize>,

    /// 在输出末尾打印当前可选项（用于调试与差分）
    #[arg(long = "dump-choices")]
    dump_choices: bool,

    /// 显示未实现语义 warning（human 模式下写 stderr）
    #[arg(long = "show-warnings")]
    show_warnings: bool,

    /// 显示 rust 编译阶段诊断（仅输入为 .ink 时有效）
    #[arg(long = "show-compiler-diagnostics")]
    show_compiler_diagnostics: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let started_at = Instant::now();

    let input_path = PathBuf::from(&cli.input);
    let input_is_json = input_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));

    if input_is_json && cli.stats {
        bail!("Cannot show stats for .json, only for .ink");
    }

    let should_use_csharp_fallback = cli.stats || !cli.plugin_dirs.is_empty();
    if should_use_csharp_fallback {
        run_via_csharp_fallback(&cli)?;
        return Ok(());
    }

    let mut issues = Vec::<String>::new();

    if cli.stats {
        bail!("stats mode currently requires C# fallback backend");
    }

    if input_is_json {
        run_play_from_json_file(&cli, &input_path, &mut issues)?;
    } else if cli.play_mode {
        run_play_from_ink_file(&cli, &input_path, &mut issues)?;
    } else {
        run_compile_mode(&cli, &input_path, &mut issues)?;
    }

    if cli.strict && !issues.is_empty() {
        bail!(
            "strict mode failed: encountered {} diagnostics/warnings",
            issues.len()
        );
    }

    if cli.verbose {
        eprintln!(
            "[verbose] total elapsed: {} ms",
            started_at.elapsed().as_millis()
        );
    }

    Ok(())
}

fn run_compile_mode(cli: &Cli, input_path: &Path, issues: &mut Vec<String>) -> Result<()> {
    let started_at = Instant::now();

    let compile_out = ink_compiler::compile_ink_from_path(
        input_path,
        ink_compiler::CompileOptions {
            strict: cli.strict,
            source_name: Some(input_path.display().to_string()),
        },
    )
    .with_context(|| format!("failed to compile ink source: {}", input_path.display()))?;

    let output_path = cli
        .output_file
        .clone()
        .unwrap_or_else(|| default_output_path(input_path));

    std::fs::write(&output_path, &compile_out.story_json)
        .with_context(|| format!("failed to write output file: {output_path}"))?;

    if cli.count_all_visits {
        issues.push("WARNING: -c/--count-all-visits is accepted but not implemented in rust compiler backend".to_string());
    }

    for d in &compile_out.diagnostics {
        let msg = format!(
            "{:?}: {}:{} {} ({})",
            d.severity, d.line, d.column, d.message, d.code
        );
        if cli.show_compiler_diagnostics {
            eprintln!("[compiler] {msg}");
        }
        issues.push(msg);
    }

    if cli.json_output {
        emit_json_event(&CliEvent::CompileSuccess {
            compile_success: true,
        })?;
        if !issues.is_empty() {
            emit_json_event(&CliEvent::Issues {
                issues: issues.clone(),
            })?;
        }
        emit_json_event(&CliEvent::ExportComplete {
            export_complete: true,
        })?;
    }

    if cli.verbose {
        eprintln!(
            "[verbose] compile elapsed: {} ms",
            started_at.elapsed().as_millis()
        );
    }

    Ok(())
}

fn run_play_from_ink_file(cli: &Cli, input_path: &Path, issues: &mut Vec<String>) -> Result<()> {
    let compile_out = ink_compiler::compile_ink_from_path(
        input_path,
        ink_compiler::CompileOptions {
            strict: cli.strict,
            source_name: Some(input_path.display().to_string()),
        },
    )
    .with_context(|| format!("failed to compile ink source: {}", input_path.display()))?;

    for d in &compile_out.diagnostics {
        issues.push(format!(
            "{:?}: {}:{} {} ({})",
            d.severity, d.line, d.column, d.message, d.code
        ));
    }

    let doc = ink_json::load_ink_doc_from_str(&compile_out.story_json)
        .context("failed to parse compiled runtime json")?;
    let mut story = ink_runtime::Story::from_doc(doc);
    run_play_loop(cli, &mut story, issues)
}

fn run_play_from_json_file(cli: &Cli, input_path: &Path, issues: &mut Vec<String>) -> Result<()> {
    let doc = ink_json::load_ink_doc_from_path(input_path)
        .with_context(|| format!("failed to load ink json: {}", input_path.display()))?;
    let mut story = ink_runtime::Story::from_doc(doc);
    run_play_loop(cli, &mut story, issues)
}

fn run_play_loop(
    cli: &Cli,
    story: &mut ink_runtime::Story,
    issues: &mut Vec<String>,
) -> Result<()> {
    let mut choose_iter = cli.choose_indices.iter().copied();

    loop {
        while story.can_continue() {
            let line = story.continue_line();
            if cli.json_output {
                emit_json_event(&CliEvent::Text { text: line })?;
            } else {
                print!("{line}");
            }
        }

        let choices = story.current_choices();
        if choices.is_empty() {
            if cli.keep_open_after_story_finish {
                issues.push("WARNING: -k/--keep-open is accepted but interactive input loop is not implemented in rust backend".to_string());
            }
            break;
        }

        if cli.dump_choices {
            for (i, c) in choices.iter().enumerate() {
                eprintln!("[choice #{i}] text={} target={}", c.text, c.target_path);
            }
        }

        if cli.json_output {
            let payload = choices
                .iter()
                .map(|c| ChoiceItem::new(c.text.clone(), c.tags.clone()))
                .collect::<Vec<_>>();
            emit_json_event(&CliEvent::Choices { choices: payload })?;
        }

        let chosen_idx = choose_iter.next().unwrap_or(0);
        if let Err(err) = story.choose_choice_index(chosen_idx) {
            issues.push(format!("ERROR: failed to choose index {chosen_idx}: {err}"));
            break;
        }
    }

    let warnings = story.take_warnings();
    if cli.show_warnings && !cli.json_output {
        for w in &warnings {
            eprintln!("[warn] {w}");
        }
    }
    for w in warnings {
        issues.push(format!("WARNING: {w}"));
    }

    if cli.json_output && !issues.is_empty() {
        emit_json_event(&CliEvent::Issues {
            issues: issues.clone(),
        })?;
    }

    Ok(())
}

fn run_via_csharp_fallback(cli: &Cli) -> Result<()> {
    let csharp_project = resolve_csharp_project_path()?;

    let mut command = Command::new("dotnet");
    command
        .arg("run")
        .arg("--project")
        .arg(csharp_project)
        .arg("--");

    if let Some(out) = &cli.output_file {
        command.arg("-o").arg(out);
    }
    if cli.play_mode {
        command.arg("-p");
    }
    if cli.json_output {
        command.arg("-j");
    }
    if cli.stats {
        command.arg("-s");
    }
    if cli.verbose {
        command.arg("-v");
    }
    if cli.count_all_visits {
        command.arg("-c");
    }
    if cli.keep_open_after_story_finish {
        command.arg("-k");
    }
    for dir in &cli.plugin_dirs {
        command.arg("-x").arg(dir);
    }
    command.arg(&cli.input);

    let out = command
        .output()
        .context("failed to run csharp fallback command")?;

    io::stdout().write_all(&out.stdout)?;
    io::stderr().write_all(&out.stderr)?;

    if !out.status.success() {
        bail!("csharp fallback failed with status: {}", out.status);
    }
    Ok(())
}

fn resolve_csharp_project_path() -> Result<PathBuf> {
    let candidates = [
        PathBuf::from("ink/inklecate/inklecate.csproj"),
        PathBuf::from("../ink/inklecate/inklecate.csproj"),
    ];

    for c in candidates {
        if c.exists() {
            return Ok(c);
        }
    }

    bail!("failed to auto-detect C# inklecate project for fallback backend")
}

fn emit_json_event(event: &CliEvent) -> Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    write_event_json_line(&mut lock, event)?;
    lock.flush()?;
    Ok(())
}

fn default_output_path(input_path: &Path) -> String {
    if let Some(raw) = input_path.to_str() {
        if let Some(stem) = raw.strip_suffix(".ink") {
            return format!("{stem}.ink.json");
        }
    }

    let mut out = input_path.to_path_buf();
    out.set_extension("ink.json");
    out.display().to_string()
}
