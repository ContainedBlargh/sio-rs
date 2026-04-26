use std::collections::VecDeque;
use std::io;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;

use crate::register::DebugOutputShared;
use crate::value::FlatValue;

use super::commands::{key_to_action, Action};
use super::state::{DebugCommand, NodeDebugState, NodeUpdate};

pub struct DebuggerApp {
    pub nodes: Vec<NodeDebugState>,
    pub active_node: usize,
    pub update_rx: mpsc::Receiver<NodeUpdate>,
    pub source_scroll: usize,
    pub register_scroll: usize,
    pub output_scroll: usize,
    pub status_message: Option<String>,
    pub pending_reload: bool,
    pub input_buf: String,
    pub captured_output: VecDeque<(bool, String)>, // (is_err, text)
}

impl DebuggerApp {
    pub fn new(nodes: Vec<NodeDebugState>, update_rx: mpsc::Receiver<NodeUpdate>) -> Self {
        Self {
            nodes,
            active_node: 0,
            update_rx,
            source_scroll: 0,
            register_scroll: 0,
            output_scroll: 0,
            status_message: None,
            pending_reload: false,
            input_buf: String::new(),
            captured_output: VecDeque::new(),
        }
    }

    pub fn stdin_mode(&self) -> bool {
        self.active_node()
            .map(|n| n.stdin_pending().is_some())
            .unwrap_or(false)
    }

    fn drain_updates(&mut self) {
        while let Ok(update) = self.update_rx.try_recv() {
            let idx = update.node_index;
            if let Some(node) = self.nodes.get_mut(idx) {
                node.push_snapshot(update.snapshot);
                node.is_paused = update.is_paused;
                if update.is_terminated {
                    node.is_terminated = true;
                    node.is_paused = true;
                }
                if idx != self.active_node {
                    node.has_unseen_update = true;
                }
            }
        }
    }

    fn active_node(&self) -> Option<&NodeDebugState> {
        self.nodes.get(self.active_node)
    }

    fn active_node_mut(&mut self) -> Option<&mut NodeDebugState> {
        self.nodes.get_mut(self.active_node)
    }

    fn handle_action(&mut self, action: Action) -> bool {
        match action {
            Action::Quit => return true,
            Action::StepForward => {
                let any_at_head = self.nodes.iter()
                    .filter(|n| !n.is_terminated)
                    .any(|n| n.at_history_head());
                if any_at_head {
                    // At least one node is at history head — send StepForward to all
                    // live nodes so they advance together (required for XBus handshakes).
                    // Also advance any nodes that are mid-history so they stay in sync.
                    for node in &mut self.nodes {
                        if node.is_terminated { continue; }
                        if !node.step_forward_in_history() && node.is_paused {
                            let _ = node.cmd_tx.send(DebugCommand::StepForward);
                        }
                    }
                } else {
                    // All live nodes have history ahead — replay locally.
                    for node in &mut self.nodes {
                        if !node.is_terminated {
                            node.step_forward_in_history();
                        }
                    }
                    self.sync_source_scroll();
                }
            }
            Action::StepBack => {
                // Step back is local history replay — move all nodes back one step.
                for node in &mut self.nodes {
                    node.step_back();
                }
                self.sync_source_scroll();
            }
            Action::Continue => {
                for node in &mut self.nodes {
                    if !node.is_terminated {
                        node.is_paused = false;
                        let _ = node.cmd_tx.send(DebugCommand::Continue);
                    }
                }
            }
            Action::Pause => {
                for node in &mut self.nodes {
                    let _ = node.cmd_tx.send(DebugCommand::Pause);
                }
            }
            Action::NextNode => {
                if !self.nodes.is_empty() {
                    self.active_node = (self.active_node + 1) % self.nodes.len();
                    if let Some(n) = self.nodes.get_mut(self.active_node) {
                        n.has_unseen_update = false;
                    }
                    self.source_scroll = 0;
                    self.register_scroll = 0;
                    self.sync_source_scroll();
                }
            }
            Action::PrevNode => {
                if !self.nodes.is_empty() {
                    self.active_node = self
                        .active_node
                        .checked_sub(1)
                        .unwrap_or(self.nodes.len() - 1);
                    if let Some(n) = self.nodes.get_mut(self.active_node) {
                        n.has_unseen_update = false;
                    }
                    self.source_scroll = 0;
                    self.register_scroll = 0;
                    self.sync_source_scroll();
                }
            }
            Action::EditSource | Action::Reload => {
                // Handled by DebuggerRunner before reaching handle_action.
            }
            Action::ScrollUp => {
                self.source_scroll = self.source_scroll.saturating_sub(1);
            }
            Action::ScrollDown => {
                self.source_scroll = self.source_scroll.saturating_add(1);
            }
            Action::ScrollRegUp => {
                self.register_scroll = self.register_scroll.saturating_sub(1);
            }
            Action::ScrollRegDown => {
                self.register_scroll = self.register_scroll.saturating_add(1);
            }
            Action::None => {}
        }
        false
    }

    fn sync_source_scroll(&mut self) {
        if let Some(node) = self.active_node() {
            if let Some(src_line) = node.current_source_line() {
                // Keep the active line in the middle of the panel (approximate).
                self.source_scroll = src_line.saturating_sub(10);
            }
        }
    }

}

pub struct DebuggerRunner {
    app: DebuggerApp,
    spare_tx: mpsc::Sender<NodeUpdate>,
    stdin_from_file: bool,
    program_args: Vec<String>,
    output_shareds: Vec<Arc<DebugOutputShared>>,
}

impl DebuggerRunner {
    pub fn new(
        nodes: Vec<NodeDebugState>,
        update_rx: mpsc::Receiver<NodeUpdate>,
        spare_tx: mpsc::Sender<NodeUpdate>,
        stdin_from_file: bool,
        program_args: Vec<String>,
        output_shareds: Vec<Arc<DebugOutputShared>>,
    ) -> Self {
        let app = DebuggerApp::new(nodes, update_rx);
        Self { app, spare_tx, stdin_from_file, program_args, output_shareds }
    }

    pub fn run(mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        // Quit all node threads.
        for node in &self.app.nodes {
            let _ = node.cmd_tx.send(DebugCommand::Quit);
        }

        result
    }

    fn event_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> io::Result<()> {
        loop {
            self.app.drain_updates();
            self.drain_output();

            terminal.draw(|f| render(f, &self.app, self.stdin_from_file))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        // When the node is blocked waiting for stdin and we're
                        // NOT reading from a file, route keypresses to the input
                        // buffer instead of normal actions.
                        if !self.stdin_from_file && self.app.stdin_mode() {
                            self.handle_stdin_key(key.code);
                            continue;
                        }
                        // Handle edit/reload specially: they need spare_tx.
                        if key.code == crossterm::event::KeyCode::Char('e') {
                            self.handle_edit(terminal);
                            continue;
                        }
                        if key.code == crossterm::event::KeyCode::Char('r') {
                            self.handle_reload();
                            continue;
                        }
                        let action = key_to_action(key);
                        if self.app.handle_action(action) {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    fn drain_output(&mut self) {
        for shared in &self.output_shareds {
            let mut g = shared.lines.lock().unwrap();
            while let Some(entry) = g.pop_front() {
                if self.app.captured_output.len() >= 1000 {
                    self.app.captured_output.pop_front();
                }
                self.app.captured_output.push_back(entry);
            }
        }
    }

    fn handle_stdin_key(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
        match code {
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.app.input_buf);
                if let Some(node) = self.app.active_node() {
                    node.stdin_shared.provide(text + "\n");
                }
            }
            KeyCode::Backspace => {
                self.app.input_buf.pop();
            }
            KeyCode::Char(c) => {
                self.app.input_buf.push(c);
            }
            _ => {}
        }
    }

    fn handle_edit<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) {
        let path = match self.app.active_node() {
            Some(n) => n.source_path.clone(),
            None => return,
        };
        if let Some(node) = self.app.active_node_mut() {
            if !node.is_terminated {
                let _ = node.cmd_tx.send(DebugCommand::Pause);
            }
        }

        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);

        launch_editor(&path);

        let _ = enable_raw_mode();
        let _ = execute!(io::stdout(), EnterAlternateScreen);
        let _ = terminal.clear();

        self.app.status_message = Some(
            "File saved. Press 'r' to reload (restarts from step 0).".to_string(),
        );
        self.app.pending_reload = true;
    }

    fn handle_reload(&mut self) {
        let (path, idx, history_size) = match self.app.active_node() {
            Some(n) => (n.source_path.clone(), self.app.active_node, n.history_size),
            None => return,
        };

        if let Some(node) = self.app.active_node_mut() {
            let _ = node.cmd_tx.send(DebugCommand::Quit);
        }

        let spare_tx = self.spare_tx.clone();
        let (cmd_tx, cmd_rx) = mpsc::sync_channel(4);

        match super::spawn_reload_thread(path.clone(), idx, cmd_rx, spare_tx, self.program_args.clone()) {
            Some(meta) if meta.error.is_none() => {
                let new_state = NodeDebugState::new(
                    meta.name,
                    meta.source_lines,
                    path,
                    meta.pc_to_source_line,
                    cmd_tx,
                    history_size,
                    meta.stdin_shared,
                );
                if let Some(slot) = self.app.nodes.get_mut(idx) {
                    *slot = new_state;
                }
                if let Some(slot) = self.output_shareds.get_mut(idx) {
                    *slot = meta.output_shared;
                }
                self.app.source_scroll = 0;
                self.app.register_scroll = 0;
                self.app.status_message = Some("Node reloaded.".to_string());
            }
            Some(meta) => {
                self.app.status_message =
                    Some(format!("Reload failed: {}", meta.error.unwrap_or_default()));
            }
            None => {
                self.app.status_message = Some("Reload failed: thread error.".to_string());
            }
        }
        self.app.pending_reload = false;
    }
}

fn launch_editor(path: &str) {
    // Try VS Code first (code --wait), then $EDITOR, then notepad.
    let editors: &[&[&str]] = &[
        &["code", "--wait"],
        // $EDITOR is handled specially below
    ];

    let mut launched = false;

    for args in editors {
        if let Ok(status) = std::process::Command::new(args[0])
            .args(&args[1..])
            .arg(path)
            .status()
        {
            if status.success() || status.code().is_some() {
                launched = true;
                break;
            }
        }
    }

    if !launched {
        if let Ok(editor) = std::env::var("EDITOR") {
            if !editor.is_empty() {
                let _ = std::process::Command::new(&editor).arg(path).status();
                launched = true;
            }
        }
    }

    if !launched {
        // Windows fallback
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("notepad.exe").arg(path).status();
        #[cfg(not(target_os = "windows"))]
        let _ = std::process::Command::new("vi").arg(path).status();
    }
}

fn render(f: &mut ratatui::Frame, app: &DebuggerApp, stdin_from_file: bool) {
    let size = f.area();
    let show_stdin_bar = !stdin_from_file && app.stdin_mode();
    let show_output = !app.captured_output.is_empty();

    // Outer layout: main area + node tabs + [stdin bar] + help bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(if show_stdin_bar { 3 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(size);

    let main_area = outer[0];
    let tabs_area = outer[1];
    let stdin_area = outer[2];
    let help_area = outer[3];

    // Split main area: source (left) | right column
    let main_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(main_area);

    // Right column: registers on top, output panel below (when there's output)
    let right_split = if show_output {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(main_split[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])
            .split(main_split[1])
    };

    render_source(f, app, main_split[0]);
    render_registers(f, app, right_split[0]);
    if show_output {
        render_output(f, app, right_split[1]);
    }
    render_tabs(f, app, tabs_area);
    if show_stdin_bar {
        render_stdin_input(f, app, stdin_area);
    }
    render_help(f, app, help_area, stdin_from_file);
}

fn render_source(f: &mut ratatui::Frame, app: &DebuggerApp, area: Rect) {
    let node = match app.active_node() {
        Some(n) => n,
        None => {
            let p = Paragraph::new("No nodes loaded.")
                .block(Block::default().borders(Borders::ALL).title("SOURCE"));
            f.render_widget(p, area);
            return;
        }
    };

    let active_src_line = node.current_source_line();
    let inner_height = area.height.saturating_sub(2) as usize;

    // Auto-scroll: keep active line visible
    let scroll = if let Some(al) = active_src_line {
        let center = al.saturating_sub(inner_height / 2);
        center.min(node.source_lines.len().saturating_sub(inner_height))
    } else {
        app.source_scroll
    };

    let snap = node.current_snapshot();
    let title = match snap {
        Some(s) => {
            let status = if node.is_terminated {
                "DONE"
            } else if node.is_paused {
                "PAUSED"
            } else {
                "RUNNING"
            };
            format!(" {} — step {} [{}] ", node.name, s.step_index, status)
        }
        None => format!(" {} ", node.name),
    };

    let items: Vec<ListItem> = node
        .source_lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(inner_height)
        .map(|(line_idx, line_text)| {
            let is_active = active_src_line == Some(line_idx);
            let line_no = format!("{:4}  ", line_idx + 1);
            let cursor = if is_active { "> " } else { "  " };
            let content = format!("{}{}{}", cursor, line_no, line_text);
            let style = if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![Span::styled(content, style)]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, area);

    // Show current instruction repr below if we have a snapshot
    if let Some(snap) = snap {
        // We render it as part of the title block above, so also show it as a
        // short subtitle inside the block if there's room. Since we're using
        // List, just let the highlight do the talking.
        let _ = snap; // already used above
    }
}

fn render_registers(f: &mut ratatui::Frame, app: &DebuggerApp, area: Rect) {
    let node = match app.active_node() {
        Some(n) => n,
        None => {
            let p = Paragraph::new("").block(
                Block::default().borders(Borders::ALL).title("REGISTERS"),
            );
            f.render_widget(p, area);
            return;
        }
    };

    let snap = match node.current_snapshot() {
        Some(s) => s,
        None => {
            let p = Paragraph::new("No snapshot yet.")
                .block(Block::default().borders(Borders::ALL).title("REGISTERS"));
            f.render_widget(p, area);
            return;
        }
    };

    let inner_height = area.height.saturating_sub(2) as usize;
    let scroll = app.register_scroll;

    let items: Vec<ListItem> = snap
        .registers
        .iter()
        .skip(scroll)
        .take(inner_height)
        .map(|(name, val)| {
            let val_str = fmt_value_display(val);
            let content = format!("  {:12} = {}", name, val_str);
            ListItem::new(Line::from(Span::raw(content)))
        })
        .collect();

    // Also show the current instruction above the register list
    let instr_line = format!(" instr: {}", snap.instruction_repr);
    let title = format!("REGISTERS (pc={})", snap.pc);

    let mut all_items: Vec<ListItem> = vec![
        ListItem::new(Line::from(Span::styled(
            instr_line,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))),
        ListItem::new(Line::from(Span::raw("─".repeat(area.width.saturating_sub(2) as usize)))),
    ];
    all_items.extend(items);

    let list = List::new(all_items)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, area);
}

fn render_tabs(f: &mut ratatui::Frame, app: &DebuggerApp, area: Rect) {
    let mut spans: Vec<Span> = vec![Span::raw(" NODES: ")];
    for (i, node) in app.nodes.iter().enumerate() {
        let status = if node.is_terminated {
            "DONE"
        } else if node.is_paused {
            "PAUSED"
        } else {
            "RUN"
        };
        let marker = if node.has_unseen_update { "*" } else { " " };
        let label = format!("[{}{}:{}] ", node.name, marker, status);
        let style = if i == app.active_node {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(label, style));
    }

    if let Some(ref msg) = app.status_message {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans));
    f.render_widget(paragraph, area);
}

fn render_help(f: &mut ratatui::Frame, app: &DebuggerApp, area: Rect, stdin_from_file: bool) {
    let text = if !stdin_from_file && app.stdin_mode() {
        " [STDIN] type input and press Enter to send"
    } else {
        " n:step  b:back  c:continue  p:pause  Tab:next-node  e:edit  r:reload  q:quit  j/k:scroll  J/K:scroll-regs"
    };
    let help = Paragraph::new(Line::from(vec![Span::styled(
        text,
        Style::default().fg(Color::DarkGray),
    )]));
    f.render_widget(help, area);
}

fn render_stdin_input(f: &mut ratatui::Frame, app: &DebuggerApp, area: Rect) {
    let pending_desc = app
        .active_node()
        .and_then(|n| n.stdin_pending())
        .map(|req| match req {
            crate::register::StdinRequest::Bytes(n) => format!("stdin waiting ({} bytes)", n),
            crate::register::StdinRequest::Pattern(p) => format!("stdin waiting (until {:?})", p),
        })
        .unwrap_or_default();

    let content = format!("{}_", app.input_buf);
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(content, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(format!(" {} ", pending_desc)),
    );
    f.render_widget(paragraph, area);
}

fn render_output(f: &mut ratatui::Frame, app: &DebuggerApp, area: Rect) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let total = app.captured_output.len();
    // Auto-scroll to bottom unless user has scrolled up.
    let scroll = if app.output_scroll == 0 {
        total.saturating_sub(inner_height)
    } else {
        app.output_scroll.min(total.saturating_sub(inner_height))
    };

    let items: Vec<ListItem> = app
        .captured_output
        .iter()
        .enumerate()
        .skip(scroll)
        .take(inner_height)
        .map(|(_, (is_err, text))| {
            let style = if *is_err {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            };
            // Escape literal newlines so they don't break the line layout.
            let display = text.replace('\n', "↵").replace('\r', "");
            ListItem::new(Line::from(Span::styled(display, style)))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" OUTPUT "));
    f.render_widget(list, area);
}

fn fmt_value_display(v: &FlatValue) -> String {
    match v {
        FlatValue::Null => "null".to_string(),
        FlatValue::I(i) => i.to_string(),
        FlatValue::F(f) => format!("{:.4}", f),
        FlatValue::S(s) => {
            if s.len() > 40 {
                format!("\"{}...\"", &s[..37])
            } else {
                format!("\"{}\"", s)
            }
        }
    }
}
