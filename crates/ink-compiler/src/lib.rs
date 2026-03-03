use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: &'static str,
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl Diagnostic {
    fn unimplemented(line: usize, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            code: "UNIMPLEMENTED_SYNTAX",
            message: message.into(),
            line,
            column: 1,
        }
    }

    fn parse_error(line: usize, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            code: "PARSE_ERROR",
            message: message.into(),
            line,
            column: 1,
        }
    }

    pub fn is_unimplemented(&self) -> bool {
        self.code.starts_with("UNIMPLEMENTED")
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// 严格模式下，遇到 UNIMPLEMENTED 诊断直接失败。
    pub strict: bool,
    /// 可选来源名称，便于上层工具打印上下文。
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOutput {
    pub story_json: String,
    pub diagnostics: Vec<Diagnostic>,
    pub ast: AstStory,
}

#[derive(thiserror::Error, Debug)]
pub enum CompilerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("parse error at line {line}: {message}")]
    Parse { line: usize, message: String },
    #[error("strict mode failed: encountered {count} unimplemented diagnostics")]
    StrictMode { count: usize },
}

pub fn compile_ink_from_path(
    path: impl AsRef<Path>,
    mut options: CompileOptions,
) -> Result<CompileOutput, CompilerError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)?;
    if options.source_name.is_none() {
        options.source_name = Some(path.display().to_string());
    }
    compile_ink(&raw, options)
}

pub fn compile_ink(raw: &str, options: CompileOptions) -> Result<CompileOutput, CompilerError> {
    let mut parser = Parser::new(raw);
    let (ast, diagnostics) = parser.parse_story();

    if let Some(first_error) = diagnostics
        .iter()
        .find(|d| d.severity == DiagnosticSeverity::Error)
    {
        return Err(CompilerError::Parse {
            line: first_error.line,
            message: first_error.message.clone(),
        });
    }

    if options.strict {
        let unimplemented_count = diagnostics.iter().filter(|d| d.is_unimplemented()).count();
        if unimplemented_count > 0 {
            return Err(CompilerError::StrictMode {
                count: unimplemented_count,
            });
        }
    }

    let json_value = codegen_story(&ast);
    let story_json = serde_json::to_string_pretty(&json_value)?;

    Ok(CompileOutput {
        story_json,
        diagnostics,
        ast,
    })
}

pub fn canonicalize_json(raw: &str) -> Result<String, CompilerError> {
    let value: Value = serde_json::from_str(raw)?;
    let canonical = canonicalize_value(&value);
    Ok(serde_json::to_string_pretty(&canonical)?)
}

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();

            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), canonicalize_value(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_value).collect()),
        _ => value.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstStory {
    pub root: Vec<AstStmt>,
    pub knots: Vec<AstKnot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstKnot {
    pub name: String,
    pub body: Vec<AstStmt>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstStmt {
    Text { text: String, line: usize },
    Divert { target: String, line: usize },
    Unsupported { raw: String, line: usize },
}

#[derive(Debug, Clone, Copy)]
struct SourceLine<'a> {
    raw: &'a str,
    line: usize,
}

type Checkpoint = usize;

#[derive(Debug)]
struct LineCursor<'a> {
    lines: Vec<SourceLine<'a>>,
    index: usize,
}

impl<'a> LineCursor<'a> {
    fn new(raw: &'a str) -> Self {
        let lines = raw
            .lines()
            .enumerate()
            .map(|(idx, line)| SourceLine {
                raw: line,
                line: idx + 1,
            })
            .collect();

        Self { lines, index: 0 }
    }

    fn checkpoint(&self) -> Checkpoint {
        self.index
    }

    fn rollback(&mut self, checkpoint: Checkpoint) {
        self.index = checkpoint;
    }

    fn commit(&mut self, _checkpoint: Checkpoint) {
        // 目前 checkpoint 仅用于语义表达，提交时无需额外动作。
    }

    fn peek(&self) -> Option<SourceLine<'a>> {
        self.lines.get(self.index).copied()
    }

    fn advance(&mut self) -> Option<SourceLine<'a>> {
        let line = self.peek()?;
        self.index += 1;
        Some(line)
    }

    fn is_eof(&self) -> bool {
        self.index >= self.lines.len()
    }
}

#[derive(Debug)]
struct Parser<'a> {
    cursor: LineCursor<'a>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
struct KnotHeader {
    name: String,
    line: usize,
}

impl<'a> Parser<'a> {
    fn new(raw: &'a str) -> Self {
        Self {
            cursor: LineCursor::new(raw),
            diagnostics: Vec::new(),
        }
    }

    fn parse_story(&mut self) -> (AstStory, Vec<Diagnostic>) {
        let mut root = Vec::new();
        let mut knots = Vec::new();

        while !self.cursor.is_eof() {
            if self.current_line_trimmed().is_some_and(str::is_empty) {
                self.cursor.advance();
                continue;
            }

            if let Some(knot) = self.parse_knot_block() {
                knots.push(knot);
                continue;
            }

            if let Some(stmt) = self.parse_line_stmt() {
                root.push(stmt);
            }
        }

        (
            AstStory { root, knots },
            std::mem::take(&mut self.diagnostics),
        )
    }

    fn parse_knot_block(&mut self) -> Option<AstKnot> {
        let checkpoint = self.cursor.checkpoint();

        let Some(header) = self.parse_knot_header_line() else {
            self.cursor.rollback(checkpoint);
            return None;
        };

        let mut body = Vec::new();
        while !self.cursor.is_eof() {
            if self.next_line_looks_like_knot_header() {
                break;
            }

            if self.current_line_trimmed().is_some_and(str::is_empty) {
                self.cursor.advance();
                continue;
            }

            if let Some(stmt) = self.parse_line_stmt() {
                body.push(stmt);
            }
        }

        self.cursor.commit(checkpoint);
        Some(AstKnot {
            name: header.name,
            body,
            line: header.line,
        })
    }

    fn parse_knot_header_line(&mut self) -> Option<KnotHeader> {
        let line = self.cursor.peek()?;
        let name = parse_knot_header_name(line.raw)?;
        self.cursor.advance();

        if name.is_empty() {
            self.diagnostics
                .push(Diagnostic::parse_error(line.line, "knot name cannot be empty"));
            return Some(KnotHeader {
                name: format!("invalid_knot_{}", line.line),
                line: line.line,
            });
        }

        Some(KnotHeader {
            name: name.to_string(),
            line: line.line,
        })
    }

    fn parse_line_stmt(&mut self) -> Option<AstStmt> {
        if self.cursor.is_eof() {
            return None;
        }

        if self.current_line_trimmed().is_some_and(str::is_empty) {
            self.cursor.advance();
            return None;
        }

        let cp = self.cursor.checkpoint();
        if let Some(stmt) = self.parse_divert_line() {
            self.cursor.commit(cp);
            return Some(stmt);
        }
        self.cursor.rollback(cp);

        let cp = self.cursor.checkpoint();
        if let Some(stmt) = self.parse_text_line() {
            self.cursor.commit(cp);
            return Some(stmt);
        }
        self.cursor.rollback(cp);

        self.parse_unsupported_line()
    }

    fn parse_divert_line(&mut self) -> Option<AstStmt> {
        let line = self.cursor.peek()?;
        let trimmed = line.raw.trim();
        if !trimmed.starts_with("->") {
            return None;
        }

        self.cursor.advance();

        if trimmed.starts_with("->->") {
            self.diagnostics.push(Diagnostic::unimplemented(
                line.line,
                "tunnel onwards (`->->`) is not implemented in rust compiler phase-2 scaffold",
            ));
            return Some(AstStmt::Unsupported {
                raw: trimmed.to_string(),
                line: line.line,
            });
        }

        let target = trimmed.trim_start_matches("->").trim();
        if target.is_empty() {
            self.diagnostics.push(Diagnostic::parse_error(
                line.line,
                "empty divert target is not allowed",
            ));
            return Some(AstStmt::Unsupported {
                raw: trimmed.to_string(),
                line: line.line,
            });
        }

        Some(AstStmt::Divert {
            target: target.to_string(),
            line: line.line,
        })
    }

    fn parse_text_line(&mut self) -> Option<AstStmt> {
        let line = self.cursor.peek()?;
        let trimmed = line.raw.trim();
        if trimmed.is_empty() || looks_like_non_text_construct(trimmed) {
            return None;
        }

        self.cursor.advance();
        Some(AstStmt::Text {
            text: trimmed.to_string(),
            line: line.line,
        })
    }

    fn parse_unsupported_line(&mut self) -> Option<AstStmt> {
        let line = self.cursor.advance()?;
        let trimmed = line.raw.trim();

        if trimmed.starts_with("//") {
            return None;
        }

        self.diagnostics.push(Diagnostic::unimplemented(
            line.line,
            format!("unsupported ink syntax in phase-2 scaffold: `{trimmed}`"),
        ));

        Some(AstStmt::Unsupported {
            raw: trimmed.to_string(),
            line: line.line,
        })
    }

    fn next_line_looks_like_knot_header(&self) -> bool {
        let Some(line) = self.cursor.peek() else {
            return false;
        };
        parse_knot_header_name(line.raw).is_some()
    }

    fn current_line_trimmed(&self) -> Option<&str> {
        self.cursor.peek().map(|line| line.raw.trim())
    }
}

fn parse_knot_header_name(raw_line: &str) -> Option<&str> {
    let trimmed = raw_line.trim();

    let leading_equals = trimmed.len() - trimmed.trim_start_matches('=').len();
    let trailing_equals = trimmed.len() - trimmed.trim_end_matches('=').len();

    if leading_equals < 2 || trailing_equals < 2 {
        return None;
    }

    let inner = &trimmed[leading_equals..trimmed.len() - trailing_equals];
    Some(inner.trim())
}

fn looks_like_non_text_construct(trimmed: &str) -> bool {
    let keyword_prefixes = ["VAR ", "CONST ", "LIST ", "EXTERNAL ", "INCLUDE "];
    if keyword_prefixes.iter().any(|prefix| trimmed.starts_with(prefix)) {
        return true;
    }

    if trimmed.starts_with("==") {
        return true;
    }

    matches!(
        trimmed.chars().next(),
        Some('~' | '*' | '+' | '-' | '{' | '}' | '<' | '=')
    )
}

fn codegen_story(ast: &AstStory) -> Value {
    let mut root_content = codegen_stmts(&ast.root);
    ensure_terminator(&mut root_content);

    let mut root_named = Map::new();
    for knot in &ast.knots {
        let mut knot_content = codegen_stmts(&knot.body);
        ensure_terminator(&mut knot_content);
        root_named.insert(knot.name.clone(), build_container(knot_content, Map::new()));
    }

    let root = build_container(root_content, root_named);
    json!({
        "inkVersion": 21,
        "root": root,
    })
}

fn codegen_stmts(stmts: &[AstStmt]) -> Vec<Value> {
    let mut out = Vec::new();

    for stmt in stmts {
        match stmt {
            AstStmt::Text { text, .. } => {
                out.push(Value::String(format!("^{text}")));
                out.push(Value::String("\n".to_string()));
            }
            AstStmt::Divert { target, .. } => {
                let upper = target.to_ascii_uppercase();
                if upper == "END" {
                    out.push(Value::String("end".to_string()));
                } else if upper == "DONE" {
                    out.push(Value::String("done".to_string()));
                } else {
                    let mut map = Map::new();
                    map.insert("->".to_string(), Value::String(target.clone()));
                    out.push(Value::Object(map));
                }
            }
            AstStmt::Unsupported { .. } => {
                // unsupported 语句仅出诊断，不参与 codegen。
            }
        }
    }

    out
}

fn ensure_terminator(content: &mut Vec<Value>) {
    let has_terminator = content
        .last()
        .and_then(Value::as_str)
        .is_some_and(|s| s == "done" || s == "end");

    if !has_terminator {
        content.push(Value::String("done".to_string()));
    }
}

fn build_container(mut content: Vec<Value>, named: Map<String, Value>) -> Value {
    if named.is_empty() {
        content.push(Value::Null);
    } else {
        content.push(Value::Object(named));
    }
    Value::Array(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_plain_text_and_end_divert() {
        let source = "Hello world\n-> END\n";
        let out = compile_ink(source, CompileOptions::default()).expect("compile should succeed");

        assert!(out.diagnostics.is_empty());

        let json: Value = serde_json::from_str(&out.story_json).expect("json should parse");
        let root = json["root"].as_array().expect("root should be array");

        assert_eq!(root[0], Value::String("^Hello world".to_string()));
        assert_eq!(root[1], Value::String("\n".to_string()));
        assert_eq!(root[2], Value::String("end".to_string()));
    }

    #[test]
    fn compile_knot_block_into_named_container() {
        let source = "Root line\n== start ==\nInside knot\n-> DONE\n";
        let out = compile_ink(source, CompileOptions::default()).expect("compile should succeed");

        let json: Value = serde_json::from_str(&out.story_json).expect("json should parse");
        let root = json["root"].as_array().expect("root should be array");
        let root_tail = root
            .last()
            .and_then(Value::as_object)
            .expect("root tail should be named map");

        assert!(root_tail.contains_key("start"));

        let knot_container = root_tail["start"]
            .as_array()
            .expect("knot container should be array");
        assert_eq!(knot_container[0], Value::String("^Inside knot".to_string()));
        assert_eq!(knot_container[1], Value::String("\n".to_string()));
        assert_eq!(knot_container[2], Value::String("done".to_string()));
    }

    #[test]
    fn strict_mode_fails_on_unimplemented_syntax() {
        let source = "* a choice\n";
        let err = compile_ink(
            source,
            CompileOptions {
                strict: true,
                source_name: None,
            },
        )
        .expect_err("strict mode should fail");

        assert!(matches!(err, CompilerError::StrictMode { .. }));
    }

    #[test]
    fn canonicalize_json_sorts_object_keys_recursively() {
        let raw = r#"{
  "b": {
    "z": 1,
    "a": 2
  },
  "a": [
    {
      "d": 4,
      "c": 3
    }
  ]
}"#;

        let canonical = canonicalize_json(raw).expect("canonicalization should succeed");

        let expected = r#"{
  "a": [
    {
      "c": 3,
      "d": 4
    }
  ],
  "b": {
    "a": 2,
    "z": 1
  }
}"#;

        assert_eq!(canonical, expected);
    }
}