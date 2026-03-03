use std::collections::{HashMap, HashSet};

use ink_model::{
    ChoicePoint as ModelChoicePoint, Container, ControlCommandKind, Divert as ModelDivert, InkDoc,
    PushPopType, RuntimeNode,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub text: String,
    pub tags: Vec<String>,
    pub target_path: String,
    pub target_index: usize,
    pub is_invisible_default: bool,
}

#[derive(Debug, Clone)]
struct ProgramNode {
    path: String,
    node: RuntimeNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallFrame {
    push_type: PushPopType,
    return_index: usize,
}

#[derive(Debug, Clone)]
struct FlowState {
    cursor: usize,
    done: bool,
    callstack: Vec<CallFrame>,
    current_choices: Vec<Choice>,
}

impl FlowState {
    fn new() -> Self {
        Self {
            cursor: 0,
            done: false,
            callstack: Vec::new(),
            current_choices: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct Story {
    program: Vec<ProgramNode>,
    path_to_index: HashMap<String, usize>,

    current_flow_name: String,
    current_flow: FlowState,
    other_flows: HashMap<String, FlowState>,

    choice_taken_counts: HashMap<String, u32>,
    visit_counts: HashMap<String, u32>,
    turn_index: u64,

    pub current_text: String,
    warnings: Vec<String>,
    warning_set: HashSet<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum StoryError {
    #[error("invalid choice index: {0}")]
    InvalidChoiceIndex(usize),
    #[error("state json error: {0}")]
    StateJson(#[from] serde_json::Error),
    #[error("invalid state: {0}")]
    InvalidState(String),
}

#[derive(Debug, Clone, Copy)]
enum StepSignal {
    Output,
    ChoiceGenerated,
    NoOutput,
    Done,
}

impl Story {
    pub fn from_doc(doc: InkDoc) -> Self {
        let (program, path_to_index) = build_program(&doc.root);

        Self {
            program,
            path_to_index,
            current_flow_name: "default".to_string(),
            current_flow: FlowState::new(),
            other_flows: HashMap::new(),
            choice_taken_counts: HashMap::new(),
            visit_counts: HashMap::new(),
            turn_index: 0,
            current_text: String::new(),
            warnings: Vec::new(),
            warning_set: HashSet::new(),
        }
    }

    pub fn can_continue(&self) -> bool {
        !self.current_flow.done
            && self.current_flow.current_choices.is_empty()
            && self.current_flow.cursor < self.program.len()
    }

    pub fn continue_line(&mut self) -> String {
        self.current_text.clear();

        let mut safety_counter = 0usize;
        let safety_limit = self.program.len().saturating_mul(4).max(32);

        while self.can_continue() {
            safety_counter += 1;
            if safety_counter > safety_limit {
                self.push_warning(
                    "continue_line safety break hit (possible infinite loop)".to_string(),
                );
                break;
            }

            match self.step() {
                StepSignal::Output => {
                    if self.current_text.ends_with('\n') {
                        break;
                    }
                }
                StepSignal::ChoiceGenerated | StepSignal::Done => {
                    break;
                }
                StepSignal::NoOutput => {
                    if !self.can_continue() {
                        break;
                    }
                }
            }
        }

        self.current_text.clone()
    }

    pub fn continue_maximally(&mut self) -> String {
        let mut out = String::new();

        let mut safety_counter = 0usize;
        let safety_limit = self.program.len().saturating_mul(8).max(64);

        while self.can_continue() {
            let before_cursor = self.current_flow.cursor;
            let before_choices = self.current_flow.current_choices.len();

            let line = self.continue_line();
            out.push_str(&line);

            safety_counter += 1;
            if safety_counter > safety_limit {
                self.push_warning(
                    "continue_maximally safety break hit (possible infinite loop)".to_string(),
                );
                break;
            }

            if line.is_empty()
                && before_cursor == self.current_flow.cursor
                && before_choices == self.current_flow.current_choices.len()
            {
                self.push_warning("continue_maximally made no progress, aborting".to_string());
                break;
            }
        }

        out
    }

    pub fn current_choices(&self) -> &[Choice] {
        &self.current_flow.current_choices
    }

    pub fn choose_choice_index(&mut self, index: usize) -> Result<(), StoryError> {
        if index >= self.current_flow.current_choices.len() {
            return Err(StoryError::InvalidChoiceIndex(index));
        }

        let choice = self.current_flow.current_choices[index].clone();
        self.current_flow.current_choices.clear();

        if choice.target_index >= self.program.len() {
            return Err(StoryError::InvalidState(format!(
                "choice target index out of bounds: {}",
                choice.target_index
            )));
        }

        self.current_flow.cursor = choice.target_index;
        self.current_flow.done = false;
        self.turn_index = self.turn_index.saturating_add(1);

        let taken = self
            .choice_taken_counts
            .entry(choice.target_path)
            .or_insert(0);
        *taken = taken.saturating_add(1);

        Ok(())
    }

    pub fn switch_flow(&mut self, flow_name: &str) {
        if flow_name == self.current_flow_name {
            return;
        }

        let old_name = std::mem::replace(&mut self.current_flow_name, flow_name.to_string());
        let old_state = std::mem::replace(&mut self.current_flow, FlowState::new());
        self.other_flows.insert(old_name, old_state);

        if let Some(next_state) = self.other_flows.remove(flow_name) {
            self.current_flow = next_state;
        }
    }

    pub fn current_flow_name(&self) -> &str {
        &self.current_flow_name
    }

    pub fn save_json(&self) -> Result<String, StoryError> {
        let mut other_flows = HashMap::new();
        for (name, flow) in &self.other_flows {
            other_flows.insert(name.clone(), FlowSnapshot::from(flow));
        }

        let snapshot = StorySnapshot {
            current_flow_name: self.current_flow_name.clone(),
            current_flow: FlowSnapshot::from(&self.current_flow),
            other_flows,
            choice_taken_counts: self.choice_taken_counts.clone(),
            visit_counts: self.visit_counts.clone(),
            turn_index: self.turn_index,
        };

        Ok(serde_json::to_string_pretty(&snapshot)?)
    }

    pub fn load_json(&mut self, raw: &str) -> Result<(), StoryError> {
        let snapshot: StorySnapshot = serde_json::from_str(raw)?;

        let current_flow = FlowState::try_from(snapshot.current_flow)?;
        self.validate_flow_state(&current_flow)?;

        let mut other_flows = HashMap::new();
        for (name, flow_snapshot) in snapshot.other_flows {
            let flow = FlowState::try_from(flow_snapshot)?;
            self.validate_flow_state(&flow)?;
            other_flows.insert(name, flow);
        }

        self.current_flow_name = if snapshot.current_flow_name.is_empty() {
            "default".to_string()
        } else {
            snapshot.current_flow_name
        };
        self.current_flow = current_flow;
        self.other_flows = other_flows;
        self.choice_taken_counts = snapshot.choice_taken_counts;
        self.visit_counts = snapshot.visit_counts;
        self.turn_index = snapshot.turn_index;

        Ok(())
    }

    pub fn take_warnings(&mut self) -> Vec<String> {
        self.warning_set.clear();
        std::mem::take(&mut self.warnings)
    }

    fn step(&mut self) -> StepSignal {
        if self.current_flow.done || self.current_flow.cursor >= self.program.len() {
            return StepSignal::Done;
        }

        let index = self.current_flow.cursor;
        let program_node = self.program[index].clone();

        self.current_flow.cursor = self.current_flow.cursor.saturating_add(1);
        self.bump_visit_count(&program_node.path);

        match program_node.node {
            RuntimeNode::Str(s) => {
                self.current_text.push_str(&s);
                StepSignal::Output
            }
            RuntimeNode::Newline => {
                self.current_text.push('\n');
                StepSignal::Output
            }
            RuntimeNode::ControlCommand(cmd) => self.apply_control_command(cmd),
            RuntimeNode::Divert(divert) => {
                self.apply_divert(divert, &program_node.path);
                StepSignal::NoOutput
            }
            RuntimeNode::ChoicePoint(choice_point) => {
                self.collect_choice(choice_point, &program_node.path);
                self.consume_following_choice_points();
                if self.current_flow.current_choices.is_empty() {
                    StepSignal::NoOutput
                } else {
                    StepSignal::ChoiceGenerated
                }
            }
            RuntimeNode::VariableReference(var_ref) => {
                self.push_warning(format!(
                    "unimplemented variable reference encountered: name={:?}, read_count_path={:?}",
                    var_ref.name, var_ref.read_count_path
                ));
                StepSignal::NoOutput
            }
            RuntimeNode::VariableAssignment(var_ass) => {
                self.push_warning(format!(
                    "unimplemented variable assignment encountered: name={}, is_global={}, is_new_declaration={}",
                    var_ass.name, var_ass.is_global, var_ass.is_new_declaration
                ));
                StepSignal::NoOutput
            }
            RuntimeNode::Unsupported(node) => {
                self.push_warning(format!(
                    "unsupported runtime node encountered: reason={:?}, raw={}",
                    node.reason, node.raw
                ));
                StepSignal::NoOutput
            }
            RuntimeNode::Container(_) => {
                self.push_warning(
                    "unexpected nested container at runtime (should be flattened)".to_string(),
                );
                StepSignal::NoOutput
            }
            RuntimeNode::Null => {
                self.push_warning("unimplemented runtime null token encountered".to_string());
                StepSignal::NoOutput
            }
            RuntimeNode::Int(v) => {
                self.push_warning(format!("unimplemented runtime int token encountered: {v}"));
                StepSignal::NoOutput
            }
            RuntimeNode::Float(v) => {
                self.push_warning(format!("unimplemented runtime float token encountered: {v}"));
                StepSignal::NoOutput
            }
            RuntimeNode::Bool(v) => {
                self.push_warning(format!("unimplemented runtime bool token encountered: {v}"));
                StepSignal::NoOutput
            }
            RuntimeNode::DivertTarget(path) => {
                self.push_warning(format!(
                    "unimplemented divert target value encountered: {path}"
                ));
                StepSignal::NoOutput
            }
        }
    }

    fn apply_control_command(&mut self, cmd: ControlCommandKind) -> StepSignal {
        match cmd {
            ControlCommandKind::Done | ControlCommandKind::End => {
                self.current_flow.done = true;
                StepSignal::Done
            }
            ControlCommandKind::NoOp
            | ControlCommandKind::EvalStart
            | ControlCommandKind::EvalOutput
            | ControlCommandKind::EvalEnd
            | ControlCommandKind::BeginString
            | ControlCommandKind::EndString
            | ControlCommandKind::BeginTag
            | ControlCommandKind::EndTag => StepSignal::NoOutput,
            ControlCommandKind::PopFunction => {
                self.pop_callstack(PushPopType::Function);
                StepSignal::NoOutput
            }
            ControlCommandKind::PopTunnel => {
                self.pop_callstack(PushPopType::Tunnel);
                StepSignal::NoOutput
            }
            ControlCommandKind::StartThread => {
                self.push_warning("unimplemented StartThread control command encountered".to_string());
                StepSignal::NoOutput
            }
            other => {
                self.push_warning(format!(
                    "unimplemented control command encountered: {}",
                    other.token()
                ));
                StepSignal::NoOutput
            }
        }
    }

    fn apply_divert(&mut self, divert: ModelDivert, current_node_path: &str) {
        if divert.is_external {
            self.push_warning(format!(
                "unimplemented external divert encountered: {}",
                divert.target
            ));
            return;
        }

        if divert.is_variable_target {
            self.push_warning(format!(
                "unimplemented variable divert encountered: {}",
                divert.target
            ));
            return;
        }

        if divert.is_conditional {
            self.push_warning(format!(
                "divert conditional check is not implemented, falling through as taken: {}",
                divert.target
            ));
        }

        let Some(target_index) = self.resolve_path(&divert.target, Some(current_node_path)) else {
            self.push_warning(format!("failed to resolve divert target path: {}", divert.target));
            self.current_flow.done = true;
            return;
        };

        if divert.pushes_to_stack {
            let push_type = divert.stack_push_type.unwrap_or(PushPopType::Function);
            let return_index = self.current_flow.cursor;
            self.current_flow.callstack.push(CallFrame {
                push_type,
                return_index,
            });
        }

        self.current_flow.cursor = target_index;
    }

    fn collect_choice(&mut self, choice_point: ModelChoicePoint, current_node_path: &str) {
        if choice_point.has_condition() {
            self.push_warning(format!(
                "choice with condition is not fully implemented: {}",
                choice_point.path
            ));
        }

        if choice_point.has_start_content() || choice_point.has_choice_only_content() {
            self.push_warning(format!(
                "choice with dynamic start/choice-only content is not fully implemented: {}",
                choice_point.path
            ));
        }

        let Some(target_index) = self.resolve_path(&choice_point.path, Some(current_node_path)) else {
            self.push_warning(format!(
                "failed to resolve choice target path: {}",
                choice_point.path
            ));
            return;
        };

        let canonical_target_path = self.program[target_index].path.clone();

        if choice_point.once_only()
            && self
                .choice_taken_counts
                .get(&canonical_target_path)
                .copied()
                .unwrap_or_default()
                > 0
        {
            return;
        }

        if self
            .current_flow
            .current_choices
            .iter()
            .any(|c| c.target_index == target_index)
        {
            return;
        }

        let choice_text = choice_point.path.clone();
        let is_invisible_default = choice_point.is_invisible_default();

        self.current_flow.current_choices.push(Choice {
            text: choice_text,
            tags: Vec::new(),
            target_path: canonical_target_path,
            target_index,
            is_invisible_default,
        });
    }

    fn consume_following_choice_points(&mut self) {
        while self.current_flow.cursor < self.program.len() {
            let next_index = self.current_flow.cursor;
            let next_program_node = self.program[next_index].clone();

            let RuntimeNode::ChoicePoint(choice_point) = next_program_node.node else {
                break;
            };

            self.current_flow.cursor = self.current_flow.cursor.saturating_add(1);
            self.bump_visit_count(&next_program_node.path);
            self.collect_choice(choice_point, &next_program_node.path);
        }
    }

    fn pop_callstack(&mut self, expected: PushPopType) {
        let Some(frame) = self.current_flow.callstack.pop() else {
            self.push_warning(format!(
                "callstack underflow when attempting to pop {:?}",
                expected
            ));
            self.current_flow.done = true;
            return;
        };

        if frame.push_type != expected {
            self.push_warning(format!(
                "mismatched callstack pop: expected {:?}, found {:?}",
                expected, frame.push_type
            ));
            self.current_flow.done = true;
            return;
        }

        self.current_flow.cursor = frame.return_index.min(self.program.len());
    }

    fn resolve_path(&self, raw_path: &str, current_node_path: Option<&str>) -> Option<usize> {
        let target = raw_path.trim();
        if target.is_empty() {
            return None;
        }

        if let Some(idx) = self.path_to_index.get(target) {
            return Some(*idx);
        }

        if target.contains('^') {
            if let Some(current) = current_node_path {
                if let Some(relative_idx) = self.resolve_relative_path(target, current) {
                    return Some(relative_idx);
                }
            }
        }

        if let Some(current) = current_node_path {
            let mut scope = parent_path(current);
            while let Some(parent) = scope {
                let candidate = join_path(&parent, target);
                if let Some(idx) = self.path_to_index.get(&candidate) {
                    return Some(*idx);
                }
                scope = parent_path(&parent);
            }
        }

        let suffix = format!(".{target}");
        let mut matches = self
            .path_to_index
            .iter()
            .filter(|(path, _)| path.as_str() == target || path.ends_with(&suffix))
            .map(|(_, idx)| *idx);

        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }

        Some(first)
    }

    fn resolve_relative_path(&self, raw_path: &str, current_node_path: &str) -> Option<usize> {
        let mut components: Vec<String> = parent_path(current_node_path)
            .map(|p| {
                p.split('.')
                    .filter(|c| !c.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();

        for comp in raw_path.split('.') {
            if comp.is_empty() {
                continue;
            }

            if comp == "^" {
                components.pop();
            } else {
                components.push(comp.to_string());
            }
        }

        let candidate = components.join(".");
        if candidate.is_empty() {
            return None;
        }

        self.path_to_index.get(&candidate).copied()
    }

    fn validate_flow_state(&self, flow: &FlowState) -> Result<(), StoryError> {
        if flow.cursor > self.program.len() {
            return Err(StoryError::InvalidState(format!(
                "flow cursor out of bounds: {} > {}",
                flow.cursor,
                self.program.len()
            )));
        }

        for frame in &flow.callstack {
            if frame.return_index > self.program.len() {
                return Err(StoryError::InvalidState(format!(
                    "callstack return index out of bounds: {} > {}",
                    frame.return_index,
                    self.program.len()
                )));
            }
        }

        for choice in &flow.current_choices {
            if choice.target_index >= self.program.len() {
                return Err(StoryError::InvalidState(format!(
                    "choice target index out of bounds: {} >= {}",
                    choice.target_index,
                    self.program.len()
                )));
            }
        }

        Ok(())
    }

    fn bump_visit_count(&mut self, path: &str) {
        let count = self.visit_counts.entry(path.to_string()).or_insert(0);
        *count = count.saturating_add(1);
    }

    fn push_warning(&mut self, warning: String) {
        if self.warning_set.insert(warning.clone()) {
            self.warnings.push(warning);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StorySnapshot {
    current_flow_name: String,
    current_flow: FlowSnapshot,
    other_flows: HashMap<String, FlowSnapshot>,
    choice_taken_counts: HashMap<String, u32>,
    visit_counts: HashMap<String, u32>,
    turn_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FlowSnapshot {
    cursor: usize,
    done: bool,
    callstack: Vec<CallFrameSnapshot>,
    current_choices: Vec<Choice>,
}

impl From<&FlowState> for FlowSnapshot {
    fn from(value: &FlowState) -> Self {
        Self {
            cursor: value.cursor,
            done: value.done,
            callstack: value.callstack.iter().map(CallFrameSnapshot::from).collect(),
            current_choices: value.current_choices.clone(),
        }
    }
}

impl TryFrom<FlowSnapshot> for FlowState {
    type Error = StoryError;

    fn try_from(value: FlowSnapshot) -> Result<Self, Self::Error> {
        let mut callstack = Vec::with_capacity(value.callstack.len());
        for frame in value.callstack {
            callstack.push(CallFrame::try_from(frame)?);
        }

        Ok(Self {
            cursor: value.cursor,
            done: value.done,
            callstack,
            current_choices: value.current_choices,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CallFrameSnapshot {
    push_type: String,
    return_index: usize,
}

impl From<&CallFrame> for CallFrameSnapshot {
    fn from(value: &CallFrame) -> Self {
        let push_type = match value.push_type {
            PushPopType::Tunnel => "tunnel",
            PushPopType::Function => "function",
        }
        .to_string();

        Self {
            push_type,
            return_index: value.return_index,
        }
    }
}

impl TryFrom<CallFrameSnapshot> for CallFrame {
    type Error = StoryError;

    fn try_from(value: CallFrameSnapshot) -> Result<Self, Self::Error> {
        let push_type = match value.push_type.as_str() {
            "tunnel" => PushPopType::Tunnel,
            "function" => PushPopType::Function,
            other => {
                return Err(StoryError::InvalidState(format!(
                    "unknown call frame type: {other}"
                )));
            }
        };

        Ok(Self {
            push_type,
            return_index: value.return_index,
        })
    }
}

fn build_program(root: &Container) -> (Vec<ProgramNode>, HashMap<String, usize>) {
    let mut program = Vec::new();
    let mut path_to_index = HashMap::new();

    flatten_container(root, "", &mut program, &mut path_to_index);

    (program, path_to_index)
}

fn flatten_container(
    container: &Container,
    base_path: &str,
    out: &mut Vec<ProgramNode>,
    path_to_index: &mut HashMap<String, usize>,
) {
    if !base_path.is_empty() {
        register_path(path_to_index, base_path, out.len());
    }

    for (idx, node) in container.content.iter().enumerate() {
        match node {
            RuntimeNode::Container(child) => {
                let segment = child.name.clone().unwrap_or_else(|| idx.to_string());
                let child_path = join_path(base_path, &segment);
                flatten_container(child, &child_path, out, path_to_index);
            }
            other => {
                let node_path = join_path(base_path, &idx.to_string());
                let program_index = out.len();
                out.push(ProgramNode {
                    path: node_path.clone(),
                    node: other.clone(),
                });
                register_path(path_to_index, &node_path, program_index);
            }
        }
    }

    if !container.named.is_empty() {
        let mut named_keys: Vec<&str> = container.named.keys().map(String::as_str).collect();
        named_keys.sort_unstable();

        for key in named_keys {
            let named_path = join_path(base_path, key);
            if let Some(named_node) = container.named.get(key) {
                match named_node {
                    RuntimeNode::Container(named_container) => {
                        flatten_container(named_container, &named_path, out, path_to_index);
                    }
                    other => {
                        let program_index = out.len();
                        out.push(ProgramNode {
                            path: named_path.clone(),
                            node: other.clone(),
                        });
                        register_path(path_to_index, &named_path, program_index);
                    }
                }
            }
        }
    }
}

fn register_path(path_to_index: &mut HashMap<String, usize>, path: &str, index: usize) {
    if path.is_empty() {
        return;
    }

    path_to_index.entry(path.to_string()).or_insert(index);
}

fn join_path(base: &str, segment: &str) -> String {
    if base.is_empty() {
        segment.to_string()
    } else {
        format!("{base}.{segment}")
    }
}

fn parent_path(path: &str) -> Option<String> {
    let (parent, _) = path.rsplit_once('.')?;
    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story_from_json(raw: &str) -> Story {
        let doc = ink_json::load_ink_doc_from_str(raw).expect("ink json should parse");
        Story::from_doc(doc)
    }

    #[test]
    fn continue_maximally_outputs_plain_text() {
        let mut story = story_from_json(
            r#"{
                "inkVersion": 21,
                "root": ["^hello", "\n", "^world", "\n", "done", null]
            }"#,
        );

        let out = story.continue_maximally();
        assert_eq!(out, "hello\nworld\n");
        assert!(story.take_warnings().is_empty());
    }

    #[test]
    fn function_divert_and_pop_return_to_caller() {
        let mut story = story_from_json(
            r#"{
                "inkVersion": 21,
                "root": [
                    {"f()": "fn"},
                    "^tail",
                    "\n",
                    "end",
                    {
                        "fn": [
                            "^head",
                            "\n",
                            "~ret",
                            null
                        ]
                    }
                ]
            }"#,
        );

        let out = story.continue_maximally();
        assert_eq!(out, "head\ntail\n");
        assert!(story.take_warnings().is_empty());
    }

    #[test]
    fn choice_collection_and_choose_works() {
        let mut story = story_from_json(
            r#"{
                "inkVersion": 21,
                "root": [
                    {"*": "choiceA", "flg": 16},
                    {"*": "choiceB", "flg": 0},
                    "done",
                    {
                        "choiceA": ["^A", "\n", "end", null],
                        "choiceB": ["^B", "\n", "end", null]
                    }
                ]
            }"#,
        );

        let first_continue = story.continue_maximally();
        assert_eq!(first_continue, "");
        assert_eq!(story.current_choices().len(), 2);

        story
            .choose_choice_index(1)
            .expect("choice index 1 should be valid");

        let after_choose = story.continue_maximally();
        assert_eq!(after_choose, "B\n");
    }

    #[test]
    fn save_and_load_roundtrip_restores_flow_state() {
        let raw = r#"{
            "inkVersion": 21,
            "root": [
                {"*": "choiceA", "flg": 16},
                {"*": "choiceB", "flg": 0},
                "done",
                {
                    "choiceA": ["^A", "\n", "end", null],
                    "choiceB": ["^B", "\n", "end", null]
                }
            ]
        }"#;

        let doc = ink_json::load_ink_doc_from_str(raw).expect("ink json should parse");
        let mut story = Story::from_doc(doc.clone());

        let _ = story.continue_line();
        assert_eq!(story.current_choices().len(), 2);

        let state_json = story.save_json().expect("save should succeed");

        let mut restored = Story::from_doc(doc);
        restored
            .load_json(&state_json)
            .expect("load should succeed");

        assert_eq!(restored.current_choices().len(), 2);
        assert_eq!(restored.current_flow_name(), "default");
    }

    #[test]
    fn switch_flow_keeps_independent_cursors() {
        let mut story = story_from_json(
            r#"{
                "inkVersion": 21,
                "root": ["^a", "\n", "^b", "\n", "done", null]
            }"#,
        );

        let first = story.continue_line();
        assert_eq!(first, "a\n");

        story.switch_flow("alt");
        let alt_first = story.continue_line();
        assert_eq!(alt_first, "a\n");

        story.switch_flow("default");
        let second = story.continue_line();
        assert_eq!(second, "b\n");
    }
}
