use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct InkDoc {
    pub ink_version: i32,
    pub root: Container,
}

#[derive(Debug, Clone, Default)]
pub struct Container {
    pub content: Vec<RuntimeNode>,
    pub named: HashMap<String, RuntimeNode>,
    pub name: Option<String>,
    pub flags: Option<i32>,
}

#[derive(Debug, Clone)]
pub enum RuntimeNode {
    Str(String),
    Newline,
    Int(i64),
    Float(f64),
    Bool(bool),
    Container(Container),
    Command(String),
    DivertTarget(String),
    UnknownJson(String),
    Null,
}

impl RuntimeNode {
    pub fn as_text_fragment(&self) -> Option<&str> {
        match self {
            RuntimeNode::Str(s) => Some(s.as_str()),
            RuntimeNode::Newline => Some("\n"),
            _ => None,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum InkModelError {
    #[error("unsupported runtime node: {0}")]
    UnsupportedNode(String),
}
