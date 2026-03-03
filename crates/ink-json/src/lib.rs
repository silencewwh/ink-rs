use std::{collections::HashMap, fs, path::Path};

use ink_model::{
    ChoicePoint, Container, ControlCommandKind, Divert, InkDoc, PushPopType, RuntimeNode,
    UnsupportedNode, UnsupportedNodeReason, VariableAssignment, VariableReference,
};
use serde_json::Value;

#[derive(thiserror::Error, Debug)]
pub enum InkJsonError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid ink json: {0}")]
    InvalidFormat(String),
}

pub fn load_ink_doc_from_path(path: impl AsRef<Path>) -> Result<InkDoc, InkJsonError> {
    let raw = fs::read_to_string(path)?;
    load_ink_doc_from_str(&raw)
}

pub fn load_ink_doc_from_str(raw: &str) -> Result<InkDoc, InkJsonError> {
    let v: Value = serde_json::from_str(raw)?;
    parse_top_level(v)
}

fn parse_top_level(v: Value) -> Result<InkDoc, InkJsonError> {
    let obj = v
        .as_object()
        .ok_or_else(|| InkJsonError::InvalidFormat("top level must be object".into()))?;

    let ink_version = obj
        .get("inkVersion")
        .and_then(Value::as_i64)
        .ok_or_else(|| InkJsonError::InvalidFormat("missing inkVersion".into()))?
        as i32;

    let root_v = obj
        .get("root")
        .ok_or_else(|| InkJsonError::InvalidFormat("missing root".into()))?;

    let root = parse_container(root_v)?;

    Ok(InkDoc { ink_version, root })
}

fn parse_container(v: &Value) -> Result<Container, InkJsonError> {
    let arr = v
        .as_array()
        .ok_or_else(|| InkJsonError::InvalidFormat("container must be array".into()))?;

    if arr.is_empty() {
        return Err(InkJsonError::InvalidFormat(
            "container array must not be empty".into(),
        ));
    }

    let mut content: Vec<RuntimeNode> = Vec::new();
    let mut named: HashMap<String, RuntimeNode> = HashMap::new();
    let mut name: Option<String> = None;
    let mut flags: Option<i32> = None;

    // 最后一项是“终结元数据对象或 null”
    let (body, tail) = arr.split_at(arr.len() - 1);

    for item in body {
        content.push(parse_node(item)?);
    }

    let tail_item = &tail[0];
    if !tail_item.is_null() {
        let tail_obj = tail_item.as_object().ok_or_else(|| {
            InkJsonError::InvalidFormat("container tail must be object or null".into())
        })?;

        for (k, v) in tail_obj {
            match k.as_str() {
                "#f" => {
                    flags = v.as_i64().map(|f| f as i32);
                }
                "#n" => {
                    name = v.as_str().map(ToString::to_string);
                }
                _ => {
                    named.insert(k.clone(), parse_node(v)?);
                }
            }
        }
    }

    Ok(Container {
        content,
        named,
        name,
        flags,
    })
}

fn parse_node(v: &Value) -> Result<RuntimeNode, InkJsonError> {
    match v {
        Value::Null => Ok(RuntimeNode::Null),
        Value::Bool(b) => Ok(RuntimeNode::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(RuntimeNode::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(RuntimeNode::Float(f))
            } else {
                Err(InkJsonError::InvalidFormat(format!(
                    "unsupported number format: {n}"
                )))
            }
        }
        Value::String(s) => {
            if s == "\n" {
                return Ok(RuntimeNode::Newline);
            }

            if let Some(stripped) = s.strip_prefix('^') {
                return Ok(RuntimeNode::Str(stripped.to_string()));
            }

            if let Some(cmd) = ControlCommandKind::from_token(s) {
                return Ok(RuntimeNode::ControlCommand(cmd));
            }

            Ok(RuntimeNode::Unsupported(UnsupportedNode {
                raw: s.clone(),
                reason: UnsupportedNodeReason::UnknownStringToken,
            }))
        }
        Value::Array(_) => Ok(RuntimeNode::Container(parse_container(v)?)),
        Value::Object(map) => {
            if let Some(target) = map.get("^->").and_then(Value::as_str) {
                return Ok(RuntimeNode::DivertTarget(target.to_string()));
            }

            if let Some(divert) = parse_divert_object(map) {
                return Ok(RuntimeNode::Divert(divert));
            }

            if let Some(path) = map.get("*").and_then(Value::as_str) {
                let flags = map.get("flg").and_then(Value::as_i64).unwrap_or_default() as i32;
                return Ok(RuntimeNode::ChoicePoint(ChoicePoint {
                    path: path.to_string(),
                    flags,
                }));
            }

            if let Some(name) = map.get("VAR?").and_then(Value::as_str) {
                return Ok(RuntimeNode::VariableReference(VariableReference {
                    name: Some(name.to_string()),
                    read_count_path: None,
                }));
            }

            if let Some(path) = map.get("CNT?").and_then(Value::as_str) {
                return Ok(RuntimeNode::VariableReference(VariableReference {
                    name: None,
                    read_count_path: Some(path.to_string()),
                }));
            }

            if let Some(name) = map.get("VAR=").and_then(Value::as_str) {
                let is_new_declaration = !map.get("re").and_then(Value::as_bool).unwrap_or(false);
                return Ok(RuntimeNode::VariableAssignment(VariableAssignment {
                    name: name.to_string(),
                    is_global: true,
                    is_new_declaration,
                }));
            }

            if let Some(name) = map.get("temp=").and_then(Value::as_str) {
                let is_new_declaration = !map.get("re").and_then(Value::as_bool).unwrap_or(false);
                return Ok(RuntimeNode::VariableAssignment(VariableAssignment {
                    name: name.to_string(),
                    is_global: false,
                    is_new_declaration,
                }));
            }

            Ok(RuntimeNode::Unsupported(UnsupportedNode {
                raw: v.to_string(),
                reason: UnsupportedNodeReason::UnsupportedObject,
            }))
        }
    }
}

fn parse_divert_object(map: &serde_json::Map<String, Value>) -> Option<Divert> {
    let (target_key, target) = if let Some(target) = map.get("->").and_then(Value::as_str) {
        ("->", target)
    } else if let Some(target) = map.get("f()").and_then(Value::as_str) {
        ("f()", target)
    } else if let Some(target) = map.get("->t->").and_then(Value::as_str) {
        ("->t->", target)
    } else if let Some(target) = map.get("x()").and_then(Value::as_str) {
        ("x()", target)
    } else {
        return None;
    };

    let (pushes_to_stack, stack_push_type, is_external) = match target_key {
        "->" => (false, None, false),
        "f()" => (true, Some(PushPopType::Function), false),
        "->t->" => (true, Some(PushPopType::Tunnel), false),
        "x()" => (false, None, true),
        _ => (false, None, false),
    };

    let is_variable_target = map.get("var").and_then(Value::as_bool).unwrap_or(false);
    let is_conditional = map.get("c").and_then(Value::as_bool).unwrap_or(false);
    let external_args = map.get("exArgs").and_then(Value::as_u64).unwrap_or(0) as usize;

    Some(Divert {
        target: target.to_string(),
        is_variable_target,
        pushes_to_stack,
        stack_push_type,
        is_external,
        external_args,
        is_conditional,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_control_command_and_text() {
        let raw = r#"{
            "inkVersion": 21,
            "root": ["^hello", "\n", "done", null]
        }"#;

        let doc = load_ink_doc_from_str(raw).expect("json should parse");
        assert_eq!(doc.ink_version, 21);
        assert!(matches!(doc.root.content[0], RuntimeNode::Str(_)));
        assert!(matches!(doc.root.content[1], RuntimeNode::Newline));
        assert!(matches!(
            doc.root.content[2],
            RuntimeNode::ControlCommand(ControlCommandKind::Done)
        ));
    }

    #[test]
    fn parse_divert_choice_and_vars() {
        let raw = r#"{
            "inkVersion": 21,
            "root": [
                {"->": "knot"},
                {"*": "choice.path", "flg": 16},
                {"VAR=": "x", "re": true},
                {"VAR?": "x"},
                null
            ]
        }"#;

        let doc = load_ink_doc_from_str(raw).expect("json should parse");

        assert!(matches!(doc.root.content[0], RuntimeNode::Divert(_)));

        let choice = match &doc.root.content[1] {
            RuntimeNode::ChoicePoint(c) => c,
            _ => panic!("expected choice point"),
        };
        assert_eq!(choice.path, "choice.path");
        assert_eq!(choice.flags, 16);

        let ass = match &doc.root.content[2] {
            RuntimeNode::VariableAssignment(v) => v,
            _ => panic!("expected assignment"),
        };
        assert!(!ass.is_new_declaration);

        let vr = match &doc.root.content[3] {
            RuntimeNode::VariableReference(v) => v,
            _ => panic!("expected variable reference"),
        };
        assert_eq!(vr.name.as_deref(), Some("x"));
    }

    #[test]
    fn unknown_string_token_becomes_unsupported_node() {
        let raw = r#"{
            "inkVersion": 21,
            "root": ["UNKNOWN_PHASE1_TOKEN", null]
        }"#;

        let doc = load_ink_doc_from_str(raw).expect("json should parse");

        assert!(matches!(
            doc.root.content[0],
            RuntimeNode::Unsupported(UnsupportedNode {
                reason: UnsupportedNodeReason::UnknownStringToken,
                ..
            })
        ));
    }
}
