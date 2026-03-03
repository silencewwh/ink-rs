use anyhow::{bail, Context, Result};
use clap::Parser;
use ink_compiler::{canonicalize_json, CompileOptions};
use std::fs;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

#[derive(Parser, Debug)]
#[command(name = "ink-diff-harness")]
#[command(about = "Differential harness: C# inklecate vs Rust runtime (phase-1 scaffold)")]
struct Cli {
    /// 待测试 .ink 文件
    #[arg(long)]
    ink: PathBuf,

    /// C# inklecate 项目路径（可选，不传则自动探测）
    #[arg(long)]
    csharp_project: Option<PathBuf>,

    /// 是否打印双方输出
    #[arg(long)]
    dump_output: bool,

    /// 传递给 Rust 端的自动 choice 序列（可重复）
    #[arg(long = "choose")]
    choose_indices: Vec<usize>,

    /// 严格模式：Rust 端 warning 直接失败
    #[arg(long)]
    strict: bool,

    /// 比较前去除双方输出中的空白差异
    #[arg(long)]
    normalize_whitespace: bool,

    /// 跳过“编译产物 JSON 结构差分”
    #[arg(long)]
    skip_struct_diff: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if !cli.ink.exists() {
        bail!("ink file not found: {}", cli.ink.display());
    }

    let csharp_project = resolve_csharp_project(cli.csharp_project)?;

    let temp = TempDir::new().context("failed to create temp dir")?;
    let csharp_output_json = temp.path().join("out.csharp.ink.json");
    let rust_output_json = temp.path().join("out.rust.ink.json");

    compile_with_csharp(&csharp_project, &cli.ink, &csharp_output_json)?;

    compile_with_rust(&cli.ink, &rust_output_json, cli.strict)?;

    if !cli.skip_struct_diff {
        compare_compiled_json_structure(&csharp_output_json, &rust_output_json)?;
        println!("[OK] compiled JSON structure matches (canonicalized)");
    }

    let csharp_text = play_with_csharp(&csharp_project, &csharp_output_json)?;
    let rust_result = play_with_rust(&csharp_output_json, &cli.choose_indices, cli.strict)?;

    let csharp_for_cmp = if cli.normalize_whitespace {
        normalize_text_for_diff(&csharp_text)
    } else {
        csharp_text.clone()
    };

    let rust_for_cmp = if cli.normalize_whitespace {
        normalize_text_for_diff(&rust_result.stdout)
    } else {
        rust_result.stdout.clone()
    };

    if cli.dump_output {
        println!("===== C# OUTPUT =====");
        println!("{csharp_text}");
        println!("===== RUST OUTPUT =====");
        println!("{}", rust_result.stdout);
        if !rust_result.stderr.trim().is_empty() {
            println!("===== RUST STDERR =====");
            println!("{}", rust_result.stderr);
        }
    }

    if csharp_for_cmp == rust_for_cmp {
        println!("[OK] outputs match");
        Ok(())
    } else {
        bail!(
            "[DIFF] outputs differ (phase-1 runtime still partial; inspect --dump-output for details)"
        )
    }
}

fn compile_with_csharp(csharp_project: &Path, ink_file: &Path, output_json: &Path) -> Result<()> {
    let status = Command::new("dotnet")
        .arg("run")
        .arg("--project")
        .arg(csharp_project)
        .arg("--")
        .arg("-o")
        .arg(output_json)
        .arg(ink_file)
        .status()
        .with_context(|| "failed to run dotnet inklecate compile")?;

    if !status.success() {
        bail!("csharp compile failed with status: {status}");
    }

    Ok(())
}

fn play_with_csharp(csharp_project: &Path, output_json: &Path) -> Result<String> {
    let out = Command::new("dotnet")
        .arg("run")
        .arg("--project")
        .arg(csharp_project)
        .arg("--")
        .arg("-p")
        .arg(output_json)
        .output()
        .with_context(|| "failed to run dotnet inklecate play")?;

    if !out.status.success() {
        bail!(
            "csharp play failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn compile_with_rust(ink_file: &Path, output_json: &Path, strict: bool) -> Result<()> {
    let compile_out = ink_compiler::compile_ink_from_path(
        ink_file,
        CompileOptions {
            strict,
            source_name: Some(ink_file.display().to_string()),
        },
    )
    .with_context(|| format!("failed to compile ink with rust compiler: {}", ink_file.display()))?;

    fs::write(output_json, compile_out.story_json)
        .with_context(|| format!("failed to write rust compiled json: {}", output_json.display()))?;

    Ok(())
}

fn compare_compiled_json_structure(csharp_output_json: &Path, rust_output_json: &Path) -> Result<()> {
    let csharp_raw = fs::read_to_string(csharp_output_json).with_context(|| {
        format!(
            "failed to read csharp compiled json: {}",
            csharp_output_json.display()
        )
    })?;

    let rust_raw = fs::read_to_string(rust_output_json).with_context(|| {
        format!(
            "failed to read rust compiled json: {}",
            rust_output_json.display()
        )
    })?;

    let csharp_canonical = canonicalize_json(&csharp_raw)
        .context("failed to canonicalize csharp compiled json")?;
    let rust_canonical =
        canonicalize_json(&rust_raw).context("failed to canonicalize rust compiled json")?;

    if csharp_canonical == rust_canonical {
        Ok(())
    } else {
        bail!(
            "[DIFF] compiled JSON structure differs (csharp vs rust compiler). You can re-run with --skip-struct-diff to focus on runtime behavior diff only"
        )
    }
}

#[derive(Debug)]
struct RustRunResult {
    stdout: String,
    stderr: String,
}

fn play_with_rust(output_json: &Path, choose_indices: &[usize], strict: bool) -> Result<RustRunResult> {
    let manifest_path = workspace_manifest_path();

    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("-q")
        .arg("-p")
        .arg("inklecate-rs")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--")
        .arg(output_json)
        .arg("-p")
        .arg("--show-warnings")
        .arg("--dump-choices");

    if strict {
        command.arg("--strict");
    }

    for idx in choose_indices {
        command.arg("--choose").arg(idx.to_string());
    }

    let out = command
        .output()
        .with_context(|| "failed to run rust inklecate-rs")?;

    if !out.status.success() {
        bail!(
            "rust play failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    Ok(RustRunResult {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

fn resolve_csharp_project(user_input: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = user_input {
        if p.exists() {
            return Ok(p);
        }
        bail!("provided csharp project does not exist: {}", p.display());
    }

    let candidates = [
        PathBuf::from("ink/inklecate/inklecate.csproj"),
        PathBuf::from("../ink/inklecate/inklecate.csproj"),
    ];

    for p in candidates {
        if p.exists() {
            return Ok(p);
        }
    }

    bail!(
        "failed to auto-detect C# inklecate project. Please pass --csharp-project <path>"
    )
}

fn workspace_manifest_path() -> PathBuf {
    // crates/ink-diff-harness -> (../..) -> ink-rs/Cargo.toml
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml")
}

fn normalize_text_for_diff(raw: &str) -> String {
    raw.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}
