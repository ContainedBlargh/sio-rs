pub mod commands;
pub mod state;
pub mod tui;

use std::io::{BufRead, BufReader};
use std::sync::{mpsc, Arc};
use std::thread;

use crate::node::DebugBridge;
use crate::parser::parse_from_path_debug;
use crate::register::{DebugOutputShared, DebugStdinShared};
use state::{NodeDebugState, NodeUpdate};
use tui::DebuggerRunner;

/// Metadata sent from node thread back to main thread before debug loop starts.
#[allow(dead_code)]
pub struct NodeMeta {
    pub source_lines: Vec<String>,
    pub pc_to_source_line: Vec<usize>,
    pub pc_to_instr_repr: Vec<String>,
    pub name: String,
    pub stdin_shared: Arc<DebugStdinShared>,
    pub output_shared: Arc<DebugOutputShared>,
    pub error: Option<String>,
}

pub fn run_debug_session(args: Vec<String>, history_size: usize, stdin_path: Option<String>, program_args: Vec<String>) {
    if args.is_empty() {
        eprintln!("debugger: no .sio files provided");
        return;
    }

    let (update_tx, update_rx) = mpsc::channel::<NodeUpdate>();
    let mut node_states: Vec<NodeDebugState> = Vec::new();
    let mut stdin_shareds: Vec<Arc<DebugStdinShared>> = Vec::new();
    let mut output_shareds: Vec<Arc<DebugOutputShared>> = Vec::new();

    for (idx, path) in args.iter().enumerate() {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel(4);
        let (meta_tx, meta_rx) = mpsc::sync_channel::<NodeMeta>(0);

        let path_clone = path.clone();
        let update_tx_clone = update_tx.clone();
        let program_args_clone = program_args.clone();

        thread::spawn(move || {
            match parse_from_path_debug(&path_clone, &program_args_clone) {
                Ok(result) => {
                    let pc_to_instr_repr = result.pc_to_instr_repr.clone();
                    let meta = NodeMeta {
                        source_lines: result.source_lines,
                        pc_to_source_line: result.pc_to_source_line,
                        pc_to_instr_repr: result.pc_to_instr_repr,
                        name: result.node.name.clone(),
                        stdin_shared: result.stdin_shared,
                        output_shared: result.output_shared,
                        error: None,
                    };
                    let _ = meta_tx.send(meta);
                    let bridge = DebugBridge { cmd_rx, update_tx: update_tx_clone, node_index: idx, pc_to_instr_repr };
                    result.node.run_debug(bridge);
                }
                Err(e) => {
                    let _ = meta_tx.send(NodeMeta {
                        source_lines: Vec::new(),
                        pc_to_source_line: Vec::new(),
                        pc_to_instr_repr: Vec::new(),
                        name: path_clone.clone(),
                        stdin_shared: DebugStdinShared::new(),
                        output_shared: DebugOutputShared::new(),
                        error: Some(e),
                    });
                }
            }
        });

        match meta_rx.recv() {
            Ok(meta) => {
                if let Some(ref e) = meta.error {
                    eprintln!("{}", e);
                    continue;
                }
                let state = NodeDebugState::new(
                    meta.name,
                    meta.source_lines,
                    path.clone(),
                    meta.pc_to_source_line,
                    cmd_tx,
                    history_size,
                    Arc::clone(&meta.stdin_shared),
                );
                stdin_shareds.push(meta.stdin_shared);
                output_shareds.push(meta.output_shared);
                node_states.push(state);
            }
            Err(_) => {
                eprintln!("debugger: failed to receive metadata from node thread");
            }
        }
    }

    if node_states.is_empty() {
        eprintln!("debugger: no nodes could be loaded");
        return;
    }

    // If --stdin was given, spawn a background thread that reads from the
    // file (or real stdin if "-") and feeds whichever node is waiting.
    if let Some(ref path) = stdin_path {
        let path = path.clone();
        thread::spawn(move || {
            let reader: Box<dyn BufRead + Send> = if path == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(&path) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("sio-dbg: cannot open stdin file '{}': {}", path, e);
                        return;
                    }
                }
            };
            feed_stdin_from_reader(reader, stdin_shareds);
        });
    }

    let runner = DebuggerRunner::new(node_states, update_rx, update_tx, stdin_path.is_some(), program_args, output_shareds);
    if let Err(e) = runner.run() {
        eprintln!("debugger TUI error: {}", e);
    }
}

/// Reads from `reader` and feeds data to whichever node has a pending stdin
/// request. Mirrors the request semantics of `Register::Stdin`: `Bytes(n)`
/// reads exactly n bytes; `Pattern(pat)` reads until the pattern appears.
fn feed_stdin_from_reader(
    mut reader: Box<dyn BufRead + Send>,
    shareds: Vec<Arc<DebugStdinShared>>,
) {
    use crate::register::StdinRequest;
    use std::io::Read;

    loop {
        // Find the first node with a pending request, blocking until one appears.
        let shared = loop {
            let found = shareds.iter().find(|s| {
                s.inner.lock().unwrap().pending.is_some()
            });
            if let Some(s) = found {
                break Arc::clone(s);
            }
            // No node is waiting yet — yield briefly and check again.
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        let request = {
            let g = shared.inner.lock().unwrap();
            g.pending.clone()
        };

        let text = match request {
            None => continue,
            Some(StdinRequest::Bytes(n)) => {
                let mut buf = vec![0u8; n];
                match reader.read(&mut buf) {
                    Ok(0) => return, // EOF
                    Ok(k) => String::from_utf8_lossy(&buf[..k]).into_owned(),
                    Err(_) => return,
                }
            }
            Some(StdinRequest::Pattern(ref pat)) => {
                // Read byte-by-byte until the pattern appears, same as StdinState::search.
                let pat_bytes = pat.as_bytes().to_vec();
                let pat_len = pat_bytes.len();
                let mut collected = Vec::new();
                let mut tail: std::collections::VecDeque<u8> = std::collections::VecDeque::with_capacity(pat_len);
                let mut one = [0u8; 1];
                loop {
                    match reader.read(&mut one) {
                        Ok(0) | Err(_) => return, // EOF
                        Ok(_) => {
                            collected.push(one[0]);
                            if tail.len() == pat_len { tail.pop_front(); }
                            tail.push_back(one[0]);
                            if tail.len() == pat_len && tail.iter().copied().eq(pat_bytes.iter().copied()) {
                                break;
                            }
                        }
                    }
                }
                String::from_utf8_lossy(&collected).into_owned()
            }
        };

        shared.provide(text);
    }
}

/// Spawn a replacement node thread after a reload.
pub fn spawn_reload_thread(
    path: String,
    node_index: usize,
    cmd_rx: mpsc::Receiver<crate::debugger::state::DebugCommand>,
    update_tx: mpsc::Sender<NodeUpdate>,
    program_args: Vec<String>,
) -> Option<NodeMeta> {
    let (meta_tx, meta_rx) = mpsc::sync_channel::<NodeMeta>(0);

    thread::spawn(move || {
        match parse_from_path_debug(&path, &program_args) {
            Ok(result) => {
                let pc_to_instr_repr = result.pc_to_instr_repr.clone();
                let meta = NodeMeta {
                    source_lines: result.source_lines,
                    pc_to_source_line: result.pc_to_source_line,
                    pc_to_instr_repr: result.pc_to_instr_repr,
                    name: result.node.name.clone(),
                    stdin_shared: result.stdin_shared,
                    output_shared: result.output_shared,
                    error: None,
                };
                let _ = meta_tx.send(meta);
                let bridge = DebugBridge { cmd_rx, update_tx, node_index, pc_to_instr_repr };
                result.node.run_debug(bridge);
            }
            Err(e) => {
                let _ = meta_tx.send(NodeMeta {
                    source_lines: Vec::new(),
                    pc_to_source_line: Vec::new(),
                    pc_to_instr_repr: Vec::new(),
                    name: path.clone(),
                    stdin_shared: DebugStdinShared::new(),
                    output_shared: DebugOutputShared::new(),
                    error: Some(e),
                });
            }
        }
    });

    meta_rx.recv().ok()
}
