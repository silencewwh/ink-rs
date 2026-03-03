use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct InkDoc {
    pub ink_version: i32,
    pub root: Container,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Container {
    pub content: Vec<RuntimeNode>,
    pub named: HashMap<String, RuntimeNode>,
    pub name: Option<String>,
    pub flags: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PushPopType {
    Tunnel,
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlCommandKind {
    EvalStart,
    EvalOutput,
    EvalEnd,
    Duplicate,
    PopEvaluatedValue,
    PopFunction,
    PopTunnel,
    BeginString,
    EndString,
    NoOp,
    ChoiceCount,
    Turns,
    TurnsSince,
    ReadCount,
    Random,
    SeedRandom,
    VisitIndex,
    SequenceShuffleIndex,
    StartThread,
    Done,
    End,
    ListFromInt,
    ListRange,
    ListRandom,
    BeginTag,
    EndTag,
}

impl ControlCommandKind {
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "ev" => Self::EvalStart,
            "out" => Self::EvalOutput,
            "/ev" => Self::EvalEnd,
            "du" => Self::Duplicate,
            "pop" => Self::PopEvaluatedValue,
            "~ret" => Self::PopFunction,
            "->->" => Self::PopTunnel,
            "str" => Self::BeginString,
            "/str" => Self::EndString,
            "nop" => Self::NoOp,
            "choiceCnt" => Self::ChoiceCount,
            "turn" => Self::Turns,
            "turns" => Self::TurnsSince,
            "readc" => Self::ReadCount,
            "rnd" => Self::Random,
            "srnd" => Self::SeedRandom,
            "visit" => Self::VisitIndex,
            "seq" => Self::SequenceShuffleIndex,
            "thread" => Self::StartThread,
            "done" => Self::Done,
            "end" => Self::End,
            "listInt" => Self::ListFromInt,
            "range" => Self::ListRange,
            "lrnd" => Self::ListRandom,
            "#" => Self::BeginTag,
            "/#" => Self::EndTag,
            _ => return None,
        })
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::EvalStart => "ev",
            Self::EvalOutput => "out",
            Self::EvalEnd => "/ev",
            Self::Duplicate => "du",
            Self::PopEvaluatedValue => "pop",
            Self::PopFunction => "~ret",
            Self::PopTunnel => "->->",
            Self::BeginString => "str",
            Self::EndString => "/str",
            Self::NoOp => "nop",
            Self::ChoiceCount => "choiceCnt",
            Self::Turns => "turn",
            Self::TurnsSince => "turns",
            Self::ReadCount => "readc",
            Self::Random => "rnd",
            Self::SeedRandom => "srnd",
            Self::VisitIndex => "visit",
            Self::SequenceShuffleIndex => "seq",
            Self::StartThread => "thread",
            Self::Done => "done",
            Self::End => "end",
            Self::ListFromInt => "listInt",
            Self::ListRange => "range",
            Self::ListRandom => "lrnd",
            Self::BeginTag => "#",
            Self::EndTag => "/#",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divert {
    pub target: String,
    pub is_variable_target: bool,
    pub pushes_to_stack: bool,
    pub stack_push_type: Option<PushPopType>,
    pub is_external: bool,
    pub external_args: usize,
    pub is_conditional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoicePoint {
    pub path: String,
    pub flags: i32,
}

impl ChoicePoint {
    pub fn has_condition(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn has_start_content(&self) -> bool {
        self.flags & 2 != 0
    }

    pub fn has_choice_only_content(&self) -> bool {
        self.flags & 4 != 0
    }

    pub fn is_invisible_default(&self) -> bool {
        self.flags & 8 != 0
    }

    pub fn once_only(&self) -> bool {
        self.flags & 16 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableReference {
    pub name: Option<String>,
    pub read_count_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableAssignment {
    pub name: String,
    pub is_global: bool,
    pub is_new_declaration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedNodeReason {
    UnknownStringToken,
    UnsupportedObject,
    UnsupportedValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedNode {
    pub raw: String,
    pub reason: UnsupportedNodeReason,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeNode {
    Str(String),
    Newline,
    Int(i64),
    Float(f64),
    Bool(bool),
    Container(Container),
    ControlCommand(ControlCommandKind),
    Divert(Divert),
    ChoicePoint(ChoicePoint),
    VariableReference(VariableReference),
    VariableAssignment(VariableAssignment),
    DivertTarget(String),
    Unsupported(UnsupportedNode),
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
