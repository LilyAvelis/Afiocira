use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Inspect,
    Preview(usize),
    Undo,
    Redo,
    Commit,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub path: PathBuf,
    pub versions: Vec<HistoryNode>,
    pub cursor: Option<usize>,
    pub can_undo: bool,
    pub can_redo: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryNode {
    pub path: PathBuf,
    pub bytes: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryPanelState {
    #[serde(default)]
    pub thumbnail_indices: Vec<usize>,
    pub open: bool,
    pub snapshot: Option<HistorySnapshot>,
    pub preview_index: Option<usize>,
    pub preview_ready: bool,
    pub error: Option<String>,
}
