use ink_model::{InkDoc, RuntimeNode};

#[derive(Debug)]
pub struct Story {
    content: Vec<RuntimeNode>,
    cursor: usize,
    pub current_text: String,
    warnings: Vec<String>,
}

impl Story {
    pub fn from_doc(doc: InkDoc) -> Self {
        let mut flattened = Vec::new();
        flatten_content(doc.root.content, &mut flattened);

        Self {
            content: flattened,
            cursor: 0,
            current_text: String::new(),
            warnings: Vec::new(),
        }
    }

    pub fn can_continue(&self) -> bool {
        self.cursor < self.content.len()
    }

    pub fn continue_line(&mut self) -> String {
        self.current_text.clear();

        while self.cursor < self.content.len() {
            let node = &self.content[self.cursor];
            self.cursor += 1;

            if let Some(fragment) = node.as_text_fragment() {
                self.current_text.push_str(fragment);

                if fragment == "\n" {
                    break;
                }
                continue;
            }

            match node {
                RuntimeNode::Command(cmd) => {
                    // phase-0 先兼容常见终止 token
                    if matches!(cmd.as_str(), "done" | "end" | "nop") {
                        continue;
                    }

                    // 其他控制指令暂未实现，先记录 warning，避免 silent behavior drift
                    self.warnings
                        .push(format!("unimplemented command token encountered: {cmd}"));
                }
                RuntimeNode::UnknownJson(raw) => {
                    self.warnings
                        .push(format!("unimplemented runtime json node encountered: {raw}"));
                }
                RuntimeNode::Container(_) => {
                    // 构造时已 flatten，理论上不应再遇到
                    self.warnings.push(
                        "unexpected nested container at runtime (should be flattened)".to_string(),
                    );
                }
                RuntimeNode::Null
                | RuntimeNode::Int(_)
                | RuntimeNode::Float(_)
                | RuntimeNode::Bool(_)
                | RuntimeNode::DivertTarget(_) => {
                    // phase-0: ignore non-text tokens
                }
                RuntimeNode::Str(_) | RuntimeNode::Newline => {
                    // 已由 as_text_fragment 覆盖，理论不会走到这里
                }
            }
        }

        self.current_text.clone()
    }

    pub fn continue_maximally(&mut self) -> String {
        let mut out = String::new();
        while self.can_continue() {
            let line = self.continue_line();
            if line.is_empty() {
                // 避免全是控制节点时无意义死循环
                if self.cursor >= self.content.len() {
                    break;
                }
            }
            out.push_str(&line);
        }
        out
    }

    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
}

fn flatten_content(nodes: Vec<RuntimeNode>, out: &mut Vec<RuntimeNode>) {
    for node in nodes {
        match node {
            RuntimeNode::Container(c) => {
                // 只先展开有序内容；named content 后续在 divert/path 语义中使用
                flatten_content(c.content, out);
            }
            other => out.push(other),
        }
    }
}
