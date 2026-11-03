use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub count: u64,
    pub length: u64,
    pub size: u64,
    pub optional_count: Option<u64>,
    pub counts: Vec<u64>,
    pub word: usize,
    pub signed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum Format {
    Stats(Stats),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub format: Format,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub body: Body,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Body {
    pub payload: Option<Payload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Payload {
    Segments(Vec<Piece>),
    Kinds(Vec<PieceKind>),
    Scalar(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub total: u64,
    pub word: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PieceKind {
    Text { total: u64, word: usize },
}

pub fn stats() -> Envelope {
    todo!()
}
pub fn total() -> u64 {
    todo!()
}
pub fn word() -> usize {
    todo!()
}
pub fn signed() -> i64 {
    todo!()
}
