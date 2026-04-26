use std::collections::VecDeque;
use std::sync::{mpsc, Arc};

use crate::register::{DebugStdinShared, StdinRequest};
use crate::value::FlatValue;

#[derive(Clone)]
pub struct Snapshot {
    pub step_index: u64,
    pub pc: usize,
    pub registers: Vec<(String, FlatValue)>,
    pub instruction_repr: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DebugCommand {
    StepForward,
    Continue,
    Pause,
    Quit,
}

pub struct NodeUpdate {
    pub node_index: usize,
    pub snapshot: Snapshot,
    pub is_paused: bool,
    pub is_terminated: bool,
}

pub struct NodeDebugState {
    pub name: String,
    pub source_lines: Vec<String>,
    pub source_path: String,
    pub pc_to_source_line: Vec<usize>,
    pub history: VecDeque<Snapshot>,
    pub history_base: u64,
    pub history_cursor: usize,
    pub is_paused: bool,
    pub is_terminated: bool,
    pub has_unseen_update: bool,
    pub cmd_tx: mpsc::SyncSender<DebugCommand>,
    pub history_size: usize,
    pub stdin_shared: Arc<DebugStdinShared>,
}

impl NodeDebugState {
    pub fn new(
        name: String,
        source_lines: Vec<String>,
        source_path: String,
        pc_to_source_line: Vec<usize>,
        cmd_tx: mpsc::SyncSender<DebugCommand>,
        history_size: usize,
        stdin_shared: Arc<DebugStdinShared>,
    ) -> Self {
        Self {
            name,
            source_lines,
            source_path,
            pc_to_source_line,
            history: VecDeque::new(),
            history_base: 0,
            history_cursor: 0,
            is_paused: true,
            is_terminated: false,
            has_unseen_update: false,
            cmd_tx,
            history_size,
            stdin_shared,
        }
    }

    /// Returns a clone of the pending stdin request if the node is blocked on input.
    pub fn stdin_pending(&self) -> Option<StdinRequest> {
        self.stdin_shared.inner.lock().unwrap().pending.clone()
    }

    pub fn push_snapshot(&mut self, snap: Snapshot) {
        if self.history.len() >= self.history_size {
            self.history.pop_front();
            self.history_base += 1;
        }
        self.history.push_back(snap);
        self.history_cursor = self.history.len().saturating_sub(1);
        self.has_unseen_update = true;
    }

    pub fn current_snapshot(&self) -> Option<&Snapshot> {
        self.history.get(self.history_cursor)
    }

    pub fn step_back(&mut self) {
        if self.history_cursor > 0 {
            self.history_cursor -= 1;
        }
    }

    pub fn at_history_head(&self) -> bool {
        self.history.is_empty() || self.history_cursor >= self.history.len() - 1
    }

    pub fn step_forward_in_history(&mut self) -> bool {
        if self.at_history_head() {
            return false;
        }
        self.history_cursor += 1;
        true
    }

    pub fn current_source_line(&self) -> Option<usize> {
        let snap = self.current_snapshot()?;
        self.pc_to_source_line.get(snap.pc).copied()
    }
}
