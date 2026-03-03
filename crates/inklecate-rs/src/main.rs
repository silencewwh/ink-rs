use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "inklecate-rs")]
#[command(about = "Rust prototype for ink runtime (phase-0)")]
struct Cli {
    /// 已编译的 .ink.json 文件路径
    input: String,

    /// Play mode（逐次 Continue 输出）
    #[arg(short = 'p', long = "play")]
    play_mode: bool,

    /// 显示未实现语义 warning
    #[arg(long = "show-warnings")]
    show_warnings: bool,

    /// 严格模式：若出现未实现 warning 则返回非 0
    #[arg(long = "strict")]
    strict: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let doc = ink_json::load_ink_doc_from_path(&cli.input)
        .with_context(|| format!("failed to load ink json: {}", cli.input))?;

    let mut story = ink_runtime::Story::from_doc(doc);

    if cli.play_mode {
        while story.can_continue() {
            let line = story.continue_line();
            print!("{line}");
        }
    } else {
        let out = story.continue_maximally();
        print!("{out}");
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
