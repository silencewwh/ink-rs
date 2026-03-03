use std::{collections::HashMap, fs, path::Path};

use ink_model::{Container, InkDoc, RuntimeNode};
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

            // 目前把其他字符串先视为 command token，后续再扩展到强类型命令枚举
            Ok(RuntimeNode::Command(s.clone()))
        }
        Value::Array(_) => Ok(RuntimeNode::Container(parse_container(v)?)),
        Value::Object(map) => {
            if let Some(target) = map.get("^->").and_then(Value::as_str) {
                return Ok(RuntimeNode::DivertTarget(target.to_string()));
            }

            // 对常见 divert 对象先做占位保留，后续 runtime 逐步实现
            if map.contains_key("->")
                || map.contains_key("f()")
                || map.contains_key("->t->")
                || map.contains_key("x()")
                || map.contains_key("*")
                || map.contains_key("VAR=")
                || map.contains_key("temp=")
                || map.contains_key("VAR?")
                || map.contains_key("CNT?")
            {
                return Ok(RuntimeNode::UnknownJson(v.to_string()));
            }

            // 如果对象结构像容器尾部 named content，理论上应该已经在 parse_container 中处理
            // 这里兜底保留，避免 silent drop
            Ok(RuntimeNode::UnknownJson(v.to_string()))
        }
    }
}
