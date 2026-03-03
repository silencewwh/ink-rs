use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CliEvent {
    CompileSuccess {
        #[serde(rename = "compile-success")]
        compile_success: bool,
    },
    ExportComplete {
        #[serde(rename = "export-complete")]
        export_complete: bool,
    },
    Issues {
        issues: Vec<String>,
    },
    Text {
        text: String,
    },
    Tags {
        tags: Vec<String>,
    },
    Choices {
        choices: Vec<ChoiceItem>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceItem {
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_count: Option<usize>,
}

impl ChoiceItem {
    pub fn new(text: impl Into<String>, tags: Vec<String>) -> Self {
        let tag_count = if tags.is_empty() {
            None
        } else {
            Some(tags.len())
        };
        Self {
            text: text.into(),
            tags,
            tag_count,
        }
    }
}

pub fn write_event_json_line(mut writer: impl Write, event: &CliEvent) -> std::io::Result<()> {
    let raw = serde_json::to_vec(event)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;
    writer.write_all(&raw)?;
    writer.write_all(b"\n")?;
    Ok(())
}
