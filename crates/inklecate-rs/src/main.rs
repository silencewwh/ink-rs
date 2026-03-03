use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "inklecate-rs")]
#[command(about = "Rust prototype for ink runtime (phase-0)")]
struct Cli {
    /// 输入文件路径（json 模式下为 .ink.json；ink 模式下为 .ink）
    input: String,

    /// 输入格式：json（默认）或 ink（走 rust 编译器骨架）
    #[arg(long = "input-format", value_enum, default_value_t = InputFormat::Json)]
    input_format: InputFormat,

    /// Play mode（逐次 Continue 输出）
    #[arg(short = 'p', long = "play")]
    play_mode: bool,

    /// 显示未实现语义 warning
    #[arg(long = "show-warnings")]
    show_warnings: bool,

    /// 严格模式：若出现未实现 warning 则返回非 0
    #[arg(long = "strict")]
    strict: bool,

    /// 自动选择第 N 个选项（从 0 开始），可重复传入用于多轮 choice
    #[arg(long = "choose")]
    choose_indices: Vec<usize>,

    /// 在输出末尾打印当前可选项（用于调试与差分）
    #[arg(long = "dump-choices")]
    dump_choices: bool,

    /// 显示 rust 编译阶段诊断（仅 input-format=ink 时有效）
    #[arg(long = "show-compiler-diagnostics")]
    show_compiler_diagnostics: bool,

    /// 导出 rust 编译出的 runtime json 到指定路径（仅 input-format=ink 时有效）
    #[arg(long = "emit-json")]
    emit_json: Option<String>,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum InputFormat {
    Json,
    Ink,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let (doc, compiler_diags) = match cli.input_format {
        InputFormat::Json => {
            let doc = ink_json::load_ink_doc_from_path(&cli.input)
                .with_context(|| format!("failed to load ink json: {}", cli.input))?;
            (doc, Vec::new())
        }
        InputFormat::Ink => {
            let compile_out = ink_compiler::compile_ink_from_path(
                &cli.input,
                ink_compiler::CompileOptions {
                    strict: cli.strict,
                    source_name: Some(cli.input.clone()),
                },
            )
            .with_context(|| format!("failed to compile ink source: {}", cli.input))?;

            if let Some(path) = &cli.emit_json {
                std::fs::write(path, &compile_out.story_json)
                    .with_context(|| format!("failed to write emitted json: {path}"))?;
            }

            let doc = ink_json::load_ink_doc_from_str(&compile_out.story_json)
                .context("failed to parse compiled runtime json")?;

            (doc, compile_out.diagnostics)
        }
    };

    let mut story = ink_runtime::Story::from_doc(doc);

    if cli.play_mode {
        run_play_mode(&mut story, &cli.choose_indices);
    } else {
        let out = story.continue_maximally();
        print!("{out}");
    }

    if cli.dump_choices {
        for (i, c) in story.current_choices().iter().enumerate() {
            eprintln!("[choice #{i}] text={} target={}", c.text, c.target_path);
        }
    }

    if cli.show_compiler_diagnostics {
        for d in &compiler_diags {
            eprintln!(
                "[compiler:{:?}] {}:{} {} ({})",
                d.severity,
                d.line,
                d.column,
                d.message,
                d.code
            );
        }
    }

    let warnings = story.take_warnings();
    if cli.show_warnings {
        for w in &warnings {
            eprintln!("[warn] {w}");
        }
    }

    if cli.strict && !warnings.is_empty() {
        anyhow::bail!(
            "strict mode failed: encountered {} unimplemented runtime behaviors",
            warnings.len()
        );
    }

    Ok(())
}

fn run_play_mode(story: &mut ink_runtime::Story, choose_indices: &[usize]) {
    let mut choose_iter = choose_indices.iter().copied();

    loop {
        while story.can_continue() {
            let line = story.continue_line();
            print!("{line}");
        }

        if story.current_choices().is_empty() {
            break;
        }

        let chosen_idx = choose_iter.next().unwrap_or(0);
        if let Err(err) = story.choose_choice_index(chosen_idx) {
            eprintln!("[error] failed to choose index {chosen_idx}: {err}");
            break;
        }
    }
}
